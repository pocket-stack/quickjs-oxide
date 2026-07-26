//! In-place `%TypedArray%.prototype` mutation algorithms.
//!
//! Pinned QuickJS operates directly on the branded backing view after one
//! initial length snapshot and a post-coercion bounds revalidation. Keeping
//! these methods together makes those resize/detach rules explicit without
//! growing the runtime facade or the shared constructor/indexed-property owner.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_copy_within(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.copyWithin received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial_length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => i64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let to = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "TypedArray.copyWithin target argv was not padded",
            ))?,
            0,
            initial_length,
            initial_length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let from = match self.native_to_int64_clamp(
            realm,
            arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "TypedArray.copyWithin start argv was not padded",
            ))?,
            0,
            initial_length,
            initial_length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let final_index = if arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined))
        {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(2).ok_or(RuntimeError::Invariant(
                    "TypedArray.copyWithin end argv was missing",
                ))?,
                0,
                initial_length,
                initial_length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            initial_length
        };

        let current = self.typed_array_state(&target)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }
        // QuickJS caps the initial copy count by the live space remaining
        // after hostile coercions resize a length-tracking backing store.
        let live_space = i64::from(current.length) - to.max(from);
        let count = (final_index - from)
            .min(initial_length - to)
            .min(live_space);
        if count > 0 {
            let to = u64::try_from(to).map_err(|_| {
                RuntimeError::Invariant("TypedArray.copyWithin target was negative")
            })?;
            let from = u64::try_from(from).map_err(|_| {
                RuntimeError::Invariant("TypedArray.copyWithin source was negative")
            })?;
            let count = usize::try_from(count).map_err(|_| {
                RuntimeError::Invariant("TypedArray.copyWithin count overflowed usize")
            })?;
            let width = usize::from(current.snapshot.element.byte_length());
            let source_start = typed_array_absolute_byte_offset(current.snapshot, from)?;
            let target_start = typed_array_absolute_byte_offset(current.snapshot, to)?;
            let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
                "TypedArray.copyWithin byte count overflowed usize",
            ))?;
            self.0.state.borrow_mut().heap.move_array_buffer_range(
                current.snapshot.buffer,
                current.snapshot.buffer,
                source_start,
                target_start,
                byte_count,
            )?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(super) fn call_typed_array_fill(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.fill received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial_length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => i64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let element = self.typed_array_snapshot(&target)?.element;
        // QuickJS converts the fill value before either bound. The resulting
        // machine word is retained across later resize/detach side effects.
        let converted = match self.typed_array_convert_element(
            realm,
            element,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "TypedArray.fill value argv was not padded",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start = if arguments.actual_arg_count > 1 {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "TypedArray.fill start argv was missing",
                ))?,
                0,
                initial_length,
                initial_length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            0
        };
        let end = if arguments.actual_arg_count > 2
            && !matches!(arguments.readable.get(2), Some(Value::Undefined))
        {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(2).ok_or(RuntimeError::Invariant(
                    "TypedArray.fill end argv was missing",
                ))?,
                0,
                initial_length,
                initial_length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            initial_length
        };

        let current = self.typed_array_state(&target)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "out of bound",
            )?));
        }
        let end = end.min(i64::from(current.length));
        if start < end {
            let count = usize::try_from(end - start)
                .map_err(|_| RuntimeError::Invariant("TypedArray.fill count overflowed usize"))?;
            let start = u64::try_from(start)
                .map_err(|_| RuntimeError::Invariant("TypedArray.fill start was negative"))?;
            let width = usize::from(element.byte_length());
            let byte_start = typed_array_absolute_byte_offset(current.snapshot, start)?;
            self.0.state.borrow_mut().heap.fill_array_buffer_words(
                current.snapshot.buffer,
                byte_start,
                &converted[..width],
                count,
            )?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(super) fn call_typed_array_reverse(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.reverse received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let current = self.typed_array_state(&target)?;
        if current.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        }
        if current.length > 1 {
            let count = usize::try_from(current.length).map_err(|_| {
                RuntimeError::Invariant("TypedArray.reverse length overflowed usize")
            })?;
            let width = usize::from(current.snapshot.element.byte_length());
            let start = typed_array_absolute_byte_offset(current.snapshot, 0)?;
            self.0.state.borrow_mut().heap.reverse_array_buffer_words(
                current.snapshot.buffer,
                start,
                width,
                count,
            )?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }
}
