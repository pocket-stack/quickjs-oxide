//! Transfer assignment inputs once, then let the object protocol own the write.
use super::{
    Completion, driver::CallStep, exception::runtime_error_to_vm_error,
    execution::RunningExecution, frame::FrameId, property_driver::PropertyProgress,
};
use crate::engine::{
    api::{Error, ErrorKind, runtime::Runtime},
    object::PropertyKey,
    value::{Value, conversion::NativeConversion},
};

pub(super) struct ConvertedWrite {
    pub base: Value,
    pub key: Value,
    pub value: Value,
}

pub(super) fn write(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    static_key: Option<u32>,
) -> Result<CallStep, Error> {
    write_progress(runtime, execution, frame, static_key).map(PropertyProgress::into_call_step)
}

pub(super) fn write_progress(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    static_key: Option<u32>,
) -> Result<PropertyProgress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let depth = execution.slots.depth(&parent.window);
    let key = if let Some(index) = static_key {
        let atom = parent
            .executable
            .property_key_atoms
            .as_ref()
            .and_then(|atoms| atoms.get(index as usize))
            .copied()
            .filter(|atom| !atom.is_null())
            .ok_or_else(|| Error::internal("property write has no linked key"))?;
        PropertyKey::from_borrowed_atom(runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))?
    } else {
        let value = execution.slots.peek(&parent.window, 1)?.clone();
        if matches!(value, Value::Object(_)) {
            return Err(Error::internal("object write key did not enter conversion"));
        }
        match runtime
            .native_to_property_key(realm, value)
            .map_err(runtime_error_to_vm_error)?
        {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => {
                return Ok(PropertyProgress::Deferred(CallStep::Complete(
                    Completion::Throw(value),
                )));
            }
        }
    };
    let (base, value, discarded_key) = {
        // Key conversion has completed; authenticate this no-callback owner
        // transfer once and end the borrow before entering object storage.
        let mut slots = execution.slots.run_window(&mut parent.window)?;
        let value = slots.pop()?;
        let discarded_key = if static_key.is_none() {
            Some(slots.pop()?)
        } else {
            None
        };
        (slots.pop()?, value, discarded_key)
    };
    drop(discarded_key);
    dispatch(runtime, execution, frame, base, key, value, depth)
}

pub(super) fn converted(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    input: Box<ConvertedWrite>,
) -> Result<CallStep, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let depth = execution.slots.depth(&parent.window) + 3;
    let ConvertedWrite { base, key, value } = *input;
    if matches!(key, Value::Object(_)) {
        return Err(Error::internal("write key conversion returned an object"));
    }
    let key = match runtime
        .native_to_property_key(realm, key)
        .map_err(runtime_error_to_vm_error)?
    {
        NativeConversion::Value(key) => key,
        NativeConversion::Throw(value) => return Ok(CallStep::Complete(Completion::Throw(value))),
    };
    dispatch(runtime, execution, frame, base, key, value, depth)
        .map(PropertyProgress::into_call_step)
}

fn dispatch(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    frame: FrameId,
    base: Value,
    key: PropertyKey,
    value: Value,
    depth: usize,
) -> Result<PropertyProgress, Error> {
    let parent = execution.frames.current_mut(frame)?;
    let realm = parent.executable.realm;
    let strict = parent.executable.metadata.strict;
    let object = match &base {
        Value::Object(_) => {
            return super::proxy_get_driver::start_receiver_write_progress(
                runtime, execution, frame, key, value, base, strict, depth,
            );
        }
        Value::Null | Value::Undefined => {
            let suffix = if matches!(base, Value::Null) {
                "' of null"
            } else {
                "' of undefined"
            };
            let error = runtime
                .native_atom_error(ErrorKind::Type, "cannot set property '", &key, suffix)
                .map_err(runtime_error_to_vm_error)?;
            return super::property_driver::throw_error(runtime, realm, error)
                .map(PropertyProgress::Deferred);
        }
        value => {
            use crate::engine::builtins::native::PrimitiveKind;
            let kind = match value {
                Value::Bool(_) => PrimitiveKind::Boolean,
                Value::Int(_) | Value::Float(_) => PrimitiveKind::Number,
                Value::String(_) => PrimitiveKind::String,
                Value::BigInt(_) => PrimitiveKind::BigInt,
                Value::Symbol(_) => PrimitiveKind::Symbol,
                _ => unreachable!(),
            };
            runtime
                .primitive_prototype_for_realm(realm, kind)
                .map_err(runtime_error_to_vm_error)?
        }
    };
    super::proxy_get_driver::start_write_progress(
        runtime, execution, frame, object, key, value, base, strict, depth,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowed_set_vm_keeps_selected_callbacks_and_strict_rejection() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
            (() => {
                let trace = '';
                const target = Object.create({set x(v) { trace += 's' + v; }});
                target[{toString() { trace += 'k'; return 'x'; }}] = 7;
                const proxy = new Proxy({}, {
                    set(t, k, v, receiver) {
                        trace += 'p' + v;
                        return Reflect.set(t, k, v, receiver);
                    }
                });
                proxy.x = 9;
                const frozen = Object.freeze({x: 1});
                frozen.x = 2;
                try { (function() { 'use strict'; frozen.x = 3; })(); }
                catch (e) { trace += e instanceof TypeError ? 't' : '?'; }
                return trace === 'ks7p9t' && proxy.x === 9 && frozen.x === 1;
            })()
        "#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn borrowed_set_vm_preserves_typed_conversion_reentry_and_throw() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
            (() => {
                const target = new Uint8Array(1), marker = {};
                let calls = 0;
                target[0] = {valueOf() { calls++; target[0] = 8; return 257; }};
                try { target[0] = {valueOf() { calls++; throw marker; }}; }
                catch (e) { if (e !== marker) return false; }
                return calls === 2 && target[0] === 1;
            })()
        "#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}
