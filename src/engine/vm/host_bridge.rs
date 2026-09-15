//! Bytecode VM adapter and per-frame binding state.
//!
//! This module owns the translation between VM completions/errors and the
//! runtime's object, call, iterator, realm and captured-variable machinery.

use super::frames::ActiveFrameToken;
use crate::engine::vm::bindings::{
    FrameBinding, capture_frame_binding, close_frame_binding, read_frame_binding,
    write_frame_binding,
};
use crate::engine::vm::exception::runtime_error_to_vm_error;

use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::atom::Atom;
use crate::engine::builtins::native::{ArrayIteratorKind, NativeFunctionId, PrimitiveKind};
use crate::engine::code::bytecode::{
    ApplyKind, ArgumentsKind, DefineMethodKind, DynamicEnvironmentSource, EvalVariableSource,
    Instruction, PrivateNameSource,
};
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, EvalBindingSource,
    EvalEnvironment, FunctionKind, VariableDefinition,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
#[cfg(test)]
use crate::engine::code::runtime::PublishedFunctionData;
use crate::engine::code::runtime::PublishedFunctionSnapshot;
#[cfg(test)]
use crate::engine::heap::roots::VarRefRoot;

use crate::engine::heap::{BytecodeConstant, ContextId, ObjectPayload, RawValue};

use crate::engine::object::operations::{InternalSetResult, PropertyDefineOutcome};
use crate::engine::object::{
    CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PrivateNameRef,
    PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
#[cfg(test)]
use crate::engine::vm::bindings::closure_view_matches_cell;
use crate::engine::vm::call::NativeInvokeOutcome;
use crate::engine::vm::frames::ActiveFrameGuard;
use crate::engine::vm::{
    AppendStartOutcome, ArgumentListOutcome, BytecodePc, CallInput, Completion, DefineClassOutcome,
    DirectEvalInvocation, ForInNextOutcome, ForInStartOutcome, ForOfNextOutcome, ForOfStartOutcome,
    IteratorCloseOutcome, ToPrimitiveHint, Vm, VmActivation, VmHost, VmSuspendKind,
};
use std::rc::Rc;

#[cfg(feature = "stack-vm")]
pub(super) mod owned;

mod dynamic_environment;
mod eval_validation;
mod private_elements;
mod super_property;

// Relative order from pinned QuickJS 2026-06-04 `quickjs-atom.h`. QuickJS
// installs the complete generated atom table at runtime construction; this
// focused frontier pins every spelling that `js_operator_typeof` can return.
pub(crate) const TYPEOF_STATIC_ATOMS: [&str; 8] = [
    "function",
    "undefined",
    "number",
    "boolean",
    "string",
    "object",
    "symbol",
    "bigint",
];

pub(crate) use super::eval_bindings::{MaterializedEvalEnvironment, PreparedEvalEnvironment};

pub(crate) struct RuntimeVmHost {
    pub(super) runtime: Runtime,
    pub(super) active_frame_token: ActiveFrameToken,
    pub(super) current_realm: ContextId,
    /// Realm of the invocation which entered this bytecode frame. Derived
    /// constructor return-protocol errors are allocated here, unlike ordinary
    /// bytecode errors which belong to `current_realm`.
    pub(super) caller_realm: ContextId,
    pub(super) executable: PublishedFunctionSnapshot,
    /// Current callee retained for sloppy mapped `arguments.callee`.
    /// Detached host-only tests do not execute the arguments opcode.
    pub(super) current_function: Option<ObjectRef>,
    /// Authored call arity before the argument frame was padded to formal
    /// width. `arguments.length` and its dense prefix use this exact count.
    pub(super) actual_argument_count: usize,
    pub(super) closure_slots: crate::engine::vm::closure::ClosureSlots,
    pub(super) arguments: Vec<FrameBinding>,
    pub(super) locals: Vec<FrameBinding>,
    /// QuickJS can resume the same frame after a caught throw or a return
    /// unwind without emitting `CloseLocal` for captured lexical cells. Only
    /// cells captured at one of those exact boundaries may be reset in place
    /// by the next lexical scope entry.
    pub(super) reusable_captured_locals: Vec<bool>,
}

enum VmPropertyKeyConversion {
    Key(PropertyKey),
    Throw(Value),
}

impl RuntimeVmHost {
    /// Code and layout are taken from this host's sealed snapshot, never from
    /// caller-supplied slices. Keep ordinary and suspendable drivers separate.
    #[inline]
    pub(super) fn new_activation(
        &self,
        mut input: CallInput,
    ) -> Result<(Rc<[Instruction]>, VmActivation), Error> {
        if self.executable.root().is_none() {
            return Err(Error::internal(
                "unpublished host cannot execute published code",
            ));
        }
        let metadata = self.executable.metadata;
        if self.closure_count() != usize::from(metadata.closure_count) {
            return Err(Error::internal(
                "function object closure slot count does not match bytecode metadata",
            ));
        }
        let function = self
            .current_function
            .as_ref()
            .ok_or_else(|| Error::internal("published frame has no current function"))?
            .clone();
        let callee_global = input
            .callee_global(&self.runtime, self.current_realm)?
            .clone();
        let activation = VmActivation::new_in_realm(
            self.executable.frame_layout(),
            self.caller_realm,
            self.current_realm,
            function,
            input.this_value,
            input.new_target,
            callee_global,
        );
        Ok((self.executable.code.clone(), activation))
    }

    #[cfg(test)]
    pub(crate) fn empty_for_test(runtime: Runtime, current_realm: ContextId) -> Self {
        Self {
            runtime,
            active_frame_token: ActiveFrameToken(0),
            current_realm,
            caller_realm: current_realm,
            executable: PublishedFunctionSnapshot::empty_for_test(current_realm),
            current_function: None,
            actual_argument_count: 0,
            closure_slots: Default::default(),
            arguments: Vec::new(),
            locals: Vec::new(),
            reusable_captured_locals: Vec::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn eval_frame_for_test(
        runtime: Runtime,
        current_realm: ContextId,
        bytecode: &FunctionBytecodeRef,
        closure_slots: Vec<VarRefRoot>,
        arguments: Vec<Value>,
        locals: Vec<Value>,
    ) -> Result<Self, RuntimeError> {
        let executable = runtime.snapshot_function_bytecode(bytecode)?;
        let PublishedFunctionData {
            argument_definitions,
            local_definitions,
            realm,
            ..
        } = &*executable;
        let realm = *realm;
        if realm != current_realm {
            return Err(RuntimeError::Invariant(
                "test eval frame realm disagrees with its bytecode",
            ));
        }
        if arguments.len() != argument_definitions.len()
            || locals.len() != local_definitions.len()
            || closure_slots.len() != executable.frame_layout().closures().len()
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
            caller_realm: current_realm,
            executable,
            current_function: None,
            actual_argument_count: arguments.len(),
            closure_slots: closure_slots.into(),
            arguments: arguments.into_iter().map(FrameBinding::Direct).collect(),
            locals: locals.into_iter().map(FrameBinding::Direct).collect(),
            reusable_captured_locals: vec![false; frame_local_count],
        })
    }

    #[cfg(test)]
    pub(crate) fn eval_binding_is_captured_for_test(&self, source: EvalBindingSource) -> bool {
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
        self.executable
            .frame_layout()
            .locals()
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("local definition index is out of bounds"))
    }

    #[cfg(test)]
    fn argument_definition(&self, index: u16) -> Result<VariableDefinition, Error> {
        self.executable
            .argument_definitions
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("argument definition index is out of bounds"))
    }

    #[cfg(test)]
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

    fn prepare_direct_eval_environment(
        &self,
        index: u16,
        caller_strict: bool,
    ) -> Result<PreparedEvalEnvironment, Error> {
        let descriptor = self
            .executable
            .eval_environment(index)
            .ok_or_else(|| Error::internal("eval environment index is out of bounds"))?;
        // Publication authenticates the immutable topology and source modes.
        // Check this frame's actual slots before compiling or capturing anything.
        self.validate_eval_frame_bindings(&descriptor, caller_strict)?;
        Ok(PreparedEvalEnvironment { index, descriptor })
    }

    fn materialize_direct_eval_environment(
        &mut self,
        prepared: PreparedEvalEnvironment,
    ) -> Result<MaterializedEvalEnvironment, Error> {
        super::eval_bindings::materialize(prepared, &self.closure_slots, |source, descriptor| {
            let binding = match source {
                EvalBindingSource::Local(index) => self
                    .locals
                    .get_mut(usize::from(index))
                    .ok_or_else(|| Error::internal("eval local binding index is out of bounds"))?,
                EvalBindingSource::Argument(index) => {
                    self.arguments.get_mut(usize::from(index)).ok_or_else(|| {
                        Error::internal("eval argument binding index is out of bounds")
                    })?
                }
                EvalBindingSource::Closure(_) => {
                    return Err(Error::internal("eval closure reached frame capture"));
                }
            };
            capture_frame_binding(&self.runtime, binding, descriptor)
        })
    }

    fn lexical_uninitialized_error_with_visibility(
        &self,
        name: Option<Atom>,
        name_visible: bool,
    ) -> Result<Error, Error> {
        crate::engine::vm::bindings::lexical_uninitialized_error(&self.runtime, name, name_visible)
    }

    /// QuickJS strips vardef names per function when StripDebug was sampled
    /// and that function contains no syntactic direct eval. Oxide retains some
    /// of those atoms as publication/authentication metadata, so diagnostics
    /// must apply the same independent rule without deleting semantic names.
    fn local_lexical_uninitialized_error(&self, name: Option<Atom>) -> Result<Error, Error> {
        self.lexical_uninitialized_error_with_visibility(
            name,
            !self.executable.metadata.strip_variable_debug,
        )
    }

    fn lexical_read_only_error(&self, name: Option<Atom>) -> Result<Error, Error> {
        crate::engine::vm::bindings::lexical_read_only_error(&self.runtime, name)
    }

    fn constant_property_key(&self, index: u32) -> Result<PropertyKey, Error> {
        // Synthetic host-only unit tests have no published bytecode owner.
        // Production and published-code tests always require the linked table.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let name = match self.executable.constant(index) {
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
            return Ok(key);
        }
        let atom = usize::try_from(index)
            .ok()
            .and_then(|index| self.executable.property_key_atoms.as_ref()?.get(index))
            .copied()
            .filter(|atom| !atom.is_null())
            .ok_or_else(|| Error::internal("static name opcode has no linked property key"))?;
        PropertyKey::from_borrowed_atom(self.runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn eval_variable_object(&self, source: EvalVariableSource) -> Result<ObjectRef, Error> {
        super::environment_bindings::eval_variable_object(
            &self.runtime,
            &self.executable,
            source,
            |index| self.locals.get(usize::from(index)),
            &self.closure_slots,
        )
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

        if let Some(key) = self.runtime.immediate_numeric_property_key(&value) {
            return Ok(VmPropertyKeyConversion::Key(key));
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
        super::property_keys::canonical(&self.runtime, value)
    }

    fn finish_internal_set(
        &self,
        result: NativeConversion<InternalSetResult>,
        key: &PropertyKey,
        strict: bool,
    ) -> Result<Completion, Error> {
        self.runtime
            .finish_property_set(result, key, strict)
            .map_err(runtime_error_to_vm_error)
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
            Value::Object(object) => self
                .runtime
                .internal_get(self.current_realm, object, key, base.clone())
                .map_err(runtime_error_to_vm_error),
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
                    .internal_get(self.current_realm, &prototype, key, base.clone())
                    .map_err(runtime_error_to_vm_error)
            }
            Value::String(string) => self
                .runtime
                .get_string_property_with_receiver(self.current_realm, string, key, base.clone())
                .map_err(runtime_error_to_vm_error),
        }
    }

    fn set_property_with_key(
        &mut self,
        base: Value,
        key: &PropertyKey,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        if let Value::Object(object) = &base {
            let result = self
                .runtime
                .internal_set(self.current_realm, object, key, value, base.clone())
                .map_err(runtime_error_to_vm_error)?;
            return self.finish_internal_set(result, key, strict);
        }
        match &base {
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
                let result = self
                    .runtime
                    .internal_set(self.current_realm, &prototype, key, value, base.clone())
                    .map_err(runtime_error_to_vm_error)?;
                self.finish_internal_set(result, key, strict)
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
                Err(error)
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
                let result = self
                    .runtime
                    .internal_set(self.current_realm, &prototype, key, value, base.clone())
                    .map_err(runtime_error_to_vm_error)?;
                self.finish_internal_set(result, key, strict)
            }
            Value::Object(_) => unreachable!("object Set returned above"),
        }
    }

    fn delete_property_with_key(
        &mut self,
        base: Value,
        key: &PropertyKey,
        strict: bool,
    ) -> Result<Completion, Error> {
        let result = if let Value::Object(object) = &base {
            self.runtime
                .internal_delete_property(self.current_realm, object, key)
                .map_err(runtime_error_to_vm_error)?
        } else {
            NativeConversion::Value(
                self.runtime
                    .primitive_delete_property(&base, key)
                    .map_err(runtime_error_to_vm_error)?,
            )
        };
        self.runtime
            .finish_property_delete(result, strict)
            .map_err(runtime_error_to_vm_error)
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
        super::iterator_support::is_direct_native_target(&self.runtime, value, expected)
    }

    fn append_fast_array_values(
        &self,
        source: &Value,
        next_method: &Value,
        builtin_values_probe: bool,
    ) -> Result<Option<Vec<Value>>, Error> {
        super::iterator_support::append_fast_array_values(
            &self.runtime,
            source,
            next_method,
            builtin_values_probe,
        )
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
    /// Keep generator-only suspension payloads and match temporaries out of
    /// every ordinary bytecode call's native frame. Debug ARM64 stack budgets
    /// are intentionally measured on recursive ordinary calls, so this
    /// resumable tail must remain an outlined ownership boundary.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn start_generator_bytecode_callable(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        host: RuntimeVmHost,
        input: CallInput,
        active_frame: ActiveFrameGuard,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let result = super::suspend::start(host, input, arguments);
        active_frame.finish()?;
        match result? {
            super::suspend::VmRunOutcome::Suspend { activation, .. }
                if activation.kind == VmSuspendKind::Initial =>
            {
                self.finish_generator_function_call(caller_realm, callable, *activation)
            }
            super::suspend::VmRunOutcome::Suspend { .. } => Err(RuntimeError::Invariant(
                "generator call did not stop at its initial-yield barrier",
            )),
            super::suspend::VmRunOutcome::Complete(Completion::Throw(value)) => {
                Ok(Completion::Throw(value))
            }
            super::suspend::VmRunOutcome::Complete(Completion::Return(_)) => {
                Err(RuntimeError::Invariant(
                    "generator call completed before its initial-yield barrier",
                ))
            }
        }
    }

    // Return all prepared owners before dispatching bytecode or entering a
    // callback. Preparation temporaries must not consume recursive stack room.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn prepare_bytecode_host(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        this_value: Value,
        new_target: Value,
        arguments: &[Value],
        bytecode: FunctionBytecodeRef,
        closure_slots: crate::engine::vm::closure::ClosureSlots,
    ) -> Result<(RuntimeVmHost, CallInput, ActiveFrameGuard), RuntimeError> {
        let crate::engine::vm::call::PreparedBytecodeFrame {
            executable,
            active_frame,
            input,
            arguments: frame_arguments,
            locals: frame_locals,
        } = self.prepare_bytecode_frame(callable, this_value, new_target, arguments, bytecode)?;
        let realm = executable.realm;
        let frame_local_count = frame_locals.len();
        let host = RuntimeVmHost {
            runtime: self.clone(),
            active_frame_token: active_frame.token(),
            current_realm: realm,
            caller_realm,
            executable,
            current_function: Some(callable.as_object().clone()),
            actual_argument_count: arguments.len(),
            closure_slots,
            arguments: frame_arguments,
            locals: frame_locals,
            reusable_captured_locals: vec![false; frame_local_count],
        };
        Ok((host, input, active_frame))
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_bytecode_callable(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        this_value: Value,
        new_target: Value,
        arguments: &[Value],
        bytecode: FunctionBytecodeRef,
        closure_slots: crate::engine::vm::closure::ClosureSlots,
    ) -> Result<Completion, RuntimeError> {
        if self.bytecode_call_would_overflow() {
            return self.bytecode_stack_overflow_completion(caller_realm, &bytecode);
        }
        #[cfg(feature = "stack-vm")]
        if !bytecode.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        #[cfg(feature = "stack-vm")]
        if self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(bytecode.bytecode_id())?
            .metadata
            .function_kind
            == FunctionKind::Normal
        {
            return owned::execute_call(
                self,
                caller_realm,
                callable,
                this_value,
                new_target,
                arguments,
                bytecode,
                closure_slots,
            );
        }
        let (mut host, input, active_frame) = self.prepare_bytecode_host(
            caller_realm,
            callable,
            this_value,
            new_target,
            arguments,
            bytecode,
            closure_slots,
        )?;
        let metadata = host.executable.metadata;
        let is_module_link_entry = metadata.is_module && input.this_value == Value::Bool(true);
        // A module callable is deliberately an ordinary hidden function
        // object even though its root bytecode is Async. QuickJS invokes the
        // canonical `this = true` prefix synchronously during linking and
        // enters the async-function driver only for evaluation (`this` is
        // undefined there). The link prefix returns before any authored body
        // instruction, so the non-suspending VM driver is the exact boundary.
        if is_module_link_entry {
            let result = Vm::new().execute_published(input, &mut host);
            active_frame.finish()?;
            return result.map_err(RuntimeError::Engine);
        }
        match metadata.function_kind {
            FunctionKind::Generator => {
                return self.start_generator_bytecode_callable(
                    caller_realm,
                    callable,
                    host,
                    input,
                    active_frame,
                    arguments,
                );
            }
            FunctionKind::Async => {
                return self.start_async_bytecode_callable(
                    caller_realm,
                    host,
                    input,
                    active_frame,
                    arguments,
                );
            }
            FunctionKind::AsyncGenerator => {
                return self.start_async_generator_bytecode_callable(
                    caller_realm,
                    callable,
                    host,
                    input,
                    active_frame,
                    arguments,
                );
            }
            FunctionKind::Normal => {}
        }
        // Root handoff starts only after all owned preparation/driver Rust
        // frames have returned. The active bytecode guard remains here.
        #[cfg(feature = "stack-vm")]
        let result =
            owned::execute(host, input, arguments).and_then(|exit| exit.finish(self.clone()));
        #[cfg(not(feature = "stack-vm"))]
        let result = Vm::new().execute_published(input, &mut host);
        active_frame.finish()?;
        result.map_err(RuntimeError::Engine)
    }
}

impl VmHost for RuntimeVmHost {
    #[inline]
    fn static_branch_target(&self, target: u32, _code_len: usize) -> Result<usize, Error> {
        // Production execution takes code from this host's immutable snapshot.
        // The verifier checks every immediate target, including unreachable
        // instructions. Keep synthetic hosts on the checked path.
        #[cfg(test)]
        if self.executable.root().is_none() {
            return super::activation::checked_target(target, _code_len);
        }
        usize::try_from(target).map_err(|_| Error::internal("jump target overflow"))
    }

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
        match self.runtime.next_for_in(self.current_realm, &iterator) {
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

    fn for_await_of_start(&mut self, iterable: Value) -> Result<ForOfStartOutcome, Error> {
        match self
            .runtime
            .get_async_iterator_record(self.current_realm, iterable)
            .map_err(runtime_error_to_vm_error)?
        {
            NativeConversion::Value((iterator, next_method)) => Ok(ForOfStartOutcome::Record {
                iterator,
                next_method,
            }),
            NativeConversion::Throw(value) => Ok(ForOfStartOutcome::Throw(value)),
        }
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
            .try_call_native_iterator_next_raw(self.current_realm, &next_method, iterator.clone())
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
            Ok(Completion::Return(value)) => self
                .runtime
                .value_to_boolean(&value)
                .map_err(runtime_error_to_vm_error)?,
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

    fn iterator_get_value_done(&mut self, result: Value) -> Result<ForOfNextOutcome, Error> {
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
            Ok(Completion::Return(value)) => self
                .runtime
                .value_to_boolean(&value)
                .map_err(runtime_error_to_vm_error)?,
            Ok(Completion::Throw(value)) => return Ok(ForOfNextOutcome::Throw(value)),
            Err(error) => {
                return Ok(ForOfNextOutcome::Throw(
                    self.materialize_iterator_error(error)?,
                ));
            }
        };

        // QuickJS's async iterator completion helper always performs this Get,
        // including on the `done = true` path. This intentionally differs from
        // the synchronous ForOfNext hook above.
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
        Ok(ForOfNextOutcome::Result { value, done })
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
        super::pure_operations::load_value_constant(&self.runtime, &self.executable, index)
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

    fn to_boolean(&mut self, value: &Value) -> Result<bool, Error> {
        self.runtime
            .value_to_boolean(value)
            .map_err(runtime_error_to_vm_error)
    }

    fn is_html_dda(&mut self, value: &Value) -> Result<bool, Error> {
        self.runtime
            .value_is_html_dda(value)
            .map_err(runtime_error_to_vm_error)
    }

    fn is_callable(&mut self, value: &Value) -> Result<bool, Error> {
        self.runtime
            .value_is_callable(value)
            .map_err(runtime_error_to_vm_error)
    }

    fn type_of(&mut self, value: &Value) -> Result<JsString, Error> {
        super::pure_operations::type_of(&self.runtime, value)
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
        let constant = self
            .executable
            .constant(index)
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
                    #[cfg(test)]
                    if self.executable.root().is_none() {
                        self.validate_capture_definition(
                            self.local_definition(index)?,
                            descriptor,
                        )?;
                    }
                    let binding = self
                        .locals
                        .get_mut(usize::from(index))
                        .ok_or_else(|| Error::internal("captured local index is out of bounds"))?;
                    crate::engine::vm::bindings::capture_local_binding(
                        &self.runtime,
                        binding,
                        *self
                            .executable
                            .local_definitions
                            .get(usize::from(index))
                            .ok_or_else(|| {
                                Error::internal("local definition index is out of bounds")
                            })?,
                        descriptor,
                    )?
                }
                ClosureSource::ParentArgument(index) => {
                    #[cfg(test)]
                    if self.executable.root().is_none() {
                        self.validate_capture_definition(
                            self.argument_definition(index)?,
                            descriptor,
                        )?;
                    }
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
                        .validate_var_ref_metadata(&root, descriptor)
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
                ClosureSource::ModuleDeclaration
                | ClosureSource::ModuleImport
                | ClosureSource::ModuleImportCollision
                | ClosureSource::ModuleImportMeta => {
                    return Err(Error::internal(
                        "child closure attempted to resolve a module-root descriptor",
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
        let constant = self
            .executable
            .constant(name_index)
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
        let name = super::property_keys::computed_name(&self.runtime, key)?;
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
                let mapped_argument_count = self
                    .actual_argument_count
                    .min(self.executable.argument_definitions.len());
                let mut roots = Vec::with_capacity(self.actual_argument_count);
                for (index, binding) in self
                    .arguments
                    .iter_mut()
                    .take(mapped_argument_count)
                    .enumerate()
                {
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
                // `quickjs.c::js_build_mapped_arguments` shares VarRefs only
                // for `min(argc, b->arg_count)`. Extra actual arguments get
                // detached cells: they are observable through `arguments`,
                // but are not bytecode argument slots. Keeping that split is
                // also essential when a generator parks this frame.
                for binding in self
                    .arguments
                    .iter()
                    .take(self.actual_argument_count)
                    .skip(mapped_argument_count)
                {
                    let value = read_frame_binding(&self.runtime, binding)?;
                    roots.push(
                        self.runtime
                            .new_var_ref(value, false, false, ClosureVariableKind::Normal)
                            .map_err(runtime_error_to_vm_error)?,
                    );
                }
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
        if self
            .executable
            .metadata
            .eval_variable_object_local
            .is_none()
            && self.executable.arg_eval_variable_object_local.is_none()
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
        if self
            .executable
            .closure_variables
            .get(usize::from(index))
            .is_some_and(|descriptor| descriptor.kind.is_private())
        {
            return Err(Error::internal(
                "global reference referenced a private-name binding",
            ));
        }
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
        super::pure_operations::create_regexp(
            &self.runtime,
            self.current_realm,
            &self.executable,
            index,
        )
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
        let result =
            self.runtime
                .define_public_class_field(self.current_realm, &object, &key, value);
        self.finish_property_define(result)
    }

    fn define_field_computed(
        &mut self,
        base: Value,
        key: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = base else {
            return Err(Error::new(ErrorKind::Type, "not an object"));
        };
        // `ToPropKey` already performed the only observable conversion. This
        // helper accepts just its canonical VM representations and never calls
        // `property_key_from_value`.
        let key = self.canonical_property_key_from_value(&key)?;
        let result =
            self.runtime
                .define_public_class_field(self.current_realm, &object, &key, value);
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
        let name = match self.executable.constant(name) {
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

    fn install_class_instance_initializer(
        &mut self,
        constructor: Value,
        prototype: Value,
        initializer: Value,
    ) -> Result<Completion, Error> {
        self.runtime
            .install_class_instance_initializer(
                self.current_realm,
                constructor,
                prototype,
                initializer,
            )
            .map(|()| Completion::Return(Value::Undefined))
            .map_err(runtime_error_to_vm_error)
    }

    fn call_class_instance_initializer(
        &mut self,
        active_constructor: Value,
        receiver: Value,
    ) -> Result<Completion, Error> {
        self.runtime
            .call_class_instance_initializer(self.current_realm, active_constructor, receiver)
            .map_err(runtime_error_to_vm_error)
    }

    fn run_class_static_initializer(
        &mut self,
        constructor: Value,
        initializer: Value,
    ) -> Result<Completion, Error> {
        self.runtime
            .run_class_static_initializer(self.current_realm, constructor, initializer)
            .map_err(runtime_error_to_vm_error)
    }

    fn call_class_static_block(
        &mut self,
        static_initializer: ObjectRef,
        this_value: Value,
        block: Value,
    ) -> Result<Completion, Error> {
        self.runtime
            .call_class_static_block(self.current_realm, &static_initializer, this_value, block)
            .map_err(runtime_error_to_vm_error)
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
        use crate::engine::object::object_literal::element::{self, LiteralDefinitionStep};
        let step =
            LiteralDefinitionStep::start(&self.runtime, self.current_realm, object, index, value)
                .map_err(runtime_error_to_vm_error)?;
        element::finish(&self.runtime, self.current_realm, step).map_err(runtime_error_to_vm_error)
    }

    fn set_object_prototype(
        &mut self,
        object: Value,
        prototype: Value,
    ) -> Result<Completion, Error> {
        super::pure_operations::set_object_prototype(&self.runtime, object, prototype)
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
            .executable
            .closure_variables
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global closure index is out of bounds"))?;
        if descriptor.kind.is_private() {
            return Err(Error::internal(
                "global read referenced a private-name binding",
            ));
        }
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
        let Some(key) = super::environment_bindings::prepare_global_delete(
            &self.runtime,
            &self.executable,
            &self.closure_slots,
            index,
        )?
        else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let global_object = self
            .runtime
            .global_object_for_realm(self.current_realm)
            .map_err(runtime_error_to_vm_error)?;
        super::environment_bindings::operation::finish(
            &self.runtime,
            self.current_realm,
            super::environment_bindings::operation::EnvironmentStep::delete_global(
                self.current_realm,
                global_object,
                key,
            ),
        )
        .map_err(runtime_error_to_vm_error)
    }

    fn put_global_var(
        &mut self,
        index: u16,
        value: Value,
        initialize: bool,
        strict: bool,
    ) -> Result<Completion, Error> {
        let key = match super::environment_bindings::prepare_global_write(
            &self.runtime,
            &self.executable,
            &self.closure_slots,
            index,
            initialize,
        )? {
            super::environment_bindings::GlobalWrite::Cell(root) => {
                self.runtime
                    .write_var_ref(&root, value)
                    .map_err(runtime_error_to_vm_error)?;
                return Ok(Completion::Return(Value::Undefined));
            }
            super::environment_bindings::GlobalWrite::Property(key) => key,
        };
        let global_object = self
            .runtime
            .global_object_for_realm(self.current_realm)
            .map_err(runtime_error_to_vm_error)?;
        super::environment_bindings::operation::finish(
            &self.runtime,
            self.current_realm,
            super::environment_bindings::operation::EnvironmentStep::put(
                self.current_realm,
                global_object,
                key,
                value,
                strict,
                false,
            ),
        )
        .map_err(runtime_error_to_vm_error)
    }

    fn initialize_private_name(&mut self, index: u16) -> Result<(), Error> {
        self.initialize_private_name_binding(index)
    }

    fn initialize_private_method(
        &mut self,
        index: u16,
        home_object: Value,
        method: Value,
    ) -> Result<(), Error> {
        self.initialize_private_method_binding(index, home_object, method)
    }

    fn initialize_private_accessor(
        &mut self,
        index: u16,
        home_object: Value,
        accessor: Value,
    ) -> Result<(), Error> {
        self.initialize_private_accessor_binding(index, home_object, accessor)
    }

    fn get_private_field(
        &mut self,
        source: PrivateNameSource,
        base: Value,
    ) -> Result<Completion, Error> {
        self.get_private_field_value(source, base)
    }

    fn put_private_field(
        &mut self,
        source: PrivateNameSource,
        base: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        self.put_private_field_value(source, base, value)
    }

    fn define_private_field(
        &mut self,
        source: PrivateNameSource,
        base: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        self.define_private_field_value(source, base, value)
    }

    fn private_in(&mut self, source: PrivateNameSource, base: Value) -> Result<Completion, Error> {
        self.private_in_value(source, base)
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

    fn dynamic_import(&mut self, specifier: Value, options: Value) -> Result<Completion, Error> {
        let step = crate::engine::modules::import::ImportStep::start(
            &self.runtime,
            self.current_realm,
            self.executable.root(),
            specifier,
            options,
        )
        .map_err(runtime_error_to_vm_error)?;
        crate::engine::modules::import::finish(&self.runtime, self.current_realm, step)
            .map_err(runtime_error_to_vm_error)
    }

    fn call(
        &mut self,
        function: Value,
        this_value: Value,
        arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        self.call_with_borrowed_arguments(function, this_value, &arguments)
    }

    fn call_with_borrowed_arguments(
        &mut self,
        function: Value,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, Error> {
        self.runtime
            .call_value_internal(self.current_realm, function, this_value, arguments)
            .map_err(runtime_error_to_vm_error)
    }

    fn apply(
        &mut self,
        function: Value,
        this_or_new_target: Value,
        argument_array: Value,
        kind: ApplyKind,
    ) -> Result<Completion, Error> {
        // Pinned QuickJS's js_function_apply checks callability before its
        // nullish-list shortcut and build_arg_list for both magic values.
        // Constructor capability deliberately remains after list construction;
        // constructor-mode newTarget is forwarded as the untouched raw value.
        let callable = self
            .runtime
            .callable_from_value(function.clone())
            .map_err(runtime_error_to_vm_error)?;
        if matches!(argument_array, Value::Undefined | Value::Null) {
            return self
                .runtime
                .call_internal(self.current_realm, &callable, this_or_new_target, &[])
                .map_err(runtime_error_to_vm_error);
        }
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
                .construct_callable_with_raw_new_target_internal(
                    self.current_realm,
                    &callable,
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
            .construct_value_with_raw_new_target_internal(
                self.current_realm,
                function,
                new_target,
                &arguments,
            )
            .map_err(runtime_error_to_vm_error)
    }

    fn init_derived_constructor(
        &mut self,
        active_function: ObjectRef,
        new_target: Value,
    ) -> Result<Completion, Error> {
        if matches!(new_target, Value::Undefined) {
            return Err(Error::new(
                ErrorKind::Type,
                "class constructors must be invoked with 'new'",
            ));
        }
        if self.current_function.as_ref() != Some(&active_function) {
            return Err(Error::internal(
                "derived constructor initializer received a non-active function",
            ));
        }
        if self.actual_argument_count > self.arguments.len() {
            return Err(Error::internal(
                "derived constructor actual argument count exceeds its frame",
            ));
        }

        // `super()` is deliberately resolved from the function object's live
        // [[Prototype]]. Object.setPrototypeOf on the derived constructor is
        // therefore observable, matching QuickJS and ECMAScript GetSuperConstructor.
        let super_constructor = self
            .runtime
            .get_prototype_of(&active_function)
            .map_err(runtime_error_to_vm_error)?
            .map_or(Value::Null, Value::Object);
        let arguments = self.arguments[..self.actual_argument_count]
            .iter()
            .map(|binding| read_frame_binding(&self.runtime, binding))
            .collect::<Result<Vec<_>, _>>()?;
        self.runtime
            .construct_value_with_raw_new_target_internal(
                self.current_realm,
                super_constructor,
                new_target,
                &arguments,
            )
            .map_err(runtime_error_to_vm_error)
    }

    fn closure_count(&self) -> usize {
        self.closure_slots.len()
    }

    fn get_local(&mut self, index: u16) -> Result<Value, Error> {
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let definition = self.local_definition(index)?;
            if definition.kind == ClosureVariableKind::WithObject {
                return Err(Error::internal(
                    "ordinary local read referenced a private with object",
                ));
            }
            if definition.kind.is_private() {
                return Err(Error::internal(
                    "ordinary local read referenced a private-name binding",
                ));
            }
            if definition.is_lexical {
                return Err(Error::internal(
                    "unchecked local read referenced a lexical definition",
                ));
            }
        }
        let binding = self
            .locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        read_frame_binding(&self.runtime, binding)
    }

    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let definition = self.local_definition(index)?;
            if definition.kind == ClosureVariableKind::WithObject {
                return Err(Error::internal(
                    "ordinary local write referenced a private with object",
                ));
            }
            if definition.kind.is_private() {
                return Err(Error::internal(
                    "ordinary local write referenced a private-name binding",
                ));
            }
            if definition.is_lexical {
                return Err(Error::internal(
                    "unchecked local write referenced a lexical definition",
                ));
            }
        }
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        write_frame_binding(&self.runtime, binding, value)
    }

    fn set_local_uninitialized(&mut self, index: u16) -> Result<(), Error> {
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let definition = self.local_definition(index)?;
            if !definition.is_lexical {
                return Err(Error::internal(
                    "lexical scope entry referenced an ordinary local definition",
                ));
            }
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
            return crate::engine::vm::bindings::reset_captured_binding(
                &self.runtime,
                &root,
                reusable,
            );
        }

        *binding = FrameBinding::Uninitialized;
        Ok(())
    }

    fn get_local_checked(&mut self, index: u16) -> Result<Value, Error> {
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let definition = self.local_definition(index)?;
            if definition.kind.is_private() {
                return Err(Error::internal(
                    "checked local read referenced a private-name binding",
                ));
            }
            if !definition.is_lexical {
                return Err(Error::internal(
                    "checked local read referenced an ordinary definition",
                ));
            }
        }
        let binding = self
            .locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        match binding {
            FrameBinding::Direct(value) => Ok(value.clone()),
            FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => Err(Error::internal(
                "checked local read reached a private-element frame cell",
            )),
            FrameBinding::Uninitialized => {
                Err(self.local_lexical_uninitialized_error(self.local_definition(index)?.name)?)
            }
            FrameBinding::Captured(root) => {
                let raw = self
                    .runtime
                    .raw_var_ref_value(&root)
                    .map_err(runtime_error_to_vm_error)?;
                if matches!(raw, RawValue::Uninitialized) {
                    Err(self.local_lexical_uninitialized_error(self.local_definition(index)?.name)?)
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
        // Publication proves the operand mode; actual binding state stays dynamic.
        #[cfg(test)]
        if self.executable.root().is_none() {
            if definition.kind.is_private() {
                return Err(Error::internal(
                    "ordinary lexical initialization referenced a private-name binding",
                ));
            }
            if !definition.is_lexical && definition.kind != ClosureVariableKind::WithObject {
                return Err(Error::internal(
                    "local initialization referenced an ordinary local definition",
                ));
            }
        }
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        crate::engine::vm::bindings::initialize_local_binding(
            &self.runtime,
            definition.kind,
            binding,
            value,
        )
    }

    fn initialize_derived_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        if let Some(binding) = crate::engine::vm::bindings::initialize_derived_binding(
            &self.runtime,
            self.local_definition(index)?,
            self.locals.get(usize::from(index)),
            value,
        )? {
            self.locals[usize::from(index)] = binding;
        }
        Ok(())
    }

    fn put_local_checked(&mut self, index: u16, value: Value) -> Result<(), Error> {
        // Publication proves the operand mode; actual binding state stays dynamic.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let definition = self.local_definition(index)?;
            if definition.kind.is_private() {
                return Err(Error::internal(
                    "checked local write referenced a private-name binding",
                ));
            }
            if !definition.is_lexical {
                return Err(Error::internal(
                    "checked local write referenced an ordinary definition",
                ));
            }
        }
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        match binding {
            FrameBinding::Direct(slot) => {
                #[cfg(test)]
                if self.executable.root().is_none() {
                    let definition = self.executable.local_definitions[usize::from(index)];
                    if definition.is_const {
                        return Err(self.lexical_read_only_error(definition.name)?);
                    }
                }
                *slot = value;
                Ok(())
            }
            FrameBinding::Private(_) | FrameBinding::PrivateCallable(_) => Err(Error::internal(
                "checked local write reached a private-element frame cell",
            )),
            FrameBinding::Uninitialized => {
                Err(self.local_lexical_uninitialized_error(self.local_definition(index)?.name)?)
            }
            FrameBinding::Captured(root) => {
                let (uninitialized, is_const) = {
                    let state = self.runtime.0.state.borrow();
                    let cell = state
                        .heap
                        .var_ref(root.id())
                        .map_err(|error| Error::internal(error.to_string()))?;
                    (matches!(cell.value, RawValue::Uninitialized), cell.is_const)
                };
                if uninitialized {
                    return Err(
                        self.local_lexical_uninitialized_error(self.local_definition(index)?.name)?
                    );
                }
                if is_const {
                    return Err(self.lexical_read_only_error(self.local_definition(index)?.name)?);
                }
                self.runtime
                    .write_var_ref(&root, value)
                    .map_err(runtime_error_to_vm_error)
            }
        }
    }

    fn close_local(&mut self, index: u16) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        // Publication proves the operand mode; actual binding state stays dynamic.
        #[cfg(test)]
        if self.executable.root().is_none() {
            if !definition.is_lexical && definition.kind != ClosureVariableKind::WithObject {
                return Err(Error::internal(
                    "CloseLocal referenced an ordinary local definition",
                ));
            }
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
        close_frame_binding(&self.runtime, binding, definition.kind)
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
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let descriptor = self
                .executable
                .closure_variables
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            if descriptor.kind.is_private() {
                return Err(Error::internal(
                    "ordinary closure read referenced a private-name binding",
                ));
            }
            if descriptor.is_lexical {
                return Err(Error::internal(
                    "unchecked closure read referenced a lexical binding",
                ));
            }
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .read_var_ref(&root)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn put_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error> {
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let descriptor = self
                .executable
                .closure_variables
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            if descriptor.kind.is_private() {
                return Err(Error::internal(
                    "ordinary closure write referenced a private-name binding",
                ));
            }
            if descriptor.is_lexical {
                return Err(Error::internal(
                    "unchecked closure write referenced a lexical binding",
                ));
            }
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .write_var_ref(&root, value)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn get_var_ref_checked(&mut self, index: u16) -> Result<Value, Error> {
        // Published instructions already authenticate this access mode.
        // Synthetic host tests keep their checked internal-operation contract.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let descriptor = self
                .executable
                .closure_variables
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            if descriptor.kind.is_private() {
                return Err(Error::internal(
                    "checked closure read referenced a private-name binding",
                ));
            }
            if !descriptor.is_lexical {
                return Err(Error::internal(
                    "checked closure read referenced an ordinary binding",
                ));
            }
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        crate::engine::vm::bindings::read_checked_closure(
            &self.runtime,
            &root,
            self.executable.closure_variables[usize::from(index)],
            self.executable.metadata.strip_variable_debug,
        )
    }

    fn put_var_ref_checked(&mut self, index: u16, value: Value) -> Result<(), Error> {
        // Publication proves the operand mode; actual binding state stays dynamic.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let descriptor = self
                .executable
                .closure_variables
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            if descriptor.kind.is_private() {
                return Err(Error::internal(
                    "checked closure write referenced a private-name binding",
                ));
            }
            if !descriptor.is_lexical {
                return Err(Error::internal(
                    "checked closure write referenced an ordinary binding",
                ));
            }
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        crate::engine::vm::bindings::write_checked_closure(
            &self.runtime,
            &root,
            self.executable.closure_variables[usize::from(index)],
            self.executable.metadata.strip_variable_debug,
            value,
        )
    }

    fn initialize_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error> {
        // Publication proves the operand mode; actual binding state stays dynamic.
        #[cfg(test)]
        if self.executable.root().is_none() {
            let descriptor = self
                .executable
                .closure_variables
                .get(usize::from(index))
                .copied()
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            if descriptor.source != ClosureSource::ModuleDeclaration
                || !descriptor.is_lexical
                || descriptor.kind != ClosureVariableKind::Normal
            {
                return Err(Error::internal(
                    "module lexical initialization referenced a non-declaration binding",
                ));
            }
        }
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?
            .clone();
        self.runtime
            .write_var_ref(&root, value)
            .map_err(runtime_error_to_vm_error)
    }

    fn initialize_module_import_collision(
        &mut self,
        index: u16,
        value: Value,
    ) -> Result<(), Error> {
        let descriptor = self
            .executable
            .closure_variables
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        super::bindings::validate_module_import_collision(descriptor)?;
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .write_var_ref(&root, value)
            .map_err(runtime_error_to_vm_error)
    }

    fn initialize_derived_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let descriptor = self
            .executable
            .closure_variables
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        super::bindings::initialize_derived_closure(&self.runtime, &root, descriptor, value)
    }

    fn return_derived(&mut self, index: u16, value: Value) -> Result<Completion, Error> {
        crate::engine::vm::bindings::finish_derived_return(
            &self.runtime,
            self.caller_realm,
            self.local_definition(index)?,
            self.locals.get(usize::from(index)),
            value,
        )
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "stack-vm")]
    #[test]
    fn owned_root_rejects_foreign_bytecode_before_looking_up_its_raw_id() {
        let foreign = Runtime::new();
        let mut foreign_context = foreign.new_context();
        let callable = CallableRef::from_validated_object(eval_object(
            &mut foreign_context,
            "(function(){return 42})",
        ));
        let crate::engine::vm::call::CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } = foreign.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("fixture must be bytecode");
        };
        let runtime = Runtime::new();
        let context = runtime.new_context();
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode(bytecode.bytecode_id())
                .is_err()
        );
        let result = runtime.execute_bytecode_callable(
            context.realm,
            &callable,
            Value::Undefined,
            Value::Undefined,
            &[],
            bytecode,
            closure_slots,
        );
        assert!(matches!(
            result,
            Err(RuntimeError::WrongRuntime("function bytecode"))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert!(foreign.0.state.borrow().active_frames.is_empty());
    }

    #[cfg(feature = "stack-vm")]
    #[test]
    fn owned_root_closure_mismatch_keeps_error_shape_and_retires_active_guard() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let callable = CallableRef::from_validated_object(eval_object(
            &mut context,
            "(function(){let captured=42; return function(){return captured}})()",
        ));
        let crate::engine::vm::call::CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        } = runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("fixture must be bytecode");
        };
        assert!(!closure_slots.is_empty());
        let result = runtime.execute_bytecode_callable(
            context.realm,
            &callable,
            Value::Undefined,
            Value::Undefined,
            &[],
            bytecode,
            Default::default(),
        );
        assert!(matches!(
            result,
            Err(RuntimeError::Engine(error))
                if error.message() == "function object closure slot count does not match bytecode metadata"
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        // The rejected entry must not disturb later ordinary root calls.
        assert_eq!(
            context.eval("(function(){return 42})()").unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn unpublished_host_cannot_enter_published_execution() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
        let error = Vm::new()
            .execute_published(
                CallInput {
                    this_value: Value::Undefined,
                    new_target: Value::Undefined,
                    callee_global: Some(runtime.global_object_for_realm(context.realm).unwrap()),
                },
                &mut host,
            )
            .unwrap_err();
        assert_eq!(
            error.message(),
            "unpublished host cannot execute published code"
        );
    }

    use super::*;
    use crate::engine::api::Context;
    use crate::engine::code::bytecode::EvalVariableSource;
    use crate::engine::heap::PromiseState;
    use crate::engine::object::CompleteOrdinaryPropertyDescriptor;

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
        host.executable.constants = Rc::from([BytecodeConstant::Value(RawValue::String(
            JsString::from_static("added"),
        ))]);
        host.executable.local_definitions = Rc::from([VariableDefinition {
            name: None,
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind,
        }]);
        host.executable.metadata.eval_variable_object_local = authenticated_local;
        host.locals = vec![FrameBinding::Direct(Value::Object(object.clone()))];
        host.reusable_captured_locals = vec![false];
        (host, object)
    }

    fn derived_this_definition() -> VariableDefinition {
        VariableDefinition {
            name: None,
            is_lexical: true,
            is_const: false,
            is_parameter_initializer: false,
            kind: ClosureVariableKind::Normal,
        }
    }

    #[test]
    fn dynamic_import_without_an_active_filename_rejects_in_the_load_job() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
        let Completion::Return(Value::Object(promise)) = host
            .dynamic_import(Value::Int(20), Value::Undefined)
            .unwrap()
        else {
            panic!("dynamic import did not return its Promise");
        };
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .promise_snapshot(promise.object_id())
                .unwrap()
                .state,
            PromiseState::Pending
        );
        assert!(runtime.execute_pending_job().unwrap().executed());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .promise_snapshot(promise.object_id())
                .unwrap()
                .state,
            PromiseState::Rejected
        );
    }

    #[test]
    fn only_a_sealed_module_import_view_may_alias_different_cell_flags() {
        let ordinary_const = ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: ClosureVariableName::None,
            is_lexical: true,
            is_const: true,
            kind: ClosureVariableKind::Normal,
        };
        assert!(!closure_view_matches_cell(
            (true, false, ClosureVariableKind::Normal),
            ordinary_const,
        ));

        let import_view = ClosureVariable {
            kind: ClosureVariableKind::ModuleImportView,
            ..ordinary_const
        };
        assert!(closure_view_matches_cell(
            (true, false, ClosureVariableKind::Normal),
            import_view,
        ));
        assert!(closure_view_matches_cell(
            (false, false, ClosureVariableKind::Normal),
            import_view,
        ));
    }

    #[test]
    fn derived_this_initializers_are_one_shot_for_locals_and_var_refs() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let first = runtime.new_object(None).unwrap();
        let second = runtime.new_object(None).unwrap();

        let mut local_host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
        local_host.executable.local_definitions = Rc::from([derived_this_definition()]);
        local_host.locals = vec![FrameBinding::Uninitialized];
        local_host.reusable_captured_locals = vec![false];
        local_host
            .initialize_derived_local(0, Value::Object(first.clone()))
            .unwrap();
        assert_eq!(
            local_host.get_local_checked(0).unwrap(),
            Value::Object(first)
        );
        let error = local_host
            .initialize_derived_local(0, Value::Object(second.clone()))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Reference);
        assert_eq!(error.message(), "'this' can be initialized only once");

        let this_key = runtime.intern_property_key("<this>").unwrap();
        let root = runtime
            .new_uninitialized_captured_var_ref(true, false, ClosureVariableKind::Normal)
            .unwrap();
        let mut closure_host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
        closure_host.executable.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::Atom(this_key.atom()),
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }]);
        closure_host.closure_slots = vec![root.clone()].into();
        closure_host
            .initialize_derived_var_ref(0, Value::Object(second.clone()))
            .unwrap();
        assert_eq!(runtime.read_var_ref(&root).unwrap(), Value::Object(second));
        let replacement = runtime.new_object(None).unwrap();
        let error = closure_host
            .initialize_derived_var_ref(0, Value::Object(replacement))
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Reference);
        assert_eq!(error.message(), "this is not initialized");
    }

    #[test]
    fn derived_return_errors_are_created_in_the_caller_realm() {
        let runtime = Runtime::new();
        let mut defining = runtime.new_context();
        let mut caller = runtime.new_context();
        let defining_reference = eval_object(&mut defining, "ReferenceError.prototype");
        let caller_reference = eval_object(&mut caller, "ReferenceError.prototype");
        let caller_type = eval_object(&mut caller, "TypeError.prototype");
        assert_ne!(defining_reference, caller_reference);

        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), defining.realm);
        host.caller_realm = caller.realm;
        host.executable.local_definitions = Rc::from([derived_this_definition()]);
        host.locals = vec![FrameBinding::Uninitialized];
        host.reusable_captured_locals = vec![false];

        let Completion::Throw(Value::Object(missing_this)) =
            host.return_derived(0, Value::Undefined).unwrap()
        else {
            panic!("missing derived this did not throw an Object")
        };
        assert_eq!(
            runtime.get_prototype_of(&missing_this).unwrap(),
            Some(caller_reference)
        );

        let Completion::Throw(Value::Object(primitive_return)) =
            host.return_derived(0, Value::Int(1)).unwrap()
        else {
            panic!("primitive derived return did not throw an Object")
        };
        assert_eq!(
            runtime.get_prototype_of(&primitive_return).unwrap(),
            Some(caller_type)
        );

        // An explicit Object return succeeds without observing the still-TDZ
        // `this` binding.
        let explicit = runtime.new_object(None).unwrap();
        assert_eq!(
            host.return_derived(0, Value::Object(explicit.clone()))
                .unwrap(),
            Completion::Return(Value::Object(explicit))
        );
    }

    #[test]
    fn default_derived_initialization_uses_live_super_raw_args_and_new_target() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let base = eval_object(
            &mut context,
            "(function Base(a, b) { \
                this.sum = a + b; \
                this.count = arguments.length; \
                this.rawNewTarget = new.target; \
            })",
        );
        let active = eval_object(&mut context, "(function Derived() {})");
        assert!(runtime.set_prototype_of(&active, Some(&base)).unwrap());

        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
        host.current_function = Some(active.clone());
        host.actual_argument_count = 2;
        host.arguments = vec![
            FrameBinding::Direct(Value::Int(20)),
            FrameBinding::Direct(Value::Int(22)),
            // Frame padding or later slots must not become forwarded actuals.
            FrameBinding::Direct(Value::Int(999)),
        ];
        let Completion::Return(Value::Object(instance)) = host
            .init_derived_constructor(active.clone(), Value::Int(17))
            .unwrap()
        else {
            panic!("default derived initialization did not construct an Object")
        };

        let sum = runtime.intern_property_key("sum").unwrap();
        let count = runtime.intern_property_key("count").unwrap();
        assert_eq!(
            context.get_property(&instance, &sum).unwrap(),
            Value::Int(42)
        );
        assert_eq!(
            context.get_property(&instance, &count).unwrap(),
            Value::Int(2)
        );
        let raw_new_target = runtime.intern_property_key("rawNewTarget").unwrap();
        assert_eq!(
            context.get_property(&instance, &raw_new_target).unwrap(),
            Value::Int(17)
        );
        assert_eq!(
            runtime.get_prototype_of(&instance).unwrap(),
            Some(context.object_prototype().unwrap())
        );
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
        closure.executable.constants = Rc::from([BytecodeConstant::Value(RawValue::String(
            JsString::from_static("added"),
        ))]);
        closure.executable.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }]);
        closure.closure_slots = vec![root].into();
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
        closure.executable.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::ParentClosure(0),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::EvalVariableObject,
        }]);
        closure.closure_slots = vec![root].into();
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
        host.executable.local_definitions = Rc::from([VariableDefinition {
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

        host.executable.local_definitions = Rc::from([VariableDefinition {
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
}
