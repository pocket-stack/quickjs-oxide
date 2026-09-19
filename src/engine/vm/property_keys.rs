//! Canonical VM property keys: never repeat user-observable coercion.
use crate::engine::api::{error::Error, runtime::Runtime};
use crate::engine::object::PropertyKey;
use crate::engine::value::Value;

pub(super) fn canonical(runtime: &Runtime, value: &crate::engine::value::JsValue) -> Result<PropertyKey, Error> {
    if let Some(key) = runtime.immediate_numeric_property_key_jsvalue(value) {
        return Ok(key);
    }
    match value {
        crate::engine::value::JsValue::Symbol(index) => {
            let atom = runtime
                .0
                .state
                .borrow()
                .atoms
                .brand(*index)
                .map_err(|error| Error::internal(error.to_string()))?;
            return PropertyKey::from_borrowed_atom(runtime.clone(), atom)
                .map_err(|error| Error::internal(error.to_string()));
        }
        crate::engine::value::JsValue::String(id) => {
            let string = runtime
                .0
                .state
                .borrow()
                .heap
                .string(*id)
                .map_err(|error| Error::internal(error.to_string()))?
                .clone();
            return runtime
                .intern_property_key_js_string(&string)
                .map_err(|error| Error::internal(error.to_string()));
        }
        value => {
            let rooted = runtime
                .root_value(value)
                .map_err(|error| Error::internal(error.to_string()))?;
            canonical_rooted(runtime, &rooted)
        }
    }
}

fn canonical_rooted(runtime: &Runtime, value: &Value) -> Result<PropertyKey, Error> {
    if let Some(key) = runtime.immediate_numeric_property_key(value) {
        return Ok(key);
    }
    match value {
        Value::Symbol(symbol) => {
            if !symbol.belongs_to(runtime) {
                return Err(Error::internal(
                    "computed method symbol belongs to another runtime",
                ));
            }
            PropertyKey::from_borrowed_atom(runtime.clone(), symbol.atom())
                .map_err(|error| Error::internal(error.to_string()))
        }
        Value::String(string) => runtime
            .intern_property_key_js_string(string)
            .map_err(|error| Error::internal(error.to_string())),
        Value::Int(value) => runtime
            .intern_property_key_js_string(&Value::Int(*value).to_js_string()?)
            .map_err(|error| Error::internal(error.to_string())),
        Value::Undefined
        | Value::Null
        | Value::Bool(_)
        | Value::Float(_)
        | Value::BigInt(_)
        | Value::Object(_) => Err(Error::internal(
            "computed property key was not canonicalized by ToPropKey",
        )),
    }
}

/// Name inference consumes a canonical key, including Symbol descriptions.
pub(super) fn computed_name(
    runtime: &Runtime,
    key: &Value,
) -> Result<crate::engine::value::JsString, Error> {
    use super::exception::runtime_error_to_vm_error;
    use crate::engine::value::JsString;
    Ok(match key {
        Value::Int(_) => key.to_js_string()?,
        Value::String(name) => name.clone(),
        Value::Symbol(symbol) => match runtime
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
    })
}

#[inline(never)]
pub(super) fn set_name(
    runtime: &Runtime,
    execution: &mut super::execution::RunningExecution,
    id: super::frame::FrameId,
    index: Option<u32>,
) -> Result<Option<Value>, Error> {
    use super::exception::runtime_error_to_vm_error;
    let frame = execution.frames.current_mut(id)?;
    let result = (|| -> Result<(), Error> {
        let name = match index {
            Some(index) => {
                let Some(crate::engine::heap::BytecodeConstant::Value(
                    crate::engine::heap::RawValue::String(name),
                )) = frame.executable.constant(index)
                else {
                    return Err(Error::internal(
                        "function-name opcode referenced a non-string constant",
                    ));
                };
                // The published bytecode node owns the constant-pool edge, so
                // the trusted read clones the payload Rc without retaining.
                runtime.0.state.borrow().heap.string_fast(*name).clone()
            }
            None => computed_name(runtime, execution.slots.peek(&frame.window, 1)?)?,
        };
        runtime
            .define_object_name(execution.slots.peek(&frame.window, 0)?, &name)
            .map_err(runtime_error_to_vm_error)
    })();
    match result {
        Ok(()) => {
            frame.resume_pc = frame
                .fault_pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("name resume PC overflow"))?;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_instruction(
                execution.slots.depth(&frame.window),
            );
            Ok(None)
        }
        Err(error) => {
            let Some(kind) =
                crate::engine::api::error::NativeErrorKind::from_javascript_error(error.kind())
            else {
                return Err(error);
            };
            Ok(Some(
                runtime
                    .new_native_error_from_error(frame.executable.realm, kind, &error)
                    .map_err(runtime_error_to_vm_error)?,
            ))
        }
    }
}
