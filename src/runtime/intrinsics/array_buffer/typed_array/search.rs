//! Indexed `%TypedArray%.prototype` lookup and search algorithms.
//!
//! Pinned QuickJS snapshots the validated length before observable argument
//! coercion, then reads the live backing-view length before direct word
//! access. These methods deliberately do not reuse the generic Array kernels:
//! integer-indexed views have no holes or prototype lookup, and shrinking a
//! resizable buffer gives `includes(undefined)` a distinct observable result.

use super::*;

#[cfg(test)]
mod tests;

impl Runtime {
    pub(super) fn call_typed_array_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype.at received a constructor invocation",
            ));
        };
        let target = match self.require_typed_array(realm, this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let initial_state = self.typed_array_state(&target)?;
        if initial_state.out_of_bounds {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "ArrayBuffer is detached",
            )?));
        }
        let initial_length = i64::from(initial_state.length);
        let index = match self.native_to_int64_sat(
            realm,
            arguments.readable.first().ok_or(RuntimeError::Invariant(
                "TypedArray.at index argv was not padded",
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
        if index < 0 {
            return Ok(Completion::Return(Value::Undefined));
        }
        let index = u64::try_from(index)
            .map_err(|_| RuntimeError::Invariant("TypedArray.at index overflowed u64"))?;
        Ok(Completion::Return(
            self.typed_array_read_index(&target, index)?
                .unwrap_or(Value::Undefined),
        ))
    }

    pub(super) fn call_typed_array_search(
        &self,
        realm: ContextId,
        kind: ArraySearchKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray.prototype search received a constructor invocation",
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
        let not_found = || match kind {
            ArraySearchKind::Includes => Value::Bool(false),
            ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => Value::Int(-1),
        };
        if initial_length == 0 {
            return Ok(Completion::Return(not_found()));
        }
        let search = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "TypedArray search value argv was not padded",
        ))?;
        let from_index = if arguments.actual_arg_count > 1 {
            Some(arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "TypedArray search fromIndex argv was missing",
            ))?)
        } else {
            None
        };

        let (mut index, step) = match kind {
            ArraySearchKind::Includes | ArraySearchKind::IndexOf => {
                let index = if let Some(from_index) = from_index {
                    match self.native_to_int64_clamp(
                        realm,
                        from_index,
                        0,
                        initial_length,
                        initial_length,
                    )? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                } else {
                    0
                };
                (index, 1)
            }
            ArraySearchKind::LastIndexOf => {
                let index = if let Some(from_index) = from_index {
                    match self.native_to_int64_clamp(
                        realm,
                        from_index,
                        -1,
                        initial_length - 1,
                        initial_length,
                    )? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                } else {
                    initial_length - 1
                };
                if index < 0 {
                    return Ok(Completion::Return(not_found()));
                }
                (index, -1)
            }
        };

        let current_length = i64::from(self.typed_array_state(&target)?.length);
        // QuickJS treats integer indices that disappeared during fromIndex
        // coercion as `undefined` only for includes. indexOf/lastIndexOf scan
        // direct machine words and therefore never observe that missing tail.
        if kind == ArraySearchKind::Includes
            && matches!(search, Value::Undefined)
            && initial_length > current_length
            && index < initial_length
        {
            return Ok(Completion::Return(Value::Bool(true)));
        }

        let length = initial_length.min(current_length);
        if length == 0 {
            return Ok(Completion::Return(not_found()));
        }
        let end = match kind {
            ArraySearchKind::Includes | ArraySearchKind::IndexOf => {
                index = index.min(length);
                length
            }
            ArraySearchKind::LastIndexOf => {
                index = index.min(length - 1);
                -1
            }
        };
        while index != end {
            let value = self
                .typed_array_read_index(
                    &target,
                    u64::try_from(index).map_err(|_| {
                        RuntimeError::Invariant("TypedArray search index was negative")
                    })?,
                )?
                .ok_or(RuntimeError::Invariant(
                    "stable TypedArray search range lost an element",
                ))?;
            let matches = match kind {
                ArraySearchKind::Includes => search.same_value_zero(&value),
                ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                    search.strict_equal(&value)
                }
            };
            if matches {
                return Ok(Completion::Return(match kind {
                    ArraySearchKind::Includes => Value::Bool(true),
                    ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                        Value::number(index as f64)
                    }
                }));
            }
            index += step;
        }
        Ok(Completion::Return(not_found()))
    }
}
