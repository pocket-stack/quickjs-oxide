//! `%TypedArray%.prototype` stringification algorithms.
//!
//! Pinned QuickJS uses a dedicated TypedArray kernel rather than the generic
//! Array join path. It validates the branded view up front, snapshots the old
//! element count, and then keeps resizable-buffer changes observable without
//! consulting ordinary `length` or indexed properties.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_join(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_typed_array_join_with_string_limit(
            realm,
            kind,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    fn call_typed_array_join_with_string_limit(
        &self,
        realm: ContextId,
        kind: ArrayJoinKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray stringification received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial_length = match self.typed_array_validated_length(realm, &target)? {
            NativeConversion::Value(value) => u64::from(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut current_length = initial_length;
        let separator = match kind {
            ArrayJoinKind::ToLocaleString => JsString::from_static(","),
            ArrayJoinKind::Join
                if arguments.actual_arg_count == 0
                    || matches!(arguments.readable.first(), Some(Value::Undefined)) =>
            {
                JsString::from_static(",")
            }
            ArrayJoinKind::Join => {
                let separator = arguments.readable.first().ok_or(RuntimeError::Invariant(
                    "TypedArray.join separator argv was not padded",
                ))?;
                let separator = match self.native_to_js_string(realm, separator)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                // QuickJS re-reads the live count only after an observable
                // separator conversion. A shrink clips reads, while the final
                // separator padding still reflects the old count.
                current_length = u64::from(self.typed_array_state(&target)?.length);
                separator
            }
        };
        let traversal_length = initial_length.min(current_length);
        let mut output = JsStringBuilder::with_limit(0, string_limit);

        for index in 0..traversal_length {
            if index != 0 {
                // Unlike generic Array.join, QuickJS's TypedArray kernel stops
                // immediately when separator assembly overflows.
                output.push_js_string(&separator)?;
            }
            let Some(element) = self.typed_array_read_index(&target, index)? else {
                continue;
            };
            let element = match kind {
                ArrayJoinKind::Join => match self.native_to_js_string(realm, &element)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                },
                ArrayJoinKind::ToLocaleString => {
                    let localized = match self.native_element_locale_value(realm, element)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    };
                    match self.native_to_js_string(realm, &localized)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
            };
            output.push_js_string(&element)?;
        }

        // Separator coercion can shrink or detach the source before traversal.
        // QuickJS still returns the same old-length slot shape, including the
        // zero-live-element case where `old_length - 1` separators are needed.
        for _ in current_length.max(1)..initial_length {
            output.push_js_string(&separator)?;
        }

        Ok(Completion::Return(Value::String(output.finish()?)))
    }
}
