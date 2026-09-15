//! String prototype intrinsics beyond the shared primitive-wrapper substrate.
mod factory;
#[cfg(feature = "stack-vm")]
pub(crate) use factory::{StringFactoryKind, StringFactoryResume, StringFactoryStep};

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{
    NativeFunctionId, PrimitiveKind, StringCaseKind, StringCharAtKind, StringCreateHtmlKind,
    StringIncludesKind, StringIndexOfKind, StringPadKind, StringReplaceKind, StringStaticKind,
    StringSubrangeKind, StringTrimKind, StringWellFormedKind,
};
use crate::engine::heap::{ContextId, ObjectPayload};
use crate::engine::object::{ObjectRef, SymbolRef};
#[cfg(test)]
use crate::engine::object::{PropertyKey, WellKnownSymbol};
use crate::engine::value::{
    CreateHtmlStringBuffer, JsString, JsStringBuilder, JsStringError, Value,
};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

mod regexp;
#[cfg(feature = "stack-vm")]
pub(crate) use regexp::{StringProtocolKind, StringProtocolResume, StringProtocolStep};
mod split;
#[cfg(feature = "stack-vm")]
pub(crate) use split::{StringSplitResume, StringSplitStep};
mod search;
#[cfg(feature = "stack-vm")]
pub(crate) use search::{StringSearchKind, StringSearchResume, StringSearchStep};
mod text;
#[cfg(feature = "stack-vm")]
pub(crate) use text::{StringTextKind, StringTextResume, StringTextStep};
mod replace;

#[cfg(feature = "stack-vm")]
pub(crate) use replace::{StringReplaceResume, StringReplaceStep};

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
    let mut result = crate::engine::value::number::to_int32_sat(number);
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

fn create_html_definition(selector: StringCreateHtmlKind) -> (&'static str, Option<&'static str>) {
    match selector {
        StringCreateHtmlKind::Anchor => ("a", Some("name")),
        StringCreateHtmlKind::Big => ("big", None),
        StringCreateHtmlKind::Blink => ("blink", None),
        StringCreateHtmlKind::Bold => ("b", None),
        StringCreateHtmlKind::Fixed => ("tt", None),
        StringCreateHtmlKind::FontColor => ("font", Some("color")),
        StringCreateHtmlKind::FontSize => ("font", Some("size")),
        StringCreateHtmlKind::Italics => ("i", None),
        StringCreateHtmlKind::Link => ("a", Some("href")),
        StringCreateHtmlKind::Small => ("small", None),
        StringCreateHtmlKind::Strike => ("strike", None),
        StringCreateHtmlKind::Sub => ("sub", None),
        StringCreateHtmlKind::Sup => ("sup", None),
    }
}

impl Runtime {
    /// Install the currently implemented entries from QuickJS's String
    /// prototype table in their exact relative order. Missing table entries
    /// remain unpublished; later parity slices must be inserted at their
    /// pinned position rather than appended after the conversion methods.
    pub(crate) fn initialize_string_prototype_methods(
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
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeMatch,
            "match",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeMatchAll,
            "matchAll",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeSearch,
            "search",
            1,
            1,
        )?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeSplit,
            "split",
            2,
            2,
        )?;
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
        for (selector, name) in [
            (StringReplaceKind::Replace, "replace"),
            (StringReplaceKind::ReplaceAll, "replaceAll"),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypeReplace(selector),
                name,
                2,
                2,
            )?;
        }
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

    /// Install the four case-conversion entries between the String brand
    /// methods and @@iterator, matching QuickJS's physical prototype table.
    /// Locale variants deliberately share the same native operation while
    /// retaining separate AutoInit properties and function identities.
    pub(crate) fn initialize_string_case_methods(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        for (selector, name) in [
            (StringCaseKind::Lower, "toLowerCase"),
            (StringCaseKind::Upper, "toUpperCase"),
            (StringCaseKind::Lower, "toLocaleLowerCase"),
            (StringCaseKind::Upper, "toLocaleUpperCase"),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypeCase(selector),
                name,
                0,
                0,
            )?;
        }
        Ok(())
    }

    /// Install the legacy Annex-B CreateHTML table after @@iterator and before
    /// the `%String%` constructor becomes observable, matching the pinned
    /// QuickJS prototype-table order.
    pub(crate) fn initialize_string_annex_b_html_methods(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        for (selector, name, length) in [
            (StringCreateHtmlKind::Anchor, "anchor", 1),
            (StringCreateHtmlKind::Big, "big", 0),
            (StringCreateHtmlKind::Blink, "blink", 0),
            (StringCreateHtmlKind::Bold, "bold", 0),
            (StringCreateHtmlKind::Fixed, "fixed", 0),
            (StringCreateHtmlKind::FontColor, "fontcolor", 1),
            (StringCreateHtmlKind::FontSize, "fontsize", 1),
            (StringCreateHtmlKind::Italics, "italics", 0),
            (StringCreateHtmlKind::Link, "link", 1),
            (StringCreateHtmlKind::Small, "small", 0),
            (StringCreateHtmlKind::Strike, "strike", 0),
            (StringCreateHtmlKind::Sub, "sub", 0),
            (StringCreateHtmlKind::Sup, "sup", 0),
        ] {
            self.define_native_builtin_auto_init(
                string_prototype,
                realm,
                NativeFunctionId::StringPrototypeCreateHtml(selector),
                name,
                length,
                length,
            )?;
        }
        Ok(())
    }

    /// Install the two Unicode-aware intrinsics which QuickJS appends with
    /// `JS_AddIntrinsicStringNormalize` after its complete base String table.
    /// Their physical order is observable after the `%String%` constructor.
    pub(crate) fn initialize_string_normalize_intrinsic(
        &self,
        realm: ContextId,
        string_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeNormalize,
            "normalize",
            0,
            0,
        )?;
        self.define_native_builtin_auto_init(
            string_prototype,
            realm,
            NativeFunctionId::StringPrototypeLocaleCompare,
            "localeCompare",
            1,
            1,
        )
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
    pub(crate) fn initialize_string_constructor_intrinsic(
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
    pub(crate) fn symbol_descriptive_string(
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
    pub(crate) fn call_string_static(
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

    /// Rust port of QuickJS's test262-only `js_string_codePointRange`.
    ///
    /// Both bounds use `ToUint32`; the converted end is then capped at one
    /// past the Unicode limit. The result contains every code point in the
    /// ascending half-open range encoded as UTF-16, including lone surrogate
    /// values when the requested range crosses the surrogate interval.
    #[cfg(feature = "test262-host")]
    pub(crate) fn call_string_code_point_range(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "String codePointRange did not receive a generic invocation",
            ));
        };
        factory::finish(
            self,
            realm,
            factory::StringFactoryStep::start(
                self,
                realm,
                factory::StringFactoryKind::CodePointRange,
                arguments,
            )?,
        )
    }

    /// Rust port of pinned QuickJS `js_string_fromCharCode`.
    fn call_string_from_char_code(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        factory::finish(
            self,
            realm,
            factory::StringFactoryStep::start(
                self,
                realm,
                factory::StringFactoryKind::Static(StringStaticKind::FromCharCode),
                arguments,
            )?,
        )
    }

    /// Rust port of pinned QuickJS `js_string_fromCodePoint`, including its
    /// integer-tag fast path and acceptance of lone-surrogate code points.
    fn call_string_from_code_point(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        factory::finish(
            self,
            realm,
            factory::StringFactoryStep::start(
                self,
                realm,
                factory::StringFactoryKind::Static(StringStaticKind::FromCodePoint),
                arguments,
            )?,
        )
    }

    /// Rust port of pinned QuickJS `js_string_raw` with an injectable output
    /// limit for white-box tests of StringBuffer's latched-error behavior.
    fn call_string_raw_with_limit(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        factory::finish(
            self,
            realm,
            factory::StringFactoryStep::with_limit(
                self,
                realm,
                factory::StringFactoryKind::Static(StringStaticKind::Raw),
                arguments,
                string_limit,
            )?,
        )
    }

    /// Rust port of pinned QuickJS `js_string_indexOf`.
    ///
    /// The two table entries deliberately retain their different position
    /// conversions: forward search uses `JS_ToInt32Clamp`, while reverse
    /// search treats NaN as the omitted-position default after `JS_ToFloat64`.
    pub(crate) fn call_string_prototype_index_of(
        &self,
        realm: ContextId,
        selector: StringIndexOfKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        search::finish(
            self,
            realm,
            search::StringSearchStep::start(
                self,
                realm,
                search::StringSearchKind::Index(selector),
                &invocation,
                arguments,
            )?,
        )
    }
    fn finish_string_index_of(
        &self,
        selector: StringIndexOfKind,
        source: JsString,
        needle: JsString,
        position_number: Option<f64>,
    ) -> Result<Completion, RuntimeError> {
        let source_len = i32::try_from(source.len()).map_err(|_| {
            RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
        })?;
        let needle_len = i32::try_from(needle.len()).map_err(|_| {
            RuntimeError::Invariant("String search length exceeded QuickJS's signed index range")
        })?;

        let result = match selector {
            StringIndexOfKind::IndexOf => {
                let position = position_number.map_or(0, |number| {
                    crate::engine::value::number::to_int32_sat(number).clamp(0, source_len)
                });
                scan_string_region(&source, &needle, position, source_len - needle_len, 1)
            }
            StringIndexOfKind::LastIndexOf => {
                let mut position = source_len - needle_len;
                if let Some(number) = position_number {
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
    pub(crate) fn is_regexp_from_match(
        &self,
        object: &ObjectRef,
        matcher: &Value,
    ) -> Result<bool, RuntimeError> {
        if matches!(matcher, Value::Undefined) {
            self.native_object_has_regexp_brand(object)
        } else {
            self.value_to_boolean(matcher)
        }
    }

    pub(crate) fn native_object_has_regexp_brand(
        &self,
        object: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(match &object.payload {
            ObjectPayload::RegExp(_) => true,
            ObjectPayload::Ordinary
            | ObjectPayload::ArrayBuffer(_)
            | ObjectPayload::SharedArrayBuffer(_)
            | ObjectPayload::DataView(_)
            | ObjectPayload::TypedArray(_)
            | ObjectPayload::Proxy(_)
            | ObjectPayload::RawJson
            | ObjectPayload::Promise(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::IteratorHelper(_)
            | ObjectPayload::IteratorWrap(_)
            | ObjectPayload::AsyncFromSyncIterator(_)
            | ObjectPayload::IteratorConcat(_)
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::WeakMap { .. }
            | ObjectPayload::WeakSet { .. }
            | ObjectPayload::WeakRef { .. }
            | ObjectPayload::FinalizationRegistry(_)
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::AsyncFunctionState(_)
            | ObjectPayload::Generator { .. }
            | ObjectPayload::AsyncGenerator(_) => false,
        })
    }

    /// Rust port of pinned QuickJS `js_string_includes`, shared by the
    /// `includes`, `endsWith`, and `startsWith` magic variants.
    pub(crate) fn call_string_prototype_includes(
        &self,
        realm: ContextId,
        selector: StringIncludesKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        search::finish(
            self,
            realm,
            search::StringSearchStep::start(
                self,
                realm,
                search::StringSearchKind::Includes(selector),
                &invocation,
                arguments,
            )?,
        )
    }
    fn finish_string_includes(
        &self,
        selector: StringIncludesKind,
        source: JsString,
        needle: JsString,
        position_number: Option<f64>,
    ) -> Result<Completion, RuntimeError> {
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
        if let Some(number) = position_number {
            position = crate::engine::value::number::to_int32_sat(number).clamp(0, source_len);
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

    /// Rust port of pinned QuickJS `js_string_split` for the generic
    /// non-RegExp path. An object separator may still supply its own
    /// `@@split`; this method delegates to that callable without coercing the
    /// original receiver or limit. Falling through preserves QuickJS's exact
    /// conversion order and splits flat UTF-16 code-unit ranges directly.
    pub(crate) fn call_string_prototype_split(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        split::finish(
            self,
            realm,
            split::StringSplitStep::start(self, realm, &invocation, arguments)?,
        )
    }
    fn finish_string_split(
        &self,
        realm: ContextId,
        source: JsString,
        result: ObjectRef,
        separator: &Value,
        separator_string: JsString,
        limit: u32,
    ) -> Result<Completion, RuntimeError> {
        let mut length = 0_u32;
        if limit == 0 {
            return Ok(Completion::Return(Value::Object(result)));
        }
        if matches!(separator, Value::Undefined) {
            if let Some(value) = self.define_string_split_element(
                realm,
                &result,
                &mut length,
                Value::String(source.clone()),
            )? {
                return Ok(Completion::Throw(value));
            }
            return Ok(Completion::Return(Value::Object(result)));
        }

        let source_len = source.len();
        let separator_len = separator_string.len();
        if source_len == 0 {
            if separator_len != 0 {
                if let Some(value) = self.define_string_split_element(
                    realm,
                    &result,
                    &mut length,
                    Value::String(source),
                )? {
                    return Ok(Completion::Throw(value));
                }
            }
            return Ok(Completion::Return(Value::Object(result)));
        }

        if separator_len == 0 {
            for index in 0..source_len {
                if let Some(value) = self.define_string_split_element(
                    realm,
                    &result,
                    &mut length,
                    Value::String(source.sub_string(index, index + 1)),
                )? {
                    return Ok(Completion::Throw(value));
                }
                if length == limit {
                    break;
                }
            }
            return Ok(Completion::Return(Value::Object(result)));
        }

        let source_len_i32 = i32::try_from(source_len).map_err(|_| {
            RuntimeError::Invariant("String split source exceeded QuickJS's signed length range")
        })?;
        let separator_len_i32 = i32::try_from(separator_len).map_err(|_| {
            RuntimeError::Invariant("String split separator exceeded QuickJS's signed length range")
        })?;
        let stop = source_len_i32 - separator_len_i32;
        let mut start = 0_i32;
        while start <= stop {
            let end = scan_string_region(&source, &separator_string, start, stop, 1);
            if end < 0 {
                break;
            }
            if let Some(value) = self.define_string_split_element(
                realm,
                &result,
                &mut length,
                Value::String(source.sub_string(
                    usize::try_from(start).expect("non-negative split start fits usize"),
                    usize::try_from(end).expect("non-negative split end fits usize"),
                )),
            )? {
                return Ok(Completion::Throw(value));
            }
            if length == limit {
                return Ok(Completion::Return(Value::Object(result)));
            }
            start = end + separator_len_i32;
        }
        if let Some(value) = self.define_string_split_element(
            realm,
            &result,
            &mut length,
            Value::String(source.sub_string(
                usize::try_from(start).expect("non-negative split tail start fits usize"),
                source_len,
            )),
        )? {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    /// CreateDataProperty on the fresh result Array. `JsString::MAX_LEN` keeps
    /// the algorithm below the Uint32 ceiling, but retain checked bookkeeping
    /// so a future string-limit expansion cannot silently wrap the output key.
    fn define_string_split_element(
        &self,
        realm: ContextId,
        result: &ObjectRef,
        length: &mut u32,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        let index = *length;
        let next = index.checked_add(1).ok_or(RuntimeError::Invariant(
            "String split output index exceeded Uint32",
        ))?;
        if let Some(value) = self.create_array_data_property(realm, result, index, value)? {
            return Ok(Some(value));
        }
        *length = next;
        Ok(None)
    }

    /// Rust port of pinned QuickJS `js_string_substring`, `js_string_substr`,
    /// and `js_string_slice`. The native identities remain generic functions;
    /// only their conversion and UTF-16 copying machinery is shared here.
    pub(crate) fn call_string_prototype_subrange(
        &self,
        realm: ContextId,
        selector: StringSubrangeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        search::finish(
            self,
            realm,
            search::StringSearchStep::start(
                self,
                realm,
                search::StringSearchKind::Subrange(selector),
                &invocation,
                arguments,
            )?,
        )
    }
    fn finish_string_subrange(
        &self,
        selector: StringSubrangeKind,
        source: JsString,
        start_number: f64,
        end_number: Option<f64>,
    ) -> Result<Completion, RuntimeError> {
        let source_len = i32::try_from(source.len()).map_err(|_| {
            RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
        })?;
        let start_offset = match selector {
            StringSubrangeKind::Substring => 0,
            StringSubrangeKind::Substr | StringSubrangeKind::Slice => source_len,
        };
        let start = string_to_int32_clamp(start_number, source_len, start_offset);

        let (range_start, range_end) = match selector {
            StringSubrangeKind::Substring => {
                let mut end = source_len;
                if let Some(number) = end_number {
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
                if let Some(number) = end_number {
                    count = string_to_int32_clamp(number, remaining, 0);
                }
                (start, start + count)
            }
            StringSubrangeKind::Slice => {
                let mut end = source_len;
                if let Some(number) = end_number {
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
    pub(crate) fn call_string_prototype_repeat(
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
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::Repeat,
                &invocation,
                Some(arguments),
                string_limit,
            )?,
        )
    }
    fn finish_string_repeat(
        &self,
        realm: ContextId,
        source: JsString,
        count: i64,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
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
    pub(crate) fn call_string_prototype_pad(
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
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::Pad(selector),
                &invocation,
                Some(arguments),
                string_limit,
            )?,
        )
    }
    fn finish_string_pad(
        &self,
        realm: ContextId,
        selector: StringPadKind,
        source: JsString,
        target: i32,
        filler: Option<JsString>,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
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
    pub(crate) fn call_string_prototype_trim(
        &self,
        realm: ContextId,
        selector: StringTrimKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::Trim(selector),
                &invocation,
                None,
                JsString::MAX_LEN,
            )?,
        )
    }
    fn finish_string_trim(
        &self,
        realm: ContextId,
        selector: StringTrimKind,
        source: JsString,
    ) -> Result<Completion, RuntimeError> {
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

    /// Rust port of pinned QuickJS `js_string_toLowerCase`. Its magic bit
    /// selects upper/lower conversion; locale-named callers reach the exact
    /// same kernel and no argument is inspected.
    pub(crate) fn call_string_prototype_case(
        &self,
        realm: ContextId,
        selector: StringCaseKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_prototype_case_with_limit(realm, selector, invocation, JsString::MAX_LEN)
    }

    fn call_string_prototype_case_with_limit(
        &self,
        realm: ContextId,
        selector: StringCaseKind,
        invocation: NativeInvocation,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::Case(selector),
                &invocation,
                None,
                string_limit,
            )?,
        )
    }
    fn finish_string_case(
        &self,
        realm: ContextId,
        selector: StringCaseKind,
        source: JsString,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let converted = match crate::source::unicode::case::convert_case_with_limit(
            &source,
            matches!(selector, StringCaseKind::Upper),
            string_limit,
        ) {
            Ok(value) => value,
            Err(error @ (JsStringError::TooLong | JsStringError::OutOfMemory)) => {
                let message = match error {
                    JsStringError::TooLong => "string too long",
                    JsStringError::OutOfMemory => "out of memory",
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    message,
                )?));
            }
        };
        Ok(Completion::Return(Value::String(converted)))
    }

    /// Rust port of pinned QuickJS `js_string_normalize`. Receiver coercion
    /// precedes form coercion, omitted and undefined forms select NFC, and the
    /// accepted form spellings are exact ASCII matches.
    pub(crate) fn call_string_prototype_normalize(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_prototype_normalize_with_limit(
            realm,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    fn call_string_prototype_normalize_with_limit(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::Normalize,
                &invocation,
                Some(arguments),
                string_limit,
            )?,
        )
    }
    fn finish_string_normalize(
        &self,
        realm: ContextId,
        source: JsString,
        form: crate::source::unicode::normalize::NormalizationForm,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        let normalized = match crate::source::unicode::normalize::normalize_with_limit(
            &source,
            form,
            string_limit,
        ) {
            Ok(value) => value,
            Err(error @ (JsStringError::TooLong | JsStringError::OutOfMemory)) => {
                let message = match error {
                    JsStringError::TooLong => "string too long",
                    JsStringError::OutOfMemory => "out of memory",
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    message,
                )?));
            }
        };
        Ok(Completion::Return(Value::String(normalized)))
    }

    /// Rust port of pinned QuickJS `js_string_localeCompare`. QuickJS's
    /// non-Intl build deliberately ignores locales and options: it coerces
    /// only the receiver and first argument, NFC-normalizes both to UTF-32,
    /// then returns the raw first code-point difference (or +/-1 for a
    /// proper-prefix ordering).
    pub(crate) fn call_string_prototype_locale_compare(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::LocaleCompare,
                &invocation,
                Some(arguments),
                JsString::MAX_LEN,
            )?,
        )
    }
    fn finish_string_locale_compare(
        &self,
        realm: ContextId,
        source: JsString,
        that: JsString,
    ) -> Result<Completion, RuntimeError> {
        let source = match crate::source::unicode::normalize::normalize_code_points(
            &source,
            crate::source::unicode::normalize::NormalizationForm::Nfc,
        ) {
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
                    "NFC code-point normalization reported a string length limit",
                ));
            }
        };
        let that = match crate::source::unicode::normalize::normalize_code_points(
            &that,
            crate::source::unicode::normalize::NormalizationForm::Nfc,
        ) {
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
                    "NFC code-point normalization reported a string length limit",
                ));
            }
        };

        let comparison = source
            .iter()
            .zip(&that)
            .find_map(|(left, right)| (*left != *right).then_some(*left as i32 - *right as i32))
            .unwrap_or_else(|| match source.len().cmp(&that.len()) {
                std::cmp::Ordering::Less => -1,
                std::cmp::Ordering::Equal => 0,
                std::cmp::Ordering::Greater => 1,
            });
        Ok(Completion::Return(Value::Int(comparison)))
    }

    /// Rust port of pinned QuickJS `js_string_CreateHTML`. Receiver coercion
    /// precedes buffer initialization; attribute variants then coerce exactly
    /// argv[0] after the prefix writes, even when those writes already latched
    /// a recoverable buffer error.
    pub(crate) fn call_string_prototype_create_html(
        &self,
        realm: ContextId,
        selector: StringCreateHtmlKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_prototype_create_html_with_limit(
            realm,
            selector,
            invocation,
            arguments,
            JsString::MAX_LEN,
        )
    }

    fn call_string_prototype_create_html_with_limit(
        &self,
        realm: ContextId,
        selector: StringCreateHtmlKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        string_limit: usize,
    ) -> Result<Completion, RuntimeError> {
        text::finish(
            self,
            realm,
            text::StringTextStep::start_with_limit(
                self,
                realm,
                text::StringTextKind::Html(selector),
                &invocation,
                Some(arguments),
                string_limit,
            )?,
        )
    }
    fn finish_string_create_html(
        &self,
        realm: ContextId,
        source: JsString,
        buffer: CreateHtmlStringBuffer,
        tag: &'static str,
    ) -> Result<Completion, RuntimeError> {
        let result = match buffer.finish(&source, tag) {
            Ok(value) => value,
            Err(error @ (JsStringError::TooLong | JsStringError::OutOfMemory)) => {
                let message = match error {
                    JsStringError::TooLong => "string too long",
                    JsStringError::OutOfMemory => "out of memory",
                };
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    message,
                )?));
            }
        };
        Ok(Completion::Return(Value::String(result)))
    }
}
