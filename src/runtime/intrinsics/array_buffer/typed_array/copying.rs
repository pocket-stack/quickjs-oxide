//! Non-species copying `%TypedArray%.prototype` algorithms.
//!
//! Pinned QuickJS implements `with` and `toReversed` by allocating the source
//! element class in the builtin's defining realm, copying the branded source,
//! and then applying the requested mutation to that private result.  The
//! source's public constructor and `Symbol.species` are never observed.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_with(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.with received a constructor invocation",
            ));
        };
        let source = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial = self.typed_array_state(&source)?;
        if initial.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let initial_length = i64::from(initial.length);
        let index = match self.native_to_int64_sat(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "TypedArray.with index argv was not padded",
            ))?,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let index = if index < 0 {
            initial_length + index
        } else {
            index
        };

        // QuickJS performs number-hint ToPrimitive before revalidating the
        // index. The eventual element conversion remains part of the write to
        // the freshly copied result.
        let replacement = match self.to_primitive(
            realm,
            arguments
                .readable
                .get(1)
                .ok_or(RuntimeError::Invariant(
                    "TypedArray.with replacement argv was not padded",
                ))?
                .clone(),
            ToPrimitiveHint::Number,
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let current = self.typed_array_state(&source)?;
        if current.out_of_bounds || index < 0 || index >= i64::from(current.length) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array index",
            )?));
        }

        // The allocation retains the pre-coercion length. If a tracking RAB
        // shrank, QuickJS fills the missing numeric tail through ordinary
        // element conversion; a BigInt tail consequently throws.
        let target = match self.typed_array_copy_to_default(
            realm,
            &source,
            initial.snapshot.element,
            u64::from(initial.length),
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let index = u64::try_from(index)
            .map_err(|_| RuntimeError::Invariant("validated TypedArray.with index was negative"))?;
        match self.typed_array_set_index(realm, &target, index, &replacement)? {
            NativeConversion::Value(()) => {}
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    pub(super) fn call_typed_array_to_reversed(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.toReversed received a constructor invocation",
            ));
        };
        let source = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let state = self.typed_array_state(&source)?;
        let target = match self.typed_array_copy_to_default(
            realm,
            &source,
            state.snapshot.element,
            u64::from(state.length),
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target_state = self.typed_array_state(&target)?;
        if target_state.length > 1 {
            let count = usize::try_from(target_state.length).map_err(|_| {
                RuntimeError::Invariant("TypedArray.toReversed length overflowed usize")
            })?;
            let width = usize::from(target_state.snapshot.element.byte_length());
            let start = typed_array_absolute_byte_offset(target_state.snapshot, 0)?;
            self.0.state.borrow_mut().heap.reverse_array_buffer_words(
                target_state.snapshot.buffer,
                start,
                width,
                count,
            )?;
        }
        Ok(Completion::Return(Value::Object(target)))
    }

    /// QuickJS's internal same-class TypedArray constructor used by copying
    /// methods. The target always has the builtin's defining-realm prototype.
    fn typed_array_copy_to_default(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        element: TypedArrayElementKind,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let prototype = self.typed_array_default_prototype(realm, element)?;
        let source_snapshot = self.typed_array_snapshot(source)?;
        self.typed_array_copy_into_new(realm, &prototype, element, source, source_snapshot, length)
    }

    /// Shared tail of QuickJS `js_typed_array_constructor_ta`: validate the
    /// source around target allocation, preserve same-class machine words when
    /// the complete requested range remains live, and otherwise perform the
    /// element-by-element conversion path.
    pub(super) fn typed_array_copy_into_new(
        &self,
        realm: ContextId,
        prototype: &ObjectRef,
        target_element: TypedArrayElementKind,
        source: &ObjectRef,
        source_snapshot: TypedArraySnapshot,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let source_state = self.typed_array_state_from_snapshot(source_snapshot)?;
        if source_state.out_of_bounds {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        }
        let target =
            match self.new_typed_array_for_length(realm, prototype, target_element, length)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };

        let source_state = self.typed_array_state(source)?;
        if source_state.out_of_bounds {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached or resized",
            )?));
        }
        let target_state = self.typed_array_state(&target)?;
        if source_snapshot.element == target_element && u64::from(source_state.length) >= length {
            let byte_count = usize::try_from(
                length
                    .checked_mul(u64::from(target_element.byte_length()))
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray copy byte length overflowed u64",
                    ))?,
            )
            .map_err(|_| RuntimeError::Invariant("TypedArray copy byte length overflowed usize"))?;
            self.0.state.borrow_mut().heap.copy_array_buffer_range(
                source_snapshot.buffer,
                target_state.snapshot.buffer,
                usize::try_from(source_snapshot.byte_offset).map_err(|_| {
                    RuntimeError::Invariant("TypedArray source byteOffset overflowed usize")
                })?,
                byte_count,
            )?;
        } else {
            for index in 0..length {
                let value = self
                    .typed_array_read_index(source, index)?
                    .unwrap_or(Value::Undefined);
                match self.typed_array_set_index(realm, &target, index, &value)? {
                    NativeConversion::Value(()) => {}
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                }
            }
        }
        Ok(NativeConversion::Value(target))
    }
}
