//! Callback-based `%TypedArray%.prototype` find algorithms.
//!
//! Pinned QuickJS validates and snapshots the branded view once, then visits
//! every index in that original range without a `HasProperty` check. Each
//! element read remains live: shrink, detach, regrow, and callback writes are
//! observed at the next index while growth never extends the traversal.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_find(
        &self,
        realm: ContextId,
        kind: ArrayFindKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype find received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => i64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let predicate = self.callable_from_value(
            arguments
                .readable
                .first()
                .ok_or(RuntimeError::Invariant(
                    "TypedArray find predicate argv was not padded",
                ))?
                .clone(),
        )?;
        let this_arg = if arguments.actual_arg_count > 1 {
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "TypedArray find thisArg was missing",
                ))?
                .clone()
        } else {
            Value::Undefined
        };
        let receiver = Value::Object(target.clone());
        let (mut index, end, direction) = match kind {
            ArrayFindKind::Find | ArrayFindKind::FindIndex => (0, length, 1),
            ArrayFindKind::FindLast | ArrayFindKind::FindLastIndex => (length - 1, -1, -1),
        };

        while index != end {
            let value = self
                .typed_array_read_index(
                    &target,
                    u64::try_from(index).map_err(|_| {
                        RuntimeError::Invariant("TypedArray find index was negative")
                    })?,
                )?
                .unwrap_or(Value::Undefined);
            let index_value = Value::number(index as f64);
            let matches = match self.call_internal(
                realm,
                &predicate,
                this_arg.clone(),
                &[value.clone(), index_value.clone(), receiver.clone()],
            )? {
                Completion::Return(value) => value.to_boolean(),
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if matches {
                return Ok(Completion::Return(match kind {
                    ArrayFindKind::Find | ArrayFindKind::FindLast => value,
                    ArrayFindKind::FindIndex | ArrayFindKind::FindLastIndex => index_value,
                }));
            }
            index += direction;
        }
        Ok(Completion::Return(match kind {
            ArrayFindKind::Find | ArrayFindKind::FindLast => Value::Undefined,
            ArrayFindKind::FindIndex | ArrayFindKind::FindLastIndex => Value::Int(-1),
        }))
    }
}
