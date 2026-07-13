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
    /// Publish the complete own table of QuickJS's `%String%` constructor.
    ///
    /// The three static entries must precede the non-configurable `prototype`
    /// property: their order is fixed by `js_string_funcs` and cannot be
    /// repaired after the constructor becomes observable.
    pub(in crate::runtime) fn initialize_string_constructor_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        string_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::PrimitiveConstructor(PrimitiveKind::String),
            1,
            "String",
            1,
        )?;
        for (selector, name) in [
            (StringStaticKind::FromCharCode, "fromCharCode"),
            (StringStaticKind::FromCodePoint, "fromCodePoint"),
            (StringStaticKind::Raw, "raw"),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                NativeFunctionId::StringStatic(selector),
                name,
                1,
                1,
            )?;
        }
        self.define_function_data_property(
            global_object,
            "String",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, string_prototype)
    }

    /// Build the direct Symbol spelling used by both `%String%`'s call-only
    /// exception and `%Symbol.prototype%.toString`. Missing and explicitly
    /// empty descriptions both produce `Symbol()`.
    pub(in crate::runtime) fn symbol_descriptive_string(
        &self,
        symbol: &SymbolRef,
    ) -> Result<JsString, RuntimeError> {
        let description = self
            .symbol_description(symbol)?
            .unwrap_or_else(|| JsString::from_static(""));
        let mut builder = JsStringBuilder::new(8);
        builder.push_utf8("Symbol(")?;
        builder.push_js_string(&description)?;
        builder.push_utf8(")")?;
        Ok(builder.finish()?)
    }

    /// Dispatch the three generic C functions in pinned `js_string_funcs`.
    pub(in crate::runtime) fn call_string_static(
        &self,
        realm: ContextId,
        selector: StringStaticKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "String static did not receive a generic invocation",
            ));
        };
        match selector {
            StringStaticKind::FromCharCode => self.call_string_from_char_code(realm, arguments),
            StringStaticKind::FromCodePoint => self.call_string_from_code_point(realm, arguments),
            StringStaticKind::Raw => {
                self.call_string_raw_with_limit(realm, arguments, JsString::MAX_LEN)
            }
        }
    }

    /// Rust port of pinned QuickJS `js_string_fromCharCode`.
    fn call_string_from_char_code(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let mut builder = JsStringBuilder::new(arguments.actual_arg_count);
        for argument in &arguments.readable[..arguments.actual_arg_count] {
            let number = match self.native_to_number(realm, argument)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let code_unit = (crate::number::to_int32(number) as u32) & 0xffff;
            builder.push_code_point(code_unit)?;
        }
        Ok(Completion::Return(Value::String(builder.finish()?)))
    }

    /// Rust port of pinned QuickJS `js_string_fromCodePoint`, including its
    /// integer-tag fast path and acceptance of lone-surrogate code points.
    fn call_string_from_code_point(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let mut builder = JsStringBuilder::new(arguments.actual_arg_count);
        for argument in &arguments.readable[..arguments.actual_arg_count] {
            let code_point = match argument {
                Value::Int(value) if (0..=0x10_ffff).contains(value) => *value as u32,
                Value::Int(_) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Range,
                        "invalid code point",
                    )?));
                }
                _ => {
                    let number = match self.native_to_number(realm, argument)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if !number.is_finite()
                        || number < 0.0
                        || number > 0x10_ffff as f64
                        || number.fract() != 0.0
                    {
                        return Ok(Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "invalid code point",
                        )?));
                    }
                    number as u32
                }
            };
            builder.push_code_point(code_point)?;
        }
        Ok(Completion::Return(Value::String(builder.finish()?)))
    }

    /// Rust port of pinned QuickJS `js_string_raw` with an injectable output
    /// limit for white-box tests of StringBuffer's latched-error behavior.
    fn call_string_raw_with_limit(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let template = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String.raw template argv was not padded",
        ))?;
        let cooked = match self.native_to_object(realm, template.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let raw_key = self.intern_property_key("raw")?;
        let raw = match self.get_property_in_realm(realm, &cooked, &raw_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let raw = match self.native_to_object(realm, raw)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length_key = self.intern_property_key("length")?;
        let length = match self.get_property_in_realm(realm, &raw, &length_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let length = match self.native_to_length(realm, &length)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut builder = JsStringBuilder::with_limit(0, string_limit);
        for index in 0..length {
            let index_key = self.intern_property_key(&index.to_string())?;
            let chunk = match self.get_property_in_realm(realm, &raw, &index_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            // QuickJS performs Get+ToString even after a prior raw append has
            // latched a StringBuffer error. A later user throw may therefore
            // replace the pending `string too long` exception.
            let chunk = match self.native_to_js_string(realm, &chunk)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let raw_append = builder.push_js_string(&chunk);

            let substitution_index = index + 1;
            let substitution_index = usize::try_from(substitution_index).ok();
            let has_substitution = index + 1 < length
                && substitution_index.is_some_and(|index| index < arguments.actual_arg_count);
            if !has_substitution {
                // `string_buffer_concat_value_free` has a deliberately ignored
                // result in `js_string_raw`; preserve its latched error.
                let _ = raw_append;
                continue;
            }
            raw_append?;
            let substitution_index =
                substitution_index.expect("a present String.raw substitution index fits usize");
            let substitution =
                arguments
                    .readable
                    .get(substitution_index)
                    .ok_or(RuntimeError::Invariant(
                        "String.raw substitution argv was not readable",
                    ))?;
            let substitution = match self.native_to_js_string(realm, substitution)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            builder.push_js_string(&substitution)?;
        }
        Ok(Completion::Return(Value::String(builder.finish()?)))
    }

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
