//! `RegExp.prototype[Symbol.replace]`.

use crate::value::ReplacementStringBuffer;

use super::super::super::*;
use super::super::replacement::SubstitutionInput;
use super::match_protocol::advance_string_index;

const MAX_REPLACER_ARGUMENTS: usize = 65_534;

struct CollectedReplace {
    input: JsString,
    functional: Option<CallableRef>,
    replacement: Option<JsString>,
    output: ReplacementStringBuffer,
    results: Vec<ObjectRef>,
    zero: PropertyKey,
}

impl Runtime {
    /// Rust port of the generic path in pinned QuickJS
    /// `js_regexp_Symbol_replace`.
    ///
    /// The standard-RegExp direct matcher is deliberately a separate parity
    /// slice because QuickJS's predicate changes observable getter traffic.
    /// This path preserves the complete abstract protocol: collect every exec
    /// result first, then observe captures and invoke replacement callbacks.
    pub(super) fn call_regexp_symbol_replace(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value: regexp } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@replace did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = regexp else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let input_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp @@replace input argv was not padded",
        ))?;
        let replace_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "RegExp @@replace replacement argv was not padded",
        ))?;

        // Both buffers are initialized before the first observable input
        // conversion in QuickJS. The output error remains latched while all
        // required result getters and callbacks continue.
        let output = ReplacementStringBuffer::new(0);
        let input = match self.native_to_js_string(realm, input_value)? {
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

        let flags_key = self.intern_property_key("flags")?;
        let flags = match self.get_property_in_realm(realm, &regexp, &flags_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let flags = match self.native_to_js_string(realm, &flags)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let global = flags.utf16_units().any(|unit| unit == u16::from(b'g'));
        let full_unicode = global
            && flags
                .utf16_units()
                .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
        let last_index = self.intern_property_key("lastIndex")?;
        if global
            && let Some(value) =
                self.set_property_or_throw(realm, &regexp, &last_index, Value::Int(0))?
        {
            return Ok(Completion::Throw(value));
        }

        let zero = self.intern_property_key("0")?;
        let mut results = Vec::<ObjectRef>::new();
        loop {
            let result = match self.regexp_exec_abstract(
                realm,
                Value::Object(regexp.clone()),
                Value::String(input.clone()),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let result = match result {
                Value::Null => break,
                Value::Object(value) => value,
                _ => {
                    return Err(RuntimeError::Invariant(
                        "RegExpExec returned neither an object nor null",
                    ));
                }
            };
            if results.try_reserve(1).is_err() {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            results.push(result.clone());
            if !global {
                break;
            }

            let matched = match self.get_property_in_realm(realm, &result, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let matched = match self.native_to_js_string(realm, &matched)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if matched.is_empty() {
                let current = match self.get_property_in_realm(realm, &regexp, &last_index)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let current = match self.native_to_length(realm, &current)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let next = advance_string_index(&input, current, full_unicode);
                if let Some(value) = self.set_property_or_throw(
                    realm,
                    &regexp,
                    &last_index,
                    Value::number(next as f64),
                )? {
                    return Ok(Completion::Throw(value));
                }
            }
        }

        self.finish_regexp_symbol_replace(
            realm,
            CollectedReplace {
                input,
                functional,
                replacement,
                output,
                results,
                zero,
            },
        )
    }

    #[inline(never)]
    fn finish_regexp_symbol_replace(
        &self,
        realm: ContextId,
        collected: CollectedReplace,
    ) -> Result<Completion, RuntimeError> {
        let CollectedReplace {
            input,
            functional,
            replacement,
            mut output,
            results,
            zero,
        } = collected;
        let length_key = self.intern_property_key("length")?;
        let index_key = self.intern_property_key("index")?;
        let groups_key = self.intern_property_key("groups")?;
        let mut next_source_position = 0_usize;

        for result in results {
            let capture_count = match self.get_property_in_realm(realm, &result, &length_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let capture_count = match self.native_to_number(realm, &capture_count)? {
                NativeConversion::Value(value) => Self::to_uint32_number(value),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            let matched = match self.get_property_in_realm(realm, &result, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let matched = match self.native_to_js_string(realm, &matched)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            let position = match self.get_property_in_realm(realm, &result, &index_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let position = match self.native_to_length(realm, &position)? {
                NativeConversion::Value(value) => value.min(input.len() as u64),
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let position = usize::try_from(position).map_err(|_| {
                RuntimeError::Invariant("RegExp replace position did not fit usize")
            })?;

            let mut captures = Vec::<Value>::new();
            if captures.try_reserve(1).is_err() {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            captures.push(Value::String(matched.clone()));
            for capture_index in 1..capture_count {
                let key = self.intern_property_key(&capture_index.to_string())?;
                let capture = match self.get_property_in_realm(realm, &result, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                let capture = if matches!(capture, Value::Undefined) {
                    Value::Undefined
                } else {
                    match self.native_to_js_string(realm, &capture)? {
                        NativeConversion::Value(value) => Value::String(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                if captures.try_reserve(1).is_err() {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?));
                }
                captures.push(capture);
            }

            let groups = match self.get_property_in_realm(realm, &result, &groups_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let replacement_text = if let Some(callable) = &functional {
                let extra_arguments = if matches!(groups, Value::Undefined) {
                    2
                } else {
                    3
                };
                let position_value = Value::Int(i32::try_from(position).map_err(|_| {
                    RuntimeError::Invariant("RegExp replace position exceeded signed range")
                })?);
                if captures.try_reserve(extra_arguments).is_err() {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Internal,
                        "out of memory",
                    )?));
                }
                captures.push(position_value);
                captures.push(Value::String(input.clone()));
                if !matches!(groups, Value::Undefined) {
                    captures.push(groups);
                }
                if captures.len() > MAX_REPLACER_ARGUMENTS {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "too many arguments in function call (only 65534 allowed)",
                    )?));
                }
                let result =
                    match self.call_internal(realm, callable, Value::Undefined, &captures)? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                match self.native_to_js_string(realm, &result)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            } else {
                let named_captures = if matches!(groups, Value::Undefined) {
                    None
                } else {
                    match self.native_to_object(realm, groups)? {
                        NativeConversion::Value(value) => Some(value),
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                };
                let mut substitution = ReplacementStringBuffer::new(0);
                let status = self.append_get_substitution(
                    realm,
                    &mut substitution,
                    SubstitutionInput {
                        matched: &matched,
                        input: &input,
                        position,
                        captures: Some(&captures),
                        named_captures: named_captures.as_ref(),
                        replacement: replacement
                            .as_ref()
                            .expect("non-functional replacement was not converted"),
                    },
                )?;
                if let Err(value) = status {
                    return Ok(Completion::Throw(value));
                }
                match self.finish_replacement_buffer(realm, substitution)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            };

            // Custom exec results may move backward. QuickJS still performs
            // every getter, coercion, named lookup and callback above before
            // deciding whether this replacement contributes to the output.
            if position >= next_source_position {
                output.append_range(&input, next_source_position, position);
                output.append_js_string(&replacement_text);
                next_source_position = position.saturating_add(matched.len());
            }
        }

        if next_source_position < input.len() {
            output.append_range(&input, next_source_position, input.len());
        }
        match self.finish_replacement_buffer(realm, output)? {
            NativeConversion::Value(value) => Ok(Completion::Return(Value::String(value))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
