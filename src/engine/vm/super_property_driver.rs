//! Preserve the pinned super read/call/write ordering around owned key conversion.
use super::{
    Completion, driver::CallStep, exception::runtime_error_to_vm_error,
    execution::RunningExecution, frame::FrameId,
};
use crate::engine::{
    api::{Error, ErrorKind, runtime::Runtime},
    value::{Value, conversion::NativeConversion},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Read,
    Call,
    Write,
}
pub(super) struct Input {
    receiver: Value,
    base: Value,
    pub key: Value,
    value: Option<Value>,
    kind: Kind,
    depth: usize,
}
pub(super) enum Progress {
    Convert(Box<Input>),
    Call(CallStep),
}
impl Input {
    #[cfg(feature = "profiling")]
    pub(super) fn operand_count(&self) -> usize {
        if self.kind == Kind::Write { 4 } else { 3 }
    }
}
pub(super) fn start(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    kind: Kind,
) -> Result<Progress, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let count = if kind == Kind::Write { 4 } else { 3 };
    for offset in 0..count {
        runtime
            .validate_value_domain(
                execution.slots.peek(&frame.window, offset)?,
                "super property input",
            )
            .map_err(runtime_error_to_vm_error)?;
    }
    let depth = execution.slots.depth(&frame.window);
    let value = if kind == Kind::Write {
        Some(execution.slots.pop(&mut frame.window)?)
    } else {
        None
    };
    let key = execution.slots.pop(&mut frame.window)?;
    let base = execution.slots.pop(&mut frame.window)?;
    let receiver = execution.slots.pop(&mut frame.window)?;
    // PutSuperValue rejects the base before converting a raw key, after RHS.
    // At a call site QuickJS uses ordinary GetArrayEl's nullish precheck.
    let error = if kind == Kind::Write && !matches!(base, Value::Object(_)) {
        Some("not an object")
    } else if kind == Kind::Call && matches!(base, Value::Null | Value::Undefined) {
        Some(if matches!(base, Value::Null) {
            "cannot read property of null"
        } else {
            "cannot read property of undefined"
        })
    } else {
        None
    };
    if let Some(message) = error {
        return super::property_driver::throw_error(
            runtime,
            realm,
            Error::new(ErrorKind::Type, message),
        )
        .map(Progress::Call);
    }
    let input = Box::new(Input {
        receiver,
        base,
        key,
        value,
        kind,
        depth,
    });
    if matches!(input.key, Value::Object(_)) {
        Ok(Progress::Convert(input))
    } else {
        converted(runtime, execution, id, input).map(Progress::Call)
    }
}
pub(super) fn converted(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    input: Box<Input>,
) -> Result<CallStep, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let strict = frame.executable.metadata.strict;
    let Input {
        receiver,
        base,
        key,
        value,
        kind,
        depth,
    } = *input;
    if matches!(key, Value::Object(_)) {
        return Err(Error::internal("super key conversion returned an object"));
    }
    let key = match runtime
        .native_to_property_key(realm, key)
        .map_err(runtime_error_to_vm_error)?
    {
        NativeConversion::Value(key) => key,
        NativeConversion::Throw(value) => return Ok(CallStep::Complete(Completion::Throw(value))),
    };
    if kind == Kind::Write {
        let Value::Object(base) = base else {
            return Err(Error::internal("super write lost its validated base"));
        };
        let value = value.ok_or_else(|| Error::internal("super write lost its value"))?;
        return super::proxy_get_driver::start_write(
            runtime, execution, id, base, key, value, receiver, strict, depth,
        );
    }
    if let Value::Object(object) = base {
        let getter_receiver = if kind == Kind::Call {
            Value::Object(object.clone())
        } else {
            receiver.clone()
        };
        if kind == Kind::Call {
            let frame = execution.frames.current_mut(id)?;
            execution.slots.push(&mut frame.window, receiver)?;
        }
        return super::proxy_get_driver::start_owned_read(
            runtime,
            execution,
            id,
            object,
            key,
            getter_receiver,
            depth,
        );
    }
    if kind == Kind::Read {
        let suffix = match base {
            Value::Null => "' of null",
            Value::Undefined => "' of undefined",
            _ => {
                return super::property_driver::throw_error(
                    runtime,
                    realm,
                    Error::new(ErrorKind::Type, "not an object"),
                );
            }
        };
        let error = runtime
            .native_atom_error(ErrorKind::Type, "cannot read property '", &key, suffix)
            .map_err(runtime_error_to_vm_error)?;
        return super::property_driver::throw_error(runtime, realm, error);
    }
    let read = runtime.prepare_value_property_read(realm, base, &key);
    let read = match read {
        Ok(read) => read,
        Err(error) => {
            return super::property_driver::throw_error(
                runtime,
                realm,
                runtime_error_to_vm_error(error),
            );
        }
    };
    super::property_driver::read_prepared(
        runtime,
        execution,
        id,
        receiver,
        key,
        read,
        None,
        kind == Kind::Call,
        0,
        depth,
    )
}
