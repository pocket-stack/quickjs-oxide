//! Pure opcode leaves shared by the synchronous consumer and owned cold entry.
use crate::engine::{
    api::{
        error::{Error, ErrorKind},
        runtime::Runtime,
    },
    code::runtime::PublishedFunctionSnapshot,
    heap::ContextId,
    heap::{BytecodeConstant, ObjectPayload},
    value::{JsString, Value},
    vm::{Completion, exception::runtime_error_to_vm_error},
};

/// Load a published value constant while its executable owns the raw edge.
/// Template objects and Symbols need the same checked retain as the old host.
pub(super) fn load_value_constant(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    index: u32,
) -> Result<Value, Error> {
    let constant = executable
        .constant(index)
        .ok_or_else(|| Error::internal("constant index is out of bounds"))?;
    match constant {
        BytecodeConstant::Value(value) => runtime
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

/// QuickJS `OP_typeof` converts one of its predefined type atoms back to
/// the atom's canonical String cell. Runtime construction pins the full
/// result set, so every realm reuses the same representation while sibling
/// runtimes remain isolated.
fn canonical_typeof_string(runtime: &Runtime, spelling: &'static str) -> Result<JsString, Error> {
    let mut state = runtime.0.state.borrow_mut();
    let atom = state
        .atoms
        .intern_static(spelling)
        .map_err(|error| runtime_error_to_vm_error(error.into()))?;
    state
        .atoms
        .to_js_string(atom)
        .map_err(|error| runtime_error_to_vm_error(error.into()))
}

pub(super) fn type_of(runtime: &Runtime, value: &Value) -> Result<JsString, Error> {
    let Value::Object(object) = value else {
        return canonical_typeof_string(runtime, value.type_of());
    };
    if !object.belongs_to(runtime) {
        return Err(Error::internal("typeof operand belongs to another runtime"));
    }
    let state = runtime.0.state.borrow();
    let object = state
        .heap
        .object(object.object_id())
        .map_err(|error| Error::internal(error.to_string()))?;
    if object.is_html_dda {
        drop(state);
        return canonical_typeof_string(runtime, "undefined");
    }
    let spelling = match &object.payload {
        ObjectPayload::NativeFunction { .. }
        | ObjectPayload::BoundFunction { .. }
        | ObjectPayload::BytecodeFunction { .. } => "function",
        ObjectPayload::Proxy(proxy) if proxy.is_callable => "function",
        ObjectPayload::Proxy(_) => "object",
        ObjectPayload::Ordinary
        | ObjectPayload::ArrayBuffer(_)
        | ObjectPayload::SharedArrayBuffer(_)
        | ObjectPayload::DataView(_)
        | ObjectPayload::TypedArray(_)
        | ObjectPayload::AsyncFunctionState(_)
        | ObjectPayload::RawJson
        | ObjectPayload::Promise(_)
        | ObjectPayload::Date(_)
        | ObjectPayload::RegExp(_)
        | ObjectPayload::Array { .. }
        | ObjectPayload::Arguments { .. }
        | ObjectPayload::ArrayIterator { .. }
        | ObjectPayload::IteratorHelper(_)
        | ObjectPayload::IteratorWrap(_)
        | ObjectPayload::AsyncFromSyncIterator(_)
        | ObjectPayload::IteratorConcat(_)
        | ObjectPayload::Map { .. }
        | ObjectPayload::MapIterator { .. }
        | ObjectPayload::Set { .. }
        | ObjectPayload::WeakMap { .. }
        | ObjectPayload::WeakSet { .. }
        | ObjectPayload::WeakRef { .. }
        | ObjectPayload::FinalizationRegistry(_)
        | ObjectPayload::SetIterator { .. }
        | ObjectPayload::ForInIterator(_)
        | ObjectPayload::Primitive(_)
        | ObjectPayload::GlobalObject { .. }
        | ObjectPayload::Error
        | ObjectPayload::StringIterator { .. }
        | ObjectPayload::RegExpStringIterator { .. }
        | ObjectPayload::Generator { .. }
        | ObjectPayload::AsyncGenerator(_) => "object",
    };
    drop(state);
    canonical_typeof_string(runtime, spelling)
}

pub(super) fn create_regexp(
    runtime: &Runtime,
    realm: ContextId,
    executable: &PublishedFunctionSnapshot,
    index: u32,
) -> Result<Completion, Error> {
    let (pattern, program) = match executable.constant(index) {
        Some(BytecodeConstant::RegExp { pattern, program }) => (pattern.clone(), program.clone()),
        Some(BytecodeConstant::Value(_) | BytecodeConstant::Function(_)) => {
            return Err(Error::internal(
                "RegExp opcode referenced a non-RegExp constant",
            ));
        }
        None => return Err(Error::internal("constant index is out of bounds")),
    };
    runtime
        .new_compiled_regexp_literal(realm, pattern, program)
        .map(|object| Completion::Return(Value::Object(object)))
        .map_err(runtime_error_to_vm_error)
}

pub(super) fn set_object_prototype(
    runtime: &Runtime,
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
    let changed = runtime
        .set_prototype_of(&object, prototype.as_ref())
        .map_err(runtime_error_to_vm_error)?;
    if !changed {
        return Err(Error::new(ErrorKind::Type, "prototype is immutable"));
    }
    Ok(Completion::Return(Value::Undefined))
}

#[cfg(feature = "stack-vm")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PureOperation {
    Constant(u32),
    AtomValue(u32),
    RegExp(u32),
    DeleteSuper,
    ConstructorWithoutNew,
    IteratorCheckObject,
    IteratorMissingThrow,
    InitializeClosure { index: u16, derived: bool },
    InitializeModuleImportCollision(u16),
    SetPrototype,
    TypeOf,
    IsUndefinedOrNull,
    IsUndefined,
    IsNull,
    TypeOfIsUndefined,
    TypeOfIsFunction,
    Branch { target: u32, when: bool },
}
#[cfg(feature = "stack-vm")]
pub(super) fn step(
    runtime: &Runtime,
    execution: &mut super::execution::RunningExecution,
    id: super::frame::FrameId,
    operation: PureOperation,
) -> Result<super::driver::CallStep, Error> {
    use super::driver::CallStep;
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    #[cfg(feature = "profiling")]
    let depth = execution.slots.depth(&frame.window);
    let outcome = perform(runtime, execution, id, operation);
    match outcome {
        Ok(target) => {
            let frame = execution.frames.current_mut(id)?;
            frame.resume_pc = match target {
                Some(target) => target,
                None => frame
                    .fault_pc
                    .checked_add(1)
                    .ok_or_else(|| Error::internal("pure operation resume PC overflow"))?,
            };
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(depth);
            Ok(CallStep::Entered)
        }
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            let value = runtime
                .new_native_error_from_error(realm, kind, &error)
                .map_err(runtime_error_to_vm_error)?;
            Ok(CallStep::Complete(Completion::Throw(value)))
        }
    }
}
#[cfg(feature = "stack-vm")]
fn perform(
    runtime: &Runtime,
    execution: &mut super::execution::RunningExecution,
    id: super::frame::FrameId,
    operation: PureOperation,
) -> Result<Option<usize>, Error> {
    let frame = execution.frames.current_mut(id)?;
    let slots = &mut execution.slots;
    use PureOperation as P;
    let result = match operation {
        P::Constant(index) => {
            let value = load_value_constant(runtime, &frame.executable, index)?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_storage(
                crate::engine::api::profiling::OwnedStorageEvent::Copy {
                    heap_root: matches!(value, Value::Object(_) | Value::Symbol(_)),
                },
            );
            value
        }
        P::IteratorCheckObject => {
            super::iterator_support::check_result_object(slots.peek(&frame.window, 0)?)?;
            return Ok(None);
        }
        P::IteratorMissingThrow => return Err(super::iterator_support::missing_throw()),
        P::AtomValue(value) => Value::String(JsString::from_fresh_decimal_u32(value)),
        P::RegExp(index) => {
            match create_regexp(runtime, frame.executable.realm, &frame.executable, index)? {
                Completion::Return(value) => value,
                Completion::Throw(_) => {
                    return Err(Error::internal(
                        "pure RegExp literal unexpectedly returned a completion throw",
                    ));
                }
            }
        }
        P::DeleteSuper => {
            slots.pop(&mut frame.window)?;
            slots.pop(&mut frame.window)?;
            slots.pop(&mut frame.window)?;
            return Err(Error::new(
                ErrorKind::Reference,
                "unsupported reference to 'super'",
            ));
        }
        P::ConstructorWithoutNew => {
            return Err(Error::new(
                ErrorKind::Type,
                "class constructors must be invoked with 'new'",
            ));
        }
        P::InitializeModuleImportCollision(index) => {
            let value = slots.pop(&mut frame.window)?;
            let descriptor = frame
                .executable
                .closure_variables
                .get(usize::from(index))
                .copied()
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            super::bindings::validate_module_import_collision(descriptor)?;
            let root = frame
                .cold
                .closure_slots
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
            runtime
                .write_var_ref(&root, value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(None);
        }
        P::InitializeClosure { index, derived } => {
            let value = slots.pop(&mut frame.window)?;
            let root = frame
                .cold
                .closure_slots
                .get(usize::from(index))
                .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?
                .clone();
            if derived {
                let descriptor = frame
                    .executable
                    .closure_variables
                    .get(usize::from(index))
                    .copied()
                    .ok_or_else(|| Error::internal("closure variable index is out of bounds"))?;
                super::bindings::initialize_derived_closure(runtime, &root, descriptor, value)?;
            } else {
                runtime
                    .write_var_ref(&root, value)
                    .map_err(runtime_error_to_vm_error)?;
            }
            return Ok(None);
        }
        P::SetPrototype => {
            let prototype = slots.pop(&mut frame.window)?;
            let object = slots.pop(&mut frame.window)?;
            let retained = object.clone();
            match set_object_prototype(runtime, object, prototype)? {
                Completion::Return(_) => retained,
                Completion::Throw(_) => {
                    return Err(Error::internal(
                        "pure literal prototype unexpectedly returned a completion throw",
                    ));
                }
            }
        }
        P::Branch { target, when } => {
            let value = slots.pop(&mut frame.window)?;
            let truthy = runtime
                .value_to_boolean(&value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok((truthy == when).then_some(target as usize));
        }
        P::TypeOf
        | P::IsUndefinedOrNull
        | P::IsUndefined
        | P::IsNull
        | P::TypeOfIsUndefined
        | P::TypeOfIsFunction => {
            let value = slots.pop(&mut frame.window)?;
            match operation {
                P::TypeOf => Value::String(type_of(runtime, &value)?),
                P::IsUndefinedOrNull => {
                    Value::Bool(matches!(value, Value::Null | Value::Undefined))
                }
                P::IsUndefined => Value::Bool(matches!(value, Value::Undefined)),
                P::IsNull => Value::Bool(matches!(value, Value::Null)),
                P::TypeOfIsUndefined => Value::Bool(
                    matches!(value, Value::Undefined)
                        || runtime
                            .value_is_html_dda(&value)
                            .map_err(runtime_error_to_vm_error)?,
                ),
                P::TypeOfIsFunction => Value::Bool(
                    !runtime
                        .value_is_html_dda(&value)
                        .map_err(runtime_error_to_vm_error)?
                        && runtime
                            .value_is_callable(&value)
                            .map_err(runtime_error_to_vm_error)?,
                ),
                _ => unreachable!(),
            }
        }
    };
    slots.push(&mut frame.window, result)?;
    Ok(None)
}
