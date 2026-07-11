//! Runtime and context ownership boundaries.
//!
//! As in QuickJS, a runtime owns resources shared by multiple contexts, while
//! each context is a separate realm and execution surface. The heap and
//! intrinsics extend this boundary; they are not hidden in the compiler or VM.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::error::Error as StdError;
use std::fmt;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::atom::{Atom, AtomError, AtomKind, AtomTable};
use crate::bytecode::verify_parts;
use crate::compiler::{
    CompileOptions, DEFAULT_EVAL_FILENAME, compile_unlinked_script_with_filename,
};
use crate::debug::{DebugInfoMode, LineColumn, QuickJsSourceLocator};
use crate::error::{Error, ErrorKind, NativeErrorKind};
use crate::function::{
    FunctionBytecodeRef, UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug,
};
use crate::heap::{
    AutoInitProperty, BytecodeConstant, ClosureSource, ClosureVariable, ClosureVariableKind,
    ClosureVariableName, ConstructorKind, ContextData, ContextId, DynamicFunctionKind,
    ErrorConstructorKind, FunctionBytecodeData, FunctionBytecodeId, FunctionDebugInfo,
    FunctionDebugPosition, FunctionKind, FunctionMetadata, GcStats, Heap, HeapCleanup, HeapCounts,
    HeapError, NativeCProto, NativeFunctionId, NumberParseKind, ObjectData, ObjectId, ObjectKind,
    ObjectPayload, PrimitiveKind, PrimitiveObjectData, PropertySlot, RawValue, ShapeId, VarRefData,
    VarRefId,
};
use crate::object::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, SymbolRef, WellKnownSymbol,
};
use crate::property::{
    CompletePropertyDescriptor, PropertyDefinitionError, PropertyDescriptor,
    validate_and_apply_property_descriptor,
};
use crate::shape::{PropertyFlags, Shape, ShapeEntry, ShapeError};
use crate::value::JsString;
use crate::value::Value;
use crate::vm::{BytecodePc, CallInput, Completion, ToPrimitiveHint, Vm, VmHost};

static NEXT_RUNTIME_DOMAIN_ID: AtomicU64 = AtomicU64::new(1);

struct RuntimeInner {
    state: RefCell<RuntimeState>,
    deferred_references: RefCell<VecDeque<DeferredRefOp>>,
    next_context_id: Cell<u64>,
    domain_id: u64,
}

#[derive(Clone, Copy, Debug)]
enum DeferredRefOp {
    Object(ObjectId),
    Context(ContextId),
    FunctionBytecode(FunctionBytecodeId),
    VarRef(VarRefId),
    Atom(Atom),
    ActiveFramePop {
        token: ActiveFrameToken,
        depth: usize,
    },
    BacktraceBarrierRestore {
        token: ActiveFrameToken,
        previous: bool,
    },
}

struct RuntimeOperation<'a>(&'a Runtime);

impl Drop for RuntimeOperation<'_> {
    fn drop(&mut self) {
        let result = self.0.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
    }
}

struct RuntimeState {
    atoms: AtomTable,
    heap: Heap,
    /// Runtime-owned pending JavaScript exception. Object and Symbol payloads
    /// carry one manually retained root; no public `Value::Exception` sentinel
    /// exists.
    pending_exception: Option<RawValue>,
    /// Observable function-debug portion of QuickJS `JS_SetStripInfo`, sampled
    /// by each subsequent compilation.
    debug_info_mode: DebugInfoMode,
    /// QuickJS's shape hash is non-owning. These generational IDs are likewise
    /// weak and are validated before reuse.
    shape_cache: HashMap<ShapeFingerprint, ShapeId>,
    shape_fingerprints: HashMap<ShapeId, ShapeFingerprint>,
    well_known_symbols: HashMap<WellKnownSymbol, Atom>,
    /// Unified QuickJS-style execution-frame chain. Records contain only raw
    /// stable identities and diagnostic state; the corresponding stack-local
    /// [`ActiveFrameGuard`] owns the object and bytecode roots.
    active_frames: Vec<ActiveFrameRecord>,
    next_active_frame_token: u64,
    #[cfg(test)]
    active_frame_probe_snapshots: Vec<Vec<ActiveFrameRecord>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveFrameRecord {
    token: ActiveFrameToken,
    function: ObjectId,
    realm: ContextId,
    flags: ActiveFrameFlags,
    kind: ActiveFrameKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ActiveFrameToken(u64);

/// Flags which belong to a QuickJS stack frame rather than to the callable
/// heap object. Async execution and backtrace barriers are reserved here for
/// the later stack/debug-info slice even though this first infrastructure step
/// only populates `strict`.
#[allow(dead_code)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ActiveFrameFlags {
    strict: bool,
    is_async: bool,
    backtrace_barrier: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ActiveFrameKind {
    Bytecode {
        bytecode: FunctionBytecodeId,
        pc: Option<BytecodePc>,
    },
    Native {
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    },
}

#[derive(Clone, Debug)]
struct ExplicitBacktraceLocation {
    filename: JsString,
    position: LineColumn,
}

enum RawStringProperty {
    Missing,
    String(JsString),
    Other,
}

enum NativeInvocation {
    Call { this_value: Value },
    Construct { new_target: Value },
    Getter { this_value: Value },
}

struct NativeArguments {
    actual_arg_count: usize,
    readable: Vec<Value>,
}

enum NativeConversion<T> {
    Value(T),
    Throw(Value),
}

enum Compilation {
    Published(FunctionBytecodeRef),
    Throw(Value),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct ShapeFingerprint {
    prototype: Option<ObjectId>,
    entries: Box<[ShapeEntry]>,
}

enum PropertySnapshot {
    Data {
        value: RawValue,
        flags: PropertyFlags,
    },
    VarRef {
        var_ref: VarRefId,
        flags: PropertyFlags,
    },
    Accessor {
        get: Option<ObjectId>,
        set: Option<ObjectId>,
        flags: PropertyFlags,
    },
    AutoInit,
}

enum PropertyGetAction {
    Complete(Value),
    Call {
        getter: CallableRef,
        receiver: Value,
    },
}

enum PropertySetAction {
    Complete,
    Rejected(PropertySetRejection),
    Call {
        setter: CallableRef,
        receiver: Value,
        argument: Value,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PropertySetRejection {
    ReadOnly,
    NoSetter,
    NotExtensible,
    NotObject,
}

/// Immutable VM inputs detached from the runtime `RefCell` borrow.
///
/// `constants` contains raw heap identities, so the owning bytecode root is
/// part of the snapshot. The raw constant pool therefore cannot outlive the GC
/// node whose edges keep those identities valid.
struct PublishedFunctionSnapshot {
    root: FunctionBytecodeRef,
    code: Rc<[crate::bytecode::Instruction]>,
    constants: Rc<[BytecodeConstant]>,
    closure_variables: Rc<[ClosureVariable]>,
    metadata: FunctionMetadata,
    realm: ContextId,
}

enum CallableExecution {
    Bytecode {
        bytecode: FunctionBytecodeRef,
        closure_slots: Vec<VarRefRoot>,
    },
    Native {
        target: NativeFunctionId,
        realm: ContextId,
        min_readable_args: u8,
    },
    Bound {
        target: CallableRef,
        this_value: Value,
        arguments: Vec<Value>,
    },
}

struct VarRefRoot {
    runtime: Runtime,
    id: VarRefId,
}

impl VarRefRoot {
    fn from_owned_handle(runtime: Runtime, id: VarRefId) -> Self {
        Self { runtime, id }
    }

    fn from_borrowed_handle(runtime: Runtime, id: VarRefId) -> Result<Self, HeapError> {
        runtime.retain_var_ref_handle(id)?;
        Ok(Self { runtime, id })
    }

    const fn id(&self) -> VarRefId {
        self.id
    }

    fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }
}

impl Clone for VarRefRoot {
    fn clone(&self) -> Self {
        self.runtime
            .retain_var_ref_handle(self.id)
            .expect("a live VarRef root must retain its cell");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
        }
    }
}

impl Drop for VarRefRoot {
    fn drop(&mut self) {
        self.runtime.release_var_ref_handle(self.id);
    }
}

enum FrameBinding {
    Direct(Value),
    Captured(VarRefRoot),
}

fn read_frame_binding(runtime: &Runtime, binding: &FrameBinding) -> Result<Value, Error> {
    match binding {
        FrameBinding::Direct(value) => Ok(value.clone()),
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
        FrameBinding::Captured(root) => {
            runtime
                .validate_var_ref_metadata(root, descriptor)
                .map_err(|error| Error::internal(error.to_string()))?;
            Ok(root.clone())
        }
    }
}

fn runtime_error_to_vm_error(error: RuntimeError) -> Error {
    match error {
        RuntimeError::Engine(error) => error,
        error => Error::internal(error.to_string()),
    }
}

struct RuntimeVmHost {
    runtime: Runtime,
    active_frame_token: ActiveFrameToken,
    current_realm: ContextId,
    constants: Rc<[BytecodeConstant]>,
    closure_variables: Rc<[ClosureVariable]>,
    closure_slots: Vec<VarRefRoot>,
    arguments: Vec<FrameBinding>,
    locals: Vec<FrameBinding>,
}

enum VmPropertyKeyConversion {
    Key(PropertyKey),
    Throw(Value),
}

impl RuntimeVmHost {
    fn constant_property_key(&self, index: u32) -> Result<(JsString, PropertyKey), Error> {
        let name = match usize::try_from(index)
            .ok()
            .and_then(|index| self.constants.get(index))
        {
            Some(BytecodeConstant::Value(RawValue::String(name))) => name.clone(),
            Some(BytecodeConstant::Value(_) | BytecodeConstant::Function(_)) => {
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
        Ok((name, key))
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
            PropertySetAction::Rejected(_) if !strict => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Rejected(rejection) => {
                let message = match rejection {
                    PropertySetRejection::ReadOnly => {
                        let name = self
                            .runtime
                            .property_key_to_js_string(key)
                            .map_err(runtime_error_to_vm_error)?;
                        format!("'{}' is read-only", name.to_utf8_lossy())
                    }
                    PropertySetRejection::NoSetter => "no setter for property".to_owned(),
                    PropertySetRejection::NotExtensible => "object is not extensible".to_owned(),
                    PropertySetRejection::NotObject => "not an object".to_owned(),
                };
                Err(Error::new(ErrorKind::Type, message))
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
        static_name: Option<&JsString>,
    ) -> Result<Completion, Error> {
        match &base {
            Value::Null | Value::Undefined => {
                let base_name = if matches!(base, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                let message = static_name.map_or_else(
                    || format!("cannot read property of {base_name}"),
                    |name| {
                        format!(
                            "cannot read property '{}' of {base_name}",
                            name.to_utf8_lossy()
                        )
                    },
                );
                return Err(Error::new(ErrorKind::Type, message));
            }
            Value::Object(object) => {
                let action = self
                    .runtime
                    .prepare_get_property_with_receiver(object, key, base.clone())
                    .map_err(runtime_error_to_vm_error)?;
                return self.finish_property_get_action(action);
            }
            Value::Bool(_) => {
                let prototype = self
                    .runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::Boolean)
                    .map_err(runtime_error_to_vm_error)?;
                let action = self
                    .runtime
                    .prepare_get_property_with_receiver(&prototype, key, base.clone())
                    .map_err(runtime_error_to_vm_error)?;
                return self.finish_property_get_action(action);
            }
            Value::String(string) => {
                let index = self
                    .runtime
                    .0
                    .state
                    .borrow()
                    .atoms
                    .array_index(key.atom())
                    .map_err(|error| Error::internal(error.to_string()))?;
                if let Some(index) = index
                    && let Ok(index) = usize::try_from(index)
                    && let Some(unit) = string.utf16_units().nth(index)
                {
                    return Ok(Completion::Return(Value::String(JsString::from_utf16([
                        unit,
                    ]))));
                }
                let length = self
                    .runtime
                    .intern_property_key("length")
                    .map_err(|error| Error::internal(error.to_string()))?;
                if key == &length {
                    let length = i32::try_from(string.len())
                        .map(Value::Int)
                        .unwrap_or_else(|_| Value::number(string.len() as f64));
                    return Ok(Completion::Return(length));
                }
            }
            Value::Int(_) | Value::Float(_) | Value::BigInt(_) | Value::Symbol(_) => {}
        }

        Err(Error::internal(
            "primitive prototype property lookup is not implemented yet",
        ))
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
                .prepare_set_property_with_receiver(object, key, value, base.clone())
                .map_err(runtime_error_to_vm_error)?,
            Value::Bool(_) => {
                let prototype = self
                    .runtime
                    .primitive_prototype_for_realm(self.current_realm, PrimitiveKind::Boolean)
                    .map_err(runtime_error_to_vm_error)?;
                self.runtime
                    .prepare_set_property_with_receiver(&prototype, key, value, base.clone())
                    .map_err(runtime_error_to_vm_error)?
            }
            Value::Null | Value::Undefined => {
                let base_name = if matches!(base, Value::Null) {
                    "null"
                } else {
                    "undefined"
                };
                let name = self
                    .runtime
                    .property_key_to_js_string(key)
                    .map_err(runtime_error_to_vm_error)?;
                return Err(Error::new(
                    ErrorKind::Type,
                    format!(
                        "cannot set property '{}' of {base_name}",
                        name.to_utf8_lossy()
                    ),
                ));
            }
            Value::String(_) => {
                if !strict {
                    return Ok(Completion::Return(Value::Undefined));
                }
                let length = self
                    .runtime
                    .intern_property_key("length")
                    .map_err(|error| Error::internal(error.to_string()))?;
                if key == &length {
                    return Err(Error::new(ErrorKind::Type, "'length' is read-only"));
                }
                return Err(Error::new(ErrorKind::Type, "not an object"));
            }
            Value::Int(_) | Value::Float(_) | Value::BigInt(_) | Value::Symbol(_) => {
                if !strict {
                    return Ok(Completion::Return(Value::Undefined));
                }
                return Err(Error::new(ErrorKind::Type, "not an object"));
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
        }
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
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error => "object",
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
            Value::Undefined
            | Value::Null
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_)
            | Value::Object(_) => {
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
            .new_native_error(self.current_realm, kind, error.message())
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
                    let binding = self
                        .locals
                        .get_mut(usize::from(index))
                        .ok_or_else(|| Error::internal("captured local index is out of bounds"))?;
                    capture_frame_binding(&self.runtime, binding, descriptor)?
                }
                ClosureSource::ParentArgument(index) => {
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
                ClosureSource::Global => {
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
        let name = self
            .runtime
            .property_key_to_js_string(&key)
            .map_err(runtime_error_to_vm_error)?;
        if cell.is_lexical {
            return Err(Error::new(
                ErrorKind::Reference,
                format!("{} is not initialized", name.to_utf8_lossy()),
            ));
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
            Err(Error::new(
                ErrorKind::Reference,
                format!("'{}' is not defined", name.to_utf8_lossy()),
            ))
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
        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))?;
        let name = self
            .runtime
            .property_key_to_js_string(&key)
            .map_err(runtime_error_to_vm_error)?;

        if cell.is_lexical {
            if matches!(cell.value, RawValue::Uninitialized) && !initialize {
                return Err(Error::new(
                    ErrorKind::Reference,
                    format!("{} is not initialized", name.to_utf8_lossy()),
                ));
            }
            if cell.is_const && !initialize {
                return Err(Error::new(
                    ErrorKind::Type,
                    format!("'{}' is read-only", name.to_utf8_lossy()),
                ));
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
            return Err(Error::new(
                ErrorKind::Reference,
                format!("'{}' is not defined", name.to_utf8_lossy()),
            ));
        }
        match self
            .runtime
            .prepare_set_property(&global_object, &key, value)
            .map_err(runtime_error_to_vm_error)?
        {
            PropertySetAction::Complete => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Rejected(_) if !strict => Ok(Completion::Return(Value::Undefined)),
            PropertySetAction::Rejected(PropertySetRejection::ReadOnly) => Err(Error::new(
                ErrorKind::Type,
                format!("'{}' is read-only", name.to_utf8_lossy()),
            )),
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
        let (name, key) = self.constant_property_key(key_index)?;
        self.get_property_with_key(base, &key, Some(&name))
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
        self.get_property_with_key(base, &key, None)
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
        let (_, key) = self.constant_property_key(key_index)?;
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
        let binding = self
            .locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        read_frame_binding(&self.runtime, binding)
    }

    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local index is out of bounds"))?;
        write_frame_binding(&self.runtime, binding, value)
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
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .read_var_ref(root)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn put_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
        self.runtime
            .write_var_ref(root, value)
            .map_err(|error| Error::internal(error.to_string()))
    }
}

enum FlatConstant {
    Value(RawValue),
    Child(usize),
}

struct FlatFunction {
    code: Vec<crate::bytecode::Instruction>,
    constants: Vec<FlatConstant>,
    metadata: FunctionMetadata,
    func_name: Option<JsString>,
    closure_variables: Vec<ClosureVariable>,
    debug: Option<UnlinkedFunctionDebug>,
}

struct FlattenFrame {
    code: Vec<crate::bytecode::Instruction>,
    remaining: std::vec::IntoIter<UnlinkedConstant>,
    constants: Vec<FlatConstant>,
    metadata: FunctionMetadata,
    func_name: Option<JsString>,
    closure_variables: Vec<ClosureVariable>,
    debug: Option<UnlinkedFunctionDebug>,
}

impl FlattenFrame {
    fn new(function: UnlinkedFunction) -> Self {
        let parts = function.into_parts();
        Self {
            code: parts.code,
            constants: Vec::with_capacity(parts.constants.len()),
            remaining: parts.constants.into_iter(),
            metadata: parts.metadata,
            func_name: parts.func_name,
            closure_variables: parts.closure_variables,
            debug: parts.debug,
        }
    }
}

/// Checked failures at the public runtime-domain boundary.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuntimeError {
    WrongRuntime(&'static str),
    Invariant(&'static str),
    Exception,
    Engine(Error),
    Atom(AtomError),
    Heap(HeapError),
    Shape(ShapeError),
    Property(PropertyDefinitionError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongRuntime(kind) => write!(formatter, "{kind} belongs to another runtime"),
            Self::Invariant(message) => write!(formatter, "runtime invariant failed: {message}"),
            Self::Exception => formatter.write_str("JavaScript exception"),
            Self::Engine(error) => error.fmt(formatter),
            Self::Atom(error) => error.fmt(formatter),
            Self::Heap(error) => error.fmt(formatter),
            Self::Shape(error) => error.fmt(formatter),
            Self::Property(error) => error.fmt(formatter),
        }
    }
}

impl StdError for RuntimeError {}

impl From<Error> for RuntimeError {
    fn from(error: Error) -> Self {
        Self::Engine(error)
    }
}

impl From<AtomError> for RuntimeError {
    fn from(error: AtomError) -> Self {
        Self::Atom(error)
    }
}

impl From<HeapError> for RuntimeError {
    fn from(error: HeapError) -> Self {
        Self::Heap(error)
    }
}

impl From<ShapeError> for RuntimeError {
    fn from(error: ShapeError) -> Self {
        Self::Shape(error)
    }
}

impl From<PropertyDefinitionError> for RuntimeError {
    fn from(error: PropertyDefinitionError) -> Self {
        Self::Property(error)
    }
}

/// A single-threaded QuickJS-compatible runtime.
///
/// Cloning this handle does not clone the runtime; it creates another owner of
/// the same heap/atom domain so multiple contexts can share runtime resources.
#[derive(Clone)]
pub struct Runtime(Rc<RuntimeInner>);

/// Stack-owned root set and LIFO token for one active execution frame.
///
/// Normal execution calls [`Self::finish`] so token/order corruption becomes
/// an engine error. `Drop` is a no-fail fallback for unwinding paths and keeps
/// stale diagnostic frames from escaping their invocation.
struct ActiveFrameGuard {
    runtime: Runtime,
    token: ActiveFrameToken,
    depth: usize,
    active: bool,
    _function_root: ObjectRef,
    _bytecode_root: Option<FunctionBytecodeRef>,
}

struct BacktraceBarrierGuard {
    runtime: Runtime,
    token: Option<ActiveFrameToken>,
    previous: bool,
    active: bool,
}

impl ActiveFrameGuard {
    const fn token(&self) -> ActiveFrameToken {
        self.token
    }

    fn finish(mut self) -> Result<(), RuntimeError> {
        let result = self.runtime.pop_active_frame(self.token, self.depth);
        self.active = false;
        result
    }
}

impl Drop for ActiveFrameGuard {
    fn drop(&mut self) {
        if self.active {
            self.runtime
                .pop_active_frame_fallback(self.token, self.depth);
            self.active = false;
        }
    }
}

impl BacktraceBarrierGuard {
    fn finish(mut self) -> Result<(), RuntimeError> {
        if let Some(token) = self.token {
            self.runtime
                .restore_backtrace_barrier(token, self.previous)?;
        }
        self.active = false;
        Ok(())
    }
}

impl Drop for BacktraceBarrierGuard {
    fn drop(&mut self) {
        if self.active {
            if let Some(token) = self.token {
                self.runtime
                    .restore_backtrace_barrier_fallback(token, self.previous);
            }
            self.active = false;
        }
    }
}

impl Default for Runtime {
    fn default() -> Self {
        Self::new()
    }
}

impl Runtime {
    #[must_use]
    pub fn new() -> Self {
        let domain_id = NEXT_RUNTIME_DOMAIN_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(domain_id, 0, "runtime domain ID space exhausted");
        let mut atoms = AtomTable::new();
        let mut well_known_symbols = HashMap::new();
        for symbol in WellKnownSymbol::ALL {
            let atom = atoms
                .new_static_symbol(Some(symbol.description()))
                .expect("fixed well-known symbol set fits the atom table");
            well_known_symbols.insert(symbol, atom);
        }
        Self(Rc::new(RuntimeInner {
            state: RefCell::new(RuntimeState {
                atoms,
                heap: Heap::new(),
                pending_exception: None,
                debug_info_mode: DebugInfoMode::Full,
                shape_cache: HashMap::new(),
                shape_fingerprints: HashMap::new(),
                well_known_symbols,
                active_frames: Vec::new(),
                next_active_frame_token: 1,
                #[cfg(test)]
                active_frame_probe_snapshots: Vec::new(),
            }),
            deferred_references: RefCell::new(VecDeque::new()),
            next_context_id: Cell::new(0),
            domain_id,
        }))
    }

    /// Set the runtime-wide debug information policy for future compilations.
    /// Existing bytecode is immutable and keeps the mode used when published.
    pub fn set_debug_info_mode(&self, mode: DebugInfoMode) {
        self.0.state.borrow_mut().debug_info_mode = mode;
    }

    /// Return the policy which the next compilation will sample.
    #[must_use]
    pub fn debug_info_mode(&self) -> DebugInfoMode {
        self.0.state.borrow().debug_info_mode
    }

    #[must_use]
    pub fn new_context(&self) -> Context {
        let _operation = self.operation();
        let id = self.0.next_context_id.get();
        self.0.next_context_id.set(
            id.checked_add(1)
                .expect("runtime context identity space exhausted"),
        );
        let object_prototype = self
            .new_object(None)
            .expect("initial Object.prototype allocation must succeed");
        self.0
            .state
            .borrow_mut()
            .heap
            .set_immutable_prototype(object_prototype.object_id())
            .expect("Object.prototype immutable-prototype initialization must succeed");
        let function_prototype = self
            .new_native_function(&object_prototype, NativeFunctionId::FunctionPrototype, 0)
            .expect("initial Function.prototype allocation must succeed");
        self.define_function_data_property(
            &function_prototype,
            "length",
            Value::Int(0),
            false,
            true,
        )
        .expect("Function.prototype.length initialization must succeed");
        self.define_function_data_property(
            &function_prototype,
            "name",
            Value::String(JsString::from("")),
            false,
            true,
        )
        .expect("Function.prototype.name initialization must succeed");
        let error_prototype = self
            .new_object(Some(&object_prototype))
            .expect("initial Error.prototype allocation must succeed");
        let mut native_error_prototypes = Vec::with_capacity(NativeErrorKind::COUNT);
        for kind in NativeErrorKind::ALL {
            let prototype = self
                .new_object(Some(&error_prototype))
                .expect("native Error prototype allocation must succeed");
            self.define_bootstrap_string_property(&prototype, "name", kind.name())
                .expect("native Error prototype name initialization must succeed");
            native_error_prototypes.push(prototype);
        }
        let boolean_prototype = self
            .new_primitive_object(
                &object_prototype,
                PrimitiveKind::Boolean,
                Value::Bool(false),
            )
            .expect("initial Boolean.prototype allocation must succeed");
        let native_error_ids =
            std::array::from_fn(|index| native_error_prototypes[index].object_id());
        let uninitialized_vars = self
            .new_object(None)
            .expect("global unresolved-name table allocation must succeed");
        let global_object = self
            .new_global_object(&object_prototype, &uninitialized_vars)
            .expect("initial global object allocation must succeed");
        let global_var_object = self
            .new_object(None)
            .expect("initial global variable object allocation must succeed");
        self.0
            .state
            .borrow_mut()
            .heap
            .set_immutable_prototype(global_var_object.object_id())
            .expect("global lexical object immutable-prototype initialization must succeed");
        let realm = {
            let mut state = self.0.state.borrow_mut();
            state
                .heap
                .allocate_context(
                    ContextData::new(
                        object_prototype.object_id(),
                        function_prototype.object_id(),
                        global_object.object_id(),
                        global_var_object.object_id(),
                    )
                    .with_primitive_prototype(PrimitiveKind::Boolean, boolean_prototype.object_id())
                    .with_error_prototypes(error_prototype.object_id(), native_error_ids),
                )
                .expect("initial realm allocation must succeed")
        };
        // Own the freshly published realm before any fallible intrinsic
        // initialization. If a bootstrap expect panics, Context::drop releases
        // the initial strong reference so the partial graph remains
        // collectable instead of permanently leaking its raw ContextId.
        let context = Context {
            runtime: self.clone(),
            id,
            realm,
        };
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_native_function_realm(function_prototype.object_id(), realm)
            .expect("Function.prototype defining-realm attachment must succeed");
        self.initialize_function_restricted_properties(realm, &function_prototype)
            .expect("Function restricted-property initialization must succeed");
        self.initialize_object_prototype_intrinsics(realm, &object_prototype)
            .expect("Object.prototype intrinsic initialization must succeed");
        self.initialize_function_prototype_methods(realm, &function_prototype)
            .expect("Function.prototype method initialization must succeed");
        self.initialize_error_intrinsics(
            realm,
            &function_prototype,
            &error_prototype,
            &native_error_prototypes,
            &global_object,
        )
        .expect("Error intrinsic initialization must succeed");
        self.initialize_function_constructor(realm, &function_prototype, &global_object)
            .expect("Function constructor initialization must succeed");
        self.initialize_global_number_parsers(realm, &function_prototype, &global_object)
            .expect("global numeric parser initialization must succeed");
        self.initialize_boolean_intrinsic(
            realm,
            &function_prototype,
            &boolean_prototype,
            &global_object,
        )
        .expect("Boolean intrinsic initialization must succeed");
        self.initialize_global_primitive_constants(&global_object)
            .expect("global primitive constant initialization must succeed");
        drop(global_var_object);
        drop(global_object);
        drop(uninitialized_vars);
        drop(boolean_prototype);
        drop(native_error_prototypes);
        drop(error_prototype);
        drop(function_prototype);
        drop(object_prototype);
        context
    }

    fn define_bootstrap_string_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: &str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let defined = self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from(value))),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "intrinsic bootstrap property definition was rejected",
            ));
        }
        Ok(())
    }

    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.0, &other.0)
    }

    /// Stable identity used by rooted handle hashing and diagnostics.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.domain_id
    }

    /// Intern an exact ECMAScript string as a runtime-owned property key.
    pub fn intern_property_key_js_string(&self, text: &JsString) -> Result<PropertyKey, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .intern_property_key_js_string(text)?;
        Ok(PropertyKey::from_owned_atom(self.clone(), atom))
    }

    /// Intern a UTF-8 property spelling without losing the exact UTF-16 path
    /// used by language-level keys.
    pub fn intern_property_key(&self, text: &str) -> Result<PropertyKey, AtomError> {
        self.intern_property_key_js_string(&JsString::from(text))
    }

    /// Create a unique ECMAScript Symbol primitive.
    pub fn new_symbol(&self, description: Option<JsString>) -> Result<SymbolRef, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .new_symbol_js_string(description)?;
        Ok(SymbolRef::from_owned_atom(self.clone(), atom))
    }

    /// Return the runtime-global symbol for an exact registry key.
    pub fn symbol_for(&self, key: &JsString) -> Result<SymbolRef, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .intern_global_symbol_js_string(key)?;
        Ok(SymbolRef::from_owned_atom(self.clone(), atom))
    }

    /// Return one pinned, runtime-unique well-known symbol. It is deliberately
    /// absent from the `Symbol.for` registry.
    pub fn well_known_symbol(&self, symbol: WellKnownSymbol) -> SymbolRef {
        let _operation = self.operation();
        let atom = self.0.state.borrow().well_known_symbols[&symbol];
        SymbolRef::from_owned_atom(self.clone(), atom)
    }

    /// Implement the identity test used by `Symbol.keyFor`.
    pub fn symbol_key_for(&self, symbol: &SymbolRef) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        if !symbol.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("symbol"));
        }
        let state = self.0.state.borrow();
        if state.atoms.kind(symbol.atom())? == AtomKind::GlobalSymbol {
            Ok(Some(state.atoms.to_js_string(symbol.atom())?))
        } else {
            Ok(None)
        }
    }

    /// Return the exact UTF-16 spelling or symbol description of a key.
    pub fn property_key_to_js_string(&self, key: &PropertyKey) -> Result<JsString, RuntimeError> {
        let _operation = self.operation();
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        Ok(self.0.state.borrow().atoms.to_js_string(key.atom())?)
    }

    /// Allocate an ordinary object whose prototype is `prototype` or null.
    pub fn new_object(&self, prototype: Option<&ObjectRef>) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if prototype.is_some_and(|prototype| !prototype.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let prototype = prototype.map(ObjectRef::object_id);

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(prototype, &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn new_primitive_object(
        &self,
        prototype: &ObjectRef,
        kind: PrimitiveKind,
        value: Value,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("primitive prototype"));
        }
        let data = match (kind, value) {
            (PrimitiveKind::Boolean, Value::Bool(value)) => PrimitiveObjectData::Boolean(value),
            _ => {
                return Err(RuntimeError::Invariant(
                    "primitive wrapper class or payload is not implemented yet",
                ));
            }
        };
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object =
            match state
                .heap
                .allocate_object(ObjectData::primitive(shape, Vec::new(), data))
            {
                Ok(object) => object,
                Err(error) => {
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error.into());
                }
            };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn new_global_object(
        &self,
        prototype: &ObjectRef,
        uninitialized_vars: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) || !uninitialized_vars.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("global object edge"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state.heap.allocate_object(ObjectData::global_object(
            shape,
            Vec::new(),
            uninitialized_vars.object_id(),
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn new_native_function(
        &self,
        prototype: &ObjectRef,
        target: NativeFunctionId,
        min_readable_args: u8,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object =
            match state
                .heap
                .allocate_bootstrap_native_function(ObjectData::native_function(
                    shape,
                    Vec::new(),
                    target,
                    min_readable_args,
                )) {
                Ok(object) => object,
                Err(error) => {
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error.into());
                }
            };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    /// Allocate a native callable after its defining realm has been
    /// published. `%Function.prototype%` cannot use this path because it is
    /// itself one of the roots needed to publish the realm.
    fn new_bound_native_function(
        &self,
        prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::bound_native_function(
                shape,
                Vec::new(),
                target,
                realm,
                min_readable_args,
            )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_owned_handle(self.clone(), object),
        ))
    }

    fn new_bound_function(
        &self,
        realm: ContextId,
        target: &CallableRef,
        this_value: &Value,
        arguments: &[Value],
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !target.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("bound function target"));
        }
        self.validate_value_domain(this_value, "bound this value")?;
        for argument in arguments {
            self.validate_value_domain(argument, "bound function argument")?;
        }

        let raw_this = self.raw_property_value(this_value)?;
        let raw_arguments = arguments
            .iter()
            .map(|argument| self.raw_property_value(argument))
            .collect::<Result<Vec<_>, _>>()?;
        let is_constructor = self.is_constructor(target.as_object())?;

        let mut state = self.0.state.borrow_mut();
        let function_prototype = state.heap.context(realm)?.function_prototype;
        let shape = state.get_or_create_shape(Some(function_prototype), &[])?;
        let retained_atoms = match state
            .retain_raw_value_atoms(std::iter::once(&raw_this).chain(raw_arguments.iter()))
        {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };
        let object = match state.heap.allocate_object(ObjectData::bound_function(
            shape,
            Vec::new(),
            target.as_object().object_id(),
            raw_this,
            raw_arguments.into(),
            is_constructor,
        )) {
            Ok(object) => object,
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_owned_handle(self.clone(), object),
        ))
    }

    /// Allocate a fully initialized realm-bound native builtin. Internal
    /// readable arity remains in the payload while own `length` is an
    /// independent configurable ordinary property.
    fn new_native_builtin(
        &self,
        prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        name: &str,
        length: i32,
    ) -> Result<CallableRef, RuntimeError> {
        let callable =
            self.new_bound_native_function(prototype, realm, target, min_readable_args)?;
        self.define_function_data_property(
            callable.as_object(),
            "length",
            Value::Int(length),
            false,
            true,
        )?;
        self.define_function_data_property(
            callable.as_object(),
            "name",
            Value::String(JsString::from(name)),
            false,
            true,
        )?;
        Ok(callable)
    }

    fn initialize_error_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        error_prototype: &ObjectRef,
        native_error_prototypes: &[ObjectRef],
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        if native_error_prototypes.len() != NativeErrorKind::COUNT {
            return Err(RuntimeError::Invariant(
                "native Error prototype count did not match NativeErrorKind",
            ));
        }

        // JS_NewCConstructor installs prototype fields before the constructor
        // back-reference. Preserve that observable own-key order.
        self.define_native_builtin_auto_init(
            error_prototype,
            realm,
            NativeFunctionId::ErrorPrototypeToString,
            "toString",
            0,
            0,
        )?;
        self.define_string_auto_init(error_prototype, realm, "name", "Error")?;
        self.define_string_auto_init(error_prototype, realm, "message", "")?;

        for prototype in native_error_prototypes {
            self.define_string_auto_init(prototype, realm, "message", "")?;
        }

        let error_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ErrorConstructor(ErrorConstructorKind::Error),
            1,
            "Error",
            1,
        )?;
        self.define_native_builtin_auto_init(
            error_constructor.as_object(),
            realm,
            NativeFunctionId::ErrorIsError,
            "isError",
            1,
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Error",
            Value::Object(error_constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&error_constructor, error_prototype)?;

        for kind in NativeErrorKind::ALL {
            // AggregateError needs iterable-to-array semantics and an Array
            // instance for its `errors` property. Do not expose a fake
            // constructor before that substrate exists.
            if kind == NativeErrorKind::Aggregate {
                continue;
            }
            let prototype =
                native_error_prototypes
                    .get(kind.index())
                    .ok_or(RuntimeError::Invariant(
                        "native Error prototype index was out of bounds",
                    ))?;
            let constructor = self.new_native_builtin(
                error_constructor.as_object(),
                realm,
                NativeFunctionId::ErrorConstructor(ErrorConstructorKind::Native(kind)),
                1,
                kind.name(),
                1,
            )?;
            self.define_function_data_property(
                global_object,
                kind.name(),
                Value::Object(constructor.as_object().clone()),
                true,
                true,
            )?;
            self.define_constructor_relationship(&constructor, prototype)?;
        }
        Ok(())
    }

    fn initialize_function_constructor(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // QuickJS publishes Function after its prototype table and the Error
        // family, then closes the constructor/prototype cycle. Its magic
        // selector makes this same handler reusable by the future dynamic
        // GeneratorFunction/AsyncFunction constructors.
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
            1,
            "Function",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Function",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, function_prototype)?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_function_constructor(realm, constructor.as_object().object_id())?;
        Ok(())
    }

    fn initialize_boolean_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        boolean_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let kind = PrimitiveKind::Boolean;
        // QuickJS installs the complete Boolean prototype table before the
        // constructor back-reference, which fixes own-key order as
        // `toString,valueOf,constructor`.
        self.define_native_builtin_auto_init(
            boolean_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeToString(kind),
            "toString",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            boolean_prototype,
            realm,
            NativeFunctionId::PrimitivePrototypeValueOf(kind),
            "valueOf",
            0,
            0,
        )?;
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::PrimitiveConstructor(kind),
            1,
            "Boolean",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "Boolean",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, boolean_prototype)
    }

    fn initialize_global_number_parsers(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // QuickJS publishes these global functions before `%Number%`, whose
        // static parseInt/parseFloat properties capture the same callable
        // identities during Number initialization.
        for (kind, name, arity) in [
            (NumberParseKind::ParseInt, "parseInt", 2),
            (NumberParseKind::ParseFloat, "parseFloat", 1),
        ] {
            let callable = self.new_native_builtin(
                function_prototype,
                realm,
                NativeFunctionId::GlobalNumberParse(kind),
                arity,
                name,
                i32::from(arity),
            )?;
            self.define_function_data_property(
                global_object,
                name,
                Value::Object(callable.as_object().clone()),
                true,
                true,
            )?;
        }
        Ok(())
    }

    fn initialize_global_primitive_constants(
        &self,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // Tail of QuickJS `js_global_funcs`: all three are non-writable,
        // non-enumerable and non-configurable global data properties.
        for (name, value) in [
            ("Infinity", Value::Float(f64::INFINITY)),
            ("NaN", Value::Float(f64::NAN)),
            ("undefined", Value::Undefined),
        ] {
            self.define_function_data_property(global_object, name, value, false, false)?;
        }
        Ok(())
    }

    fn initialize_function_restricted_properties(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // JS_AddIntrinsicBaseObjects creates one frozen %ThrowTypeError% and
        // installs that same callable as both halves of both legacy poison
        // accessors before publishing the Function prototype method table.
        let throw_type_error = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ThrowTypeError,
            0,
            "",
            0,
        )?;
        for name in ["length", "name"] {
            let key = self.intern_property_key(name)?;
            let accepted = self.define_own_property(
                throw_type_error.as_object(),
                &key,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !accepted {
                return Err(RuntimeError::Invariant(
                    "%ThrowTypeError% own property could not be frozen",
                ));
            }
        }
        self.prevent_extensions(throw_type_error.as_object())?;

        for name in ["caller", "arguments"] {
            let key = self.intern_property_key(name)?;
            let accepted = self.define_own_property(
                function_prototype,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(
                        throw_type_error.clone(),
                    )),
                    set: DescriptorField::Present(AccessorValue::Callable(
                        throw_type_error.clone(),
                    )),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !accepted {
                return Err(RuntimeError::Invariant(
                    "Function.prototype poison accessor definition was rejected",
                ));
            }
        }

        self.0
            .state
            .borrow_mut()
            .heap
            .attach_throw_type_error(realm, throw_type_error.as_object().object_id())?;
        Ok(())
    }

    fn initialize_function_prototype_methods(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeCall,
            "call",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeApply,
            "apply",
            2,
            2,
        )?;
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeBind,
            "bind",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            function_prototype,
            realm,
            NativeFunctionId::FunctionPrototypeToString,
            "toString",
            0,
            0,
        )?;

        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        self.define_native_builtin_auto_init_with_key(
            function_prototype,
            realm,
            &has_instance,
            NativeFunctionId::FunctionPrototypeHasInstance,
            "[Symbol.hasInstance]",
            1,
            1,
            PropertyFlags::data(false, false, false),
        )?;

        // Unlike C functions in JS_SetPropertyFunctionList, QuickJS's
        // CGETSET entries are instantiated eagerly. Keep that distinction so
        // descriptor reads and realm/GC edges match the upstream table.
        for (target, property_name, getter_name) in [
            (
                NativeFunctionId::FunctionPrototypeFileName,
                "fileName",
                "get fileName",
            ),
            (
                NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Line),
                "lineNumber",
                "get lineNumber",
            ),
            (
                NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Column),
                "columnNumber",
                "get columnNumber",
            ),
        ] {
            self.define_native_builtin_getter(
                function_prototype,
                realm,
                target,
                property_name,
                getter_name,
            )?;
        }
        Ok(())
    }

    fn define_native_builtin_getter(
        &self,
        function_prototype: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        property_name: &str,
        getter_name: &str,
    ) -> Result<(), RuntimeError> {
        let getter =
            self.new_native_builtin(function_prototype, realm, target, 0, getter_name, 0)?;
        let key = self.intern_property_key(property_name)?;
        if !self.define_own_property(
            function_prototype,
            &key,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "native builtin getter definition was rejected",
            ));
        }
        Ok(())
    }

    fn initialize_object_prototype_intrinsics(
        &self,
        realm: ContextId,
        object_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        // Preserve the prefix of `js_object_proto_funcs`. Later Object
        // methods can append after these without changing upstream key order.
        for (target, name) in [
            (NativeFunctionId::ObjectPrototypeToString, "toString"),
            (
                NativeFunctionId::ObjectPrototypeToLocaleString,
                "toLocaleString",
            ),
            (NativeFunctionId::ObjectPrototypeValueOf, "valueOf"),
        ] {
            self.define_native_builtin_auto_init(object_prototype, realm, target, name, 0, 0)?;
        }
        Ok(())
    }

    fn define_constructor_relationship(
        &self,
        constructor: &CallableRef,
        prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(prototype.clone()),
            false,
            false,
        )?;
        self.define_function_data_property(
            prototype,
            "constructor",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )
    }

    fn new_native_error(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: &str,
    ) -> Result<Value, RuntimeError> {
        let value = self.new_native_error_without_backtrace(realm, kind, message)?;
        let capture_now = self
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .is_none_or(|frame| matches!(frame.kind, ActiveFrameKind::Native { .. }));
        if capture_now {
            self.ensure_error_backtrace(&value, false, None)?;
        }
        Ok(value)
    }

    /// `JS_ThrowError2(..., add_backtrace = FALSE)` construction path used by
    /// parser diagnostics, which prepend their explicit filename location
    /// before adding the active frame chain.
    fn new_native_error_without_backtrace(
        &self,
        realm: ContextId,
        kind: NativeErrorKind,
        message: &str,
    ) -> Result<Value, RuntimeError> {
        let prototype = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.native_error_prototypes[kind.index()].ok_or(
                RuntimeError::Invariant("realm has no native Error prototype"),
            )?
        };
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let object = self.new_error_object(&prototype)?;
        let key = self.intern_property_key("message")?;
        let defined = self.define_own_property(
            &object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from(message))),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "native Error message definition was rejected",
            ));
        }
        Ok(Value::Object(object))
    }

    fn new_error_object(&self, prototype: &ObjectRef) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Error prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::error(shape, Vec::new()))
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    /// Return whether `object` carries the native Error class tag. Prototype
    /// spoofing alone does not make an object an Error.
    pub fn is_error_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self.0.state.borrow().heap.object(object.object_id())?.kind == ObjectKind::Error)
    }

    /// Return the object's `[[Construct]]` capability bit. Callability and
    /// constructability are intentionally independent, as in QuickJS.
    pub fn is_constructor(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .is_constructor)
    }

    /// Set the object's `[[Construct]]` capability independently of its call
    /// protocol, matching QuickJS `JS_SetConstructorBit`.
    pub fn set_constructor_bit(
        &self,
        object: &ObjectRef,
        enabled: bool,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_object_constructor_bit(object.object_id(), enabled)?;
        Ok(())
    }

    /// Promote an ordinary object root to a checked callable capability.
    /// Returns `None` for objects without `[[Call]]`; runtime-domain and stale
    /// handle failures remain explicit errors.
    pub fn as_callable(&self, object: &ObjectRef) -> Result<Option<CallableRef>, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let callable = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
        );
        if !callable {
            return Ok(None);
        }
        Ok(Some(CallableRef::from_validated_object(object.clone())))
    }

    /// Instantiate one runtime-owned bytecode node as a callable object in the
    /// caller's realm, matching QuickJS's `js_closure` boundary.
    fn new_bytecode_closure(
        &self,
        caller_realm: ContextId,
        function: &FunctionBytecodeRef,
    ) -> Result<CallableRef, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let (defining_realm, descriptors) = {
            let state = self.0.state.borrow();
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            (bytecode.realm, bytecode.closure_variables.clone())
        };
        let mut slots = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors.iter().copied() {
            if descriptor.source != ClosureSource::Global {
                return Err(RuntimeError::Invariant(
                    "root bytecode closure descriptor did not use Global",
                ));
            }
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published global closure descriptor has no atom",
                ));
            };
            slots.push(self.resolve_global_var(defining_realm, name)?);
        }
        self.new_bytecode_closure_with_slots(caller_realm, function, &slots)
    }

    fn resolve_global_var(&self, realm: ContextId, name: Atom) -> Result<VarRefRoot, RuntimeError> {
        let key = PropertyKey::from_borrowed_atom(self.clone(), name)?;
        let (global_var_object, global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (
                context.global_var_object,
                context.global_object,
                uninitialized_vars,
            )
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        if let Some(root) = self.own_var_ref_root(&global_var_object, &key)? {
            return Ok(root);
        }

        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        if self.is_auto_init_own_property(&global_object, &key)? {
            self.materialize_auto_init_property(&global_object, &key)?;
        }
        if let Some(root) = self.own_var_ref_root(&global_object, &key)? {
            return Ok(root);
        }

        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        if let Some(root) = self.own_var_ref_root(&hidden, &key)? {
            return Ok(root);
        }
        let root = self.new_uninitialized_var_ref()?;
        self.store_property_slot(
            &hidden,
            &key,
            PropertyFlags::data(true, true, true),
            PropertySlot::VarRef(root.id()),
        )?;
        Ok(root)
    }

    #[cfg(test)]
    fn create_global_lexical_for_test(
        &self,
        realm: ContextId,
        name: &str,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let (global_var_object, global_object, hidden) = {
            let state = self.0.state.borrow();
            let context = state.heap.context(realm)?;
            let global = state.heap.object(context.global_object)?;
            let ObjectPayload::GlobalObject { uninitialized_vars } = &global.payload else {
                return Err(RuntimeError::Invariant(
                    "realm global object has no unresolved-name table",
                ));
            };
            (
                context.global_var_object,
                context.global_object,
                *uninitialized_vars,
            )
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        if self.has_own_property(&global_var_object, &key)? {
            return Err(RuntimeError::Invariant(
                "test attempted to redeclare a global lexical binding",
            ));
        }
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
        let global_object = ObjectRef::from_borrowed_handle(self.clone(), global_object)?;
        let root = if let Some(root) = self.own_var_ref_root(&global_object, &key)? {
            let (flags, value) = {
                let state = self.0.state.borrow();
                let object = state.heap.object(global_object.object_id())?;
                let shape = state.heap.shape(object.shape)?;
                let index = shape.find(key.atom()).ok_or(RuntimeError::Invariant(
                    "global VarRef disappeared during lexical creation",
                ))? as usize;
                let flags = shape.entries()[index].flags;
                let value = state.heap.var_ref(root.id())?.value.clone();
                (flags, value)
            };
            let value = self.root_raw_value(&value)?;
            let replacement =
                self.new_var_ref(value, false, !flags.writable, ClosureVariableKind::Normal)?;
            self.store_property_slot(
                &global_object,
                &key,
                flags,
                PropertySlot::VarRef(replacement.id()),
            )?;
            self.reset_var_ref_uninitialized(&root)?;
            root
        } else if let Some(root) = self.own_var_ref_root(&hidden, &key)? {
            if !self.delete_property(&hidden, &key)? {
                return Err(RuntimeError::Invariant(
                    "hidden global VarRef property was not configurable",
                ));
            }
            root
        } else {
            self.new_uninitialized_var_ref()?
        };
        self.set_var_ref_metadata(&root, true, is_const, ClosureVariableKind::Normal)?;
        if let Some(value) = initial_value {
            self.write_var_ref(&root, value)?;
        }
        self.store_property_slot(
            &global_var_object,
            &key,
            PropertyFlags::data(!is_const, true, true),
            PropertySlot::VarRef(root.id()),
        )
    }

    #[cfg(test)]
    fn initialize_global_lexical_for_test(
        &self,
        realm: ContextId,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let global_var_object = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?.global_var_object
        };
        let global_var_object = ObjectRef::from_borrowed_handle(self.clone(), global_var_object)?;
        let root =
            self.own_var_ref_root(&global_var_object, &key)?
                .ok_or(RuntimeError::Invariant(
                    "test initialized a missing global lexical binding",
                ))?;
        self.write_var_ref(&root, value)
    }

    fn new_bytecode_closure_with_slots(
        &self,
        caller_realm: ContextId,
        function: &FunctionBytecodeRef,
        closure_slots: &[VarRefRoot],
    ) -> Result<CallableRef, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        if closure_slots.iter().any(|slot| !slot.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }

        let mut state = self.0.state.borrow_mut();
        let function_prototype = state.heap.context(caller_realm)?.function_prototype;
        let (metadata, func_name) = {
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            (bytecode.metadata, bytecode.func_name.clone())
        };
        let shape = state.get_or_create_shape(Some(function_prototype), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::bytecode_function_with_closures(
                shape,
                Vec::new(),
                function.bytecode_id(),
                None,
                closure_slots.iter().map(VarRefRoot::id).collect(),
                metadata.constructor_kind != ConstructorKind::None,
            )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        let callable =
            CallableRef::from_validated_object(ObjectRef::from_owned_handle(self.clone(), object));
        self.initialize_bytecode_function_properties(caller_realm, &callable, metadata, func_name)?;
        Ok(callable)
    }

    fn initialize_bytecode_function_properties(
        &self,
        realm: ContextId,
        callable: &CallableRef,
        metadata: FunctionMetadata,
        func_name: Option<JsString>,
    ) -> Result<(), RuntimeError> {
        self.define_function_data_property(
            callable.as_object(),
            "length",
            Value::Int(i32::from(metadata.defined_argument_count)),
            false,
            true,
        )?;
        self.define_function_data_property(
            callable.as_object(),
            "name",
            Value::String(func_name.unwrap_or_else(|| JsString::from(""))),
            false,
            true,
        )?;
        if !metadata.has_prototype {
            return Ok(());
        }
        self.define_function_auto_init_prototype(callable.as_object(), realm)
    }

    fn define_function_auto_init_prototype(
        &self,
        function: &ObjectRef,
        realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("prototype")?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = function.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "function prototype autoinit property already exists",
                ));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::data(true, false, false),
        });
        slots.push(PropertySlot::AutoInit(
            AutoInitProperty::FunctionPrototype { realm },
        ));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn define_native_builtin_auto_init(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        name: &'static str,
        length: u8,
        min_readable_args: u8,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.define_native_builtin_auto_init_with_key(
            object,
            realm,
            &key,
            target,
            name,
            length,
            min_readable_args,
            PropertyFlags::data(true, false, true),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn define_native_builtin_auto_init_with_key(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        key: &PropertyKey,
        target: NativeFunctionId,
        name: &'static str,
        length: u8,
        min_readable_args: u8,
        flags: PropertyFlags,
    ) -> Result<(), RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "native builtin autoinit property already exists",
                ));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags,
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::NativeBuiltin {
            realm,
            target,
            name,
            length,
            min_readable_args,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn define_string_auto_init(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        name: &str,
        value: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            if shape.find(key.atom()).is_some() {
                return Err(RuntimeError::Invariant(
                    "string autoinit property already exists",
                ));
            }
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::data(true, false, true),
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::String {
            realm,
            value,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    #[cfg(test)]
    fn define_failure_auto_init(
        &self,
        object: &ObjectRef,
        realm: ContextId,
        name: &str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let mut state = self.0.state.borrow_mut();
        state.heap.context(realm)?;
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots) = {
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object.slots.clone(),
            )
        };
        entries.push(ShapeEntry {
            atom: key.atom(),
            flags: PropertyFlags::data(true, false, true),
        });
        slots.push(PropertySlot::AutoInit(AutoInitProperty::FailureProbe {
            realm,
        }));
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn define_function_data_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
        writable: bool,
        configurable: bool,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        let defined = self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(writable),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(configurable),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Invariant(
                "function intrinsic property definition was rejected",
            ));
        }
        Ok(())
    }

    /// QuickJS `JS_DefineObjectName`: define a configurable, non-writable,
    /// non-enumerable own `name` only when the object does not already carry a
    /// non-empty (or otherwise authoritative) own name.
    fn define_object_name(&self, value: &Value, name: &JsString) -> Result<(), RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(());
        };
        let key = self.intern_property_key("name")?;
        let should_define = match self.get_own_property(object, &key)? {
            None => true,
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(current),
                ..
            }) => current.is_empty(),
            Some(
                CompleteOrdinaryPropertyDescriptor::Data { .. }
                | CompleteOrdinaryPropertyDescriptor::Accessor { .. },
            ) => false,
        };
        if !should_define {
            return Ok(());
        }
        let defined = self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(name.clone())),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "cannot define function name",
            )));
        }
        Ok(())
    }

    fn new_var_ref(
        &self,
        value: Value,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "captured variable")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atom = if let RawValue::Symbol(atom) = &raw {
            state.atoms.retain(*atom)?;
            Some(*atom)
        } else {
            None
        };
        let data = VarRefData::captured(raw, is_lexical, is_const, kind);
        let id = match state.heap.allocate_var_ref(data) {
            Ok(id) => id,
            Err(error) => {
                if let Some(atom) = retained_atom {
                    state.atoms.release(atom)?;
                }
                return Err(error.into());
            }
        };
        drop(state);
        drop(value);
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    fn new_uninitialized_var_ref(&self) -> Result<VarRefRoot, RuntimeError> {
        let _operation = self.operation();
        let id = self
            .0
            .state
            .borrow_mut()
            .heap
            .allocate_var_ref(VarRefData::captured(
                RawValue::Uninitialized,
                false,
                false,
                ClosureVariableKind::Normal,
            ))?;
        Ok(VarRefRoot::from_owned_handle(self.clone(), id))
    }

    fn set_var_ref_metadata(
        &self,
        root: &VarRefRoot,
        is_lexical: bool,
        is_const: bool,
        kind: ClosureVariableKind,
    ) -> Result<(), RuntimeError> {
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        self.0.state.borrow_mut().heap.set_var_ref_metadata(
            root.id(),
            is_lexical,
            is_const,
            kind,
        )?;
        Ok(())
    }

    fn reset_var_ref_uninitialized(&self, root: &VarRefRoot) -> Result<(), RuntimeError> {
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .replace_var_ref_value(root.id(), RawValue::Uninitialized)?;
        state.apply_cleanup(cleanup)
    }

    fn read_var_ref(&self, root: &VarRefRoot) -> Result<Value, RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let raw = self.0.state.borrow().heap.var_ref(root.id())?.value.clone();
        self.root_raw_value(&raw)
    }

    fn validate_var_ref_metadata(
        &self,
        root: &VarRefRoot,
        descriptor: ClosureVariable,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        let var_ref = self.0.state.borrow();
        let var_ref = var_ref.heap.var_ref(root.id())?;
        if var_ref.is_lexical != descriptor.is_lexical
            || var_ref.is_const != descriptor.is_const
            || var_ref.kind != descriptor.kind
        {
            return Err(RuntimeError::Invariant(
                "closure descriptor metadata does not match the shared variable cell",
            ));
        }
        Ok(())
    }

    fn write_var_ref(&self, root: &VarRefRoot, value: Value) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("closure variable"));
        }
        self.validate_value_domain(&value, "captured variable")?;
        let raw = self.raw_property_value(&value)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atom = if let RawValue::Symbol(atom) = &raw {
            state.atoms.retain(*atom)?;
            Some(*atom)
        } else {
            None
        };
        let cleanup = match state.heap.replace_var_ref_value(root.id(), raw) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                if let Some(atom) = retained_atom {
                    state.atoms.release(atom)?;
                }
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(value);
        Ok(())
    }

    /// Snapshot an ordinary own property as a complete descriptor.
    pub fn get_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        let snapshot = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let shape = state.heap.shape(object_data.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(None);
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let entry = shape.entries().get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            let slot = object_data
                .slots
                .get(index)
                .ok_or(RuntimeError::Invariant("object property slot was missing"))?;
            match slot {
                PropertySlot::Data(value) => PropertySnapshot::Data {
                    value: value.clone(),
                    flags: entry.flags,
                },
                PropertySlot::VarRef(var_ref) => PropertySnapshot::VarRef {
                    var_ref: *var_ref,
                    flags: entry.flags,
                },
                PropertySlot::Accessor { get, set } => PropertySnapshot::Accessor {
                    get: *get,
                    set: *set,
                    flags: entry.flags,
                },
                PropertySlot::AutoInit(_) => PropertySnapshot::AutoInit,
            }
        };

        match snapshot {
            PropertySnapshot::Data { value, flags } => {
                Ok(Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: self.root_raw_value(&value)?,
                    writable: flags.writable,
                    enumerable: flags.enumerable,
                    configurable: flags.configurable,
                }))
            }
            PropertySnapshot::VarRef { var_ref, flags } => {
                let value = self.0.state.borrow().heap.var_ref(var_ref)?.value.clone();
                if matches!(value, RawValue::Uninitialized) {
                    let name = self.property_key_to_js_string(key)?.to_utf8_lossy();
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Reference,
                        format!("{name} is not initialized"),
                    )));
                }
                Ok(Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: self.root_raw_value(&value)?,
                    writable: flags.writable,
                    enumerable: flags.enumerable,
                    configurable: flags.configurable,
                }))
            }
            PropertySnapshot::Accessor { get, set, flags } => {
                let get = get
                    .map(|id| ObjectRef::from_borrowed_handle(self.clone(), id))
                    .transpose()?
                    .map(CallableRef::from_validated_object);
                let set = set
                    .map(|id| ObjectRef::from_borrowed_handle(self.clone(), id))
                    .transpose()?
                    .map(CallableRef::from_validated_object);
                Ok(Some(CompleteOrdinaryPropertyDescriptor::Accessor {
                    get,
                    set,
                    enumerable: flags.enumerable,
                    configurable: flags.configurable,
                }))
            }
            PropertySnapshot::AutoInit => {
                self.materialize_auto_init_property(object, key)?;
                self.get_own_property(object, key)
            }
        }
    }

    /// Read a string property without materializing autoinit slots or running
    /// accessors, for diagnostics which must remain side-effect free.
    ///
    /// This mirrors QuickJS `get_prop_string`: an own property shadows the
    /// prototype even when it is not an ordinary string data property. Only
    /// when the own property is absent is exactly one prototype level checked.
    pub fn raw_string_property_for_diagnostics(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        raw_string_property_one_level(&self.0.state.borrow(), object.object_id(), key.atom())
    }

    fn materialize_auto_init_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let object_id = object.object_id();
        let (slot_index, initializer) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            let slot_index = usize::try_from(
                shape
                    .find(key.atom())
                    .ok_or(RuntimeError::Invariant("autoinit property disappeared"))?,
            )
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let initializer = match object.slots.get(slot_index) {
                Some(PropertySlot::AutoInit(initializer)) => *initializer,
                Some(
                    PropertySlot::Data(_) | PropertySlot::VarRef(_) | PropertySlot::Accessor { .. },
                ) => return Ok(()),
                None => {
                    return Err(RuntimeError::Invariant(
                        "autoinit property slot was missing",
                    ));
                }
            };
            (slot_index, initializer)
        };

        let initialized = (|| -> Result<Value, RuntimeError> {
            Ok(match initializer {
                AutoInitProperty::FunctionPrototype { realm } => {
                    let object_prototype =
                        self.0.state.borrow().heap.context(realm)?.object_prototype;
                    let object_prototype =
                        ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?;
                    let prototype = self.new_object(Some(&object_prototype))?;
                    self.define_function_data_property(
                        &prototype,
                        "constructor",
                        Value::Object(object.clone()),
                        true,
                        true,
                    )?;
                    Value::Object(prototype)
                }
                AutoInitProperty::NativeBuiltin {
                    realm,
                    target,
                    name,
                    length,
                    min_readable_args,
                } => {
                    let function_prototype = self
                        .0
                        .state
                        .borrow()
                        .heap
                        .context(realm)?
                        .function_prototype;
                    let function_prototype =
                        ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
                    let callable = self.new_native_builtin(
                        &function_prototype,
                        realm,
                        target,
                        min_readable_args,
                        name,
                        i32::from(length),
                    )?;
                    Value::Object(callable.as_object().clone())
                }
                AutoInitProperty::String { value, .. } => Value::String(JsString::from(value)),
                #[cfg(test)]
                AutoInitProperty::FailureProbe { .. } => {
                    return Err(RuntimeError::Invariant("autoinit failure probe"));
                }
            })
        })();
        let initialized = match initialized {
            Ok(initialized) => initialized,
            Err(initializer_error) => {
                // Once QuickJS has entered an autoinit callback, failure is
                // terminal for that slot: it becomes an ordinary undefined
                // data property and releases the stored realm edge.
                let mut state = self.0.state.borrow_mut();
                let cleanup = state.heap.replace_object_slot(
                    object_id,
                    slot_index,
                    PropertySlot::Data(RawValue::Undefined),
                )?;
                state.apply_cleanup(cleanup)?;
                return Err(initializer_error);
            }
        };
        let raw = self.raw_property_value(&initialized)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atoms = state.retain_slot_atoms(&[PropertySlot::Data(raw.clone())])?;
        let cleanup =
            match state
                .heap
                .replace_object_slot(object_id, slot_index, PropertySlot::Data(raw))
            {
                Ok(cleanup) => cleanup,
                Err(error) => {
                    state.release_atoms(retained_atoms)?;
                    return Err(error.into());
                }
            };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(initialized);
        Ok(())
    }

    fn prepare_get_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<PropertyGetAction, RuntimeError> {
        self.prepare_get_property_with_receiver(object, key, Value::Object(object.clone()))
    }

    fn prepare_get_property_or_missing(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<PropertyGetAction>, RuntimeError> {
        self.prepare_get_property_with_receiver_or_missing(
            object,
            key,
            Value::Object(object.clone()),
        )
    }

    fn prepare_get_property_with_receiver(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<PropertyGetAction, RuntimeError> {
        Ok(self
            .prepare_get_property_with_receiver_or_missing(object, key, receiver)?
            .unwrap_or(PropertyGetAction::Complete(Value::Undefined)))
    }

    fn prepare_get_property_with_receiver_or_missing(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Option<PropertyGetAction>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_value_domain(&receiver, "property receiver")?;
        let mut cursor = Some(object.clone());
        while let Some(current) = cursor {
            if let Some(property) = self.get_own_property(&current, key)? {
                return match property {
                    CompleteOrdinaryPropertyDescriptor::Data { value, .. } => {
                        Ok(Some(PropertyGetAction::Complete(value)))
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor { get: None, .. } => {
                        Ok(Some(PropertyGetAction::Complete(Value::Undefined)))
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor {
                        get: Some(getter), ..
                    } => Ok(Some(PropertyGetAction::Call { getter, receiver })),
                };
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(None)
    }

    fn prepare_set_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        let _operation = self.operation();
        self.prepare_set_property_with_receiver(object, key, value, Value::Object(object.clone()))
    }

    fn prepare_set_property_with_receiver(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_value_domain(&value, "property value")?;
        self.validate_value_domain(&receiver, "property receiver")?;
        let mut cursor = Some(object.clone());
        let mut inherited_allows_write = true;
        while let Some(current) = cursor {
            if let Some(property) = self.get_own_property(&current, key)? {
                match property {
                    CompleteOrdinaryPropertyDescriptor::Data { writable, .. } => {
                        inherited_allows_write = writable;
                        break;
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor { set: None, .. } => {
                        return Ok(PropertySetAction::Rejected(PropertySetRejection::NoSetter));
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor {
                        set: Some(setter), ..
                    } => {
                        return Ok(PropertySetAction::Call {
                            setter,
                            receiver,
                            argument: value,
                        });
                    }
                }
            }
            cursor = self.get_prototype_of(&current)?;
        }
        if !inherited_allows_write {
            return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
        }

        let Value::Object(receiver) = receiver else {
            return Ok(PropertySetAction::Rejected(PropertySetRejection::NotObject));
        };
        let descriptor = match self.get_own_property(&receiver, key)? {
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            }) => {
                return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
            }
            Some(CompleteOrdinaryPropertyDescriptor::Accessor { set: None, .. }) => {
                return Ok(PropertySetAction::Rejected(PropertySetRejection::NoSetter));
            }
            Some(CompleteOrdinaryPropertyDescriptor::Accessor { set: Some(_), .. }) => {
                return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
            }
            Some(CompleteOrdinaryPropertyDescriptor::Data { .. }) => OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                ..OrdinaryPropertyDescriptor::new()
            },
            None => {
                if !self.is_extensible(&receiver)? {
                    return Ok(PropertySetAction::Rejected(
                        PropertySetRejection::NotExtensible,
                    ));
                }
                OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            }
        };
        if self.define_own_property(&receiver, key, &descriptor)? {
            Ok(PropertySetAction::Complete)
        } else {
            Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly))
        }
    }

    /// Validate and apply an ordinary own-property descriptor.
    ///
    /// Ordinary semantic rejection is returned as `Ok(false)` so `Reflect`
    /// and throwing callers can make their distinct language-level choices.
    pub fn define_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_descriptor_domains(descriptor)?;
        if let Some(flags) = self.auto_init_own_property_flags(object, key)? {
            if descriptor.is_mixed_descriptor() {
                return Err(PropertyDefinitionError::InvalidDescriptor.into());
            }
            // QuickJS check_define_prop_flags checks only the lazy slot's
            // current attributes before JS_AutoInitProperty. Configurable
            // autoinit builtins therefore accept kind and attribute changes,
            // while non-configurable function prototypes and @@hasInstance
            // can reject impossible changes without allocating their value.
            if !flags.configurable
                && (matches!(descriptor.configurable, DescriptorField::Present(true))
                    || matches!(
                        descriptor.enumerable,
                        DescriptorField::Present(enumerable) if enumerable != flags.enumerable
                    )
                    || descriptor.is_accessor_descriptor()
                    || (!flags.writable
                        && matches!(descriptor.writable, DescriptorField::Present(true))))
            {
                return Ok(false);
            }
            // QuickJS performs compatibility checks against the lazy data
            // flags first, then materializes for every compatible define,
            // including an empty descriptor or `writable: false`.
            self.materialize_auto_init_property(object, key)?;
        }
        let current = self.get_own_property(object, key)?;
        let descriptor = descriptor_to_validation_record(descriptor);
        let current_record = current.as_ref().map(complete_to_validation_record);
        let complete = match validate_and_apply_property_descriptor(
            self.is_extensible(object)?,
            &descriptor,
            current_record.as_ref(),
            &Value::Undefined,
            Value::same_value,
        ) {
            Ok(complete) => complete,
            Err(PropertyDefinitionError::InvalidDescriptor) => {
                return Err(PropertyDefinitionError::InvalidDescriptor.into());
            }
            Err(_) => return Ok(false),
        };
        let complete = validation_record_to_complete(complete)?;
        self.store_complete_property(object, key, complete)?;
        Ok(true)
    }

    fn auto_init_own_property_flags(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<PropertyFlags>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        let shape = state.heap.shape(object.shape)?;
        let Some(index) = shape.find(key.atom()) else {
            return Ok(None);
        };
        let index = usize::try_from(index)
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
        Ok(
            matches!(object.slots.get(index), Some(PropertySlot::AutoInit(_)))
                .then_some(shape.entries()[index].flags),
        )
    }

    fn is_auto_init_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        Ok(self.auto_init_own_property_flags(object, key)?.is_some())
    }

    /// Test own-property presence without materializing autoinit payloads.
    pub fn has_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(state.heap.shape(object.shape)?.find(key.atom()).is_some())
    }

    /// Delete an ordinary own property without invoking accessors.
    pub fn delete_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        let global_var_ref = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            match &object_data.payload {
                ObjectPayload::GlobalObject { uninitialized_vars } => {
                    let shape = state.heap.shape(object_data.shape)?;
                    let Some(index) = shape.find(key.atom()) else {
                        return Ok(true);
                    };
                    let index = index as usize;
                    let entry = shape.entries().get(index).ok_or(RuntimeError::Invariant(
                        "shape lookup index was out of bounds",
                    ))?;
                    match object_data.slots.get(index).ok_or(RuntimeError::Invariant(
                        "shape property has no parallel object slot",
                    ))? {
                        PropertySlot::VarRef(var_ref)
                            if state.heap.var_ref_strong_count(*var_ref)? > 1 =>
                        {
                            Some((*uninitialized_vars, *var_ref, entry.flags.configurable))
                        }
                        PropertySlot::VarRef(_)
                        | PropertySlot::Data(_)
                        | PropertySlot::Accessor { .. }
                        | PropertySlot::AutoInit(_) => None,
                    }
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Error
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => None,
            }
        };
        if let Some((hidden, var_ref, configurable)) = global_var_ref {
            if !configurable {
                return Ok(false);
            }
            let root = VarRefRoot::from_borrowed_handle(self.clone(), var_ref)?;
            let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
            match self.own_var_ref_root(&hidden, key)? {
                Some(existing) if existing.id() != root.id() => {
                    return Err(RuntimeError::Invariant(
                        "hidden global table contains a different VarRef",
                    ));
                }
                Some(_) => {}
                None => self.store_property_slot(
                    &hidden,
                    key,
                    PropertyFlags::data(true, true, true),
                    PropertySlot::VarRef(root.id()),
                )?,
            }
            self.reset_var_ref_uninitialized(&root)?;
            self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
        }
        let mut state = self.0.state.borrow_mut();
        let object_id = object.object_id();
        let (prototype, entries, mut slots, index, configurable) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(true);
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let entry = *shape.entries().get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object_data.slots.clone(),
                index,
                entry.flags.configurable,
            )
        };
        if !configurable {
            return Ok(false);
        }

        let mut next_entries = entries;
        next_entries.remove(index);
        slots.remove(index);
        state.replace_layout(object_id, prototype, &next_entries, slots)?;
        Ok(true)
    }

    /// Return a rooted own-key snapshot in ECMAScript order.
    pub fn own_property_keys(&self, object: &ObjectRef) -> Result<Vec<PropertyKey>, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let atoms = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            state
                .heap
                .shape(object.shape)?
                .ordered_own_keys(&state.atoms)?
        };
        atoms
            .into_iter()
            .map(|atom| PropertyKey::from_borrowed_atom(self.clone(), atom).map_err(Into::into))
            .collect()
    }

    /// Return the ordinary object's prototype as a new root.
    pub fn get_prototype_of(&self, object: &ObjectRef) -> Result<Option<ObjectRef>, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let prototype = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            state.heap.shape(object.shape)?.prototype()
        };
        prototype
            .map(|prototype| ObjectRef::from_borrowed_handle(self.clone(), prototype))
            .transpose()
            .map_err(Into::into)
    }

    /// Apply ordinary `[[SetPrototypeOf]]`, including same-value success,
    /// immutable/non-extensible rejection and cycle detection.
    pub fn set_prototype_of(
        &self,
        object: &ObjectRef,
        prototype: Option<&ObjectRef>,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        if prototype.is_some_and(|prototype| !prototype.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let object_id = object.object_id();
        let prototype = prototype.map(ObjectRef::object_id);
        let mut state = self.0.state.borrow_mut();
        let (current, extensible, immutable, entries, slots) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            (
                shape.prototype(),
                object_data.extensible,
                object_data.immutable_prototype,
                shape.entries().to_vec(),
                object_data.slots.clone(),
            )
        };
        if current == prototype {
            return Ok(true);
        }
        if immutable || !extensible {
            return Ok(false);
        }

        let mut cursor = prototype;
        while let Some(candidate) = cursor {
            if candidate == object_id {
                return Ok(false);
            }
            let candidate = state.heap.object(candidate)?;
            cursor = state.heap.shape(candidate.shape)?.prototype();
        }
        state.replace_layout(object_id, prototype, &entries, slots)?;
        Ok(true)
    }

    /// Return the ordinary object's extensibility bit.
    pub fn is_extensible(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .extensible)
    }

    /// Make the ordinary object non-extensible.
    pub fn prevent_extensions(&self, object: &ObjectRef) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_object_extensible(object.object_id(), false)?;
        Ok(())
    }

    /// Run QuickJS-style cycle collection for this runtime.
    pub fn run_gc(&self) -> Result<GcStats, RuntimeError> {
        let _operation = self.operation();
        let mut state = self.0.state.borrow_mut();
        let mut stats = state.heap.run_gc()?;
        let atoms = std::mem::take(&mut stats.cleanup.atoms);
        state.unlink_finalized_shapes(stats.cleanup.finalized_shape_ids.iter().copied());
        state.release_atoms(atoms)?;
        Ok(stats)
    }

    /// Runtime heap population for diagnostics and lifecycle tests.
    #[must_use]
    pub fn heap_counts(&self) -> HeapCounts {
        let _operation = self.operation();
        self.0.state.borrow().heap.counts()
    }

    fn set_pending_exception(&self, value: Value) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "exception value")?;
        let raw = self.raw_property_value(&value)?;
        {
            let mut state = self.0.state.borrow_mut();
            state.retain_raw_root(&raw)?;
            if let Some(previous) = state.pending_exception.replace(raw) {
                state.release_owned_raw_root(previous)?;
            }
        }
        // `raw` now owns its own retained occurrence.
        drop(value);
        Ok(())
    }

    fn take_pending_exception(&self) -> Result<Option<Value>, RuntimeError> {
        let _operation = self.operation();
        let pending = self.0.state.borrow_mut().pending_exception.take();
        pending
            .map(|value| self.take_owned_raw_value(value))
            .transpose()
    }

    fn has_pending_exception(&self) -> bool {
        let _operation = self.operation();
        self.0.state.borrow().pending_exception.is_some()
    }

    fn take_owned_raw_value(&self, value: RawValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            RawValue::Undefined => Value::Undefined,
            RawValue::Null => Value::Null,
            RawValue::Bool(value) => Value::Bool(value),
            RawValue::Int(value) => Value::Int(value),
            RawValue::Float(value) => Value::Float(value),
            RawValue::BigInt(value) => Value::BigInt(value),
            RawValue::String(value) => Value::String(value),
            RawValue::Symbol(atom) => Value::Symbol(SymbolRef::from_owned_atom(self.clone(), atom)),
            RawValue::Object(object) => {
                Value::Object(ObjectRef::from_owned_handle(self.clone(), object))
            }
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel occupied the pending exception slot",
                ));
            }
        })
    }

    /// Consume a verified compiler draft and publish an immutable bytecode GC
    /// node in `realm`.
    fn publish_unlinked_function(
        &self,
        realm: ContextId,
        function: UnlinkedFunction,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        verify_unlinked_tree(&function)?;
        let flat_functions = flatten_unlinked_tree(function)?;
        let _operation = self.operation();
        let mut roots: Vec<Option<FunctionBytecodeRef>> = Vec::with_capacity(flat_functions.len());

        for function in flat_functions {
            let mut linked_constants = Vec::with_capacity(function.constants.len());
            let mut children = Vec::new();
            for constant in function.constants {
                match constant {
                    FlatConstant::Value(value) => {
                        linked_constants.push(BytecodeConstant::Value(value));
                    }
                    FlatConstant::Child(index) => {
                        let child = roots.get(index).and_then(Option::as_ref).ok_or(
                            RuntimeError::Invariant(
                                "flattened child function root was unavailable",
                            ),
                        )?;
                        linked_constants.push(BytecodeConstant::Function(child.bytecode_id()));
                        children.push(index);
                    }
                }
            }

            let mut closure_variables = function.closure_variables;
            let mut unlinked_debug = function.debug;
            let mut linked_debug = None;
            let mut auxiliary_atoms = Vec::new();
            let id = {
                let mut state = self.0.state.borrow_mut();
                let linking = (|| -> Result<(), RuntimeError> {
                    if let Some(debug) = unlinked_debug.take() {
                        let filename =
                            state.atoms.intern_property_key_js_string(&debug.filename)?;
                        auxiliary_atoms.push(filename);
                        linked_debug = Some(FunctionDebugInfo {
                            filename,
                            pc2line: debug.pc2line,
                            source: debug.source,
                        });
                    }
                    for descriptor in &mut closure_variables {
                        let ClosureVariableName::Constant(index) = descriptor.name else {
                            continue;
                        };
                        let name = usize::try_from(index)
                            .ok()
                            .and_then(|index| linked_constants.get(index))
                            .and_then(|constant| match constant {
                                BytecodeConstant::Value(RawValue::String(name)) => Some(name),
                                BytecodeConstant::Value(_) | BytecodeConstant::Function(_) => None,
                            })
                            .ok_or(RuntimeError::Invariant(
                                "verified closure name was not a string constant",
                            ))?;
                        let atom = state.atoms.intern_property_key_js_string(name)?;
                        auxiliary_atoms.push(atom);
                        descriptor.name = ClosureVariableName::Atom(atom);
                    }
                    Ok(())
                })();
                if let Err(error) = linking {
                    state.release_atoms(auxiliary_atoms.drain(..))?;
                    return Err(error);
                }

                let owned_atoms = auxiliary_atoms.clone();
                let bytecode = FunctionBytecodeData {
                    code: function.code.into(),
                    constants: linked_constants.into(),
                    realm,
                    metadata: function.metadata,
                    func_name: function.func_name,
                    closure_variables: closure_variables.into(),
                    debug: linked_debug,
                    auxiliary_atoms: auxiliary_atoms.into_boxed_slice(),
                };
                match state.heap.allocate_function_bytecode(bytecode) {
                    Ok(id) => id,
                    Err(error) => {
                        state.release_atoms(owned_atoms)?;
                        return Err(error.into());
                    }
                }
            };
            let root = FunctionBytecodeRef::from_owned_handle(self.clone(), id);

            // The parent node now owns each child through its cpool edge.
            for child in children {
                drop(roots[child].take());
            }
            roots.push(Some(root));
        }

        roots
            .last_mut()
            .and_then(Option::take)
            .ok_or(RuntimeError::Invariant(
                "unlinked function tree produced no published root",
            ))
    }

    /// Compile and publish source without mutating the runtime pending-
    /// exception slot. Native indirect-eval paths need the thrown value as a
    /// normal completion, while the public Context boundary installs that
    /// same value into the pending slot before returning `Exception`.
    fn compile_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
    ) -> Result<Compilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let function = match compile_unlinked_script_with_filename(source, filename, debug_info) {
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
                                    "syntax-error byte offset is invalid for its source",
                                )
                            })?;
                        Some(ExplicitBacktraceLocation {
                            filename: JsString::from_utf8(filename),
                            position,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let exception = if error.kind() == ErrorKind::Syntax {
                    self.new_native_error_without_backtrace(realm, kind, error.message())?
                } else {
                    self.new_native_error(realm, kind, error.message())?
                };
                self.ensure_error_backtrace(&exception, false, explicit_location)?;
                return Ok(Compilation::Throw(exception));
            }
        };
        Ok(Compilation::Published(
            self.publish_unlinked_function(realm, function)?,
        ))
    }

    fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let root = function.clone();
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        Ok(PublishedFunctionSnapshot {
            root,
            code: bytecode.code.clone(),
            constants: bytecode.constants.clone(),
            closure_variables: bytecode.closure_variables.clone(),
            metadata: bytecode.metadata,
            realm: bytecode.realm,
        })
    }

    #[cfg(test)]
    pub(crate) fn test_function_debug_location(
        &self,
        function: &FunctionBytecodeRef,
        pc: Option<usize>,
    ) -> Result<Option<(JsString, LineColumn)>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        let Some(debug) = &bytecode.debug else {
            return Ok(None);
        };
        let filename = state.atoms.to_js_string(debug.filename)?;
        let position = debug
            .pc2line
            .as_ref()
            .map(|table| table.lookup(pc.and_then(|pc| u32::try_from(pc).ok())));
        Ok(position.map(|position| (filename, position)))
    }

    #[cfg(test)]
    pub(crate) fn test_function_debug_source(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        Ok(state
            .heap
            .function_bytecode(function.bytecode_id())?
            .debug
            .as_ref()
            .and_then(|debug| debug.source.as_deref())
            .map(<[u8]>::to_vec))
    }

    #[cfg(test)]
    pub(crate) fn test_function_code(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Vec<crate::bytecode::Instruction>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(function.bytecode_id())?
            .code
            .to_vec())
    }

    #[cfg(test)]
    pub(crate) fn test_function_name(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<JsString>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(function.bytecode_id())?
            .func_name
            .clone())
    }

    #[cfg(test)]
    pub(crate) fn test_debug_filename_atom_ownership(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<(usize, Option<u32>)>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        let Some(filename) = bytecode.debug.as_ref().map(|debug| debug.filename) else {
            return Ok(None);
        };
        let local_ownership = bytecode
            .auxiliary_atoms
            .iter()
            .filter(|atom| **atom == filename)
            .count();
        let total_ref_count = state.atoms.resolve(filename)?.ref_count;
        Ok(Some((local_ownership, total_ref_count)))
    }

    #[cfg(test)]
    pub(crate) fn test_atom_count(&self) -> usize {
        self.0.state.borrow().atoms.len()
    }

    #[cfg(test)]
    pub(crate) fn test_child_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
        constant_index: usize,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let id = {
            let state = self.0.state.borrow();
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            match bytecode.constants.get(constant_index) {
                Some(BytecodeConstant::Function(id)) => *id,
                Some(BytecodeConstant::Value(_)) => {
                    return Err(RuntimeError::Invariant(
                        "requested child constant is a value",
                    ));
                }
                None => {
                    return Err(RuntimeError::Invariant(
                        "requested child constant is out of bounds",
                    ));
                }
            }
        };
        Ok(FunctionBytecodeRef::from_borrowed_handle(self.clone(), id)?)
    }

    fn push_active_frame(
        &self,
        function_root: ObjectRef,
        bytecode_root: Option<FunctionBytecodeRef>,
        realm: ContextId,
        flags: ActiveFrameFlags,
        kind: ActiveFrameKind,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        if !function_root.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("active-frame function"));
        }
        if bytecode_root
            .as_ref()
            .is_some_and(|root| !root.belongs_to(self))
        {
            return Err(RuntimeError::WrongRuntime("active-frame bytecode"));
        }

        let (token, depth) = {
            let mut state = self.0.state.borrow_mut();
            state.heap.context(realm)?;
            let object = state.heap.object(function_root.object_id())?;
            match (kind, &object.payload, bytecode_root.as_ref()) {
                (
                    ActiveFrameKind::Bytecode { bytecode, .. },
                    ObjectPayload::BytecodeFunction {
                        bytecode: object_bytecode,
                        ..
                    },
                    Some(root),
                ) if *object_bytecode == bytecode && root.bytecode_id() == bytecode => {
                    if state.heap.function_bytecode(bytecode)?.realm != realm {
                        return Err(RuntimeError::Invariant(
                            "bytecode active frame realm disagrees with its bytecode",
                        ));
                    }
                }
                (
                    ActiveFrameKind::Native {
                        target,
                        actual_arg_count,
                        readable_arg_count,
                    },
                    ObjectPayload::NativeFunction { data },
                    None,
                ) if data.target == target
                    && data.realm == Some(realm)
                    && readable_arg_count
                        == actual_arg_count.max(usize::from(data.min_readable_args)) => {}
                (ActiveFrameKind::Bytecode { .. }, _, _) => {
                    return Err(RuntimeError::Invariant(
                        "bytecode active frame disagrees with its rooted callable",
                    ));
                }
                (ActiveFrameKind::Native { .. }, _, _) => {
                    return Err(RuntimeError::Invariant(
                        "native active frame disagrees with its rooted callable",
                    ));
                }
            }

            let token = ActiveFrameToken(state.next_active_frame_token);
            state.next_active_frame_token =
                state
                    .next_active_frame_token
                    .checked_add(1)
                    .ok_or(RuntimeError::Invariant(
                        "active-frame token space was exhausted",
                    ))?;
            let depth = state.active_frames.len();
            state.active_frames.push(ActiveFrameRecord {
                token,
                function: function_root.object_id(),
                realm,
                flags,
                kind,
            });
            (token, depth)
        };

        Ok(ActiveFrameGuard {
            runtime: self.clone(),
            token,
            depth,
            active: true,
            _function_root: function_root,
            _bytecode_root: bytecode_root,
        })
    }

    fn push_bytecode_active_frame(
        &self,
        function_root: ObjectRef,
        bytecode_root: FunctionBytecodeRef,
        realm: ContextId,
        strict: bool,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        let bytecode = bytecode_root.bytecode_id();
        self.push_active_frame(
            function_root,
            Some(bytecode_root),
            realm,
            ActiveFrameFlags {
                strict,
                ..ActiveFrameFlags::default()
            },
            ActiveFrameKind::Bytecode { bytecode, pc: None },
        )
    }

    fn push_native_active_frame(
        &self,
        function_root: ObjectRef,
        realm: ContextId,
        target: NativeFunctionId,
        actual_arg_count: usize,
        readable_arg_count: usize,
    ) -> Result<ActiveFrameGuard, RuntimeError> {
        self.push_active_frame(
            function_root,
            None,
            realm,
            ActiveFrameFlags::default(),
            ActiveFrameKind::Native {
                target,
                actual_arg_count,
                readable_arg_count,
            },
        )
    }

    fn update_active_bytecode_pc(
        &self,
        token: ActiveFrameToken,
        pc: BytecodePc,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let frame = state
            .active_frames
            .last_mut()
            .ok_or(RuntimeError::Invariant(
                "bytecode PC update ran without an active frame",
            ))?;
        if frame.token != token {
            return Err(RuntimeError::Invariant(
                "bytecode PC update did not target the top active frame",
            ));
        }
        let ActiveFrameKind::Bytecode { pc: frame_pc, .. } = &mut frame.kind else {
            return Err(RuntimeError::Invariant(
                "bytecode PC update targeted a native active frame",
            ));
        };
        *frame_pc = Some(pc);
        Ok(())
    }

    /// QuickJS `JS_EVAL_FLAG_BACKTRACE_BARRIER` temporarily marks the frame
    /// which existed before eval begins. New eval/nested frames remain visible
    /// and stack traversal stops before printing this caller frame.
    fn install_backtrace_barrier(
        &self,
        enabled: bool,
    ) -> Result<BacktraceBarrierGuard, RuntimeError> {
        if !enabled {
            return Ok(BacktraceBarrierGuard {
                runtime: self.clone(),
                token: None,
                previous: false,
                active: true,
            });
        }
        let (token, previous) = {
            let mut state = self.0.state.borrow_mut();
            let Some(frame) = state.active_frames.last_mut() else {
                return Ok(BacktraceBarrierGuard {
                    runtime: self.clone(),
                    token: None,
                    previous: false,
                    active: true,
                });
            };
            let previous = frame.flags.backtrace_barrier;
            frame.flags.backtrace_barrier = true;
            (frame.token, previous)
        };
        Ok(BacktraceBarrierGuard {
            runtime: self.clone(),
            token: Some(token),
            previous,
            active: true,
        })
    }

    fn restore_backtrace_barrier(
        &self,
        token: ActiveFrameToken,
        previous: bool,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let frame = state
            .active_frames
            .iter_mut()
            .find(|frame| frame.token == token)
            .ok_or(RuntimeError::Invariant(
                "backtrace-barrier caller frame disappeared during eval",
            ))?;
        frame.flags.backtrace_barrier = previous;
        Ok(())
    }

    fn restore_backtrace_barrier_fallback(&self, token: ActiveFrameToken, previous: bool) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            if let Some(frame) = state
                .active_frames
                .iter_mut()
                .find(|frame| frame.token == token)
            {
                frame.flags.backtrace_barrier = previous;
            }
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_front(DeferredRefOp::BacktraceBarrierRestore { token, previous });
        }
    }

    fn pop_active_frame(&self, token: ActiveFrameToken, depth: usize) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        if state.active_frames.len() == depth + 1
            && state.active_frames.last().map(|frame| frame.token) == Some(token)
        {
            state.active_frames.pop();
            return Ok(());
        }

        if let Some(position) = state
            .active_frames
            .iter()
            .rposition(|frame| frame.token == token)
        {
            state.active_frames.truncate(position);
        } else if state.active_frames.len() > depth {
            state.active_frames.truncate(depth);
        }
        Err(RuntimeError::Invariant(
            "active frame stack was not restored in LIFO order",
        ))
    }

    fn pop_active_frame_fallback(&self, token: ActiveFrameToken, depth: usize) {
        if let Ok(mut state) = self.0.state.try_borrow_mut() {
            if let Some(position) = state
                .active_frames
                .iter()
                .rposition(|frame| frame.token == token)
            {
                state.active_frames.truncate(position);
            } else if state.active_frames.len() > depth {
                state.active_frames.truncate(depth);
            }
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_front(DeferredRefOp::ActiveFramePop { token, depth });
        }
    }

    fn bytecode_for_callable(
        &self,
        callable: &CallableRef,
    ) -> Result<CallableExecution, RuntimeError> {
        let _operation = self.operation();
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let (bytecode, closure_slots) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            match &object.payload {
                ObjectPayload::NativeFunction { data } => {
                    let realm = data.realm.ok_or(RuntimeError::Invariant(
                        "native function was called before its defining realm was attached",
                    ))?;
                    state.heap.context(realm)?;
                    return Ok(CallableExecution::Native {
                        target: data.target,
                        realm,
                        min_readable_args: data.min_readable_args,
                    });
                }
                ObjectPayload::BoundFunction {
                    target,
                    this_value,
                    arguments,
                } => {
                    let target = *target;
                    let this_value = this_value.clone();
                    let arguments = arguments.clone();
                    drop(state);
                    let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                    let target = CallableRef::from_validated_object(target);
                    let this_value = self.root_raw_value(&this_value)?;
                    let arguments = arguments
                        .iter()
                        .map(|argument| self.root_raw_value(argument))
                        .collect::<Result<Vec<_>, _>>()?;
                    return Ok(CallableExecution::Bound {
                        target,
                        this_value,
                        arguments,
                    });
                }
                ObjectPayload::BytecodeFunction {
                    bytecode,
                    closure_slots,
                    ..
                } => (*bytecode, closure_slots.clone()),
                ObjectPayload::Ordinary
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
            }
        };
        let bytecode = FunctionBytecodeRef::from_borrowed_handle(self.clone(), bytecode)?;
        let closure_slots = closure_slots
            .into_iter()
            .map(|id| VarRefRoot::from_borrowed_handle(self.clone(), id))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(CallableExecution::Bytecode {
            bytecode,
            closure_slots,
        })
    }

    fn callable_from_value(&self, value: Value) -> Result<CallableRef, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let is_callable = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
        );
        if !is_callable {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        }
        Ok(CallableRef::from_validated_object(object))
    }

    fn global_object_for_realm(&self, realm: ContextId) -> Result<ObjectRef, RuntimeError> {
        let global_object = self.0.state.borrow().heap.context(realm)?.global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.clone(),
            global_object,
        )?)
    }

    fn primitive_prototype_for_realm(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .primitive_prototypes[kind.index()]
        .ok_or(RuntimeError::Invariant(
            "primitive prototype is not implemented in this realm",
        ))?;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }

    fn concatenate_bound_arguments(
        &self,
        realm: ContextId,
        bound_arguments: &[Value],
        call_arguments: &[Value],
    ) -> Result<NativeConversion<Vec<Value>>, RuntimeError> {
        const MAX_CALL_ARGUMENTS: usize = 65_534;

        let Some(total) = bound_arguments.len().checked_add(call_arguments.len()) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        };
        if total > MAX_CALL_ARGUMENTS {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let mut arguments = Vec::with_capacity(total);
        arguments.extend_from_slice(bound_arguments);
        arguments.extend_from_slice(call_arguments);
        Ok(NativeConversion::Value(arguments))
    }

    fn call_internal(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.0.state.borrow().heap.context(caller_realm)?;
        self.validate_value_domain(&this_value, "call this value")?;
        for argument in arguments {
            self.validate_value_domain(argument, "call argument")?;
        }
        let mut callable = callable.clone();
        let mut this_value = this_value;
        let mut arguments = arguments.to_vec();
        loop {
            match self.bytecode_for_callable(&callable)? {
                CallableExecution::Bytecode {
                    bytecode,
                    closure_slots,
                } => {
                    return self.execute_bytecode_callable(
                        caller_realm,
                        &callable,
                        this_value,
                        Value::Undefined,
                        &arguments,
                        bytecode,
                        closure_slots,
                    );
                }
                CallableExecution::Native {
                    target,
                    realm,
                    min_readable_args,
                } => {
                    return self.call_native_function(
                        &callable,
                        realm,
                        target,
                        min_readable_args,
                        this_value,
                        &arguments,
                    );
                }
                CallableExecution::Bound {
                    target,
                    this_value: bound_this,
                    arguments: bound_arguments,
                } => {
                    arguments = match self.concatenate_bound_arguments(
                        caller_realm,
                        &bound_arguments,
                        &arguments,
                    )? {
                        NativeConversion::Value(arguments) => arguments,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    callable = target;
                    this_value = bound_this;
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn execute_bytecode_callable(
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
            closure_variables,
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
        let mut frame_locals = (0..metadata.local_count)
            .map(|_| FrameBinding::Direct(Value::Undefined))
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
        let mut host = RuntimeVmHost {
            runtime: self.clone(),
            active_frame_token: active_frame.token(),
            current_realm: realm,
            constants,
            closure_variables,
            closure_slots,
            arguments: frame_arguments,
            locals: frame_locals,
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

    fn construct_value_internal(
        &self,
        caller_realm: ContextId,
        function: Value,
        new_target: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let constructor = self.constructor_from_value(function)?;
        let new_target = self.constructor_from_value(new_target)?;
        self.construct_internal(caller_realm, &constructor, &new_target, arguments)
    }

    fn constructor_from_value(&self, value: Value) -> Result<CallableRef, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        let (is_callable, is_constructor) = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let callable = matches!(
                object_data.payload,
                ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. }
            );
            (callable, object_data.is_constructor)
        };
        if !is_constructor {
            return Err(RuntimeError::Engine(
                self.not_constructor_error(&object, is_callable)?,
            ));
        }
        if !is_callable {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        }
        Ok(CallableRef::from_validated_object(object))
    }

    fn not_constructor_error(
        &self,
        object: &ObjectRef,
        is_callable: bool,
    ) -> Result<Error, RuntimeError> {
        let message = if is_callable {
            let name = self.intern_property_key("name")?;
            match self.get_own_property(object, &name)? {
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: Value::String(name),
                    ..
                }) => format!("{} is not a constructor", name.to_utf8_lossy()),
                _ => "not a constructor".to_owned(),
            }
        } else {
            "not a constructor".to_owned()
        };
        Ok(Error::new(ErrorKind::Type, message))
    }

    fn construct_internal(
        &self,
        caller_realm: ContextId,
        constructor: &CallableRef,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.0.state.borrow().heap.context(caller_realm)?;
        if !constructor.belongs_to(self) || !new_target.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("constructor"));
        }
        for argument in arguments {
            self.validate_value_domain(argument, "construct argument")?;
        }
        let mut constructor = constructor.clone();
        let mut new_target = new_target.clone();
        let mut arguments = arguments.to_vec();
        loop {
            if !self.is_constructor(constructor.as_object())? {
                return Err(RuntimeError::Engine(
                    self.not_constructor_error(constructor.as_object(), true)?,
                ));
            }
            if !self.is_constructor(new_target.as_object())? {
                return Err(RuntimeError::Engine(
                    self.not_constructor_error(new_target.as_object(), true)?,
                ));
            }

            match self.bytecode_for_callable(&constructor)? {
                CallableExecution::Bound {
                    target,
                    this_value: _,
                    arguments: bound_arguments,
                } => {
                    arguments = match self.concatenate_bound_arguments(
                        caller_realm,
                        &bound_arguments,
                        &arguments,
                    )? {
                        NativeConversion::Value(arguments) => arguments,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if constructor.as_object() == new_target.as_object() {
                        new_target = target.clone();
                    }
                    constructor = target;
                }
                CallableExecution::Native {
                    target,
                    realm,
                    min_readable_args,
                } => {
                    return self.construct_native_function(
                        &constructor,
                        realm,
                        target,
                        min_readable_args,
                        &new_target,
                        &arguments,
                    );
                }
                CallableExecution::Bytecode {
                    bytecode,
                    closure_slots,
                } => {
                    let constructor_kind = self
                        .0
                        .state
                        .borrow()
                        .heap
                        .function_bytecode(bytecode.bytecode_id())?
                        .metadata
                        .constructor_kind;
                    match constructor_kind {
                        ConstructorKind::None => {
                            return Err(RuntimeError::Invariant(
                                "constructor bit disagrees with bytecode constructor metadata",
                            ));
                        }
                        ConstructorKind::Derived => {
                            return Err(RuntimeError::Engine(Error::internal(
                                "derived constructor execution is not implemented yet",
                            )));
                        }
                        ConstructorKind::Base => {}
                    }
                    let this_value =
                        match self.create_from_constructor(caller_realm, &new_target)? {
                            Completion::Return(value) => value,
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        };
                    let completion = self.execute_bytecode_callable(
                        caller_realm,
                        &constructor,
                        this_value.clone(),
                        Value::Object(new_target.as_object().clone()),
                        &arguments,
                        bytecode,
                        closure_slots,
                    )?;
                    return Ok(match completion {
                        Completion::Return(value @ Value::Object(_)) => Completion::Return(value),
                        Completion::Throw(value) => Completion::Throw(value),
                        Completion::Return(_) => Completion::Return(this_value),
                    });
                }
            }
        }
    }

    fn create_from_constructor(
        &self,
        caller_realm: ContextId,
        new_target: &CallableRef,
    ) -> Result<Completion, RuntimeError> {
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype = match self.prepare_get_property(new_target.as_object(), &prototype_key)? {
            PropertyGetAction::Complete(value) => value,
            PropertyGetAction::Call { getter, receiver } => {
                match self.call_internal(caller_realm, &getter, receiver, &[])? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        };
        let prototype = if let Value::Object(prototype) = prototype {
            prototype
        } else {
            let realm = self.callable_realm(new_target)?;
            let object_prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
            ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?
        };
        Ok(Completion::Return(Value::Object(
            self.new_object(Some(&prototype))?,
        )))
    }

    fn callable_realm(&self, callable: &CallableRef) -> Result<ContextId, RuntimeError> {
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("callable"));
        }
        let mut callable = callable.clone();
        loop {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            match &object.payload {
                ObjectPayload::NativeFunction { data } if data.realm.is_some() => {
                    let realm = data
                        .realm
                        .expect("guard proved native function has a defining realm");
                    state.heap.context(realm)?;
                    return Ok(realm);
                }
                ObjectPayload::BytecodeFunction { bytecode, .. } => {
                    let realm = state.heap.function_bytecode(*bytecode)?.realm;
                    state.heap.context(realm)?;
                    return Ok(realm);
                }
                ObjectPayload::BoundFunction { target, .. } => {
                    let target = *target;
                    drop(state);
                    let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                    callable = CallableRef::from_validated_object(target);
                }
                ObjectPayload::NativeFunction { .. } => {
                    return Err(RuntimeError::Invariant(
                        "native function has no defining realm",
                    ));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn construct_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Construct {
                new_target: Value::Object(new_target.as_object().clone()),
            },
            arguments,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn call_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        self.invoke_native_function(
            callable,
            realm,
            target,
            min_readable_args,
            NativeInvocation::Call { this_value },
            arguments,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn invoke_native_function(
        &self,
        callable: &CallableRef,
        realm: ContextId,
        target: NativeFunctionId,
        min_readable_args: u8,
        invocation: NativeInvocation,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        if !callable.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("native callable"));
        }
        self.0.state.borrow().heap.context(realm)?;

        // The callable root held by the caller owns the native payload and its
        // defining-realm edge for the whole invocation. Revalidate the
        // detached snapshot before recording raw identities in the frame.
        {
            let state = self.0.state.borrow();
            let object = state.heap.object(callable.as_object().object_id())?;
            let ObjectPayload::NativeFunction { data } = &object.payload else {
                return Err(RuntimeError::Invariant(
                    "native invocation target was not a native function",
                ));
            };
            if data.target != target
                || data.realm != Some(realm)
                || data.min_readable_args != min_readable_args
            {
                return Err(RuntimeError::Invariant(
                    "native invocation metadata changed after snapshot",
                ));
            }
        }

        let actual_arg_count = arguments.len();
        let available_arg_count = actual_arg_count.max(usize::from(min_readable_args));
        let mut readable = Vec::with_capacity(available_arg_count);
        readable.extend_from_slice(arguments);
        readable.resize(available_arg_count, Value::Undefined);
        let arguments = NativeArguments {
            actual_arg_count,
            readable,
        };
        let active_frame = self.push_native_active_frame(
            callable.as_object().clone(),
            realm,
            target,
            actual_arg_count,
            available_arg_count,
        )?;

        // JavaScript-style engine errors are materialized in the native
        // function's defining realm while its frame is still visible. A
        // pre-existing Error returned as an ordinary Throw completion is not
        // captured here: QuickJS pops the C frame first and lets the enclosing
        // bytecode exception boundary add any missing stack.
        let result = (|| match self.dispatch_native_function(target, realm, invocation, &arguments)
        {
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible native error");
                let value = self.new_native_error(realm, kind, error.message())?;
                Ok(Completion::Throw(value))
            }
            result => result,
        })();
        active_frame.finish()?;
        result
    }

    fn active_function(&self) -> Result<ObjectRef, RuntimeError> {
        let function = self
            .0
            .state
            .borrow()
            .active_frames
            .last()
            .ok_or(RuntimeError::Invariant(
                "active function was requested without an active frame",
            ))?
            .function;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), function)?)
    }

    /// QuickJS `is_backtrace_needed` plus `build_backtrace`.
    ///
    /// Only real Error-class objects without any own `stack` property are
    /// eligible. Function names are read from raw ordinary data slots so this
    /// path never invokes user code while an exception is already in flight.
    fn ensure_error_backtrace(
        &self,
        value: &Value,
        skip_first_frame: bool,
        explicit_location: Option<ExplicitBacktraceLocation>,
    ) -> Result<(), RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(());
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("backtrace Error object"));
        }

        let stack_key = self.intern_property_key("stack")?;
        let needs_backtrace = {
            let state = self.0.state.borrow();
            let data = state.heap.object(object.object_id())?;
            if !matches!(data.payload, ObjectPayload::Error) {
                false
            } else {
                state
                    .heap
                    .shape(data.shape)?
                    .find(stack_key.atom())
                    .is_none()
            }
        };
        if !needs_backtrace {
            return Ok(());
        }

        let name_key = self.intern_property_key("name")?;
        let stack = self.build_backtrace_string(
            name_key.atom(),
            skip_first_frame,
            explicit_location.as_ref(),
        )?;

        // Parse errors add SpiderMonkey-compatible metadata before `stack`,
        // exactly as QuickJS does. Rejection (for example after
        // preventExtensions) is intentionally silent: build_backtrace must
        // not replace the original JavaScript completion.
        if let Some(location) = explicit_location {
            let Some((line, column)) = location.position.one_based() else {
                return Err(RuntimeError::Invariant(
                    "backtrace location cannot be represented one-based",
                ));
            };
            let line = i32::try_from(line).map_err(|_| {
                RuntimeError::Invariant("backtrace line does not fit an ECMAScript Int32")
            })?;
            let column = i32::try_from(column).map_err(|_| {
                RuntimeError::Invariant("backtrace column does not fit an ECMAScript Int32")
            })?;
            for (name, property_value) in [
                ("fileName", Value::String(location.filename)),
                ("lineNumber", Value::Int(line)),
                ("columnNumber", Value::Int(column)),
            ] {
                if !self.define_backtrace_property(object, name, property_value)? {
                    return Ok(());
                }
            }
        }

        let _ = self.define_backtrace_property(object, "stack", Value::String(stack))?;
        Ok(())
    }

    fn define_backtrace_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
    }

    fn build_backtrace_string(
        &self,
        name_atom: Atom,
        mut skip_first_frame: bool,
        explicit_location: Option<&ExplicitBacktraceLocation>,
    ) -> Result<JsString, RuntimeError> {
        let state = self.0.state.borrow();
        let mut output = Vec::<u16>::new();

        if let Some(location) = explicit_location {
            let (line, column) = location
                .position
                .one_based()
                .ok_or(RuntimeError::Invariant(
                    "backtrace location cannot be represented one-based",
                ))?;
            append_backtrace_ascii(&mut output, "    at ");
            append_backtrace_string(&mut output, &location.filename);
            append_backtrace_ascii(&mut output, ":");
            append_backtrace_ascii(&mut output, &line.to_string());
            append_backtrace_ascii(&mut output, ":");
            append_backtrace_ascii(&mut output, &column.to_string());
            append_backtrace_ascii(&mut output, "\n");
        }

        for frame in state.active_frames.iter().rev() {
            if frame.flags.backtrace_barrier {
                break;
            }
            if skip_first_frame {
                skip_first_frame = false;
                continue;
            }

            let name = raw_string_property_one_level(&state, frame.function, name_atom)?
                .map(truncate_backtrace_c_string)
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| JsString::from("<anonymous>"));
            append_backtrace_ascii(&mut output, "    at ");
            append_backtrace_string(&mut output, &name);

            match frame.kind {
                ActiveFrameKind::Native { .. } => {
                    append_backtrace_ascii(&mut output, " (native)");
                }
                ActiveFrameKind::Bytecode { bytecode, pc } => {
                    let bytecode = state.heap.function_bytecode(bytecode)?;
                    if let Some(debug) = &bytecode.debug {
                        let filename = state.atoms.to_js_string(debug.filename)?;
                        append_backtrace_ascii(&mut output, " (");
                        append_backtrace_string(&mut output, &filename);
                        if let Some(table) = &debug.pc2line {
                            let pc = pc
                                .map(BytecodePc::index)
                                .map(u32::try_from)
                                .transpose()
                                .map_err(|_| {
                                    RuntimeError::Invariant(
                                        "active bytecode PC does not fit debug metadata",
                                    )
                                })?;
                            let (line, column) =
                                table.lookup(pc).one_based().ok_or(RuntimeError::Invariant(
                                    "bytecode debug position cannot be represented one-based",
                                ))?;
                            append_backtrace_ascii(&mut output, ":");
                            append_backtrace_ascii(&mut output, &line.to_string());
                            append_backtrace_ascii(&mut output, ":");
                            append_backtrace_ascii(&mut output, &column.to_string());
                        }
                        append_backtrace_ascii(&mut output, ")");
                    }
                }
            }
            append_backtrace_ascii(&mut output, "\n");
        }

        Ok(JsString::from_utf16(output))
    }

    fn get_property_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        Ok(self
            .get_property_or_missing_in_realm(realm, object, key)?
            .unwrap_or(Completion::Return(Value::Undefined)))
    }

    fn get_value_property_in_realm(
        &self,
        realm: ContextId,
        receiver: Value,
        key: &PropertyKey,
    ) -> Result<Completion, RuntimeError> {
        let action = match &receiver {
            Value::Object(object) => {
                self.prepare_get_property_with_receiver(object, key, receiver.clone())?
            }
            Value::Bool(_) => {
                let prototype =
                    self.primitive_prototype_for_realm(realm, PrimitiveKind::Boolean)?;
                self.prepare_get_property_with_receiver(&prototype, key, receiver.clone())?
            }
            Value::Undefined
            | Value::Null
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => {
                return Err(RuntimeError::Engine(Error::internal(
                    "primitive value property lookup is not implemented yet",
                )));
            }
        };
        match action {
            PropertyGetAction::Complete(value) => Ok(Completion::Return(value)),
            PropertyGetAction::Call { getter, receiver } => {
                self.call_internal(realm, &getter, receiver, &[])
            }
        }
    }

    fn get_property_or_missing_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<Completion>, RuntimeError> {
        match self.prepare_get_property_or_missing(object, key)? {
            None => Ok(None),
            Some(PropertyGetAction::Complete(value)) => Ok(Some(Completion::Return(value))),
            Some(PropertyGetAction::Call { getter, receiver }) => {
                self.call_internal(realm, &getter, receiver, &[]).map(Some)
            }
        }
    }

    fn has_property(&self, object: &ObjectRef, key: &PropertyKey) -> Result<bool, RuntimeError> {
        let mut cursor = Some(object.clone());
        while let Some(current) = cursor {
            if self.has_own_property(&current, key)? {
                return Ok(true);
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(false)
    }

    fn native_to_js_string(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::String)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value.to_js_string() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    kind,
                    error.message(),
                )?))
            }
        }
    }

    fn native_to_dynamic_source_fragment(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<String>, RuntimeError> {
        let value = match self.native_to_js_string(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let units = value.utf16_units().collect::<Vec<_>>();
        match String::from_utf16(&units) {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "dynamic Function source containing a lone UTF-16 surrogate is not implemented",
            )?)),
        }
    }

    fn native_to_number(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        let value = if matches!(value, Value::Object(_)) {
            match self.to_primitive(realm, value.clone(), ToPrimitiveHint::Number)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        } else {
            value.clone()
        };
        match value.to_number() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    kind,
                    error.message(),
                )?))
            }
        }
    }

    fn native_to_length(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<u64>, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        let number = match self.native_to_number(realm, value)? {
            NativeConversion::Value(number) => number,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let length = if number.is_nan() || number <= 0.0 {
            0
        } else if number >= MAX_SAFE_INTEGER as f64 {
            MAX_SAFE_INTEGER
        } else {
            // This branch is finite, positive and below 2^53, so the
            // truncating cast is the exact ToIntegerOrInfinity result.
            number as u64
        };
        Ok(NativeConversion::Value(length))
    }

    fn to_primitive(
        &self,
        realm: ContextId,
        value: Value,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(Completion::Return(value));
        };
        let to_primitive = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToPrimitive));
        let exotic = match self.get_property_in_realm(realm, &object, &to_primitive)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !matches!(exotic, Value::Undefined | Value::Null) {
            let Value::Object(exotic_object) = exotic else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            let Some(exotic) = self.as_callable(&exotic_object)? else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            };
            return match self.call_internal(
                realm,
                &exotic,
                Value::Object(object),
                &[Value::String(JsString::from(match hint {
                    ToPrimitiveHint::String => "string",
                    ToPrimitiveHint::Number => "number",
                    ToPrimitiveHint::Default => "default",
                }))],
            )? {
                Completion::Return(Value::Object(_)) => Ok(Completion::Throw(
                    self.new_native_error(realm, NativeErrorKind::Type, "toPrimitive")?,
                )),
                completion => Ok(completion),
            };
        }

        let ordinary_methods = match hint {
            ToPrimitiveHint::String => ["toString", "valueOf"],
            ToPrimitiveHint::Number | ToPrimitiveHint::Default => ["valueOf", "toString"],
        };
        for name in ordinary_methods {
            let key = self.intern_property_key(name)?;
            let method = match self.get_property_in_realm(realm, &object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let Value::Object(method_object) = method else {
                continue;
            };
            let Some(method) = self.as_callable(&method_object)? else {
                continue;
            };
            match self.call_internal(realm, &method, Value::Object(object.clone()), &[])? {
                Completion::Return(Value::Object(_)) => {}
                completion => return Ok(completion),
            }
        }
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "toPrimitive",
        )?))
    }

    fn call_throw_type_error(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "%ThrowTypeError% did not receive a generic invocation",
            ));
        };

        // QuickJS keeps the ES5-compatible sloppy ordinary-function getter
        // exception: reading `.caller`/`.arguments` returns undefined when
        // the receiver has bytecode, is non-strict, has a prototype and the
        // shared poison function was invoked without a setter argument.
        let sloppy_legacy_get = if arguments.actual_arg_count == 0 {
            match this_value {
                Value::Object(object) => {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(object.object_id())?;
                    match object.payload {
                        ObjectPayload::BytecodeFunction { bytecode, .. } => {
                            let metadata = state.heap.function_bytecode(bytecode)?.metadata;
                            !metadata.strict && metadata.has_prototype
                        }
                        ObjectPayload::Ordinary
                        | ObjectPayload::Primitive(_)
                        | ObjectPayload::GlobalObject { .. }
                        | ObjectPayload::Error
                        | ObjectPayload::BoundFunction { .. }
                        | ObjectPayload::NativeFunction { .. } => false,
                    }
                }
                Value::Undefined
                | Value::Null
                | Value::Bool(_)
                | Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_) => false,
            }
        } else {
            false
        };
        if sloppy_legacy_get {
            return Ok(Completion::Return(Value::Undefined));
        }
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "invalid property access",
        )?))
    }

    fn call_function_prototype_call(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.call did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        if arguments.actual_arg_count == 0 {
            return self.call_internal(realm, &target, Value::Undefined, &[]);
        }
        let this_argument = arguments.readable[0].clone();
        self.call_internal(
            realm,
            &target,
            this_argument,
            &arguments.readable[1..arguments.actual_arg_count],
        )
    }

    fn call_function_constructor(
        &self,
        realm: ContextId,
        kind: DynamicFunctionKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function constructor did not receive constructor-or-function invocation",
            ));
        };

        // Match js_function_constructor byte-for-byte: all parameter strings
        // precede the final body string, argc == 0 reads no padded argument,
        // and the complete wrapper is parsed as one indirect-eval unit so a
        // body strict directive can retroactively validate the parameters.
        let mut source = String::from("(");
        match kind {
            DynamicFunctionKind::Normal | DynamicFunctionKind::Generator => {}
            DynamicFunctionKind::Async | DynamicFunctionKind::AsyncGenerator => {
                source.push_str("async ");
            }
        }
        source.push_str("function");
        if matches!(
            kind,
            DynamicFunctionKind::Generator | DynamicFunctionKind::AsyncGenerator
        ) {
            source.push('*');
        }
        source.push_str(" anonymous(");

        let parameter_count = arguments.actual_arg_count.saturating_sub(1);
        for index in 0..parameter_count {
            if index != 0 {
                source.push(',');
            }
            let parameter =
                match self.native_to_dynamic_source_fragment(realm, &arguments.readable[index])? {
                    NativeConversion::Value(parameter) => parameter,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            source.push_str(&parameter);
        }
        source.push_str("\n) {\n");
        if arguments.actual_arg_count != 0 {
            let body_index = arguments.actual_arg_count - 1;
            let body = match self
                .native_to_dynamic_source_fragment(realm, &arguments.readable[body_index])?
            {
                NativeConversion::Value(body) => body,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            source.push_str(&body);
        }
        source.push_str("\n})");

        let script = match self.compile_in_realm(realm, &source, DEFAULT_EVAL_FILENAME)? {
            Compilation::Published(script) => script,
            Compilation::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let script_callable = self.new_bytecode_closure(realm, &script)?;
        let global_object = {
            let global_object = self.0.state.borrow().heap.context(realm)?.global_object;
            ObjectRef::from_borrowed_handle(self.clone(), global_object)?
        };
        let value =
            match self.call_internal(realm, &script_callable, Value::Object(global_object), &[])? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };

        if matches!(new_target, Value::Undefined) {
            return Ok(Completion::Return(value));
        }
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "Function constructor new.target was neither undefined nor an object",
            ));
        };
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype = match self.get_property_in_realm(realm, &new_target, &prototype_key)? {
            Completion::Return(Value::Object(prototype)) => prototype,
            Completion::Return(_) => {
                let new_target = self.callable_from_value(Value::Object(new_target))?;
                let fallback_realm = self.callable_realm(&new_target)?;
                let prototype = match kind {
                    DynamicFunctionKind::Normal => {
                        self.0
                            .state
                            .borrow()
                            .heap
                            .context(fallback_realm)?
                            .function_prototype
                    }
                    DynamicFunctionKind::Generator
                    | DynamicFunctionKind::Async
                    | DynamicFunctionKind::AsyncGenerator => {
                        return Err(RuntimeError::Invariant(
                            "dynamic Function kind has no intrinsic prototype yet",
                        ));
                    }
                };
                ObjectRef::from_borrowed_handle(self.clone(), prototype)?
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Value::Object(function) = value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        if !self.set_prototype_of(&function, Some(&prototype))? {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "prototype is immutable",
            )?));
        }
        Ok(Completion::Return(Value::Object(function)))
    }

    fn call_function_prototype_apply(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_APPLY_ARGUMENTS: u64 = 65_534;

        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.apply did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let this_argument = arguments.readable[0].clone();
        let array_argument = &arguments.readable[1];
        if matches!(array_argument, Value::Undefined | Value::Null) {
            return self.call_internal(realm, &target, this_argument, &[]);
        }
        let Value::Object(array_like) = array_argument else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a object",
            )?));
        };

        let length_key = self.intern_property_key("length")?;
        let length_value = match self.get_property_in_realm(realm, array_like, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length_value)? {
            NativeConversion::Value(length) => length,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if length > MAX_APPLY_ARGUMENTS {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "too many arguments in function call (only 65534 allowed)",
            )?));
        }

        let length = usize::try_from(length)
            .map_err(|_| RuntimeError::Invariant("apply length does not fit usize"))?;
        let mut forwarded = Vec::with_capacity(length);
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            match self.get_property_in_realm(realm, array_like, &key)? {
                Completion::Return(value) => forwarded.push(value),
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }
        self.call_internal(realm, &target, this_argument, &forwarded)
    }

    fn call_function_prototype_bind(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.bind did not receive a generic invocation",
            ));
        };
        let Value::Object(target_object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(target) = self.as_callable(&target_object)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let bound_argument_count = arguments.actual_arg_count.saturating_sub(1);
        let bound_arguments = if arguments.actual_arg_count > 1 {
            &arguments.readable[1..arguments.actual_arg_count]
        } else {
            &[]
        };
        let bound =
            self.new_bound_function(realm, &target, &arguments.readable[0], bound_arguments)?;

        let length_key = self.intern_property_key("length")?;
        let length = if self.has_own_property(&target_object, &length_key)? {
            let value = match self.get_property_in_realm(realm, &target_object, &length_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            bound_function_length(&value, bound_argument_count)?
        } else {
            Value::Int(0)
        };
        self.define_function_data_property(bound.as_object(), "length", length, false, true)?;

        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &target_object, &name_key)? {
            Completion::Return(Value::String(name)) => name,
            Completion::Return(_) => JsString::from(""),
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let name = JsString::from("bound ").concat(&name);
        self.define_function_data_property(
            bound.as_object(),
            "name",
            Value::String(name),
            false,
            true,
        )?;
        Ok(Completion::Return(Value::Object(bound.into_object())))
    }

    fn call_function_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };

        let (is_callable, source, function_kind) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            match &object.payload {
                ObjectPayload::BytecodeFunction { bytecode, .. } => {
                    let bytecode = state.heap.function_bytecode(*bytecode)?;
                    (
                        true,
                        bytecode
                            .debug
                            .as_ref()
                            .and_then(|debug| debug.source.clone()),
                        bytecode.metadata.function_kind,
                    )
                }
                ObjectPayload::NativeFunction { .. } | ObjectPayload::BoundFunction { .. } => {
                    (true, None, FunctionKind::Normal)
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error => (false, None, FunctionKind::Normal),
            }
        };
        if !is_callable {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        }
        if let Some(source) = source {
            let source = std::str::from_utf8(&source).map_err(|_| {
                RuntimeError::Invariant("published function source was not valid UTF-8")
            })?;
            return Ok(Completion::Return(Value::String(JsString::from_utf8(
                source,
            ))));
        }

        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &function, &name_key)? {
            Completion::Return(Value::Undefined) => JsString::from(""),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(name) => name,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let prefix = match function_kind {
            FunctionKind::Normal => "function ",
            FunctionKind::Generator => "function *",
            FunctionKind::Async => "async function ",
            FunctionKind::AsyncGenerator => "async function *",
        };
        let source = JsString::from(prefix)
            .concat(&name)
            .concat(&JsString::from("() {\n    [native code]\n}"));
        Ok(Completion::Return(Value::String(source)))
    }

    fn call_function_prototype_file_name(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.fileName getter received the wrong native invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let filename = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let bytecode = state.heap.function_bytecode(*bytecode)?;
            bytecode
                .debug
                .as_ref()
                .map(|debug| state.atoms.to_js_string(debug.filename))
                .transpose()?
        };
        Ok(Completion::Return(
            filename.map_or(Value::Undefined, Value::String),
        ))
    }

    fn call_function_prototype_position(
        &self,
        invocation: NativeInvocation,
        selector: FunctionDebugPosition,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype position getter received the wrong native invocation",
            ));
        };
        let Value::Object(function) = this_value else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let position = {
            let state = self.0.state.borrow();
            let object = state.heap.object(function.object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Ok(Completion::Return(Value::Undefined));
            };
            let bytecode = state.heap.function_bytecode(*bytecode)?;
            bytecode
                .debug
                .as_ref()
                .map(|debug| debug.pc2line.as_ref().map(|table| table.lookup(None)))
        };
        let Some(position) = position else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let Some(position) = position else {
            return Ok(Completion::Return(Value::Int(0)));
        };
        let (line, column) = position.one_based().ok_or(RuntimeError::Invariant(
            "function definition position cannot be represented one-based",
        ))?;
        let selected = match selector {
            FunctionDebugPosition::Line => line,
            FunctionDebugPosition::Column => column,
        };
        let selected = i32::try_from(selected).map_err(|_| {
            RuntimeError::Invariant("function definition position does not fit Int32")
        })?;
        Ok(Completion::Return(Value::Int(selected)))
    }

    fn call_function_prototype_has_instance(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype[Symbol.hasInstance] did not receive a generic invocation",
            ));
        };
        let Value::Object(target) = this_value else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let Some(_target_callable) = self.as_callable(&target)? else {
            return Ok(Completion::Return(Value::Bool(false)));
        };
        let target = CallableRef::from_validated_object(target);
        self.ordinary_is_instance_of(
            realm,
            &target,
            arguments
                .readable
                .first()
                .cloned()
                .unwrap_or(Value::Undefined),
        )
    }

    fn ordinary_is_instance_of(
        &self,
        mut realm: ContextId,
        target: &CallableRef,
        candidate: Value,
    ) -> Result<Completion, RuntimeError> {
        let mut target = target.clone();
        let mut delegated_standard_frames = Vec::new();
        let has_instance = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::HasInstance));
        let result = (|| -> Result<Completion, RuntimeError> {
            loop {
                let bound_target = {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(target.as_object().object_id())?;
                    match &object.payload {
                        ObjectPayload::BoundFunction { target, .. } => Some(*target),
                        ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::BytecodeFunction { .. } => None,
                        ObjectPayload::Ordinary
                        | ObjectPayload::Primitive(_)
                        | ObjectPayload::GlobalObject { .. }
                        | ObjectPayload::Error => {
                            return Err(RuntimeError::Invariant(
                                "ordinary instanceof received a non-callable target",
                            ));
                        }
                    }
                };

                let Some(bound_target) = bound_target else {
                    let Value::Object(candidate) = &candidate else {
                        return Ok(Completion::Return(Value::Bool(false)));
                    };
                    let prototype_key = self.intern_property_key("prototype")?;
                    let prototype = match self.get_property_in_realm(
                        realm,
                        target.as_object(),
                        &prototype_key,
                    )? {
                        Completion::Return(Value::Object(prototype)) => prototype,
                        Completion::Return(_) => {
                            return Ok(Completion::Throw(self.new_native_error(
                                realm,
                                NativeErrorKind::Type,
                                "operand 'prototype' property is not an object",
                            )?));
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };

                    let mut cursor = self.get_prototype_of(candidate)?;
                    while let Some(current) = cursor {
                        if current == prototype {
                            return Ok(Completion::Return(Value::Bool(true)));
                        }
                        cursor = self.get_prototype_of(&current)?;
                    }
                    return Ok(Completion::Return(Value::Bool(false)));
                };

                // QuickJS bound OrdinaryHasInstance delegates through the full
                // JS_IsInstanceOf path. Perform every observable GetMethod in
                // order, but trampoline direct calls to the inherited standard
                // method so a deep bound chain cannot recurse through Rust's
                // host stack. Synthetic native frames preserve backtraces.
                let target_object = ObjectRef::from_borrowed_handle(self.clone(), bound_target)?;
                let method =
                    match self.get_property_in_realm(realm, &target_object, &has_instance)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                if matches!(method, Value::Undefined | Value::Null) {
                    let Some(next_target) = self.as_callable(&target_object)? else {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "invalid 'instanceof' right operand",
                        )?));
                    };
                    target = next_target;
                    continue;
                }

                let Value::Object(method_object) = method else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                let Some(method) = self.as_callable(&method_object)? else {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?));
                };
                let standard_method = {
                    let state = self.0.state.borrow();
                    let object = state.heap.object(method.as_object().object_id())?;
                    match &object.payload {
                        ObjectPayload::NativeFunction { data }
                            if data.target == NativeFunctionId::FunctionPrototypeHasInstance =>
                        {
                            Some((
                                data.realm.ok_or(RuntimeError::Invariant(
                                    "standard hasInstance method has no defining realm",
                                ))?,
                                data.min_readable_args,
                            ))
                        }
                        ObjectPayload::Ordinary
                        | ObjectPayload::Primitive(_)
                        | ObjectPayload::GlobalObject { .. }
                        | ObjectPayload::Error
                        | ObjectPayload::NativeFunction { .. }
                        | ObjectPayload::BoundFunction { .. }
                        | ObjectPayload::BytecodeFunction { .. } => None,
                    }
                };
                if let Some((method_realm, min_readable_args)) = standard_method {
                    let readable_arg_count = 1usize.max(usize::from(min_readable_args));
                    delegated_standard_frames.push(self.push_native_active_frame(
                        method.as_object().clone(),
                        method_realm,
                        NativeFunctionId::FunctionPrototypeHasInstance,
                        1,
                        readable_arg_count,
                    )?);
                    let Some(next_target) = self.as_callable(&target_object)? else {
                        return Ok(Completion::Return(Value::Bool(false)));
                    };
                    realm = method_realm;
                    target = next_target;
                    continue;
                }

                return Ok(
                    match self.call_internal(
                        realm,
                        &method,
                        Value::Object(target_object),
                        std::slice::from_ref(&candidate),
                    )? {
                        Completion::Return(value) => {
                            Completion::Return(Value::Bool(value.to_boolean()))
                        }
                        Completion::Throw(value) => Completion::Throw(value),
                    },
                );
            }
        })();

        let mut frame_error = None;
        while let Some(frame) = delegated_standard_frames.pop() {
            if let Err(error) = frame.finish() {
                if frame_error.is_none() {
                    frame_error = Some(error);
                }
            }
        }
        if let Some(error) = frame_error {
            return Err(error);
        }
        result
    }

    fn call_primitive_constructor(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if kind != PrimitiveKind::Boolean {
            return Err(RuntimeError::Invariant(
                "unimplemented primitive constructor reached native dispatch",
            ));
        }
        let value = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "Boolean constructor readable argv was not padded to one",
            ))?
            .to_boolean();
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive constructor did not receive constructor-or-function invocation",
            ));
        };
        if matches!(new_target, Value::Undefined) {
            return Ok(Completion::Return(Value::Bool(value)));
        }
        let Value::Object(new_target) = new_target else {
            return Err(RuntimeError::Invariant(
                "primitive constructor new.target was neither undefined nor an object",
            ));
        };
        let new_target = self.callable_from_value(Value::Object(new_target))?;
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(realm, new_target.as_object(), &prototype_key)? {
                Completion::Return(Value::Object(prototype)) => prototype,
                Completion::Return(_) => {
                    let fallback_realm = self.callable_realm(&new_target)?;
                    self.primitive_prototype_for_realm(fallback_realm, kind)?
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        Ok(Completion::Return(Value::Object(
            self.new_primitive_object(&prototype, kind, Value::Bool(value))?,
        )))
    }

    fn call_global_number_parse(
        &self,
        realm: ContextId,
        kind: NumberParseKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global numeric parser did not receive a generic call",
            ));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "global numeric parser argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = match kind {
            NumberParseKind::ParseFloat => crate::number_parse::parse_float(&input),
            NumberParseKind::ParseInt => {
                let radix = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "parseInt radix argv was not padded",
                ))?;
                let radix = match self.native_to_number(realm, radix)? {
                    NativeConversion::Value(value) => crate::number::to_int32(value),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                crate::number_parse::parse_int(&input, radix)
            }
        };
        Ok(Completion::Return(Value::number(result)))
    }

    fn primitive_this_value(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        this_value: Value,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        let direct = matches!(
            (&this_value, kind),
            (Value::Int(_) | Value::Float(_), PrimitiveKind::Number)
                | (Value::String(_), PrimitiveKind::String)
                | (Value::Bool(_), PrimitiveKind::Boolean)
                | (Value::Symbol(_), PrimitiveKind::Symbol)
                | (Value::BigInt(_), PrimitiveKind::BigInt)
        );
        if direct {
            return Ok(NativeConversion::Value(this_value));
        }
        if let Value::Object(object) = &this_value {
            let payload = {
                let state = self.0.state.borrow();
                match &state.heap.object(object.object_id())?.payload {
                    ObjectPayload::Primitive(PrimitiveObjectData::Boolean(value))
                        if kind == PrimitiveKind::Boolean =>
                    {
                        Some(Value::Bool(*value))
                    }
                    ObjectPayload::Ordinary
                    | ObjectPayload::Primitive(_)
                    | ObjectPayload::GlobalObject { .. }
                    | ObjectPayload::Error
                    | ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. } => None,
                }
            };
            if let Some(payload) = payload {
                return Ok(NativeConversion::Value(payload));
            }
        }
        let message = match kind {
            PrimitiveKind::Number => "not a number",
            PrimitiveKind::String => "not a string",
            PrimitiveKind::Boolean => "not a boolean",
            PrimitiveKind::Symbol => "not a symbol",
            PrimitiveKind::BigInt => "not a BigInt",
        };
        Ok(NativeConversion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }

    fn call_primitive_prototype_to_string(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive toString did not receive a generic invocation",
            ));
        };
        let value = match self.primitive_this_value(realm, kind, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match (kind, value) {
            (PrimitiveKind::Boolean, Value::Bool(value)) => Ok(Completion::Return(Value::String(
                JsString::from(if value { "true" } else { "false" }),
            ))),
            _ => Err(RuntimeError::Invariant(
                "unimplemented primitive toString reached native dispatch",
            )),
        }
    }

    fn call_primitive_prototype_value_of(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "primitive valueOf did not receive a generic invocation",
            ));
        };
        match self.primitive_this_value(realm, kind, this_value)? {
            NativeConversion::Value(value) => Ok(Completion::Return(value)),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    fn object_to_string_tag(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        let default_tag = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            match &object_data.payload {
                ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => JsString::from("Function"),
                ObjectPayload::Error => JsString::from("Error"),
                ObjectPayload::Primitive(data) => JsString::from(match data.kind() {
                    PrimitiveKind::Number => "Number",
                    PrimitiveKind::String => "String",
                    PrimitiveKind::Boolean => "Boolean",
                    PrimitiveKind::Symbol => "Symbol",
                    PrimitiveKind::BigInt => "BigInt",
                }),
                ObjectPayload::Ordinary | ObjectPayload::GlobalObject { .. } => {
                    JsString::from("Object")
                }
            }
        };
        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        match self.get_property_in_realm(realm, object, &to_string_tag)? {
            Completion::Return(Value::String(tag)) => Ok(NativeConversion::Value(tag)),
            Completion::Return(_) => Ok(NativeConversion::Value(default_tag)),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn call_object_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.toString did not receive a generic invocation",
            ));
        };
        let tag = match this_value {
            Value::Undefined => JsString::from("Undefined"),
            Value::Null => JsString::from("Null"),
            Value::Bool(value) => {
                let prototype =
                    self.primitive_prototype_for_realm(realm, PrimitiveKind::Boolean)?;
                let object = self.new_primitive_object(
                    &prototype,
                    PrimitiveKind::Boolean,
                    Value::Bool(value),
                )?;
                match self.object_to_string_tag(realm, &object)? {
                    NativeConversion::Value(tag) => tag,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
            Value::Int(_) | Value::Float(_) => JsString::from("Number"),
            Value::BigInt(_) => JsString::from("BigInt"),
            Value::String(_) => JsString::from("String"),
            Value::Symbol(_) => JsString::from("Symbol"),
            Value::Object(object) => match self.object_to_string_tag(realm, &object)? {
                NativeConversion::Value(tag) => tag,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
        };
        let result = JsString::from("[object ")
            .concat(&tag)
            .concat(&JsString::from("]"));
        Ok(Completion::Return(Value::String(result)))
    }

    fn call_object_prototype_to_locale_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.toLocaleString did not receive a generic invocation",
            ));
        };
        if !matches!(this_value, Value::Object(_) | Value::Bool(_)) {
            return Err(RuntimeError::Engine(Error::internal(
                "primitive Object.prototype.toLocaleString is not implemented yet",
            )));
        }
        let to_string = self.intern_property_key("toString")?;
        let method =
            match self.get_value_property_in_realm(realm, this_value.clone(), &to_string)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let Value::Object(method) = method else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        let Some(method) = self.as_callable(&method)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        self.call_internal(realm, &method, this_value, &[])
    }

    fn call_object_prototype_value_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.valueOf did not receive a generic invocation",
            ));
        };
        match this_value {
            value @ Value::Object(_) => Ok(Completion::Return(value)),
            Value::Undefined | Value::Null => Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to object",
            )?)),
            value @ Value::Bool(_) => {
                let prototype =
                    self.primitive_prototype_for_realm(realm, PrimitiveKind::Boolean)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Boolean, value)?,
                )))
            }
            Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => Err(RuntimeError::Engine(Error::internal(
                "primitive wrapper objects are not implemented",
            ))),
        }
    }

    fn call_error_constructor(
        &self,
        realm: ContextId,
        kind: ErrorConstructorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if kind == ErrorConstructorKind::Native(NativeErrorKind::Aggregate) {
            return Err(RuntimeError::Engine(Error::internal(
                "AggregateError construction requires iterable and Array support",
            )));
        }
        let NativeInvocation::Construct { mut new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Error constructor did not receive constructor-or-function invocation",
            ));
        };
        if matches!(new_target, Value::Undefined) {
            new_target = Value::Object(self.active_function()?);
        }
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "Error constructor new.target was neither undefined nor an object",
            ));
        };
        let new_target_callable =
            self.callable_from_value(Value::Object(new_target_object.clone()))?;
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(realm, &new_target_object, &prototype_key)? {
                Completion::Return(Value::Object(prototype)) => prototype,
                Completion::Return(_) => {
                    let fallback_realm = self.callable_realm(&new_target_callable)?;
                    let prototype = {
                        let state = self.0.state.borrow();
                        let context = state.heap.context(fallback_realm)?;
                        match kind {
                            ErrorConstructorKind::Error => context
                                .error_prototype
                                .ok_or(RuntimeError::Invariant("realm has no Error prototype"))?,
                            ErrorConstructorKind::Native(kind) => {
                                context.native_error_prototypes[kind.index()].ok_or(
                                    RuntimeError::Invariant("realm has no native Error prototype"),
                                )?
                            }
                        }
                    };
                    ObjectRef::from_borrowed_handle(self.clone(), prototype)?
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        let object = self.new_error_object(&prototype)?;

        let message = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Error constructor readable argv was not padded to length one",
        ))?;
        if !matches!(message, Value::Undefined) {
            let message = match self.native_to_js_string(realm, message)? {
                NativeConversion::Value(message) => message,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.define_function_data_property(
                &object,
                "message",
                Value::String(message),
                true,
                true,
            )?;
        }

        if arguments.actual_arg_count > 1 {
            if let Some(Value::Object(options)) = arguments.readable.get(1) {
                let cause_key = self.intern_property_key("cause")?;
                if self.has_property(options, &cause_key)? {
                    let cause = match self.get_property_in_realm(realm, options, &cause_key)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    self.define_function_data_property(&object, "cause", cause, true, true)?;
                }
            }
        }

        let value = Value::Object(object);
        // `js_error_constructor` eagerly snapshots the stack after message,
        // cause, and AggregateError payload work, skipping only its own top
        // native frame.
        self.ensure_error_backtrace(&value, true, None)?;
        Ok(Completion::Return(value))
    }

    fn call_error_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Error.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let name_key = self.intern_property_key("name")?;
        let name = match self.get_property_in_realm(realm, &object, &name_key)? {
            Completion::Return(Value::Undefined) => JsString::from("Error"),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let message_key = self.intern_property_key("message")?;
        let message = match self.get_property_in_realm(realm, &object, &message_key)? {
            Completion::Return(Value::Undefined) => JsString::from(""),
            Completion::Return(value) => match self.native_to_js_string(realm, &value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            },
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = if name.is_empty() {
            message
        } else if message.is_empty() {
            name
        } else {
            name.concat(&JsString::from(": ")).concat(&message)
        };
        Ok(Completion::Return(Value::String(result)))
    }

    fn call_error_is_error(&self, arguments: &NativeArguments) -> Result<Completion, RuntimeError> {
        let value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Error.isError readable argv was not padded to length one",
        ))?;
        let is_error = match value {
            Value::Object(object) => self.is_error_object(object)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => false,
        };
        Ok(Completion::Return(Value::Bool(is_error)))
    }

    #[cfg(test)]
    fn call_active_frame_probe(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match arguments.readable.first() {
            Some(Value::Object(value))
                if matches!(arguments.readable.get(1), Some(Value::Bool(false))) =>
            {
                Ok(Completion::Throw(Value::Object(value.clone())))
            }
            Some(Value::Object(callback)) => {
                let callback = self.callable_from_value(Value::Object(callback.clone()))?;
                let active_function = self.active_function()?;
                self.call_internal(
                    realm,
                    &callback,
                    Value::Undefined,
                    &[Value::Object(active_function)],
                )
            }
            Some(Value::Bool(false)) => Ok(Completion::Throw(Value::String(JsString::from(
                "active frame probe throw",
            )))),
            Some(Value::Bool(true)) => {
                Err(RuntimeError::Invariant("active frame probe engine error"))
            }
            Some(_) => Err(RuntimeError::Invariant(
                "active frame probe received an unsupported command",
            )),
            None => {
                let snapshot = self.0.state.borrow().active_frames.clone();
                self.0
                    .state
                    .borrow_mut()
                    .active_frame_probe_snapshots
                    .push(snapshot);
                Ok(Completion::Return(Value::Undefined))
            }
        }
    }

    fn dispatch_native_function(
        &self,
        target: NativeFunctionId,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let frame =
            self.0
                .state
                .borrow()
                .active_frames
                .last()
                .copied()
                .ok_or(RuntimeError::Invariant(
                    "native handler ran without an active frame",
                ))?;
        let ActiveFrameKind::Native {
            target: frame_target,
            actual_arg_count,
            readable_arg_count,
        } = frame.kind
        else {
            return Err(RuntimeError::Invariant(
                "native handler was not the top active frame",
            ));
        };
        if frame.realm != realm
            || frame_target != target
            || actual_arg_count != arguments.actual_arg_count
            || readable_arg_count != arguments.readable.len()
        {
            return Err(RuntimeError::Invariant(
                "active native frame disagrees with handler arguments",
            ));
        }
        // Some handlers do not inspect their adapted this/new-target input,
        // but keeping it rooted for the full dispatch is part of the ABI.
        let invocation = match (target.descriptor().cproto, invocation) {
            (
                NativeCProto::Generic | NativeCProto::GenericMagic,
                NativeInvocation::Call { this_value },
            ) => NativeInvocation::Call { this_value },
            (
                NativeCProto::Generic | NativeCProto::GenericMagic,
                NativeInvocation::Construct { new_target },
            ) => {
                // QuickJS's generic ABI receives new.target in its `this`
                // slot when an embedding independently enables the
                // constructor bit on the native function object.
                NativeInvocation::Call {
                    this_value: new_target,
                }
            }
            (
                NativeCProto::Constructor | NativeCProto::ConstructorMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Construct { new_target },
            (
                NativeCProto::Constructor | NativeCProto::ConstructorMagic,
                NativeInvocation::Call { .. },
            ) => {
                let exception =
                    self.new_native_error(realm, NativeErrorKind::Type, "must be called with new")?;
                return Ok(Completion::Throw(exception));
            }
            (
                NativeCProto::ConstructorOrFunction | NativeCProto::ConstructorOrFunctionMagic,
                NativeInvocation::Call { .. },
            ) => NativeInvocation::Construct {
                new_target: Value::Undefined,
            },
            (
                NativeCProto::ConstructorOrFunction | NativeCProto::ConstructorOrFunctionMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Construct { new_target },
            (
                NativeCProto::Getter | NativeCProto::GetterMagic,
                NativeInvocation::Call { this_value },
            ) => NativeInvocation::Getter { this_value },
            (
                NativeCProto::Getter | NativeCProto::GetterMagic,
                NativeInvocation::Construct { new_target },
            ) => NativeInvocation::Getter {
                this_value: new_target,
            },
            (_, NativeInvocation::Getter { .. }) => {
                return Err(RuntimeError::Invariant(
                    "native invocation was adapted as a getter more than once",
                ));
            }
            (
                NativeCProto::UnaryF64
                | NativeCProto::BinaryF64
                | NativeCProto::Setter
                | NativeCProto::SetterMagic
                | NativeCProto::IteratorNext,
                _,
            ) => {
                return Err(RuntimeError::Invariant(
                    "native cproto adapter is not implemented yet",
                ));
            }
        };
        let _ = &invocation;
        match target {
            NativeFunctionId::FunctionPrototype => Ok(Completion::Return(Value::Undefined)),
            NativeFunctionId::FunctionConstructor(kind) => {
                self.call_function_constructor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ThrowTypeError => {
                self.call_throw_type_error(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeCall => {
                self.call_function_prototype_call(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeApply => {
                self.call_function_prototype_apply(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeBind => {
                self.call_function_prototype_bind(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeToString => {
                self.call_function_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::FunctionPrototypeHasInstance => {
                self.call_function_prototype_has_instance(realm, invocation, arguments)
            }
            NativeFunctionId::FunctionPrototypeFileName => {
                self.call_function_prototype_file_name(invocation)
            }
            NativeFunctionId::FunctionPrototypePosition(selector) => {
                self.call_function_prototype_position(invocation, selector)
            }
            NativeFunctionId::ObjectPrototypeToString => {
                self.call_object_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::ObjectPrototypeToLocaleString => {
                self.call_object_prototype_to_locale_string(realm, invocation)
            }
            NativeFunctionId::ObjectPrototypeValueOf => {
                self.call_object_prototype_value_of(realm, invocation)
            }
            NativeFunctionId::PrimitiveConstructor(kind) => {
                self.call_primitive_constructor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::PrimitivePrototypeToString(kind) => {
                self.call_primitive_prototype_to_string(realm, kind, invocation)
            }
            NativeFunctionId::PrimitivePrototypeValueOf(kind) => {
                self.call_primitive_prototype_value_of(realm, kind, invocation)
            }
            NativeFunctionId::GlobalNumberParse(kind) => {
                self.call_global_number_parse(realm, kind, invocation, arguments)
            }
            NativeFunctionId::NumberPredicate(_) | NativeFunctionId::NumberPrototypeFormat(_) => {
                Err(RuntimeError::Invariant(
                    "unpublished Number native reached dispatch",
                ))
            }
            NativeFunctionId::ErrorConstructor(kind) => {
                self.call_error_constructor(realm, kind, invocation, arguments)
            }
            NativeFunctionId::ErrorPrototypeToString => {
                self.call_error_prototype_to_string(realm, invocation)
            }
            NativeFunctionId::ErrorIsError => self.call_error_is_error(arguments),
            #[cfg(test)]
            NativeFunctionId::ActiveFrameProbe => self.call_active_frame_probe(realm, arguments),
            #[cfg(test)]
            NativeFunctionId::ArgumentProbe
            | NativeFunctionId::ConstructorProbe
            | NativeFunctionId::ConstructorOrFunctionProbe => {
                if matches!(arguments.readable.first(), Some(Value::Bool(false))) {
                    return Ok(Completion::Throw(Value::String(JsString::from(
                        "native probe throw",
                    ))));
                }
                if matches!(arguments.readable.first(), Some(Value::Bool(true))) {
                    return Err(RuntimeError::Invariant("native probe engine error"));
                }
                let padded_undefined = arguments.readable[arguments.actual_arg_count..]
                    .iter()
                    .filter(|value| matches!(value, Value::Undefined))
                    .count();
                let invocation_target_is_function = match invocation {
                    NativeInvocation::Call {
                        this_value: Value::Object(object),
                    } => object.object_id() == frame.function,
                    NativeInvocation::Construct {
                        new_target: Value::Object(object),
                    } => object.object_id() == frame.function,
                    NativeInvocation::Getter {
                        this_value: Value::Object(object),
                    } => object.object_id() == frame.function,
                    NativeInvocation::Call { .. }
                    | NativeInvocation::Construct { .. }
                    | NativeInvocation::Getter { .. } => false,
                };
                let result = format!(
                    "{}|{}|{}|{}",
                    arguments.actual_arg_count,
                    arguments.readable.len(),
                    padded_undefined,
                    invocation_target_is_function
                );
                Ok(Completion::Return(Value::String(JsString::from(
                    result.as_str(),
                ))))
            }
        }
    }

    fn validate_object_and_key(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        Ok(())
    }

    fn validate_descriptor_domains(
        &self,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        if let DescriptorField::Present(value) = &descriptor.value {
            match value {
                Value::Object(object) if !object.belongs_to(self) => {
                    return Err(RuntimeError::WrongRuntime("descriptor value"));
                }
                Value::Symbol(symbol) if !symbol.belongs_to(self) => {
                    return Err(RuntimeError::WrongRuntime("descriptor value"));
                }
                _ => {}
            }
        }
        for accessor in [&descriptor.get, &descriptor.set] {
            if let DescriptorField::Present(AccessorValue::Callable(callable)) = accessor {
                if !callable.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("descriptor accessor"));
                }
            }
        }
        Ok(())
    }

    fn validate_value_domain(&self, value: &Value, role: &'static str) -> Result<(), RuntimeError> {
        match value {
            Value::Object(object) if !object.belongs_to(self) => {
                Err(RuntimeError::WrongRuntime(role))
            }
            Value::Symbol(symbol) if !symbol.belongs_to(self) => {
                Err(RuntimeError::WrongRuntime(role))
            }
            _ => Ok(()),
        }
    }

    fn root_raw_value(&self, value: &RawValue) -> Result<Value, RuntimeError> {
        Ok(match value {
            RawValue::Undefined => Value::Undefined,
            RawValue::Null => Value::Null,
            RawValue::Bool(value) => Value::Bool(*value),
            RawValue::Int(value) => Value::Int(*value),
            RawValue::Float(value) => Value::Float(*value),
            RawValue::BigInt(value) => Value::BigInt(value.clone()),
            RawValue::String(value) => Value::String(value.clone()),
            RawValue::Symbol(atom) => {
                Value::Symbol(SymbolRef::from_borrowed_atom(self.clone(), *atom)?)
            }
            RawValue::Object(object) => {
                Value::Object(ObjectRef::from_borrowed_handle(self.clone(), *object)?)
            }
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel escaped from an object property",
                ));
            }
        })
    }

    fn raw_property_value(&self, value: &Value) -> Result<RawValue, RuntimeError> {
        Ok(match value {
            Value::Undefined => RawValue::Undefined,
            Value::Null => RawValue::Null,
            Value::Bool(value) => RawValue::Bool(*value),
            Value::Int(value) => RawValue::Int(*value),
            Value::Float(value) => RawValue::Float(*value),
            Value::BigInt(value) => RawValue::BigInt(value.clone()),
            Value::String(value) => RawValue::String(value.clone()),
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("property value"));
                }
                RawValue::Symbol(symbol.atom())
            }
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("property value"));
                }
                RawValue::Object(object.object_id())
            }
        })
    }

    fn store_complete_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        complete: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        let global_hidden = {
            let state = self.0.state.borrow();
            match state.heap.object(object.object_id())?.payload {
                ObjectPayload::GlobalObject { uninitialized_vars } => Some(uninitialized_vars),
                ObjectPayload::Ordinary
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Error
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => None,
            }
        };
        if let Some(hidden) = global_hidden {
            return self.store_complete_global_property(object, hidden, key, complete);
        }

        let (flags, replacement) = match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => (
                PropertyFlags::data(writable, enumerable, configurable),
                PropertySlot::Data(self.raw_property_value(&value)?),
            ),
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => (
                PropertyFlags::accessor(enumerable, configurable),
                PropertySlot::Accessor {
                    get: get.as_ref().map(|value| value.as_object().object_id()),
                    set: set.as_ref().map(|value| value.as_object().object_id()),
                },
            ),
        };
        self.store_property_slot(object, key, flags, replacement)
    }

    fn store_complete_global_property(
        &self,
        object: &ObjectRef,
        hidden_id: ObjectId,
        key: &PropertyKey,
        complete: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<(), RuntimeError> {
        let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden_id)?;
        match complete {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => {
                let global_root = self.own_var_ref_root(object, key)?;
                let hidden_root = if global_root.is_none() {
                    self.own_var_ref_root(&hidden, key)?
                } else {
                    None
                };
                let root = if let Some(root) = global_root {
                    self.write_var_ref(&root, value)?;
                    root
                } else if let Some(root) = hidden_root {
                    if !self.delete_property(&hidden, key)? {
                        return Err(RuntimeError::Invariant(
                            "hidden global VarRef property was not configurable",
                        ));
                    }
                    self.write_var_ref(&root, value)?;
                    root
                } else {
                    self.new_var_ref(value, false, !writable, ClosureVariableKind::Normal)?
                };
                self.set_var_ref_metadata(&root, false, !writable, ClosureVariableKind::Normal)?;
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::data(writable, enumerable, configurable),
                    PropertySlot::VarRef(root.id()),
                )
            }
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                if let Some(root) = self.own_var_ref_root(object, key)? {
                    let shared = self.0.state.borrow().heap.var_ref_strong_count(root.id())? > 2;
                    if shared {
                        if self.own_var_ref_root(&hidden, key)?.is_some() {
                            return Err(RuntimeError::Invariant(
                                "global property and hidden table contain distinct VarRefs",
                            ));
                        }
                        self.reset_var_ref_uninitialized(&root)?;
                        self.set_var_ref_metadata(
                            &root,
                            false,
                            false,
                            ClosureVariableKind::Normal,
                        )?;
                        self.store_property_slot(
                            &hidden,
                            key,
                            PropertyFlags::data(true, true, true),
                            PropertySlot::VarRef(root.id()),
                        )?;
                    }
                }
                self.store_property_slot(
                    object,
                    key,
                    PropertyFlags::accessor(enumerable, configurable),
                    PropertySlot::Accessor {
                        get: get.as_ref().map(|value| value.as_object().object_id()),
                        set: set.as_ref().map(|value| value.as_object().object_id()),
                    },
                )
            }
        }
    }

    fn own_var_ref_root(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<VarRefRoot>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let id = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            let shape = state.heap.shape(object.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(None);
            };
            match object.slots.get(index as usize) {
                Some(PropertySlot::VarRef(id)) => Some(*id),
                Some(
                    PropertySlot::Data(_)
                    | PropertySlot::Accessor { .. }
                    | PropertySlot::AutoInit(_),
                ) => None,
                None => {
                    return Err(RuntimeError::Invariant(
                        "shape property has no parallel object slot",
                    ));
                }
            }
        };
        id.map(|id| VarRefRoot::from_borrowed_handle(self.clone(), id).map_err(Into::into))
            .transpose()
    }

    fn store_property_slot(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        flags: PropertyFlags,
        replacement: PropertySlot,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let object_id = object.object_id();
        let (prototype, mut entries, mut slots, existing) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object_data.slots.clone(),
                shape.find(key.atom()).map(|index| index as usize),
            )
        };

        if let Some(index) = existing {
            let entry = entries.get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            if entry.flags == flags {
                let retained_atoms = state.retain_slot_atoms(std::slice::from_ref(&replacement))?;
                match state
                    .heap
                    .replace_object_slot(object_id, index, replacement)
                {
                    Ok(cleanup) => return state.apply_cleanup(cleanup),
                    Err(error) => {
                        state.release_atoms(retained_atoms)?;
                        return Err(error.into());
                    }
                }
            }
            entries[index].flags = flags;
            slots[index] = replacement;
        } else {
            entries.push(ShapeEntry {
                atom: key.atom(),
                flags,
            });
            slots.push(replacement);
        }
        state.replace_layout(object_id, prototype, &entries, slots)
    }

    fn operation(&self) -> RuntimeOperation<'_> {
        let result = self.drain_deferred_references();
        debug_assert!(result.is_ok(), "deferred root release failed: {result:?}");
        RuntimeOperation(self)
    }

    fn drain_deferred_references(&self) -> Result<(), RuntimeError> {
        loop {
            let operation = self.0.deferred_references.borrow_mut().pop_front();
            let Some(operation) = operation else {
                return Ok(());
            };
            let Ok(mut state) = self.0.state.try_borrow_mut() else {
                self.0
                    .deferred_references
                    .borrow_mut()
                    .push_front(operation);
                return Ok(());
            };
            match operation {
                DeferredRefOp::Object(object) => {
                    let cleanup = state.heap.release_object(object)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::Context(context) => {
                    let cleanup = state.heap.release_context(context)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::FunctionBytecode(bytecode) => {
                    let cleanup = state.heap.release_function_bytecode(bytecode)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::VarRef(var_ref) => {
                    let cleanup = state.heap.release_var_ref(var_ref)?;
                    state.apply_cleanup(cleanup)?;
                }
                DeferredRefOp::Atom(atom) => {
                    state.atoms.release(atom)?;
                }
                DeferredRefOp::ActiveFramePop { token, depth } => {
                    if let Some(position) = state
                        .active_frames
                        .iter()
                        .rposition(|frame| frame.token == token)
                    {
                        state.active_frames.truncate(position);
                    } else if state.active_frames.len() > depth {
                        state.active_frames.truncate(depth);
                    }
                }
                DeferredRefOp::BacktraceBarrierRestore { token, previous } => {
                    if let Some(frame) = state
                        .active_frames
                        .iter_mut()
                        .find(|frame| frame.token == token)
                    {
                        frame.flags.backtrace_barrier = previous;
                    }
                }
            }
        }
    }

    pub(crate) fn retain_object_handle(&self, id: ObjectId) -> Result<(), HeapError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            HeapError::Invariant("object root retained during a runtime state borrow")
        })?;
        state.heap.retain_object(id)
    }

    pub(crate) fn release_object_handle(&self, id: ObjectId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_object(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Object(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid object root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred object release failed: {drain:?}");
    }

    pub(crate) fn retain_atom_handle(&self, atom: Atom) -> Result<(), AtomError> {
        self.0.state.borrow_mut().atoms.retain(atom).map(drop)
    }

    pub(crate) fn retain_function_bytecode_handle(
        &self,
        id: FunctionBytecodeId,
    ) -> Result<(), HeapError> {
        let mut state = self.0.state.try_borrow_mut().map_err(|_| {
            HeapError::Invariant("function bytecode retained during a runtime state borrow")
        })?;
        state.heap.retain_function_bytecode(id)
    }

    fn retain_context_handle(&self, id: ContextId) -> Result<(), HeapError> {
        let mut state =
            self.0.state.try_borrow_mut().map_err(|_| {
                HeapError::Invariant("context retained during a runtime state borrow")
            })?;
        state.heap.retain_context(id)
    }

    fn release_context_handle(&self, id: ContextId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_context(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Context(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid context root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred context release failed: {drain:?}");
    }

    pub(crate) fn release_function_bytecode_handle(&self, id: FunctionBytecodeId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state
                .heap
                .release_function_bytecode(id)
                .map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::FunctionBytecode(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid bytecode root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred bytecode release failed: {drain:?}");
    }

    fn retain_var_ref_handle(&self, id: VarRefId) -> Result<(), HeapError> {
        let mut state =
            self.0.state.try_borrow_mut().map_err(|_| {
                HeapError::Invariant("VarRef retained during a runtime state borrow")
            })?;
        state.heap.retain_var_ref(id)
    }

    fn release_var_ref_handle(&self, id: VarRefId) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            let result = state.heap.release_var_ref(id).map_err(RuntimeError::Heap);
            result.and_then(|cleanup| state.apply_cleanup(cleanup))
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::VarRef(id));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid VarRef root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred VarRef release failed: {drain:?}");
    }

    pub(crate) fn release_atom_handle(&self, atom: Atom) {
        let result = if let Ok(mut state) = self.0.state.try_borrow_mut() {
            state.atoms.release(atom).map(drop)
        } else {
            self.0
                .deferred_references
                .borrow_mut()
                .push_back(DeferredRefOp::Atom(atom));
            Ok(())
        };
        debug_assert!(result.is_ok(), "invalid atom root release: {result:?}");
        let drain = self.drain_deferred_references();
        debug_assert!(drain.is_ok(), "deferred atom release failed: {drain:?}");
    }
}

fn bound_function_length(
    value: &Value,
    bound_argument_count: usize,
) -> Result<Value, RuntimeError> {
    let count = u32::try_from(bound_argument_count)
        .map_err(|_| RuntimeError::Invariant("bound argument count does not fit u32"))?;
    Ok(match value {
        Value::Int(length) => {
            let length = i64::from(*length);
            let count = i64::from(count);
            if length <= count {
                Value::Int(0)
            } else {
                Value::Int(i32::try_from(length - count).map_err(|_| {
                    RuntimeError::Invariant("bound function integer length does not fit i32")
                })?)
            }
        }
        Value::Float(length) => {
            let length = if length.is_nan() {
                0.0
            } else {
                let length = length.trunc();
                if length <= f64::from(count) {
                    0.0
                } else {
                    length - f64::from(count)
                }
            };
            Value::number(length)
        }
        Value::Undefined
        | Value::Null
        | Value::Bool(_)
        | Value::BigInt(_)
        | Value::String(_)
        | Value::Symbol(_)
        | Value::Object(_) => Value::Int(0),
    })
}

fn append_backtrace_ascii(output: &mut Vec<u16>, value: &str) {
    output.extend(value.encode_utf16());
}

fn append_backtrace_string(output: &mut Vec<u16>, value: &JsString) {
    output.extend(value.utf16_units());
}

fn truncate_backtrace_c_string(value: JsString) -> JsString {
    let prefix = value.utf16_units().take_while(|unit| *unit != 0);
    JsString::from_utf16(prefix)
}

fn raw_string_property_on_object(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<RawStringProperty, RuntimeError> {
    let object = state.heap.object(object)?;
    let shape = state.heap.shape(object.shape)?;
    let Some(index) = shape.find(atom) else {
        return Ok(RawStringProperty::Missing);
    };
    let slot = object
        .slots
        .get(index as usize)
        .ok_or(RuntimeError::Invariant(
            "backtrace name shape has no parallel property slot",
        ))?;
    Ok(match slot {
        PropertySlot::Data(RawValue::String(value)) => RawStringProperty::String(value.clone()),
        PropertySlot::Data(_)
        | PropertySlot::VarRef(_)
        | PropertySlot::Accessor { .. }
        | PropertySlot::AutoInit(_) => RawStringProperty::Other,
    })
}

fn raw_string_property_one_level(
    state: &RuntimeState,
    object: ObjectId,
    atom: Atom,
) -> Result<Option<JsString>, RuntimeError> {
    match raw_string_property_on_object(state, object, atom)? {
        RawStringProperty::String(name) => return Ok(Some(name)),
        RawStringProperty::Other => return Ok(None),
        RawStringProperty::Missing => {}
    }

    let object = state.heap.object(object)?;
    let Some(prototype) = state.heap.shape(object.shape)?.prototype() else {
        return Ok(None);
    };
    Ok(
        match raw_string_property_on_object(state, prototype, atom)? {
            RawStringProperty::String(name) => Some(name),
            RawStringProperty::Missing | RawStringProperty::Other => None,
        },
    )
}

impl RuntimeState {
    fn retain_raw_root(&mut self, value: &RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Object(object) => self.heap.retain_object(*object)?,
            RawValue::Symbol(atom) => {
                self.atoms.retain(*atom)?;
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_)
            | RawValue::BigInt(_)
            | RawValue::String(_) => {}
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel cannot become a runtime root",
                ));
            }
        }
        Ok(())
    }

    fn release_owned_raw_root(&mut self, value: RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Object(object) => {
                let cleanup = self.heap.release_object(object)?;
                self.apply_cleanup(cleanup)?;
            }
            RawValue::Symbol(atom) => {
                self.atoms.release(atom)?;
            }
            RawValue::Undefined
            | RawValue::Null
            | RawValue::Bool(_)
            | RawValue::Int(_)
            | RawValue::Float(_)
            | RawValue::BigInt(_)
            | RawValue::String(_) => {}
            RawValue::Uninitialized | RawValue::Exception => {
                return Err(RuntimeError::Invariant(
                    "internal value sentinel occupied a runtime root",
                ));
            }
        }
        Ok(())
    }

    fn get_or_create_shape(
        &mut self,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
    ) -> Result<ShapeId, RuntimeError> {
        let fingerprint = ShapeFingerprint {
            prototype,
            entries: entries.into(),
        };
        if let Some(&shape) = self.shape_cache.get(&fingerprint) {
            if self.heap.shape(shape).is_ok() {
                self.heap.retain_shape(shape)?;
                return Ok(shape);
            }
            self.shape_cache.remove(&fingerprint);
            self.shape_fingerprints.remove(&shape);
        }

        let mut retained_atoms = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Err(error) = self.atoms.resolve(entry.atom) {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            if let Err(error) = self.atoms.retain(entry.atom) {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            retained_atoms.push(entry.atom);
        }

        let shape = match Shape::new(prototype, entries.iter().copied()) {
            Ok(shape) => shape,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        let shape = match self.heap.allocate_shape(shape) {
            Ok(shape) => shape,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        self.shape_cache.insert(fingerprint.clone(), shape);
        self.shape_fingerprints.insert(shape, fingerprint);
        Ok(shape)
    }

    fn retain_slot_atoms(&mut self, slots: &[PropertySlot]) -> Result<Vec<Atom>, RuntimeError> {
        let atoms = slots
            .iter()
            .filter_map(|slot| match slot {
                PropertySlot::Data(RawValue::Symbol(atom)) => Some(*atom),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut retained = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = self.atoms.retain(atom) {
                self.release_atoms(retained)?;
                return Err(error.into());
            }
            retained.push(atom);
        }
        Ok(retained)
    }

    fn retain_raw_value_atoms<'a>(
        &mut self,
        values: impl IntoIterator<Item = &'a RawValue>,
    ) -> Result<Vec<Atom>, RuntimeError> {
        let atoms = values
            .into_iter()
            .filter_map(|value| match value {
                RawValue::Symbol(atom) => Some(*atom),
                _ => None,
            })
            .collect::<Vec<_>>();
        let mut retained = Vec::with_capacity(atoms.len());
        for atom in atoms {
            if let Err(error) = self.atoms.retain(atom) {
                self.release_atoms(retained)?;
                return Err(error.into());
            }
            retained.push(atom);
        }
        Ok(retained)
    }

    fn replace_layout(
        &mut self,
        object: ObjectId,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
        slots: Vec<PropertySlot>,
    ) -> Result<(), RuntimeError> {
        let shape = self.get_or_create_shape(prototype, entries)?;
        let retained_atoms = match self.retain_slot_atoms(&slots) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };

        let layout_cleanup = match self.heap.replace_object_layout(object, shape, slots) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                self.release_atoms(retained_atoms)?;
                let cleanup = self.heap.release_shape(shape)?;
                self.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let shape_cleanup = self.heap.release_shape(shape)?;
        self.apply_cleanup(layout_cleanup)?;
        self.apply_cleanup(shape_cleanup)
    }

    fn apply_cleanup(&mut self, cleanup: HeapCleanup) -> Result<(), RuntimeError> {
        self.unlink_finalized_shapes(cleanup.finalized_shape_ids);
        self.release_atoms(cleanup.atoms)
    }

    fn unlink_finalized_shapes(&mut self, shapes: impl IntoIterator<Item = ShapeId>) {
        for shape in shapes {
            let Some(fingerprint) = self.shape_fingerprints.remove(&shape) else {
                continue;
            };
            if self.shape_cache.get(&fingerprint) == Some(&shape) {
                self.shape_cache.remove(&fingerprint);
            }
        }
    }

    fn release_atoms(&mut self, atoms: impl IntoIterator<Item = Atom>) -> Result<(), RuntimeError> {
        for atom in atoms {
            self.atoms.release(atom)?;
        }
        Ok(())
    }
}

impl Drop for RuntimeInner {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        let deferred = self.deferred_references.get_mut();
        while let Some(operation) = deferred.pop_front() {
            let result = match operation {
                DeferredRefOp::Object(object) => state
                    .heap
                    .release_object(object)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::Context(context) => state
                    .heap
                    .release_context(context)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::FunctionBytecode(bytecode) => state
                    .heap
                    .release_function_bytecode(bytecode)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::VarRef(var_ref) => state
                    .heap
                    .release_var_ref(var_ref)
                    .map_err(RuntimeError::Heap)
                    .and_then(|cleanup| state.apply_cleanup(cleanup)),
                DeferredRefOp::Atom(atom) => {
                    state.atoms.release(atom).map(drop).map_err(Into::into)
                }
                DeferredRefOp::ActiveFramePop { token, depth } => {
                    if let Some(position) = state
                        .active_frames
                        .iter()
                        .rposition(|frame| frame.token == token)
                    {
                        state.active_frames.truncate(position);
                    } else if state.active_frames.len() > depth {
                        state.active_frames.truncate(depth);
                    }
                    Ok(())
                }
                DeferredRefOp::BacktraceBarrierRestore { token, previous } => {
                    if let Some(frame) = state
                        .active_frames
                        .iter_mut()
                        .find(|frame| frame.token == token)
                    {
                        frame.flags.backtrace_barrier = previous;
                    }
                    Ok(())
                }
            };
            debug_assert!(
                result.is_ok(),
                "runtime deferred teardown failed: {result:?}"
            );
        }
        if let Some(exception) = state.pending_exception.take() {
            let result = state.release_owned_raw_root(exception);
            debug_assert!(
                result.is_ok(),
                "pending exception teardown failed: {result:?}"
            );
        }
        let result = state
            .heap
            .run_gc()
            .map_err(RuntimeError::Heap)
            .and_then(|mut stats| {
                let atoms = std::mem::take(&mut stats.cleanup.atoms);
                state.release_atoms(atoms)
            });
        debug_assert!(result.is_ok(), "runtime teardown failed: {result:?}");
        debug_assert_eq!(
            state.heap.counts().live,
            0,
            "runtime teardown left live heap nodes"
        );
    }
}

fn unlinked_closure_name<'a>(
    function: &'a UnlinkedFunction,
    descriptor: &ClosureVariable,
) -> Result<Option<&'a JsString>, RuntimeError> {
    match descriptor.name {
        ClosureVariableName::None => Ok(None),
        ClosureVariableName::Constant(index) => {
            let name = usize::try_from(index)
                .ok()
                .and_then(|index| function.constants().get(index))
                .and_then(UnlinkedConstant::as_primitive);
            let Some(Value::String(name)) = name else {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor referenced a non-string name constant",
                )));
            };
            Ok(Some(name))
        }
        ClosureVariableName::Atom(_) => Err(RuntimeError::Engine(Error::internal(
            "unlinked closure descriptor already contained a runtime atom",
        ))),
    }
}

fn verify_unlinked_tree(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    let mut pending = vec![(function, true)];
    while let Some((function, is_root)) = pending.pop() {
        if function.metadata().defined_argument_count > function.metadata().argument_count {
            return Err(RuntimeError::Engine(Error::internal(
                "defined argument count exceeds function argument slots",
            )));
        }
        if function
            .metadata()
            .function_name_local
            .is_some_and(|index| index >= function.metadata().local_count)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "function-name local is outside bytecode local slots",
            )));
        }
        if function.metadata().function_name_local.is_some()
            && function.func_name().is_none_or(JsString::is_empty)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "function-name local requires a non-empty intrinsic function name",
            )));
        }
        if function.closure_variables().len() != usize::from(function.metadata().closure_count) {
            return Err(RuntimeError::Engine(Error::internal(
                "function closure descriptor count does not match bytecode metadata",
            )));
        }
        verify_unlinked_debug(function)?;
        if function.closure_variables().iter().any(|descriptor| {
            descriptor.is_const
                && !descriptor.is_lexical
                && descriptor.kind != ClosureVariableKind::FunctionName
        }) {
            return Err(RuntimeError::Engine(Error::internal(
                "a const closure descriptor must also be lexical",
            )));
        }
        for descriptor in function.closure_variables() {
            let requires_name = descriptor.kind == ClosureVariableKind::FunctionName
                || matches!(
                    descriptor.source,
                    ClosureSource::Global | ClosureSource::ParentGlobal(_)
                );
            let name = unlinked_closure_name(function, descriptor)?;
            if requires_name != name.is_some() {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor name does not match its binding kind",
                )));
            }
            match descriptor.source {
                ClosureSource::Global if !is_root => {
                    return Err(RuntimeError::Engine(Error::internal(
                        "only root bytecode may resolve a global closure binding",
                    )));
                }
                ClosureSource::ParentGlobal(_) if is_root => {
                    return Err(RuntimeError::Engine(Error::internal(
                        "root bytecode cannot relay a parent global closure binding",
                    )));
                }
                _ => {}
            }
            if matches!(
                descriptor.source,
                ClosureSource::Global | ClosureSource::ParentGlobal(_)
            ) && descriptor.kind != ClosureVariableKind::Normal
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "global closure descriptor has a non-global binding kind",
                )));
            }
        }
        verify_parts(
            function.code(),
            function.constants().len(),
            function.metadata().max_stack,
        )?;

        for instruction in function.code() {
            match instruction {
                crate::bytecode::Instruction::PushConst(index) => {
                    let index = usize::try_from(*index)
                        .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                    let constant = function.constants().get(index).ok_or_else(|| {
                        RuntimeError::Engine(Error::internal("constant index is out of bounds"))
                    })?;
                    if constant.as_child().is_some() {
                        return Err(RuntimeError::Engine(Error::internal(
                            "value-constant opcode referenced child function bytecode",
                        )));
                    }
                }
                crate::bytecode::Instruction::FClosure(index) => {
                    let index = usize::try_from(*index)
                        .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                    let constant = function.constants().get(index).ok_or_else(|| {
                        RuntimeError::Engine(Error::internal("constant index is out of bounds"))
                    })?;
                    if constant.as_child().is_none() {
                        return Err(RuntimeError::Engine(Error::internal(
                            "function-closure opcode referenced a value constant",
                        )));
                    }
                }
                crate::bytecode::Instruction::SetName(index)
                | crate::bytecode::Instruction::ThrowReadOnly(index)
                | crate::bytecode::Instruction::GetField(index)
                | crate::bytecode::Instruction::GetField2(index)
                | crate::bytecode::Instruction::PutField(index) => {
                    let index = usize::try_from(*index)
                        .map_err(|_| RuntimeError::Invariant("constant index did not fit usize"))?;
                    let constant = function.constants().get(index).ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "string-key constant index is out of bounds",
                        ))
                    })?;
                    if !matches!(constant.as_primitive(), Some(Value::String(_))) {
                        return Err(RuntimeError::Engine(Error::internal(
                            "string-key opcode referenced a non-string constant",
                        )));
                    }
                }
                crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
                    if function.metadata().function_name_local == Some(*index) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "bytecode directly writes its private function-name local",
                    )));
                }
                crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| {
                            descriptor.kind == ClosureVariableKind::FunctionName
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "bytecode directly writes a private function-name closure",
                    )));
                }
                crate::bytecode::Instruction::GetVarRef(index)
                | crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| {
                            matches!(
                                descriptor.source,
                                ClosureSource::Global | ClosureSource::ParentGlobal(_)
                            )
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "lexical closure opcode referenced a global closure descriptor",
                    )));
                }
                crate::bytecode::Instruction::GetVar(index)
                | crate::bytecode::Instruction::GetVarUndef(index)
                | crate::bytecode::Instruction::DeleteVar(index)
                | crate::bytecode::Instruction::PutVar(index)
                | crate::bytecode::Instruction::PutVarInit(index)
                    if function
                        .closure_variables()
                        .get(usize::from(*index))
                        .is_some_and(|descriptor| {
                            !matches!(
                                descriptor.source,
                                ClosureSource::Global | ClosureSource::ParentGlobal(_)
                            ) || !matches!(descriptor.name, ClosureVariableName::Constant(_))
                        }) =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "global closure opcode referenced a non-global closure descriptor",
                    )));
                }
                crate::bytecode::Instruction::GetLocal(index)
                | crate::bytecode::Instruction::PutLocal(index)
                | crate::bytecode::Instruction::SetLocal(index)
                    if *index >= function.metadata().local_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "local bytecode operand is out of bounds",
                    )));
                }
                crate::bytecode::Instruction::GetArg(index)
                | crate::bytecode::Instruction::PutArg(index)
                | crate::bytecode::Instruction::SetArg(index)
                    if *index >= function.metadata().argument_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "argument bytecode operand is out of bounds",
                    )));
                }
                crate::bytecode::Instruction::GetVarRef(index)
                | crate::bytecode::Instruction::PutVarRef(index)
                | crate::bytecode::Instruction::SetVarRef(index)
                | crate::bytecode::Instruction::GetVar(index)
                | crate::bytecode::Instruction::GetVarUndef(index)
                | crate::bytecode::Instruction::DeleteVar(index)
                | crate::bytecode::Instruction::PutVar(index)
                | crate::bytecode::Instruction::PutVarInit(index)
                    if *index >= function.metadata().closure_count =>
                {
                    return Err(RuntimeError::Engine(Error::internal(
                        "closure variable bytecode operand is out of bounds",
                    )));
                }
                _ => {}
            }
        }
        let mut local_flags = vec![None; usize::from(function.metadata().local_count)];
        let mut argument_flags = vec![None; usize::from(function.metadata().argument_count)];
        for constant in function.constants() {
            if let Some(child) = constant.as_child() {
                for descriptor in child.closure_variables() {
                    let flags = (descriptor.is_lexical, descriptor.is_const, descriptor.kind);
                    match descriptor.source {
                        ClosureSource::ParentLocal(index) => {
                            let slot =
                                local_flags.get_mut(usize::from(index)).ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent bounds",
                                    ))
                                })?;
                            let is_function_name =
                                function.metadata().function_name_local == Some(index);
                            if is_function_name {
                                let expected = (
                                    false,
                                    function.metadata().strict,
                                    ClosureVariableKind::FunctionName,
                                );
                                if flags != expected {
                                    return Err(RuntimeError::Engine(Error::internal(
                                        "function-name closure descriptor disagrees with parent metadata",
                                    )));
                                }
                                if unlinked_closure_name(child, descriptor)? != function.func_name()
                                {
                                    return Err(RuntimeError::Engine(Error::internal(
                                        "function-name closure descriptor disagrees with the parent name",
                                    )));
                                }
                            } else if descriptor.kind == ClosureVariableKind::FunctionName {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "function-name closure descriptor points at an ordinary parent local",
                                )));
                            }
                            verify_capture_flags(slot, flags)?;
                        }
                        ClosureSource::ParentArgument(index) => {
                            let slot =
                                argument_flags.get_mut(usize::from(index)).ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent bounds",
                                    ))
                                })?;
                            if descriptor.kind == ClosureVariableKind::FunctionName {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "function-name closure descriptor points at a parent argument",
                                )));
                            }
                            verify_capture_flags(slot, flags)?;
                        }
                        ClosureSource::ParentClosure(index) => {
                            let parent = function
                                .closure_variables()
                                .get(usize::from(index))
                                .ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child closure descriptor source is out of parent bounds",
                                    ))
                                })?;
                            if (parent.is_lexical, parent.is_const, parent.kind) != flags {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "transitive closure descriptor flags do not match the parent slot",
                                )));
                            }
                            if descriptor.kind == ClosureVariableKind::FunctionName
                                && unlinked_closure_name(child, descriptor)?
                                    != unlinked_closure_name(function, parent)?
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "transitive function-name relay changed its binding name",
                                )));
                            }
                        }
                        ClosureSource::Global => {
                            if descriptor.kind != ClosureVariableKind::Normal {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "global closure descriptor has a non-global binding kind",
                                )));
                            }
                        }
                        ClosureSource::ParentGlobal(index) => {
                            let parent = function
                                .closure_variables()
                                .get(usize::from(index))
                                .ok_or_else(|| {
                                    RuntimeError::Engine(Error::internal(
                                        "child global relay source is out of parent bounds",
                                    ))
                                })?;
                            if !matches!(
                                parent.source,
                                ClosureSource::Global | ClosureSource::ParentGlobal(_)
                            ) || (parent.is_lexical, parent.is_const, parent.kind) != flags
                                || descriptor.kind != ClosureVariableKind::Normal
                                || unlinked_closure_name(child, descriptor)?
                                    != unlinked_closure_name(function, parent)?
                            {
                                return Err(RuntimeError::Engine(Error::internal(
                                    "parent global relay descriptor disagrees with the parent slot",
                                )));
                            }
                        }
                    }
                }
                pending.push((child, false));
            } else if constant.as_primitive().is_none() {
                return Err(RuntimeError::Invariant(
                    "unlinked constant did not contain exactly one payload",
                ));
            }
        }
    }
    Ok(())
}

fn verify_unlinked_debug(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    let Some(debug) = function.debug() else {
        return Ok(());
    };
    if debug
        .source
        .as_deref()
        .is_some_and(|source| std::str::from_utf8(source).is_err())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode debug source is not valid UTF-8",
        )));
    }
    let Some(table) = &debug.pc2line else {
        return Ok(());
    };
    if table.definition.line == u32::MAX || table.definition.column == u32::MAX {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode debug definition position cannot be represented one-based",
        )));
    }
    let mut previous_pc = None;
    for entry in &table.entries {
        if usize::try_from(entry.pc)
            .ok()
            .is_none_or(|pc| pc >= function.code().len())
        {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug PC is outside the instruction stream",
            )));
        }
        if previous_pc.is_some_and(|previous| entry.pc < previous) {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug PCs are not ordered",
            )));
        }
        if entry.position.line == u32::MAX || entry.position.column == u32::MAX {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug position cannot be represented one-based",
            )));
        }
        previous_pc = Some(entry.pc);
    }
    Ok(())
}

fn verify_capture_flags(
    previous: &mut Option<(bool, bool, ClosureVariableKind)>,
    current: (bool, bool, ClosureVariableKind),
) -> Result<(), RuntimeError> {
    if previous.is_some_and(|previous| previous != current) {
        return Err(RuntimeError::Engine(Error::internal(
            "sibling closure descriptors disagree about one parent binding",
        )));
    }
    *previous = Some(current);
    Ok(())
}

fn flatten_unlinked_tree(function: UnlinkedFunction) -> Result<Vec<FlatFunction>, RuntimeError> {
    let mut frames = vec![FlattenFrame::new(function)];
    let mut functions = Vec::new();

    loop {
        let next = frames
            .last_mut()
            .ok_or(RuntimeError::Invariant(
                "unlinked function flattening lost its root frame",
            ))?
            .remaining
            .next();
        if let Some(constant) = next {
            let (primitive, child) = constant.into_parts();
            match (primitive, child) {
                (Some(value), None) => frames
                    .last_mut()
                    .expect("flatten frame remains present")
                    .constants
                    .push(FlatConstant::Value(raw_unlinked_primitive(value)?)),
                (None, Some(child)) => frames.push(FlattenFrame::new(child)),
                (None, None) | (Some(_), Some(_)) => {
                    return Err(RuntimeError::Invariant(
                        "unlinked constant did not contain exactly one payload",
                    ));
                }
            }
            continue;
        }

        let frame = frames.pop().ok_or(RuntimeError::Invariant(
            "unlinked function flattening lost a completed frame",
        ))?;
        let index = functions.len();
        functions.push(FlatFunction {
            code: frame.code,
            constants: frame.constants,
            metadata: frame.metadata,
            func_name: frame.func_name,
            closure_variables: frame.closure_variables,
            debug: frame.debug,
        });
        if let Some(parent) = frames.last_mut() {
            parent.constants.push(FlatConstant::Child(index));
        } else {
            return Ok(functions);
        }
    }
}

fn raw_unlinked_primitive(value: Value) -> Result<RawValue, RuntimeError> {
    match value {
        Value::Undefined => Ok(RawValue::Undefined),
        Value::Null => Ok(RawValue::Null),
        Value::Bool(value) => Ok(RawValue::Bool(value)),
        Value::Int(value) => Ok(RawValue::Int(value)),
        Value::Float(value) => Ok(RawValue::Float(value)),
        Value::BigInt(value) => Ok(RawValue::BigInt(value)),
        Value::String(value) => Ok(RawValue::String(value)),
        Value::Object(_) | Value::Symbol(_) => Err(RuntimeError::Invariant(
            "runtime-bound value escaped the unlinked constant invariant",
        )),
    }
}

fn descriptor_to_validation_record(
    descriptor: &OrdinaryPropertyDescriptor,
) -> PropertyDescriptor<Value> {
    PropertyDescriptor {
        value: descriptor.value.as_ref().into_option().cloned(),
        writable: descriptor.writable.as_ref().into_option().copied(),
        get: descriptor.get.as_ref().into_option().map(|accessor| {
            accessor
                .as_callable()
                .map(|callable| Value::Object(callable.as_object().clone()))
        }),
        set: descriptor.set.as_ref().into_option().map(|accessor| {
            accessor
                .as_callable()
                .map(|callable| Value::Object(callable.as_object().clone()))
        }),
        enumerable: descriptor.enumerable.as_ref().into_option().copied(),
        configurable: descriptor.configurable.as_ref().into_option().copied(),
    }
}

fn complete_to_validation_record(
    descriptor: &CompleteOrdinaryPropertyDescriptor,
) -> CompletePropertyDescriptor<Value> {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => CompletePropertyDescriptor::Data {
            value: value.clone(),
            writable: *writable,
            enumerable: *enumerable,
            configurable: *configurable,
        },
        CompleteOrdinaryPropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => CompletePropertyDescriptor::Accessor {
            get: get
                .as_ref()
                .map(|callable| Value::Object(callable.as_object().clone())),
            set: set
                .as_ref()
                .map(|callable| Value::Object(callable.as_object().clone())),
            enumerable: *enumerable,
            configurable: *configurable,
        },
    }
}

fn validation_record_to_complete(
    descriptor: CompletePropertyDescriptor<Value>,
) -> Result<CompleteOrdinaryPropertyDescriptor, RuntimeError> {
    match descriptor {
        CompletePropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => Ok(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }),
        CompletePropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => {
            let get = get
                .map(|value| match value {
                    Value::Object(object) => Ok(CallableRef::from_validated_object(object)),
                    _ => Err(RuntimeError::Invariant(
                        "validated accessor getter was not callable",
                    )),
                })
                .transpose()?;
            let set = set
                .map(|value| match value {
                    Value::Object(object) => Ok(CallableRef::from_validated_object(object)),
                    _ => Err(RuntimeError::Invariant(
                        "validated accessor setter was not callable",
                    )),
                })
                .transpose()?;
            Ok(CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            })
        }
    }
}

/// One realm and its execution state.
/// Execution-only eval options. Compilation metadata remains in
/// [`CompileOptions`]; the barrier mirrors QuickJS
/// `JS_EVAL_FLAG_BACKTRACE_BARRIER` and temporarily marks the caller frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalOptions {
    pub filename: String,
    pub backtrace_barrier: bool,
}

impl EvalOptions {
    #[must_use]
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            backtrace_barrier: false,
        }
    }
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self::new(crate::compiler::DEFAULT_EVAL_FILENAME)
    }
}

pub struct Context {
    runtime: Runtime,
    id: u64,
    realm: ContextId,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        self.runtime
            .retain_context_handle(self.realm)
            .expect("a live Context handle must retain its realm");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
            realm: self.realm,
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.runtime.release_context_handle(self.realm);
    }
}

impl Context {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Return this realm's `%Object.prototype%` root.
    pub fn object_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .object_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%Function.prototype%` root.
    pub fn function_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's boxed-false `%Boolean.prototype%` root.
    pub fn boolean_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Boolean)
    }

    /// Return this realm's `%Function%` constructor root.
    pub fn function_constructor(&self) -> Result<CallableRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant("realm has no Function constructor"))?;
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_borrowed_handle(self.runtime.clone(), object)?,
        ))
    }

    /// Return this realm's global object root.
    pub fn global_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return the null-prototype object used for global lexical bindings.
    pub fn global_var_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_var_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    #[cfg(test)]
    pub(crate) fn create_global_lexical_for_test(
        &self,
        name: &str,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .create_global_lexical_for_test(self.realm, name, is_const, initial_value)
    }

    #[cfg(test)]
    pub(crate) fn initialize_global_lexical_for_test(
        &self,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .initialize_global_lexical_for_test(self.realm, name, value)
    }

    /// Allocate an ordinary object with this realm's `%Object.prototype%`.
    pub fn new_object(&mut self) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.object_prototype()?;
        self.runtime.new_object(Some(&prototype))
    }

    /// Allocate an ordinary object with an explicit object-or-null prototype.
    pub fn new_object_with_prototype(
        &mut self,
        prototype: Option<&ObjectRef>,
    ) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_object(prototype)
    }

    pub fn get_own_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        self.runtime.get_own_property(object, key)
    }

    pub fn define_own_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<bool, RuntimeError> {
        self.runtime.define_own_property(object, key, descriptor)
    }

    pub fn get_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        match self.runtime.prepare_get_property(object, key)? {
            PropertyGetAction::Complete(value) => Ok(value),
            PropertyGetAction::Call { getter, receiver } => {
                let completion = self
                    .runtime
                    .call_internal(self.realm, &getter, receiver, &[])?;
                self.finish_completion(completion)
            }
        }
    }

    pub fn get_property_with_receiver(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Value, RuntimeError> {
        match self
            .runtime
            .prepare_get_property_with_receiver(object, key, receiver)?
        {
            PropertyGetAction::Complete(value) => Ok(value),
            PropertyGetAction::Call { getter, receiver } => {
                let completion = self
                    .runtime
                    .call_internal(self.realm, &getter, receiver, &[])?;
                self.finish_completion(completion)
            }
        }
    }

    pub fn set_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        match self.runtime.prepare_set_property(object, key, value)? {
            PropertySetAction::Complete => Ok(true),
            PropertySetAction::Rejected(_) => Ok(false),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => {
                let completion =
                    self.runtime
                        .call_internal(self.realm, &setter, receiver, &[argument])?;
                let returned = self.finish_completion(completion)?;
                drop(returned);
                Ok(true)
            }
        }
    }

    pub fn set_property_with_receiver(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<bool, RuntimeError> {
        match self
            .runtime
            .prepare_set_property_with_receiver(object, key, value, receiver)?
        {
            PropertySetAction::Complete => Ok(true),
            PropertySetAction::Rejected(_) => Ok(false),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => {
                let completion =
                    self.runtime
                        .call_internal(self.realm, &setter, receiver, &[argument])?;
                let returned = self.finish_completion(completion)?;
                drop(returned);
                Ok(true)
            }
        }
    }

    /// Compile one script and publish its immutable bytecode in this realm.
    ///
    /// The returned handle is a runtime root. Its constant pool and captured
    /// realm remain alive even if this particular `Context` handle is dropped.
    pub fn compile(&mut self, source: &str) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::default())
    }

    /// Compile one script with an explicit filename attached independently to
    /// every published function's debug metadata.
    pub fn compile_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one script with named compilation options.
    pub fn compile_with_options(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        match self
            .runtime
            .compile_in_realm(self.realm, source, &options.filename)?
        {
            Compilation::Published(function) => Ok(function),
            Compilation::Throw(exception) => {
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Instantiate and evaluate runtime-owned script bytecode.
    ///
    /// As in QuickJS's `JS_EvalFunctionInternal`, the raw bytecode is first
    /// wrapped in a callable object in the initiating context. The call then
    /// executes in the realm captured by the bytecode.
    pub fn execute(&mut self, function: &FunctionBytecodeRef) -> Result<Value, RuntimeError> {
        let callable = self.runtime.new_bytecode_closure(self.realm, function)?;
        let this_value = Value::Object(self.global_object()?);
        self.call(&callable, this_value, &[])
    }

    /// Invoke a validated callable with an explicit `this` value and arguments.
    pub fn call(
        &mut self,
        callable: &CallableRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        let completion = self
            .runtime
            .call_internal(self.realm, callable, this_value, arguments)?;
        self.finish_completion(completion)
    }

    /// Invoke a validated constructor with itself as `new.target`, matching
    /// `JS_CallConstructor` and source-level `new`.
    pub fn construct(
        &mut self,
        constructor: &CallableRef,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        self.construct_with_new_target(constructor, constructor, arguments)
    }

    /// Invoke a constructor with an explicit `new.target`, matching
    /// `JS_CallConstructor2`/`Reflect.construct` semantics.
    pub fn construct_with_new_target(
        &mut self,
        constructor: &CallableRef,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        match self
            .runtime
            .construct_internal(self.realm, constructor, new_target, arguments)
        {
            Ok(completion) => self.finish_completion(completion),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible native error");
                let exception = self
                    .runtime
                    .new_native_error(self.realm, kind, error.message())?;
                self.runtime
                    .ensure_error_backtrace(&exception, false, None)?;
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            Err(error) => Err(error),
        }
    }

    fn finish_completion(&mut self, completion: Completion) -> Result<Value, RuntimeError> {
        match completion {
            Completion::Return(value) => Ok(value),
            Completion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Return whether this runtime currently carries a pending JavaScript
    /// exception completion.
    #[must_use]
    pub fn has_exception(&self) -> bool {
        self.runtime.has_pending_exception()
    }

    /// Move the pending JavaScript exception value out of the runtime slot.
    pub fn take_exception(&mut self) -> Result<Option<Value>, RuntimeError> {
        self.runtime.take_pending_exception()
    }

    /// Compile and evaluate one script through runtime-owned bytecode.
    ///
    /// # Errors
    /// Returns syntax, publication, runtime-domain, or execution errors.
    pub fn eval(&mut self, source: &str) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::default())
    }

    /// Compile and evaluate a script with an explicit debug filename.
    pub fn eval_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::new(filename))
    }

    /// Compile and evaluate a script with filename and execution options.
    pub fn eval_with_options(
        &mut self,
        source: &str,
        options: &EvalOptions,
    ) -> Result<Value, RuntimeError> {
        let barrier = self
            .runtime
            .install_backtrace_barrier(options.backtrace_barrier)?;
        let result = (|| {
            let function =
                self.compile_with_options(source, &CompileOptions::new(&options.filename))?;
            self.execute(&function)
        })();
        barrier.finish()?;
        result
    }
}

#[cfg(test)]
mod tests {
    use crate::JsBigInt;
    use crate::bytecode::{BytecodeFunction, Instruction};
    use crate::debug::{DebugInfoMode, LineColumn, Pc2LineEntry, Pc2LineTable};
    use crate::error::NativeErrorKind;
    use crate::function::{UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug};
    use crate::heap::{
        ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, ConstructorKind,
        DynamicFunctionKind, FunctionDebugPosition, FunctionMetadata, NativeCProto,
        NativeFunctionId, ObjectPayload, PrimitiveKind, PrimitiveObjectData,
    };
    use crate::object::{
        AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField,
        OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
    };
    use crate::value::{JsString, Value};
    use crate::vm::Completion;

    use super::{
        ActiveFrameKind, CallableExecution, DeferredRefOp, EvalOptions, PropertyGetAction,
        PropertySetAction, Runtime, RuntimeError, VarRefRoot,
    };

    fn data_descriptor(
        value: Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    ) -> OrdinaryPropertyDescriptor {
        OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(value),
            writable: DescriptorField::Present(writable),
            enumerable: DescriptorField::Present(enumerable),
            configurable: DescriptorField::Present(configurable),
            ..OrdinaryPropertyDescriptor::new()
        }
    }

    fn set_property(
        runtime: &Runtime,
        object: &crate::ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        match runtime.prepare_set_property(object, key, value)? {
            PropertySetAction::Complete => Ok(true),
            PropertySetAction::Rejected(_) => Ok(false),
            PropertySetAction::Call { .. } => Err(RuntimeError::Invariant(
                "ordinary-property test helper unexpectedly reached a setter",
            )),
        }
    }

    fn set_property_with_receiver(
        runtime: &Runtime,
        object: &crate::ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<bool, RuntimeError> {
        match runtime.prepare_set_property_with_receiver(object, key, value, receiver)? {
            PropertySetAction::Complete => Ok(true),
            PropertySetAction::Rejected(_) => Ok(false),
            PropertySetAction::Call { .. } => Err(RuntimeError::Invariant(
                "ordinary-property test helper unexpectedly reached a setter",
            )),
        }
    }

    fn get_property(
        runtime: &Runtime,
        object: &crate::ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        match runtime.prepare_get_property(object, key)? {
            PropertyGetAction::Complete(value) => Ok(value),
            PropertyGetAction::Call { .. } => Err(RuntimeError::Invariant(
                "ordinary-property test helper unexpectedly reached a getter",
            )),
        }
    }

    fn global_callable(runtime: &Runtime, context: &mut super::Context, name: &str) -> CallableRef {
        let key = runtime.intern_property_key(name).unwrap();
        let Value::Object(object) = context
            .get_property(&context.global_object().unwrap(), &key)
            .unwrap()
        else {
            panic!("global {name} was not an object");
        };
        runtime
            .as_callable(&object)
            .unwrap()
            .unwrap_or_else(|| panic!("global {name} was not callable"))
    }

    fn eval_callable(runtime: &Runtime, context: &mut super::Context, source: &str) -> CallableRef {
        let Value::Object(object) = context.eval(source).unwrap() else {
            panic!("callable source did not produce an object: {source:?}");
        };
        runtime
            .as_callable(&object)
            .unwrap()
            .unwrap_or_else(|| panic!("source did not produce a callable: {source:?}"))
    }

    fn property_callable(
        runtime: &Runtime,
        context: &mut super::Context,
        object: &crate::ObjectRef,
        name: &str,
    ) -> CallableRef {
        let key = runtime.intern_property_key(name).unwrap();
        let Value::Object(value) = context.get_property(object, &key).unwrap() else {
            panic!("property {name} was not an object");
        };
        runtime
            .as_callable(&value)
            .unwrap()
            .unwrap_or_else(|| panic!("property {name} was not callable"))
    }

    fn own_key_names(runtime: &Runtime, object: &crate::ObjectRef) -> Vec<String> {
        runtime
            .own_property_keys(object)
            .unwrap()
            .into_iter()
            .map(|key| {
                runtime
                    .property_key_to_js_string(&key)
                    .unwrap()
                    .to_utf8_lossy()
            })
            .collect()
    }

    fn own_data_value(runtime: &Runtime, object: &crate::ObjectRef, name: &str) -> Value {
        let key = runtime.intern_property_key(name).unwrap();
        let Some(CompleteOrdinaryPropertyDescriptor::Data { value, .. }) =
            runtime.get_own_property(object, &key).unwrap()
        else {
            panic!("{name} was not an own data property");
        };
        value
    }

    fn own_stack_string(runtime: &Runtime, object: &crate::ObjectRef) -> JsString {
        let Value::String(stack) = own_data_value(runtime, object, "stack") else {
            panic!("stack was not a string");
        };
        stack
    }

    fn take_error_message(runtime: &Runtime, context: &mut super::Context) -> JsString {
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("pending exception was not an Error object");
        };
        let message = runtime.intern_property_key("message").unwrap();
        let Value::String(message) = context.get_property(&error, &message).unwrap() else {
            panic!("Error.message was not a string");
        };
        message
    }

    fn bytecode_callable(
        runtime: &Runtime,
        context: &super::Context,
        code: Vec<Instruction>,
        metadata: FunctionMetadata,
    ) -> CallableRef {
        let function = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(code, Vec::new(), metadata),
            )
            .unwrap();
        runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap()
    }

    #[test]
    fn contexts_share_runtime_but_have_distinct_identity() {
        let runtime = Runtime::new();
        let mut first = runtime.new_context();
        let second = runtime.new_context();
        assert_ne!(first.id(), second.id());
        assert!(first.runtime().is_same_runtime(second.runtime()));
        let first_prototype = first.object_prototype().unwrap();
        let second_prototype = second.object_prototype().unwrap();
        assert_ne!(first_prototype, second_prototype);
        let function_prototype = first.function_prototype().unwrap();
        let global_object = first.global_object().unwrap();
        let global_var_object = first.global_var_object().unwrap();
        assert_eq!(
            runtime.get_prototype_of(&function_prototype).unwrap(),
            Some(first_prototype.clone())
        );
        assert_eq!(
            runtime.get_prototype_of(&global_object).unwrap(),
            Some(first_prototype.clone())
        );
        assert_eq!(runtime.get_prototype_of(&global_var_object).unwrap(), None);
        assert!(runtime.set_prototype_of(&global_var_object, None).unwrap());
        assert!(
            !runtime
                .set_prototype_of(&global_var_object, Some(&first_prototype))
                .unwrap()
        );
        let object = first.new_object().unwrap();
        assert_eq!(
            runtime.get_prototype_of(&object).unwrap(),
            Some(first_prototype.clone())
        );
        assert!(runtime.set_prototype_of(&first_prototype, None).unwrap());
        assert!(
            !runtime
                .set_prototype_of(&first_prototype, Some(&object))
                .unwrap()
        );
    }

    #[test]
    fn boolean_intrinsic_graph_payload_and_brand_methods_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = global_callable(&runtime, &mut context, "Boolean");
        let prototype = context.boolean_prototype().unwrap();

        assert_eq!(
            runtime.get_prototype_of(constructor.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert_eq!(
            runtime.get_prototype_of(&prototype).unwrap(),
            Some(context.object_prototype().unwrap())
        );
        assert_eq!(
            own_key_names(&runtime, constructor.as_object()),
            ["length", "name", "prototype"]
        );
        assert_eq!(
            own_key_names(&runtime, &prototype),
            ["toString", "valueOf", "constructor"]
        );
        assert!(matches!(
            &runtime
                .0
                .state
                .borrow()
                .heap
                .object(prototype.object_id())
                .unwrap()
                .payload,
            ObjectPayload::Primitive(PrimitiveObjectData::Boolean(false))
        ));
        {
            let state = runtime.0.state.borrow();
            let slots = state
                .heap
                .context(context.realm)
                .unwrap()
                .primitive_prototypes;
            assert_eq!(
                slots[PrimitiveKind::Boolean.index()],
                Some(prototype.object_id())
            );
            for kind in [
                PrimitiveKind::Number,
                PrimitiveKind::String,
                PrimitiveKind::Symbol,
                PrimitiveKind::BigInt,
            ] {
                assert_eq!(slots[kind.index()], None, "{kind:?} slot was enabled early");
            }
        }

        assert_eq!(
            context.call(&constructor, Value::Undefined, &[]).unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .call(&constructor, Value::Undefined, &[Value::Int(1)])
                .unwrap(),
            Value::Bool(true)
        );
        let wrapper = context
            .construct(&constructor, &[Value::Bool(false)])
            .unwrap();
        let Value::Object(wrapper) = wrapper else {
            panic!("new Boolean did not return an object");
        };
        assert_eq!(runtime.own_property_keys(&wrapper).unwrap(), []);
        assert_eq!(
            runtime.get_prototype_of(&wrapper).unwrap(),
            Some(prototype.clone())
        );
        let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
        let to_string = property_callable(&runtime, &mut context, &prototype, "toString");
        assert_eq!(
            context
                .call(&value_of, Value::Object(wrapper.clone()), &[])
                .unwrap(),
            Value::Bool(false)
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(wrapper.clone()), &[])
                .unwrap(),
            Value::String(JsString::from("false"))
        );
        assert_eq!(
            context.call(&value_of, Value::Bool(true), &[]).unwrap(),
            Value::Bool(true)
        );
        let spoof = runtime.new_object(Some(&prototype)).unwrap();
        assert!(matches!(
            context.call(&value_of, Value::Object(spoof), &[]),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from("not a boolean")
        );
        assert_eq!(
            context
                .eval("true.toString() + '|' + false.valueOf()")
                .unwrap(),
            Value::String(JsString::from("true|false"))
        );
        assert_eq!(context.eval("+new Boolean(false)").unwrap(), Value::Int(0));
    }

    #[test]
    fn global_numeric_parsers_match_quickjs_graph_conversion_order_and_results() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let parse_int = global_callable(&runtime, &mut context, "parseInt");
        let parse_float = global_callable(&runtime, &mut context, "parseFloat");

        for (name, callable, length) in
            [("parseInt", &parse_int, 2), ("parseFloat", &parse_float, 1)]
        {
            assert_eq!(
                own_key_names(&runtime, callable.as_object()),
                ["length", "name"]
            );
            assert_eq!(
                runtime.get_prototype_of(callable.as_object()).unwrap(),
                Some(context.function_prototype().unwrap())
            );
            assert!(!runtime.is_constructor(callable.as_object()).unwrap());
            assert_eq!(
                own_data_value(&runtime, callable.as_object(), "length"),
                Value::Int(length)
            );
            assert_eq!(
                own_data_value(&runtime, callable.as_object(), "name"),
                Value::String(JsString::from(name))
            );
            for property in ["length", "name"] {
                let property = runtime.intern_property_key(property).unwrap();
                assert!(matches!(
                    runtime
                        .get_own_property(callable.as_object(), &property)
                        .unwrap(),
                    Some(CompleteOrdinaryPropertyDescriptor::Data {
                        writable: false,
                        enumerable: false,
                        configurable: true,
                        ..
                    })
                ));
            }
            let key = runtime.intern_property_key(name).unwrap();
            assert!(matches!(
                runtime.get_own_property(&global, &key).unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    writable: true,
                    enumerable: false,
                    configurable: true,
                    ..
                })
            ));
        }

        assert_eq!(
            context
                .call(
                    &parse_int,
                    Value::Bool(true),
                    &[Value::String(JsString::from("0x10"))],
                )
                .unwrap(),
            Value::Int(16)
        );
        assert_eq!(
            context
                .call(
                    &parse_int,
                    Value::Undefined,
                    &[
                        Value::String(JsString::from("10")),
                        Value::Float(4_294_967_298.0),
                    ],
                )
                .unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            context
                .call(
                    &parse_float,
                    Value::Undefined,
                    &[Value::String(JsString::from(
                        "1.0000000000000001110223024625156540423631668090820313",
                    ))],
                )
                .unwrap(),
            Value::Int(1)
        );
        let Value::Float(negative_zero) = context
            .call(
                &parse_int,
                Value::Undefined,
                &[Value::String(JsString::from("-0"))],
            )
            .unwrap()
        else {
            panic!("parseInt('-0') did not preserve the float tag");
        };
        assert_eq!(negative_zero.to_bits(), (-0.0_f64).to_bits());

        let log_key = runtime.intern_property_key("parseLog").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &global,
                    &log_key,
                    &data_descriptor(Value::String(JsString::from("")), true, true, true),
                )
                .unwrap()
        );
        let to_primitive =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
        let input = context.new_object().unwrap();
        let input_conversion = eval_callable(
            &runtime,
            &mut context,
            "(function(hint) { parseLog = parseLog + 'input:' + hint + '|'; return '10'; })",
        );
        assert!(
            runtime
                .define_own_property(
                    &input,
                    &to_primitive,
                    &data_descriptor(
                        Value::Object(input_conversion.as_object().clone()),
                        true,
                        false,
                        true,
                    ),
                )
                .unwrap()
        );
        let radix = context.new_object().unwrap();
        let radix_conversion = eval_callable(
            &runtime,
            &mut context,
            "(function(hint) { parseLog = parseLog + 'radix:' + hint + '|'; return 2; })",
        );
        assert!(
            runtime
                .define_own_property(
                    &radix,
                    &to_primitive,
                    &data_descriptor(
                        Value::Object(radix_conversion.as_object().clone()),
                        true,
                        false,
                        true,
                    ),
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(
                    &parse_int,
                    Value::Undefined,
                    &[Value::Object(input.clone()), Value::Object(radix)],
                )
                .unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            context.get_property(&global, &log_key).unwrap(),
            Value::String(JsString::from("input:string|radix:number|"))
        );

        assert!(
            runtime
                .define_own_property(
                    &global,
                    &log_key,
                    &data_descriptor(Value::String(JsString::from("")), true, true, true),
                )
                .unwrap()
        );
        let throwing_input = context.new_object().unwrap();
        let input_throw = eval_callable(
            &runtime,
            &mut context,
            "(function(hint) { parseLog = parseLog + 'input-throw:' + hint + '|'; throw 'input boom'; })",
        );
        assert!(
            runtime
                .define_own_property(
                    &throwing_input,
                    &to_primitive,
                    &data_descriptor(
                        Value::Object(input_throw.as_object().clone()),
                        true,
                        false,
                        true,
                    ),
                )
                .unwrap()
        );
        let late_radix = context.new_object().unwrap();
        let late_radix_conversion = eval_callable(
            &runtime,
            &mut context,
            "(function() { parseLog = parseLog + 'late-radix|'; return 2; })",
        );
        assert!(
            runtime
                .define_own_property(
                    &late_radix,
                    &to_primitive,
                    &data_descriptor(
                        Value::Object(late_radix_conversion.as_object().clone()),
                        true,
                        false,
                        true,
                    ),
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(
                &parse_int,
                Value::Undefined,
                &[Value::Object(throwing_input), Value::Object(late_radix),],
            ),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(
            context.take_exception().unwrap(),
            Some(Value::String(JsString::from("input boom")))
        );
        assert_eq!(
            context.get_property(&global, &log_key).unwrap(),
            Value::String(JsString::from("input-throw:string|"))
        );

        let symbol = runtime.new_symbol(Some(JsString::from("parse"))).unwrap();
        assert!(
            runtime
                .define_own_property(
                    &global,
                    &log_key,
                    &data_descriptor(Value::String(JsString::from("")), true, true, true),
                )
                .unwrap()
        );
        let symbol_radix = context.new_object().unwrap();
        let symbol_radix_conversion = eval_callable(
            &runtime,
            &mut context,
            "(function() { parseLog = parseLog + 'symbol-radix|'; return 2; })",
        );
        assert!(
            runtime
                .define_own_property(
                    &symbol_radix,
                    &to_primitive,
                    &data_descriptor(
                        Value::Object(symbol_radix_conversion.as_object().clone()),
                        true,
                        false,
                        true,
                    ),
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(
                &parse_int,
                Value::Undefined,
                &[Value::Symbol(symbol.clone()), Value::Object(symbol_radix),],
            ),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from("cannot convert symbol to string")
        );
        assert_eq!(
            context.get_property(&global, &log_key).unwrap(),
            Value::String(JsString::from(""))
        );
        assert!(matches!(
            context.call(&parse_float, Value::Undefined, &[Value::Symbol(symbol)],),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from("cannot convert symbol to string")
        );

        let type_error = global_callable(&runtime, &mut context, "TypeError");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let Value::Object(defining_type_error_prototype) = context
            .get_property(type_error.as_object(), &prototype_key)
            .unwrap()
        else {
            panic!("defining TypeError.prototype was not an object");
        };
        let mut caller = runtime.new_context();
        assert_eq!(
            caller.call(
                &parse_int,
                Value::Undefined,
                &[
                    Value::String(JsString::from("10")),
                    Value::BigInt(JsBigInt::one()),
                ],
            ),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
            panic!("cross-realm parseInt did not throw an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(defining_type_error_prototype)
        );
    }

    #[test]
    fn global_primitive_constants_match_quickjs_frozen_descriptors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        for (name, expected) in [
            ("undefined", Value::Undefined),
            ("NaN", Value::Float(f64::NAN)),
            ("Infinity", Value::Float(f64::INFINITY)),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            let Some(CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            }) = runtime.get_own_property(&global, &key).unwrap()
            else {
                panic!("global {name} was not an own data property");
            };
            assert!(value.same_value(&expected), "global {name}");
            assert!(!writable, "global {name}");
            assert!(!enumerable, "global {name}");
            assert!(!configurable, "global {name}");
            assert!(!runtime.delete_property(&global, &key).unwrap());
            assert_eq!(
                context.eval(&format!("delete {name}")).unwrap(),
                Value::Bool(false)
            );
        }
    }

    #[test]
    fn eval_uses_the_compiler_and_vm_path() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval("6 * 7").unwrap(), Value::Int(42));
        assert_eq!(
            context.eval("this").unwrap(),
            Value::Object(context.global_object().unwrap())
        );
    }

    #[test]
    fn boolean_wrappers_lookup_and_new_target_use_the_required_realms() {
        let runtime = Runtime::new();
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_constructor = global_callable(&runtime, &mut first, "Boolean");
        let second_constructor = global_callable(&runtime, &mut second, "Boolean");
        let first_prototype = first.boolean_prototype().unwrap();
        let second_prototype = second.boolean_prototype().unwrap();
        let first_object_prototype = first.object_prototype().unwrap();
        let first_object_value_of =
            property_callable(&runtime, &mut first, &first_object_prototype, "valueOf");
        let Value::Object(method_wrapper) = second
            .call(&first_object_value_of, Value::Bool(false), &[])
            .unwrap()
        else {
            panic!("cross-realm Object.prototype.valueOf did not box Boolean");
        };
        assert_eq!(
            runtime.get_prototype_of(&method_wrapper).unwrap(),
            Some(first_prototype.clone())
        );

        let Value::Object(cross_wrapper) = second
            .construct_with_new_target(
                &first_constructor,
                &second_constructor,
                &[Value::Bool(true)],
            )
            .unwrap()
        else {
            panic!("cross-realm Boolean construction did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&cross_wrapper).unwrap(),
            Some(second_prototype.clone())
        );
        let second_value_of =
            property_callable(&runtime, &mut second, &second_prototype, "valueOf");
        assert_eq!(
            first
                .call(&second_value_of, Value::Object(cross_wrapper.clone()), &[],)
                .unwrap(),
            Value::Bool(true)
        );

        let marker = runtime.intern_property_key("realmMarker").unwrap();
        assert!(
            first
                .define_own_property(
                    &first_prototype,
                    &marker,
                    &data_descriptor(Value::Int(1), true, false, true),
                )
                .unwrap()
        );
        assert!(
            second
                .define_own_property(
                    &second_prototype,
                    &marker,
                    &data_descriptor(Value::Int(2), true, false, true),
                )
                .unwrap()
        );
        let callable = |runtime: &Runtime, context: &mut super::Context, source: &str| {
            let Value::Object(function) = context.eval(source).unwrap() else {
                panic!("realm lookup probe did not produce a function");
            };
            runtime.as_callable(&function).unwrap().unwrap()
        };
        let first_reader = callable(
            &runtime,
            &mut first,
            "(function(){ return true.realmMarker; })",
        );
        let second_reader = callable(
            &runtime,
            &mut second,
            "(function(){ return true.realmMarker; })",
        );
        assert_eq!(
            second.call(&first_reader, Value::Undefined, &[]).unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            first.call(&second_reader, Value::Undefined, &[]).unwrap(),
            Value::Int(2)
        );

        let custom_prototype = second.new_object().unwrap();
        let new_target = runtime
            .new_bound_native_function(
                &second.function_prototype().unwrap(),
                second.realm,
                NativeFunctionId::ConstructorProbe,
                0,
            )
            .unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        assert!(
            second
                .define_own_property(
                    new_target.as_object(),
                    &prototype_key,
                    &data_descriptor(Value::Object(custom_prototype.clone()), true, false, true,),
                )
                .unwrap()
        );
        let Value::Object(custom_wrapper) = first
            .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)])
            .unwrap()
        else {
            panic!("custom newTarget did not produce a Boolean wrapper");
        };
        assert_eq!(
            runtime.get_prototype_of(&custom_wrapper).unwrap(),
            Some(custom_prototype)
        );
        assert!(
            second
                .define_own_property(
                    new_target.as_object(),
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(fallback_wrapper) = first
            .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)])
            .unwrap()
        else {
            panic!("fallback newTarget did not produce a Boolean wrapper");
        };
        assert_eq!(
            runtime.get_prototype_of(&fallback_wrapper).unwrap(),
            Some(second_prototype.clone())
        );
        let throwing_getter = bytecode_callable(
            &runtime,
            &second,
            vec![Instruction::PushI32(77), Instruction::Throw],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            second
                .define_own_property(
                    new_target.as_object(),
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(throwing_getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            first
                .construct_with_new_target(&first_constructor, &new_target, &[Value::Bool(false)],),
            Err(RuntimeError::Exception)
        );
        assert_eq!(first.take_exception().unwrap(), Some(Value::Int(77)));

        let escaped_this = callable(&runtime, &mut first, "(function(){ return this; })");
        let Value::Object(boxed_this) =
            second.call(&escaped_this, Value::Bool(false), &[]).unwrap()
        else {
            panic!("sloppy Boolean this did not escape as a wrapper");
        };
        assert_eq!(
            runtime.get_prototype_of(&boxed_this).unwrap(),
            Some(first_prototype)
        );
        let stable_this = callable(
            &runtime,
            &mut first,
            "(function(){ return this === this; })",
        );
        assert_eq!(
            second.call(&stable_this, Value::Bool(false), &[]).unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn boolean_primitive_accessors_writes_and_delete_preserve_raw_receiver_semantics() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let prototype = context.boolean_prototype().unwrap();
        let value_of = property_callable(&runtime, &mut context, &prototype, "valueOf");
        let strict_getter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushThis, Instruction::Return],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let sloppy_getter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushThis, Instruction::Return],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        for (name, getter) in [
            ("strictReceiver", strict_getter),
            ("sloppyReceiver", sloppy_getter),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            assert!(
                context
                    .define_own_property(
                        &prototype,
                        &key,
                        &OrdinaryPropertyDescriptor {
                            get: DescriptorField::Present(AccessorValue::Callable(getter)),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .unwrap()
            );
        }
        assert_eq!(
            context.eval("false.strictReceiver").unwrap(),
            Value::Bool(false)
        );
        let Value::Object(sloppy_receiver) = context.eval("false.sloppyReceiver").unwrap() else {
            panic!("sloppy primitive getter did not receive a Boolean wrapper");
        };
        assert_eq!(
            context
                .call(&value_of, Value::Object(sloppy_receiver), &[])
                .unwrap(),
            Value::Bool(false)
        );

        let strict_setter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushThis, Instruction::Throw],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let sloppy_setter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushThis, Instruction::Throw],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        for (name, setter) in [("strictSink", strict_setter), ("sloppySink", sloppy_setter)] {
            let key = runtime.intern_property_key(name).unwrap();
            assert!(
                context
                    .define_own_property(
                        &prototype,
                        &key,
                        &OrdinaryPropertyDescriptor {
                            set: DescriptorField::Present(AccessorValue::Callable(setter)),
                            configurable: DescriptorField::Present(true),
                            ..OrdinaryPropertyDescriptor::new()
                        },
                    )
                    .unwrap()
            );
        }
        assert_eq!(
            context.eval("false.strictSink = 7"),
            Err(RuntimeError::Exception)
        );
        assert_eq!(context.take_exception().unwrap(), Some(Value::Bool(false)));
        assert_eq!(
            context.eval("false.sloppySink = 7"),
            Err(RuntimeError::Exception)
        );
        let Some(Value::Object(sloppy_receiver)) = context.take_exception().unwrap() else {
            panic!("sloppy primitive setter did not receive a Boolean wrapper");
        };
        assert_eq!(
            context
                .call(&value_of, Value::Object(sloppy_receiver), &[])
                .unwrap(),
            Value::Bool(false)
        );

        let writable = runtime.intern_property_key("writablePrimitive").unwrap();
        let read_only = runtime.intern_property_key("readOnlyPrimitive").unwrap();
        assert!(
            context
                .define_own_property(
                    &prototype,
                    &writable,
                    &data_descriptor(Value::Int(1), true, false, true),
                )
                .unwrap()
        );
        assert!(
            context
                .define_own_property(
                    &prototype,
                    &read_only,
                    &data_descriptor(Value::Int(1), false, false, true),
                )
                .unwrap()
        );
        assert_eq!(
            context.eval("false.writablePrimitive = 7").unwrap(),
            Value::Int(7)
        );
        assert_eq!(
            context.eval("false.writablePrimitive").unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            context.eval("'use strict'; false.writablePrimitive = 7"),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from("not an object")
        );
        assert_eq!(
            context.eval("'use strict'; false.readOnlyPrimitive = 7"),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from("'readOnlyPrimitive' is read-only")
        );
        assert_eq!(
            context.eval("delete false.writablePrimitive").unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            context.eval("false.writablePrimitive").unwrap(),
            Value::Int(1)
        );
    }

    #[test]
    fn object_prototype_boolean_methods_box_only_the_quickjs_paths() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object_prototype = context.object_prototype().unwrap();
        let boolean_prototype = context.boolean_prototype().unwrap();
        let object_to_string =
            property_callable(&runtime, &mut context, &object_prototype, "toString");
        let object_value_of =
            property_callable(&runtime, &mut context, &object_prototype, "valueOf");
        let object_to_locale_string =
            property_callable(&runtime, &mut context, &object_prototype, "toLocaleString");
        let boolean_value_of =
            property_callable(&runtime, &mut context, &boolean_prototype, "valueOf");

        assert_eq!(
            context
                .call(&object_to_string, Value::Bool(false), &[])
                .unwrap(),
            Value::String(JsString::from("[object Boolean]"))
        );
        assert_eq!(
            context
                .call(&object_to_locale_string, Value::Bool(false), &[])
                .unwrap(),
            Value::String(JsString::from("false"))
        );
        let Value::Object(first_wrapper) = context
            .call(&object_value_of, Value::Bool(false), &[])
            .unwrap()
        else {
            panic!("Object.prototype.valueOf did not box Boolean primitive");
        };
        let Value::Object(second_wrapper) = context
            .call(&object_value_of, Value::Bool(false), &[])
            .unwrap()
        else {
            panic!("second Object.prototype.valueOf did not box Boolean primitive");
        };
        assert_ne!(first_wrapper, second_wrapper);
        assert_eq!(
            runtime.get_prototype_of(&first_wrapper).unwrap(),
            Some(boolean_prototype.clone())
        );
        assert_eq!(
            context
                .call(&boolean_value_of, Value::Object(first_wrapper), &[])
                .unwrap(),
            Value::Bool(false)
        );

        let tag_receiver = runtime.intern_property_key("tagReceiver").unwrap();
        assert!(
            context
                .define_own_property(
                    &context.global_object().unwrap(),
                    &tag_receiver,
                    &data_descriptor(Value::Undefined, true, true, true),
                )
                .unwrap()
        );
        let Value::Object(tag_getter) = context
            .eval(
                "(function(){ 'use strict'; tagReceiver = typeof this; return 'CustomBoolean'; })",
            )
            .unwrap()
        else {
            panic!("@@toStringTag probe did not produce a function");
        };
        let tag_getter = runtime.as_callable(&tag_getter).unwrap().unwrap();
        let to_string_tag =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
        assert!(
            context
                .define_own_property(
                    &boolean_prototype,
                    &to_string_tag,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(tag_getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(&object_to_string, Value::Bool(false), &[])
                .unwrap(),
            Value::String(JsString::from("[object CustomBoolean]"))
        );
        assert_eq!(
            context
                .get_property(&context.global_object().unwrap(), &tag_receiver)
                .unwrap(),
            Value::String(JsString::from("object"))
        );

        let locale_receiver = runtime.intern_property_key("localeReceiver").unwrap();
        assert!(
            context
                .define_own_property(
                    &context.global_object().unwrap(),
                    &locale_receiver,
                    &data_descriptor(Value::Undefined, true, true, true),
                )
                .unwrap()
        );
        let Value::Object(locale_method) = context
            .eval("(function(){ 'use strict'; return typeof this; })")
            .unwrap()
        else {
            panic!("toLocaleString method probe did not produce a function");
        };
        let locale_method = runtime.as_callable(&locale_method).unwrap().unwrap();
        let locale_method_key = runtime.intern_property_key("localeMethod").unwrap();
        assert!(
            context
                .define_own_property(
                    &context.global_object().unwrap(),
                    &locale_method_key,
                    &data_descriptor(
                        Value::Object(locale_method.as_object().clone()),
                        true,
                        true,
                        true,
                    ),
                )
                .unwrap()
        );
        let Value::Object(locale_getter) = context
            .eval(
                "(function(){ 'use strict'; localeReceiver = typeof this; return localeMethod; })",
            )
            .unwrap()
        else {
            panic!("toLocaleString getter probe did not produce a function");
        };
        let locale_getter = runtime.as_callable(&locale_getter).unwrap().unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        assert!(
            context
                .define_own_property(
                    &boolean_prototype,
                    &to_string_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(locale_getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(&object_to_locale_string, Value::Bool(false), &[])
                .unwrap(),
            Value::String(JsString::from("boolean"))
        );
        assert_eq!(
            context
                .get_property(&context.global_object().unwrap(), &locale_receiver)
                .unwrap(),
            Value::String(JsString::from("boolean"))
        );
    }

    #[test]
    fn boolean_wrapper_keeps_its_realm_graph_alive_until_collection() {
        let runtime = Runtime::new();
        let wrapper = {
            let mut context = runtime.new_context();
            let constructor = global_callable(&runtime, &mut context, "Boolean");
            let Value::Object(wrapper) = context
                .construct(&constructor, &[Value::Bool(true)])
                .unwrap()
            else {
                panic!("Boolean construction did not return a wrapper");
            };
            wrapper
        };
        runtime.run_gc().unwrap();
        assert_eq!(runtime.heap_counts().context_nodes, 1);
        drop(wrapper);
        runtime.run_gc().unwrap();
        assert_eq!(runtime.heap_counts().live, 0);
    }

    #[test]
    fn bytecode_is_rooted_and_calls_separate_caller_from_callee_realm() {
        let runtime = Runtime::new();
        let mut compiler_context = runtime.new_context();
        let compiler_realm = compiler_context.realm;
        let intrinsic_realm_roots = runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(compiler_realm)
            .unwrap();
        let function = compiler_context.compile("this").unwrap();
        let bytecode_id = function.bytecode_id();

        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode_strong_count(bytecode_id),
            Ok(1)
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(compiler_realm),
            Ok(intrinsic_realm_roots + 1)
        );
        let duplicate = function.clone();
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode_strong_count(bytecode_id),
            Ok(2)
        );
        drop(duplicate);

        let mut caller_context = runtime.new_context();
        let caller_global = caller_context.global_object().unwrap();
        drop(compiler_context);
        assert_eq!(runtime.heap_counts().context_nodes, 2);

        let snapshot = runtime.snapshot_function_bytecode(&function).unwrap();
        assert_eq!(snapshot.realm, compiler_realm);
        drop(snapshot);
        assert_eq!(
            caller_context.execute(&function).unwrap(),
            Value::Object(caller_global)
        );

        drop(function);
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        assert_eq!(runtime.heap_counts().context_nodes, 2);
        runtime.run_gc().unwrap();
        assert_eq!(runtime.heap_counts().context_nodes, 1);
    }

    #[test]
    fn publication_rejects_value_opcode_for_child_bytecode_before_heap_changes() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let child = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        let root = UnlinkedFunction::new(
            vec![Instruction::PushConst(0), Instruction::Return],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );

        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, root),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    #[test]
    fn publication_rejects_string_key_opcodes_with_non_string_constants() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        for (code, max_stack) in [
            (
                vec![
                    Instruction::Undefined,
                    Instruction::SetName(0),
                    Instruction::Return,
                ],
                1,
            ),
            (
                vec![
                    Instruction::Undefined,
                    Instruction::GetField(0),
                    Instruction::Return,
                ],
                1,
            ),
            (
                vec![
                    Instruction::Undefined,
                    Instruction::GetField2(0),
                    Instruction::Drop,
                    Instruction::Return,
                ],
                2,
            ),
            (
                vec![
                    Instruction::Undefined,
                    Instruction::Undefined,
                    Instruction::PutField(0),
                    Instruction::Undefined,
                    Instruction::Return,
                ],
                2,
            ),
        ] {
            let function = UnlinkedFunction::new(
                code,
                vec![UnlinkedConstant::primitive(Value::Int(1)).unwrap()],
                FunctionMetadata {
                    max_stack,
                    ..FunctionMetadata::default()
                },
            );
            assert!(matches!(
                runtime.publish_unlinked_function(context.realm, function),
                Err(RuntimeError::Engine(_))
            ));
            assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        }

        let child = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        let function = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::GetField(0),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    #[test]
    fn call_frame_loads_arguments_and_moves_values_through_locals() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::PutLocal(0),
                Instruction::GetLocal(0),
                Instruction::GetArg(1),
                Instruction::Add,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 2,
                local_count: 1,
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        );
        let function = runtime
            .publish_unlinked_function(context.realm, function)
            .unwrap();
        let callable = runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap();

        assert_eq!(
            context
                .call(
                    &callable,
                    Value::Undefined,
                    &[Value::Int(20), Value::Int(22)]
                )
                .unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn runtime_typeof_distinguishes_callable_and_ordinary_objects() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::TypeOf,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        let function = runtime
            .publish_unlinked_function(context.realm, function)
            .unwrap();
        let callable = runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap();
        let ordinary = runtime.new_object(None).unwrap();

        assert_eq!(
            context
                .call(
                    &callable,
                    Value::Undefined,
                    &[Value::Object(callable.as_object().clone())],
                )
                .unwrap(),
            Value::String(JsString::from("function"))
        );
        assert_eq!(
            context
                .call(&callable, Value::Undefined, &[Value::Object(ordinary)],)
                .unwrap(),
            Value::String(JsString::from("object"))
        );
    }

    #[test]
    fn ordinary_function_object_properties_match_quickjs_descriptors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context.eval("(function(a, b) {})").unwrap() else {
            panic!("function expression did not produce an object");
        };
        assert!(runtime.is_constructor(&function).unwrap());

        let name = runtime.intern_property_key("name").unwrap();
        let length = runtime.intern_property_key("length").unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let constructor = runtime.intern_property_key("constructor").unwrap();

        assert_eq!(
            runtime.own_property_keys(&function).unwrap(),
            vec![length.clone(), name.clone(), prototype_key.clone()]
        );

        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::String(name_value),
            writable: false,
            enumerable: false,
            configurable: true,
        } = runtime.get_own_property(&function, &name).unwrap().unwrap()
        else {
            panic!("unexpected function name descriptor");
        };
        assert!(name_value.is_empty());
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Int(2),
            writable: false,
            enumerable: false,
            configurable: true,
        } = runtime
            .get_own_property(&function, &length)
            .unwrap()
            .unwrap()
        else {
            panic!("unexpected function length descriptor");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(prototype),
            writable: true,
            enumerable: false,
            configurable: false,
        } = runtime
            .get_own_property(&function, &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("unexpected function prototype descriptor");
        };
        assert_eq!(
            context.get_property(&prototype, &constructor).unwrap(),
            Value::Object(function.clone())
        );
        assert_eq!(
            runtime.get_prototype_of(&prototype).unwrap().unwrap(),
            context.object_prototype().unwrap()
        );

        drop(prototype);
        drop(function);
        assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 2);
    }

    #[test]
    fn function_prototype_autoinit_preserves_keys_without_eager_object_cycle() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline_objects = runtime.heap_counts().object_nodes;
        let Value::Object(unread) = context.eval("(0, function(){})").unwrap() else {
            panic!("function expression did not produce an object");
        };
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
        drop(unread);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

        let Value::Object(function) = context.eval("(0, function(){})").unwrap() else {
            panic!("function expression did not produce an object");
        };
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);

        let length = runtime.intern_property_key("length").unwrap();
        let name = runtime.intern_property_key("name").unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        assert_eq!(
            runtime.own_property_keys(&function).unwrap(),
            vec![length, name, prototype_key.clone()]
        );
        assert!(runtime.has_own_property(&function, &prototype_key).unwrap());
        assert!(!runtime.delete_property(&function, &prototype_key).unwrap());
        assert!(
            !runtime
                .define_own_property(
                    &function,
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);

        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(prototype),
            writable: true,
            enumerable: false,
            configurable: false,
        } = runtime
            .get_own_property(&function, &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("prototype autoinit produced the wrong descriptor");
        };
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(second),
            ..
        } = runtime
            .get_own_property(&function, &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("second prototype read did not return an object");
        };
        assert_eq!(prototype, second);

        drop(second);
        drop(prototype);
        drop(function);
        assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 2);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
    }

    #[test]
    fn compatible_define_materializes_function_prototype_but_value_override_releases_it() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline_objects = runtime.heap_counts().object_nodes;
        let prototype_key = runtime.intern_property_key("prototype").unwrap();

        let Value::Object(empty_define) = context.eval("(0, function(){})").unwrap() else {
            panic!("function expression did not produce an object");
        };
        assert!(
            runtime
                .define_own_property(
                    &empty_define,
                    &prototype_key,
                    &OrdinaryPropertyDescriptor::new(),
                )
                .unwrap()
        );
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
        drop(empty_define);
        runtime.run_gc().unwrap();
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

        let Value::Object(value_define) = context.eval("(0, function(){})").unwrap() else {
            panic!("function expression did not produce an object");
        };
        assert!(
            runtime
                .define_own_property(
                    &value_define,
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 1);
        assert!(matches!(
            runtime
                .get_own_property(&value_define, &prototype_key)
                .unwrap()
                .unwrap(),
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(1),
                writable: true,
                enumerable: false,
                configurable: false,
            }
        ));
    }

    #[test]
    fn autoinit_define_checks_lazy_flags_before_materializing_and_retries() {
        let runtime = Runtime::new();
        let call_key = runtime.intern_property_key("call").unwrap();

        let configurable_context = runtime.new_context();
        let configurable_fp = configurable_context.function_prototype().unwrap();
        assert!(
            runtime
                .is_auto_init_own_property(&configurable_fp, &call_key)
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(
                    &configurable_fp,
                    &call_key,
                    &OrdinaryPropertyDescriptor {
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(
            !runtime
                .is_auto_init_own_property(&configurable_fp, &call_key)
                .unwrap()
        );
        assert!(matches!(
            runtime
                .get_own_property(&configurable_fp, &call_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: false,
                configurable: true,
                ..
            })
        ));

        let enumerable_context = runtime.new_context();
        let enumerable_fp = enumerable_context.function_prototype().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &enumerable_fp,
                    &call_key,
                    &OrdinaryPropertyDescriptor {
                        enumerable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(matches!(
            runtime.get_own_property(&enumerable_fp, &call_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: true,
                configurable: true,
                ..
            })
        ));

        let mut accessor_context = runtime.new_context();
        let accessor_fp = accessor_context.function_prototype().unwrap();
        let Value::Object(getter) = accessor_context
            .eval("(function replacementCall(){ return 7; })")
            .unwrap()
        else {
            panic!("replacement getter was not an object");
        };
        let getter = runtime.as_callable(&getter).unwrap().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &accessor_fp,
                    &call_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter.clone())),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(matches!(
            runtime.get_own_property(&accessor_fp, &call_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Accessor {
                get: Some(ref actual),
                set: None,
                enumerable: false,
                configurable: true,
            }) if actual == &getter
        ));
        assert_eq!(
            accessor_context
                .get_property(&accessor_fp, &call_key)
                .unwrap(),
            Value::Int(7)
        );

        let has_instance_context = runtime.new_context();
        let has_instance_fp = has_instance_context.function_prototype().unwrap();
        let has_instance_key =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
        assert!(
            runtime
                .is_auto_init_own_property(&has_instance_fp, &has_instance_key)
                .unwrap()
        );
        assert!(
            !runtime
                .define_own_property(
                    &has_instance_fp,
                    &has_instance_key,
                    &OrdinaryPropertyDescriptor {
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(
            runtime
                .is_auto_init_own_property(&has_instance_fp, &has_instance_key)
                .unwrap()
        );
    }

    #[test]
    fn failed_autoinit_commits_undefined_and_releases_initializer_realm() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = context.new_object().unwrap();
        let key = runtime.intern_property_key("failureProbe").unwrap();
        let before = runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm)
            .unwrap();
        runtime
            .define_failure_auto_init(&object, context.realm, "failureProbe")
            .unwrap();
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(context.realm),
            Ok(before + 1)
        );

        assert!(matches!(
            runtime.get_own_property(&object, &key),
            Err(RuntimeError::Invariant("autoinit failure probe"))
        ));
        assert!(!runtime.is_auto_init_own_property(&object, &key).unwrap());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(context.realm),
            Ok(before)
        );
        assert!(matches!(
            runtime.get_own_property(&object, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Undefined,
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));
    }

    #[test]
    fn function_prototype_autoinit_owns_and_uses_closure_creation_realm() {
        let runtime = Runtime::new();
        let compiler_context = runtime.new_context();
        let creation_context = runtime.new_context();
        let creation_realm = creation_context.realm;
        let function = runtime
            .publish_unlinked_function(
                compiler_context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        has_prototype: true,
                        constructor_kind: ConstructorKind::Base,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let callable = runtime
            .new_bytecode_closure(creation_realm, &function)
            .unwrap();
        drop(creation_context);
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context(creation_realm)
                .is_ok()
        );

        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(prototype),
            ..
        } = runtime
            .get_own_property(callable.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("prototype autoinit did not materialize an object");
        };
        let creation_object_prototype = runtime
            .0
            .state
            .borrow()
            .heap
            .context(creation_realm)
            .unwrap()
            .object_prototype;
        assert_eq!(
            runtime
                .get_prototype_of(&prototype)
                .unwrap()
                .unwrap()
                .object_id(),
            creation_object_prototype
        );

        drop(prototype);
        drop(callable);
        drop(function);
        runtime.run_gc().unwrap();
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context(creation_realm)
                .is_err()
        );
    }

    #[test]
    fn base_construct_uses_explicit_new_target_prototype_and_realm_fallback() {
        let runtime = Runtime::new();
        let mut constructor_context = runtime.new_context();
        let mut target_context = runtime.new_context();

        let Value::Object(constructor_object) = constructor_context
            .eval("(0, function(){ return 1; })")
            .unwrap()
        else {
            panic!("constructor source did not produce an object");
        };
        let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
        let Value::Object(target_object) = target_context.eval("(0, function(){})").unwrap() else {
            panic!("new-target source did not produce an object");
        };
        let new_target = runtime.as_callable(&target_object).unwrap().unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();

        let explicit_prototype = target_context.new_object().unwrap();
        assert!(
            target_context
                .define_own_property(
                    &target_object,
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Object(explicit_prototype.clone())),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(instance) = constructor_context
            .construct_with_new_target(&constructor, &new_target, &[])
            .unwrap()
        else {
            panic!("base constructor did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&instance).unwrap(),
            Some(explicit_prototype)
        );

        assert!(
            target_context
                .define_own_property(
                    &target_object,
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Null),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(fallback_instance) = constructor_context
            .construct_with_new_target(&constructor, &new_target, &[])
            .unwrap()
        else {
            panic!("base constructor fallback did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&fallback_instance).unwrap(),
            Some(target_context.object_prototype().unwrap())
        );
        assert_ne!(
            runtime.get_prototype_of(&fallback_instance).unwrap(),
            Some(constructor_context.object_prototype().unwrap())
        );
    }

    #[test]
    fn new_target_prototype_getter_throw_short_circuits_constructor_body() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(constructor_object) = context
            .eval("(0, function(){ return function(){}; })")
            .unwrap()
        else {
            panic!("constructor source did not produce an object");
        };
        let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
        let new_target = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::Undefined, Instruction::Return],
            FunctionMetadata {
                max_stack: 1,
                constructor_kind: ConstructorKind::Base,
                ..FunctionMetadata::default()
            },
        );
        let Value::Object(getter_object) = context.eval("(0, function(){ throw 9; })").unwrap()
        else {
            panic!("getter source did not produce an object");
        };
        let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
        let prototype = runtime.intern_property_key("prototype").unwrap();
        assert!(
            context
                .define_own_property(
                    new_target.as_object(),
                    &prototype,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );

        assert_eq!(
            context.construct_with_new_target(&constructor, &new_target, &[]),
            Err(RuntimeError::Exception)
        );
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
    }

    #[test]
    fn construct_rejects_non_constructor_callable_with_caller_realm_type_error() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let callable = runtime.as_callable(&function_prototype).unwrap().unwrap();

        assert_eq!(
            context.construct(&callable, &[]),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("construct failure did not materialize TypeError");
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from("TypeError"))
        );
        assert_eq!(
            context.get_property(&error, &message).unwrap(),
            Value::String(JsString::from(" is not a constructor"))
        );
    }

    #[test]
    fn function_prototype_is_callable_non_constructable_and_has_no_prototype_property() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let callable = runtime
            .callable_from_value(Value::Object(function_prototype.clone()))
            .unwrap();
        assert_eq!(
            context
                .call(&callable, Value::Undefined, &[Value::Int(1)])
                .unwrap(),
            Value::Undefined
        );
        assert!(!runtime.is_constructor(&function_prototype).unwrap());
        assert_eq!(
            runtime
                .get_prototype_of(&function_prototype)
                .unwrap()
                .unwrap(),
            context.object_prototype().unwrap()
        );

        let name = runtime.intern_property_key("name").unwrap();
        let length = runtime.intern_property_key("length").unwrap();
        let caller = runtime.intern_property_key("caller").unwrap();
        let arguments = runtime.intern_property_key("arguments").unwrap();
        let call = runtime.intern_property_key("call").unwrap();
        let apply = runtime.intern_property_key("apply").unwrap();
        let bind = runtime.intern_property_key("bind").unwrap();
        let to_string = runtime.intern_property_key("toString").unwrap();
        let file_name = runtime.intern_property_key("fileName").unwrap();
        let line_number = runtime.intern_property_key("lineNumber").unwrap();
        let column_number = runtime.intern_property_key("columnNumber").unwrap();
        let constructor = runtime.intern_property_key("constructor").unwrap();
        let has_instance =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
        let prototype = runtime.intern_property_key("prototype").unwrap();
        assert_eq!(
            runtime.own_property_keys(&function_prototype).unwrap(),
            vec![
                length.clone(),
                name.clone(),
                caller,
                arguments,
                call,
                apply,
                bind,
                to_string,
                file_name,
                line_number,
                column_number,
                constructor,
                has_instance,
            ]
        );
        assert!(matches!(
            runtime
                .get_own_property(&function_prototype, &name)
                .unwrap()
                .unwrap(),
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            } if value.is_empty()
        ));
        assert!(matches!(
            runtime
                .get_own_property(&function_prototype, &length)
                .unwrap()
                .unwrap(),
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(0),
                writable: false,
                enumerable: false,
                configurable: true,
            }
        ));
        assert_eq!(
            runtime
                .get_own_property(&function_prototype, &prototype)
                .unwrap(),
            None
        );
    }

    #[test]
    fn function_constructor_intrinsic_and_dynamic_source_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context.function_constructor().unwrap();
        let function_prototype = context.function_prototype().unwrap();
        let global = context.global_object().unwrap();
        let length = runtime.intern_property_key("length").unwrap();
        let name = runtime.intern_property_key("name").unwrap();
        let prototype = runtime.intern_property_key("prototype").unwrap();
        let constructor_key = runtime.intern_property_key("constructor").unwrap();
        let function_key = runtime.intern_property_key("Function").unwrap();

        assert!(runtime.is_constructor(constructor.as_object()).unwrap());
        assert_eq!(runtime.callable_realm(&constructor).unwrap(), context.realm);
        assert_eq!(
            runtime.get_prototype_of(constructor.as_object()).unwrap(),
            Some(function_prototype.clone())
        );
        assert_eq!(
            runtime.own_property_keys(constructor.as_object()).unwrap(),
            vec![length.clone(), name.clone(), prototype.clone()]
        );
        assert!(matches!(
            runtime
                .get_own_property(constructor.as_object(), &length)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(1),
                writable: false,
                enumerable: false,
                configurable: true,
            })
        ));
        assert!(matches!(
            runtime
                .get_own_property(constructor.as_object(), &name)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: false,
                configurable: true,
            }) if value == JsString::from("Function")
        ));
        assert!(matches!(
            runtime
                .get_own_property(constructor.as_object(), &prototype)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(value),
                writable: false,
                enumerable: false,
                configurable: false,
            }) if value == function_prototype
        ));
        assert!(matches!(
            runtime.get_own_property(&global, &function_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(value),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if value == constructor.as_object().clone()
        ));
        assert!(matches!(
            runtime
                .get_own_property(&function_prototype, &constructor_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(value),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if value == constructor.as_object().clone()
        ));
        {
            let state = runtime.0.state.borrow();
            let ObjectPayload::NativeFunction { data } = &state
                .heap
                .object(constructor.as_object().object_id())
                .unwrap()
                .payload
            else {
                panic!("Function was not a native constructor");
            };
            assert_eq!(
                data.target,
                NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal)
            );
            assert_eq!(
                data.target.descriptor().cproto,
                NativeCProto::ConstructorOrFunctionMagic
            );
            assert_eq!(data.min_readable_args, 1);
        }

        let to_string = runtime.intern_property_key("toString").unwrap();
        let Value::Object(to_string) = context
            .get_property(&function_prototype, &to_string)
            .unwrap()
        else {
            panic!("Function.prototype.toString was not callable");
        };
        let to_string = runtime.as_callable(&to_string).unwrap().unwrap();
        assert_eq!(
            context
                .call(
                    &to_string,
                    Value::Object(constructor.as_object().clone()),
                    &[],
                )
                .unwrap(),
            Value::String(JsString::from(
                "function Function() {\n    [native code]\n}"
            ))
        );

        let Value::Object(empty) = context.call(&constructor, Value::Null, &[]).unwrap() else {
            panic!("Function() did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&empty).unwrap(),
            Some(function_prototype)
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(empty.clone()), &[])
                .unwrap(),
            Value::String(JsString::from("function anonymous(\n) {\n\n}"))
        );
        for (property_name, expected) in [
            ("name", Value::String(JsString::from("anonymous"))),
            ("length", Value::Int(0)),
            ("fileName", Value::String(JsString::from("<input>"))),
            ("lineNumber", Value::Int(1)),
            ("columnNumber", Value::Int(2)),
        ] {
            let key = runtime.intern_property_key(property_name).unwrap();
            assert_eq!(context.get_property(&empty, &key).unwrap(), expected);
        }

        let Value::Object(add) = context
            .call(
                &constructor,
                Value::Undefined,
                &[
                    Value::String(JsString::from("a")),
                    Value::String(JsString::from("b")),
                    Value::String(JsString::from("return a + b")),
                ],
            )
            .unwrap()
        else {
            panic!("Function parameters did not produce an object");
        };
        let add_callable = runtime.as_callable(&add).unwrap().unwrap();
        assert_eq!(
            context
                .call(
                    &add_callable,
                    Value::Undefined,
                    &[Value::Int(20), Value::Int(22)],
                )
                .unwrap(),
            Value::Int(42)
        );
        assert_eq!(
            context.call(&to_string, Value::Object(add), &[]).unwrap(),
            Value::String(JsString::from(
                "function anonymous(a,b\n) {\nreturn a + b\n}"
            ))
        );

        let Value::Object(duplicate) = context
            .call(
                &constructor,
                Value::Undefined,
                &[
                    Value::String(JsString::from("a")),
                    Value::String(JsString::from("a")),
                    Value::String(JsString::from("return a")),
                ],
            )
            .unwrap()
        else {
            panic!("sloppy duplicate parameters were rejected");
        };
        let duplicate = runtime.as_callable(&duplicate).unwrap().unwrap();
        assert_eq!(
            context
                .call(
                    &duplicate,
                    Value::Undefined,
                    &[Value::Int(1), Value::Int(2)],
                )
                .unwrap(),
            Value::Int(2)
        );

        assert_eq!(
            context.call(
                &constructor,
                Value::Undefined,
                &[
                    Value::String(JsString::from("a")),
                    Value::String(JsString::from("a")),
                    Value::String(JsString::from("\"use strict\"; return a")),
                ],
            ),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("strict duplicate parameters did not throw an Error");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from("SyntaxError"))
        );

        assert_eq!(
            context.call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from_utf16([0xd800]))],
            ),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("lone-surrogate source did not throw an Error");
        };
        assert_eq!(
            context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from("InternalError"))
        );
    }

    #[test]
    fn function_constructor_uses_defining_realm_and_new_target_prototype() {
        let runtime = Runtime::new();
        let first = runtime.new_context();
        let mut second = runtime.new_context();
        let constructor = first.function_constructor().unwrap();
        let first_function_prototype = first.function_prototype().unwrap();
        let second_function_prototype = second.function_prototype().unwrap();
        let first_object_prototype = first.object_prototype().unwrap();
        let marker = runtime.intern_property_key("dynamicRealmMarker").unwrap();
        runtime
            .define_function_data_property(
                &first.global_object().unwrap(),
                "dynamicRealmMarker",
                Value::Int(11),
                true,
                true,
            )
            .unwrap();
        runtime
            .define_function_data_property(
                &second.global_object().unwrap(),
                "dynamicRealmMarker",
                Value::Int(22),
                true,
                true,
            )
            .unwrap();
        assert_eq!(
            second
                .get_property(&first.global_object().unwrap(), &marker)
                .unwrap(),
            Value::Int(11)
        );

        let Value::Object(dynamic) = second
            .call(
                &constructor,
                Value::Object(second.global_object().unwrap()),
                &[Value::String(JsString::from("return dynamicRealmMarker"))],
            )
            .unwrap()
        else {
            panic!("cross-realm Function call did not return an object");
        };
        let dynamic_callable = runtime.as_callable(&dynamic).unwrap().unwrap();
        assert_eq!(
            runtime.callable_realm(&dynamic_callable).unwrap(),
            first.realm
        );
        assert_eq!(
            runtime.get_prototype_of(&dynamic).unwrap(),
            Some(first_function_prototype.clone())
        );
        assert_eq!(
            second
                .call(&dynamic_callable, Value::Undefined, &[])
                .unwrap(),
            Value::Int(11)
        );

        let Value::Object(new_target) = second.eval("(function NewTarget(){})").unwrap() else {
            panic!("newTarget source did not return a function");
        };
        let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let custom_prototype = second.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    new_target.as_object(),
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Object(custom_prototype.clone())),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(customized) = second
            .construct_with_new_target(
                &constructor,
                &new_target,
                &[Value::String(JsString::from("return dynamicRealmMarker"))],
            )
            .unwrap()
        else {
            panic!("custom newTarget did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&customized).unwrap(),
            Some(custom_prototype)
        );
        let customized_callable = runtime.as_callable(&customized).unwrap().unwrap();
        assert_eq!(
            second
                .call(&customized_callable, Value::Undefined, &[])
                .unwrap(),
            Value::Int(11)
        );
        let Value::Object(instance_prototype) =
            second.get_property(&customized, &prototype_key).unwrap()
        else {
            panic!("dynamic function prototype was not an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&instance_prototype).unwrap(),
            Some(first_object_prototype)
        );

        assert!(
            runtime
                .define_own_property(
                    new_target.as_object(),
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(1)),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(fallback) = second
            .construct_with_new_target(
                &constructor,
                &new_target,
                &[Value::String(JsString::from("return 3"))],
            )
            .unwrap()
        else {
            panic!("fallback newTarget did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&fallback).unwrap(),
            Some(second_function_prototype)
        );
        assert_eq!(runtime.callable_realm(&constructor).unwrap(), first.realm);
    }

    #[test]
    fn function_constructor_samples_strip_mode_and_keeps_parse_locations() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context.function_constructor().unwrap();
        let function_prototype = context.function_prototype().unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let Value::Object(to_string) = context
            .get_property(&function_prototype, &to_string_key)
            .unwrap()
        else {
            panic!("Function.prototype.toString was not an object");
        };
        let to_string = runtime.as_callable(&to_string).unwrap().unwrap();
        let keys = ["fileName", "lineNumber", "columnNumber"]
            .map(|name| runtime.intern_property_key(name).unwrap());

        runtime.set_debug_info_mode(DebugInfoMode::StripSource);
        let Value::Object(source_stripped) = context
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from("return 1"))],
            )
            .unwrap()
        else {
            panic!("strip-source Function did not return an object");
        };
        for (key, expected) in keys.iter().zip([
            Value::String(JsString::from("<input>")),
            Value::Int(1),
            Value::Int(2),
        ]) {
            assert_eq!(
                context.get_property(&source_stripped, key).unwrap(),
                expected
            );
        }
        assert_eq!(
            context
                .call(&to_string, Value::Object(source_stripped.clone()), &[])
                .unwrap(),
            Value::String(JsString::from(
                "function anonymous() {\n    [native code]\n}"
            ))
        );
        let name = runtime.intern_property_key("name").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &source_stripped,
                    &name,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::String(JsString::from("renamed"))),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(source_stripped), &[])
                .unwrap(),
            Value::String(JsString::from("function renamed() {\n    [native code]\n}"))
        );

        runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
        let Value::Object(debug_stripped) = context
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from("return 2"))],
            )
            .unwrap()
        else {
            panic!("strip-debug Function did not return an object");
        };
        for key in &keys {
            assert_eq!(
                context.get_property(&debug_stripped, key).unwrap(),
                Value::Undefined
            );
        }
        assert_eq!(
            context
                .call(&to_string, Value::Object(debug_stripped), &[])
                .unwrap(),
            Value::String(JsString::from(
                "function anonymous() {\n    [native code]\n}"
            ))
        );

        assert_eq!(
            context.call(
                &constructor,
                Value::Undefined,
                &[
                    Value::String(JsString::from("a-")),
                    Value::String(JsString::from("return 1")),
                ],
            ),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("malformed Function did not throw an Error");
        };
        for (name, expected) in [
            ("fileName", Value::String(JsString::from("<input>"))),
            ("lineNumber", Value::Int(1)),
            ("columnNumber", Value::Int(22)),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            assert_eq!(context.get_property(&error, &key).unwrap(), expected);
        }
        let stack = runtime.intern_property_key("stack").unwrap();
        let Value::String(stack) = context.get_property(&error, &stack).unwrap() else {
            panic!("Function syntax error had no stack");
        };
        assert_eq!(
            stack,
            JsString::from("    at <input>:1:22\n    at Function (native)\n")
        );
    }

    #[test]
    fn function_constructor_orders_source_conversion_parse_and_prototype_get() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context.function_constructor().unwrap();
        let global = context.global_object().unwrap();
        runtime
            .define_function_data_property(
                &global,
                "functionOrder",
                Value::String(JsString::from("")),
                true,
                true,
            )
            .unwrap();
        let custom_prototype = context.new_object().unwrap();
        runtime
            .define_function_data_property(
                &global,
                "functionCustomPrototype",
                Value::Object(custom_prototype.clone()),
                true,
                true,
            )
            .unwrap();

        let (
            parameter_to_string,
            body_to_string,
            bad_parameter_to_string,
            throwing_to_string,
            prototype_getter,
        ) = {
            let mut eval_callable = |source: &str| {
                let Value::Object(function) = context.eval(source).unwrap() else {
                    panic!("conversion helper was not a function");
                };
                runtime.as_callable(&function).unwrap().unwrap()
            };
            (
                eval_callable(
                    "(function(){ functionOrder = functionOrder + \"p\"; return \"a\"; })",
                ),
                eval_callable(
                    "(function(){ functionOrder = functionOrder + \"b\"; return \"return a\"; })",
                ),
                eval_callable(
                    "(function(){ functionOrder = functionOrder + \"p\"; return \"a-\"; })",
                ),
                eval_callable(
                    "(function(){ functionOrder = functionOrder + \"t\"; throw \"stop\"; })",
                ),
                eval_callable(
                    "(function(){ functionOrder = functionOrder + \"x\"; return functionCustomPrototype; })",
                ),
            )
        };

        let to_string = runtime.intern_property_key("toString").unwrap();
        let parameter = context.new_object().unwrap();
        let body = context.new_object().unwrap();
        runtime
            .define_function_data_property(
                &parameter,
                "toString",
                Value::Object(parameter_to_string.as_object().clone()),
                true,
                true,
            )
            .unwrap();
        runtime
            .define_function_data_property(
                &body,
                "toString",
                Value::Object(body_to_string.as_object().clone()),
                true,
                true,
            )
            .unwrap();
        assert!(runtime.has_own_property(&parameter, &to_string).unwrap());

        let function_prototype = context.function_prototype().unwrap();
        let bind_key = runtime.intern_property_key("bind").unwrap();
        let Value::Object(bind) = context
            .get_property(&function_prototype, &bind_key)
            .unwrap()
        else {
            panic!("Function.prototype.bind was not an object");
        };
        let bind = runtime.as_callable(&bind).unwrap().unwrap();
        let Value::Object(new_target) = context
            .call(
                &bind,
                Value::Object(constructor.as_object().clone()),
                &[Value::Undefined],
            )
            .unwrap()
        else {
            panic!("bound Function was not an object");
        };
        let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
        let prototype = runtime.intern_property_key("prototype").unwrap();
        assert!(
            runtime
                .define_own_property(
                    new_target.as_object(),
                    &prototype,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(prototype_getter)),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(false),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );

        let Value::Object(function) = context
            .construct_with_new_target(
                &constructor,
                &new_target,
                &[
                    Value::Object(parameter.clone()),
                    Value::Object(body.clone()),
                ],
            )
            .unwrap()
        else {
            panic!("converted Function source did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&function).unwrap(),
            Some(custom_prototype)
        );
        let order = runtime.intern_property_key("functionOrder").unwrap();
        assert_eq!(
            context.get_property(&global, &order).unwrap(),
            Value::String(JsString::from("pbx"))
        );

        runtime
            .define_function_data_property(
                &global,
                "functionOrder",
                Value::String(JsString::from("")),
                true,
                true,
            )
            .unwrap();
        runtime
            .define_function_data_property(
                &parameter,
                "toString",
                Value::Object(bad_parameter_to_string.as_object().clone()),
                true,
                true,
            )
            .unwrap();
        assert_eq!(
            context.construct_with_new_target(
                &constructor,
                &new_target,
                &[
                    Value::Object(parameter.clone()),
                    Value::Object(body.clone())
                ],
            ),
            Err(RuntimeError::Exception)
        );
        drop(context.take_exception().unwrap());
        assert_eq!(
            context.get_property(&global, &order).unwrap(),
            Value::String(JsString::from("pb"))
        );

        runtime
            .define_function_data_property(
                &global,
                "functionOrder",
                Value::String(JsString::from("")),
                true,
                true,
            )
            .unwrap();
        runtime
            .define_function_data_property(
                &parameter,
                "toString",
                Value::Object(throwing_to_string.as_object().clone()),
                true,
                true,
            )
            .unwrap();
        assert_eq!(
            context.call(
                &constructor,
                Value::Undefined,
                &[Value::Object(parameter), Value::Object(body)],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            context.take_exception().unwrap(),
            Some(Value::String(JsString::from("stop")))
        );
        assert_eq!(
            context.get_property(&global, &order).unwrap(),
            Value::String(JsString::from("t"))
        );
    }

    #[test]
    fn function_constructor_typed_realm_root_and_cycles_are_collectable() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let realm = context.realm;
        let constructor = context.function_constructor().unwrap();
        let function_prototype = context.function_prototype().unwrap();
        let global = context.global_object().unwrap();
        let function_key = runtime.intern_property_key("Function").unwrap();
        let constructor_key = runtime.intern_property_key("constructor").unwrap();

        assert!(runtime.delete_property(&global, &function_key).unwrap());
        assert!(
            runtime
                .delete_property(&function_prototype, &constructor_key)
                .unwrap()
        );
        drop(constructor);
        let rooted = context.function_constructor().unwrap();
        assert!(runtime.is_constructor(rooted.as_object()).unwrap());
        assert!(matches!(
            runtime.bytecode_for_callable(&rooted).unwrap(),
            CallableExecution::Native {
                target: NativeFunctionId::FunctionConstructor(DynamicFunctionKind::Normal),
                realm: target_realm,
                min_readable_args: 1,
            } if target_realm == realm
        ));

        drop(rooted);
        drop(function_key);
        drop(constructor_key);
        drop(global);
        drop(function_prototype);
        drop(context);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.context(realm).is_err());
        let counts = runtime.heap_counts();
        assert_eq!(counts.context_nodes, 0);
        assert_eq!(counts.object_nodes, 0);
        assert_eq!(counts.shape_nodes, 0);
        assert_eq!(counts.function_bytecode_nodes, 0);
    }

    #[test]
    fn dynamic_function_keeps_its_defining_realm_alive() {
        let runtime = Runtime::new();
        let mut defining = runtime.new_context();
        let defining_realm = defining.realm;
        let constructor = defining.function_constructor().unwrap();
        let Value::Object(function_object) = defining
            .call(
                &constructor,
                Value::Undefined,
                &[Value::String(JsString::from("return 9"))],
            )
            .unwrap()
        else {
            panic!("Function did not return a bytecode function");
        };
        let function = runtime.as_callable(&function_object).unwrap().unwrap();
        drop(function_object);
        let mut caller = runtime.new_context();

        drop(constructor);
        drop(defining);
        runtime.run_gc().unwrap();
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context(defining_realm)
                .is_ok()
        );
        assert_eq!(
            caller.call(&function, Value::Undefined, &[]).unwrap(),
            Value::Int(9)
        );

        drop(function);
        runtime.run_gc().unwrap();
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context(defining_realm)
                .is_err()
        );
        assert_eq!(runtime.heap_counts().context_nodes, 1);
    }

    #[test]
    fn function_constructor_failure_paths_release_temporary_graphs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let constructor = context.function_constructor().unwrap();
        let live_counts = || {
            let counts = runtime.heap_counts();
            (
                counts.object_nodes,
                counts.shape_nodes,
                counts.var_ref_nodes,
                counts.context_nodes,
                counts.function_bytecode_nodes,
            )
        };

        let parse_baseline = live_counts();
        let parse_atom_baseline = runtime.test_atom_count();
        for _ in 0..3 {
            assert_eq!(
                context.call(
                    &constructor,
                    Value::Undefined,
                    &[
                        Value::String(JsString::from("a-")),
                        Value::String(JsString::from("return 1")),
                    ],
                ),
                Err(RuntimeError::Exception)
            );
            drop(context.take_exception().unwrap());
            runtime.run_gc().unwrap();
            assert_eq!(live_counts(), parse_baseline);
            assert_eq!(runtime.test_atom_count(), parse_atom_baseline);
        }

        let function_prototype = context.function_prototype().unwrap();
        let bind_key = runtime.intern_property_key("bind").unwrap();
        let Value::Object(bind) = context
            .get_property(&function_prototype, &bind_key)
            .unwrap()
        else {
            panic!("Function.prototype.bind was not an object");
        };
        let bind = runtime.as_callable(&bind).unwrap().unwrap();
        let Value::Object(new_target) = context
            .call(
                &bind,
                Value::Object(constructor.as_object().clone()),
                &[Value::Undefined],
            )
            .unwrap()
        else {
            panic!("bound Function was not an object");
        };
        let new_target = runtime.as_callable(&new_target).unwrap().unwrap();
        let Value::Object(getter) = context
            .eval("(function(){ throw \"prototype\"; })")
            .unwrap()
        else {
            panic!("prototype getter was not an object");
        };
        let getter = runtime.as_callable(&getter).unwrap().unwrap();
        let prototype = runtime.intern_property_key("prototype").unwrap();
        assert!(
            runtime
                .define_own_property(
                    new_target.as_object(),
                    &prototype,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(false),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        runtime.run_gc().unwrap();
        let getter_baseline = live_counts();
        let getter_atom_baseline = runtime.test_atom_count();
        for _ in 0..3 {
            assert_eq!(
                context.construct_with_new_target(
                    &constructor,
                    &new_target,
                    &[Value::String(JsString::from("return 1"))],
                ),
                Err(RuntimeError::Exception)
            );
            assert_eq!(
                context.take_exception().unwrap(),
                Some(Value::String(JsString::from("prototype")))
            );
            runtime.run_gc().unwrap();
            assert_eq!(live_counts(), getter_baseline);
            assert_eq!(runtime.test_atom_count(), getter_atom_baseline);
        }
    }

    #[test]
    fn function_debug_accessors_match_quickjs_descriptors_realms_and_receivers() {
        let runtime = Runtime::new();
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let function_prototype = first.function_prototype().unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let length_key = runtime.intern_property_key("length").unwrap();
        let name_key = runtime.intern_property_key("name").unwrap();
        let specs = [
            (
                "fileName",
                "get fileName",
                NativeFunctionId::FunctionPrototypeFileName,
                NativeCProto::Getter,
            ),
            (
                "lineNumber",
                "get lineNumber",
                NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Line),
                NativeCProto::GetterMagic,
            ),
            (
                "columnNumber",
                "get columnNumber",
                NativeFunctionId::FunctionPrototypePosition(FunctionDebugPosition::Column),
                NativeCProto::GetterMagic,
            ),
        ];
        let mut getters = Vec::new();
        for (property_name, getter_name, target, cproto) in specs {
            let key = runtime.intern_property_key(property_name).unwrap();
            let CompleteOrdinaryPropertyDescriptor::Accessor {
                get: Some(getter),
                set: None,
                enumerable: false,
                configurable: true,
            } = runtime
                .get_own_property(&function_prototype, &key)
                .unwrap()
                .unwrap()
            else {
                panic!("{property_name} was not the expected getter-only accessor");
            };
            assert_eq!(
                runtime.get_prototype_of(getter.as_object()).unwrap(),
                Some(function_prototype.clone())
            );
            assert_eq!(runtime.callable_realm(&getter).unwrap(), first.realm);
            assert!(!runtime.is_constructor(getter.as_object()).unwrap());
            assert_eq!(
                runtime
                    .get_own_property(getter.as_object(), &prototype_key)
                    .unwrap(),
                None
            );
            assert!(matches!(
                runtime
                    .get_own_property(getter.as_object(), &length_key)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: Value::Int(0),
                    writable: false,
                    enumerable: false,
                    configurable: true,
                })
            ));
            assert!(matches!(
                runtime
                    .get_own_property(getter.as_object(), &name_key)
                    .unwrap(),
                Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: Value::String(value),
                    writable: false,
                    enumerable: false,
                    configurable: true,
                }) if value == JsString::from(getter_name)
            ));
            let state = runtime.0.state.borrow();
            let ObjectPayload::NativeFunction { data } = &state
                .heap
                .object(getter.as_object().object_id())
                .unwrap()
                .payload
            else {
                panic!("debug accessor getter was not a native function");
            };
            assert_eq!(data.target, target);
            assert_eq!(data.target.descriptor().cproto, cproto);
            drop(state);
            getters.push((key, getter));
        }
        assert_ne!(getters[0].1.as_object(), getters[1].1.as_object());
        assert_ne!(getters[1].1.as_object(), getters[2].1.as_object());

        let source = "\n  (function named(){})";
        let Value::Object(function) = second.eval_with_filename(source, "receiver.js").unwrap()
        else {
            panic!("debug receiver source did not return a function");
        };
        for (index, expected) in [
            Value::String(JsString::from("receiver.js")),
            Value::Int(2),
            Value::Int(4),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                first.get_property(&function, &getters[index].0).unwrap(),
                expected
            );
            assert_eq!(
                first
                    .call(
                        &getters[index].1,
                        Value::Object(function.clone()),
                        &[Value::Int(1), Value::Int(2)],
                    )
                    .unwrap(),
                expected,
                "getter ABI must ignore arguments and inspect the receiver"
            );
        }

        let bind_key = runtime.intern_property_key("bind").unwrap();
        let Value::Object(bind_object) =
            first.get_property(&function_prototype, &bind_key).unwrap()
        else {
            panic!("Function.prototype.bind was not an object");
        };
        let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
        let Value::Object(bound) = first
            .call(&bind, Value::Object(function.clone()), &[Value::Null])
            .unwrap()
        else {
            panic!("bind did not return an object");
        };
        let ordinary = first.new_object().unwrap();
        for (_, getter) in &getters {
            for receiver in [
                Value::Undefined,
                Value::Null,
                Value::Int(1),
                Value::Object(ordinary.clone()),
                Value::Object(function_prototype.clone()),
                Value::Object(bound.clone()),
                Value::Object(getter.as_object().clone()),
            ] {
                assert_eq!(first.call(getter, receiver, &[]).unwrap(), Value::Undefined);
            }
        }

        // If an embedder enables the constructor bit, QuickJS's getter cproto
        // receives newTarget as its receiver.
        let target = runtime.as_callable(&function).unwrap().unwrap();
        runtime
            .set_constructor_bit(getters[0].1.as_object(), true)
            .unwrap();
        assert_eq!(
            first
                .construct_with_new_target(&getters[0].1, &target, &[])
                .unwrap(),
            Value::String(JsString::from("receiver.js"))
        );
        runtime
            .set_constructor_bit(getters[0].1.as_object(), false)
            .unwrap();
    }

    #[test]
    fn runtime_debug_info_mode_matches_quickjs_strip_source_and_strip_debug() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let keys = ["fileName", "lineNumber", "columnNumber"]
            .map(|name| runtime.intern_property_key(name).unwrap());
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let Value::Object(to_string_object) = context
            .get_property(&function_prototype, &to_string_key)
            .unwrap()
        else {
            panic!("Function.prototype.toString was not an object");
        };
        let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();
        let expression = "\n  (function stripped() {})";

        let Value::Object(full) = context.eval_with_filename(expression, "full.js").unwrap() else {
            panic!("full debug compile did not return a function");
        };
        assert_eq!(runtime.debug_info_mode(), DebugInfoMode::Full);
        assert_eq!(
            context
                .call(&to_string, Value::Object(full.clone()), &[])
                .unwrap(),
            Value::String(JsString::from("function stripped() {}"))
        );

        runtime.set_debug_info_mode(DebugInfoMode::StripSource);
        let Value::Object(source_stripped) = context
            .eval_with_filename(expression, "source-stripped.js")
            .unwrap()
        else {
            panic!("source-stripped compile did not return a function");
        };
        for (key, expected) in keys.iter().zip([
            Value::String(JsString::from("source-stripped.js")),
            Value::Int(2),
            Value::Int(4),
        ]) {
            assert_eq!(
                context.get_property(&source_stripped, key).unwrap(),
                expected
            );
        }
        assert_eq!(
            context
                .call(&to_string, Value::Object(source_stripped), &[])
                .unwrap(),
            Value::String(JsString::from(
                "function stripped() {\n    [native code]\n}"
            ))
        );

        runtime.set_debug_info_mode(DebugInfoMode::StripDebug);
        let Value::Object(debug_stripped) = context
            .eval_with_filename(expression, "debug-stripped.js")
            .unwrap()
        else {
            panic!("debug-stripped compile did not return a function");
        };
        for key in &keys {
            assert_eq!(
                context.get_property(&debug_stripped, key).unwrap(),
                Value::Undefined
            );
        }
        assert_eq!(
            context
                .call(&to_string, Value::Object(debug_stripped), &[])
                .unwrap(),
            Value::String(JsString::from(
                "function stripped() {\n    [native code]\n}"
            ))
        );

        // Changing the runtime policy never mutates already-published bytecode.
        assert_eq!(
            context.call(&to_string, Value::Object(full), &[]).unwrap(),
            Value::String(JsString::from("function stripped() {}"))
        );
    }

    #[test]
    fn function_debug_position_distinguishes_missing_debug_from_missing_pc_table() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let keys = ["fileName", "lineNumber", "columnNumber"]
            .map(|name| runtime.intern_property_key(name).unwrap());

        let with_debug = runtime
            .publish_unlinked_function(
                context.realm,
                debug_draft(UnlinkedFunctionDebug {
                    filename: JsString::from("no-pc-table.js"),
                    pc2line: None,
                    source: None,
                }),
            )
            .unwrap();
        let with_debug = runtime
            .new_bytecode_closure(context.realm, &with_debug)
            .unwrap();
        for (key, expected) in keys.iter().zip([
            Value::String(JsString::from("no-pc-table.js")),
            Value::Int(0),
            Value::Int(0),
        ]) {
            assert_eq!(
                context.get_property(with_debug.as_object(), key).unwrap(),
                expected
            );
        }

        let without_debug = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let without_debug = runtime
            .new_bytecode_closure(context.realm, &without_debug)
            .unwrap();
        for key in &keys {
            assert_eq!(
                context
                    .get_property(without_debug.as_object(), key)
                    .unwrap(),
                Value::Undefined
            );
        }
    }

    #[test]
    fn function_bind_and_to_string_use_quickjs_payload_and_source_paths() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let bind_key = runtime.intern_property_key("bind").unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let Value::Object(bind_object) = context
            .get_property(&function_prototype, &bind_key)
            .unwrap()
        else {
            panic!("Function.prototype.bind was not an object");
        };
        let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
        let Value::Object(to_string_object) = context
            .get_property(&function_prototype, &to_string_key)
            .unwrap()
        else {
            panic!("Function.prototype.toString was not an object");
        };
        let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

        let authored = "function /*keep*/ named(a, b) { return a + b; }";
        let Value::Object(target_object) = context.eval(&format!("({authored})")).unwrap() else {
            panic!("function source did not evaluate to an object");
        };
        assert_eq!(
            context
                .call(&to_string, Value::Object(target_object.clone()), &[],)
                .unwrap(),
            Value::String(JsString::from(authored))
        );
        let name_key = runtime.intern_property_key("name").unwrap();
        assert!(
            context
                .define_own_property(
                    &target_object,
                    &name_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::String(JsString::from("changed"))),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(target_object.clone()), &[],)
                .unwrap(),
            Value::String(JsString::from(authored)),
            "stored bytecode source must not read the mutable name property"
        );

        let Value::Object(zero_argument_bound) = context
            .call(&bind, Value::Object(target_object.clone()), &[])
            .unwrap()
        else {
            panic!("zero-argument bind did not return an object");
        };
        assert_eq!(
            own_data_value(&runtime, &zero_argument_bound, "length"),
            Value::Int(2)
        );
        assert_eq!(
            own_data_value(&runtime, &zero_argument_bound, "name"),
            Value::String(JsString::from("bound changed"))
        );

        let bound = context
            .call(
                &bind,
                Value::Object(target_object.clone()),
                &[Value::Undefined, Value::Int(4)],
            )
            .unwrap();
        let Value::Object(bound_object) = bound else {
            panic!("bind did not return an object");
        };
        let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
        assert_eq!(
            context.call(&bound, Value::Null, &[Value::Int(5)]).unwrap(),
            Value::Int(9)
        );
        assert_eq!(
            runtime.get_prototype_of(&bound_object).unwrap(),
            Some(function_prototype.clone())
        );
        assert_eq!(own_key_names(&runtime, &bound_object), ["length", "name"]);
        assert_eq!(
            own_data_value(&runtime, &bound_object, "length"),
            Value::Int(1)
        );
        assert_eq!(
            own_data_value(&runtime, &bound_object, "name"),
            Value::String(JsString::from("bound changed"))
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(bound_object.clone()), &[])
                .unwrap(),
            Value::String(JsString::from(
                "function bound changed() {\n    [native code]\n}"
            ))
        );

        let Value::Object(sum_target_object) = context
            .eval("(function(a,b,c){return a * 100 + b * 10 + c;})")
            .unwrap()
        else {
            panic!("sum target was not a function");
        };
        let inner = context
            .call(
                &bind,
                Value::Object(sum_target_object),
                &[Value::Undefined, Value::Int(1)],
            )
            .unwrap();
        let Value::Object(inner_object) = inner else {
            panic!("inner bind was not an object");
        };
        let outer = context
            .call(
                &bind,
                Value::Object(inner_object),
                &[Value::Undefined, Value::Int(2)],
            )
            .unwrap();
        let Value::Object(outer_object) = outer else {
            panic!("outer bind was not an object");
        };
        let outer = runtime.as_callable(&outer_object).unwrap().unwrap();
        assert_eq!(
            context.call(&outer, Value::Null, &[Value::Int(3)]).unwrap(),
            Value::Int(123)
        );

        let first_this = context.new_object().unwrap();
        let second_this = context.new_object().unwrap();
        let Value::Object(this_target) = context.eval("(function(){return this;})").unwrap() else {
            panic!("this target was not a function");
        };
        let inner = context
            .call(
                &bind,
                Value::Object(this_target),
                &[Value::Object(first_this.clone())],
            )
            .unwrap();
        let Value::Object(inner) = inner else {
            panic!("bound this function was not an object");
        };
        let rebound = context
            .call(&bind, Value::Object(inner), &[Value::Object(second_this)])
            .unwrap();
        let Value::Object(rebound) = rebound else {
            panic!("rebound function was not an object");
        };
        let rebound = runtime.as_callable(&rebound).unwrap().unwrap();
        assert_eq!(
            context.call(&rebound, Value::Null, &[]).unwrap(),
            Value::Object(first_this)
        );

        let Value::Object(constructor_object) = context
            .eval("(function Constructor(){return new.target;})")
            .unwrap()
        else {
            panic!("constructor target was not a function");
        };
        let constructor = runtime.as_callable(&constructor_object).unwrap().unwrap();
        let Value::Object(bound_constructor) = context
            .call(
                &bind,
                Value::Object(constructor_object.clone()),
                &[Value::Undefined],
            )
            .unwrap()
        else {
            panic!("bound constructor was not an object");
        };
        let bound_constructor = runtime.as_callable(&bound_constructor).unwrap().unwrap();
        assert_eq!(
            context.construct(&bound_constructor, &[]).unwrap(),
            Value::Object(constructor_object)
        );
        let Value::Object(other_object) = context.eval("(function Other(){})").unwrap() else {
            panic!("explicit new target was not a function");
        };
        let other = runtime.as_callable(&other_object).unwrap().unwrap();
        assert_eq!(
            context
                .construct_with_new_target(&bound_constructor, &other, &[])
                .unwrap(),
            Value::Object(other_object)
        );
        drop(constructor);

        assert_eq!(
            context.eval("(function named(){}) + \"\"").unwrap(),
            Value::String(JsString::from("function named(){}"))
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(function_prototype.clone()), &[],)
                .unwrap(),
            Value::String(JsString::from("function () {\n    [native code]\n}"))
        );

        for (function_kind, expected) in [
            (
                crate::heap::FunctionKind::Generator,
                "function *fallback() {\n    [native code]\n}",
            ),
            (
                crate::heap::FunctionKind::Async,
                "async function fallback() {\n    [native code]\n}",
            ),
            (
                crate::heap::FunctionKind::AsyncGenerator,
                "async function *fallback() {\n    [native code]\n}",
            ),
        ] {
            let function = runtime
                .publish_unlinked_function(
                    context.realm,
                    UnlinkedFunction::new(
                        vec![Instruction::Undefined, Instruction::Return],
                        Vec::new(),
                        FunctionMetadata {
                            max_stack: 1,
                            function_kind,
                            ..FunctionMetadata::default()
                        },
                    )
                    .with_name(Some(JsString::from("fallback"))),
                )
                .unwrap();
            let callable = runtime
                .new_bytecode_closure(context.realm, &function)
                .unwrap();
            assert_eq!(
                context
                    .call(&to_string, Value::Object(callable.as_object().clone()), &[],)
                    .unwrap(),
                Value::String(JsString::from(expected))
            );
        }

        assert!(
            context
                .define_own_property(
                    &to_string_object,
                    &name_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(3)),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(to_string_object.clone()), &[],)
                .unwrap(),
            Value::String(JsString::from("function 3() {\n    [native code]\n}"))
        );

        let Value::Object(name_getter_object) =
            context.eval("(function(){throw \"NAME\";})").unwrap()
        else {
            panic!("name getter was not a function");
        };
        let name_getter = runtime.as_callable(&name_getter_object).unwrap().unwrap();
        let throwing_name = OrdinaryPropertyDescriptor {
            get: DescriptorField::Present(AccessorValue::Callable(name_getter.clone())),
            set: DescriptorField::Present(AccessorValue::Undefined),
            enumerable: DescriptorField::Present(false),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(
            context
                .define_own_property(&target_object, &name_key, &throwing_name)
                .unwrap()
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(target_object), &[])
                .unwrap(),
            Value::String(JsString::from(authored)),
            "stored bytecode source must bypass a throwing name getter"
        );
        assert!(
            context
                .define_own_property(&to_string_object, &name_key, &throwing_name)
                .unwrap()
        );
        assert_eq!(
            context.call(&to_string, Value::Object(to_string_object), &[],),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            context.take_exception().unwrap(),
            Some(Value::String(JsString::from("NAME")))
        );

        let symbol_name = runtime
            .new_symbol(Some(JsString::from("native-name")))
            .unwrap();
        assert!(
            context
                .define_own_property(
                    &bind_object,
                    &name_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Symbol(symbol_name)),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context.call(&to_string, Value::Object(bind_object), &[]),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("Symbol function name did not throw an Error object");
        };
        let message_key = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&error, &message_key).unwrap(),
            Value::String(JsString::from("cannot convert symbol to string"))
        );
    }

    #[test]
    fn bound_function_payload_owns_symbols_and_cycles_across_layout_changes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let bind_key = runtime.intern_property_key("bind").unwrap();
        let Value::Object(bind_object) = context
            .get_property(&function_prototype, &bind_key)
            .unwrap()
        else {
            panic!("Function.prototype.bind was not an object");
        };
        let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
        let Value::Object(target_object) =
            context.eval("(function(value){return value;})").unwrap()
        else {
            panic!("bound payload target was not a function");
        };
        let baseline_atoms = runtime.test_atom_count();
        let symbol = runtime
            .new_symbol(Some(JsString::from("bound-payload")))
            .unwrap();
        let Value::Object(bound_object) = context
            .call(
                &bind,
                Value::Object(target_object.clone()),
                &[Value::Undefined, Value::Symbol(symbol.clone())],
            )
            .unwrap()
        else {
            panic!("symbol-bound function was not an object");
        };
        let extra_key = runtime.intern_property_key("bound-extra").unwrap();
        assert!(
            context
                .define_own_property(
                    &bound_object,
                    &extra_key,
                    &data_descriptor(Value::Int(1), true, true, true),
                )
                .unwrap()
        );
        let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
        drop(symbol);
        let returned = context.call(&bound, Value::Undefined, &[]).unwrap();
        assert!(matches!(returned, Value::Symbol(_)));
        drop(returned);
        drop(bound);
        drop(bound_object);
        drop(extra_key);
        assert_eq!(runtime.test_atom_count(), baseline_atoms);

        runtime.run_gc().unwrap();
        let baseline_objects = runtime.heap_counts().object_nodes;
        let argument = context.new_object().unwrap();
        let Value::Object(bound_object) = context
            .call(
                &bind,
                Value::Object(target_object.clone()),
                &[Value::Undefined, Value::Object(argument.clone())],
            )
            .unwrap()
        else {
            panic!("cycle-bound function was not an object");
        };
        let back_key = runtime.intern_property_key("bound-back").unwrap();
        assert!(
            set_property(
                &runtime,
                &argument,
                &back_key,
                Value::Object(bound_object.clone()),
            )
            .unwrap()
        );
        drop(bound_object);
        drop(argument);
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 2);
        let stats = runtime.run_gc().unwrap();
        assert!(stats.cleanup.finalized_objects >= 2);
        assert!(runtime.as_callable(&target_object).unwrap().is_some());
        assert_eq!(
            runtime.heap_counts().object_nodes,
            baseline_objects,
            "unexpected GC delta: {stats:?}"
        );
    }

    #[test]
    fn bound_function_uses_bind_realm_but_delegates_function_realm_and_has_instance() {
        let runtime = Runtime::new();
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_function_prototype = first.function_prototype().unwrap();
        let bind_key = runtime.intern_property_key("bind").unwrap();
        let Value::Object(bind_object) = first
            .get_property(&first_function_prototype, &bind_key)
            .unwrap()
        else {
            panic!("first realm bind was not an object");
        };
        let bind = runtime.as_callable(&bind_object).unwrap().unwrap();
        let Value::Object(target_object) = second.eval("(function Target(){})").unwrap() else {
            panic!("second realm target was not a function");
        };

        let has_instance_key =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
        let Value::Object(custom_method) = second.eval("(function(value){return value;})").unwrap()
        else {
            panic!("custom hasInstance method was not a function");
        };
        assert!(
            second
                .define_own_property(
                    &target_object,
                    &has_instance_key,
                    &data_descriptor(Value::Object(custom_method), true, false, true),
                )
                .unwrap()
        );

        let Value::Object(bound_object) = first
            .call(&bind, Value::Object(target_object), &[Value::Undefined])
            .unwrap()
        else {
            panic!("cross-realm bound function was not an object");
        };
        let bound = runtime.as_callable(&bound_object).unwrap().unwrap();
        assert_eq!(
            runtime.get_prototype_of(&bound_object).unwrap(),
            Some(first_function_prototype.clone()),
            "bound [[Prototype]] must come from the bind method realm"
        );
        assert_eq!(runtime.callable_realm(&bound).unwrap(), second.realm);

        let Value::Object(nested_bound_object) = first
            .call(
                &bind,
                Value::Object(bound_object.clone()),
                &[Value::Undefined],
            )
            .unwrap()
        else {
            panic!("nested cross-realm bound function was not an object");
        };
        let nested_bound = runtime.as_callable(&nested_bound_object).unwrap().unwrap();
        assert_eq!(runtime.callable_realm(&nested_bound).unwrap(), second.realm);

        let Value::Object(has_instance_object) = first
            .get_property(&first_function_prototype, &has_instance_key)
            .unwrap()
        else {
            panic!("Function.prototype[Symbol.hasInstance] was not an object");
        };
        let has_instance = runtime.as_callable(&has_instance_object).unwrap().unwrap();
        assert_eq!(
            first
                .call(
                    &has_instance,
                    Value::Object(nested_bound_object),
                    &[Value::Int(1)],
                )
                .unwrap(),
            Value::Bool(true),
            "bound ordinary hasInstance must delegate the primitive candidate to target @@hasInstance"
        );
    }

    #[test]
    fn deep_standard_bound_has_instance_delegation_is_host_stack_safe() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let has_instance_key =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::HasInstance));
        let Value::Object(has_instance_object) = context
            .get_property(&function_prototype, &has_instance_key)
            .unwrap()
        else {
            panic!("Function.prototype[Symbol.hasInstance] was not an object");
        };
        let has_instance = runtime.as_callable(&has_instance_object).unwrap().unwrap();
        let Value::Object(target_object) = context.eval("(function Target(){})").unwrap() else {
            panic!("target was not a function");
        };
        let mut target = runtime.as_callable(&target_object).unwrap().unwrap();

        // Pinned QuickJS still completes at this depth with its default stack
        // budget. The Rust path must preserve that result without recursively
        // consuming the host stack.
        for _ in 0..512 {
            target = runtime
                .new_bound_function(context.realm, &target, &Value::Undefined, &[])
                .unwrap();
        }
        assert_eq!(
            context
                .call(
                    &has_instance,
                    Value::Object(target.into_object()),
                    &[Value::Int(1)],
                )
                .unwrap(),
            Value::Bool(false)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn error_intrinsic_graph_and_lazy_methods_match_quickjs_descriptors() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error = global_callable(&runtime, &mut context, "Error");
        let type_error = global_callable(&runtime, &mut context, "TypeError");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let constructor_key = runtime.intern_property_key("constructor").unwrap();
        let is_error_key = runtime.intern_property_key("isError").unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let name_key = runtime.intern_property_key("name").unwrap();
        let aggregate_key = runtime.intern_property_key("AggregateError").unwrap();

        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(error_prototype),
            writable: false,
            enumerable: false,
            configurable: false,
        } = runtime
            .get_own_property(error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("Error.prototype descriptor did not match QuickJS");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(type_error_prototype),
            writable: false,
            enumerable: false,
            configurable: false,
        } = runtime
            .get_own_property(type_error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("TypeError.prototype descriptor did not match QuickJS");
        };

        let object_count = runtime.heap_counts().object_nodes;
        let realm_strong_count = runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(context.realm)
            .unwrap();
        assert_eq!(
            own_key_names(&runtime, error.as_object()),
            ["length", "name", "isError", "prototype"]
        );
        assert_eq!(
            own_key_names(&runtime, &error_prototype),
            ["toString", "name", "message", "constructor"]
        );
        assert!(
            runtime
                .has_own_property(error.as_object(), &is_error_key)
                .unwrap()
        );
        assert!(
            runtime
                .has_own_property(&error_prototype, &to_string_key)
                .unwrap()
        );
        assert_eq!(runtime.heap_counts().object_nodes, object_count);

        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(is_error),
            writable: true,
            enumerable: false,
            configurable: true,
        } = runtime
            .get_own_property(error.as_object(), &is_error_key)
            .unwrap()
            .unwrap()
        else {
            panic!("Error.isError did not materialize as a native data property");
        };
        assert_eq!(runtime.heap_counts().object_nodes, object_count + 1);
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(context.realm),
            Ok(realm_strong_count)
        );
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(to_string),
            writable: true,
            enumerable: false,
            configurable: true,
        } = runtime
            .get_own_property(&error_prototype, &to_string_key)
            .unwrap()
            .unwrap()
        else {
            panic!("Error.prototype.toString did not materialize as native data");
        };
        assert_eq!(runtime.heap_counts().object_nodes, object_count + 2);
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(context.realm),
            Ok(realm_strong_count)
        );
        assert!(runtime.as_callable(&is_error).unwrap().is_some());
        assert!(runtime.as_callable(&to_string).unwrap().is_some());

        assert!(matches!(
            runtime.get_own_property(&error_prototype, &name_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if value == JsString::from("Error")
        ));
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(context.realm),
            Ok(realm_strong_count - 1)
        );

        assert_eq!(
            runtime.get_prototype_of(error.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert_eq!(
            runtime.get_prototype_of(type_error.as_object()).unwrap(),
            Some(error.as_object().clone())
        );
        assert_eq!(
            runtime.get_prototype_of(&error_prototype).unwrap(),
            Some(context.object_prototype().unwrap())
        );
        assert_eq!(
            runtime.get_prototype_of(&type_error_prototype).unwrap(),
            Some(error_prototype.clone())
        );
        assert!(matches!(
            runtime
                .get_own_property(&error_prototype, &constructor_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(value),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if value == *error.as_object()
        ));
        assert!(matches!(
            runtime
                .get_own_property(&type_error_prototype, &constructor_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(value),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if value == *type_error.as_object()
        ));
        assert!(!runtime.is_error_object(&error_prototype).unwrap());
        assert!(!runtime.is_error_object(&type_error_prototype).unwrap());
        assert_eq!(
            context
                .get_property(&context.global_object().unwrap(), &aggregate_key)
                .unwrap(),
            Value::Undefined
        );
    }

    #[test]
    fn error_constructors_to_string_is_error_and_cause_follow_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error = global_callable(&runtime, &mut context, "Error");
        let type_error = global_callable(&runtime, &mut context, "TypeError");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let message_key = runtime.intern_property_key("message").unwrap();
        let cause_key = runtime.intern_property_key("cause").unwrap();
        let is_error_key = runtime.intern_property_key("isError").unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(error_prototype),
            ..
        } = runtime
            .get_own_property(error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("Error constructor had no object prototype");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(type_error_prototype),
            ..
        } = runtime
            .get_own_property(type_error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("TypeError constructor had no object prototype");
        };
        let Value::Object(is_error_object) = context
            .get_property(error.as_object(), &is_error_key)
            .unwrap()
        else {
            panic!("Error.isError was not an object");
        };
        let is_error = runtime.as_callable(&is_error_object).unwrap().unwrap();
        let Value::Object(to_string_object) = context
            .get_property(&error_prototype, &to_string_key)
            .unwrap()
        else {
            panic!("Error.prototype.toString was not an object");
        };
        let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();

        let Value::Object(empty) = context.call(&error, Value::Undefined, &[]).unwrap() else {
            panic!("Error() did not return an object");
        };
        assert!(runtime.is_error_object(&empty).unwrap());
        assert_eq!(
            runtime.get_prototype_of(&empty).unwrap(),
            Some(error_prototype.clone())
        );
        assert!(!runtime.has_own_property(&empty, &message_key).unwrap());
        assert_eq!(
            context
                .call(&to_string, Value::Object(empty.clone()), &[],)
                .unwrap(),
            Value::String(JsString::from("Error"))
        );

        let Value::Object(with_message) = context
            .call(&error, Value::Undefined, &[Value::Int(42)])
            .unwrap()
        else {
            panic!("Error(42) did not return an object");
        };
        assert!(matches!(
            runtime.get_own_property(&with_message, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if value == JsString::from("42")
        ));
        assert_eq!(
            context
                .call(&to_string, Value::Object(with_message.clone()), &[],)
                .unwrap(),
            Value::String(JsString::from("Error: 42"))
        );

        let Value::Object(typed) = context
            .construct(&type_error, &[Value::String(JsString::from("boom"))])
            .unwrap()
        else {
            panic!("new TypeError did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&typed).unwrap(),
            Some(type_error_prototype)
        );
        assert_eq!(
            context
                .call(&to_string, Value::Object(typed.clone()), &[])
                .unwrap(),
            Value::String(JsString::from("TypeError: boom"))
        );
        assert_eq!(
            context
                .call(&is_error, Value::Undefined, &[Value::Object(typed.clone())],)
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(
            context
                .call(
                    &is_error,
                    Value::Undefined,
                    &[Value::Object(error_prototype.clone())],
                )
                .unwrap(),
            Value::Bool(false)
        );
        let spoof = context.new_object().unwrap();
        assert!(
            runtime
                .set_prototype_of(&spoof, Some(&error_prototype))
                .unwrap()
        );
        assert_eq!(
            context
                .call(&is_error, Value::Undefined, &[Value::Object(spoof)],)
                .unwrap(),
            Value::Bool(false)
        );

        let options = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &options,
                    &cause_key,
                    &data_descriptor(Value::Undefined, true, true, true),
                )
                .unwrap()
        );
        let Value::Object(with_cause) = context
            .call(
                &error,
                Value::Undefined,
                &[Value::Undefined, Value::Object(options)],
            )
            .unwrap()
        else {
            panic!("Error(undefined, options) did not return an object");
        };
        assert!(!runtime.has_own_property(&with_cause, &message_key).unwrap());
        assert!(matches!(
            runtime.get_own_property(&with_cause, &cause_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Undefined,
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));

        let inherited_cause_holder = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &inherited_cause_holder,
                    &cause_key,
                    &data_descriptor(Value::Int(5), true, true, true),
                )
                .unwrap()
        );
        let inherited_options = context.new_object().unwrap();
        assert!(
            runtime
                .set_prototype_of(&inherited_options, Some(&inherited_cause_holder))
                .unwrap()
        );
        let Value::Object(with_inherited_cause) = context
            .call(
                &error,
                Value::Undefined,
                &[Value::Undefined, Value::Object(inherited_options)],
            )
            .unwrap()
        else {
            panic!("inherited cause Error construction did not return an object");
        };
        assert!(matches!(
            runtime
                .get_own_property(&with_inherited_cause, &cause_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(5),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));

        assert!(matches!(
            context.call(&to_string, Value::Int(1), &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("non-object Error.prototype.toString throw was not an object");
        };
        assert!(matches!(
            runtime.get_own_property(&exception, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("not an object")
        ));
        assert_eq!(
            own_stack_string(&runtime, &exception),
            JsString::from("    at toString (native)\n")
        );
    }

    #[test]
    fn error_stack_eager_capture_matches_quickjs_frames_sites_and_descriptor() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source =
            "(function outer(){ return (function inner(){ return new Error(\"boom\"); })(); })()";
        let Value::Object(error) = context.eval_with_filename(source, "<cmdline>").unwrap() else {
            panic!("nested Error constructor did not return an object");
        };
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from(
                "    at inner (<cmdline>:1:62)\n    at outer (<cmdline>:1:20)\n    at <eval> (<cmdline>:1:80)\n"
            )
        );
        assert_eq!(own_key_names(&runtime, &error), ["message", "stack"]);
        let stack_key = runtime.intern_property_key("stack").unwrap();
        assert!(matches!(
            runtime.get_own_property(&error, &stack_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(_),
                writable: true,
                enumerable: false,
                configurable: true,
            })
        ));

        let error_constructor = global_callable(&runtime, &mut context, "Error");
        let Value::Object(direct) = context
            .call(&error_constructor, Value::Undefined, &[])
            .unwrap()
        else {
            panic!("direct Error() did not return an object");
        };
        assert_eq!(own_key_names(&runtime, &direct), ["stack"]);
        assert_eq!(own_stack_string(&runtime, &direct), JsString::from(""));
    }

    #[test]
    fn error_constructor_skips_only_itself_and_preserves_other_native_frames() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let probe = runtime
            .new_bound_native_function(
                &function_prototype,
                context.realm,
                NativeFunctionId::ActiveFrameProbe,
                0,
            )
            .unwrap();
        runtime
            .define_function_data_property(
                probe.as_object(),
                "name",
                Value::String(JsString::from("probe")),
                false,
                true,
            )
            .unwrap();
        let probe_key = runtime.intern_property_key("probe").unwrap();
        let global = context.global_object().unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &probe_key,
                    &data_descriptor(Value::Object(probe.as_object().clone()), true, true, true,),
                )
                .unwrap()
        );

        let source = "(function viaNative(){ return probe(function callback(){ return new Error(\"x\"); }); })()";
        let Value::Object(error) = context.eval_with_filename(source, "native.js").unwrap() else {
            panic!("native callback did not return an Error");
        };
        let callback_construct = source.find("Error").unwrap() + "Error".len() + 1;
        let outer_return = source.find("return").unwrap() + 1;
        let root_call = source.rfind("()").unwrap() + 1;
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from_utf8(&format!(
                "    at callback (native.js:1:{callback_construct})\n    at probe (native)\n    at viaNative (native.js:1:{outer_return})\n    at <eval> (native.js:1:{root_call})\n"
            ))
        );
    }

    #[test]
    fn native_rethrow_pops_its_frame_before_bytecode_captures_missing_stack() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error_constructor = global_callable(&runtime, &mut context, "Error");
        let Value::Object(error) = context
            .call(&error_constructor, Value::Undefined, &[])
            .unwrap()
        else {
            panic!("Error() did not return an object");
        };
        let stack_key = runtime.intern_property_key("stack").unwrap();
        assert!(runtime.delete_property(&error, &stack_key).unwrap());

        let function_prototype = context.function_prototype().unwrap();
        let rethrow = runtime
            .new_bound_native_function(
                &function_prototype,
                context.realm,
                NativeFunctionId::ActiveFrameProbe,
                0,
            )
            .unwrap();
        runtime
            .define_function_data_property(
                rethrow.as_object(),
                "name",
                Value::String(JsString::from("rethrowProbe")),
                false,
                true,
            )
            .unwrap();

        assert_eq!(
            context.call(
                &rethrow,
                Value::Undefined,
                &[Value::Object(error.clone()), Value::Bool(false)],
            ),
            Err(RuntimeError::Exception)
        );
        let Value::Object(direct) = context.take_exception().unwrap().unwrap() else {
            panic!("direct native rethrow lost its Error");
        };
        assert_eq!(direct, error);
        assert!(!runtime.has_own_property(&error, &stack_key).unwrap());

        let global = context.global_object().unwrap();
        for (name, value) in [
            ("rethrowProbe", Value::Object(rethrow.as_object().clone())),
            ("heldError", Value::Object(error.clone())),
        ] {
            let key = runtime.intern_property_key(name).unwrap();
            assert!(
                context
                    .define_own_property(&global, &key, &data_descriptor(value, true, true, true),)
                    .unwrap()
            );
        }
        let source = "rethrowProbe(heldError, false)";
        assert!(matches!(
            context.eval_with_filename(source, "rethrow.js"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(from_bytecode) = context.take_exception().unwrap().unwrap() else {
            panic!("bytecode native rethrow lost its Error");
        };
        assert_eq!(from_bytecode, error);
        let call_column = source.find('(').unwrap() + 1;
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from_utf8(&format!("    at <eval> (rethrow.js:1:{call_column})\n"))
        );
    }

    #[test]
    fn vm_error_stack_uses_fault_tail_call_and_root_call_sites() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = "(function outer(){ return (function inner(){ return 1n + 1; })(); })()";
        assert!(matches!(
            context.eval_with_filename(source, "<cmdline>"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("VM TypeError was not an object");
        };
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from(
                "    at inner (<cmdline>:1:56)\n    at outer (<cmdline>:1:20)\n    at <eval> (<cmdline>:1:69)\n"
            )
        );
        assert_eq!(own_key_names(&runtime, &error), ["message", "stack"]);
    }

    #[test]
    fn syntax_error_stack_prepends_parse_location_and_metadata_in_quickjs_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert!(matches!(
            context.eval_with_filename("1 +", "parse.js"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("SyntaxError was not an object");
        };
        assert_eq!(
            own_key_names(&runtime, &error),
            ["message", "fileName", "lineNumber", "columnNumber", "stack"]
        );
        assert_eq!(
            own_data_value(&runtime, &error, "fileName"),
            Value::String(JsString::from("parse.js"))
        );
        assert_eq!(
            own_data_value(&runtime, &error, "lineNumber"),
            Value::Int(1)
        );
        assert_eq!(
            own_data_value(&runtime, &error, "columnNumber"),
            Value::Int(4)
        );
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from("    at parse.js:1:4\n")
        );
    }

    #[test]
    fn eval_backtrace_barrier_marks_only_the_preexisting_caller_frame() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let caller = runtime
            .new_bound_native_function(
                &function_prototype,
                context.realm,
                NativeFunctionId::ActiveFrameProbe,
                0,
            )
            .unwrap();
        let caller_frame = runtime
            .push_native_active_frame(
                caller.as_object().clone(),
                context.realm,
                NativeFunctionId::ActiveFrameProbe,
                0,
                0,
            )
            .unwrap();
        let options = EvalOptions {
            filename: "barrier.js".to_owned(),
            backtrace_barrier: true,
        };

        assert!(matches!(
            context.eval_with_options("1n + 1", &options),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(runtime_error) = context.take_exception().unwrap().unwrap() else {
            panic!("barrier VM error was not an object");
        };
        assert_eq!(
            own_stack_string(&runtime, &runtime_error),
            JsString::from("    at <eval> (barrier.js:1:4)\n")
        );
        assert!(
            !runtime
                .0
                .state
                .borrow()
                .active_frames
                .last()
                .unwrap()
                .flags
                .backtrace_barrier
        );

        assert!(matches!(
            context.eval_with_options("1 +", &options),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(parse_error) = context.take_exception().unwrap().unwrap() else {
            panic!("barrier parse error was not an object");
        };
        assert_eq!(
            own_stack_string(&runtime, &parse_error),
            JsString::from("    at barrier.js:1:4\n")
        );
        assert!(
            !runtime
                .0
                .state
                .borrow()
                .active_frames
                .last()
                .unwrap()
                .flags
                .backtrace_barrier
        );
        caller_frame.finish().unwrap();
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn backtrace_capture_respects_own_stack_and_real_error_class() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error_constructor = global_callable(&runtime, &mut context, "Error");
        let Value::Object(error) = context
            .call(&error_constructor, Value::Undefined, &[])
            .unwrap()
        else {
            panic!("Error() did not return an object");
        };
        let stack_key = runtime.intern_property_key("stack").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &error,
                    &stack_key,
                    &data_descriptor(Value::Undefined, true, false, true),
                )
                .unwrap()
        );

        let held_key = runtime.intern_property_key("heldError").unwrap();
        let global = context.global_object().unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &held_key,
                    &data_descriptor(Value::Object(error.clone()), true, true, true),
                )
                .unwrap()
        );
        assert!(matches!(
            context.eval_with_filename("throw heldError", "throw.js"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(first_throw) = context.take_exception().unwrap().unwrap() else {
            panic!("held Error throw lost its object identity");
        };
        assert_eq!(first_throw, error);
        assert_eq!(
            own_data_value(&runtime, &first_throw, "stack"),
            Value::Undefined
        );

        assert!(runtime.delete_property(&error, &stack_key).unwrap());
        assert!(matches!(
            context.eval_with_filename("throw heldError", "throw.js"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(second_throw) = context.take_exception().unwrap().unwrap() else {
            panic!("rethrow lost its Error object");
        };
        assert_eq!(second_throw, error);
        assert_eq!(
            own_stack_string(&runtime, &second_throw),
            JsString::from("    at <eval> (throw.js:1:1)\n")
        );

        let Value::Object(error_prototype) =
            own_data_value(&runtime, error_constructor.as_object(), "prototype")
        else {
            panic!("Error.prototype was not an object");
        };
        let spoof = context
            .new_object_with_prototype(Some(&error_prototype))
            .unwrap();
        let spoof_key = runtime.intern_property_key("spoofError").unwrap();
        assert!(
            context
                .define_own_property(
                    &global,
                    &spoof_key,
                    &data_descriptor(Value::Object(spoof.clone()), true, true, true),
                )
                .unwrap()
        );
        assert!(matches!(
            context.eval("throw spoofError"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(thrown_spoof) = context.take_exception().unwrap().unwrap() else {
            panic!("ordinary spoof throw lost its object");
        };
        assert_eq!(thrown_spoof, spoof);
        assert!(!runtime.has_own_property(&spoof, &stack_key).unwrap());
    }

    #[test]
    fn backtrace_function_name_lookup_is_raw_and_only_one_prototype_deep() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = "(function leaf(){ return 1n + 1; })";
        let Value::Object(leaf_object) = context.eval_with_filename(source, "name.js").unwrap()
        else {
            panic!("leaf function was not an object");
        };
        let leaf = runtime.as_callable(&leaf_object).unwrap().unwrap();
        let Value::Object(getter_object) = context
            .eval("(function nameGetter(){ throw \"name getter ran\"; })")
            .unwrap()
        else {
            panic!("name getter was not an object");
        };
        let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
        let name_key = runtime.intern_property_key("name").unwrap();
        assert!(
            runtime
                .define_own_property(
                    leaf.as_object(),
                    &name_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        set: DescriptorField::Present(AccessorValue::Undefined),
                        enumerable: DescriptorField::Present(false),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(&leaf, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("leaf TypeError was replaced by the name getter");
        };
        let plus_column = source.find('+').unwrap() + 1;
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from_utf8(&format!("    at <anonymous> (name.js:1:{plus_column})\n"))
        );

        for (name, expected) in [
            (
                JsString::from_utf16([u16::from(b'a'), 0, u16::from(b'b')]),
                "a",
            ),
            (
                JsString::from_utf16([0, u16::from(b'a'), u16::from(b'b')]),
                "<anonymous>",
            ),
        ] {
            assert!(
                runtime
                    .define_own_property(
                        leaf.as_object(),
                        &name_key,
                        &data_descriptor(Value::String(name), false, false, true),
                    )
                    .unwrap()
            );
            assert!(matches!(
                context.call(&leaf, Value::Undefined, &[]),
                Err(RuntimeError::Exception)
            ));
            let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
                panic!("renamed leaf TypeError was not an object");
            };
            assert_eq!(
                own_stack_string(&runtime, &error),
                JsString::from_utf8(&format!("    at {expected} (name.js:1:{plus_column})\n"))
            );
        }
    }

    #[test]
    fn cross_realm_backtrace_uses_each_bytecode_filename_and_throwing_realm_error() {
        let runtime = Runtime::new();
        let mut realm_a = runtime.new_context();
        let mut realm_b = runtime.new_context();
        let source_a = "(function inA(){ return 1n + 1; })";
        let Value::Object(in_a) = realm_a.eval_with_filename(source_a, "a.js").unwrap() else {
            panic!("realm A function was not an object");
        };
        let global_b = realm_b.global_object().unwrap();
        let in_a_key = runtime.intern_property_key("inA").unwrap();
        assert!(
            realm_b
                .define_own_property(
                    &global_b,
                    &in_a_key,
                    &data_descriptor(Value::Object(in_a), true, true, true),
                )
                .unwrap()
        );

        let source_b = "(function inB(){ return inA(); })()";
        assert!(matches!(
            realm_b.eval_with_filename(source_b, "b.js"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(error) = realm_b.take_exception().unwrap().unwrap() else {
            panic!("cross-realm TypeError was not an object");
        };
        let plus_column = source_a.find('+').unwrap() + 1;
        let return_column = source_b.find("return").unwrap() + 1;
        let root_call_column = source_b.rfind("()").unwrap() + 1;
        assert_eq!(
            own_stack_string(&runtime, &error),
            JsString::from_utf8(&format!(
                "    at inA (a.js:1:{plus_column})\n    at inB (b.js:1:{return_column})\n    at <eval> (b.js:1:{root_call_column})\n"
            ))
        );

        let type_error_a = global_callable(&runtime, &mut realm_a, "TypeError");
        let Value::Object(type_error_prototype_a) =
            own_data_value(&runtime, type_error_a.as_object(), "prototype")
        else {
            panic!("realm A TypeError.prototype was not an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(type_error_prototype_a)
        );
    }

    #[test]
    fn error_constructor_fallback_uses_explicit_new_target_realm() {
        let runtime = Runtime::new();
        let mut constructor_context = runtime.new_context();
        let mut target_context = runtime.new_context();
        let type_error = global_callable(&runtime, &mut constructor_context, "TypeError");
        let target_type_error = global_callable(&runtime, &mut target_context, "TypeError");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(target_type_error_prototype),
            ..
        } = runtime
            .get_own_property(target_type_error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("target-realm TypeError prototype was not an object");
        };
        let Value::Object(new_target_object) = target_context.eval("(0, function(){})").unwrap()
        else {
            panic!("new.target probe did not produce a function");
        };
        let new_target = runtime.as_callable(&new_target_object).unwrap().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &new_target_object,
                    &prototype_key,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Null),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        let Value::Object(instance) = constructor_context
            .construct_with_new_target(&type_error, &new_target, &[])
            .unwrap()
        else {
            panic!("cross-realm TypeError construction did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&instance).unwrap(),
            Some(target_type_error_prototype)
        );
        assert!(runtime.is_error_object(&instance).unwrap());
    }

    #[test]
    fn strict_function_name_write_throws_a_type_error_from_the_defining_realm() {
        let runtime = Runtime::new();
        let mut defining_context = runtime.new_context();
        let mut caller_context = runtime.new_context();
        let defining_type_error = global_callable(&runtime, &mut defining_context, "TypeError");
        let caller_type_error = global_callable(&runtime, &mut caller_context, "TypeError");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(defining_type_error_prototype),
            ..
        } = runtime
            .get_own_property(defining_type_error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("defining-realm TypeError prototype was not an object");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(caller_type_error_prototype),
            ..
        } = runtime
            .get_own_property(caller_type_error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("caller-realm TypeError prototype was not an object");
        };
        assert_ne!(defining_type_error_prototype, caller_type_error_prototype);

        let Value::Object(function) = defining_context
            .eval("(0, function self(){ 'use strict'; self = 1; })")
            .unwrap()
        else {
            panic!("strict named function probe was not an object");
        };
        let function = runtime.as_callable(&function).unwrap().unwrap();
        assert_eq!(
            caller_context.call(&function, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        );
        let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
            panic!("strict function-name write did not materialize an error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&exception).unwrap(),
            Some(defining_type_error_prototype)
        );
    }

    #[test]
    fn error_constructor_preserves_getter_throw_and_defining_realm_conversion_error() {
        let runtime = Runtime::new();
        let mut defining_context = runtime.new_context();
        let mut caller_context = runtime.new_context();
        let error = global_callable(&runtime, &mut defining_context, "Error");
        let type_error = global_callable(&runtime, &mut defining_context, "TypeError");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let cause_key = runtime.intern_property_key("cause").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(error_prototype),
            ..
        } = runtime
            .get_own_property(error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("defining-realm Error prototype was not an object");
        };
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(type_error_prototype),
            ..
        } = runtime
            .get_own_property(type_error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("defining-realm TypeError prototype was not an object");
        };

        let Value::Object(cross_realm_error) = caller_context
            .call(&error, Value::Undefined, &[Value::Int(7)])
            .unwrap()
        else {
            panic!("cross-realm Error call did not return an object");
        };
        assert_eq!(
            runtime.get_prototype_of(&cross_realm_error).unwrap(),
            Some(error_prototype)
        );

        let Value::Object(getter_object) =
            caller_context.eval("(0, function(){ throw 9; })").unwrap()
        else {
            panic!("cause getter probe was not a function");
        };
        let getter = runtime.as_callable(&getter_object).unwrap().unwrap();
        let options = caller_context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &options,
                    &cause_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(matches!(
            caller_context.call(
                &error,
                Value::Undefined,
                &[Value::Undefined, Value::Object(options)],
            ),
            Err(RuntimeError::Exception)
        ));
        assert_eq!(
            caller_context.take_exception().unwrap(),
            Some(Value::Int(9))
        );

        let symbol = runtime.new_symbol(None).unwrap();
        assert!(matches!(
            caller_context.call(&error, Value::Undefined, &[Value::Symbol(symbol)],),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = caller_context.take_exception().unwrap().unwrap() else {
            panic!("symbol ToString failure was not an Error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&exception).unwrap(),
            Some(type_error_prototype)
        );
    }

    #[test]
    fn object_to_primitive_string_drives_error_message_and_to_string_values() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error = global_callable(&runtime, &mut context, "Error");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let message_key = runtime.intern_property_key("message").unwrap();
        let name_key = runtime.intern_property_key("name").unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(error_prototype),
            ..
        } = runtime
            .get_own_property(error.as_object(), &prototype_key)
            .unwrap()
            .unwrap()
        else {
            panic!("Error prototype was not an object");
        };
        let Value::Object(error_to_string_object) = context
            .get_property(&error_prototype, &to_string_key)
            .unwrap()
        else {
            panic!("Error.prototype.toString was not an object");
        };
        let error_to_string = runtime
            .as_callable(&error_to_string_object)
            .unwrap()
            .unwrap();

        let ordinary = context.new_object().unwrap();
        let Value::Object(ordinary_error) = context
            .call(&error, Value::Undefined, &[Value::Object(ordinary)])
            .unwrap()
        else {
            panic!("Error(object) did not return an object");
        };
        assert!(matches!(
            runtime
                .get_own_property(&ordinary_error, &message_key)
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("[object Object]")
        ));

        let Value::Object(custom_to_string_object) =
            context.eval("(0, function(){ return 'custom'; })").unwrap()
        else {
            panic!("custom toString probe was not a function");
        };
        let custom = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &custom,
                    &to_string_key,
                    &data_descriptor(Value::Object(custom_to_string_object), true, true, true,),
                )
                .unwrap()
        );
        let Value::Object(custom_error) = context
            .call(&error, Value::Undefined, &[Value::Object(custom)])
            .unwrap()
        else {
            panic!("Error(custom object) did not return an object");
        };
        assert!(matches!(
            runtime.get_own_property(&custom_error, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("custom")
        ));

        let Value::Object(exotic_method_object) =
            context.eval("(0, function(hint){ return hint; })").unwrap()
        else {
            panic!("@@toPrimitive probe was not a function");
        };
        let exotic = context.new_object().unwrap();
        let to_primitive =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
        assert!(
            runtime
                .define_own_property(
                    &exotic,
                    &to_primitive,
                    &data_descriptor(Value::Object(exotic_method_object), true, true, true),
                )
                .unwrap()
        );
        let Value::Object(exotic_error) = context
            .call(&error, Value::Undefined, &[Value::Object(exotic)])
            .unwrap()
        else {
            panic!("Error(exotic object) did not return an object");
        };
        assert!(matches!(
            runtime.get_own_property(&exotic_error, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("string")
        ));

        let Value::Object(name_conversion_object) =
            context.eval("(0, function(){ return 'N'; })").unwrap()
        else {
            panic!("name conversion probe was not a function");
        };
        let Value::Object(message_conversion_object) =
            context.eval("(0, function(){ return 'M'; })").unwrap()
        else {
            panic!("message conversion probe was not a function");
        };
        let name_value = context.new_object().unwrap();
        let message_value = context.new_object().unwrap();
        for (object, method) in [
            (&name_value, name_conversion_object),
            (&message_value, message_conversion_object),
        ] {
            assert!(
                runtime
                    .define_own_property(
                        object,
                        &to_string_key,
                        &data_descriptor(Value::Object(method), true, true, true),
                    )
                    .unwrap()
            );
        }
        let receiver = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &receiver,
                    &name_key,
                    &data_descriptor(Value::Object(name_value), true, true, true),
                )
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(
                    &receiver,
                    &message_key,
                    &data_descriptor(Value::Object(message_value), true, true, true),
                )
                .unwrap()
        );
        assert_eq!(
            context
                .call(&error_to_string, Value::Object(receiver), &[],)
                .unwrap(),
            Value::String(JsString::from("N: M"))
        );
    }

    #[test]
    fn to_primitive_string_rejects_exotic_failures_and_skips_noncallable_ordinary_methods() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let error = global_callable(&runtime, &mut context, "Error");
        let message_key = runtime.intern_property_key("message").unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let value_of_key = runtime.intern_property_key("valueOf").unwrap();
        let to_primitive =
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));

        let noncallable = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &noncallable,
                    &to_primitive,
                    &data_descriptor(Value::Int(1), true, true, true),
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(&error, Value::Undefined, &[Value::Object(noncallable)],),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("noncallable @@toPrimitive did not throw an Error object");
        };
        assert!(matches!(
            runtime.get_own_property(&exception, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("not a function")
        ));

        let Value::Object(object_result_method) = context
            .eval("(0, function(){ return (0, function(){}); })")
            .unwrap()
        else {
            panic!("object-result @@toPrimitive probe was not a function");
        };
        let object_result = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &object_result,
                    &to_primitive,
                    &data_descriptor(Value::Object(object_result_method), true, true, true),
                )
                .unwrap()
        );
        assert!(matches!(
            context.call(&error, Value::Undefined, &[Value::Object(object_result)],),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("object-result @@toPrimitive did not throw an Error object");
        };
        assert!(matches!(
            runtime.get_own_property(&exception, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("toPrimitive")
        ));

        let Value::Object(value_of_method) = context.eval("(0, function(){ return 7; })").unwrap()
        else {
            panic!("valueOf probe was not a function");
        };
        let ordinary = context.new_object().unwrap();
        assert!(
            runtime
                .define_own_property(
                    &ordinary,
                    &to_string_key,
                    &data_descriptor(Value::Int(1), true, true, true),
                )
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(
                    &ordinary,
                    &value_of_key,
                    &data_descriptor(Value::Object(value_of_method), true, true, true),
                )
                .unwrap()
        );
        let Value::Object(converted) = context
            .call(&error, Value::Undefined, &[Value::Object(ordinary)])
            .unwrap()
        else {
            panic!("ordinary valueOf fallback did not create an Error object");
        };
        assert!(matches!(
            runtime.get_own_property(&converted, &message_key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("7")
        ));
    }

    #[test]
    fn object_prototype_prefix_methods_are_lazy_and_report_core_tags() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object_prototype = context.object_prototype().unwrap();
        let to_string_key = runtime.intern_property_key("toString").unwrap();
        let to_locale_string_key = runtime.intern_property_key("toLocaleString").unwrap();
        let value_of_key = runtime.intern_property_key("valueOf").unwrap();
        let baseline_objects = runtime.heap_counts().object_nodes;
        assert_eq!(
            own_key_names(&runtime, &object_prototype),
            ["toString", "toLocaleString", "valueOf"]
        );
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);

        let Value::Object(to_string_object) = context
            .get_property(&object_prototype, &to_string_key)
            .unwrap()
        else {
            panic!("Object.prototype.toString was not an object");
        };
        let Value::Object(to_locale_string_object) = context
            .get_property(&object_prototype, &to_locale_string_key)
            .unwrap()
        else {
            panic!("Object.prototype.toLocaleString was not an object");
        };
        let Value::Object(value_of_object) = context
            .get_property(&object_prototype, &value_of_key)
            .unwrap()
        else {
            panic!("Object.prototype.valueOf was not an object");
        };
        assert_eq!(runtime.heap_counts().object_nodes, baseline_objects + 3);
        let to_string = runtime.as_callable(&to_string_object).unwrap().unwrap();
        let to_locale_string = runtime
            .as_callable(&to_locale_string_object)
            .unwrap()
            .unwrap();
        let value_of = runtime.as_callable(&value_of_object).unwrap().unwrap();
        let object = context.new_object().unwrap();
        let function = context.eval("(0, function(){})").unwrap();
        let error = global_callable(&runtime, &mut context, "Error");
        let error = context.call(&error, Value::Undefined, &[]).unwrap();
        for (value, expected) in [
            (Value::Null, "[object Null]"),
            (Value::Undefined, "[object Undefined]"),
            (Value::Object(object.clone()), "[object Object]"),
            (function, "[object Function]"),
            (error, "[object Error]"),
        ] {
            assert_eq!(
                context.call(&to_string, value, &[]).unwrap(),
                Value::String(JsString::from(expected))
            );
        }
        assert_eq!(
            context
                .call(&value_of, Value::Object(object.clone()), &[])
                .unwrap(),
            Value::Object(object.clone())
        );
        assert_eq!(
            context
                .call(&to_locale_string, Value::Object(object), &[])
                .unwrap(),
            Value::String(JsString::from("[object Object]"))
        );
    }

    #[test]
    fn native_function_retains_and_dispatches_in_its_defining_realm() {
        let runtime = Runtime::new();
        let defining_context = runtime.new_context();
        let defining_realm = defining_context.realm;
        let function_prototype = defining_context.function_prototype().unwrap();
        let callable = runtime
            .callable_from_value(Value::Object(function_prototype.clone()))
            .unwrap();

        let before_context_drop = runtime
            .0
            .state
            .borrow()
            .heap
            .context_strong_count(defining_realm)
            .unwrap();
        drop(defining_context);
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context_strong_count(defining_realm),
            Ok(before_context_drop - 1)
        );
        assert!(matches!(
            runtime.bytecode_for_callable(&callable).unwrap(),
            CallableExecution::Native {
                target: NativeFunctionId::FunctionPrototype,
                realm,
                min_readable_args: 0,
            } if realm == defining_realm
        ));

        let mut caller_context = runtime.new_context();
        assert_eq!(
            caller_context
                .call(&callable, Value::Undefined, &[])
                .unwrap(),
            Value::Undefined
        );

        drop(callable);
        drop(function_prototype);
        runtime.run_gc().unwrap();
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .context(defining_realm)
                .is_err()
        );
        assert_eq!(runtime.heap_counts().context_nodes, 1);
    }

    #[test]
    fn native_call_preserves_actual_argc_padding_and_restores_active_frame() {
        let runtime = Runtime::new();
        let defining_context = runtime.new_context();
        let defining_realm = defining_context.realm;
        let function_prototype = defining_context.function_prototype().unwrap();
        let probe = runtime
            .new_bound_native_function(
                &function_prototype,
                defining_realm,
                NativeFunctionId::ArgumentProbe,
                2,
            )
            .unwrap();
        runtime
            .define_function_data_property(probe.as_object(), "length", Value::Int(99), false, true)
            .unwrap();
        let length = runtime.intern_property_key("length").unwrap();
        assert!(runtime.delete_property(probe.as_object(), &length).unwrap());
        assert_eq!(
            runtime
                .get_own_property(probe.as_object(), &length)
                .unwrap(),
            None
        );

        let caller_context = runtime.new_context();
        let no_args = runtime
            .call_internal(caller_context.realm, &probe, Value::Undefined, &[])
            .unwrap();
        assert_eq!(
            no_args,
            Completion::Return(Value::String(JsString::from("0|2|2|false")))
        );
        let extra_args = runtime
            .call_internal(
                caller_context.realm,
                &probe,
                Value::Undefined,
                &[Value::Int(1), Value::Int(2), Value::Int(3)],
            )
            .unwrap();
        assert_eq!(
            extra_args,
            Completion::Return(Value::String(JsString::from("3|3|0|false")))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert_eq!(
            runtime
                .call_internal(
                    caller_context.realm,
                    &probe,
                    Value::Undefined,
                    &[Value::Bool(false)],
                )
                .unwrap(),
            Completion::Throw(Value::String(JsString::from("native probe throw")))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert!(matches!(
            runtime.call_internal(
                caller_context.realm,
                &probe,
                Value::Undefined,
                &[Value::Bool(true)],
            ),
            Err(RuntimeError::Invariant("native probe engine error"))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn native_constructor_bit_is_independent_from_generic_cproto() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let probe = runtime
            .new_bound_native_function(
                &function_prototype,
                context.realm,
                NativeFunctionId::ArgumentProbe,
                0,
            )
            .unwrap();

        assert!(!runtime.is_constructor(probe.as_object()).unwrap());
        runtime
            .set_constructor_bit(probe.as_object(), true)
            .unwrap();
        assert!(runtime.is_constructor(probe.as_object()).unwrap());
        assert_eq!(
            context.construct(&probe, &[]).unwrap(),
            Value::String(JsString::from("0|0|0|true"))
        );
        runtime
            .set_constructor_bit(probe.as_object(), false)
            .unwrap();
        assert!(!runtime.is_constructor(probe.as_object()).unwrap());

        let ordinary = context.new_object().unwrap();
        runtime.set_constructor_bit(&ordinary, true).unwrap();
        assert!(runtime.is_constructor(&ordinary).unwrap());
    }

    #[test]
    fn native_constructor_cproto_adapters_use_defining_realm_and_restore_frames() {
        let runtime = Runtime::new();
        let defining_context = runtime.new_context();
        let defining_realm = defining_context.realm;
        let function_prototype = defining_context.function_prototype().unwrap();
        let constructor_only = runtime
            .new_bound_native_function(
                &function_prototype,
                defining_realm,
                NativeFunctionId::ConstructorProbe,
                0,
            )
            .unwrap();
        let constructor_or_function = runtime
            .new_bound_native_function(
                &function_prototype,
                defining_realm,
                NativeFunctionId::ConstructorOrFunctionProbe,
                0,
            )
            .unwrap();
        let caller_context = runtime.new_context();

        let called_without_new = runtime
            .call_internal(
                caller_context.realm,
                &constructor_only,
                Value::Undefined,
                &[],
            )
            .unwrap();
        let Completion::Throw(Value::Object(exception)) = called_without_new else {
            panic!("constructor-only native did not throw an object");
        };
        let defining_type_error_prototype = runtime
            .0
            .state
            .borrow()
            .heap
            .context(defining_realm)
            .unwrap()
            .native_error_prototypes[NativeErrorKind::Type.index()]
        .unwrap();
        assert_eq!(
            runtime
                .get_prototype_of(&exception)
                .unwrap()
                .unwrap()
                .object_id(),
            defining_type_error_prototype
        );
        let message = runtime.intern_property_key("message").unwrap();
        assert!(matches!(
            runtime.get_own_property(&exception, &message).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                ..
            }) if value == JsString::from("must be called with new")
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert_eq!(
            runtime
                .construct_internal(
                    caller_context.realm,
                    &constructor_only,
                    &constructor_only,
                    &[],
                )
                .unwrap(),
            Completion::Return(Value::String(JsString::from("0|0|0|true")))
        );
        assert_eq!(
            runtime
                .call_internal(
                    caller_context.realm,
                    &constructor_or_function,
                    Value::Undefined,
                    &[],
                )
                .unwrap(),
            Completion::Return(Value::String(JsString::from("0|0|0|false")))
        );
        assert_eq!(
            runtime
                .construct_internal(
                    caller_context.realm,
                    &constructor_or_function,
                    &constructor_or_function,
                    &[],
                )
                .unwrap(),
            Completion::Return(Value::String(JsString::from("0|0|0|true")))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn unified_active_frames_preserve_order_caller_pc_and_defining_realms() {
        let runtime = Runtime::new();
        let outer_context = runtime.new_context();
        let native_context = runtime.new_context();
        let callback_context = runtime.new_context();

        let function_prototype = native_context.function_prototype().unwrap();
        let probe = runtime
            .new_bound_native_function(
                &function_prototype,
                native_context.realm,
                NativeFunctionId::ActiveFrameProbe,
                0,
            )
            .unwrap();
        let callback = bytecode_callable(
            &runtime,
            &callback_context,
            vec![
                Instruction::GetArg(0),
                Instruction::Call(0),
                Instruction::Return,
            ],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: false,
                ..FunctionMetadata::default()
            },
        );
        let outer = bytecode_callable(
            &runtime,
            &outer_context,
            vec![
                Instruction::GetArg(0),
                Instruction::GetArg(1),
                Instruction::Call(1),
                Instruction::Return,
            ],
            FunctionMetadata {
                argument_count: 2,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let (outer_bytecode, callback_bytecode) = {
            let state = runtime.0.state.borrow();
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &state
                .heap
                .object(outer.as_object().object_id())
                .unwrap()
                .payload
            else {
                panic!("outer probe caller was not bytecode");
            };
            let outer_bytecode = *bytecode;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &state
                .heap
                .object(callback.as_object().object_id())
                .unwrap()
                .payload
            else {
                panic!("probe callback was not bytecode");
            };
            (outer_bytecode, *bytecode)
        };

        assert_eq!(
            runtime
                .call_internal(
                    outer_context.realm,
                    &outer,
                    Value::Undefined,
                    &[
                        Value::Object(probe.as_object().clone()),
                        Value::Object(callback.as_object().clone()),
                    ],
                )
                .unwrap(),
            Completion::Return(Value::Undefined)
        );

        let snapshot = runtime
            .0
            .state
            .borrow_mut()
            .active_frame_probe_snapshots
            .pop()
            .expect("deep native probe should capture the active chain");
        assert_eq!(snapshot.len(), 4);
        assert_eq!(snapshot[0].function, outer.as_object().object_id());
        assert_eq!(snapshot[0].realm, outer_context.realm);
        assert!(snapshot[0].flags.strict);
        assert!(matches!(
            snapshot[0].kind,
            ActiveFrameKind::Bytecode {
                bytecode,
                pc: Some(pc),
            } if bytecode == outer_bytecode && pc.index() == 2
        ));
        assert_eq!(snapshot[1].function, probe.as_object().object_id());
        assert_eq!(snapshot[1].realm, native_context.realm);
        assert!(matches!(
            snapshot[1].kind,
            ActiveFrameKind::Native {
                target: NativeFunctionId::ActiveFrameProbe,
                actual_arg_count: 1,
                readable_arg_count: 1,
            }
        ));
        assert_eq!(snapshot[2].function, callback.as_object().object_id());
        assert_eq!(snapshot[2].realm, callback_context.realm);
        assert!(!snapshot[2].flags.strict);
        assert!(matches!(
            snapshot[2].kind,
            ActiveFrameKind::Bytecode {
                bytecode,
                pc: Some(pc),
            } if bytecode == callback_bytecode && pc.index() == 1
        ));
        assert_eq!(snapshot[3].function, probe.as_object().object_id());
        assert_eq!(snapshot[3].realm, native_context.realm);
        assert!(matches!(
            snapshot[3].kind,
            ActiveFrameKind::Native {
                target: NativeFunctionId::ActiveFrameProbe,
                actual_arg_count: 0,
                readable_arg_count: 0,
            }
        ));
        assert!(
            snapshot
                .windows(2)
                .all(|frames| frames[0].token != frames[1].token)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn unified_active_frames_restore_after_return_throw_and_engine_error() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let function_prototype = context.function_prototype().unwrap();
        let probe = runtime
            .new_bound_native_function(
                &function_prototype,
                context.realm,
                NativeFunctionId::ActiveFrameProbe,
                0,
            )
            .unwrap();
        let no_argument_call = bytecode_callable(
            &runtime,
            &context,
            vec![
                Instruction::GetArg(0),
                Instruction::Call(0),
                Instruction::Return,
            ],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let command_call = bytecode_callable(
            &runtime,
            &context,
            vec![
                Instruction::GetArg(0),
                Instruction::GetArg(1),
                Instruction::Call(1),
                Instruction::Return,
            ],
            FunctionMetadata {
                argument_count: 2,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );

        assert_eq!(
            context
                .call(
                    &no_argument_call,
                    Value::Undefined,
                    &[Value::Object(probe.as_object().clone())],
                )
                .unwrap(),
            Value::Undefined
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert_eq!(
            context.call(
                &command_call,
                Value::Undefined,
                &[Value::Object(probe.as_object().clone()), Value::Bool(false),],
            ),
            Err(RuntimeError::Exception)
        );
        assert_eq!(
            context.take_exception().unwrap(),
            Some(Value::String(JsString::from("active frame probe throw")))
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());

        assert!(matches!(
            context.call(
                &command_call,
                Value::Undefined,
                &[
                    Value::Object(probe.as_object().clone()),
                    Value::Bool(true),
                ],
            ),
            Err(RuntimeError::Engine(error))
                if error.message().contains("active frame probe engine error")
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn active_frame_guard_roots_function_and_bytecode_through_gc_and_drop_fallback() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let bytecode = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        strict: true,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let bytecode_id = bytecode.bytecode_id();
        let callable = runtime
            .new_bytecode_closure(context.realm, &bytecode)
            .unwrap();
        let function_id = callable.as_object().object_id();
        let guard = runtime
            .push_bytecode_active_frame(
                callable.as_object().clone(),
                bytecode.clone(),
                context.realm,
                true,
            )
            .unwrap();
        drop(callable);
        drop(bytecode);

        assert_eq!(runtime.run_gc().unwrap().cleanup.finalized_objects, 0);
        {
            let state = runtime.0.state.borrow();
            assert!(state.heap.object(function_id).is_ok());
            assert!(state.heap.function_bytecode(bytecode_id).is_ok());
            assert_eq!(state.active_frames.len(), 1);
        }

        drop(guard);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert!(runtime.0.state.borrow().heap.object(function_id).is_err());
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .function_bytecode(bytecode_id)
                .is_err()
        );
    }

    #[test]
    fn bytecode_active_frame_rejects_a_realm_other_than_the_bytecode_realm() {
        let runtime = Runtime::new();
        let defining_context = runtime.new_context();
        let other_context = runtime.new_context();
        let bytecode = runtime
            .publish_unlinked_function(
                defining_context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        strict: true,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let callable = runtime
            .new_bytecode_closure(defining_context.realm, &bytecode)
            .unwrap();

        assert!(matches!(
            runtime.push_bytecode_active_frame(
                callable.as_object().clone(),
                bytecode.clone(),
                other_context.realm,
                true,
            ),
            Err(RuntimeError::Invariant(
                "bytecode active frame realm disagrees with its bytecode"
            ))
        ));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn active_frame_drop_defers_nested_pops_until_the_state_borrow_ends() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let bytecode = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        strict: true,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let bytecode_id = bytecode.bytecode_id();
        let callable = runtime
            .new_bytecode_closure(context.realm, &bytecode)
            .unwrap();
        let function_id = callable.as_object().object_id();
        let outer_guard = runtime
            .push_bytecode_active_frame(
                callable.as_object().clone(),
                bytecode.clone(),
                context.realm,
                true,
            )
            .unwrap();
        let outer_token = outer_guard.token();
        let inner_guard = runtime
            .push_bytecode_active_frame(
                callable.as_object().clone(),
                bytecode.clone(),
                context.realm,
                true,
            )
            .unwrap();
        let inner_token = inner_guard.token();
        drop(callable);
        drop(bytecode);

        let state_borrow = runtime.0.state.borrow();
        drop(inner_guard);
        drop(outer_guard);

        // The state borrow forces both guard drops through the deferred path.
        // `push_front` reverses unwind order so the outer pop removes the whole
        // nested suffix before either guard releases its raw heap roots.
        assert_eq!(state_borrow.active_frames.len(), 2);
        let deferred = runtime.0.deferred_references.borrow();
        let frame_pops = deferred
            .iter()
            .filter_map(|operation| match operation {
                DeferredRefOp::ActiveFramePop { token, .. } => Some(*token),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(frame_pops, vec![outer_token, inner_token]);
        assert!(matches!(
            deferred.front(),
            Some(DeferredRefOp::ActiveFramePop { token, .. }) if *token == outer_token
        ));
        drop(deferred);
        drop(state_borrow);

        runtime.drain_deferred_references().unwrap();
        assert!(runtime.0.deferred_references.borrow().is_empty());
        let state = runtime.0.state.borrow();
        assert!(state.active_frames.is_empty());
        assert!(state.heap.object(function_id).is_err());
        assert!(state.heap.function_bytecode(bytecode_id).is_err());
    }

    #[test]
    fn fclosure_call_and_call_method_follow_quickjs_stack_layout() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let add_one = UnlinkedFunction::new(
            vec![
                Instruction::GetArg(0),
                Instruction::PushI32(1),
                Instruction::Add,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                argument_count: 1,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let caller = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::PushI32(41),
                Instruction::Call(1),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(add_one)],
            FunctionMetadata {
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let caller = runtime
            .publish_unlinked_function(context.realm, caller)
            .unwrap();
        let caller = runtime
            .new_bytecode_closure(context.realm, &caller)
            .unwrap();
        assert_eq!(
            context.call(&caller, Value::Undefined, &[]).unwrap(),
            Value::Int(42)
        );

        let return_this = UnlinkedFunction::new(
            vec![Instruction::PushThis, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let method_caller = UnlinkedFunction::new(
            vec![
                Instruction::PushThis,
                Instruction::FClosure(0),
                Instruction::CallMethod(0),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(return_this)],
            FunctionMetadata {
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let method_caller = runtime
            .publish_unlinked_function(context.realm, method_caller)
            .unwrap();
        let method_caller = runtime
            .new_bytecode_closure(context.realm, &method_caller)
            .unwrap();
        let receiver = runtime.new_object(None).unwrap();
        assert_eq!(
            context
                .call(&method_caller, Value::Object(receiver.clone()), &[])
                .unwrap(),
            Value::Object(receiver)
        );
    }

    #[test]
    fn nested_call_propagates_throw_without_publishing_it_early() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let throwing = UnlinkedFunction::new(
            vec![Instruction::PushI32(9), Instruction::Throw],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let caller = UnlinkedFunction::new(
            vec![
                Instruction::FClosure(0),
                Instruction::Call(0),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(throwing)],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let caller = runtime
            .publish_unlinked_function(context.realm, caller)
            .unwrap();
        let caller = runtime
            .new_bytecode_closure(context.realm, &caller)
            .unwrap();

        assert_eq!(
            context.call(&caller, Value::Undefined, &[]),
            Err(RuntimeError::Exception)
        );
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
    }

    #[test]
    fn push_this_applies_strict_and_sloppy_callee_realm_rules() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        let code = vec![Instruction::PushThis, Instruction::Return];

        let sloppy = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    code.clone(),
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let sloppy = runtime
            .new_bytecode_closure(context.realm, &sloppy)
            .unwrap();
        assert_eq!(
            context.call(&sloppy, Value::Undefined, &[]).unwrap(),
            Value::Object(global)
        );
        assert!(matches!(
            context.call(&sloppy, Value::Int(1), &[]),
            Err(RuntimeError::Engine(_))
        ));
        assert!(!context.has_exception());
        let ignores_this = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    vec![Instruction::PushI32(7), Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let ignores_this = runtime
            .new_bytecode_closure(context.realm, &ignores_this)
            .unwrap();
        assert_eq!(
            context.call(&ignores_this, Value::Int(1), &[]).unwrap(),
            Value::Int(7)
        );

        let strict = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(
                    code,
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        strict: true,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        let strict = runtime
            .new_bytecode_closure(context.realm, &strict)
            .unwrap();
        assert_eq!(
            context.call(&strict, Value::Undefined, &[]).unwrap(),
            Value::Undefined
        );
        assert_eq!(
            context.call(&strict, Value::Int(1), &[]).unwrap(),
            Value::Int(1)
        );
    }

    #[test]
    fn context_invokes_getters_and_setters_with_the_original_receiver() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let prototype = runtime.new_object(None).unwrap();
        let child = runtime.new_object(Some(&prototype)).unwrap();
        let explicit_receiver = runtime.new_object(None).unwrap();

        let getter_key = runtime.intern_property_key("getter").unwrap();
        let getter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushThis, Instruction::Return],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            runtime
                .define_own_property(
                    &prototype,
                    &getter_key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
        assert_eq!(
            context.get_property(&child, &getter_key).unwrap(),
            Value::Object(child.clone())
        );
        assert_eq!(
            context
                .get_property_with_receiver(
                    &prototype,
                    &getter_key,
                    Value::Object(explicit_receiver.clone())
                )
                .unwrap(),
            Value::Object(explicit_receiver)
        );

        let setter_key = runtime.intern_property_key("setter").unwrap();
        let setter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushFalse, Instruction::Return],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            runtime
                .define_own_property(
                    &prototype,
                    &setter_key,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
        assert!(
            context
                .set_property(&child, &setter_key, Value::Int(7))
                .unwrap()
        );

        let throwing_key = runtime.intern_property_key("throwing-setter").unwrap();
        let throwing_setter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::GetArg(0), Instruction::Throw],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            runtime
                .define_own_property(
                    &prototype,
                    &throwing_key,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(throwing_setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
        assert_eq!(
            context.set_property(&child, &throwing_key, Value::Int(9)),
            Err(RuntimeError::Exception)
        );
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));

        let faulting_key = runtime.intern_property_key("faulting-setter").unwrap();
        let faulting_setter = bytecode_callable(
            &runtime,
            &context,
            vec![
                Instruction::GetArg(0),
                Instruction::PushI32(1),
                Instruction::Add,
                Instruction::Return,
            ],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            runtime
                .define_own_property(
                    &prototype,
                    &faulting_key,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(faulting_setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
        assert_eq!(
            context.set_property(&child, &faulting_key, Value::BigInt(JsBigInt::one())),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("expected setter TypeError");
        };
        assert!(runtime.is_error_object(&error).unwrap());
    }

    #[test]
    fn prepared_getter_action_keeps_callable_alive_after_property_deletion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = runtime.new_object(None).unwrap();
        let key = runtime.intern_property_key("x").unwrap();
        let getter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushI32(42), Instruction::Return],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            runtime
                .define_own_property(
                    &object,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Callable(getter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );

        let action = runtime.prepare_get_property(&object, &key).unwrap();
        assert!(runtime.delete_property(&object, &key).unwrap());
        let PropertyGetAction::Call { getter, receiver } = action else {
            panic!("expected a rooted getter action");
        };
        assert_eq!(
            context.call(&getter, receiver, &[]).unwrap(),
            Value::Int(42)
        );
        assert_eq!(
            context.get_property(&object, &key).unwrap(),
            Value::Undefined
        );
    }

    #[test]
    fn prepared_setter_action_roots_callable_receiver_and_argument() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = runtime.new_object(None).unwrap();
        let argument = runtime.new_object(None).unwrap();
        let key = runtime.intern_property_key("x").unwrap();
        let setter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::GetArg(0), Instruction::Return],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        assert!(
            runtime
                .define_own_property(
                    &object,
                    &key,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );

        let action = runtime
            .prepare_set_property(&object, &key, Value::Object(argument.clone()))
            .unwrap();
        assert!(runtime.delete_property(&object, &key).unwrap());
        drop(argument);
        let super::PropertySetAction::Call {
            setter,
            receiver,
            argument,
        } = action
        else {
            panic!("expected a rooted setter action");
        };
        let returned = context.call(&setter, receiver, &[argument]).unwrap();
        assert!(matches!(returned, Value::Object(_)));
        assert_eq!(
            context.get_property(&object, &key).unwrap(),
            Value::Undefined
        );
    }

    #[test]
    fn publication_rejects_out_of_range_frame_operands() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let function = UnlinkedFunction::new(
            vec![Instruction::GetArg(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );

        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

        let function = UnlinkedFunction::new(
            vec![Instruction::DeleteVar(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

        let function = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                function_name_local: Some(0),
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, function),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    #[test]
    fn publication_rejects_malformed_function_name_metadata_and_writes() {
        fn descriptor(
            source: ClosureSource,
            is_lexical: bool,
            is_const: bool,
            kind: ClosureVariableKind,
        ) -> ClosureVariable {
            ClosureVariable {
                source,
                name: ClosureVariableName::None,
                is_lexical,
                is_const,
                kind,
            }
        }

        fn child(code: Vec<Instruction>, mut descriptor: ClosureVariable) -> UnlinkedFunction {
            let constants = if descriptor.kind == ClosureVariableKind::FunctionName {
                descriptor.name = ClosureVariableName::Constant(0);
                vec![UnlinkedConstant::primitive(Value::String(JsString::from("self"))).unwrap()]
            } else {
                Vec::new()
            };
            UnlinkedFunction::new_with_closure_variables(
                code,
                constants,
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![descriptor],
            )
        }

        fn parent(
            child: UnlinkedFunction,
            metadata: FunctionMetadata,
            name: Option<&str>,
        ) -> UnlinkedFunction {
            let function = UnlinkedFunction::new(
                vec![Instruction::FClosure(0), Instruction::Return],
                vec![UnlinkedConstant::child(child)],
                metadata,
            );
            function.with_name(name.map(JsString::from))
        }

        fn named_parent(child: UnlinkedFunction, strict: bool) -> UnlinkedFunction {
            parent(
                child,
                FunctionMetadata {
                    local_count: 1,
                    function_name_local: Some(0),
                    max_stack: 1,
                    strict,
                    ..FunctionMetadata::default()
                },
                Some("self"),
            )
        }

        let runtime = Runtime::new();
        let context = runtime.new_context();
        let reject = |function| {
            assert!(matches!(
                runtime.publish_unlinked_function(context.realm, function),
                Err(RuntimeError::Engine(_))
            ));
            assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        };

        for name in [None, Some("")] {
            reject(
                UnlinkedFunction::new(
                    vec![Instruction::GetLocal(0), Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        local_count: 1,
                        function_name_local: Some(0),
                        max_stack: 1,
                        ..FunctionMetadata::default()
                    },
                )
                .with_name(name.map(JsString::from)),
            );
        }

        reject(parent(
            child(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                descriptor(
                    ClosureSource::ParentArgument(0),
                    false,
                    false,
                    ClosureVariableKind::FunctionName,
                ),
            ),
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            None,
        ));
        reject(parent(
            child(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                descriptor(
                    ClosureSource::ParentLocal(0),
                    false,
                    false,
                    ClosureVariableKind::FunctionName,
                ),
            ),
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            None,
        ));
        reject(named_parent(
            child(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                descriptor(
                    ClosureSource::ParentLocal(0),
                    false,
                    false,
                    ClosureVariableKind::Normal,
                ),
            ),
            false,
        ));
        reject(named_parent(
            UnlinkedFunction::new_with_closure_variables(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                vec![UnlinkedConstant::primitive(Value::String(JsString::from("other"))).unwrap()],
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source: ClosureSource::ParentLocal(0),
                    name: ClosureVariableName::Constant(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::FunctionName,
                }],
            ),
            false,
        ));

        for (strict, is_lexical, is_const) in [
            (true, false, false),
            (false, false, true),
            (false, true, false),
        ] {
            reject(named_parent(
                child(
                    vec![Instruction::GetVarRef(0), Instruction::Return],
                    descriptor(
                        ClosureSource::ParentLocal(0),
                        is_lexical,
                        is_const,
                        ClosureVariableKind::FunctionName,
                    ),
                ),
                strict,
            ));
        }

        let inner = child(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            descriptor(
                ClosureSource::ParentClosure(0),
                false,
                false,
                ClosureVariableKind::Normal,
            ),
        );
        let middle = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![
                UnlinkedConstant::child(inner),
                UnlinkedConstant::primitive(Value::String(JsString::from("self"))).unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: ClosureVariableName::Constant(1),
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::FunctionName,
            }],
        );
        reject(named_parent(middle, false));

        for code in [
            vec![
                Instruction::PushI32(1),
                Instruction::PutLocal(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::SetLocal(0),
                Instruction::Return,
            ],
        ] {
            reject(
                UnlinkedFunction::new(
                    code,
                    Vec::new(),
                    FunctionMetadata {
                        local_count: 1,
                        function_name_local: Some(0),
                        max_stack: 1,
                        ..FunctionMetadata::default()
                    },
                )
                .with_name(Some(JsString::from("self"))),
            );
        }

        for code in [
            vec![
                Instruction::PushI32(1),
                Instruction::PutVarRef(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::SetVarRef(0),
                Instruction::Return,
            ],
        ] {
            reject(named_parent(
                child(
                    code,
                    descriptor(
                        ClosureSource::ParentLocal(0),
                        false,
                        false,
                        ClosureVariableKind::FunctionName,
                    ),
                ),
                false,
            ));
        }
    }

    #[test]
    fn publication_rejects_mixed_global_and_lexical_closure_opcodes() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let reject = |function| {
            assert!(matches!(
                runtime.publish_unlinked_function(context.realm, function),
                Err(RuntimeError::Engine(_))
            ));
            assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        };

        for code in [
            vec![Instruction::GetVar(0), Instruction::Return],
            vec![Instruction::GetVarUndef(0), Instruction::Return],
            vec![Instruction::DeleteVar(0), Instruction::Return],
            vec![
                Instruction::PushI32(1),
                Instruction::PutVar(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::PutVarInit(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
        ] {
            let child = UnlinkedFunction::new_with_closure_variables(
                code,
                Vec::new(),
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source: ClosureSource::ParentLocal(0),
                    name: ClosureVariableName::None,
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                }],
            );
            reject(UnlinkedFunction::new(
                vec![Instruction::FClosure(0), Instruction::Return],
                vec![UnlinkedConstant::child(child)],
                FunctionMetadata {
                    local_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
            ));
        }

        for code in [
            vec![Instruction::GetVarRef(0), Instruction::Return],
            vec![
                Instruction::PushI32(1),
                Instruction::PutVarRef(0),
                Instruction::Undefined,
                Instruction::Return,
            ],
            vec![
                Instruction::PushI32(1),
                Instruction::SetVarRef(0),
                Instruction::Return,
            ],
        ] {
            reject(UnlinkedFunction::new_with_closure_variables(
                code,
                vec![UnlinkedConstant::primitive(Value::String(JsString::from("global"))).unwrap()],
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source: ClosureSource::Global,
                    name: ClosureVariableName::Constant(0),
                    is_lexical: false,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                }],
            ));
        }
    }

    #[test]
    fn put_var_init_initializes_a_const_global_lexical_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .create_global_lexical_for_test("initializedLexical", true, None)
            .unwrap();
        let function = UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::PushI32(9),
                Instruction::PutVarInit(0),
                Instruction::GetVar(0),
                Instruction::Return,
            ],
            vec![
                UnlinkedConstant::primitive(Value::String(JsString::from("initializedLexical")))
                    .unwrap(),
            ],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::Global,
                name: ClosureVariableName::Constant(0),
                is_lexical: true,
                is_const: true,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let function = runtime
            .publish_unlinked_function(context.realm, function)
            .unwrap();
        let function = runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap();
        assert_eq!(
            context.call(&function, Value::Undefined, &[]).unwrap(),
            Value::Int(9)
        );
        assert!(matches!(
            context.eval("initializedLexical = 10"),
            Err(RuntimeError::Exception)
        ));
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("const lexical reassignment did not throw an object");
        };
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from("'initializedLexical' is read-only"))
        );
    }

    #[test]
    fn publication_preflights_closure_descriptors_before_heap_changes() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let missing_descriptor = UnlinkedFunction::new(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, missing_descriptor),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);

        let out_of_bounds_child = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: crate::heap::ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let parent = UnlinkedFunction::new(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(out_of_bounds_child)],
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, parent),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    #[test]
    fn publication_rejects_inconsistent_closure_metadata() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let child = |is_lexical| {
            UnlinkedFunction::new_with_closure_variables(
                vec![Instruction::GetVarRef(0), Instruction::Return],
                Vec::new(),
                FunctionMetadata {
                    closure_count: 1,
                    max_stack: 1,
                    ..FunctionMetadata::default()
                },
                vec![ClosureVariable {
                    source: ClosureSource::ParentLocal(0),
                    name: crate::heap::ClosureVariableName::None,
                    is_lexical,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                }],
            )
        };
        let inconsistent_siblings = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            vec![
                UnlinkedConstant::child(child(false)),
                UnlinkedConstant::child(child(true)),
            ],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, inconsistent_siblings),
            Err(RuntimeError::Engine(_))
        ));

        let illegal_const = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentClosure(0),
                name: crate::heap::ClosureVariableName::None,
                is_lexical: false,
                is_const: true,
                kind: ClosureVariableKind::Normal,
            }],
        );
        assert!(matches!(
            runtime.publish_unlinked_function(context.realm, illegal_const),
            Err(RuntimeError::Engine(_))
        ));
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
    }

    #[test]
    fn deeply_nested_child_publication_and_release_are_iterative() {
        const DEPTH: usize = 50_000;

        let runtime = Runtime::new();
        let context = runtime.new_context();
        let metadata = FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        };
        let mut function = UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            metadata,
        );
        for _ in 0..DEPTH {
            function = UnlinkedFunction::new(
                vec![Instruction::Undefined, Instruction::Return],
                vec![UnlinkedConstant::child(function)],
                metadata,
            );
        }

        let function = runtime
            .publish_unlinked_function(context.realm, function)
            .unwrap();
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, DEPTH + 1);
        drop(function);
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, 0);
        assert_eq!(runtime.heap_counts().context_nodes, 1);
    }

    #[test]
    fn function_closures_share_runtime_rooted_var_ref_cells() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
        let function = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new_with_closure_variables(
                    vec![
                        Instruction::GetVarRef(0),
                        Instruction::PushI32(1),
                        Instruction::Add,
                        Instruction::SetVarRef(0),
                        Instruction::Return,
                    ],
                    Vec::new(),
                    FunctionMetadata {
                        closure_count: 1,
                        max_stack: 2,
                        ..FunctionMetadata::default()
                    },
                    vec![ClosureVariable {
                        source: ClosureSource::ParentClosure(0),
                        name: crate::heap::ClosureVariableName::None,
                        is_lexical: false,
                        is_const: false,
                        kind: ClosureVariableKind::Normal,
                    }],
                ),
            )
            .unwrap();
        let cell = runtime
            .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
            .unwrap();
        let cell_id = cell.id();
        let first = runtime
            .new_bytecode_closure_with_slots(context.realm, &function, std::slice::from_ref(&cell))
            .unwrap();
        let second = runtime
            .new_bytecode_closure_with_slots(context.realm, &function, std::slice::from_ref(&cell))
            .unwrap();
        assert_eq!(
            runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
            Ok(3)
        );

        let mut caller = context.clone();
        assert_eq!(
            caller.call(&first, Value::Undefined, &[]).unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            caller.call(&second, Value::Undefined, &[]).unwrap(),
            Value::Int(3)
        );

        runtime.write_var_ref(&cell, Value::Int(7)).unwrap();
        assert_eq!(runtime.read_var_ref(&cell).unwrap(), Value::Int(7));
        drop(cell);
        assert_eq!(
            runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
            Ok(2)
        );
        let promoted = VarRefRoot::from_borrowed_handle(runtime.clone(), cell_id).unwrap();
        assert_eq!(runtime.read_var_ref(&promoted).unwrap(), Value::Int(7));
        drop(first);
        drop(second);
        assert_eq!(
            runtime.0.state.borrow().heap.var_ref_strong_count(cell_id),
            Ok(1)
        );
        drop(promoted);
        assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
    }

    fn incrementing_closure(source: ClosureSource) -> UnlinkedFunction {
        UnlinkedFunction::new_with_closure_variables(
            vec![
                Instruction::GetVarRef(0),
                Instruction::PushI32(1),
                Instruction::Add,
                Instruction::SetVarRef(0),
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source,
                name: crate::heap::ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        )
    }

    #[test]
    fn fclosure_captures_parent_local_and_isolates_each_invocation() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline_var_refs = runtime.heap_counts().var_ref_nodes;
        let parent = UnlinkedFunction::new(
            vec![
                Instruction::PushI32(10),
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(incrementing_closure(
                ClosureSource::ParentLocal(0),
            ))],
            FunctionMetadata {
                local_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let parent = runtime
            .publish_unlinked_function(context.realm, parent)
            .unwrap();
        let parent = runtime
            .new_bytecode_closure(context.realm, &parent)
            .unwrap();

        let first = context
            .call(&parent, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();
        let second = context
            .call(&parent, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();
        assert_eq!(
            context.call(&first, Value::Undefined, &[]).unwrap(),
            Value::Int(11)
        );
        assert_eq!(
            context.call(&first, Value::Undefined, &[]).unwrap(),
            Value::Int(12)
        );
        assert_eq!(
            context.call(&second, Value::Undefined, &[]).unwrap(),
            Value::Int(11)
        );

        drop(first);
        drop(second);
        assert_eq!(runtime.heap_counts().var_ref_nodes, baseline_var_refs);
    }

    #[test]
    fn parent_local_writes_after_fclosure_update_the_shared_cell() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let child = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::GetVarRef(0), Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentLocal(0),
                name: crate::heap::ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let parent = UnlinkedFunction::new(
            vec![
                Instruction::PushI32(1),
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::PushI32(7),
                Instruction::PutLocal(0),
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(child)],
            FunctionMetadata {
                local_count: 1,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let parent = runtime
            .publish_unlinked_function(context.realm, parent)
            .unwrap();
        let parent = runtime
            .new_bytecode_closure(context.realm, &parent)
            .unwrap();
        let child = context
            .call(&parent, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();

        assert_eq!(
            context.call(&child, Value::Undefined, &[]).unwrap(),
            Value::Int(7)
        );
    }

    #[test]
    fn repeated_fclosure_in_one_frame_reuses_the_parent_cell() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let parent = UnlinkedFunction::new(
            vec![
                Instruction::PushI32(0),
                Instruction::PutLocal(0),
                Instruction::FClosure(0),
                Instruction::FClosure(0),
                Instruction::Call(0),
                Instruction::Drop,
                Instruction::Return,
            ],
            vec![UnlinkedConstant::child(incrementing_closure(
                ClosureSource::ParentLocal(0),
            ))],
            FunctionMetadata {
                local_count: 1,
                max_stack: 2,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let parent = runtime
            .publish_unlinked_function(context.realm, parent)
            .unwrap();
        let parent = runtime
            .new_bytecode_closure(context.realm, &parent)
            .unwrap();
        let survivor = context
            .call(&parent, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();

        assert_eq!(
            context.call(&survivor, Value::Undefined, &[]).unwrap(),
            Value::Int(2)
        );
    }

    #[test]
    fn parent_argument_and_transitive_parent_closure_capture_share_identity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let inner = incrementing_closure(ClosureSource::ParentClosure(0));
        let middle = UnlinkedFunction::new_with_closure_variables(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(inner)],
            FunctionMetadata {
                closure_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
            vec![ClosureVariable {
                source: ClosureSource::ParentArgument(0),
                name: crate::heap::ClosureVariableName::None,
                is_lexical: false,
                is_const: false,
                kind: ClosureVariableKind::Normal,
            }],
        );
        let outer = UnlinkedFunction::new(
            vec![Instruction::FClosure(0), Instruction::Return],
            vec![UnlinkedConstant::child(middle)],
            FunctionMetadata {
                argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let outer = runtime
            .publish_unlinked_function(context.realm, outer)
            .unwrap();
        let outer = runtime.new_bytecode_closure(context.realm, &outer).unwrap();
        let middle = context
            .call(&outer, Value::Undefined, &[Value::Int(40)])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();
        let first_inner = context
            .call(&middle, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();
        let second_inner = context
            .call(&middle, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();

        assert_eq!(
            context.call(&first_inner, Value::Undefined, &[]).unwrap(),
            Value::Int(41)
        );
        assert_eq!(
            context.call(&second_inner, Value::Undefined, &[]).unwrap(),
            Value::Int(42)
        );
        assert_eq!(
            context.call(&first_inner, Value::Undefined, &[]).unwrap(),
            Value::Int(43)
        );

        let isolated_middle = context
            .call(&outer, Value::Undefined, &[Value::Int(40)])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();
        let isolated_inner = context
            .call(&isolated_middle, Value::Undefined, &[])
            .and_then(|value| runtime.callable_from_value(value))
            .unwrap();
        assert_eq!(
            context
                .call(&isolated_inner, Value::Undefined, &[])
                .unwrap(),
            Value::Int(41)
        );
    }

    #[test]
    fn executing_foreign_runtime_bytecode_is_rejected_before_instantiation() {
        let first = Runtime::new();
        let second = Runtime::new();
        let mut compiler_context = first.new_context();
        let function = compiler_context.compile("42").unwrap();
        let mut caller_context = second.new_context();
        let caller_realm_objects = second.heap_counts().object_nodes;

        assert!(matches!(
            caller_context.execute(&function),
            Err(RuntimeError::WrongRuntime("function bytecode"))
        ));
        assert_eq!(second.heap_counts().object_nodes, caller_realm_objects);
    }

    #[test]
    fn pending_exception_slot_owns_and_transfers_object_roots() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let object_id = object.object_id();
        runtime
            .set_pending_exception(Value::Object(object.clone()))
            .unwrap();
        assert!(runtime.has_pending_exception());
        assert_eq!(
            runtime.0.state.borrow().heap.object_strong_count(object_id),
            Ok(2)
        );
        drop(object);

        let exception = runtime.take_pending_exception().unwrap().unwrap();
        assert!(!runtime.has_pending_exception());
        assert!(matches!(
            &exception,
            Value::Object(value) if value.object_id() == object_id
        ));
        assert_eq!(
            runtime.0.state.borrow().heap.object_strong_count(object_id),
            Ok(1)
        );
        drop(exception);
        assert_eq!(runtime.heap_counts().object_nodes, 0);
    }

    #[test]
    fn pending_exception_roots_survive_gc_and_preserve_symbol_identity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let object = runtime.new_object(None).unwrap();
        let object_id = object.object_id();
        let self_key = runtime.intern_property_key("self").unwrap();
        assert!(
            context
                .set_property(&object, &self_key, Value::Object(object.clone()))
                .unwrap()
        );
        runtime
            .set_pending_exception(Value::Object(object.clone()))
            .unwrap();
        drop(object);

        assert_eq!(runtime.run_gc().unwrap().cleanup.finalized_objects, 0);
        let exception = runtime.take_pending_exception().unwrap().unwrap();
        assert!(matches!(
            &exception,
            Value::Object(object) if object.object_id() == object_id
        ));
        drop(exception);
        assert!(runtime.run_gc().unwrap().cleanup.finalized_objects >= 1);

        let symbol = runtime.new_symbol(Some(JsString::from("boom"))).unwrap();
        let expected = symbol.clone();
        runtime
            .set_pending_exception(Value::Symbol(symbol))
            .unwrap();
        let exception = runtime.take_pending_exception().unwrap().unwrap();
        assert!(matches!(exception, Value::Symbol(symbol) if symbol == expected));
    }

    #[test]
    fn throw_completion_moves_the_value_into_the_runtime_exception_slot() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval("throw 9"), Err(RuntimeError::Exception));
        assert!(context.has_exception());
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(9)));
        assert!(!context.has_exception());
    }

    #[test]
    fn vm_fault_materializes_a_native_error_in_the_callee_realm() {
        let runtime = Runtime::new();
        let mut compiler_context = runtime.new_context();
        let function = compiler_context.compile("1n + 1").unwrap();
        let expected_prototype = runtime
            .0
            .state
            .borrow()
            .heap
            .context(compiler_context.realm)
            .unwrap()
            .native_error_prototypes[NativeErrorKind::Type.index()]
        .unwrap();
        let mut caller_context = runtime.new_context();
        let caller_prototype = runtime
            .0
            .state
            .borrow()
            .heap
            .context(caller_context.realm)
            .unwrap()
            .native_error_prototypes[NativeErrorKind::Type.index()]
        .unwrap();
        assert_ne!(expected_prototype, caller_prototype);

        assert_eq!(
            caller_context.execute(&function),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = caller_context.take_exception().unwrap().unwrap() else {
            panic!("expected a native Error object");
        };
        assert!(runtime.is_error_object(&error).unwrap());
        assert!(matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(error.object_id())
                .unwrap()
                .payload,
            ObjectPayload::Error
        ));
        assert_eq!(
            runtime
                .get_prototype_of(&error)
                .unwrap()
                .unwrap()
                .object_id(),
            expected_prototype
        );

        let message = runtime.intern_property_key("message").unwrap();
        let CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } = runtime.get_own_property(&error, &message).unwrap().unwrap()
        else {
            panic!("native Error message must be an own data property");
        };
        assert_eq!(
            value,
            Value::String(JsString::from("cannot convert bigint to number"))
        );
        assert!(writable);
        assert!(!enumerable);
        assert!(configurable);

        let name = runtime.intern_property_key("name").unwrap();
        assert_eq!(
            caller_context.get_property(&error, &name).unwrap(),
            Value::String(JsString::from("TypeError"))
        );
        let prototype = runtime.get_prototype_of(&error).unwrap().unwrap();
        assert!(!runtime.is_error_object(&prototype).unwrap());
    }

    #[test]
    fn nested_fault_non_callable_and_compile_syntax_use_exception_completion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        assert_eq!(
            context.eval("(function(){ return 1n + 1; })()"),
            Err(RuntimeError::Exception)
        );
        let Value::Object(nested) = context.take_exception().unwrap().unwrap() else {
            panic!("expected nested TypeError");
        };
        assert!(runtime.is_error_object(&nested).unwrap());

        assert_eq!(context.eval("(1)()"), Err(RuntimeError::Exception));
        let Value::Object(not_callable) = context.take_exception().unwrap().unwrap() else {
            panic!("expected non-callable TypeError");
        };
        assert!(matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(not_callable.object_id())
                .unwrap()
                .payload,
            ObjectPayload::Error
        ));

        assert_eq!(context.compile("throw\n9"), Err(RuntimeError::Exception));
        let Value::Object(syntax) = context.take_exception().unwrap().unwrap() else {
            panic!("expected SyntaxError");
        };
        assert!(matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(syntax.object_id())
                .unwrap()
                .payload,
            ObjectPayload::Error
        ));
        let name = runtime.intern_property_key("name").unwrap();
        assert_eq!(
            context.get_property(&syntax, &name).unwrap(),
            Value::String(JsString::from("SyntaxError"))
        );
    }

    #[test]
    fn rooted_handles_enforce_runtime_domain_and_dup_free_counts() {
        let first = Runtime::new();
        let second = Runtime::new();
        let first_key = first.intern_property_key("first").unwrap();
        let second_key = second.intern_property_key("second").unwrap();
        assert_eq!(first_key.atom().raw(), second_key.atom().raw());
        assert_ne!(first_key, second_key);

        let object = first.new_object(None).unwrap();
        assert!(matches!(
            second.define_own_property(
                &object,
                &second_key,
                &data_descriptor(Value::Int(1), true, true, true)
            ),
            Err(RuntimeError::WrongRuntime("object"))
        ));
        assert!(matches!(
            first.define_own_property(
                &object,
                &second_key,
                &data_descriptor(Value::Int(1), true, true, true)
            ),
            Err(RuntimeError::WrongRuntime("property key"))
        ));
        let foreign_object = second.new_object(None).unwrap();
        assert!(matches!(
            set_property(&first, &object, &first_key, Value::Object(foreign_object)),
            Err(RuntimeError::WrongRuntime("property value"))
        ));

        assert_eq!(
            first
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(object.object_id()),
            Ok(1)
        );
        let value = Value::Object(object.clone());
        assert_eq!(
            first
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(object.object_id()),
            Ok(2)
        );
        let duplicate = value.clone();
        assert_eq!(
            first
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(object.object_id()),
            Ok(3)
        );
        drop(duplicate);
        drop(value);
        assert_eq!(
            first
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(object.object_id()),
            Ok(1)
        );
    }

    #[test]
    fn shape_sharing_and_descriptor_defaults_follow_quickjs_layout() {
        let runtime = Runtime::new();
        let first = runtime.new_object(None).unwrap();
        let second = runtime.new_object(None).unwrap();
        let key = runtime.intern_property_key("x").unwrap();

        let empty_shapes = {
            let state = runtime.0.state.borrow();
            (
                state.heap.object(first.object_id()).unwrap().shape,
                state.heap.object(second.object_id()).unwrap().shape,
            )
        };
        assert_eq!(empty_shapes.0, empty_shapes.1);

        let defaulted = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Int(7)),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(
            runtime
                .define_own_property(&first, &key, &defaulted)
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(&second, &key, &defaulted)
                .unwrap()
        );
        assert_eq!(
            runtime.get_own_property(&first, &key).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(7),
                writable: false,
                enumerable: false,
                configurable: false,
            })
        );
        let populated_shapes = {
            let state = runtime.0.state.borrow();
            (
                state.heap.object(first.object_id()).unwrap().shape,
                state.heap.object(second.object_id()).unwrap().shape,
            )
        };
        assert_eq!(populated_shapes.0, populated_shapes.1);
        assert_ne!(populated_shapes.0, empty_shapes.0);
    }

    #[test]
    fn own_keys_preserve_quickjs_category_order_and_utf16_identity() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let symbol_a = runtime.new_symbol(Some(JsString::from("a"))).unwrap();
        let symbol_b = runtime.new_symbol(Some(JsString::from("b"))).unwrap();
        let symbol_key_a = PropertyKey::from(&symbol_a);
        let symbol_key_b = PropertyKey::from(&symbol_b);

        for (key, value) in [
            (runtime.intern_property_key("beta").unwrap(), 1),
            (runtime.intern_property_key("4294967295").unwrap(), 2),
            (runtime.intern_property_key("2147483648").unwrap(), 3),
            (runtime.intern_property_key("01").unwrap(), 4),
            (runtime.intern_property_key("4294967294").unwrap(), 5),
            (runtime.intern_property_key("0").unwrap(), 6),
            (runtime.intern_property_key("-0").unwrap(), 7),
        ] {
            assert!(set_property(&runtime, &object, &key, Value::Int(value)).unwrap());
        }
        assert!(set_property(&runtime, &object, &symbol_key_a, Value::Int(8)).unwrap());
        assert!(
            set_property(
                &runtime,
                &object,
                &runtime.intern_property_key("2").unwrap(),
                Value::Int(9)
            )
            .unwrap()
        );
        assert!(set_property(&runtime, &object, &symbol_key_b, Value::Int(10)).unwrap());

        let expected = [
            runtime.intern_property_key("0").unwrap(),
            runtime.intern_property_key("2").unwrap(),
            runtime.intern_property_key("2147483648").unwrap(),
            runtime.intern_property_key("4294967294").unwrap(),
            runtime.intern_property_key("beta").unwrap(),
            runtime.intern_property_key("4294967295").unwrap(),
            runtime.intern_property_key("01").unwrap(),
            runtime.intern_property_key("-0").unwrap(),
            symbol_key_a.clone(),
            symbol_key_b.clone(),
        ];
        assert_eq!(runtime.own_property_keys(&object).unwrap(), expected);

        let surrogate = runtime
            .intern_property_key_js_string(&JsString::from_utf16([0xd800]))
            .unwrap();
        let replacement = runtime
            .intern_property_key_js_string(&JsString::from_utf16([0xfffd]))
            .unwrap();
        assert_ne!(surrogate, replacement);
        assert!(set_property(&runtime, &object, &surrogate, Value::Int(11)).unwrap());
        assert!(set_property(&runtime, &object, &replacement, Value::Int(12)).unwrap());
        assert_eq!(
            runtime.property_key_to_js_string(&surrogate).unwrap(),
            JsString::from_utf16([0xd800])
        );
    }

    #[test]
    fn delete_readd_and_frozen_same_value_rules_match_oracle() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let a = runtime.intern_property_key("a").unwrap();
        let b = runtime.intern_property_key("b").unwrap();
        let c = runtime.intern_property_key("c").unwrap();
        for key in [&a, &b, &c] {
            assert!(set_property(&runtime, &object, key, Value::Int(1)).unwrap());
        }
        assert!(runtime.delete_property(&object, &a).unwrap());
        assert!(set_property(&runtime, &object, &a, Value::Int(2)).unwrap());
        assert_eq!(
            runtime.own_property_keys(&object).unwrap(),
            vec![b.clone(), c.clone(), a.clone()]
        );

        let nan = runtime.intern_property_key("nan").unwrap();
        let zero = runtime.intern_property_key("zero").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &object,
                    &nan,
                    &data_descriptor(Value::Float(f64::NAN), false, true, false)
                )
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(
                    &object,
                    &zero,
                    &data_descriptor(Value::Int(0), false, true, false)
                )
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(
                    &object,
                    &nan,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Float(f64::NAN)),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
        assert!(
            !runtime
                .define_own_property(
                    &object,
                    &nan,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Int(0)),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
        assert!(
            !runtime
                .define_own_property(
                    &object,
                    &zero,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Float(-0.0)),
                        ..OrdinaryPropertyDescriptor::new()
                    }
                )
                .unwrap()
        );
    }

    #[test]
    fn inherited_set_and_prototype_constraints_match_ordinary_semantics() {
        let runtime = Runtime::new();
        let parent = runtime.new_object(None).unwrap();
        let writable = runtime.intern_property_key("writable").unwrap();
        let readonly = runtime.intern_property_key("readonly").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &parent,
                    &writable,
                    &data_descriptor(Value::Int(1), true, true, true)
                )
                .unwrap()
        );
        assert!(
            runtime
                .define_own_property(
                    &parent,
                    &readonly,
                    &data_descriptor(Value::Int(1), false, true, true)
                )
                .unwrap()
        );
        let child = runtime.new_object(Some(&parent)).unwrap();
        assert!(set_property(&runtime, &child, &writable, Value::Int(2)).unwrap());
        assert!(!set_property(&runtime, &child, &readonly, Value::Int(2)).unwrap());
        assert_eq!(
            get_property(&runtime, &child, &writable).unwrap(),
            Value::Int(2)
        );
        assert_eq!(
            get_property(&runtime, &child, &readonly).unwrap(),
            Value::Int(1)
        );

        let receiver = runtime.new_object(None).unwrap();
        assert!(
            set_property_with_receiver(
                &runtime,
                &parent,
                &writable,
                Value::Int(3),
                Value::Object(receiver.clone()),
            )
            .unwrap()
        );
        assert_eq!(
            get_property(&runtime, &parent, &writable).unwrap(),
            Value::Int(1)
        );
        assert_eq!(
            get_property(&runtime, &receiver, &writable).unwrap(),
            Value::Int(3)
        );

        let mut context = runtime.new_context();
        let Value::Object(receiver_setter) = context.eval("(function(value) {})").unwrap() else {
            panic!("receiver setter probe did not produce a function");
        };
        let receiver_setter = runtime.as_callable(&receiver_setter).unwrap().unwrap();
        let accessor_receiver = runtime.new_object(None).unwrap();
        assert!(
            runtime
                .define_own_property(
                    &accessor_receiver,
                    &writable,
                    &OrdinaryPropertyDescriptor {
                        get: DescriptorField::Present(AccessorValue::Undefined),
                        set: DescriptorField::Present(AccessorValue::Callable(receiver_setter)),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );
        assert!(
            !set_property_with_receiver(
                &runtime,
                &parent,
                &writable,
                Value::Int(4),
                Value::Object(accessor_receiver),
            )
            .unwrap()
        );

        let fixed = runtime.new_object(None).unwrap();
        runtime.prevent_extensions(&fixed).unwrap();
        assert!(runtime.set_prototype_of(&fixed, None).unwrap());
        assert!(!runtime.set_prototype_of(&fixed, Some(&parent)).unwrap());

        let first = runtime.new_object(None).unwrap();
        let second = runtime.new_object(None).unwrap();
        assert!(runtime.set_prototype_of(&first, Some(&second)).unwrap());
        assert!(!runtime.set_prototype_of(&second, Some(&first)).unwrap());
        assert_eq!(
            runtime.get_prototype_of(&first).unwrap(),
            Some(second.clone())
        );
        assert_eq!(runtime.get_prototype_of(&second).unwrap(), None);
    }

    #[test]
    fn object_property_cycle_is_collected_only_by_explicit_gc() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let self_key = runtime.intern_property_key("self").unwrap();
        assert!(set_property(&runtime, &object, &self_key, Value::Object(object.clone())).unwrap());
        assert_eq!(runtime.heap_counts().object_nodes, 1);
        let state = runtime.0.state.borrow_mut();
        drop(object);
        drop(state);
        let stats = runtime.run_gc().unwrap();
        assert_eq!(stats.cleanup.finalized_objects, 1);
        assert_eq!(runtime.heap_counts().object_nodes, 0);
    }

    #[test]
    fn named_function_self_capture_cycle_is_collected() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let baseline = runtime.heap_counts();
        let closure = context
            .eval(
                "(function() {\
                    var child;\
                    var owner = function self() {\
                        child = function() { return self; };\
                        return child;\
                    };\
                    return owner();\
                })()",
            )
            .unwrap();
        let retained = runtime.heap_counts();
        assert!(retained.object_nodes >= baseline.object_nodes + 2);
        assert!(retained.var_ref_nodes >= baseline.var_ref_nodes + 2);
        drop(closure);
        assert!(runtime.heap_counts().object_nodes > baseline.object_nodes);

        let stats = runtime.run_gc().unwrap();
        assert!(stats.cleanup.finalized_objects >= 2);
        assert!(stats.cleanup.finalized_var_refs >= 2);
        let collected = runtime.heap_counts();
        assert_eq!(collected.object_nodes, baseline.object_nodes);
        assert_eq!(collected.var_ref_nodes, baseline.var_ref_nodes);
        assert_eq!(
            collected.function_bytecode_nodes,
            baseline.function_bytecode_nodes
        );
    }

    #[test]
    fn symbols_are_runtime_owned_and_distinct_from_registry_entries() {
        let runtime = Runtime::new();
        let name = JsString::from("Symbol.iterator");
        let unique = runtime.well_known_symbol(WellKnownSymbol::Iterator);
        let repeated = runtime.well_known_symbol(WellKnownSymbol::Iterator);
        let registry = runtime.symbol_for(&name).unwrap();
        assert_eq!(unique, repeated);
        assert_ne!(unique, registry);
        assert_ne!(PropertyKey::from(&unique), PropertyKey::from(&registry));
        assert_eq!(runtime.symbol_key_for(&unique).unwrap(), None);
        assert_eq!(runtime.symbol_key_for(&registry).unwrap(), Some(name));
    }

    #[test]
    fn exceptional_vm_exit_releases_local_frame_roots_immediately() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushConst(0),
                Instruction::PushConst(1),
                Instruction::PushI32(1),
                Instruction::Add,
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![
                Value::Object(object.clone()),
                Value::BigInt(JsBigInt::one()),
            ],
            max_stack: 3,
        };
        let before = runtime
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id())
            .unwrap();
        assert!(super::Vm::new().execute(&function).is_err());
        let after = runtime
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(object.object_id())
            .unwrap();
        assert_eq!(after, before);
    }

    #[test]
    fn drops_during_runtime_borrow_are_deferred_to_the_next_safe_point() {
        let runtime = Runtime::new();
        let object = runtime.new_object(None).unwrap();
        let key = runtime.intern_property_key("queued").unwrap();
        let state = runtime.0.state.borrow_mut();
        drop(object);
        drop(key);
        assert_eq!(runtime.0.deferred_references.borrow().len(), 2);
        drop(state);

        let context = runtime.new_context();
        assert!(runtime.0.deferred_references.borrow().is_empty());
        assert_eq!(runtime.heap_counts().context_nodes, 1);
        drop(context);
        runtime.run_gc().unwrap();
        assert_eq!(runtime.heap_counts().object_nodes, 0);
    }

    #[test]
    fn finalized_shapes_unlink_exact_weak_cache_entries() {
        let runtime = Runtime::new();
        let mut objects = Vec::new();
        for index in 0..2_000 {
            let object = runtime.new_object(None).unwrap();
            let key = runtime
                .intern_property_key(&format!("unique-{index}"))
                .unwrap();
            assert!(set_property(&runtime, &object, &key, Value::Int(index)).unwrap());
            objects.push(object);
        }
        assert!(runtime.0.state.borrow().shape_cache.len() >= objects.len());
        drop(objects);
        let state = runtime.0.state.borrow();
        assert!(state.shape_cache.is_empty());
        assert!(state.shape_fingerprints.is_empty());
    }

    fn debug_draft(debug: UnlinkedFunctionDebug) -> UnlinkedFunction {
        UnlinkedFunction::new(
            vec![Instruction::Undefined, Instruction::Return],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_debug(debug)
    }

    #[test]
    fn publication_rejects_malformed_debug_pc_order_range_source_and_position() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let baseline = runtime.heap_counts().function_bytecode_nodes;

        let malformed = [
            UnlinkedFunctionDebug {
                filename: JsString::from("range.js"),
                pc2line: Some(Pc2LineTable::new(
                    LineColumn::new(0, 0),
                    vec![Pc2LineEntry {
                        pc: 2,
                        position: LineColumn::new(0, 0),
                    }],
                )),
                source: None,
            },
            UnlinkedFunctionDebug {
                filename: JsString::from("order.js"),
                pc2line: Some(Pc2LineTable::new(
                    LineColumn::new(0, 0),
                    vec![
                        Pc2LineEntry {
                            pc: 1,
                            position: LineColumn::new(0, 1),
                        },
                        Pc2LineEntry {
                            pc: 0,
                            position: LineColumn::new(0, 0),
                        },
                    ],
                )),
                source: None,
            },
            UnlinkedFunctionDebug {
                filename: JsString::from("utf8.js"),
                pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
                source: Some(vec![0xff].into_boxed_slice()),
            },
            UnlinkedFunctionDebug {
                filename: JsString::from("position.js"),
                pc2line: Some(Pc2LineTable::new(LineColumn::new(u32::MAX, 0), Vec::new())),
                source: None,
            },
        ];

        for debug in malformed {
            assert!(
                runtime
                    .publish_unlinked_function(context.realm, debug_draft(debug))
                    .is_err()
            );
            assert_eq!(
                runtime.heap_counts().function_bytecode_nodes,
                baseline,
                "malformed debug metadata changed the heap"
            );
        }
    }

    #[test]
    fn publication_keeps_duplicate_last_and_unreachable_pc_metadata() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let duplicate = debug_draft(UnlinkedFunctionDebug {
            filename: JsString::from("duplicate.js"),
            pc2line: Some(Pc2LineTable::new(
                LineColumn::new(0, 0),
                vec![
                    Pc2LineEntry {
                        pc: 0,
                        position: LineColumn::new(1, 1),
                    },
                    Pc2LineEntry {
                        pc: 0,
                        position: LineColumn::new(2, 2),
                    },
                ],
            )),
            source: None,
        });
        let duplicate = runtime
            .publish_unlinked_function(context.realm, duplicate)
            .unwrap();
        assert_eq!(
            runtime
                .test_function_debug_location(&duplicate, Some(0))
                .unwrap(),
            Some((JsString::from("duplicate.js"), LineColumn::new(2, 2)))
        );

        let unreachable = UnlinkedFunction::new(
            vec![
                Instruction::Goto(2),
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                max_stack: 1,
                ..FunctionMetadata::default()
            },
        )
        .with_debug(UnlinkedFunctionDebug {
            filename: JsString::from("unreachable.js"),
            pc2line: Some(Pc2LineTable::new(
                LineColumn::new(0, 0),
                vec![Pc2LineEntry {
                    pc: 1,
                    position: LineColumn::new(9, 4),
                }],
            )),
            source: None,
        });
        let unreachable = runtime
            .publish_unlinked_function(context.realm, unreachable)
            .unwrap();
        assert_eq!(
            runtime
                .test_function_debug_location(&unreachable, Some(1))
                .unwrap(),
            Some((JsString::from("unreachable.js"), LineColumn::new(9, 4)))
        );
    }

    #[test]
    fn publication_rollback_releases_the_new_debug_filename_atom() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let stale_realm = context.realm;
        drop(context);
        runtime.run_gc().unwrap();
        let baseline_atoms = runtime.test_atom_count();
        let baseline_bytecode = runtime.heap_counts().function_bytecode_nodes;

        let function = debug_draft(UnlinkedFunctionDebug {
            filename: JsString::from("rollback-debug-filename.js"),
            pc2line: Some(Pc2LineTable::new(LineColumn::new(0, 0), Vec::new())),
            source: None,
        });
        assert!(
            runtime
                .publish_unlinked_function(stale_realm, function)
                .is_err()
        );
        assert_eq!(runtime.test_atom_count(), baseline_atoms);
        assert_eq!(
            runtime.heap_counts().function_bytecode_nodes,
            baseline_bytecode
        );
    }
}
