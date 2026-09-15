//! `in` validates its RHS before key conversion; `delete` checks its base after it.
use super::{
    Completion, driver::CallStep, exception::runtime_error_to_vm_error,
    execution::RunningExecution, frame::FrameId,
};
use crate::engine::{
    api::{Error, ErrorKind, runtime::Runtime},
    object::ProxyBooleanKind,
    value::{Value, conversion::NativeConversion},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    Instance,
    Has,
    Delete,
}
pub(super) struct Input {
    base: Value,
    pub key: Value,
    kind: Kind,
    depth: usize,
}
pub(super) enum Progress {
    Convert(Box<Input>),
    Call(CallStep),
}
pub(super) fn start(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    id: FrameId,
    kind: Kind,
) -> Result<Progress, Error> {
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    for offset in 0..2 {
        runtime
            .validate_value_domain(
                execution.slots.peek(&frame.window, offset)?,
                "property predicate input",
            )
            .map_err(runtime_error_to_vm_error)?;
    }
    let depth = execution.slots.depth(&frame.window);
    let right = execution.slots.pop(&mut frame.window)?;
    let left = execution.slots.pop(&mut frame.window)?;
    if kind == Kind::Instance {
        let Value::Object(target) = right else {
            return super::property_driver::throw_error(
                runtime,
                realm,
                Error::new(ErrorKind::Type, "invalid 'instanceof' right operand"),
            )
            .map(Progress::Call);
        };
        return super::proxy_get_driver::start_instance(
            runtime, execution, id, left, target, depth,
        )
        .map(Progress::Call);
    }
    let (base, key) = if kind == Kind::Has {
        (right, left)
    } else {
        (left, right)
    };
    if kind == Kind::Has && !matches!(base, Value::Object(_)) {
        return super::property_driver::throw_error(
            runtime,
            realm,
            Error::new(ErrorKind::Type, "invalid 'in' operand"),
        )
        .map(Progress::Call);
    }
    let input = Box::new(Input {
        base,
        key,
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
    let Input {
        base,
        key,
        kind,
        depth,
    } = *input;
    let frame = execution.frames.current_mut(id)?;
    let realm = frame.executable.realm;
    let strict = frame.executable.metadata.strict;
    if matches!(key, Value::Object(_)) {
        return Err(Error::internal(
            "predicate key conversion returned an object",
        ));
    }
    let key = match runtime
        .native_to_property_key(realm, key)
        .map_err(runtime_error_to_vm_error)?
    {
        NativeConversion::Value(key) => key,
        NativeConversion::Throw(value) => return Ok(CallStep::Complete(Completion::Throw(value))),
    };
    if let Value::Object(object) = base {
        let op = if kind == Kind::Has {
            ProxyBooleanKind::Has(key)
        } else {
            ProxyBooleanKind::Delete(key)
        };
        return super::proxy_get_driver::start_boolean(
            runtime,
            execution,
            id,
            object,
            op,
            kind == Kind::Delete && strict,
            depth,
        );
    }
    if kind != Kind::Delete {
        return Err(Error::internal("in lost its validated object"));
    }
    let result = runtime
        .primitive_delete_property(&base, &key)
        .and_then(|value| runtime.finish_property_delete(NativeConversion::Value(value), strict));
    let completion = match result {
        Ok(result) => result,
        Err(error) => {
            return super::property_driver::throw_error(
                runtime,
                realm,
                runtime_error_to_vm_error(error),
            );
        }
    };
    if let Completion::Return(value) = completion {
        let frame = execution.frames.current_mut(id)?;
        execution.slots.push(&mut frame.window, value)?;
        frame.resume_pc = frame
            .fault_pc
            .checked_add(1)
            .ok_or_else(|| Error::internal("predicate resume PC overflow"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_instruction(depth);
        Ok(CallStep::Entered)
    } else {
        Ok(CallStep::Complete(completion))
    }
}
