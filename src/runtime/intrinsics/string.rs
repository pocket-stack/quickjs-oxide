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

/// Saturating/clamping integer conversion used by pinned QuickJS
/// `JS_ToInt32Clamp` when `min` is zero. Negative relative indices receive
/// `min_offset` once before the lower clamp; positive overflow clamps to max.
fn string_to_int32_clamp(number: f64, max: i32, min_offset: i32) -> i32 {
    debug_assert!(max >= 0);
    debug_assert!(min_offset >= 0);
    let mut result = crate::number::to_int32_sat(number);
    if result < 0 {
        result += min_offset;
        if result < 0 {
            result = 0;
        }
    } else if result > max {
        result = max;
    }
    result
}

impl Runtime {
    /// Install the currently implemented entries from QuickJS's String
    /// prototype table in their exact relative order. Missing table entries
    /// remain unpublished; later parity slices must be inserted at their
    /// pinned position rather than appended after the conversion methods.
    pub(in crate::runtime) fn initialize_string_prototype_methods(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        for (target, name, min_readable_args) in [
            (
                NativeFunctionId::StringPrototypeCharAt(StringCharAtKind::At),
                "at",
                1,
            ),
            (NativeFunctionId::StringPrototypeCharCodeAt, "charCodeAt", 1),
            (
                NativeFunctionId::StringPrototypeCharAt(StringCharAtKind::CharAt),
                "charAt",
                1,
            ),
            (NativeFunctionId::StringPrototypeConcat, "concat", 0),
            (
                NativeFunctionId::StringPrototypeCodePointAt,
                "codePointAt",
                1,
            ),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                target,
                name,
                1,
                min_readable_args,
            )?;
        }
        for (target, name) in [
            (
                NativeFunctionId::StringPrototypeWellFormed(StringWellFormedKind::IsWellFormed),
                "isWellFormed",
            ),
            (
                NativeFunctionId::StringPrototypeWellFormed(StringWellFormedKind::ToWellFormed),
                "toWellFormed",
            ),
        ] {
            self.define_native_builtin_auto_init(string_prototype, realm, target, name, 0, 0)?;
        }
        for (selector, name) in [
            (StringIndexOfKind::IndexOf, "indexOf"),
            (StringIndexOfKind::LastIndexOf, "lastIndexOf"),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypeIndexOf(selector),
                name,
                1,
                1,
            )?;
        }
        for (selector, name) in [
            (StringIncludesKind::Includes, "includes"),
            (StringIncludesKind::EndsWith, "endsWith"),
            (StringIncludesKind::StartsWith, "startsWith"),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypeIncludes(selector),
                name,
                1,
                1,
            )?;
        }
        // QuickJS has match/matchAll/search/split between startsWith and this
        // group. They are not implemented yet, so preserve the implemented-
        // key filtered order while keeping this trio adjacent and ordered.
        for (selector, name) in [
            (StringSubrangeKind::Substring, "substring"),
            (StringSubrangeKind::Substr, "substr"),
            (StringSubrangeKind::Slice, "slice"),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypeSubrange(selector),
                name,
                2,
                2,
            )?;
        }
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeRepeat,
            "repeat",
            1,
            1,
        )?;
        // QuickJS publishes replace/replaceAll between repeat and this pair.
        // They remain absent until their own parity slice, so preserve the
        // filtered table order and the release's padEnd-before-padStart order.
        for (selector, name) in [
            (StringPadKind::End, "padEnd"),
            (StringPadKind::Start, "padStart"),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypePad(selector),
                name,
                1,
                1,
            )?;
        }
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeTrim(StringTrimKind::Both),
            "trim",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeTrim(StringTrimKind::End),
            "trimEnd",
            0,
            0,
        )?;
        self.define_string_prototype_alias(realm, string_prototype, "trimRight", "trimEnd")?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeTrim(StringTrimKind::Start),
            "trimStart",
            0,
            0,
        )?;
        self.define_string_prototype_alias(realm, string_prototype, "trimLeft", "trimStart")?;
        Ok(())
    }

    /// Materialize and copy one canonical String method for a pinned
    /// `JS_ALIAS_DEF`. QuickJS explicitly forbids AutoInit for aliases: the
    /// property read instantiates the canonical function immediately, then the
    /// alias stores that exact object with the canonical function name.
    fn define_string_prototype_alias(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
        alias: &'static str,
        canonical: &'static str,
    ) -> Result<(), RuntimeError> {
        let canonical_key = self.intern_property_key(canonical)?;
        let value = match self.get_property_in_realm(realm, string_prototype, &canonical_key)? {
            Completion::Return(value @ Value::Object(_)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "String canonical alias target was not callable",
                ));
            }
            Completion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "String canonical alias initialization threw during bootstrap",
                ));
            }
        };
        self.define_function_data_property(string_prototype, alias, value, true, true)
    }

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

    /// Internal-class fallback of pinned QuickJS `js_is_regexp` after an
    /// object has produced `undefined` for `Symbol.match`.
    ///
    /// No RegExp payload is constructible yet. Keep this match exhaustive so
    /// adding that payload makes the missing `true` branch a compile-time
    /// integration point instead of silently treating RegExp instances as
    /// ordinary objects.
    #[allow(clippy::match_single_binding)]
    fn native_object_has_regexp_brand(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(match &object.payload {
            ObjectPayload::Ordinary
            | ObjectPayload::Array
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => false,
        })
    }

    /// Rust port of pinned QuickJS `js_is_regexp`: primitives skip the
    /// `Symbol.match` lookup, objects perform one ordinary Get, a present value
    /// is converted only with ToBoolean, and `undefined` falls back to the
    /// internal RegExp brand.
    fn native_is_regexp(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Value(false));
        };
        let match_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Match));
        let matcher = match self.get_property_in_realm(realm, object, &match_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !matches!(matcher, Value::Undefined) {
            return Ok(NativeConversion::Value(matcher.to_boolean()));
        }
        Ok(NativeConversion::Value(
            self.native_object_has_regexp_brand(object)?,
        ))
    }

    /// Rust port of pinned QuickJS `js_string_includes`, shared by the
    /// `includes`, `endsWith`, and `startsWith` magic variants.
    pub(in crate::runtime) fn call_string_prototype_includes(
        &self,
        realm: ContextId,
        selector: StringIncludesKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String includes family did not receive a generic invocation",
            ));
        };

        // QuickJS converts the receiver before observing any search-value
        // property, then performs IsRegExp before converting the search value.
        let source = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let search_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String includes search argv was not padded",
        ))?;
        let is_regexp = match self.native_is_regexp(realm, search_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if is_regexp {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "regexp not supported",
            )?));
        }
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
        let mut position = if selector == StringIncludesKind::EndsWith {
            source_len
        } else {
            0
        };
        if arguments.actual_arg_count > 1 {
            let position_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "String includes position argv was not readable",
            ))?;
            // Unlike indexOf, the shared QuickJS function explicitly skips an
            // `undefined` position instead of sending it through ToNumber.
            if !matches!(position_value, Value::Undefined) {
                let number = match self.native_to_number(realm, position_value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                position = crate::number::to_int32_sat(number).clamp(0, source_len);
            }
        }

        let stop = source_len - needle_len;
        let found = match selector {
            StringIncludesKind::Includes => {
                scan_string_region(&source, &needle, position, stop, 1) >= 0
            }
            StringIncludesKind::StartsWith => {
                position <= stop && string_region_matches(&source, &needle, position)
            }
            StringIncludesKind::EndsWith => {
                let start = position - needle_len;
                start >= 0 && string_region_matches(&source, &needle, start)
            }
        };
        Ok(Completion::Return(Value::Bool(found)))
    }

    /// Rust port of pinned QuickJS `js_string_substring`, `js_string_substr`,
    /// and `js_string_slice`. The native identities remain generic functions;
    /// only their conversion and UTF-16 copying machinery is shared here.
    pub(in crate::runtime) fn call_string_prototype_subrange(
        &self,
        realm: ContextId,
        selector: StringSubrangeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String subrange method did not receive a generic invocation",
            ));
        };

        // Every pinned function converts the receiver before observing start,
        // then converts end only when it is not undefined.
        let source = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source_len = i32::try_from(source.len()).map_err(|_| {
            RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
        })?;
        let start_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String subrange start argv was not padded",
        ))?;
        let start_number = match self.native_to_number(realm, start_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let start_offset = match selector {
            StringSubrangeKind::Substring => 0,
            StringSubrangeKind::Substr | StringSubrangeKind::Slice => source_len,
        };
        let start = string_to_int32_clamp(start_number, source_len, start_offset);

        let end_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "String subrange end argv was not padded",
        ))?;
        let (range_start, range_end) = match selector {
            StringSubrangeKind::Substring => {
                let mut end = source_len;
                if !matches!(end_value, Value::Undefined) {
                    let number = match self.native_to_number(realm, end_value)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    end = string_to_int32_clamp(number, source_len, 0);
                }
                if start < end {
                    (start, end)
                } else {
                    (end, start)
                }
            }
            StringSubrangeKind::Substr => {
                let remaining = source_len - start;
                let mut count = remaining;
                if !matches!(end_value, Value::Undefined) {
                    let number = match self.native_to_number(realm, end_value)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    count = string_to_int32_clamp(number, remaining, 0);
                }
                (start, start + count)
            }
            StringSubrangeKind::Slice => {
                let mut end = source_len;
                if !matches!(end_value, Value::Undefined) {
                    let number = match self.native_to_number(realm, end_value)? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    end = string_to_int32_clamp(number, source_len, source_len);
                }
                (start, end.max(start))
            }
        };
        let range_start = usize::try_from(range_start)
            .map_err(|_| RuntimeError::Invariant("String subrange start became negative"))?;
        let range_end = usize::try_from(range_end)
            .map_err(|_| RuntimeError::Invariant("String subrange end became negative"))?;
        Ok(Completion::Return(Value::String(
            source.sub_string(range_start, range_end),
        )))
    }

    /// Rust port of pinned QuickJS `js_string_repeat`, including its distinct
    /// repeat-count and result-length RangeErrors. The native entry point uses
    /// the release's 30-bit String cap; tests inject a smaller cap through the
    /// helper to cover the boundary without allocating enormous buffers.
    pub(in crate::runtime) fn call_string_prototype_repeat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_prototype_repeat_with_limit(
            realm,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    fn call_string_prototype_repeat_with_limit(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String repeat did not receive a generic invocation",
            ));
        };

        let source = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let count_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String repeat count argv was not padded",
        ))?;
        let count = match self.native_to_int64_sat(realm, count_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !(0..=2_147_483_647).contains(&count) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid repeat count",
            )?));
        }
        let count = usize::try_from(count)
            .map_err(|_| RuntimeError::Invariant("validated repeat count did not fit usize"))?;
        let repeated = match source.repeat_with_limit(count, string_limit) {
            Ok(value) => value,
            Err(JsStringError::TooLong) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid string length",
                )?));
            }
            Err(JsStringError::OutOfMemory) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
        };
        Ok(Completion::Return(Value::String(repeated)))
    }

    /// Rust port of pinned QuickJS `js_string_pad`. The typed selector mirrors
    /// its generic-magic `padEnd=1` / `padStart=0` argument without leaking a
    /// raw integer through dispatch.
    pub(in crate::runtime) fn call_string_prototype_pad(
        &self,
        realm: ContextId,
        selector: StringPadKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_prototype_pad_with_limit(
            realm,
            selector,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    fn call_string_prototype_pad_with_limit(
        &self,
        realm: ContextId,
        selector: StringPadKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String pad did not receive a generic-magic invocation",
            ));
        };

        // JS_ToStringCheckObject produces the flat JSString consumed by
        // QuickJS's JS_VALUE_GET_STRING before any target-length coercion.
        let source = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value.linearize(),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target_value = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String pad target length argv was not padded",
        ))?;
        let target = match self.native_to_number(realm, target_value)? {
            NativeConversion::Value(value) => crate::number::to_int32_sat(value),
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source_len = i32::try_from(source.len())
            .map_err(|_| RuntimeError::Invariant("String length exceeded signed Int32"))?;
        if source_len >= target {
            return Ok(Completion::Return(Value::String(source)));
        }

        // `argc > 1` is observable: an absent second argument and an explicit
        // undefined both select U+0020, but only actual arguments may be read.
        let filler = if arguments.actual_arg_count > 1 {
            let filler_value = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
                "String pad filler argv was not padded",
            ))?;
            if matches!(filler_value, Value::Undefined) {
                None
            } else {
                match self.native_to_js_string(realm, filler_value)? {
                    NativeConversion::Value(value) => Some(value.linearize()),
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            }
        } else {
            None
        };
        if filler.as_ref().is_some_and(JsString::is_empty) {
            return Ok(Completion::Return(Value::String(source)));
        }

        let target = usize::try_from(target)
            .map_err(|_| RuntimeError::Invariant("validated String pad target was negative"))?;
        let padded = match source.pad_with_limit(
            target,
            filler.as_ref(),
            matches!(selector, StringPadKind::End),
            string_limit,
        ) {
            Ok(value) => value,
            Err(JsStringError::TooLong) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Range,
                    "invalid string length",
                )?));
            }
            Err(JsStringError::OutOfMemory) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
        };
        Ok(Completion::Return(Value::String(padded)))
    }

    /// Rust port of pinned QuickJS `js_string_trim`. The selector retains its
    /// `magic & 1` leading / `magic & 2` trailing contract, receiver conversion
    /// precedes every code-unit read, and all arguments are ignored.
    pub(in crate::runtime) fn call_string_prototype_trim(
        &self,
        realm: ContextId,
        selector: StringTrimKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String trim did not receive a generic-magic invocation",
            ));
        };

        let source = match self.native_to_string_check_object(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let (trim_start, trim_end) = match selector {
            StringTrimKind::Both => (true, true),
            StringTrimKind::End => (false, true),
            StringTrimKind::Start => (true, false),
        };
        let trimmed = match source.trim_whitespace(trim_start, trim_end) {
            Ok(value) => value,
            Err(JsStringError::OutOfMemory) => {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "out of memory",
                )?));
            }
            Err(JsStringError::TooLong) => {
                return Err(RuntimeError::Invariant(
                    "String trim unexpectedly increased the source length",
                ));
            }
        };
        Ok(Completion::Return(Value::String(trimmed)))
    }
}
