//! `RegExp.prototype[Symbol.match]`.

use super::super::super::*;

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_match`.
    ///
    /// The non-global path returns abstract RegExpExec's object-or-null result
    /// unchanged. The global path resets `lastIndex`, collects only each
    /// result's stringified zero property into a defining-realm Array, and
    /// advances an unchanged empty match by a UTF-16 code point when the flags
    /// string contains `u` or `v`.
    pub(super) fn call_regexp_symbol_match(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value: regexp } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@match did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = regexp else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp @@match input argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
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
        if !global {
            return self.regexp_exec_abstract(realm, Value::Object(regexp), Value::String(input));
        }
        let full_unicode = flags
            .utf16_units()
            .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));

        let last_index = self.intern_property_key("lastIndex")?;
        if let Some(value) =
            self.set_property_or_throw(realm, &regexp, &last_index, Value::Int(0))?
        {
            return Ok(Completion::Throw(value));
        }
        let matches = self.new_array(realm)?;
        let zero = self.intern_property_key("0")?;
        let mut count = 0_u32;

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
                Value::Null => {
                    return Ok(Completion::Return(if count == 0 {
                        Value::Null
                    } else {
                        Value::Object(matches)
                    }));
                }
                Value::Object(result) => result,
                Value::Undefined
                | Value::Bool(_)
                | Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_) => {
                    return Err(RuntimeError::Invariant(
                        "RegExpExec returned neither an object nor null",
                    ));
                }
            };
            let matched = match self.get_property_in_realm(realm, &result, &zero)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let matched = match self.native_to_js_string(realm, &matched)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let empty = matched.is_empty();
            let Some(next_count) = count.checked_add(1) else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid array length",
                )?));
            };
            if let Some(value) =
                self.create_array_data_property(realm, &matches, count, Value::String(matched))?
            {
                return Ok(Completion::Throw(value));
            }
            count = next_count;

            if empty {
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
    }
}

pub(super) fn advance_string_index(input: &JsString, index: u64, unicode: bool) -> u64 {
    let width = if unicode
        && usize::try_from(index)
            .ok()
            .filter(|index| *index < input.len())
            .and_then(|index| input.code_point_at(index))
            .is_some_and(|code_point| code_point > u32::from(u16::MAX))
    {
        2
    } else {
        1
    };
    index + width
}
