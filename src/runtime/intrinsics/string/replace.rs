//! `String.prototype.replace` and `String.prototype.replaceAll`.

use crate::value::ReplacementStringBuffer;

use super::super::replacement::{SubstitutionInput, SubstitutionStatus};
use super::*;

impl Runtime {
    /// Rust port of pinned QuickJS `js_string_replace`.
    pub(in crate::runtime) fn call_string_prototype_replace(
        &self,
        realm: ContextId,
        selector: StringReplaceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String replace family did not receive a generic-magic invocation",
            ));
        };
        if matches!(this_value, Value::Undefined | Value::Null) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to object",
            )?));
        }
        let search_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String replace search argv was not padded",
        ))?;
        let replace_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "String replace replacement argv was not padded",
        ))?;

        if let Value::Object(search_object) = search_value {
            if matches!(selector, StringReplaceKind::ReplaceAll) {
                let is_regexp = match self.native_is_regexp(realm, search_value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if is_regexp {
                    let flags_key = self.intern_property_key("flags")?;
                    let flags =
                        match self.get_property_in_realm(realm, search_object, &flags_key)? {
                            Completion::Return(value) => value,
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        };
                    if matches!(flags, Value::Undefined | Value::Null) {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "cannot convert to object",
                        )?));
                    }
                    let flags = match self.native_to_js_string(realm, &flags)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !flags.utf16_units().any(|unit| unit == u16::from(b'g')) {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "regexp must have the 'g' flag",
                        )?));
                    }
                }
            }

            let replace_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Replace));
            let method = match self.get_property_in_realm(realm, search_object, &replace_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !matches!(method, Value::Undefined | Value::Null) {
                return self.call_string_regexp_method(
                    realm,
                    search_object.clone(),
                    method,
                    &[this_value, replace_value.clone()],
                );
            }
        }

        // QuickJS initializes the StringBuffer before the fallback
        // conversions. Its error remains latched while later observable
        // coercions run.
        let mut output = ReplacementStringBuffer::new(0);
        let source = match self.native_to_js_string(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let search = match self.native_to_js_string(realm, search_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let functional = match replace_value {
            Value::Object(object) => self.as_callable(object)?,
            _ => None,
        };
        let replacement = if functional.is_none() {
            match self.native_to_js_string(realm, replace_value)? {
                NativeConversion::Value(value) => Some(value),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        } else {
            None
        };

        let mut end_of_last_match = 0_usize;
        let mut first = true;
        loop {
            let position = if search.is_empty() {
                if first {
                    Some(0)
                } else if end_of_last_match >= source.len() {
                    None
                } else {
                    Some(end_of_last_match + 1)
                }
            } else {
                let stop = match source.len().checked_sub(search.len()) {
                    Some(value) => value,
                    None => {
                        if first {
                            return Ok(Completion::Return(Value::String(source)));
                        }
                        break;
                    }
                };
                if end_of_last_match > stop {
                    None
                } else {
                    let start = i32::try_from(end_of_last_match).map_err(|_| {
                        RuntimeError::Invariant("String replace start exceeded signed range")
                    })?;
                    let stop = i32::try_from(stop).map_err(|_| {
                        RuntimeError::Invariant("String replace stop exceeded signed range")
                    })?;
                    let found = scan_string_region(&source, &search, start, stop, 1);
                    usize::try_from(found).ok()
                }
            };

            let Some(position) = position else {
                if first {
                    return Ok(Completion::Return(Value::String(source)));
                }
                break;
            };
            output.append_range(&source, end_of_last_match, position);

            if let Some(callable) = &functional {
                let position_value = Value::Int(i32::try_from(position).map_err(|_| {
                    RuntimeError::Invariant("String replace position exceeded signed range")
                })?);
                let result = match self.call_internal(
                    realm,
                    callable,
                    Value::Undefined,
                    &[
                        Value::String(search.clone()),
                        position_value,
                        Value::String(source.clone()),
                    ],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let result = match self.native_to_js_string(realm, &result)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                output.append_js_string(&result);
            } else {
                let substitution = self.append_get_substitution(
                    realm,
                    &mut output,
                    SubstitutionInput {
                        matched: &search,
                        input: &source,
                        position,
                        captures: None,
                        named_captures: None,
                        replacement: replacement
                            .as_ref()
                            .expect("non-functional replacement was not converted"),
                    },
                )?;
                match substitution {
                    Ok(SubstitutionStatus::Complete) => {}
                    Ok(SubstitutionStatus::BufferFailed) => {
                        return match self.finish_replacement_buffer(realm, output)? {
                            NativeConversion::Value(_) => Err(RuntimeError::Invariant(
                                "failed replacement buffer unexpectedly completed",
                            )),
                            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
                        };
                    }
                    Err(value) => return Ok(Completion::Throw(value)),
                }
            }

            end_of_last_match = position + search.len();
            first = false;
            if matches!(selector, StringReplaceKind::Replace) {
                break;
            }
        }

        output.append_range(&source, end_of_last_match, source.len());
        match self.finish_replacement_buffer(realm, output)? {
            NativeConversion::Value(value) => Ok(Completion::Return(Value::String(value))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
