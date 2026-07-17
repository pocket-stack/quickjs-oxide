//! Bytecode VM adapter and per-frame binding state.
//!
//! This module owns the translation between VM completions/errors and the
//! runtime's object, call, iterator, realm and captured-variable machinery.

use super::*;
use crate::bytecode::ArgumentsKind;
use crate::heap::{EvalBinding, EvalBindingSource, EvalVariableEnvironment};
use crate::vm::{CallInput, DirectEvalInvocation, Vm, VmHost};

/// Live cells paired with one immutable caller-environment descriptor.
///
/// Roots are flattened in the descriptor's scope/binding order. The
/// descriptor itself preserves the lexical boundaries and declaration target
/// needed by the future eval compiler, while the roots keep the caller's
/// actual cells live for that compilation/execution interval.
pub(in crate::runtime) struct MaterializedEvalEnvironment {
    pub(in crate::runtime) index: u16,
    pub(in crate::runtime) descriptor: EvalEnvironment<Atom>,
    pub(in crate::runtime) roots: Box<[VarRefRoot]>,
}

enum FrameBinding {
    Direct(Value),
    Uninitialized,
    Captured(VarRefRoot),
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
            current_function: None,
            actual_argument_count: 0,
            constants: Rc::from([]),
            argument_definitions: Rc::from([]),
            local_definitions: Rc::from([]),
            closure_variables: Rc::from([]),
            eval_environments: Rc::from([]),
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
            root: _,
            code: _,
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            metadata: _,
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
            current_function: None,
            actual_argument_count: arguments.len(),
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
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
        let flags_match = (definition.is_lexical, definition.is_const, definition.kind)
            == (descriptor.is_lexical, descriptor.is_const, descriptor.kind);
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

    fn validate_eval_environment(
        &self,
        environment: &EvalEnvironment<Atom>,
        caller_strict: bool,
    ) -> Result<(), Error> {
        if environment.caller_strict != caller_strict {
            return Err(Error::internal(
                "eval environment caller strictness disagrees with its bytecode frame",
            ));
        }
        if let EvalVariableEnvironment::Scope(scope) = environment.variable_environment {
            let Some(scope) = environment.scopes.get(usize::from(scope)) else {
                return Err(Error::internal(
                    "eval variable-environment scope is out of bounds",
                ));
            };
            if scope.kind != crate::heap::EvalScopeKind::FunctionRoot {
                return Err(Error::internal(
                    "eval variable environment did not select a function root",
                ));
            }
        }
        for scope in &environment.scopes {
            for binding in &scope.bindings {
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

    fn materialize_direct_eval_environment(
        &mut self,
        index: u16,
        caller_strict: bool,
    ) -> Result<MaterializedEvalEnvironment, Error> {
        let descriptor = self
            .eval_environments
            .get(usize::from(index))
            .cloned()
            .ok_or_else(|| Error::internal("eval environment index is out of bounds"))?;
        // Validate every immutable source before converting any direct frame
        // slot to a VarRef. Corrupt published bytecode therefore cannot leave
        // a partially captured frame behind.
        self.validate_eval_environment(&descriptor, caller_strict)?;

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
        let PublishedFunctionSnapshot {
            root,
            code,
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
            metadata,
            realm,
        } = self.snapshot_function_bytecode(&bytecode)?;
        let callee_global = self.global_object_for_realm(realm)?;
        let active_frame = self.push_bytecode_active_frame(
            callable.as_object().clone(),
            root,
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
            current_function: Some(callable.as_object().clone()),
            actual_argument_count: arguments.len(),
            constants,
            argument_definitions,
            local_definitions,
            closure_variables,
            eval_environments,
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
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
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
        let closure_variables = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(child_id)
            .map_err(|error| Error::internal(error.to_string()))?
            .closure_variables
            .clone();
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
                    capture_frame_binding(&self.runtime, binding, descriptor)?
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

    fn object(&mut self) -> Result<Completion, Error> {
        self.runtime
            .new_ordinary_object_in_realm(self.current_realm)
            .map(|object| Completion::Return(Value::Object(object)))
            .map_err(runtime_error_to_vm_error)
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

    fn is_original_eval(&mut self, function: &Value) -> Result<bool, Error> {
        self.runtime
            .is_original_eval(self.current_realm, function)
            .map_err(runtime_error_to_vm_error)
    }

    fn direct_eval(&mut self, invocation: DirectEvalInvocation) -> Result<Completion, Error> {
        let environment = if matches!(invocation.input, Value::String(_)) {
            Some(self.materialize_direct_eval_environment(
                invocation.environment,
                invocation.caller_strict,
            )?)
        } else {
            None
        };
        self.runtime
            .call_direct_eval_original(invocation, environment)
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
        if self.local_definition(index)?.is_lexical {
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
        if self.local_definition(index)?.is_lexical {
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
        if !self.local_definition(index)?.is_lexical {
            return Err(Error::internal(
                "lexical initialization referenced an ordinary local definition",
            ));
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
        if !self.local_definition(index)?.is_lexical {
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
