//! Bytecode VM adapter and per-frame binding state.
//!
//! This module owns the translation between VM completions/errors and the
//! runtime's object, call, iterator, realm and captured-variable machinery.

use super::*;
use crate::bytecode::{
    ApplyKind, ArgumentsKind, DefineMethodKind, DynamicEnvironmentSource, EvalVariableSource,
};
use crate::heap::{EvalBinding, EvalBindingSource, EvalVariableEnvironment};
use crate::vm::{
    AppendStartOutcome, ArgumentListOutcome, CallInput, DirectEvalInvocation, Vm, VmHost,
};

mod dynamic_environment;
mod super_property;

/// Validated caller state retained while primitive-String eval is compiled.
///
/// No frame binding has been converted to a VarRef yet. This preserves
/// QuickJS's ordering: parse/publish errors occur before closure capture.
pub(in crate::runtime) struct PreparedEvalEnvironment {
    pub(in crate::runtime) index: u16,
    pub(in crate::runtime) caller_bytecode: FunctionBytecodeRef,
    pub(in crate::runtime) descriptor: EvalEnvironment<Atom>,
}

/// Live cells paired with one immutable caller-environment descriptor.
///
/// Roots are flattened in the descriptor's scope/binding order. The
/// descriptor itself preserves the lexical boundaries and declaration target
/// authenticated by the eval compiler, while the roots keep the caller's
/// actual cells live for the instantiation/execution interval.
pub(in crate::runtime) struct MaterializedEvalEnvironment {
    pub(in crate::runtime) index: u16,
    /// Retain the owner of descriptor atoms through final instantiation.
    pub(in crate::runtime) _caller_bytecode: FunctionBytecodeRef,
    pub(in crate::runtime) descriptor: EvalEnvironment<Atom>,
    pub(in crate::runtime) roots: Box<[VarRefRoot]>,
}

enum FrameBinding {
    Direct(Value),
    Uninitialized,
    Captured(VarRefRoot),
}

/// QuickJS keeps access flags on each closure descriptor rather than on the
/// shared VarRef. Its ordinary direct-eval prepass may therefore expose one
/// FunctionName cell through a mutable Normal descriptor. Publication
/// authenticates where this one-way erasure enters the closure chain.
pub(super) fn closure_view_matches_cell(
    cell: (bool, bool, ClosureVariableKind),
    descriptor: ClosureVariable,
) -> bool {
    cell == (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
        || (cell.0 == descriptor.is_lexical
            && !cell.0
            && cell.2 == ClosureVariableKind::FunctionName
            && !descriptor.is_const
            && descriptor.kind == ClosureVariableKind::Normal)
}

fn read_frame_binding(runtime: &Runtime, binding: &FrameBinding) -> Result<Value, Error> {
    match binding {
        FrameBinding::Direct(value) => Ok(value.clone()),
        FrameBinding::Uninitialized => Err(Error::internal(
            "unchecked local read reached an uninitialized lexical binding",
        )),
        FrameBinding::Captured(root) => runtime
            .read_var_ref(root)
            .map_err(|error| Error::internal(error.to_string())),
    }
}

fn write_frame_binding(
    runtime: &Runtime,
    binding: &mut FrameBinding,
    value: Value,
) -> Result<(), Error> {
    match binding {
        FrameBinding::Direct(slot) => {
            *slot = value;
            Ok(())
        }
        FrameBinding::Uninitialized => Err(Error::internal(
            "unchecked local write reached an uninitialized lexical binding",
        )),
        FrameBinding::Captured(root) => runtime
            .write_var_ref(root, value)
            .map_err(|error| Error::internal(error.to_string())),
    }
}

fn capture_frame_binding(
    runtime: &Runtime,
    binding: &mut FrameBinding,
    descriptor: ClosureVariable,
) -> Result<VarRefRoot, Error> {
    match binding {
        FrameBinding::Direct(value) => {
            let root = runtime
                .new_var_ref(
                    value.clone(),
                    descriptor.is_lexical,
                    descriptor.is_const,
                    descriptor.kind,
                )
                .map_err(|error| Error::internal(error.to_string()))?;
            *binding = FrameBinding::Captured(root.clone());
            Ok(root)
        }
        FrameBinding::Uninitialized => {
            let root = runtime
                .new_uninitialized_captured_var_ref(
                    descriptor.is_lexical,
                    descriptor.is_const,
                    descriptor.kind,
                )
                .map_err(|error| Error::internal(error.to_string()))?;
            *binding = FrameBinding::Captured(root.clone());
            Ok(root)
        }
        FrameBinding::Captured(root) => {
            runtime
                .validate_var_ref_metadata(root, descriptor)
                .map_err(|error| Error::internal(error.to_string()))?;
            Ok(root.clone())
        }
    }
}

fn close_frame_binding(runtime: &Runtime, binding: &mut FrameBinding) -> Result<(), Error> {
    let FrameBinding::Captured(root) = binding else {
        return Ok(());
    };
    let raw = runtime
        .raw_var_ref_value(root)
        .map_err(|error| Error::internal(error.to_string()))?;
    let detached = if matches!(raw, RawValue::Uninitialized) {
        FrameBinding::Uninitialized
    } else {
        FrameBinding::Direct(
            runtime
                .root_raw_value(&raw)
                .map_err(runtime_error_to_vm_error)?,
        )
    };
    *binding = detached;
    Ok(())
}

fn runtime_error_to_vm_error(error: RuntimeError) -> Error {
    match error {
        RuntimeError::Engine(error) => error,
        error => Error::internal(error.to_string()),
    }
}

pub(super) struct RuntimeVmHost {
    runtime: Runtime,
    active_frame_token: ActiveFrameToken,
    current_realm: ContextId,
    current_bytecode: Option<FunctionBytecodeRef>,
    /// Current callee retained for sloppy mapped `arguments.callee`.
    /// Detached host-only tests do not execute the arguments opcode.
    current_function: Option<ObjectRef>,
    /// Authored call arity before the argument frame was padded to formal
    /// width. `arguments.length` and its dense prefix use this exact count.
    actual_argument_count: usize,
    constants: Rc<[BytecodeConstant]>,
    argument_definitions: Rc<[VariableDefinition]>,
    local_definitions: Rc<[VariableDefinition]>,
    closure_variables: Rc<[ClosureVariable]>,
    eval_environments: Rc<[EvalEnvironment<Atom>]>,
    /// Exact local slot authenticated by bytecode metadata as this frame's
    /// hidden sloppy-eval variable object.
    eval_variable_object_local: Option<u16>,
    /// Exact local slot authenticated by the parameter-environment layout as
    /// the independent hidden sloppy-eval argument-scope variable object.
    arg_eval_variable_object_local: Option<u16>,
    closure_slots: Vec<VarRefRoot>,
    arguments: Vec<FrameBinding>,
    locals: Vec<FrameBinding>,
    /// QuickJS can resume the same frame after a caught throw or a return
    /// unwind without emitting `CloseLocal` for captured lexical cells. Only
    /// cells captured at one of those exact boundaries may be reset in place
    /// by the next lexical scope entry.
    reusable_captured_locals: Vec<bool>,
}

enum VmPropertyKeyConversion {
    Key(PropertyKey),
    Throw(Value),
}

impl RuntimeVmHost {
    #[cfg(test)]
    pub(super) fn empty_for_test(runtime: Runtime, current_realm: ContextId) -> Self {
        Self {
            runtime,
            active_frame_token: ActiveFrameToken(0),
            current_realm,
            current_bytecode: None,
            current_function: None,
            actual_argument_count: 0,
            constants: Rc::from([]),
            argument_definitions: Rc::from([]),
            local_definitions: Rc::from([]),
            closure_variables: Rc::from([]),
            eval_environments: Rc::from([]),
            eval_variable_object_local: None,
            arg_eval_variable_object_local: None,
            closure_slots: Vec::new(),
            arguments: Vec::new(),
            locals: Vec::new(),
            reusable_captured_locals: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(super) fn eval_frame_for_test(
        runtime: Runtime,
        current_realm: ContextId,
        bytecode: &FunctionBytecodeRef,
        closure_slots: Vec<VarRefRoot>,
        arguments: Vec<Value>,
        locals: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        let PublishedFunctionSnapshot {
            root,
            code: _,
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            arg_eval_variable_object_local,
            metadata,
            realm,
        } = runtime.snapshot_function_bytecode(bytecode)?;
        if realm != current_realm {
            return Err(RuntimeError::Invariant(
                "test eval frame realm disagrees with its bytecode",
            ));
        }
        if arguments.len() != argument_definitions.len()
            || locals.len() != local_definitions.len()
            || closure_slots.len() != closure_variables.len()
        {
            return Err(RuntimeError::Invariant(
                "test eval frame slots disagree with bytecode metadata",
            ));
        }
        let frame_local_count = locals.len();
        Ok(Self {
            runtime,
            active_frame_token: ActiveFrameToken(0),
            current_realm,
            current_bytecode: Some(root),
            current_function: None,
            actual_argument_count: arguments.len(),
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            eval_variable_object_local: metadata.eval_variable_object_local,
            arg_eval_variable_object_local,
            closure_slots,
            arguments: arguments.into_iter().map(FrameBinding::Direct).collect(),
            locals: locals.into_iter().map(FrameBinding::Direct).collect(),
            reusable_captured_locals: vec![false; frame_local_count],
        })
    }

    #[cfg(test)]
    pub(super) fn eval_binding_is_captured_for_test(&self, source: EvalBindingSource) -> bool {
        match source {
            EvalBindingSource::Local(index) => self
                .locals
                .get(usize::from(index))
                .is_some_and(|binding| matches!(binding, FrameBinding::Captured(_))),
            EvalBindingSource::Argument(index) => self
                .arguments
                .get(usize::from(index))
                .is_some_and(|binding| matches!(binding, FrameBinding::Captured(_))),
            EvalBindingSource::Closure(index) => {
                self.closure_slots.get(usize::from(index)).is_some()
            }
        }
    }

    fn finish_property_define(
        &mut self,
        result: Result<PropertyDefineOutcome, RuntimeError>,
    ) -> Result<Completion, Error> {
        match result {
            Ok(PropertyDefineOutcome::Defined(true)) => Ok(Completion::Return(Value::Undefined)),
            Ok(PropertyDefineOutcome::Defined(false)) => {
                Err(Error::new(ErrorKind::Type, "property is not configurable"))
            }
            Ok(PropertyDefineOutcome::Throw(value)) => Ok(Completion::Throw(value)),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved a JavaScript-visible property error");
                let value = self
                    .runtime
                    .new_native_error_from_error(self.current_realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?;
                Ok(Completion::Throw(value))
            }
            Err(error) => Err(runtime_error_to_vm_error(error)),
        }
    }

    fn local_definition(&self, index: u16) -> Result<VariableDefinition, Error> {
        self.local_definitions
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("local definition index is out of bounds"))
    }

    fn argument_definition(&self, index: u16) -> Result<VariableDefinition, Error> {
        self.argument_definitions
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("argument definition index is out of bounds"))
    }

    fn validate_capture_definition(
        &self,
        definition: VariableDefinition,
        descriptor: ClosureVariable,
    ) -> Result<(), Error> {
        let descriptor_name = match descriptor.name {
            ClosureVariableName::None => None,
            ClosureVariableName::Atom(name) => Some(name),
            ClosureVariableName::Constant(_) => {
                return Err(Error::internal(
                    "published closure descriptor retained an unlinked name constant",
                ));
            }
        };
        let definition_flags = (definition.is_lexical, definition.is_const, definition.kind);
        // Publication has already proven that any erased FunctionName view
        // reaches a real direct-eval descriptor through its ParentClosure
        // lineage. Runtime instantiation only needs to match that authenticated
        // view against the canonical shared cell.
        let flags_match = closure_view_matches_cell(definition_flags, descriptor);
        let name_matches = if definition.is_lexical
            || definition.kind == ClosureVariableKind::FunctionName
            || descriptor_name.is_some()
        {
            definition.name == descriptor_name
        } else {
            true
        };
        if !flags_match || !name_matches {
            return Err(Error::internal(
                "closure descriptor disagrees with its parent variable definition",
            ));
        }
        Ok(())
    }

    fn eval_capture_descriptor(binding: &EvalBinding<Atom>) -> ClosureVariable {
        let source = match binding.source {
            EvalBindingSource::Local(index) => ClosureSource::ParentLocal(index),
            EvalBindingSource::Argument(index) => ClosureSource::ParentArgument(index),
            EvalBindingSource::Closure(index) => ClosureSource::ParentClosure(index),
        };
        ClosureVariable {
            source,
            name: ClosureVariableName::Atom(binding.name),
            is_lexical: binding.is_lexical,
            is_const: binding.is_const,
            kind: binding.kind,
        }
    }

    fn validate_eval_definition(
        definition: VariableDefinition,
        binding: &EvalBinding<Atom>,
    ) -> Result<(), Error> {
        if definition.name != Some(binding.name)
            || definition.is_lexical != binding.is_lexical
            || definition.is_const != binding.is_const
            || definition.kind != binding.kind
        {
            return Err(Error::internal(
                "eval binding disagrees with its caller variable definition",
            ));
        }
        Ok(())
    }

    fn validate_eval_closure(
        descriptor: ClosureVariable,
        binding: &EvalBinding<Atom>,
    ) -> Result<(), Error> {
        if matches!(
            descriptor.source,
            ClosureSource::GlobalDeclaration
                | ClosureSource::Global
                | ClosureSource::ParentGlobal(_)
        ) {
            return Err(Error::internal(
                "eval environment retained a global closure binding",
            ));
        }
        let name_matches =
            matches!(descriptor.name, ClosureVariableName::Atom(name) if name == binding.name);
        if !name_matches
            || descriptor.is_lexical != binding.is_lexical
            || descriptor.is_const != binding.is_const
            || descriptor.kind != binding.kind
        {
            return Err(Error::internal(
                "eval binding disagrees with its caller closure descriptor",
            ));
        }
        Ok(())
    }

    fn eval_variable_object_local_kind(&self, index: u16) -> Option<ClosureVariableKind> {
        if self.eval_variable_object_local == Some(index) {
            return Some(ClosureVariableKind::EvalVariableObject);
        }
        if self.arg_eval_variable_object_local == Some(index) {
            return Some(ClosureVariableKind::ArgEvalVariableObject);
        }
        None
    }

    fn validate_eval_environment(
        &self,
        environment: &EvalEnvironment<Atom>,
        caller_strict: bool,
        caller_metadata: FunctionMetadata,
    ) -> Result<(), Error> {
        if environment.caller_strict != caller_strict {
            return Err(Error::internal(
                "eval environment caller strictness disagrees with its bytecode frame",
            ));
        }
        if caller_metadata.super_call_allowed && !caller_metadata.super_allowed {
            return Err(Error::internal(
                "caller bytecode permits super() without SuperProperty",
            ));
        }
        if environment.super_call_allowed && !environment.super_allowed {
            return Err(Error::internal(
                "eval environment permits super() without SuperProperty",
            ));
        }
        if (environment.super_call_allowed, environment.super_allowed)
            != (
                caller_metadata.super_call_allowed,
                caller_metadata.super_allowed,
            )
        {
            return Err(Error::internal(
                "eval environment super capability disagrees with caller bytecode",
            ));
        }
        let first_function_anchor = environment
            .scopes
            .iter()
            .position(|scope| {
                matches!(
                    scope.kind,
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::Parameter
                )
            })
            .and_then(|scope| u16::try_from(scope).ok())
            .ok_or_else(|| {
                Error::internal(
                    "eval environment contains no representable current function anchor",
                )
            })?;
        match environment.variable_environment {
            EvalVariableEnvironment::Global => {
                let current_body_is_program = first_function_anchor
                    .checked_sub(1)
                    .and_then(|scope| environment.scopes.get(usize::from(scope)))
                    .is_some_and(|scope| scope.kind == crate::heap::EvalScopeKind::ProgramBody);
                if !current_body_is_program
                    || (caller_strict && caller_metadata.eval_kind != crate::heap::EvalKind::None)
                {
                    return Err(Error::internal(
                        "global eval variable environment escaped an authored Script root",
                    ));
                }
            }
            EvalVariableEnvironment::StrictLocal(scope) => {
                if !caller_strict {
                    return Err(Error::internal(
                        "sloppy eval environment selected a strict-local destination",
                    ));
                }
                if scope != first_function_anchor {
                    return Err(Error::internal(
                        "strict eval variable environment selected the wrong current function segment",
                    ));
                }
                let current_body_is_program = first_function_anchor
                    .checked_sub(1)
                    .and_then(|scope| environment.scopes.get(usize::from(scope)))
                    .is_some_and(|scope| scope.kind == crate::heap::EvalScopeKind::ProgramBody);
                if current_body_is_program
                    && caller_metadata.eval_kind == crate::heap::EvalKind::None
                {
                    return Err(Error::internal(
                        "authored Script eval environment used a non-canonical strict-local target",
                    ));
                }
                let Some(scope) = environment.scopes.get(usize::from(scope)) else {
                    return Err(Error::internal(
                        "eval variable-environment scope is out of bounds",
                    ));
                };
                if !matches!(
                    scope.kind,
                    crate::heap::EvalScopeKind::FunctionRoot
                        | crate::heap::EvalScopeKind::Parameter
                ) {
                    return Err(Error::internal(
                        "strict eval variable environment did not select a function anchor",
                    ));
                }
            }
            EvalVariableEnvironment::VariableObject { scope, source } => {
                if caller_strict || matches!(source, EvalBindingSource::Argument(_)) {
                    return Err(Error::internal(
                        "eval variable-object destination is not authentic",
                    ));
                }
                let target_matches_function_segment = if caller_metadata.eval_kind
                    == crate::heap::EvalKind::None
                {
                    scope == first_function_anchor && matches!(source, EvalBindingSource::Local(_))
                } else {
                    caller_metadata.eval_kind == crate::heap::EvalKind::Direct
                        && scope > first_function_anchor
                        && matches!(source, EvalBindingSource::Closure(_))
                };
                if !target_matches_function_segment {
                    return Err(Error::internal(
                        "eval variable object selected the wrong current function segment",
                    ));
                }
                let target_scope = environment.scopes.get(usize::from(scope)).ok_or_else(|| {
                    Error::internal("eval variable-object scope is out of bounds")
                })?;
                let expected_kind = match target_scope.kind {
                    crate::heap::EvalScopeKind::FunctionRoot => {
                        ClosureVariableKind::EvalVariableObject
                    }
                    crate::heap::EvalScopeKind::Parameter => {
                        ClosureVariableKind::ArgEvalVariableObject
                    }
                    _ => {
                        return Err(Error::internal(
                            "eval variable object selected a non-function scope",
                        ));
                    }
                };
                if target_scope
                    .bindings
                    .iter()
                    .filter(|binding| {
                        binding.source == source
                            && binding.kind == expected_kind
                            && !binding.is_lexical
                            && !binding.is_const
                            && !binding.is_catch_parameter
                    })
                    .count()
                    != 1
                {
                    return Err(Error::internal("eval variable-object target is not exact"));
                }
                match source {
                    EvalBindingSource::Local(index) => {
                        if self.eval_variable_object_local_kind(index) != Some(expected_kind) {
                            return Err(Error::internal(
                                "eval variable-object local role is not authentic",
                            ));
                        }
                        let definition = self.local_definition(index)?;
                        if definition.kind != expected_kind
                            || definition.is_lexical
                            || definition.is_const
                        {
                            return Err(Error::internal(
                                "eval variable-object local definition is malformed",
                            ));
                        }
                        self.locals.get(usize::from(index)).ok_or_else(|| {
                            Error::internal("eval variable-object local is out of bounds")
                        })?;
                    }
                    EvalBindingSource::Closure(index) => {
                        let descriptor = *self
                            .closure_variables
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                Error::internal("eval variable-object closure is out of bounds")
                            })?;
                        if descriptor.kind != expected_kind
                            || descriptor.is_lexical
                            || descriptor.is_const
                        {
                            return Err(Error::internal(
                                "eval variable-object closure descriptor is malformed",
                            ));
                        }
                        let root = self.closure_slots.get(usize::from(index)).ok_or_else(|| {
                            Error::internal("eval variable-object closure slot is out of bounds")
                        })?;
                        self.runtime
                            .validate_var_ref_metadata(root, descriptor)
                            .map_err(|error| Error::internal(error.to_string()))?;
                    }
                    EvalBindingSource::Argument(_) => unreachable!(
                        "argument variable-object source was rejected before validation"
                    ),
                }
            }
        }
        for scope in &environment.scopes {
            for binding in &scope.bindings {
                if binding.kind.is_eval_variable_object()
                    && match scope.kind {
                        crate::heap::EvalScopeKind::FunctionRoot => false,
                        crate::heap::EvalScopeKind::Parameter => {
                            binding.kind != ClosureVariableKind::ArgEvalVariableObject
                        }
                        _ => true,
                    }
                {
                    return Err(Error::internal(
                        "eval variable-object binding escaped its authenticated function anchor",
                    ));
                }
                match binding.source {
                    EvalBindingSource::Local(index) => {
                        let definition = self.local_definition(index)?;
                        Self::validate_eval_definition(definition, binding)?;
                        self.locals.get(usize::from(index)).ok_or_else(|| {
                            Error::internal("eval local binding index is out of bounds")
                        })?;
                    }
                    EvalBindingSource::Argument(index) => {
                        let definition = self.argument_definition(index)?;
                        Self::validate_eval_definition(definition, binding)?;
                        self.arguments.get(usize::from(index)).ok_or_else(|| {
                            Error::internal("eval argument binding index is out of bounds")
                        })?;
                    }
                    EvalBindingSource::Closure(index) => {
                        let descriptor = *self
                            .closure_variables
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                Error::internal("eval closure binding index is out of bounds")
                            })?;
                        Self::validate_eval_closure(descriptor, binding)?;
                        let root = self.closure_slots.get(usize::from(index)).ok_or_else(|| {
                            Error::internal("eval closure slot index is out of bounds")
                        })?;
                        self.runtime
                            .validate_var_ref_metadata(root, descriptor)
                            .map_err(|error| Error::internal(error.to_string()))?;
                    }
                }
            }
        }
        Ok(())
    }

    fn prepare_direct_eval_environment(
        &self,
        index: u16,
        caller_strict: bool,
    ) -> Result<PreparedEvalEnvironment, Error> {
        let descriptor = self
            .eval_environments
            .get(usize::from(index))
            .cloned()
            .ok_or_else(|| Error::internal("eval environment index is out of bounds"))?;
        let caller_bytecode = self.current_bytecode.clone().ok_or_else(|| {
            Error::internal("direct eval frame did not retain its caller bytecode")
        })?;
        let caller_metadata = self
            .runtime
            .snapshot_function_bytecode(&caller_bytecode)
            .map_err(runtime_error_to_vm_error)?
            .metadata;
        // Authenticate every immutable source before compilation. Corrupt
        // published bytecode must fail without compiling attacker-selected
        // names or converting any frame binding to a VarRef.
        self.validate_eval_environment(&descriptor, caller_strict, caller_metadata)?;
        Ok(PreparedEvalEnvironment {
            index,
            caller_bytecode,
            descriptor,
        })
    }

    fn materialize_direct_eval_environment(
        &mut self,
        prepared: PreparedEvalEnvironment,
    ) -> Result<MaterializedEvalEnvironment, Error> {
        let PreparedEvalEnvironment {
            index,
            caller_bytecode,
            descriptor,
        } = prepared;
        let binding_count = descriptor
            .scopes
            .iter()
            .map(|scope| scope.bindings.len())
            .sum();
        let mut roots = Vec::with_capacity(binding_count);
        for scope in &descriptor.scopes {
            for eval_binding in &scope.bindings {
                let root = match eval_binding.source {
                    EvalBindingSource::Local(binding_index) => {
                        let descriptor = Self::eval_capture_descriptor(eval_binding);
                        let binding =
                            self.locals
                                .get_mut(usize::from(binding_index))
                                .ok_or_else(|| {
                                    Error::internal("eval local binding index is out of bounds")
                                })?;
                        capture_frame_binding(&self.runtime, binding, descriptor)?
                    }
                    EvalBindingSource::Argument(binding_index) => {
                        let descriptor = Self::eval_capture_descriptor(eval_binding);
                        let binding = self
                            .arguments
                            .get_mut(usize::from(binding_index))
                            .ok_or_else(|| {
                                Error::internal("eval argument binding index is out of bounds")
                            })?;
                        capture_frame_binding(&self.runtime, binding, descriptor)?
                    }
                    EvalBindingSource::Closure(binding_index) => self
                        .closure_slots
                        .get(usize::from(binding_index))
                        .ok_or_else(|| Error::internal("eval closure slot index is out of bounds"))?
                        .clone(),
                };
                roots.push(root);
            }
        }
        Ok(MaterializedEvalEnvironment {
            index,
            _caller_bytecode: caller_bytecode,
            descriptor,
            roots: roots.into_boxed_slice(),
        })
    }

    fn lexical_uninitialized_error(&self, name: Option<Atom>) -> Result<Error, Error> {
        let Some(name) = name else {
            return Ok(Error::new(
                ErrorKind::Reference,
                "lexical variable is not initialized",
            ));
        };
        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), name)
            .map_err(|error| Error::internal(error.to_string()))?;
        self.runtime
            .native_atom_error(ErrorKind::Reference, "", &key, " is not initialized")
            .map_err(runtime_error_to_vm_error)
    }

    fn lexical_read_only_error(&self, name: Option<Atom>) -> Result<Error, Error> {
        let Some(name) = name else {
            return Ok(Error::new(ErrorKind::Type, "lexical variable is read-only"));
        };
        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), name)
            .map_err(|error| Error::internal(error.to_string()))?;
        self.runtime
            .native_atom_error(ErrorKind::Type, "'", &key, "' is read-only")
            .map_err(runtime_error_to_vm_error)
    }

    fn closure_name(&self, index: u16) -> Result<Option<Atom>, Error> {
        let descriptor = self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        Ok(match descriptor.name {
            ClosureVariableName::Atom(name) => Some(name),
            ClosureVariableName::None => None,
            ClosureVariableName::Constant(_) => {
                return Err(Error::internal(
                    "published closure descriptor retained an unlinked name constant",
                ));
            }
        })
    }

    fn constant_property_key(&self, index: u32) -> Result<PropertyKey, Error> {
        let name = match usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
        {
            Some(BytecodeConstant::Value(RawValue::String(name))) => name.clone(),
            Some(
                BytecodeConstant::Value(_)
                | BytecodeConstant::Function(_)
                | BytecodeConstant::RegExp { .. },
            ) => {
                return Err(Error::internal(
                    "field opcode referenced a non-string constant",
                ));
            }
            None => return Err(Error::internal("constant index is out of bounds")),
        };
        let key = self
            .runtime
            .intern_property_key_js_string(&name)
            .map_err(|error| Error::internal(error.to_string()))?;
        Ok(key)
    }

    fn eval_variable_object(&self, source: EvalVariableSource) -> Result<ObjectRef, Error> {
        let value = match source {
            EvalVariableSource::Local(index) => {
                let Some(expected_kind) = self.eval_variable_object_local_kind(index) else {
                    return Err(Error::internal(
                        "eval variable opcode referenced an unauthenticated local",
                    ));
                };
                let definition = self.local_definition(index)?;
                if definition.kind != expected_kind {
                    return Err(Error::internal(
                        "eval variable opcode referenced a non-variable-object local",
                    ));
                }
                let binding = self.locals.get(usize::from(index)).ok_or_else(|| {
                    Error::internal("eval variable-object local index is out of bounds")
                })?;
                if let FrameBinding::Captured(root) = binding {
                    self.runtime
                        .validate_var_ref_metadata(
                            root,
                            ClosureVariable {
                                source: ClosureSource::ParentLocal(index),
                                name: definition
                                    .name
                                    .map_or(ClosureVariableName::None, ClosureVariableName::Atom),
                                is_lexical: definition.is_lexical,
                                is_const: definition.is_const,
                                kind: definition.kind,
                            },
                        )
                        .map_err(runtime_error_to_vm_error)?;
                }
                read_frame_binding(&self.runtime, binding)?
            }
            EvalVariableSource::Closure(index) => {
                let descriptor = self
                    .closure_variables
                    .get(usize::from(index))
                    .copied()
                    .ok_or_else(|| {
                        Error::internal("eval variable-object closure index is out of bounds")
                    })?;
                if !descriptor.kind.is_eval_variable_object() {
                    return Err(Error::internal(
                        "eval variable opcode referenced a non-variable-object closure",
                    ));
                }
                let root = self.closure_slots.get(usize::from(index)).ok_or_else(|| {
                    Error::internal("eval variable-object closure slot is out of bounds")
                })?;
                self.runtime
                    .validate_var_ref_metadata(root, descriptor)
                    .map_err(runtime_error_to_vm_error)?;
                self.runtime
                    .read_var_ref(root)
                    .map_err(runtime_error_to_vm_error)?
            }
        };
        let Value::Object(object) = value else {
            return Err(Error::internal(
                "eval variable-object binding did not contain an Object",
            ));
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "eval variable object belongs to another runtime",
            ));
        }
        let state = self.runtime.0.state.borrow();
        let object_data = state
            .heap
            .object(object.object_id())
            .map_err(|error| Error::internal(error.to_string()))?;
        // Creation and publication authenticate an ordinary null-prototype
        // object. Once a syntactic-with method call exposes that receiver,
        // QuickJS lets user code mutate its prototype; later eval lookup must
        // therefore retain Ordinary branding without reasserting the initial
        // prototype shape.
        if !matches!(&object_data.payload, ObjectPayload::Ordinary) {
            return Err(Error::internal(
                "eval variable-object binding did not contain an ordinary Object",
            ));
        }
        drop(state);
        Ok(object)
    }

    /// QuickJS `JS_ValueToAtom` / `JS_ToPropertyKey` at the VM/runtime
    /// boundary. Object conversion can execute JavaScript and therefore keeps
    /// an ordinary thrown value distinct from an engine failure.
    fn property_key_from_value(
        &mut self,
        mut value: Value,
    ) -> Result<VmPropertyKeyConversion, Error> {
        if matches!(value, Value::Object(_)) {
            value = match self
                .runtime
                .to_primitive(self.current_realm, value, ToPrimitiveHint::String)
                .map_err(runtime_error_to_vm_error)?
            {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(VmPropertyKeyConversion::Throw(value)),
            };
        }

        let key = match value {
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(&self.runtime) {
                    return Err(Error::internal(
                        "computed property symbol belongs to another runtime",
                    ));
                }
                PropertyKey::from_borrowed_atom(self.runtime.clone(), symbol.atom())
                    .map_err(|error| Error::internal(error.to_string()))?
            }
            Value::String(string) => self
                .runtime
                .intern_property_key_js_string(&string)
                .map_err(|error| Error::internal(error.to_string()))?,
            value => {
                let string = value.to_js_string()?;
                self.runtime
                    .intern_property_key_js_string(&string)
                    .map_err(|error| Error::internal(error.to_string()))?
            }
        };
        Ok(VmPropertyKeyConversion::Key(key))
    }

    /// Convert the authenticated output of `ToPropKey` without invoking any
    /// user-observable coercion a second time.
    fn canonical_property_key_from_value(&self, value: &Value) -> Result<PropertyKey, Error> {
        match value {
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(&self.runtime) {
                    return Err(Error::internal(
                        "computed method symbol belongs to another runtime",
                    ));
                }
                PropertyKey::from_borrowed_atom(self.runtime.clone(), symbol.atom())
                    .map_err(|error| Error::internal(error.to_string()))
            }
            Value::String(string) => self
                .runtime
                .intern_property_key_js_string(string)
                .map_err(|error| Error::internal(error.to_string())),
            Value::Int(value) => self
                .runtime
                .intern_property_key_js_string(&Value::Int(*value).to_js_string()?)
                .map_err(|error| Error::internal(error.to_string())),
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Object(_) => Err(Error::internal(
                "computed method key was not canonicalized by ToPropKey",
            )),
        }
    }

    fn finish_property_get_action(
        &mut self,
        action: PropertyGetAction,
    ) -> Result<Completion, Error> {
        match action {
            PropertyGetAction::Complete(value) => Ok(Completion::Return(value)),
            PropertyGetAction::Call { getter, receiver } => self
                .runtime
                .call_internal(self.current_realm, &getter, receiver, &[])
                .map_err(runtime_error_to_vm_error),
        }
    }

    fn finish_property_set_action(
        &mut self,
        action: PropertySetAction,
        key: &PropertyKey,
        strict: bool,
    ) -> Result<Completion, Error> {
        match action {
            PropertySetAction::Complete => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Throw(value) => Ok(Completion::Throw(value)),
            PropertySetAction::Rejected(_) if !strict => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Rejected(PropertySetRejection::ReadOnly) => {
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Type, "'", key, "' is read-only")
                    .map_err(runtime_error_to_vm_error)?;
                Err(error)
            }
            PropertySetAction::Rejected(PropertySetRejection::ArrayLengthReadOnly) => {
                let length = self
                    .runtime
                    .intern_property_key("length")
                    .map_err(|error| Error::internal(error.to_string()))?;
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")
                    .map_err(runtime_error_to_vm_error)?;
                Err(error)
            }
            PropertySetAction::Rejected(PropertySetRejection::NotConfigurable) => {
                Err(Error::new(ErrorKind::Type, "not configurable"))
            }
            PropertySetAction::Rejected(PropertySetRejection::NoSetter) => {
                Err(Error::new(ErrorKind::Type, "no setter for property"))
            }
            PropertySetAction::Rejected(PropertySetRejection::NotExtensible) => {
                Err(Error::new(ErrorKind::Type, "object is not extensible"))
            }
            PropertySetAction::Rejected(PropertySetRejection::NotObject) => {
                Err(Error::new(ErrorKind::Type, "not an object"))
            }
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => self
                .runtime
                .call_internal(self.current_realm, &setter, receiver, &[argument])
                .map_err(runtime_error_to_vm_error),
        }
    }

    fn get_property_with_key(
        &mut self,
        base: Value,
        key: &PropertyKey,
        static_name: bool,
    ) -> Result<Completion, Error> {
        match &base {
            Value::Null | Value::Undefined => {
                let base_name = if matches!(base, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                if static_name {
                    let suffix = if matches!(base, Value::Null) {
                        "' of null"
                    } else {
                        "' of undefined"
                    };
                    let error = self
                        .runtime
                        .native_atom_error(ErrorKind::Type, "cannot read property '", key, suffix)
                        .map_err(runtime_error_to_vm_error)?;
                    Err(error)
                } else {
                    Err(Error::new(
                        ErrorKind::Type,
                        format!("cannot read property of {base_name}"),
                    ))
                }
            }
            Value::Object(object) => {
                let action = self
                    .runtime
                    .prepare_get_property_with_receiver(object, key, base.clone())
                    .map_err(runtime_error_to_vm_error)?;
                self.finish_property_get_action(action)
            }
            Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Symbol(_) => {
                let kind = match &base {
                    Value::Bool(_) => PrimitiveKind::Boolean,
                    Value::Int(_) | Value::Float(_) => PrimitiveKind::Number,
                    Value::BigInt(_) => PrimitiveKind::BigInt,
                    Value::Symbol(_) => PrimitiveKind::Symbol,
                    _ => unreachable!(),
                };
                let prototype = self
                    .runtime
                    .primitive_prototype_for_realm(self.current_realm, kind)
                    .map_err(runtime_error_to_vm_error)?;
                let action = self
                    .runtime
                    .prepare_get_property_with_receiver(&prototype, key, base.clone())
                    .map_err(runtime_error_to_vm_error)?;
                self.finish_property_get_action(action)
            }
            Value::String(string) => {
                let action = self
                    .runtime
                    .prepare_get_string_property_with_receiver(
                        self.current_realm,
                        string,
                        key,
                        base.clone(),
                    )
                    .map_err(runtime_error_to_vm_error)?;
                self.finish_property_get_action(action)
            }
        }
    }

    fn set_property_with_key(
        &mut self,
        base: Value,
        key: &PropertyKey,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        let action = match &base {
            Value::Object(object) => self
                .runtime
                .prepare_set_property_with_receiver_in_realm(
                    Some(self.current_realm),
                    object,
                    key,
                    value,
                    base.clone(),
                )
                .map_err(runtime_error_to_vm_error)?,
            Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Symbol(_) => {
                let kind = match &base {
                    Value::Bool(_) => PrimitiveKind::Boolean,
                    Value::Int(_) | Value::Float(_) => PrimitiveKind::Number,
                    Value::BigInt(_) => PrimitiveKind::BigInt,
                    Value::Symbol(_) => PrimitiveKind::Symbol,
                    _ => unreachable!(),
                };
                let prototype = self
                    .runtime
                    .primitive_prototype_for_realm(self.current_realm, kind)
                    .map_err(runtime_error_to_vm_error)?;
                self.runtime
                    .prepare_set_property_with_receiver_in_realm(
                        Some(self.current_realm),
                        &prototype,
                        key,
                        value,
                        base.clone(),
                    )
                    .map_err(runtime_error_to_vm_error)?
            }
            Value::Null | Value::Undefined => {
                let suffix = if matches!(base, Value::Null) {
                    "' of null"
                } else {
                    "' of undefined"
                };
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Type, "cannot set property '", key, suffix)
                    .map_err(runtime_error_to_vm_error)?;
                return Err(error);
            }
            Value::String(_) => {
                // Primitive String [[Set]] walks the realm's class prototype
                // with the raw receiver. The virtual character indices are a
                // boxing/get-own concern, so absent an inherited setter their
                // strict assignment still reports `not an object`; the real
                // non-writable prototype `length` reports read-only.
                let prototype = self
                    .runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::String)
                    .map_err(runtime_error_to_vm_error)?;
                self.runtime
                    .prepare_set_property_with_receiver_in_realm(
                        Some(self.current_realm),
                        &prototype,
                        key,
                        value,
                        base.clone(),
                    )
                    .map_err(runtime_error_to_vm_error)?
            }
        };
        self.finish_property_set_action(action, key, strict)
    }

    fn delete_property_with_key(
        &mut self,
        base: Value,
        key: &PropertyKey,
        strict: bool,
    ) -> Result<Completion, Error> {
        let deleted = match &base {
            Value::Null | Value::Undefined => {
                return Err(Error::new(ErrorKind::Type, "cannot convert to object"));
            }
            Value::Object(object) => self
                .runtime
                .delete_property(object, key)
                .map_err(runtime_error_to_vm_error)?,
            Value::String(string) => {
                let index = self
                    .runtime
                    .0
                    .state
                    .borrow()
                    .atoms
                    .array_index(key.atom())
                    .map_err(|error| Error::internal(error.to_string()))?;
                let indexed = index.is_some_and(|index| {
                    usize::try_from(index).is_ok_and(|index| index < string.len())
                });
                let length = self
                    .runtime
                    .intern_property_key("length")
                    .map_err(|error| Error::internal(error.to_string()))?;
                !indexed && key != &length
            }
            Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::Symbol(_) => true,
        };
        if !deleted && strict {
            return Err(Error::new(ErrorKind::Type, "could not delete property"));
        }
        Ok(Completion::Return(Value::Bool(deleted)))
    }

    /// Convert only JavaScript-visible engine errors into rooted thrown
    /// values. Arena/domain invariants remain Rust errors and must never be
    /// swallowed by IteratorClose's exception-precedence rule.
    fn materialize_iterator_error(&self, error: Error) -> Result<Value, Error> {
        let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
            return Err(error);
        };
        self.runtime
            .new_native_error_from_error(self.current_realm, kind, &error)
            .map_err(runtime_error_to_vm_error)
    }

    fn iterator_type_error(&self, message: &str) -> Result<Value, Error> {
        self.runtime
            .new_native_error(self.current_realm, NativeErrorKind::Type, message)
            .map_err(runtime_error_to_vm_error)
    }

    fn iterator_callable(&self, value: Value) -> Result<Option<CallableRef>, Error> {
        let Value::Object(object) = value else {
            return Ok(None);
        };
        self.runtime
            .as_callable(&object)
            .map_err(runtime_error_to_vm_error)
    }

    fn is_direct_native_target(
        &self,
        value: &Value,
        expected: NativeFunctionId,
    ) -> Result<bool, Error> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "append iterator method belongs to another runtime",
            ));
        }
        let state = self.runtime.0.state.borrow();
        let object = state
            .heap
            .object(object.object_id())
            .map_err(|error| Error::internal(error.to_string()))?;
        Ok(matches!(
            &object.payload,
            ObjectPayload::NativeFunction { data } if data.target == expected
        ))
    }

    /// Snapshot the exact values used by QuickJS's `js_append_enumerate`
    /// fast branch. Named properties may be interleaved in our shape, so the
    /// shared fast Array/Arguments storage reader reconstructs numeric order
    /// rather than slicing physical slots.
    fn append_fast_array_values(
        &self,
        source: &Value,
        next_method: &Value,
        builtin_values_probe: bool,
    ) -> Result<Option<Vec<Value>>, Error> {
        if !builtin_values_probe
            || !self.is_direct_native_target(next_method, NativeFunctionId::ArrayIteratorNext)?
        {
            return Ok(None);
        }
        let Value::Object(source) = source else {
            return Ok(None);
        };
        let is_array = {
            let state = self.runtime.0.state.borrow();
            matches!(
                &state
                    .heap
                    .object(source.object_id())
                    .map_err(|error| Error::internal(error.to_string()))?
                    .payload,
                ObjectPayload::Array { .. }
            )
        };
        if !is_array {
            return Ok(None);
        }
        let fast_len = self
            .runtime
            .array_fast_len(source)
            .map_err(runtime_error_to_vm_error)?;
        let Some(fast_len) = fast_len else {
            return Ok(None);
        };
        let (length, _) = self
            .runtime
            .array_length_state(source)
            .map_err(runtime_error_to_vm_error)?;
        if length != fast_len {
            return Ok(None);
        }

        self.runtime
            .fast_array_like_values(source, fast_len)
            .map_err(runtime_error_to_vm_error)
    }

    fn call_iterator_method(
        &self,
        callable: &CallableRef,
        receiver: Value,
    ) -> Result<Completion, Error> {
        self.runtime
            .call_internal(self.current_realm, callable, receiver, &[])
            .map_err(runtime_error_to_vm_error)
    }

    fn take_for_in_exception(&self) -> Result<Value, Error> {
        self.runtime
            .take_pending_exception()
            .map_err(runtime_error_to_vm_error)?
            .ok_or_else(|| Error::internal("for-in operation lost its JavaScript exception"))
    }
}

impl Runtime {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn execute_bytecode_callable(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        this_value: Value,
        new_target: Value,
        arguments: &[Value],
        bytecode: FunctionBytecodeRef,
        closure_slots: Vec<VarRefRoot>,
    ) -> Result<Completion, RuntimeError> {
        if self.bytecode_call_would_overflow() {
            return Ok(Completion::Throw(self.new_native_error(
                caller_realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let PublishedFunctionSnapshot {
            root,
            code,
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            arg_eval_variable_object_local,
            metadata,
            realm,
        } = self.snapshot_function_bytecode(&bytecode)?;
        let callee_global = self.global_object_for_realm(realm)?;
        let active_frame = self.push_bytecode_active_frame(
            callable.as_object().clone(),
            root.clone(),
            realm,
            metadata.strict,
        )?;
        let argument_slots = arguments.len().max(usize::from(metadata.argument_count));
        let mut frame_arguments = Vec::with_capacity(argument_slots);
        frame_arguments.extend(arguments.iter().cloned().map(FrameBinding::Direct));
        frame_arguments.resize_with(argument_slots, || FrameBinding::Direct(Value::Undefined));
        let mut frame_locals = local_definitions
            .iter()
            .map(|definition| {
                if definition.is_lexical {
                    FrameBinding::Uninitialized
                } else {
                    FrameBinding::Direct(Value::Undefined)
                }
            })
            .collect::<Vec<_>>();
        if let Some(index) = metadata.function_name_local {
            let binding =
                frame_locals
                    .get_mut(usize::from(index))
                    .ok_or(RuntimeError::Invariant(
                        "function-name local is outside the frame",
                    ))?;
            *binding = FrameBinding::Direct(Value::Object(callable.as_object().clone()));
        }
        let frame_local_count = frame_locals.len();
        let mut host = RuntimeVmHost {
            runtime: self.clone(),
            active_frame_token: active_frame.token(),
            current_realm: realm,
            current_bytecode: Some(root),
            current_function: Some(callable.as_object().clone()),
            actual_argument_count: arguments.len(),
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            eval_variable_object_local: metadata.eval_variable_object_local,
            arg_eval_variable_object_local,
            closure_slots,
            arguments: frame_arguments,
            locals: frame_locals,
            reusable_captured_locals: vec![false; frame_local_count],
        };
        let result = Vm::new().execute_published(
            CallInput {
                code: &code,
                metadata,
                caller_realm,
                callee_realm: realm,
                current_function: callable.as_object().clone(),
                this_value,
                new_target,
                callee_global,
            },
            &mut host,
        );
        active_frame.finish()?;
        result.map_err(RuntimeError::Engine)
    }
}

impl VmHost for RuntimeVmHost {
    fn update_active_bytecode_pc(&mut self, pc: BytecodePc) -> Result<(), Error> {
        self.runtime
            .update_active_bytecode_pc(self.active_frame_token, pc)
            .map_err(runtime_error_to_vm_error)
    }

    fn ensure_backtrace(&mut self, value: &Value) -> Result<(), Error> {
        self.runtime
            .ensure_error_backtrace(value, false, None)
            .map_err(runtime_error_to_vm_error)
    }

    fn prepare_captured_local_reuse(&mut self) -> Result<(), Error> {
        if self.reusable_captured_locals.len() != self.locals.len() {
            return Err(Error::internal(
                "reusable captured-local flags disagree with the frame",
            ));
        }
        for (reusable, binding) in self.reusable_captured_locals.iter_mut().zip(&self.locals) {
            *reusable = matches!(binding, FrameBinding::Captured(_));
        }
        Ok(())
    }

    fn for_in_start(&mut self, value: Value) -> Result<ForInStartOutcome, Error> {
        match self.runtime.start_for_in(self.current_realm, value) {
            Ok(iterator) => Ok(ForInStartOutcome::Iterator(Value::Object(iterator))),
            Err(RuntimeError::Exception) => {
                Ok(ForInStartOutcome::Throw(self.take_for_in_exception()?))
            }
            Err(error) => Err(runtime_error_to_vm_error(error)),
        }
    }

    fn for_in_next(&mut self, iterator: Value) -> Result<ForInNextOutcome, Error> {
        let Value::Object(iterator) = iterator else {
            return Ok(ForInNextOutcome::Result {
                value: Value::Undefined,
                done: true,
            });
        };
        let is_for_in = matches!(
            self.runtime
                .0
                .state
                .borrow()
                .heap
                .object(iterator.object_id())
                .map_err(|error| Error::internal(error.to_string()))?
                .payload,
            ObjectPayload::ForInIterator(_)
        );
        if !is_for_in {
            return Ok(ForInNextOutcome::Result {
                value: Value::Undefined,
                done: true,
            });
        }
        match self.runtime.next_for_in(&iterator) {
            Ok((value, done)) => Ok(ForInNextOutcome::Result { value, done }),
            Err(RuntimeError::Exception) => {
                Ok(ForInNextOutcome::Throw(self.take_for_in_exception()?))
            }
            Err(error) => Err(runtime_error_to_vm_error(error)),
        }
    }

    fn for_of_start(&mut self, iterable: Value) -> Result<ForOfStartOutcome, Error> {
        let iterator_key =
            PropertyKey::from(self.runtime.well_known_symbol(WellKnownSymbol::Iterator));
        let method = match self.get_property_with_key(iterable.clone(), &iterator_key, false) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(ForOfStartOutcome::Throw(value)),
            Err(error) => {
                return Ok(ForOfStartOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        let Some(method) = self.iterator_callable(method)? else {
            return Ok(ForOfStartOutcome::Throw(
                self.iterator_type_error("value is not iterable")?,
            ));
        };
        let iterator = match self.call_iterator_method(&method, iterable) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(ForOfStartOutcome::Throw(value)),
            Err(error) => {
                return Ok(ForOfStartOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        if !matches!(iterator, Value::Object(_)) {
            return Ok(ForOfStartOutcome::Throw(
                self.iterator_type_error("not an object")?,
            ));
        }

        // Cache `next` exactly once when the iterator record is created.
        // Subsequent mutation or accessors on the iterator's property cannot
        // change the method used by ForOfNext.
        let next_key = self
            .runtime
            .intern_property_key("next")
            .map_err(|error| Error::internal(error.to_string()))?;
        let next_method = match self.get_property_with_key(iterator.clone(), &next_key, false) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(ForOfStartOutcome::Throw(value)),
            Err(error) => {
                return Ok(ForOfStartOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        Ok(ForOfStartOutcome::Record {
            iterator,
            next_method,
        })
    }

    fn append_start(&mut self, iterable: Value) -> Result<AppendStartOutcome, Error> {
        // QuickJS first performs an otherwise redundant Get for its native
        // Array-values fast-path classification. The value is released before
        // the ordinary GetIterator performs its own observable Get.
        let iterator_key =
            PropertyKey::from(self.runtime.well_known_symbol(WellKnownSymbol::Iterator));
        let probe = match self.get_property_with_key(iterable.clone(), &iterator_key, false) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(AppendStartOutcome::Throw(value)),
            Err(error) => {
                return Ok(AppendStartOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        let builtin_values_probe = self.is_direct_native_target(
            &probe,
            NativeFunctionId::ArrayPrototypeIterator(ArrayIteratorKind::Value),
        )?;
        drop(probe);

        let (iterator, next_method) = match self.for_of_start(iterable.clone())? {
            ForOfStartOutcome::Record {
                iterator,
                next_method,
            } => (iterator, next_method),
            ForOfStartOutcome::Throw(value) => return Ok(AppendStartOutcome::Throw(value)),
        };
        let fast_values =
            self.append_fast_array_values(&iterable, &next_method, builtin_values_probe)?;
        Ok(AppendStartOutcome::Record {
            iterator,
            next_method,
            fast_values,
        })
    }

    fn for_of_next(
        &mut self,
        iterator: Value,
        next_method: Value,
    ) -> Result<ForOfNextOutcome, Error> {
        let Some(next_method) = self.iterator_callable(next_method)? else {
            return Ok(ForOfNextOutcome::Throw(
                self.iterator_type_error("not a function")?,
            ));
        };
        let result = match self
            .runtime
            .try_call_native_iterator_next_raw(&next_method, iterator.clone())
            .map_err(runtime_error_to_vm_error)?
        {
            Some(NativeInvokeOutcome::IteratorNextRaw { value, done }) => {
                return Ok(ForOfNextOutcome::Result {
                    value: if done { Value::Undefined } else { value },
                    done,
                });
            }
            Some(NativeInvokeOutcome::Completion(Completion::Throw(value))) => {
                return Ok(ForOfNextOutcome::Throw(value));
            }
            Some(NativeInvokeOutcome::Completion(Completion::Return(result))) => result,
            None => match self.call_iterator_method(&next_method, iterator) {
                Ok(Completion::Return(value)) => value,
                Ok(Completion::Throw(value)) => return Ok(ForOfNextOutcome::Throw(value)),
                Err(error) => {
                    return Ok(ForOfNextOutcome::Throw(
                        self.materialize_iterator_error(error)?,
                    ));
                }
            },
        };
        if !matches!(result, Value::Object(_)) {
            return Ok(ForOfNextOutcome::Throw(
                self.iterator_type_error("iterator must return an object")?,
            ));
        }

        let done_key = self
            .runtime
            .intern_property_key("done")
            .map_err(|error| Error::internal(error.to_string()))?;
        let done = match self.get_property_with_key(result.clone(), &done_key, false) {
            Ok(Completion::Return(value)) => value.to_boolean(),
            Ok(Completion::Throw(value)) => return Ok(ForOfNextOutcome::Throw(value)),
            Err(error) => {
                return Ok(ForOfNextOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        if done {
            // QuickJS deliberately does not Get `value` for a completed
            // iterator result, so a getter there remains unobserved.
            return Ok(ForOfNextOutcome::Result {
                value: Value::Undefined,
                done: true,
            });
        }

        let value_key = self
            .runtime
            .intern_property_key("value")
            .map_err(|error| Error::internal(error.to_string()))?;
        let value = match self.get_property_with_key(result, &value_key, false) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(ForOfNextOutcome::Throw(value)),
            Err(error) => {
                return Ok(ForOfNextOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        Ok(ForOfNextOutcome::Result { value, done: false })
    }

    fn iterator_close(
        &mut self,
        iterator: Value,
        exception_pending: bool,
    ) -> Result<IteratorCloseOutcome, Error> {
        let return_key = self
            .runtime
            .intern_property_key("return")
            .map_err(|error| Error::internal(error.to_string()))?;
        let method = match self.get_property_with_key(iterator.clone(), &return_key, false) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(IteratorCloseOutcome::Throw(value)),
            Err(error) => {
                return Ok(IteratorCloseOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(IteratorCloseOutcome::Closed);
        }
        let Some(method) = self.iterator_callable(method)? else {
            return Ok(IteratorCloseOutcome::Throw(
                self.iterator_type_error("not a function")?,
            ));
        };
        let result = match self.call_iterator_method(&method, iterator) {
            Ok(Completion::Return(value)) => value,
            Ok(Completion::Throw(value)) => return Ok(IteratorCloseOutcome::Throw(value)),
            Err(error) => {
                return Ok(IteratorCloseOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };
        // QuickJS deliberately skips the iterator-result Object check while
        // an earlier exception is pending. Getter/call/non-callable failures
        // still occur above so the VM can preserve the original completion,
        // but a normally returned primitive must not synthesize a new
        // TypeError on the pending-exception path.
        if exception_pending {
            return Ok(IteratorCloseOutcome::Closed);
        }
        if !matches!(result, Value::Object(_)) {
            return Ok(IteratorCloseOutcome::Throw(
                self.iterator_type_error("not an object")?,
            ));
        }
        Ok(IteratorCloseOutcome::Closed)
    }

    fn load_constant(&mut self, index: u32) -> Result<Value, Error> {
        let constant = usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
            .ok_or_else(|| Error::internal("constant index is out of bounds"))?;
        match constant {
            BytecodeConstant::Value(value) => self
                .runtime
                .root_raw_value(value)
                .map_err(|error| Error::internal(error.to_string())),
            BytecodeConstant::Function(_) => Err(Error::internal(
                "child function bytecode was loaded with a value-constant opcode",
            )),
            BytecodeConstant::RegExp { .. } => Err(Error::internal(
                "RegExp program was loaded with a value-constant opcode",
            )),
        }
    }

    fn read_only_error(&mut self, index: u32) -> Result<Error, Error> {
        let key = self.constant_property_key(index)?;
        self.runtime
            .native_atom_error(ErrorKind::Type, "'", &key, "' is read-only")
            .map_err(runtime_error_to_vm_error)
    }

    fn redeclaration_error(&mut self, index: u32) -> Result<Error, Error> {
        let key = self.constant_property_key(index)?;
        self.runtime
            .native_atom_error(ErrorKind::Syntax, "redeclaration of '", &key, "'")
            .map_err(runtime_error_to_vm_error)
    }

    fn type_of(&mut self, value: &Value) -> Result<&'static str, Error> {
        let Value::Object(object) = value else {
            return Ok(value.type_of());
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal("typeof operand belongs to another runtime"));
        }
        let state = self.runtime.0.state.borrow();
        let object = state
            .heap
            .object(object.object_id())
            .map_err(|error| Error::internal(error.to_string()))?;
        Ok(match &object.payload {
            ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => "function",
            ObjectPayload::Ordinary
            | ObjectPayload::RawJson
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. } => "object",
        })
    }

    fn box_primitive(&mut self, value: Value) -> Result<Value, Error> {
        let (kind, prototype) = match &value {
            Value::Bool(_) => (
                PrimitiveKind::Boolean,
                self.runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::Boolean)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            Value::Int(_) | Value::Float(_) => (
                PrimitiveKind::Number,
                self.runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::Number)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            Value::String(_) => (
                PrimitiveKind::String,
                self.runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::String)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            Value::BigInt(_) => (
                PrimitiveKind::BigInt,
                self.runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::BigInt)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            Value::Symbol(_) => (
                PrimitiveKind::Symbol,
                self.runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::Symbol)
                    .map_err(runtime_error_to_vm_error)?,
            ),
            Value::Undefined | Value::Null | Value::Object(_) => {
                return Err(Error::internal(
                    "primitive wrapper class is not implemented yet",
                ));
            }
        };
        self.runtime
            .new_primitive_object(&prototype, kind, value)
            .map(Value::Object)
            .map_err(runtime_error_to_vm_error)
    }

    fn to_primitive(&mut self, value: Value, hint: ToPrimitiveHint) -> Result<Completion, Error> {
        self.runtime
            .to_primitive(self.current_realm, value, hint)
            .map_err(runtime_error_to_vm_error)
    }

    fn materialize_error(&mut self, error: Error) -> Result<Value, Error> {
        let kind = NativeErrorKind::from_javascript_error(error.kind()).ok_or_else(|| {
            Error::internal("engine fault reached JavaScript error materialization")
        })?;
        self.runtime
            .new_native_error_from_error(self.current_realm, kind, &error)
            .map_err(runtime_error_to_vm_error)
    }

    fn instantiate_closure(&mut self, index: u32) -> Result<Value, Error> {
        let constant = usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
            .ok_or_else(|| Error::internal("constant index is out of bounds"))?;
        let BytecodeConstant::Function(bytecode) = constant else {
            return Err(Error::internal(
                "function-closure opcode referenced a value constant",
            ));
        };
        let child_id = *bytecode;
        let closure_variables = {
            let state = self.runtime.0.state.borrow();
            let child = state
                .heap
                .function_bytecode(child_id)
                .map_err(|error| Error::internal(error.to_string()))?;
            child.closure_variables.clone()
        };
        let bytecode = FunctionBytecodeRef::from_borrowed_handle(self.runtime.clone(), child_id)
            .map_err(|error| Error::internal(error.to_string()))?;
        let mut captured = Vec::with_capacity(closure_variables.len());
        for descriptor in closure_variables.iter().copied() {
            let root = match descriptor.source {
                ClosureSource::ParentLocal(index) => {
                    let definition = self.local_definition(index)?;
                    self.validate_capture_definition(definition, descriptor)?;
                    let binding = self
                        .locals
                        .get_mut(usize::from(index))
                        .ok_or_else(|| Error::internal("captured local index is out of bounds"))?;
                    capture_frame_binding(
                        &self.runtime,
                        binding,
                        ClosureVariable {
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            kind: definition.kind,
                            ..descriptor
                        },
                    )?
                }
                ClosureSource::ParentArgument(index) => {
                    let definition = self.argument_definition(index)?;
                    self.validate_capture_definition(definition, descriptor)?;
                    let binding = self.arguments.get_mut(usize::from(index)).ok_or_else(|| {
                        Error::internal("captured argument index is out of bounds")
                    })?;
                    capture_frame_binding(&self.runtime, binding, descriptor)?
                }
                ClosureSource::ParentClosure(index) => {
                    let root = self.closure_slots.get(usize::from(index)).ok_or_else(|| {
                        Error::internal("captured parent closure index is out of bounds")
                    })?;
                    self.runtime
                        .validate_var_ref_metadata(root, descriptor)
                        .map_err(|error| Error::internal(error.to_string()))?;
                    root.clone()
                }
                ClosureSource::ParentGlobal(index) => self
                    .closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| {
                        Error::internal("relayed parent global closure index is out of bounds")
                    })?
                    .clone(),
                ClosureSource::GlobalDeclaration | ClosureSource::Global => {
                    return Err(Error::internal(
                        "child closure attempted to resolve a root global descriptor",
                    ));
                }
                ClosureSource::EvalEnvironment(_) => {
                    return Err(Error::internal(
                        "child closure attempted to resolve an eval-root descriptor",
                    ));
                }
            };
            captured.push(root);
        }
        let callable = self
            .runtime
            .new_bytecode_closure_with_slots(self.current_realm, &bytecode, &captured)
            .map_err(|error| Error::internal(error.to_string()))?;
        Ok(Value::Object(callable.into_object()))
    }

    fn set_function_name(&mut self, value: &Value, name_index: u32) -> Result<(), Error> {
        let constant = usize::try_from(name_index)
            .ok()
            .and_then(|index| self.constants.get(index))
            .ok_or_else(|| Error::internal("function-name constant index is out of bounds"))?;
        let BytecodeConstant::Value(RawValue::String(name)) = constant else {
            return Err(Error::internal(
                "function-name opcode referenced a non-string constant",
            ));
        };
        self.runtime
            .define_object_name(value, name)
            .map_err(runtime_error_to_vm_error)
    }

    fn set_function_name_computed(&mut self, value: &Value, key: &Value) -> Result<(), Error> {
        // `OP_to_propkey` has already canonicalized this operand. In
        // particular, do not execute object conversion a second time here.
        let name = match key {
            Value::Int(_) => key.to_js_string()?,
            Value::String(name) => name.clone(),
            Value::Symbol(symbol) => match self
                .runtime
                .symbol_description(symbol)
                .map_err(runtime_error_to_vm_error)?
            {
                None => JsString::from_static(""),
                Some(description) => JsString::from_static("[")
                    .try_concat(&description)?
                    .try_concat(&JsString::from_static("]"))?,
            },
            _ => {
                return Err(Error::internal(
                    "computed function name was not a canonical property key",
                ));
            }
        };
        self.runtime
            .define_object_name(value, &name)
            .map_err(runtime_error_to_vm_error)
    }

    fn create_arguments(&mut self, kind: ArgumentsKind) -> Result<Completion, Error> {
        if self.actual_argument_count > self.arguments.len() {
            return Err(Error::internal(
                "actual argument count exceeds the padded argument frame",
            ));
        }
        let object = match kind {
            ArgumentsKind::Mapped => {
                let current_function = self
                    .current_function
                    .clone()
                    .ok_or_else(|| Error::internal("arguments creation has no current function"))?;
                let mut roots = Vec::with_capacity(self.arguments.len());
                for (index, binding) in self.arguments.iter_mut().enumerate() {
                    let index = u16::try_from(index)
                        .map_err(|_| Error::internal("argument index exceeds u16::MAX"))?;
                    roots.push(capture_frame_binding(
                        &self.runtime,
                        binding,
                        ClosureVariable {
                            source: ClosureSource::ParentArgument(index),
                            name: ClosureVariableName::None,
                            is_lexical: false,
                            is_const: false,
                            kind: ClosureVariableKind::Normal,
                        },
                    )?);
                }
                roots.truncate(self.actual_argument_count);
                self.runtime.new_mapped_arguments_object(
                    self.current_realm,
                    &current_function,
                    roots,
                )
            }
            ArgumentsKind::Unmapped => {
                let values = self
                    .arguments
                    .iter()
                    .take(self.actual_argument_count)
                    .map(|binding| read_frame_binding(&self.runtime, binding))
                    .collect::<Result<Vec<_>, _>>()?;
                self.runtime
                    .new_unmapped_arguments_object(self.current_realm, values)
            }
        }
        .map_err(runtime_error_to_vm_error)?;
        Ok(Completion::Return(Value::Object(object)))
    }

    fn create_rest(&mut self, start: u16) -> Result<Completion, Error> {
        let start = usize::from(start);
        if start > self.arguments.len() || self.actual_argument_count > self.arguments.len() {
            return Err(Error::internal(
                "rest parameter start exceeds the active argument frame",
            ));
        }
        let values = self
            .arguments
            .iter()
            .take(self.actual_argument_count)
            .skip(start)
            .map(|binding| read_frame_binding(&self.runtime, binding))
            .collect::<Result<Vec<_>, _>>()?;
        self.runtime
            .new_array_from_values(self.current_realm, values)
            .map(|array| Completion::Return(Value::Object(array)))
            .map_err(runtime_error_to_vm_error)
    }

    fn object(&mut self) -> Result<Completion, Error> {
        self.runtime
            .new_ordinary_object_in_realm(self.current_realm)
            .map(|object| Completion::Return(Value::Object(object)))
            .map_err(runtime_error_to_vm_error)
    }

    fn home_object(&mut self) -> Result<Value, Error> {
        self.active_home_object()
    }

    fn get_super(&mut self, home_object: Value) -> Result<Value, Error> {
        self.resolve_super_base(home_object)
    }

    fn create_variable_environment(&mut self) -> Result<Completion, Error> {
        if self.eval_variable_object_local.is_none()
            && self.arg_eval_variable_object_local.is_none()
        {
            return Err(Error::internal(
                "variable-environment creation has no authenticated local",
            ));
        }
        self.runtime
            .new_object(None)
            .map(|object| Completion::Return(Value::Object(object)))
            .map_err(runtime_error_to_vm_error)
    }

    fn has_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.eval_variable_object(source)?;
        let key = self.constant_property_key(name)?;
        self.runtime
            .has_property(&object, &key)
            .map(|exists| Completion::Return(Value::Bool(exists)))
            .map_err(runtime_error_to_vm_error)
    }

    fn get_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.eval_variable_object(source)?;
        let key = self.constant_property_key(name)?;
        self.get_property_with_key(Value::Object(object), &key, true)
    }

    fn put_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        let object = self.eval_variable_object(source)?;
        let key = self.constant_property_key(name)?;
        self.set_property_with_key(Value::Object(object), &key, value, false)
    }

    fn delete_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.eval_variable_object(source)?;
        let key = self.constant_property_key(name)?;
        self.delete_property_with_key(Value::Object(object), &key, false)
    }

    fn define_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        let object = self.eval_variable_object(source)?;
        let key = self.constant_property_key(name)?;
        let result = self.runtime.define_own_property_in_realm(
            Some(self.current_realm),
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        );
        self.finish_property_define(result)
    }

    fn has_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        self.has_dynamic_binding_impl(source, name)
    }

    fn get_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        self.get_dynamic_binding_impl(source, name, strict)
    }

    fn put_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        self.put_dynamic_binding_impl(source, name, value, strict)
    }

    fn delete_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        self.delete_dynamic_binding_impl(source, name)
    }

    fn dynamic_environment_object(
        &mut self,
        source: DynamicEnvironmentSource,
    ) -> Result<Completion, Error> {
        self.dynamic_environment_object_impl(source)
    }

    fn global_reference(&mut self, index: u16) -> Result<Completion, Error> {
        self.global_reference_impl(index)
    }

    fn get_ref_value(
        &mut self,
        environment: Value,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        self.get_ref_value_impl(environment, name, strict)
    }

    fn put_ref_value(
        &mut self,
        environment: Value,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        self.put_ref_value_impl(environment, name, value, strict)
    }

    fn create_regexp(&mut self, index: u32) -> Result<Completion, Error> {
        let (pattern, program) = match usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
        {
            Some(BytecodeConstant::RegExp { pattern, program }) => {
                (pattern.clone(), program.clone())
            }
            Some(BytecodeConstant::Value(_) | BytecodeConstant::Function(_)) => {
                return Err(Error::internal(
                    "RegExp opcode referenced a non-RegExp constant",
                ));
            }
            None => return Err(Error::internal("constant index is out of bounds")),
        };
        self.runtime
            .new_compiled_regexp_literal(self.current_realm, pattern, program)
            .map(|object| Completion::Return(Value::Object(object)))
            .map_err(runtime_error_to_vm_error)
    }

    fn array_from(&mut self, elements: Vec<Value>) -> Result<Completion, Error> {
        self.runtime
            .new_array_from_values(self.current_realm, elements)
            .map(|array| Completion::Return(Value::Object(array)))
            .map_err(runtime_error_to_vm_error)
    }

    fn define_field(
        &mut self,
        base: Value,
        key_index: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = base else {
            return Err(Error::new(ErrorKind::Type, "not an object"));
        };
        let key = self.constant_property_key(key_index)?;
        let result = self.runtime.define_own_property_in_realm(
            Some(self.current_realm),
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        );
        self.finish_property_define(result)
    }

    fn define_method(
        &mut self,
        base: Value,
        key_index: u32,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = base else {
            return Err(Error::internal(
                "object-literal method target was not an Object",
            ));
        };
        let key = self.constant_property_key(key_index)?;
        let result = self.runtime.define_object_literal_method(
            self.current_realm,
            &object,
            &key,
            function,
            kind,
            enumerable,
        );
        self.finish_property_define(result)
    }

    fn define_method_computed(
        &mut self,
        base: Value,
        key: Value,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = base else {
            return Err(Error::internal(
                "computed object-literal method target was not an Object",
            ));
        };
        let key = self.canonical_property_key_from_value(&key)?;
        let result = self.runtime.define_object_literal_method(
            self.current_realm,
            &object,
            &key,
            function,
            kind,
            enumerable,
        );
        self.finish_property_define(result)
    }

    fn define_class(
        &mut self,
        parent: Value,
        constructor: Value,
        name: u32,
        has_heritage: bool,
    ) -> Result<DefineClassOutcome, Error> {
        let name = match usize::try_from(name)
            .ok()
            .and_then(|index| self.constants.get(index))
        {
            Some(BytecodeConstant::Value(RawValue::String(name))) => name.clone(),
            Some(
                BytecodeConstant::Value(_)
                | BytecodeConstant::Function(_)
                | BytecodeConstant::RegExp { .. },
            ) => {
                return Err(Error::internal(
                    "class-name opcode referenced a non-string constant",
                ));
            }
            None => {
                return Err(Error::internal(
                    "class-name constant index is out of bounds",
                ));
            }
        };
        match self.runtime.define_class_pair(
            self.current_realm,
            parent,
            constructor,
            &name,
            has_heritage,
        ) {
            Ok(outcome) => Ok(outcome),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved a JavaScript-visible class error");
                let value = self
                    .runtime
                    .new_native_error_from_error(self.current_realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?;
                Ok(DefineClassOutcome::Throw(value))
            }
            Err(error) => Err(runtime_error_to_vm_error(error)),
        }
    }

    fn define_array_element(
        &mut self,
        base: Value,
        index: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = base else {
            return Err(Error::new(ErrorKind::Type, "not an object"));
        };
        let key = match self.property_key_from_value(index)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.runtime.define_own_property_in_realm(
            Some(self.current_realm),
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        );
        self.finish_property_define(result)
    }

    fn set_object_prototype(
        &mut self,
        object: Value,
        prototype: Value,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = object else {
            return Err(Error::internal(
                "object-literal prototype target was not an Object",
            ));
        };
        let prototype = match prototype {
            Value::Object(prototype) => Some(prototype),
            Value::Null => None,
            // Pinned QuickJS `OP_set_proto` consumes every primitive without
            // changing the fresh literal.
            _ => return Ok(Completion::Return(Value::Undefined)),
        };
        let changed = self
            .runtime
            .set_prototype_of(&object, prototype.as_ref())
            .map_err(runtime_error_to_vm_error)?;
        if !changed {
            return Err(Error::new(ErrorKind::Type, "prototype is immutable"));
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn copy_data_properties(&mut self, target: Value, source: Value) -> Result<Completion, Error> {
        let Value::Object(target) = target else {
            return Err(Error::internal(
                "object-literal spread target was not an Object",
            ));
        };
        self.runtime
            .copy_object_literal_data_properties(self.current_realm, &target, source)
            .map_err(runtime_error_to_vm_error)
    }

    fn copy_data_properties_excluded(
        &mut self,
        target: Value,
        source: Value,
        excluded: Value,
    ) -> Result<Completion, Error> {
        let Value::Object(target) = target else {
            return Err(Error::internal("object-rest copy target was not an Object"));
        };
        let Value::Object(source) = source else {
            return Err(Error::internal(
                "object-rest source was not an Object after ToObject",
            ));
        };
        let Value::Object(excluded) = excluded else {
            return Err(Error::internal(
                "object-rest exclusion list was not an Object",
            ));
        };
        self.runtime
            .copy_object_rest_data_properties(self.current_realm, &target, &source, &excluded)
            .map_err(runtime_error_to_vm_error)
    }

    fn get_global_var(&mut self, index: u16, throw_if_missing: bool) -> Result<Completion, Error> {
        let descriptor = *self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure index is out of bounds"))?;
        let ClosureVariableName::Atom(atom) = descriptor.name else {
            return Err(Error::internal(
                "published global closure descriptor has no name atom",
            ));
        };
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure slot is out of bounds"))?;
        let cell = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .var_ref(root.id())
            .map_err(|error| Error::internal(error.to_string()))?
            .clone();
        if !matches!(cell.value, RawValue::Uninitialized) {
            return self
                .runtime
                .root_raw_value(&cell.value)
                .map(Completion::Return)
                .map_err(runtime_error_to_vm_error);
        }

        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))?;
        // QuickJS OP_get_var consults the compiled closure descriptor here,
        // not the VarRef metadata. This preserves its observable failed-global-
        // initializer quirk across a later eval: ordinary reads report missing
        // and direct typeof yields undefined, while the declaring script and
        // its relays still observe the lexical TDZ.
        if descriptor.is_lexical {
            let error = self
                .runtime
                .native_atom_error(ErrorKind::Reference, "", &key, " is not initialized")
                .map_err(runtime_error_to_vm_error)?;
            return Err(error);
        }
        let global_object = self
            .runtime
            .global_object_for_realm(self.current_realm)
            .map_err(runtime_error_to_vm_error)?;
        if let Some(completion) = self
            .runtime
            .get_property_or_missing_in_realm(self.current_realm, &global_object, &key)
            .map_err(runtime_error_to_vm_error)?
        {
            return Ok(completion);
        }
        if throw_if_missing {
            let error = self
                .runtime
                .native_atom_error(ErrorKind::Reference, "'", &key, "' is not defined")
                .map_err(runtime_error_to_vm_error)?;
            Err(error)
        } else {
            Ok(Completion::Return(Value::Undefined))
        }
    }

    fn delete_global_var(&mut self, index: u16) -> Result<Completion, Error> {
        let descriptor = *self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure index is out of bounds"))?;
        let ClosureVariableName::Atom(atom) = descriptor.name else {
            return Err(Error::internal(
                "published global closure descriptor has no name atom",
            ));
        };
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure slot is out of bounds"))?;
        let is_lexical = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .var_ref(root.id())
            .map_err(|error| Error::internal(error.to_string()))?
            .is_lexical;
        if is_lexical {
            return Ok(Completion::Return(Value::Bool(false)));
        }

        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))?;
        let global_object = self
            .runtime
            .global_object_for_realm(self.current_realm)
            .map_err(runtime_error_to_vm_error)?;
        // QuickJS `JS_DeleteGlobalVar` performs HasProperty first. Ordinary
        // objects reach the same Boolean result without it, but the step is
        // observable through the future Proxy/exotic prototype path.
        let exists = self
            .runtime
            .has_property(&global_object, &key)
            .map_err(runtime_error_to_vm_error)?;
        let deleted = if exists {
            self.runtime
                .delete_property(&global_object, &key)
                .map_err(runtime_error_to_vm_error)?
        } else {
            true
        };
        Ok(Completion::Return(Value::Bool(deleted)))
    }

    fn put_global_var(
        &mut self,
        index: u16,
        value: Value,
        initialize: bool,
        strict: bool,
    ) -> Result<Completion, Error> {
        let descriptor = *self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure index is out of bounds"))?;
        let ClosureVariableName::Atom(atom) = descriptor.name else {
            return Err(Error::internal(
                "published global closure descriptor has no name atom",
            ));
        };
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure slot is out of bounds"))?;
        let cell = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .var_ref(root.id())
            .map_err(|error| Error::internal(error.to_string()))?
            .clone();
        // QuickJS's hoisted-definition pass uses a raw VarRef write for both
        // lexical declarations and Program function declarations. The
        // verifier limits `PutVarInit` on an ordinary descriptor to either
        // a GlobalFunction prologue or the first normal declaration slot for a
        // same-name masked Program lexical. The latter slot has been promoted
        // to the lexical VarRef during declaration instantiation, so this raw
        // initialization cannot be reached by an ordinary source assignment.
        if initialize {
            self.runtime
                .write_var_ref(root, value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(Completion::Return(Value::Undefined));
        }
        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))?;
        if cell.is_lexical {
            if matches!(cell.value, RawValue::Uninitialized) {
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Reference, "", &key, " is not initialized")
                    .map_err(runtime_error_to_vm_error)?;
                return Err(error);
            }
            if cell.is_const {
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Type, "'", &key, "' is read-only")
                    .map_err(runtime_error_to_vm_error)?;
                return Err(error);
            }
            self.runtime
                .write_var_ref(root, value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(Completion::Return(Value::Undefined));
        }

        if !matches!(cell.value, RawValue::Uninitialized) && !cell.is_const {
            self.runtime
                .write_var_ref(root, value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(Completion::Return(Value::Undefined));
        }

        let global_object = self
            .runtime
            .global_object_for_realm(self.current_realm)
            .map_err(runtime_error_to_vm_error)?;
        let exists = self
            .runtime
            .has_property(&global_object, &key)
            .map_err(runtime_error_to_vm_error)?;
        if strict && !exists {
            let error = self
                .runtime
                .native_atom_error(ErrorKind::Reference, "'", &key, "' is not defined")
                .map_err(runtime_error_to_vm_error)?;
            return Err(error);
        }
        match self
            .runtime
            .prepare_set_property_in_realm(self.current_realm, &global_object, &key, value)
            .map_err(runtime_error_to_vm_error)?
        {
            PropertySetAction::Complete => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Throw(value) => Ok(Completion::Throw(value)),
            PropertySetAction::Rejected(_) if !strict => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Rejected(PropertySetRejection::ReadOnly) => {
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Type, "'", &key, "' is read-only")
                    .map_err(runtime_error_to_vm_error)?;
                Err(error)
            }
            PropertySetAction::Rejected(PropertySetRejection::ArrayLengthReadOnly) => {
                let length = self
                    .runtime
                    .intern_property_key("length")
                    .map_err(|error| Error::internal(error.to_string()))?;
                let error = self
                    .runtime
                    .native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")
                    .map_err(runtime_error_to_vm_error)?;
                Err(error)
            }
            PropertySetAction::Rejected(PropertySetRejection::NotConfigurable) => {
                Err(Error::new(ErrorKind::Type, "not configurable"))
            }
            PropertySetAction::Rejected(PropertySetRejection::NoSetter) => {
                Err(Error::new(ErrorKind::Type, "no setter for property"))
            }
            PropertySetAction::Rejected(PropertySetRejection::NotExtensible) => {
                Err(Error::new(ErrorKind::Type, "object is not extensible"))
            }
            PropertySetAction::Rejected(PropertySetRejection::NotObject) => Err(Error::internal(
                "global object assignment produced a primitive receiver rejection",
            )),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => self
                .runtime
                .call_internal(self.current_realm, &setter, receiver, &[argument])
                .map_err(runtime_error_to_vm_error),
        }
    }

    fn get_field(&mut self, base: Value, key_index: u32) -> Result<Completion, Error> {
        let key = self.constant_property_key(key_index)?;
        self.get_property_with_key(base, &key, true)
    }

    fn get_property(&mut self, base: Value, key: Value) -> Result<Completion, Error> {
        // QuickJS `JS_GetPropertyValue` performs the ToObject null/undefined
        // check before observable ToPropertyKey conversion.
        if matches!(base, Value::Null | Value::Undefined) {
            let base_name = if matches!(base, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Err(Error::new(
                ErrorKind::Type,
                format!("cannot read property of {base_name}"),
            ));
        }
        let key = match self.property_key_from_value(key)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.get_property_with_key(base, &key, false)
    }

    fn get_super_property(
        &mut self,
        receiver: Value,
        base: Value,
        key: Value,
    ) -> Result<Completion, Error> {
        self.read_super_property(receiver, base, key)
    }

    fn has_property(&mut self, key: Value, object: ObjectRef) -> Result<Completion, Error> {
        // QuickJS `js_operator_in` validates the RHS object before
        // JS_ValueToAtom can execute arbitrary key-conversion code.
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "in right operand belongs to another runtime",
            ));
        }
        let key = match self.property_key_from_value(key)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.runtime
            .has_property_in_realm(self.current_realm, &object, &key)
            .map_err(runtime_error_to_vm_error)
    }

    fn is_instance_of(&mut self, candidate: Value, target: ObjectRef) -> Result<Completion, Error> {
        self.runtime
            .is_instance_of(self.current_realm, candidate, target)
            .map_err(runtime_error_to_vm_error)
    }

    fn convert_property_key(&mut self, key: Value) -> Result<Completion, Error> {
        let key = match key {
            key @ (Value::Int(_) | Value::String(_)) => return Ok(Completion::Return(key)),
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(&self.runtime) {
                    return Err(Error::internal(
                        "computed property symbol belongs to another runtime",
                    ));
                }
                return Ok(Completion::Return(Value::Symbol(symbol)));
            }
            key @ Value::Object(_) => match self
                .runtime
                .to_primitive(self.current_realm, key, ToPrimitiveHint::String)
                .map_err(runtime_error_to_vm_error)?
            {
                Completion::Return(key) => key,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            key => key,
        };
        match key {
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(&self.runtime) {
                    return Err(Error::internal(
                        "computed property symbol belongs to another runtime",
                    ));
                }
                Ok(Completion::Return(Value::Symbol(symbol)))
            }
            Value::String(string) => Ok(Completion::Return(Value::String(string))),
            key => key
                .to_js_string()
                .map(Value::String)
                .map(Completion::Return),
        }
    }

    fn set_field(
        &mut self,
        base: Value,
        key_index: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        let key = self.constant_property_key(key_index)?;
        self.set_property_with_key(base, &key, value, strict)
    }

    fn set_property(
        &mut self,
        base: Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        // QuickJS `OP_put_array_el` evaluates the RHS before entering here,
        // then performs observable key conversion before it checks/boxes the
        // base. This intentionally differs from computed reads.
        let key = match self.property_key_from_value(key)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.set_property_with_key(base, &key, value, strict)
    }

    fn set_super_property(
        &mut self,
        receiver: Value,
        base: Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        self.write_super_property(receiver, base, key, value, strict)
    }

    fn delete_property(
        &mut self,
        base: Value,
        key: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        // QuickJS `OP_delete` converts the key before ToObject/null checking.
        let key = match self.property_key_from_value(key)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.delete_property_with_key(base, &key, strict)
    }

    fn call(
        &mut self,
        function: Value,
        this_value: Value,
        arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        let callable = self
            .runtime
            .callable_from_value(function)
            .map_err(runtime_error_to_vm_error)?;
        self.runtime
            .call_internal(self.current_realm, &callable, this_value, &arguments)
            .map_err(runtime_error_to_vm_error)
    }

    fn apply(
        &mut self,
        function: Value,
        this_or_new_target: Value,
        argument_array: Value,
        kind: ApplyKind,
    ) -> Result<Completion, Error> {
        // Pinned QuickJS's js_function_apply checks callability before
        // build_arg_list for both magic values. Constructor capability and
        // newTarget validation deliberately remain after list construction.
        let callable = self
            .runtime
            .callable_from_value(function.clone())
            .map_err(runtime_error_to_vm_error)?;
        let arguments = match self.build_argument_list(argument_array)? {
            ArgumentListOutcome::Values(arguments) => arguments,
            ArgumentListOutcome::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match kind {
            ApplyKind::Call => self
                .runtime
                .call_internal(
                    self.current_realm,
                    &callable,
                    this_or_new_target,
                    &arguments,
                )
                .map_err(runtime_error_to_vm_error),
            ApplyKind::Construct => self
                .runtime
                .construct_value_internal(
                    self.current_realm,
                    function,
                    this_or_new_target,
                    &arguments,
                )
                .map_err(runtime_error_to_vm_error),
        }
    }

    fn build_argument_list(&mut self, argument_array: Value) -> Result<ArgumentListOutcome, Error> {
        match self
            .runtime
            .build_array_like_argument_list(self.current_realm, &argument_array)
            .map_err(runtime_error_to_vm_error)?
        {
            NativeConversion::Value(arguments) => Ok(ArgumentListOutcome::Values(arguments)),
            NativeConversion::Throw(value) => Ok(ArgumentListOutcome::Throw(value)),
        }
    }

    fn is_original_eval(&mut self, function: &Value) -> Result<bool, Error> {
        self.runtime
            .is_original_eval(self.current_realm, function)
            .map_err(runtime_error_to_vm_error)
    }

    fn direct_eval(&mut self, invocation: DirectEvalInvocation) -> Result<Completion, Error> {
        let environment = if matches!(invocation.input, Value::String(_)) {
            Some(self.prepare_direct_eval_environment(
                invocation.environment,
                invocation.caller_strict,
            )?)
        } else {
            None
        };
        let runtime = self.runtime.clone();
        runtime
            .call_direct_eval_original(self.current_realm, invocation, environment, |prepared| {
                self.materialize_direct_eval_environment(prepared)
            })
            .map_err(runtime_error_to_vm_error)
    }

    fn construct(
        &mut self,
        function: Value,
        new_target: Value,
        arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        self.runtime
            .construct_value_internal(self.current_realm, function, new_target, &arguments)
            .map_err(runtime_error_to_vm_error)
    }

    fn closure_count(&self) -> usize {
        self.closure_slots.len()
    }

    fn get_local(&mut self, index: u16) -> Result<Value, Error> {
        let definition = self.local_definition(index)?;
        if definition.kind == ClosureVariableKind::WithObject {
            return Err(Error::internal(
                "ordinary local read referenced a private with object",
            ));
        }
        if definition.is_lexical {
            return Err(Error::internal(
                "unchecked local read referenced a lexical definition",
            ));
        }
        let binding = self
            .locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        read_frame_binding(&self.runtime, binding)
    }

    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        if definition.kind == ClosureVariableKind::WithObject {
            return Err(Error::internal(
                "ordinary local write referenced a private with object",
            ));
        }
        if definition.is_lexical {
            return Err(Error::internal(
                "unchecked local write referenced a lexical definition",
            ));
        }
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        write_frame_binding(&self.runtime, binding, value)
    }

    fn set_local_uninitialized(&mut self, index: u16) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        if !definition.is_lexical {
            return Err(Error::internal(
                "lexical scope entry referenced an ordinary local definition",
            ));
        }
        let reusable = self
            .reusable_captured_locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local reuse flag index is out of bounds"))?;
        let reusable = std::mem::take(reusable);
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        if let FrameBinding::Captured(root) = binding {
            let raw = self
                .runtime
                .raw_var_ref_value(root)
                .map_err(runtime_error_to_vm_error)?;
            if matches!(raw, RawValue::Uninitialized) {
                // QuickJS creates direct FunctionBody declaration closures
                // before expanding the body scope's lexical TDZ entries. A
                // child may therefore capture this first uninitialized cell
                // before SetLocalUninitialized reaches it; entering that same
                // initial lifetime is a no-op. A live initialized capture still
                // proves that a later lifetime skipped CloseLocal.
                return Ok(());
            }
            if reusable {
                self.runtime
                    .reset_var_ref_uninitialized(root)
                    .map_err(runtime_error_to_vm_error)?;
                return Ok(());
            }
            return Err(Error::internal(
                "captured local entered a new lexical lifetime before CloseLocal",
            ));
        }
        *binding = FrameBinding::Uninitialized;
        Ok(())
    }

    fn get_local_checked(&mut self, index: u16) -> Result<Value, Error> {
        let definition = self.local_definition(index)?;
        if !definition.is_lexical {
            return Err(Error::internal(
                "checked local read referenced an ordinary definition",
            ));
        }
        let binding = self
            .locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        match binding {
            FrameBinding::Direct(value) => Ok(value.clone()),
            FrameBinding::Uninitialized => Err(self.lexical_uninitialized_error(definition.name)?),
            FrameBinding::Captured(root) => {
                let raw = self
                    .runtime
                    .raw_var_ref_value(root)
                    .map_err(runtime_error_to_vm_error)?;
                if matches!(raw, RawValue::Uninitialized) {
                    Err(self.lexical_uninitialized_error(definition.name)?)
                } else {
                    self.runtime
                        .root_raw_value(&raw)
                        .map_err(runtime_error_to_vm_error)
                }
            }
        }
    }

    fn initialize_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        if !definition.is_lexical && definition.kind != ClosureVariableKind::WithObject {
            return Err(Error::internal(
                "local initialization referenced an ordinary local definition",
            ));
        }
        if definition.kind == ClosureVariableKind::WithObject {
            let Value::Object(object) = &value else {
                return Err(Error::internal(
                    "with-object initialization did not receive an Object",
                ));
            };
            if !object.belongs_to(&self.runtime) {
                return Err(Error::internal(
                    "with-object initialization received a cross-runtime Object",
                ));
            }
        }
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        match binding {
            FrameBinding::Direct(slot) => {
                *slot = value;
                Ok(())
            }
            FrameBinding::Uninitialized => {
                *binding = FrameBinding::Direct(value);
                Ok(())
            }
            FrameBinding::Captured(root) => self
                .runtime
                .write_var_ref(root, value)
                .map_err(runtime_error_to_vm_error),
        }
    }

    fn put_local_checked(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        if !definition.is_lexical {
            return Err(Error::internal(
                "checked local write referenced an ordinary definition",
            ));
        }
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        match binding {
            FrameBinding::Direct(slot) => {
                if definition.is_const {
                    return Err(self.lexical_read_only_error(definition.name)?);
                }
                *slot = value;
                Ok(())
            }
            FrameBinding::Uninitialized => Err(self.lexical_uninitialized_error(definition.name)?),
            FrameBinding::Captured(root) => {
                let cell = self
                    .runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .var_ref(root.id())
                    .map_err(|error| Error::internal(error.to_string()))?
                    .clone();
                if matches!(cell.value, RawValue::Uninitialized) {
                    return Err(self.lexical_uninitialized_error(definition.name)?);
                }
                if cell.is_const {
                    return Err(self.lexical_read_only_error(definition.name)?);
                }
                self.runtime
                    .write_var_ref(root, value)
                    .map_err(runtime_error_to_vm_error)
            }
        }
    }

    fn close_local(&mut self, index: u16) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        if !definition.is_lexical && definition.kind != ClosureVariableKind::WithObject {
            return Err(Error::internal(
                "CloseLocal referenced an ordinary local definition",
            ));
        }
        let reusable = self
            .reusable_captured_locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local reuse flag index is out of bounds"))?;
        *reusable = false;
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        close_frame_binding(&self.runtime, binding)
    }

    fn get_argument(&mut self, index: u16) -> Result<Value, Error> {
        let binding = self
            .arguments
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("argument index is out of bounds"))?;
        read_frame_binding(&self.runtime, binding)
    }

    fn put_argument(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let binding = self
            .arguments
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("argument index is out of bounds"))?;
        write_frame_binding(&self.runtime, binding, value)
    }

    fn get_var_ref(&mut self, index: u16) -> Result<Value, Error> {
        let descriptor = self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        if descriptor.is_lexical {
            return Err(Error::internal(
                "unchecked closure read referenced a lexical binding",
            ));
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .read_var_ref(root)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn put_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let descriptor = self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        if descriptor.is_lexical {
            return Err(Error::internal(
                "unchecked closure write referenced a lexical binding",
            ));
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .write_var_ref(root, value)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn get_var_ref_checked(&mut self, index: u16) -> Result<Value, Error> {
        let descriptor = self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        if !descriptor.is_lexical {
            return Err(Error::internal(
                "checked closure read referenced an ordinary binding",
            ));
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        let raw = self
            .runtime
            .raw_var_ref_value(root)
            .map_err(runtime_error_to_vm_error)?;
        if matches!(raw, RawValue::Uninitialized) {
            return Err(self.lexical_uninitialized_error(self.closure_name(index)?)?);
        }
        self.runtime
            .root_raw_value(&raw)
            .map_err(runtime_error_to_vm_error)
    }

    fn put_var_ref_checked(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let descriptor = self
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        if !descriptor.is_lexical {
            return Err(Error::internal(
                "checked closure write referenced an ordinary binding",
            ));
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        let cell = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .var_ref(root.id())
            .map_err(|error| Error::internal(error.to_string()))?
            .clone();
        let name = self.closure_name(index)?;
        if matches!(cell.value, RawValue::Uninitialized) {
            return Err(self.lexical_uninitialized_error(name)?);
        }
        if cell.is_const {
            return Err(self.lexical_read_only_error(name)?);
        }
        self.runtime
            .write_var_ref(root, value)
            .map_err(runtime_error_to_vm_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::EvalVariableSource;
    use crate::object::CompleteOrdinaryPropertyDescriptor;

    fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
        let Value::Object(object) = context.eval(source).unwrap() else {
            panic!("{source} did not evaluate to an Object");
        };
        object
    }

    fn eval_string(context: &mut Context, source: &str) -> String {
        let Value::String(value) = context.eval(source).unwrap() else {
            panic!("{source} did not evaluate to a String");
        };
        value.to_utf8_lossy()
    }

    fn local_variable_environment_host(
        runtime: Runtime,
        realm: ContextId,
        kind: ClosureVariableKind,
        authenticated_local: Option<u16>,
    ) -> (RuntimeVmHost, ObjectRef) {
        let object = runtime.new_object(None).unwrap();
        let mut host = RuntimeVmHost::empty_for_test(runtime, realm);
        host.constants = Rc::from([BytecodeConstant::Value(RawValue::String(
            JsString::from_static("added"),
        ))]);
        host.local_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind,
        }]);
        host.eval_variable_object_local = authenticated_local;
        host.locals = vec![FrameBinding::Direct(Value::Object(object.clone()))];
        host.reusable_captured_locals = vec![false];
        (host, object)
    }

    #[test]
    fn local_eval_variable_environment_defines_overwrites_and_deletes_cwe_data() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let (mut host, object) = local_variable_environment_host(
            runtime.clone(),
            context.realm,
            ClosureVariableKind::EvalVariableObject,
            Some(0),
        );
        let source = EvalVariableSource::Local(0);

        assert_eq!(
            host.has_eval_variable(source, 0).unwrap(),
            Completion::Return(Value::Bool(false))
        );
        assert_eq!(
            host.define_eval_variable(source, 0, Value::Int(1)).unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            host.get_eval_variable(source, 0).unwrap(),
            Completion::Return(Value::Int(1))
        );

        // Define is deliberately unconditional: the eval declaration plan
        // uses it for QuickJS's repeated-var undefined overwrite.
        host.define_eval_variable(source, 0, Value::Undefined)
            .unwrap();
        assert_eq!(
            host.get_eval_variable(source, 0).unwrap(),
            Completion::Return(Value::Undefined)
        );
        host.put_eval_variable(source, 0, Value::Int(42)).unwrap();
        assert_eq!(
            host.get_eval_variable(source, 0).unwrap(),
            Completion::Return(Value::Int(42))
        );

        let key = runtime.intern_property_key("added").unwrap();
        assert_eq!(
            runtime.get_own_property(&object, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(42),
                writable: true,
                enumerable: true,
                configurable: true,
            })
        );
        assert_eq!(
            host.delete_eval_variable(source, 0).unwrap(),
            Completion::Return(Value::Bool(true))
        );
        assert_eq!(
            host.has_eval_variable(source, 0).unwrap(),
            Completion::Return(Value::Bool(false))
        );
    }

    #[test]
    fn eval_variable_sources_require_authenticated_special_metadata() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let (mut unauthenticated, _) = local_variable_environment_host(
            runtime.clone(),
            context.realm,
            ClosureVariableKind::EvalVariableObject,
            None,
        );
        assert_eq!(
            unauthenticated
                .has_eval_variable(EvalVariableSource::Local(0), 0)
                .unwrap_err()
                .message(),
            "eval variable opcode referenced an unauthenticated local"
        );

        let (mut ordinary, _) = local_variable_environment_host(
            runtime.clone(),
            context.realm,
            ClosureVariableKind::Normal,
            Some(0),
        );
        assert_eq!(
            ordinary
                .has_eval_variable(EvalVariableSource::Local(0), 0)
                .unwrap_err()
                .message(),
            "eval variable opcode referenced a non-variable-object local"
        );

        let object = runtime.new_object(None).unwrap();
        let root = runtime
            .new_var_ref(
                Value::Object(object),
                false,
                false,
                ClosureVariableKind::Normal,
            )
            .unwrap();
        let mut closure = RuntimeVmHost::empty_for_test(runtime, context.realm);
        closure.constants = Rc::from([BytecodeConstant::Value(RawValue::String(
            JsString::from_static("added"),
        ))]);
        closure.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }]);
        closure.closure_slots = vec![root];
        assert_eq!(
            closure
                .has_eval_variable(EvalVariableSource::Closure(0), 0)
                .unwrap_err()
                .message(),
            "eval variable opcode referenced a non-variable-object closure"
        );

        let runtime = closure.runtime.clone();
        let object = runtime.new_object(None).unwrap();
        let root = runtime
            .new_var_ref(
                Value::Object(object),
                false,
                false,
                ClosureVariableKind::EvalVariableObject,
            )
            .unwrap();
        closure.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }]);
        closure.closure_slots = vec![root];
        assert_eq!(
            closure
                .define_eval_variable(EvalVariableSource::Closure(0), 0, Value::Int(42))
                .unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            closure
                .get_eval_variable(EvalVariableSource::Closure(0), 0)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
    }

    #[test]
    fn with_object_local_allows_initialization_and_captured_close() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let with_object = runtime.new_object(None).unwrap();
        let root = runtime
            .new_var_ref(
                Value::Undefined,
                false,
                false,
                ClosureVariableKind::WithObject,
            )
            .unwrap();
        let mut host = RuntimeVmHost::empty_for_test(runtime, context.realm);
        host.local_definitions = Rc::from([VariableDefinition {
            name: Some(Atom::from_raw(71)),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::WithObject,
        }]);
        host.locals = vec![FrameBinding::Captured(root.clone())];
        host.reusable_captured_locals = vec![false];

        host.initialize_local(0, Value::Object(with_object.clone()))
            .unwrap();
        assert_eq!(
            host.runtime.read_var_ref(&root).unwrap(),
            Value::Object(with_object.clone())
        );
        assert_eq!(
            host.get_local(0).unwrap_err().message(),
            "ordinary local read referenced a private with object"
        );
        host.close_local(0).unwrap();
        assert!(matches!(
            &host.locals[0],
            FrameBinding::Direct(Value::Object(object)) if object == &with_object
        ));

        assert_eq!(
            host.initialize_local(0, Value::Int(42))
                .unwrap_err()
                .message(),
            "with-object initialization did not receive an Object"
        );

        host.local_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }]);
        assert_eq!(
            host.initialize_local(0, Value::Undefined)
                .unwrap_err()
                .message(),
            "local initialization referenced an ordinary local definition"
        );
    }

    #[test]
    fn object_rest_copy_snapshots_enumerability_excludes_string_and_symbol_keys_and_defines_data() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval(
                r#"
                var __restCopy = (function(){
                    var log="", setterHits=0;
                    var keep=Symbol("keep"), omit=Symbol("omit");
                    var source={}, excluded={}, target={};
                    Object.defineProperty(source,"a",{
                        enumerable:true, configurable:true,
                        get:function(){
                            log+="get-a|";
                            Object.defineProperty(source,"b",{
                                value:"B2",writable:true,enumerable:false,configurable:true
                            });
                            Object.defineProperty(source,"c",{
                                value:"C2",writable:true,enumerable:true,configurable:true
                            });
                            source.late="late";
                            return "A";
                        }
                    });
                    source.b="B";
                    Object.defineProperty(source,"c",{
                        value:"C",writable:true,enumerable:false,configurable:true
                    });
                    Object.defineProperty(source,"skip",{
                        enumerable:true,configurable:true,
                        get:function(){log+="get-skip|";throw "skip getter ran"}
                    });
                    source[keep]="S";
                    Object.defineProperty(source,omit,{
                        enumerable:true,configurable:true,
                        get:function(){log+="get-omit|";throw "omit getter ran"}
                    });
                    source.setterKey=42;
                    excluded.skip=null;
                    excluded[omit]=null;
                    Object.defineProperty(Object.prototype,"setterKey",{
                        configurable:true,set:function(){setterHits++}
                    });
                    return {
                        source:source,excluded:excluded,target:target,
                        observe:function(){
                            delete Object.prototype.setterKey;
                            function bits(key){
                                var d=Object.getOwnPropertyDescriptor(target,key);
                                return Number(d.writable)+""+Number(d.enumerable)+Number(d.configurable);
                            }
                            return Reflect.ownKeys(target).map(String).join(",")+"|"+
                                target.a+"|"+target.b+"|"+target.setterKey+"|"+target[keep]+"|"+
                                Object.hasOwn(target,"c")+"|"+Object.hasOwn(target,"late")+"|"+
                                Object.hasOwn(target,"skip")+"|"+Object.hasOwn(target,omit)+"|"+
                                log+"|"+setterHits+"|"+
                                bits("a")+bits("b")+bits("setterKey")+bits(keep);
                        }
                    };
                })();
                undefined
                "#,
            )
            .unwrap();
        let source = eval_object(&mut context, "__restCopy.source");
        let excluded = eval_object(&mut context, "__restCopy.excluded");
        let target = eval_object(&mut context, "__restCopy.target");
        let mut host = RuntimeVmHost::empty_for_test(runtime, context.realm);

        assert_eq!(
            host.copy_data_properties_excluded(
                Value::Object(target),
                Value::Object(source),
                Value::Object(excluded),
            )
            .unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            eval_string(&mut context, "__restCopy.observe()"),
            "a,b,setterKey,Symbol(keep)|A|B2|42|S|false|false|false|false|get-a||0|111111111111"
        );
    }

    #[test]
    fn object_rest_copy_stops_on_get_throw_after_preserving_prior_definitions() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval(
                r#"
                var __restThrow = (function(){
                    var log="",boom={},source={},target={},excluded={};
                    Object.defineProperty(source,"a",{
                        enumerable:true,get:function(){log+="a|";return 1}
                    });
                    Object.defineProperty(source,"b",{
                        enumerable:true,get:function(){log+="b|";throw boom}
                    });
                    Object.defineProperty(source,"c",{
                        enumerable:true,get:function(){log+="c|";return 3}
                    });
                    return {
                        boom:boom,source:source,target:target,excluded:excluded,
                        observe:function(){
                            var d=Object.getOwnPropertyDescriptor(target,"a");
                            return log+"|"+target.a+"|"+Object.hasOwn(target,"b")+"|"+
                                Object.hasOwn(target,"c")+"|"+
                                Number(d.writable)+Number(d.enumerable)+Number(d.configurable);
                        }
                    };
                })();
                undefined
                "#,
            )
            .unwrap();
        let boom = eval_object(&mut context, "__restThrow.boom");
        let source = eval_object(&mut context, "__restThrow.source");
        let target = eval_object(&mut context, "__restThrow.target");
        let excluded = eval_object(&mut context, "__restThrow.excluded");
        let mut host = RuntimeVmHost::empty_for_test(runtime, context.realm);

        assert_eq!(
            host.copy_data_properties_excluded(
                Value::Object(target),
                Value::Object(source),
                Value::Object(excluded),
            )
            .unwrap(),
            Completion::Throw(Value::Object(boom))
        );
        assert_eq!(
            eval_string(&mut context, "__restThrow.observe()"),
            "a|b||1|false|false|111"
        );
    }

    #[test]
    fn object_rest_copy_requires_compiler_preconversion_and_private_objects() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let target = runtime.new_object(None).unwrap();
        let source = runtime.new_object(None).unwrap();
        let excluded = runtime.new_object(None).unwrap();
        let mut host = RuntimeVmHost::empty_for_test(runtime, context.realm);

        assert_eq!(
            host.copy_data_properties_excluded(
                Value::Object(target.clone()),
                Value::Null,
                Value::Object(excluded.clone()),
            )
            .unwrap_err()
            .message(),
            "object-rest source was not an Object after ToObject"
        );
        assert_eq!(
            host.copy_data_properties_excluded(
                Value::Object(target.clone()),
                Value::Object(source.clone()),
                Value::Undefined,
            )
            .unwrap_err()
            .message(),
            "object-rest exclusion list was not an Object"
        );
        assert_eq!(
            host.copy_data_properties_excluded(
                Value::Int(0),
                Value::Object(source),
                Value::Object(excluded),
            )
            .unwrap_err()
            .message(),
            "object-rest copy target was not an Object"
        );
    }

    #[test]
    fn direct_eval_environment_authenticates_super_capability_before_materialization() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let host = RuntimeVmHost::empty_for_test(runtime, context.realm);
        let mut environment = EvalEnvironment::<Atom> {
            scopes: Box::new([]),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: false,
            super_call_allowed: false,
            super_allowed: false,
        };

        let error = host
            .validate_eval_environment(
                &environment,
                false,
                FunctionMetadata {
                    super_allowed: true,
                    ..FunctionMetadata::default()
                },
            )
            .unwrap_err();
        assert_eq!(
            error.message(),
            "eval environment super capability disagrees with caller bytecode"
        );

        environment.super_call_allowed = true;
        let error = host
            .validate_eval_environment(&environment, false, FunctionMetadata::default())
            .unwrap_err();
        assert_eq!(
            error.message(),
            "eval environment permits super() without SuperProperty"
        );
    }

    #[test]
    fn strict_script_global_eval_anchor_is_not_valid_for_functions_or_eval_roots() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let host = RuntimeVmHost::empty_for_test(runtime, context.realm);
        let strict_script = EvalEnvironment::<Atom> {
            scopes: vec![
                crate::heap::EvalScope {
                    kind: crate::heap::EvalScopeKind::ProgramBody,
                    bindings: Box::new([]),
                },
                crate::heap::EvalScope {
                    kind: crate::heap::EvalScopeKind::FunctionRoot,
                    bindings: Box::new([]),
                },
            ]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::Global,
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        };
        host.validate_eval_environment(
            &strict_script,
            true,
            FunctionMetadata {
                strict: true,
                ..FunctionMetadata::default()
            },
        )
        .unwrap();

        let mut strict_eval_local = strict_script.clone();
        strict_eval_local.variable_environment = EvalVariableEnvironment::StrictLocal(1);
        assert_eq!(
            host.validate_eval_environment(
                &strict_eval_local,
                true,
                FunctionMetadata {
                    strict: true,
                    ..FunctionMetadata::default()
                },
            )
            .unwrap_err()
            .message(),
            "authored Script eval environment used a non-canonical strict-local target"
        );
        host.validate_eval_environment(
            &strict_eval_local,
            true,
            FunctionMetadata {
                strict: true,
                eval_kind: crate::heap::EvalKind::Direct,
                ..FunctionMetadata::default()
            },
        )
        .unwrap();

        let mut strict_function = strict_script.clone();
        strict_function.scopes = vec![
            crate::heap::EvalScope {
                kind: crate::heap::EvalScopeKind::FunctionBody,
                bindings: Box::new([]),
            },
            crate::heap::EvalScope {
                kind: crate::heap::EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
            crate::heap::EvalScope {
                kind: crate::heap::EvalScopeKind::ProgramBody,
                bindings: Box::new([]),
            },
            crate::heap::EvalScope {
                kind: crate::heap::EvalScopeKind::FunctionRoot,
                bindings: Box::new([]),
            },
        ]
        .into_boxed_slice();
        assert_eq!(
            host.validate_eval_environment(
                &strict_function,
                true,
                FunctionMetadata {
                    strict: true,
                    ..FunctionMetadata::default()
                },
            )
            .unwrap_err()
            .message(),
            "global eval variable environment escaped an authored Script root"
        );

        assert_eq!(
            host.validate_eval_environment(
                &strict_script,
                true,
                FunctionMetadata {
                    strict: true,
                    eval_kind: crate::heap::EvalKind::Direct,
                    ..FunctionMetadata::default()
                },
            )
            .unwrap_err()
            .message(),
            "global eval variable environment escaped an authored Script root"
        );
    }
}
