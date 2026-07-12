//! String prototype intrinsics beyond the shared primitive-wrapper substrate.

use super::super::*;

#[cfg(test)]
mod tests;

/// Compare one exact UTF-16 region without decoding surrogate pairs.
fn string_region_matches(source: &JsString, needle: &JsString, start: i32) -> bool {
    let Ok(start) = usize::try_from(start) else {
        return false;
    };
    needle.utf16_units().enumerate().all(|(offset, unit)| {
        start
            .checked_add(offset)
            .and_then(|index| source.code_unit_at(index))
            == Some(unit)
    })
}

/// Exact traversal performed by QuickJS `js_string_indexOf` after conversion
/// and position selection. Both endpoints are inclusive.
fn scan_string_region(
    source: &JsString,
    needle: &JsString,
    start: i32,
    stop: i32,
    increment: i32,
) -> i32 {
    debug_assert!(increment == 1 || increment == -1);
    if source.len() < needle.len()
        || (increment == 1 && start > stop)
        || (increment == -1 && start < stop)
    {
        return -1;
    }

    let mut index = start;
    loop {
        if string_region_matches(source, needle, index) {
            return index;
        }
        if index == stop {
            return -1;
        }
        index += increment;
    }
}

impl Runtime {
    /// Rust port of pinned QuickJS `js_string_indexOf`.
    ///
    /// The two table entries deliberately retain their different position
    /// conversions: forward search uses `JS_ToInt32Clamp`, while reverse
    /// search treats NaN as the omitted-position default after `JS_ToFloat64`.
    pub(in crate::runtime) fn call_string_prototype_index_of(
        &self,
        realm: ContextId,
        selector: StringIndexOfKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String indexOf method did not receive a generic invocation",
            ));
        };

        // QuickJS converts the receiver before reading or converting either
        // argument. ToString also linearizes a rope for the code-unit loop.
        let source = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let search_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String indexOf search argv was not padded",
        ))?;
        let needle = match self.native_to_js_string(realm, search_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let source_len = i32::try_from(source.len()).map_err(|_| {
            RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
        })?;
        let needle_len = i32::try_from(needle.len()).map_err(|_| {
            RuntimeError::Invariant("String search length exceeded QuickJS's signed index range")
        })?;

        let result = match selector {
            StringIndexOfKind::IndexOf => {
                let position = if arguments.actual_arg_count > 1 {
                    let position = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                        "String indexOf position argv was not readable",
                    ))?;
                    match self.native_to_number(realm, position)? {
                        NativeConversion::Value(value) => {
                            crate::number::to_int32_sat(value).clamp(0, source_len)
                        }
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                } else {
                    0
                };
                scan_string_region(&source, &needle, position, source_len - needle_len, 1)
            }
            StringIndexOfKind::LastIndexOf => {
                let mut position = source_len - needle_len;
                if arguments.actual_arg_count > 1 {
                    let position_value =
                        arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                            "String lastIndexOf position argv was not readable",
                        ))?;
                    let number = match self.native_to_number(realm, position_value)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    };
                    if !number.is_nan() {
                        if number <= 0.0 {
                            position = 0;
                        } else if number < f64::from(position) {
                            // This branch proves 0 < number < position and
                            // String lengths are below 2^30, so the cast is
                            // exactly QuickJS's truncating C assignment.
                            position = number as i32;
                        }
                    }
                }
                scan_string_region(&source, &needle, position, 0, -1)
            }
        };

        Ok(Completion::Return(Value::Int(result)))
    }
}
