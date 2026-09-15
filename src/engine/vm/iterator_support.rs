//! Iterator classification and snapshot kernels shared by both VM adapters.
use super::exception::runtime_error_to_vm_error;
use crate::engine::{
    api::{Error, runtime::Runtime},
    builtins::native::NativeFunctionId,
    heap::ObjectPayload,
    value::Value,
};

pub(super) fn is_direct_native_target(
    runtime: &Runtime,
    value: &Value,
    expected: NativeFunctionId,
) -> Result<bool, Error> {
    let Value::Object(object) = value else {
        return Ok(false);
    };
    if !object.belongs_to(runtime) {
        return Err(Error::internal(
            "append iterator method belongs to another runtime",
        ));
    }
    let state = runtime.0.state.borrow();
    let object = state
        .heap
        .object(object.object_id())
        .map_err(|error| Error::internal(error.to_string()))?;
    Ok(matches!(
        &object.payload,
        ObjectPayload::NativeFunction { data, .. } if data.target == expected
    ))
}

/// Snapshot the exact values used by QuickJS's `js_append_enumerate`
/// fast branch. Named properties may be interleaved in our shape, so the
/// shared fast Array/Arguments storage reader reconstructs numeric order
/// rather than slicing physical slots.
pub(super) fn append_fast_array_values(
    runtime: &Runtime,
    source: &Value,
    next_method: &Value,
    builtin_values_probe: bool,
) -> Result<Option<Vec<Value>>, Error> {
    if !builtin_values_probe
        || !is_direct_native_target(runtime, next_method, NativeFunctionId::ArrayIteratorNext)?
    {
        return Ok(None);
    }
    let Value::Object(source) = source else {
        return Ok(None);
    };
    let is_array = {
        let state = runtime.0.state.borrow();
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
    let fast_len = runtime
        .array_fast_len(source)
        .map_err(runtime_error_to_vm_error)?;
    let Some(fast_len) = fast_len else {
        return Ok(None);
    };
    let (length, _) = runtime
        .array_length_state(source)
        .map_err(runtime_error_to_vm_error)?;
    if length != fast_len {
        return Ok(None);
    }

    runtime
        .fast_array_like_values(source, fast_len)
        .map_err(runtime_error_to_vm_error)
}

pub(super) fn check_result_object(value: &Value) -> Result<(), Error> {
    if !matches!(value, Value::Object(_)) {
        return Err(Error::new(
            crate::engine::api::ErrorKind::Type,
            "iterator must return an object",
        ));
    }
    Ok(())
}

pub(super) fn missing_throw() -> Error {
    Error::new(
        crate::engine::api::ErrorKind::Type,
        "iterator does not have a throw method",
    )
}
