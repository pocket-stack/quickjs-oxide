use super::*;

impl Heap {
    /// Allocate and publish a shape, retaining its prototype edge.
    ///
    /// The caller owns one returned shape reference and must eventually call
    /// [`Heap::release_shape`].  Atom references are owned by the caller until
    /// this succeeds, then by the shape until returned as cleanup.
    pub fn allocate_shape(&mut self, shape: Shape) -> Result<ShapeId, HeapError> {
        let (index, generation) = self.reserve(HeapNodeKind::Shape)?;
        let id = ShapeId { index, generation };
        let edges = shape_edges(&shape);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::Shape(shape))?;
        Ok(id)
    }

    /// Allocate and publish an object, retaining its shape and property edges.
    ///
    /// The caller owns one returned object reference and must eventually call
    /// [`Heap::release_object`].
    pub fn allocate_object(&mut self, object: ObjectData) -> Result<ObjectId, HeapError> {
        if matches!(
            &object.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData { realm: None, .. },
                ..
            }
        ) {
            return Err(HeapError::Invariant(
                "an unbound native function may only be allocated during realm bootstrap",
            ));
        }
        self.allocate_object_inner(object)
    }

    /// Allocate a genuine WeakRef behind the runtime intrinsic surface. The
    /// target remains a non-owning generational identity.
    pub(crate) fn allocate_weak_ref_object(
        &mut self,
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        target: WeakCollectionKey,
    ) -> Result<ObjectId, HeapError> {
        self.validate_live_weak_target(target)?;
        self.allocate_object_inner(ObjectData::weak_ref(shape, slots, target))
    }

    /// Allocate a genuine FinalizationRegistry behind its public runtime
    /// constructor. Callback and realm are traced by the payload itself.
    pub(crate) fn allocate_finalization_registry_object(
        &mut self,
        shape: ShapeId,
        slots: Vec<PropertySlot>,
        callback: ObjectId,
        realm: ContextId,
    ) -> Result<ObjectId, HeapError> {
        if !object_data_is_callable(self.object(callback)?) {
            return Err(HeapError::Invariant(
                "FinalizationRegistry callback is not callable",
            ));
        }
        self.context(realm)?;
        self.allocate_object_inner(ObjectData::finalization_registry(
            shape, slots, callback, realm,
        ))
    }

    /// Allocate the one provisional native callable needed to bootstrap a
    /// realm. The caller must synchronously finish it with
    /// [`Self::attach_native_function_realm`] before exposing it.
    pub(crate) fn allocate_bootstrap_native_function(
        &mut self,
        object: ObjectData,
    ) -> Result<ObjectId, HeapError> {
        if !matches!(
            &object.payload,
            ObjectPayload::NativeFunction {
                data: NativeFunctionData {
                    target: NativeFunctionId::FunctionPrototype,
                    realm: None,
                    ..
                },
                ..
            }
        ) {
            return Err(HeapError::Invariant(
                "bootstrap native-function allocation requires an unbound Function.prototype",
            ));
        }
        self.allocate_object_inner(object)
    }

    pub(in crate::engine::heap) fn allocate_object_inner(
        &mut self,
        object: ObjectData,
    ) -> Result<ObjectId, HeapError> {
        self.validate_object_layout(&object)?;
        let is_weak_object = matches!(
            &object.payload,
            ObjectPayload::WeakMap { .. }
                | ObjectPayload::WeakSet { .. }
                | ObjectPayload::WeakRef { .. }
                | ObjectPayload::FinalizationRegistry(_)
        );
        let (index, generation) = self.reserve(HeapNodeKind::Object)?;
        let id = ObjectId { index, generation };
        let edges = object_edges(&object);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::Object(object))?;
        if is_weak_object {
            self.link_weak_object(id)?;
        }
        Ok(id)
    }

    /// Allocate and publish a realm/context node, retaining all realm roots.
    /// Symbol atoms in `intrinsics` transfer to the node on success.
    pub fn allocate_context(&mut self, context: ContextData) -> Result<ContextId, HeapError> {
        if context
            .intrinsics
            .iter()
            .any(|value| matches!(value, RawValue::Private(_)))
        {
            return Err(HeapError::Invariant(
                "private-name identity escaped into a realm intrinsic",
            ));
        }
        let (index, generation) = self.reserve(HeapNodeKind::Context)?;
        let id = ContextId { index, generation };
        let edges = context_edges(&context);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::Context(context))?;
        Ok(id)
    }

    /// Allocate and publish immutable function bytecode, retaining its realm
    /// and every GC edge in its constant pool. `auxiliary_atoms` and symbol
    /// constants transfer to the node on success. No arena slot is reserved
    /// until both metadata authentication and shared bytecode verification
    /// succeed.
    pub fn allocate_function_bytecode(
        &mut self,
        bytecode: FunctionBytecodeData,
    ) -> Result<FunctionBytecodeId, HeapError> {
        if bytecode
            .constants
            .iter()
            .any(|constant| matches!(constant, BytecodeConstant::Value(RawValue::Private(_))))
        {
            return Err(HeapError::Invariant(
                "private-name identity escaped into a bytecode constant",
            ));
        }
        if bytecode.metadata.local_count > MAX_LOCAL_SLOTS {
            return Err(HeapError::Invariant(
                "bytecode local count exceeds QuickJS JS_MAX_LOCAL_VARS",
            ));
        }
        let parameter_initializer_locals = bytecode
            .local_definitions
            .iter()
            .map(|definition| definition.is_parameter_initializer)
            .collect::<Vec<_>>();
        let parameter_body_pc = validate_parameter_bytecode_layout(
            &bytecode.metadata,
            &bytecode.code,
            &parameter_initializer_locals,
            bytecode.parameter_environment.as_ref(),
        )
        .map_err(HeapError::Invariant)?;
        let initial_yields = bytecode
            .code
            .iter()
            .enumerate()
            .filter_map(|(pc, instruction)| {
                matches!(instruction, Instruction::InitialYield).then_some(pc)
            })
            .collect::<Vec<_>>();
        let has_generator_only_instruction = bytecode.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::Yield
                    | Instruction::YieldStar
                    | Instruction::AsyncYieldStar
                    | Instruction::IteratorStart
                    | Instruction::AsyncIteratorStart
                    | Instruction::IteratorNext
                    | Instruction::IteratorCall(_)
                    | Instruction::IteratorCheckObject
                    | Instruction::IteratorDetachPreserve
                    | Instruction::ThrowIteratorMissingThrow
            )
        });
        let has_await = bytecode
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Await));
        let has_sync_delegation = bytecode.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::YieldStar | Instruction::IteratorStart
            )
        });
        let has_async_delegation = bytecode.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::AsyncYieldStar | Instruction::AsyncIteratorStart
            )
        });
        let has_async_iteration = bytecode.code.iter().any(|instruction| {
            matches!(
                instruction,
                Instruction::ForAwaitOfStart
                    | Instruction::ForAwaitOfNext
                    | Instruction::IteratorGetValueDone
            )
        });
        let has_async_generator_iterator_detach = bytecode
            .code
            .iter()
            .any(|instruction| matches!(instruction, Instruction::IteratorDetachPreserve));
        match bytecode.metadata.function_kind {
            FunctionKind::Generator => {
                if initial_yields.len() != 1
                    || !bytecode.metadata.has_prototype
                    || bytecode.metadata.constructor_kind != ConstructorKind::None
                    || bytecode.metadata.class_initializer_kind.is_some()
                    || parameter_body_pc.is_some_and(|body_pc| initial_yields[0] < body_pc)
                    || has_await
                    || has_async_delegation
                    || has_async_iteration
                    || has_async_generator_iterator_detach
                {
                    return Err(HeapError::Invariant(
                        "generator bytecode has invalid suspension metadata",
                    ));
                }
            }
            FunctionKind::Async => {
                if !initial_yields.is_empty()
                    || has_generator_only_instruction
                    || bytecode.metadata.has_prototype
                    || bytecode.metadata.constructor_kind != ConstructorKind::None
                    || bytecode.metadata.class_initializer_kind.is_some()
                    || has_async_generator_iterator_detach
                {
                    return Err(HeapError::Invariant(
                        "async bytecode has invalid execution metadata",
                    ));
                }
            }
            FunctionKind::AsyncGenerator => {
                if initial_yields.len() != 1
                    || !bytecode.metadata.has_prototype
                    || bytecode.metadata.constructor_kind != ConstructorKind::None
                    || bytecode.metadata.class_initializer_kind.is_some()
                    || parameter_body_pc.is_some_and(|body_pc| initial_yields[0] < body_pc)
                    || has_sync_delegation
                {
                    return Err(HeapError::Invariant(
                        "async-generator bytecode has invalid suspension metadata",
                    ));
                }
            }
            FunctionKind::Normal => {
                if !initial_yields.is_empty()
                    || has_generator_only_instruction
                    || has_await
                    || has_async_iteration
                {
                    return Err(HeapError::Invariant(
                        "non-async bytecode contains a suspension opcode",
                    ));
                }
            }
        }
        if bytecode.metadata.is_module
            && (!bytecode.metadata.strict
                || bytecode.metadata.function_kind != FunctionKind::Async
                || bytecode.metadata.eval_kind != EvalKind::None
                || bytecode.metadata.argument_count != 0
                || bytecode.metadata.defined_argument_count != 0
                || bytecode.metadata.rest_parameter.is_some()
                || bytecode.metadata.rest_pattern_start.is_some()
                || bytecode.metadata.parameter_environment_local_count != 0
                || bytecode.metadata.pattern_argument_count != 0
                || bytecode.metadata.parameter_pattern_end.is_some()
                || bytecode.parameter_environment.is_some()
                || bytecode.metadata.function_name_local.is_some()
                || bytecode.metadata.derived_this_local.is_some()
                || bytecode.metadata.active_function_local.is_some()
                || bytecode.metadata.eval_variable_object_local.is_some()
                || bytecode.metadata.super_call_allowed
                || bytecode.metadata.super_allowed
                || bytecode.metadata.arguments_forbidden
                || bytecode.metadata.needs_home_object
                || bytecode.metadata.has_prototype
                || bytecode.metadata.constructor_kind != ConstructorKind::None
                || bytecode.metadata.class_initializer_kind.is_some()
                || bytecode.metadata.class_private_brand)
        {
            return Err(HeapError::Invariant(
                "module bytecode has invalid root metadata",
            ));
        }
        if bytecode
            .metadata
            .function_name_local
            .is_some_and(|index| index >= bytecode.metadata.local_count)
        {
            return Err(HeapError::Invariant(
                "function-name local is outside bytecode local slots",
            ));
        }
        for (local, message) in [
            (
                bytecode.metadata.derived_this_local,
                "derived this local is outside bytecode local slots",
            ),
            (
                bytecode.metadata.active_function_local,
                "active-function local is outside bytecode local slots",
            ),
        ] {
            if local.is_some_and(|index| index >= bytecode.metadata.local_count) {
                return Err(HeapError::Invariant(message));
            }
        }
        if bytecode
            .metadata
            .eval_variable_object_local
            .is_some_and(|index| index >= bytecode.metadata.local_count)
        {
            return Err(HeapError::Invariant(
                "eval variable-object local is outside bytecode local slots",
            ));
        }
        if bytecode.metadata.eval_variable_object_local.is_some()
            && bytecode.metadata.eval_variable_object_local == bytecode.metadata.function_name_local
        {
            return Err(HeapError::Invariant(
                "eval variable-object and function-name locals overlap",
            ));
        }
        let arg_eval_variable_object_local = bytecode
            .parameter_environment
            .as_ref()
            .and_then(|layout| layout.arg_eval_variable_object_local);
        if arg_eval_variable_object_local
            .is_some_and(|index| index >= bytecode.metadata.local_count)
        {
            return Err(HeapError::Invariant(
                "parameter eval variable-object local is outside bytecode local slots",
            ));
        }
        if let Some(index) = arg_eval_variable_object_local
            && (bytecode.metadata.eval_variable_object_local == Some(index)
                || bytecode.metadata.function_name_local == Some(index))
        {
            return Err(HeapError::Invariant(
                "parameter eval variable-object local overlaps another private local",
            ));
        }
        let private_locals = [
            bytecode.metadata.function_name_local,
            bytecode.metadata.eval_variable_object_local,
            arg_eval_variable_object_local,
            bytecode.metadata.derived_this_local,
            bytecode.metadata.active_function_local,
        ];
        for (index, local) in private_locals.iter().enumerate() {
            if local.is_some()
                && private_locals[..index]
                    .iter()
                    .any(|earlier| earlier == local)
            {
                return Err(HeapError::Invariant("authenticated private locals overlap"));
            }
        }
        if bytecode.argument_definitions.len() != usize::from(bytecode.metadata.argument_count) {
            return Err(HeapError::Invariant(
                "argument definition count does not match bytecode metadata",
            ));
        }
        if bytecode.local_definitions.len() != usize::from(bytecode.metadata.local_count) {
            return Err(HeapError::Invariant(
                "local definition count does not match bytecode metadata",
            ));
        }
        validate_published_private_elements(self, &bytecode)?;
        let unnamed_arguments = bytecode
            .argument_definitions
            .iter()
            .map(|definition| definition.name.is_none())
            .collect::<Vec<_>>();
        let lexical_locals = bytecode
            .local_definitions
            .iter()
            .map(|definition| definition.is_lexical)
            .collect::<Vec<_>>();
        let const_locals = bytecode
            .local_definitions
            .iter()
            .map(|definition| definition.is_const)
            .collect::<Vec<_>>();
        validate_derived_constructor_bytecode_layout(
            &bytecode.metadata,
            &bytecode.code,
            &lexical_locals,
            &const_locals,
            &bytecode.closure_variables,
        )
        .map_err(HeapError::Invariant)?;
        validate_class_initializer_bytecode_layout(&bytecode.metadata, &bytecode.code)
            .map_err(HeapError::Invariant)?;
        let pattern_body_pc = bytecode
            .metadata
            .parameter_pattern_end
            .and_then(|marker| usize::try_from(marker).ok())
            .and_then(|marker| marker.checked_add(1));
        validate_parameter_initializer_scope_layout(
            &bytecode.metadata,
            &bytecode.code,
            parameter_body_pc.or(pattern_body_pc),
            &lexical_locals,
            &parameter_initializer_locals,
        )
        .map_err(HeapError::Invariant)?;
        validate_pattern_parameter_bytecode_layout(
            &bytecode.metadata,
            &bytecode.code,
            &unnamed_arguments,
            &lexical_locals,
            &parameter_initializer_locals,
            bytecode.parameter_environment.as_ref(),
        )
        .map_err(HeapError::Invariant)?;
        let parameter_initializer_capture_locals = parameter_initializer_visible_locals(
            &bytecode.metadata,
            &bytecode.code,
            parameter_body_pc,
            &parameter_initializer_locals,
            bytecode.parameter_environment.as_ref(),
        )
        .map_err(HeapError::Invariant)?;
        validate_eval_environment_phase_layout(
            &bytecode.eval_environments,
            EvalEnvironmentPhaseContext {
                metadata: &bytecode.metadata,
                code: &bytecode.code,
                parameter_body_pc,
                pattern_body_pc,
                lexical_locals: &lexical_locals,
                parameter_initializer_locals: &parameter_initializer_locals,
                parameter_initializer_visible_locals: parameter_initializer_capture_locals
                    .as_deref(),
                parameter_environment: bytecode.parameter_environment.as_ref(),
            },
        )
        .map_err(HeapError::Invariant)?;
        for definition in bytecode.argument_definitions.iter() {
            if definition.kind != ClosureVariableKind::Normal
                || definition.is_lexical
                || definition.is_const
                || definition.is_parameter_initializer
            {
                return Err(HeapError::Invariant(
                    "argument definition is not an ordinary mutable binding",
                ));
            }
        }
        if let Some(layout) = bytecode.parameter_environment.as_ref() {
            let parameter_definitions = bytecode
                .local_definitions
                .iter()
                .take(usize::from(
                    bytecode.metadata.parameter_environment_local_count,
                ))
                .collect::<Vec<_>>();
            for (index, local) in parameter_definitions.iter().enumerate() {
                if local.kind != ClosureVariableKind::Normal
                    || !local.is_lexical
                    || local.is_const
                    || local.is_parameter_initializer
                    || local.name.is_none()
                    || parameter_definitions[..index]
                        .iter()
                        .any(|earlier| earlier.name == local.name)
                {
                    return Err(HeapError::Invariant(
                        "parameter environment cell definition is not authenticated",
                    ));
                }
            }
            let mut mapped_arguments = vec![false; bytecode.argument_definitions.len()];
            for cell in layout.argument_cells.iter() {
                mapped_arguments[usize::from(cell.argument)] = true;
                let argument = &bytecode.argument_definitions[usize::from(cell.argument)];
                let local = &bytecode.local_definitions[usize::from(cell.parameter_local)];
                if argument.name.is_none() || argument.name != local.name {
                    return Err(HeapError::Invariant(
                        "parameter argument cell name disagrees with its physical argument",
                    ));
                }
            }
            if bytecode
                .argument_definitions
                .iter()
                .zip(mapped_arguments)
                .any(|(argument, mapped)| argument.name.is_some() != mapped)
            {
                return Err(HeapError::Invariant(
                    "parameter argument-cell map is not one-to-one with named arguments",
                ));
            }
            for copy in layout.pattern_copies.iter() {
                let source = &bytecode.local_definitions[usize::from(copy.parameter_local)];
                let target = &bytecode.local_definitions[usize::from(copy.body_local)];
                if target.kind != ClosureVariableKind::Normal
                    || target.is_lexical
                    || target.is_const
                    || source.is_parameter_initializer
                    || target.is_parameter_initializer
                    || source.name != target.name
                {
                    return Err(HeapError::Invariant(
                        "parameter pattern copy definitions are not same-name lexical-to-root storage",
                    ));
                }
            }
            if let Some(index) = layout.synthetic_arguments_local {
                let definition = &bytecode.local_definitions[usize::from(index)];
                if definition.kind != ClosureVariableKind::Normal
                    || !definition.is_lexical
                    || definition.is_const
                    || definition.is_parameter_initializer
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "synthetic parameter arguments definition is not authenticated",
                    ));
                }
            }
        }
        for (index, definition) in bytecode.local_definitions.iter().enumerate() {
            let is_function_name =
                bytecode.metadata.function_name_local == u16::try_from(index).ok();
            let is_derived_this = bytecode.metadata.derived_this_local == u16::try_from(index).ok();
            let is_active_function =
                bytecode.metadata.active_function_local == u16::try_from(index).ok();
            let is_eval_variable_object =
                bytecode.metadata.eval_variable_object_local == u16::try_from(index).ok();
            let is_arg_eval_variable_object =
                arg_eval_variable_object_local == u16::try_from(index).ok();
            if is_function_name {
                if definition.kind != ClosureVariableKind::FunctionName
                    || definition.is_lexical
                    || definition.is_const != bytecode.metadata.strict
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "function-name definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_derived_this {
                if definition.kind != ClosureVariableKind::Normal
                    || !definition.is_lexical
                    || definition.is_const
                    || definition.is_parameter_initializer
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "derived this definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_active_function {
                if definition.kind != ClosureVariableKind::Normal
                    || definition.is_lexical
                    || definition.is_const
                    || definition.is_parameter_initializer
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "active-function definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_eval_variable_object {
                if definition.kind != ClosureVariableKind::EvalVariableObject
                    || definition.is_lexical
                    || definition.is_const
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "eval variable-object definition disagrees with bytecode metadata",
                    ));
                }
            } else if is_arg_eval_variable_object {
                if definition.kind != ClosureVariableKind::ArgEvalVariableObject
                    || definition.is_lexical
                    || definition.is_const
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "parameter eval variable-object definition disagrees with bytecode layout",
                    ));
                }
            } else if definition.kind == ClosureVariableKind::WithObject {
                if bytecode.metadata.strict
                    || definition.is_lexical
                    || definition.is_const
                    || definition.name.is_none()
                {
                    return Err(HeapError::Invariant(
                        "strict or malformed bytecode contains a with-object local",
                    ));
                }
            } else if definition.kind != ClosureVariableKind::Normal
                && !definition.kind.is_private()
            {
                return Err(HeapError::Invariant(
                    "ordinary local definition uses a non-local binding kind",
                ));
            } else if definition.is_const && !definition.is_lexical {
                return Err(HeapError::Invariant(
                    "a const local definition must also be lexical",
                ));
            }
        }
        if bytecode.closure_variables.len() != usize::from(bytecode.metadata.closure_count) {
            return Err(HeapError::Invariant(
                "function closure descriptor count does not match its bytecode metadata",
            ));
        }
        let mut owned_name_atoms = HashMap::<Atom, usize>::new();
        for atom in bytecode.auxiliary_atoms.iter().copied() {
            *owned_name_atoms.entry(atom).or_default() += 1;
        }
        if let Some(debug) = &bytecode.debug {
            if debug.filename.is_null() {
                return Err(HeapError::Invariant(
                    "bytecode debug filename is the null atom",
                ));
            }
            let Some(count) = owned_name_atoms.get_mut(&debug.filename) else {
                return Err(HeapError::Invariant(
                    "debug filename atom is not owned by bytecode metadata",
                ));
            };
            if *count == 0 {
                return Err(HeapError::Invariant(
                    "debug filename atom ownership multiplicity is too small",
                ));
            }
            *count -= 1;
            if let Some(table) = &debug.pc2line {
                if table.definition.line == u32::MAX || table.definition.column == u32::MAX {
                    return Err(HeapError::Invariant(
                        "bytecode debug definition cannot be represented one-based",
                    ));
                }
                let mut previous_pc = None;
                for entry in &table.entries {
                    if usize::try_from(entry.pc)
                        .ok()
                        .is_none_or(|pc| pc >= bytecode.code.len())
                    {
                        return Err(HeapError::Invariant(
                            "bytecode debug PC is outside the instruction stream",
                        ));
                    }
                    if previous_pc.is_some_and(|previous| entry.pc < previous) {
                        return Err(HeapError::Invariant("bytecode debug PCs are not ordered"));
                    }
                    if entry.position.line == u32::MAX || entry.position.column == u32::MAX {
                        return Err(HeapError::Invariant(
                            "bytecode debug position cannot be represented one-based",
                        ));
                    }
                    previous_pc = Some(entry.pc);
                }
            }
        }
        for definition in bytecode
            .argument_definitions
            .iter()
            .chain(bytecode.local_definitions.iter())
        {
            if definition.kind == ClosureVariableKind::ModuleImportView {
                return Err(HeapError::Invariant(
                    "module-import view escaped into a variable definition",
                ));
            }
            let Some(atom) = definition.name else {
                continue;
            };
            let Some(count) = owned_name_atoms.get_mut(&atom) else {
                return Err(HeapError::Invariant(
                    "variable-definition name atom is not owned by bytecode metadata",
                ));
            };
            if *count == 0 {
                return Err(HeapError::Invariant(
                    "variable-definition name atom ownership multiplicity is too small",
                ));
            }
            *count -= 1;
        }
        let mut global_declaration_names = HashMap::new();
        let mut import_meta_count = 0_u8;
        for descriptor in bytecode.closure_variables.iter().copied() {
            if bytecode.metadata.is_module {
                if !matches!(
                    descriptor.source,
                    ClosureSource::ModuleDeclaration
                        | ClosureSource::ModuleImport
                        | ClosureSource::ModuleImportCollision
                        | ClosureSource::ModuleImportMeta
                        | ClosureSource::Global
                ) {
                    return Err(HeapError::Invariant(
                        "module root closure descriptor used a non-module source",
                    ));
                }
            } else if matches!(
                descriptor.source,
                ClosureSource::ModuleDeclaration
                    | ClosureSource::ModuleImport
                    | ClosureSource::ModuleImportCollision
                    | ClosureSource::ModuleImportMeta
            ) {
                return Err(HeapError::Invariant(
                    "module closure descriptor escaped module bytecode",
                ));
            }
            match descriptor.source {
                ClosureSource::ModuleDeclaration
                    if descriptor.kind != ClosureVariableKind::Normal
                        || (descriptor.is_const && !descriptor.is_lexical) =>
                {
                    return Err(HeapError::Invariant(
                        "module declaration descriptor has invalid binding metadata",
                    ));
                }
                ClosureSource::ModuleImport
                    if descriptor.kind != ClosureVariableKind::ModuleImportView
                        || !descriptor.is_lexical
                        || !descriptor.is_const =>
                {
                    return Err(HeapError::Invariant(
                        "module import descriptor has invalid binding metadata",
                    ));
                }
                ClosureSource::ModuleImportCollision
                    if !descriptor.is_lexical
                        || !descriptor.is_const
                        || !matches!(
                            descriptor.kind,
                            ClosureVariableKind::Normal | ClosureVariableKind::ModuleImportView
                        ) =>
                {
                    return Err(HeapError::Invariant(
                        "module import collision descriptor has invalid binding metadata",
                    ));
                }
                ClosureSource::ModuleImportMeta
                    if descriptor.kind != ClosureVariableKind::Normal
                        || !descriptor.is_lexical
                        || !descriptor.is_const =>
                {
                    return Err(HeapError::Invariant(
                        "import.meta descriptor has invalid binding metadata",
                    ));
                }
                _ => {}
            }
            if descriptor.source == ClosureSource::ModuleImportMeta {
                import_meta_count = import_meta_count.saturating_add(1);
                if import_meta_count > 1 {
                    return Err(HeapError::Invariant(
                        "module bytecode contains more than one import.meta binding",
                    ));
                }
            }
            if descriptor.kind == ClosureVariableKind::ModuleImportView
                && (!descriptor.is_lexical
                    || !descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ModuleImport
                            | ClosureSource::ModuleImportCollision
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(HeapError::Invariant(
                    "module-import view descriptor has invalid provenance",
                ));
            }
            if matches!(descriptor.source, ClosureSource::EvalEnvironment(_))
                && bytecode.metadata.eval_kind != EvalKind::Direct
            {
                return Err(HeapError::Invariant(
                    "eval-environment closure escaped a direct-eval root",
                ));
            }
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && (descriptor.is_lexical || descriptor.is_const)
            {
                return Err(HeapError::Invariant(
                    "global function declaration descriptor has lexical metadata",
                ));
            }
            if descriptor.kind.is_eval_variable_object()
                && (descriptor.is_lexical
                    || descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(HeapError::Invariant(
                    "eval variable-object descriptor has invalid binding metadata",
                ));
            }
            if descriptor.kind == ClosureVariableKind::WithObject
                && (descriptor.is_lexical
                    || descriptor.is_const
                    || !matches!(
                        descriptor.source,
                        ClosureSource::ParentLocal(_)
                            | ClosureSource::ParentClosure(_)
                            | ClosureSource::EvalEnvironment(_)
                    ))
            {
                return Err(HeapError::Invariant(
                    "with-object descriptor has invalid binding metadata",
                ));
            }
            if descriptor.is_const
                && !descriptor.is_lexical
                && descriptor.kind != ClosureVariableKind::FunctionName
            {
                return Err(HeapError::Invariant(
                    "a const closure descriptor must also be lexical",
                ));
            }
            if (descriptor.source == ClosureSource::GlobalDeclaration
                && !matches!(
                    descriptor.kind,
                    ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                ))
                || (descriptor.source == ClosureSource::Global
                    && descriptor.kind != ClosureVariableKind::Normal)
                || (matches!(descriptor.source, ClosureSource::ParentGlobal(_))
                    && !matches!(
                        descriptor.kind,
                        ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                    ))
            {
                return Err(HeapError::Invariant(
                    "global declaration descriptor has non-global binding metadata",
                ));
            }
            if descriptor.kind == ClosureVariableKind::GlobalFunction
                && !matches!(
                    descriptor.source,
                    ClosureSource::GlobalDeclaration | ClosureSource::ParentGlobal(_)
                )
            {
                return Err(HeapError::Invariant(
                    "global function binding kind escaped a declaration relay",
                ));
            }
            let requires_name = matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
                    | ClosureSource::EvalEnvironment(_)
                    | ClosureSource::ModuleDeclaration
                    | ClosureSource::ModuleImport
                    | ClosureSource::ModuleImportCollision
                    | ClosureSource::ModuleImportMeta
            ) || matches!(
                descriptor.kind,
                ClosureVariableKind::FunctionName
                    | ClosureVariableKind::EvalVariableObject
                    | ClosureVariableKind::ArgEvalVariableObject
                    | ClosureVariableKind::WithObject
            ) || descriptor.kind.is_private();
            let allows_name = requires_name
                || descriptor.is_lexical
                || matches!(
                    descriptor.source,
                    ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentArgument(_)
                        | ClosureSource::ParentClosure(_)
                );
            if descriptor.source == ClosureSource::GlobalDeclaration
                && let ClosureVariableName::Atom(atom) = descriptor.name
            {
                match global_declaration_names.entry(atom) {
                    std::collections::hash_map::Entry::Vacant(entry) => {
                        entry.insert((descriptor.is_lexical, descriptor.is_lexical));
                    }
                    std::collections::hash_map::Entry::Occupied(mut entry) => {
                        let (first_is_lexical, seen_lexical) = *entry.get();
                        if first_is_lexical
                            && seen_lexical
                            && (descriptor.is_lexical
                                || descriptor.kind != ClosureVariableKind::GlobalFunction)
                        {
                            return Err(HeapError::Invariant(
                                "duplicate lexical global declaration descriptor name",
                            ));
                        }
                        // A first sloppy Annex B normal record masks every
                        // later same-name declaration in QuickJS's conflict
                        // lookup, including repeated lexical and var records.
                        // A first lexical remains stricter.
                        if descriptor.is_lexical {
                            entry.get_mut().1 = true;
                        }
                    }
                }
            }
            if matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration
                    | ClosureSource::Global
                    | ClosureSource::ParentGlobal(_)
            ) && !matches!(
                descriptor.kind,
                ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
            ) {
                return Err(HeapError::Invariant(
                    "published global closure descriptor has a non-global binding kind",
                ));
            }
            match descriptor.name {
                ClosureVariableName::Atom(atom) if allows_name => {
                    let Some(count) = owned_name_atoms.get_mut(&atom) else {
                        return Err(HeapError::Invariant(
                            "closure name atom is not owned by bytecode metadata",
                        ));
                    };
                    if *count == 0 {
                        return Err(HeapError::Invariant(
                            "closure name atom ownership multiplicity is too small",
                        ));
                    }
                    *count -= 1;
                }
                ClosureVariableName::None if !requires_name => {}
                ClosureVariableName::Constant(_) => {
                    return Err(HeapError::Invariant(
                        "published closure descriptor retained an unlinked name constant",
                    ));
                }
                ClosureVariableName::None | ClosureVariableName::Atom(_) => {
                    return Err(HeapError::Invariant(
                        "published closure descriptor name does not match its binding kind",
                    ));
                }
            }
        }
        for environment in bytecode.eval_environments.iter() {
            let first_function_anchor = environment
                .scopes
                .iter()
                .position(|scope| {
                    matches!(
                        scope.kind,
                        EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                    )
                })
                .and_then(|scope| u16::try_from(scope).ok())
                .ok_or(HeapError::Invariant(
                    "eval environment contains no representable current function anchor",
                ))?;
            match environment.variable_environment {
                EvalVariableEnvironment::Global => {
                    let current_body_is_program = first_function_anchor
                        .checked_sub(1)
                        .and_then(|scope| environment.scopes.get(usize::from(scope)))
                        .is_some_and(|scope| scope.kind == EvalScopeKind::ProgramBody);
                    if bytecode.metadata.is_module
                        || !current_body_is_program
                        || (environment.caller_strict
                            && bytecode.metadata.eval_kind != EvalKind::None)
                    {
                        return Err(HeapError::Invariant(
                            "global eval variable environment escaped an authored Script root",
                        ));
                    }
                }
                EvalVariableEnvironment::StrictLocal(scope) if environment.caller_strict => {
                    if scope != first_function_anchor {
                        return Err(HeapError::Invariant(
                            "strict eval variable environment selected the wrong current function segment",
                        ));
                    }
                    let current_body_is_program = first_function_anchor
                        .checked_sub(1)
                        .and_then(|scope| environment.scopes.get(usize::from(scope)))
                        .is_some_and(|scope| scope.kind == EvalScopeKind::ProgramBody);
                    if current_body_is_program
                        && bytecode.metadata.eval_kind == EvalKind::None
                        && !bytecode.metadata.is_module
                    {
                        return Err(HeapError::Invariant(
                            "authored Script eval environment used a non-canonical strict-local target",
                        ));
                    }
                    let Some(scope) = environment.scopes.get(usize::from(scope)) else {
                        return Err(HeapError::Invariant(
                            "strict eval variable-environment scope is out of bounds",
                        ));
                    };
                    if !matches!(
                        scope.kind,
                        EvalScopeKind::FunctionRoot | EvalScopeKind::Parameter
                    ) {
                        return Err(HeapError::Invariant(
                            "strict eval variable environment has the wrong function segment anchor",
                        ));
                    }
                }
                EvalVariableEnvironment::VariableObject { scope, source }
                    if !environment.caller_strict =>
                {
                    let target_matches_function_segment =
                        if bytecode.metadata.eval_kind == EvalKind::None {
                            scope == first_function_anchor
                                && matches!(source, EvalBindingSource::Local(_))
                        } else {
                            bytecode.metadata.eval_kind == EvalKind::Direct
                                && scope > first_function_anchor
                                && matches!(source, EvalBindingSource::Closure(_))
                        };
                    if !target_matches_function_segment {
                        return Err(HeapError::Invariant(
                            "eval variable object selected the wrong current function segment",
                        ));
                    }
                    let Some(scope) = environment.scopes.get(usize::from(scope)) else {
                        return Err(HeapError::Invariant(
                            "eval variable-object scope is out of bounds",
                        ));
                    };
                    let expected_kind = match scope.kind {
                        EvalScopeKind::FunctionRoot => ClosureVariableKind::EvalVariableObject,
                        EvalScopeKind::Parameter => ClosureVariableKind::ArgEvalVariableObject,
                        _ => {
                            return Err(HeapError::Invariant(
                                "eval variable object has the wrong function segment scope",
                            ));
                        }
                    };
                    if matches!(source, EvalBindingSource::Argument(_))
                        || scope
                            .bindings
                            .iter()
                            .filter(|binding| {
                                binding.source == source && binding.kind == expected_kind
                            })
                            .count()
                            != 1
                    {
                        return Err(HeapError::Invariant(
                            "eval variable-object target is not exact",
                        ));
                    }
                }
                EvalVariableEnvironment::StrictLocal(_)
                | EvalVariableEnvironment::VariableObject { .. } => {
                    return Err(HeapError::Invariant(
                        "eval variable environment disagrees with caller strictness",
                    ));
                }
            }
            for scope in environment.scopes.iter() {
                if scope.kind == EvalScopeKind::With && scope.bindings.len() != 1 {
                    return Err(HeapError::Invariant(
                        "eval with scope does not contain exactly one object binding",
                    ));
                }
                for binding in &scope.bindings {
                    if binding.name.is_null() {
                        return Err(HeapError::Invariant("eval binding name is the null atom"));
                    }
                    let source_name_matches = match binding.source {
                        EvalBindingSource::Local(index) => bytecode
                            .local_definitions
                            .get(usize::from(index))
                            .is_some_and(|definition| definition.name == Some(binding.name)),
                        EvalBindingSource::Argument(index) => bytecode
                            .argument_definitions
                            .get(usize::from(index))
                            .is_some_and(|definition| definition.name == Some(binding.name)),
                        EvalBindingSource::Closure(index) => bytecode
                            .closure_variables
                            .get(usize::from(index))
                            .is_some_and(|descriptor| {
                                descriptor.name == ClosureVariableName::Atom(binding.name)
                            }),
                    };
                    if !source_name_matches {
                        return Err(HeapError::Invariant(
                            "eval binding name atom disagrees with its source metadata",
                        ));
                    }
                    if binding.is_catch_parameter
                        && (scope.kind != EvalScopeKind::Catch
                            || !binding.is_lexical
                            || binding.is_const
                            || binding.kind != ClosureVariableKind::Normal)
                    {
                        return Err(HeapError::Invariant(
                            "eval catch binding metadata disagrees with its scope",
                        ));
                    }
                    if (binding.kind == ClosureVariableKind::WithObject)
                        != (scope.kind == EvalScopeKind::With)
                        || (binding.kind == ClosureVariableKind::WithObject
                            && (binding.is_lexical
                                || binding.is_const
                                || binding.is_catch_parameter
                                || matches!(binding.source, EvalBindingSource::Argument(_))))
                    {
                        return Err(HeapError::Invariant(
                            "eval with-object binding metadata disagrees with its scope",
                        ));
                    }
                    if binding.kind.is_eval_variable_object() {
                        let role_allowed = match scope.kind {
                            EvalScopeKind::FunctionRoot => true,
                            EvalScopeKind::Parameter => {
                                binding.kind == ClosureVariableKind::ArgEvalVariableObject
                            }
                            _ => false,
                        };
                        if !role_allowed
                            || binding.is_lexical
                            || binding.is_const
                            || binding.is_catch_parameter
                        {
                            return Err(HeapError::Invariant(
                                "eval variable-object binding has invalid metadata",
                            ));
                        }
                        let authenticated = match binding.source {
                            EvalBindingSource::Local(index) => {
                                let expected = match binding.kind {
                                    ClosureVariableKind::EvalVariableObject => {
                                        bytecode.metadata.eval_variable_object_local
                                    }
                                    ClosureVariableKind::ArgEvalVariableObject => {
                                        arg_eval_variable_object_local
                                    }
                                    _ => {
                                        unreachable!("eval variable-object role was checked above")
                                    }
                                };
                                expected == Some(index)
                                    && bytecode
                                        .local_definitions
                                        .get(usize::from(index))
                                        .is_some_and(|definition| definition.kind == binding.kind)
                            }
                            EvalBindingSource::Closure(index) => bytecode
                                .closure_variables
                                .get(usize::from(index))
                                .is_some_and(|descriptor| descriptor.kind == binding.kind),
                            EvalBindingSource::Argument(_) => false,
                        };
                        if !authenticated {
                            return Err(HeapError::Invariant(
                                "eval variable-object binding source is not authenticated",
                            ));
                        }
                    }
                    if binding.kind == ClosureVariableKind::WithObject {
                        let authenticated = match binding.source {
                            EvalBindingSource::Local(index) => bytecode
                                .local_definitions
                                .get(usize::from(index))
                                .is_some_and(|definition| {
                                    definition.kind == ClosureVariableKind::WithObject
                                        && !definition.is_lexical
                                        && !definition.is_const
                                }),
                            EvalBindingSource::Closure(index) => bytecode
                                .closure_variables
                                .get(usize::from(index))
                                .is_some_and(|descriptor| {
                                    descriptor.kind == ClosureVariableKind::WithObject
                                        && !descriptor.is_lexical
                                        && !descriptor.is_const
                                }),
                            EvalBindingSource::Argument(_) => false,
                        };
                        if !authenticated {
                            return Err(HeapError::Invariant(
                                "eval with-object binding source is not authenticated",
                            ));
                        }
                    }
                    let Some(count) = owned_name_atoms.get_mut(&binding.name) else {
                        return Err(HeapError::Invariant(
                            "eval binding name atom is not owned by bytecode metadata",
                        ));
                    };
                    if *count == 0 {
                        return Err(HeapError::Invariant(
                            "eval binding name atom ownership multiplicity is too small",
                        ));
                    }
                    *count -= 1;
                }
            }
        }
        crate::engine::code::bytecode::verify_parts(
            &bytecode.code,
            bytecode.constants.len(),
            bytecode.metadata.max_stack,
        )
        .map_err(|_| HeapError::Invariant("function bytecode failed generic verification"))?;
        let (index, generation) = self.reserve(HeapNodeKind::FunctionBytecode)?;
        let id = FunctionBytecodeId { index, generation };
        let edges = function_bytecode_edges(&bytecode);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::FunctionBytecode(bytecode))?;
        Ok(id)
    }

    /// Allocate a captured-variable cell and transfer ownership of its value
    /// to the heap. The returned reference is normally owned by the active
    /// frame; closure objects retain the same `VarRefId` when published.
    pub fn allocate_var_ref(&mut self, var_ref: VarRefData) -> Result<VarRefId, HeapError> {
        validate_var_ref_payload(&var_ref)?;
        let (index, generation) = self.reserve(HeapNodeKind::VarRef)?;
        let id = VarRefId { index, generation };
        let edges = var_ref_edges(&var_ref);

        if let Err(error) = self.retain_edges_transactionally(&edges) {
            self.abort_initializing(index)?;
            return Err(error);
        }

        self.publish(index, NodeData::VarRef(var_ref))?;
        Ok(id)
    }
}
