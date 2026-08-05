//! Copying and view-producing `%TypedArray%.prototype` algorithms.
//!
//! Pinned QuickJS gives `slice` and `subarray` deliberately different
//! validation points. `slice` snapshots a valid range and copies into a
//! species result, while `subarray` keeps the durable raw view metadata even
//! when a detached or resized source currently reports length zero.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_slice(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.slice received a constructor invocation",
            ));
        };
        let source = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial_length = match self.typed_array_validated_length(realm, &source)? {
            NativeConversion::Value(value) => i64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source_snapshot = self.typed_array_snapshot(&source)?;
        let start = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "TypedArray.slice start argv was not padded",
            ))?,
            0,
            initial_length,
            initial_length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let final_index = if matches!(arguments.readable.get(1), Some(Value::Undefined)) {
            initial_length
        } else {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "TypedArray.slice end argv was not padded",
                ))?,
                0,
                initial_length,
                initial_length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };
        let requested_count = u64::try_from((final_index - start).max(0))
            .map_err(|_| RuntimeError::Invariant("TypedArray.slice count was negative"))?;
        let target = match self.typed_array_species_create(
            realm,
            &source,
            source_snapshot.element,
            requested_count,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        // QuickJS deliberately skips both post-species validations for an
        // originally empty range.
        if requested_count != 0 {
            let current_source_length = match self.typed_array_validated_length(realm, &source)? {
                NativeConversion::Value(value) => u64::from(value),
                NativeConversion::Throw(value) => {
                    return Ok(Completion::Throw(value));
                }
            };
            match self.typed_array_validated_length(realm, &target)? {
                NativeConversion::Value(_) => {}
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }

            let start = u64::try_from(start)
                .map_err(|_| RuntimeError::Invariant("TypedArray.slice start was negative"))?;
            let live_count = requested_count.min(current_source_length.saturating_sub(start));
            if live_count != 0 {
                let target_snapshot = self.typed_array_snapshot(&target)?;
                if source_snapshot.element == target_snapshot.element {
                    self.typed_array_slice_raw_copy(
                        source_snapshot,
                        target_snapshot,
                        start,
                        live_count,
                    )?;
                } else {
                    for index in 0..live_count {
                        let value = self
                            .typed_array_read_index(&source, start + index)?
                            .unwrap_or(Value::Undefined);
                        match self.typed_array_set_index(realm, &target, index, &value)? {
                            NativeConversion::Value(()) => {}
                            NativeConversion::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        }
                    }
                }
            }
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(super) fn call_typed_array_subarray(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.subarray received a constructor invocation",
            ));
        };
        let source = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source_snapshot = self.typed_array_snapshot(&source)?;
        let initial_length = i64::from(
            self.typed_array_state_from_snapshot(source_snapshot)?
                .length,
        );
        let start = match self.native_to_int64_clamp(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "TypedArray.subarray begin argv was not padded",
            ))?,
            0,
            initial_length,
            initial_length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start = u64::try_from(start)
            .map_err(|_| RuntimeError::Invariant("TypedArray.subarray start was negative"))?;
        let byte_offset = u64::from(source_snapshot.byte_offset)
            .checked_add(
                start
                    .checked_mul(u64::from(source_snapshot.element.byte_length()))
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray.subarray relative offset overflowed u64",
                    ))?,
            )
            .ok_or(RuntimeError::Invariant(
                "TypedArray.subarray byteOffset overflowed u64",
            ))?;

        let end_is_undefined = matches!(arguments.readable.get(1), Some(Value::Undefined));
        let final_index = if end_is_undefined {
            initial_length
        } else {
            match self.native_to_int64_clamp(
                realm,
                arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                    "TypedArray.subarray end argv was not padded",
                ))?,
                0,
                initial_length,
                initial_length,
            )? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };
        let count = u64::try_from(
            final_index
                - i64::try_from(start).map_err(|_| {
                    RuntimeError::Invariant("TypedArray.subarray start exceeded i64")
                })?,
        )
        .unwrap_or(0);
        let length = if end_is_undefined && source_snapshot.fixed_byte_length.is_none() {
            None
        } else {
            Some(count)
        };
        let buffer = ObjectRef::from_borrowed_handle(self.clone(), source_snapshot.buffer)?;
        let target = match self.typed_array_species_create_subarray(
            realm,
            &source,
            source_snapshot.element,
            &buffer,
            byte_offset,
            length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        Ok(Completion::Return(Value::Object(target)))
    }

    fn typed_array_slice_raw_copy(
        &self,
        source: TypedArraySnapshot,
        target: TypedArraySnapshot,
        source_index: u64,
        count: u64,
    ) -> Result<(), RuntimeError> {
        let width = usize::from(source.element.byte_length());
        let source_start = typed_array_absolute_byte_offset(source, source_index)?;
        let target_start = typed_array_absolute_byte_offset(target, 0)?;
        let count = usize::try_from(count)
            .map_err(|_| RuntimeError::Invariant("TypedArray.slice count overflowed usize"))?;
        let byte_count = count.checked_mul(width).ok_or(RuntimeError::Invariant(
            "TypedArray.slice byte count overflowed usize",
        ))?;
        let source_end = source_start
            .checked_add(byte_count)
            .ok_or(RuntimeError::Invariant(
                "TypedArray.slice source range overflowed usize",
            ))?;
        let source_access = self.snapshot_buffer_access(source.buffer)?;
        let target_access = self.snapshot_buffer_access(target.buffer)?;

        // `slice_memcpy` copies overlapping ranges in increasing byte order.
        // With same-class aligned views, one element word at a time is
        // equivalent and lets earlier writes feed later reads as mandated.
        // Always using that path also covers distinct SharedArrayBuffer
        // wrappers which alias one backing store without exposing backing
        // identity through the runtime heap.
        for index in 0..count {
            let relative = index.checked_mul(width).ok_or(RuntimeError::Invariant(
                "TypedArray.slice overlap offset overflowed usize",
            ))?;
            let bytes = self.read_buffer_word(&source_access, source_start + relative, width)?;
            self.write_buffer_word(&target_access, target_start + relative, &bytes[..width])?;
        }
        debug_assert_eq!(source_start + byte_count, source_end);
        Ok(())
    }
}
