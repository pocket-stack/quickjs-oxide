//! `RegExp.prototype[Symbol.split]`.

use super::super::super::*;
use super::match_protocol::advance_string_index;

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_split`.
    pub(super) fn call_regexp_symbol_split(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value: regexp } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@split did not receive a generic invocation",
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
            "RegExp @@split input argv was not padded",
        ))?;
        let limit_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "RegExp @@split limit argv was not padded",
        ))?;

        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let constructor = match self.regexp_species_constructor(realm, &regexp)? {
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
        let unicode_matching = flags
            .utf16_units()
            .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
        let flags = if flags.utf16_units().any(|unit| unit == u16::from(b'y')) {
            flags
        } else {
            flags.try_concat(&JsString::from_static("y"))?
        };

        let splitter = match self.construct_internal(
            realm,
            &constructor,
            &constructor,
            &[Value::Object(regexp), Value::String(flags)],
        )? {
            Completion::Return(Value::Object(value)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "RegExp species constructor returned a primitive",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let result = self.new_array(realm)?;
        let limit = if matches!(limit_value, Value::Undefined) {
            u32::MAX
        } else {
            let number = match self.native_to_number(realm, limit_value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            Self::to_uint32_number(number)
        };
        if limit == 0 {
            return Ok(Completion::Return(Value::Object(result)));
        }

        let size = input.len();
        let mut length = 0_u32;
        if size == 0 {
            match self.regexp_exec_abstract(
                realm,
                Value::Object(splitter),
                Value::String(input.clone()),
            )? {
                Completion::Return(Value::Null) => {
                    if let Some(value) = self.append_regexp_split_value(
                        realm,
                        &result,
                        &mut length,
                        Value::String(input),
                    )? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Completion::Return(Value::Object(_)) => {}
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "RegExpExec returned neither an object nor null",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            }
            return Ok(Completion::Return(Value::Object(result)));
        }

        let last_index = self.intern_property_key("lastIndex")?;
        let length_key = self.intern_property_key("length")?;
        let mut p = 0_usize;
        let mut q = 0_usize;
        while q < size {
            let q_value = Value::Int(i32::try_from(q).map_err(|_| {
                RuntimeError::Invariant("RegExp split index exceeded signed String range")
            })?);
            if let Some(value) =
                self.set_property_or_throw(realm, &splitter, &last_index, q_value)?
            {
                return Ok(Completion::Throw(value));
            }
            let matched = match self.regexp_exec_abstract(
                realm,
                Value::Object(splitter.clone()),
                Value::String(input.clone()),
            )? {
                Completion::Return(Value::Null) => None,
                Completion::Return(Value::Object(value)) => Some(value),
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "RegExpExec returned neither an object nor null",
                    ));
                }
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let Some(matched) = matched else {
                q = usize::try_from(advance_string_index(&input, q as u64, unicode_matching))
                    .map_err(|_| {
                        RuntimeError::Invariant("advanced split index did not fit usize")
                    })?;
                continue;
            };

            let end = match self.get_property_in_realm(realm, &splitter, &last_index)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let end = match self.native_to_length(realm, &end)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let end = usize::try_from(end.min(size as u64))
                .map_err(|_| RuntimeError::Invariant("split end index did not fit usize"))?;
            if end == p {
                q = usize::try_from(advance_string_index(&input, q as u64, unicode_matching))
                    .map_err(|_| {
                        RuntimeError::Invariant("advanced split index did not fit usize")
                    })?;
                continue;
            }

            if let Some(value) = self.append_regexp_split_value(
                realm,
                &result,
                &mut length,
                Value::String(input.sub_string(p, q)),
            )? {
                return Ok(Completion::Throw(value));
            }
            if length == limit {
                return Ok(Completion::Return(Value::Object(result)));
            }
            p = end;

            let capture_count = match self.get_property_in_realm(realm, &matched, &length_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let capture_count = match self.native_to_length(realm, &capture_count)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            for index in 1..capture_count {
                let key = self.intern_property_key(&index.to_string())?;
                let capture = match self.get_property_in_realm(realm, &matched, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if let Some(value) =
                    self.append_regexp_split_value(realm, &result, &mut length, capture)?
                {
                    return Ok(Completion::Throw(value));
                }
                if length == limit {
                    return Ok(Completion::Return(Value::Object(result)));
                }
            }
            q = p;
        }

        if let Some(value) = self.append_regexp_split_value(
            realm,
            &result,
            &mut length,
            Value::String(input.sub_string(p.min(size), size)),
        )? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    fn append_regexp_split_value(
        &self,
        realm: ContextId,
        result: &ObjectRef,
        length: &mut u32,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let index = *length;
        let next = index.checked_add(1).ok_or(RuntimeError::Invariant(
            "RegExp split output index exceeded Uint32",
        ))?;
        if let Some(value) = self.create_array_data_property(realm, result, index, value)? {
            return Ok(Some(value));
        }
        *length = next;
        Ok(None)
    }
}
