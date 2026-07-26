//! Accumulator-based `%TypedArray%.prototype` reduction algorithms.
//!
//! Pinned QuickJS validates and snapshots the branded view before checking the
//! callback. Reduction then visits every index in that original range without
//! `HasProperty`, while each element read remains live.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_reduce(
        &self,
        realm: ContextId,
        kind: ArrayReduceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype reduce received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => u64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let callback = self.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "TypedArray reduce callback argv was not padded",
                ))?
                .clone(),
        )?;
        let receiver = Value::Object(target.clone());
        let index_at = |step: u64| match kind {
            ArrayReduceKind::Reduce => step,
            ArrayReduceKind::ReduceRight => length - step - 1,
        };
        let mut step = 0;
        let mut accumulator = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "TypedArray reduce initial value was missing",
                ))?
                .clone()
        } else {
            if length == 0 {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "empty array",
                )?));
            }
            let value = self
                .typed_array_read_index(&target, index_at(step))?
                .unwrap_or(Value::Undefined);
            step += 1;
            value
        };

        while step < length {
            let index = index_at(step);
            step += 1;
            let value = self
                .typed_array_read_index(&target, index)?
                .unwrap_or(Value::Undefined);
            accumulator = match self.call_internal(
                realm,
                &callback,
                Value::Undefined,
                &[
                    accumulator,
                    value,
                    Value::number(index as f64),
                    receiver.clone(),
                ],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
        }
        Ok(Completion::Return(accumulator))
    }
}
