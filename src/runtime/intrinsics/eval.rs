use super::*;
use crate::compiler::{EvalCompileContext, compile_unlinked_eval_with_filename};
use crate::heap::{
    EvalCallerProfile, EvalCallerVariableTarget, EvalKind, EvalRootBinding, EvalVariableEnvironment,
};
use crate::vm::DirectEvalInvocation;

impl Runtime {
    /// Publish a synthetic eval root only after the eval-specific verifier has
    /// matched every external closure slot against the invocation environment.
    fn publish_unlinked_eval_function(
        &self,
        realm: ContextId,
        function: UnlinkedFunction,
        expected: &EvalCompileContext,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        bytecode_publish::verify_unlinked_eval_tree_with_profile_and_arguments(
            &function,
            expected.kind,
            expected.caller_strict,
            &expected.bindings,
            &expected.caller_profile,
            bytecode_publish::EvalPublicationCapabilities {
                super_call_allowed: expected.super_call_allowed,
                super_allowed: expected.super_allowed,
                arguments_forbidden: expected.arguments_forbidden,
            },
        )?;
        self.publish_verified_unlinked_function(realm, function)
    }

    pub(in crate::runtime) fn initialize_eval_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let function = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::GlobalEval,
            1,
            "eval",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "eval",
            Value::Object(function.as_object().clone()),
            true,
            true,
        )?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_eval_function(realm, function.as_object().object_id())?;
        Ok(())
    }

    pub(in crate::runtime) fn call_global_eval(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global eval used an unexpected native invocation protocol",
            ));
        };
        let input = arguments
            .readable
            .first()
            .cloned()
            .unwrap_or(Value::Undefined);
        let Value::String(source) = input else {
            return Ok(Completion::Return(input));
        };
        let global_object = self.global_object_for_realm(realm)?;
        self.execute_string_eval(
            realm,
            &source,
            EvalCompileContext::indirect(),
            &[],
            Value::Object(global_object),
        )
    }

    /// Execute the original-eval branch selected by QuickJS `OP_eval` after
    /// realm-local identity matching. This deliberately bypasses the native
    /// `%eval%` call frame so String execution sees the bytecode caller's
    /// linked lexical environment.
    pub(in crate::runtime) fn call_direct_eval_original<F>(
        &self,
        realm: ContextId,
        invocation: DirectEvalInvocation,
        environment: Option<crate::runtime::vm_host::PreparedEvalEnvironment>,
        materialize: F,
    ) -> Result<Completion, RuntimeError>
    where
        F: FnOnce(
            crate::runtime::vm_host::PreparedEvalEnvironment,
        ) -> Result<crate::runtime::vm_host::MaterializedEvalEnvironment, Error>,
    {
        let DirectEvalInvocation {
            input,
            environment: environment_index,
            this_value,
            new_target: _,
            caller_strict,
        } = invocation;
        if !matches!(input, Value::String(_)) {
            if environment.is_some() {
                return Err(RuntimeError::Invariant(
                    "non-String direct eval prepared a caller environment",
                ));
            }
            return Ok(Completion::Return(input));
        }

        let environment = environment.ok_or(RuntimeError::Invariant(
            "String direct eval has no prepared caller environment",
        ))?;
        if environment.index != environment_index {
            return Err(RuntimeError::Invariant(
                "prepared eval environment has the wrong bytecode index",
            ));
        }
        if environment.descriptor.caller_strict != caller_strict {
            return Err(RuntimeError::Invariant(
                "prepared eval environment has the wrong caller strictness",
            ));
        }
        let binding_count = environment
            .descriptor
            .scopes
            .iter()
            .map(|scope| scope.bindings.len())
            .sum::<usize>();
        let expected_descriptor = environment.descriptor.clone();
        let source = Self::eval_source_text(match &input {
            Value::String(source) => source,
            _ => unreachable!("String direct eval was checked above"),
        })?;
        let (bindings, caller_profile) = self.direct_eval_root_bindings(realm, &environment)?;
        let arguments_forbidden = self
            .snapshot_function_bytecode(&environment.caller_bytecode)?
            .metadata
            .arguments_forbidden;
        let function = match self.compile_eval_in_realm(
            realm,
            &source,
            DEFAULT_EVAL_FILENAME,
            EvalCompileContext::direct_with_profile_and_arguments(
                caller_strict,
                bindings.clone(),
                caller_profile,
                environment.descriptor.super_call_allowed,
                environment.descriptor.super_allowed,
                arguments_forbidden,
            ),
        )? {
            Compilation::Published(function) => function,
            Compilation::Throw(value) => return Ok(Completion::Throw(value)),
        };

        // QuickJS parses and publishes eval bytecode before `js_closure2`
        // attaches it to caller VarRefs. Preserve that error/GC ordering by
        // invoking the host's capture step only after successful compilation.
        let environment = materialize(environment).map_err(RuntimeError::Engine)?;
        if environment.index != environment_index || environment.descriptor != expected_descriptor {
            return Err(RuntimeError::Invariant(
                "materialized eval environment disagrees with its prepared descriptor",
            ));
        }
        if environment.roots.len() != binding_count {
            return Err(RuntimeError::Invariant(
                "materialized eval roots disagree with the environment descriptor",
            ));
        }
        let callable = self.new_eval_bytecode_closure(
            realm,
            &function,
            EvalKind::Direct,
            &bindings,
            &environment.roots,
        )?;
        self.call_internal(realm, &callable, this_value, &[])
    }

    pub(in crate::runtime) fn is_original_eval(
        &self,
        realm: ContextId,
        function: &Value,
    ) -> Result<bool, RuntimeError> {
        if let Value::Object(object) = function {
            if !object.belongs_to(self) {
                return Err(RuntimeError::WrongRuntime("eval function"));
            }
        }
        let original = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .eval_function
            .ok_or(RuntimeError::Invariant(
                "context has no original eval function root",
            ))?;
        Ok(matches!(
            function,
            Value::Object(object) if object.object_id() == original
        ))
    }

    fn direct_eval_root_bindings(
        &self,
        realm: ContextId,
        environment: &crate::runtime::vm_host::PreparedEvalEnvironment,
    ) -> Result<(Vec<EvalRootBinding<JsString>>, EvalCallerProfile), RuntimeError> {
        if !environment.caller_bytecode.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("direct eval caller bytecode"));
        }
        let caller_realm = self
            .snapshot_function_bytecode(&environment.caller_bytecode)?
            .realm;
        if caller_realm != realm {
            return Err(RuntimeError::Invariant(
                "direct eval environment belongs to a different caller realm",
            ));
        }

        let binding_count = environment
            .descriptor
            .scopes
            .iter()
            .map(|scope| scope.bindings.len())
            .sum();
        let mut bindings = Vec::with_capacity(binding_count);
        let mut variable_target = None;
        let state = self.0.state.borrow();
        for (scope_index, descriptor_scope) in environment.descriptor.scopes.iter().enumerate() {
            if descriptor_scope.kind == crate::heap::EvalScopeKind::With
                && descriptor_scope.bindings.len() != 1
            {
                return Err(RuntimeError::Invariant(
                    "direct eval with scope does not contain exactly one object binding",
                ));
            }
            let scope = u16::try_from(scope_index).map_err(|_| {
                RuntimeError::Invariant("direct eval scope index exceeds bytecode range")
            })?;
            for binding in &descriptor_scope.bindings {
                let is_catch_scope = descriptor_scope.kind == crate::heap::EvalScopeKind::Catch;
                if (binding.is_catch_parameter && !is_catch_scope)
                    || (binding.is_catch_parameter
                        && (!binding.is_lexical
                            || binding.is_const
                            || binding.kind != ClosureVariableKind::Normal))
                {
                    return Err(RuntimeError::Invariant(
                        "direct eval catch binding metadata is not authentic",
                    ));
                }
                let is_with_scope = descriptor_scope.kind == crate::heap::EvalScopeKind::With;
                let name = state.atoms.to_js_string(binding.name)?;
                if (binding.kind == ClosureVariableKind::WithObject) != is_with_scope
                    || (binding.kind == ClosureVariableKind::WithObject
                        && (binding.is_lexical
                            || binding.is_const
                            || binding.is_catch_parameter
                            || name.utf16_units().ne("<with>".encode_utf16())
                            || matches!(
                                binding.source,
                                crate::heap::EvalBindingSource::Argument(_)
                            )))
                {
                    return Err(RuntimeError::Invariant(
                        "direct eval with-object binding metadata is not authentic",
                    ));
                }
                if binding.kind.is_eval_variable_object() {
                    let role_allowed = match descriptor_scope.kind {
                        crate::heap::EvalScopeKind::FunctionRoot => true,
                        crate::heap::EvalScopeKind::Parameter => {
                            binding.kind == ClosureVariableKind::ArgEvalVariableObject
                        }
                        _ => false,
                    };
                    let sentinel = match binding.kind {
                        ClosureVariableKind::EvalVariableObject => "<var>",
                        ClosureVariableKind::ArgEvalVariableObject => "<arg_var>",
                        _ => unreachable!("function anchor selected a variable-object role"),
                    };
                    if !role_allowed
                        || binding.is_lexical
                        || binding.is_const
                        || binding.is_catch_parameter
                        || matches!(binding.source, crate::heap::EvalBindingSource::Argument(_))
                        || name.utf16_units().ne(sentinel.encode_utf16())
                    {
                        return Err(RuntimeError::Invariant(
                            "direct eval variable-object binding metadata is not authentic",
                        ));
                    }
                }
                let external_index = u16::try_from(bindings.len()).map_err(|_| {
                    RuntimeError::Invariant("direct eval binding index exceeds bytecode range")
                })?;
                let targets_variable_environment =
                    match environment.descriptor.variable_environment {
                        EvalVariableEnvironment::Global
                        | EvalVariableEnvironment::StrictLocal(_) => false,
                        EvalVariableEnvironment::VariableObject {
                            scope: target_scope,
                            source,
                        } => {
                            let target_kind = match descriptor_scope.kind {
                                crate::heap::EvalScopeKind::FunctionRoot => {
                                    ClosureVariableKind::EvalVariableObject
                                }
                                crate::heap::EvalScopeKind::Parameter => {
                                    ClosureVariableKind::ArgEvalVariableObject
                                }
                                _ => ClosureVariableKind::Normal,
                            };
                            target_scope == scope
                                && binding.source == source
                                && binding.kind == target_kind
                        }
                    };
                if targets_variable_environment && variable_target.replace(external_index).is_some()
                {
                    return Err(RuntimeError::Invariant(
                        "direct eval variable environment has multiple target bindings",
                    ));
                }
                bindings.push(EvalRootBinding {
                    name,
                    scope,
                    is_lexical: binding.is_lexical,
                    is_const: binding.is_const,
                    kind: binding.kind,
                    is_catch_parameter: binding.is_catch_parameter,
                });
            }
        }
        let variable_target = match environment.descriptor.variable_environment {
            EvalVariableEnvironment::Global if variable_target.is_none() => {
                if environment.descriptor.caller_strict {
                    // A strict authored Script still has the global caller
                    // variable environment, but strict direct eval creates
                    // its declarations in the eval root's own local record.
                    EvalCallerVariableTarget::StrictLocal
                } else {
                    EvalCallerVariableTarget::Global
                }
            }
            EvalVariableEnvironment::StrictLocal(_)
                if environment.descriptor.caller_strict && variable_target.is_none() =>
            {
                EvalCallerVariableTarget::StrictLocal
            }
            EvalVariableEnvironment::VariableObject { .. }
                if !environment.descriptor.caller_strict =>
            {
                EvalCallerVariableTarget::ExternalBinding(variable_target.ok_or(
                    RuntimeError::Invariant(
                        "direct eval variable environment has no target binding",
                    ),
                )?)
            }
            EvalVariableEnvironment::Global
            | EvalVariableEnvironment::StrictLocal(_)
            | EvalVariableEnvironment::VariableObject { .. } => {
                return Err(RuntimeError::Invariant(
                    "direct eval variable environment disagrees with caller strictness or target",
                ));
            }
        };
        let caller_profile = EvalCallerProfile {
            scope_kinds: environment
                .descriptor
                .scopes
                .iter()
                .map(|scope| scope.kind)
                .collect::<Vec<_>>()
                .into_boxed_slice(),
            variable_target,
        };
        Ok((bindings, caller_profile))
    }

    fn execute_string_eval(
        &self,
        realm: ContextId,
        source: &JsString,
        context: EvalCompileContext,
        environment_roots: &[VarRefRoot],
        this_value: Value,
    ) -> Result<Completion, RuntimeError> {
        let source = Self::eval_source_text(source)?;
        let kind = context.kind;
        let bindings = context.bindings.clone();
        let function =
            match self.compile_eval_in_realm(realm, &source, DEFAULT_EVAL_FILENAME, context)? {
                Compilation::Published(function) => function,
                Compilation::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let callable =
            self.new_eval_bytecode_closure(realm, &function, kind, &bindings, environment_roots)?;
        self.call_internal(realm, &callable, this_value, &[])
    }

    fn eval_source_text(source: &JsString) -> Result<String, RuntimeError> {
        if !source.is_well_formed() {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Unsupported,
                "eval source containing an unpaired UTF-16 surrogate is not implemented yet",
            )));
        }
        // The well-formedness check makes this conversion exact despite the
        // helper's historical name.
        Ok(source.to_utf8_lossy())
    }

    fn compile_eval_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
        context: EvalCompileContext,
    ) -> Result<Compilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let expected = context.clone();
        let function =
            match compile_unlinked_eval_with_filename(source, filename, debug_info, context) {
                Ok(function) => function,
                Err(error) => {
                    let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                        return Err(RuntimeError::Engine(error));
                    };
                    let explicit_location = if error.kind() == ErrorKind::Syntax {
                        if let Some(span) = error.span() {
                            let position = QuickJsSourceLocator::new(source)
                                .locate_byte_offset(span.start.byte_offset)
                                .map_err(|_| {
                                    RuntimeError::Invariant(
                                        "eval syntax-error byte offset is invalid for its source",
                                    )
                                })?;
                            Some(ExplicitBacktraceLocation {
                                filename: JsString::try_from_utf8(filename)?,
                                position,
                            })
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    let exception = if error.kind() == ErrorKind::Syntax {
                        self.new_native_error_without_backtrace_from_error(realm, kind, &error)?
                    } else {
                        self.new_native_error_from_error(realm, kind, &error)?
                    };
                    self.ensure_error_backtrace(&exception, false, explicit_location)?;
                    return Ok(Compilation::Throw(exception));
                }
            };
        Ok(Compilation::Published(
            self.publish_unlinked_eval_function(realm, function, &expected)?,
        ))
    }

    fn new_eval_bytecode_closure(
        &self,
        realm: ContextId,
        function: &FunctionBytecodeRef,
        kind: EvalKind,
        bindings: &[EvalRootBinding<JsString>],
        environment_roots: &[VarRefRoot],
    ) -> Result<CallableRef, RuntimeError> {
        let PublishedFunctionSnapshot {
            closure_variables,
            metadata,
            realm: function_realm,
            ..
        } = self.snapshot_function_bytecode(function)?;
        if function_realm != realm || metadata.eval_kind != kind {
            return Err(RuntimeError::Invariant(
                "published eval bytecode disagrees with its invocation realm or kind",
            ));
        }
        match kind {
            EvalKind::Direct if environment_roots.len() == bindings.len() => {}
            EvalKind::Indirect if environment_roots.is_empty() && bindings.is_empty() => {}
            EvalKind::None => {
                return Err(RuntimeError::Invariant(
                    "ordinary bytecode reached eval closure instantiation",
                ));
            }
            EvalKind::Direct | EvalKind::Indirect => {
                return Err(RuntimeError::Invariant(
                    "eval invocation roots disagree with its compiler environment",
                ));
            }
        }

        let allows_global_declarations = !metadata.strict
            && match kind {
                EvalKind::Direct => !bindings
                    .iter()
                    .any(|binding| binding.kind.is_eval_variable_object()),
                EvalKind::Indirect => true,
                EvalKind::None => false,
            };

        // QuickJS checks every GLOBAL_DECL before attaching or creating any
        // binding. A later conflict must not leave earlier eval declarations
        // installed on the global object.
        for descriptor in closure_variables.iter().copied() {
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published eval closure descriptor has no atom",
                ));
            };
            match descriptor.source {
                ClosureSource::GlobalDeclaration => {
                    if !allows_global_declarations {
                        return Err(RuntimeError::Invariant(
                            "eval bytecode retained a global declaration for a local variable environment",
                        ));
                    }
                    let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
                    match descriptor.kind {
                        ClosureVariableKind::Normal
                            if !descriptor.is_lexical && !descriptor.is_const =>
                        {
                            self.check_global_var_declaration(realm, &key)?;
                        }
                        ClosureVariableKind::GlobalFunction
                            if !descriptor.is_lexical && !descriptor.is_const =>
                        {
                            self.check_global_function_declaration(realm, &key)?;
                        }
                        ClosureVariableKind::Normal
                        | ClosureVariableKind::FunctionName
                        | ClosureVariableKind::GlobalFunction
                        | ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                        | ClosureVariableKind::WithObject => {
                            return Err(RuntimeError::Invariant(
                                "eval global declaration has non-global binding metadata",
                            ));
                        }
                    }
                }
                ClosureSource::Global => {
                    if descriptor.kind != ClosureVariableKind::Normal {
                        return Err(RuntimeError::Invariant(
                            "resolved eval global has declaration-only binding metadata",
                        ));
                    }
                }
                ClosureSource::EvalEnvironment(_) => {}
                ClosureSource::ParentLocal(_)
                | ClosureSource::ParentArgument(_)
                | ClosureSource::ParentClosure(_)
                | ClosureSource::ParentGlobal(_) => {
                    return Err(RuntimeError::Invariant(
                        "eval root closure descriptor used a child source",
                    ));
                }
            }
        }

        if closure_variables.len() < bindings.len() {
            return Err(RuntimeError::Invariant(
                "eval environment closure descriptor count is too small",
            ));
        }

        // Caller roots form an exact authenticated prefix. Attach every root
        // before global declaration creation so a forged late descriptor can
        // never cause partial global state.
        let mut slots = vec![None; closure_variables.len()];
        for (index, (expected, root)) in bindings.iter().zip(environment_roots).enumerate() {
            let descriptor = closure_variables[index];
            let expected_index = u16::try_from(index).map_err(|_| {
                RuntimeError::Invariant("eval environment closure index exceeds bytecode range")
            })?;
            if descriptor.source != ClosureSource::EvalEnvironment(expected_index) {
                return Err(RuntimeError::Invariant(
                    "eval environment closure descriptors are not an exact prefix",
                ));
            }
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published eval environment descriptor has no atom",
                ));
            };
            if expected.is_catch_parameter
                && (!expected.is_lexical
                    || expected.is_const
                    || expected.kind != ClosureVariableKind::Normal)
            {
                return Err(RuntimeError::Invariant(
                    "eval catch binding has invalid binding metadata",
                ));
            }
            if expected.kind.is_eval_variable_object()
                && (expected.is_lexical || expected.is_const || expected.is_catch_parameter)
            {
                return Err(RuntimeError::Invariant(
                    "eval variable-object binding has invalid binding metadata",
                ));
            }
            if expected.kind == ClosureVariableKind::WithObject
                && (expected.is_lexical
                    || expected.is_const
                    || expected.is_catch_parameter
                    || expected.name.utf16_units().ne("<with>".encode_utf16()))
            {
                return Err(RuntimeError::Invariant(
                    "eval with-object binding has invalid binding metadata",
                ));
            }
            let published_name = self.0.state.borrow().atoms.to_js_string(name)?;
            if published_name != expected.name
                || descriptor.is_lexical != expected.is_lexical
                || descriptor.is_const != expected.is_const
                || descriptor.kind != expected.kind
            {
                return Err(RuntimeError::Invariant(
                    "published eval closure disagrees with its caller binding",
                ));
            }
            self.validate_var_ref_metadata(root, descriptor)?;
            slots[index] = Some(root.clone());
        }

        // Every caller root is now attached. Instantiate global slots in
        // descriptor order, retaining QuickJS's eval-specific configurable
        // property attributes for newly created/replaced bindings.
        for (index, descriptor) in closure_variables
            .iter()
            .copied()
            .enumerate()
            .skip(bindings.len())
        {
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published eval closure descriptor has no atom",
                ));
            };
            let root = match descriptor.source {
                ClosureSource::GlobalDeclaration => {
                    let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
                    match descriptor.kind {
                        ClosureVariableKind::Normal
                            if !descriptor.is_lexical && !descriptor.is_const =>
                        {
                            self.create_global_var_binding(
                                realm,
                                &key,
                                GlobalBindingCreationMode::Eval,
                            )?
                        }
                        ClosureVariableKind::GlobalFunction
                            if !descriptor.is_lexical && !descriptor.is_const =>
                        {
                            self.create_global_function_binding(
                                realm,
                                &key,
                                GlobalBindingCreationMode::Eval,
                            )?
                        }
                        ClosureVariableKind::Normal
                        | ClosureVariableKind::FunctionName
                        | ClosureVariableKind::GlobalFunction
                        | ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                        | ClosureVariableKind::WithObject => {
                            return Err(RuntimeError::Invariant(
                                "eval global declaration has non-global binding metadata",
                            ));
                        }
                    }
                }
                ClosureSource::Global => self.resolve_global_var(realm, name)?,
                ClosureSource::EvalEnvironment(_) => {
                    return Err(RuntimeError::Invariant(
                        "eval environment closure descriptors are not an exact prefix",
                    ));
                }
                ClosureSource::ParentLocal(_)
                | ClosureSource::ParentArgument(_)
                | ClosureSource::ParentClosure(_)
                | ClosureSource::ParentGlobal(_) => {
                    return Err(RuntimeError::Invariant(
                        "eval root closure descriptor used a child source",
                    ));
                }
            };
            slots[index] = Some(root);
        }
        let slots =
            slots
                .into_iter()
                .collect::<Option<Vec<_>>>()
                .ok_or(RuntimeError::Invariant(
                    "eval closure instantiation left an unattached slot",
                ))?;
        self.new_bytecode_closure_with_slots(realm, function, &slots)
    }
}
