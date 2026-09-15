//! Ordered text coercions share the existing callback-free string kernels.
use super::create_html_definition;
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{StringCaseKind, StringCreateHtmlKind, StringPadKind, StringTrimKind},
    heap::ContextId,
    value::{CreateHtmlStringBuffer, JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};
use crate::source::unicode::normalize::NormalizationForm;
#[derive(Clone, Copy)]
pub(crate) enum StringTextKind {
    Trim(StringTrimKind),
    Case(StringCaseKind),
    Repeat,
    Pad(StringPadKind),
    Normalize,
    LocaleCompare,
    Html(StringCreateHtmlKind),
}
impl StringTextKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::StringPrototypeTrim(kind) => Self::Trim(kind),
            NativeFunctionId::StringPrototypeCase(kind) => Self::Case(kind),
            NativeFunctionId::StringPrototypeRepeat => Self::Repeat,
            NativeFunctionId::StringPrototypePad(kind) => Self::Pad(kind),
            NativeFunctionId::StringPrototypeNormalize => Self::Normalize,
            NativeFunctionId::StringPrototypeLocaleCompare => Self::LocaleCompare,
            NativeFunctionId::StringPrototypeCreateHtml(kind) => Self::Html(kind),
            _ => return None,
        })
    }
}
pub(crate) enum StringTextStep {
    Complete(Completion),
    Primitive {
        value: Value,
        hint: ToPrimitiveHint,
        resume: StringTextResume,
    },
}
pub(crate) struct StringTextResume(Box<StringTextResumeState>);
impl std::ops::Deref for StringTextResume {
    type Target = StringTextResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for StringTextResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<StringTextResume>() <= 8);
pub(crate) struct StringTextResumeState {
    realm: ContextId,
    kind: StringTextKind,
    first: Value,
    second: Value,
    actual: usize,
    limit: usize,
    phase: TextPhase,
}
enum TextPhase {
    Source,
    Count(JsString),
    Target(JsString),
    Filler {
        source: JsString,
        target: i32,
    },
    Form(JsString),
    Compare(JsString),
    Attribute {
        source: JsString,
        buffer: CreateHtmlStringBuffer,
        tag: &'static str,
    },
}
impl StringTextStep {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: StringTextKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_limit(
            runtime,
            realm,
            kind,
            invocation,
            Some(arguments),
            JsString::MAX_LEN,
        )
    }
    pub(super) fn start_with_limit(
        runtime: &Runtime,
        realm: ContextId,
        kind: StringTextKind,
        invocation: &NativeInvocation,
        arguments: Option<&NativeArguments>,
        limit: usize,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String text conversion did not receive a call",
            ));
        };
        if matches!(this_value, Value::Undefined | Value::Null) {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "null or undefined are forbidden",
                )?,
            )));
        }
        Ok(Self::Primitive {
            value: this_value.clone(),
            hint: ToPrimitiveHint::String,
            resume: StringTextResume(Box::new(StringTextResumeState {
                realm,
                kind,
                first: arguments
                    .and_then(|args| args.readable.first())
                    .cloned()
                    .unwrap_or(Value::Undefined),
                second: arguments
                    .and_then(|args| args.readable.get(1))
                    .cloned()
                    .unwrap_or(Value::Undefined),
                actual: arguments.map_or(0, |args| args.actual_arg_count),
                limit,
                phase: TextPhase::Source,
            })),
        })
    }
}
impl StringTextResume {
    fn convert(mut self, value: Value, hint: ToPrimitiveHint, phase: TextPhase) -> StringTextStep {
        StringTextStep::Primitive {
            value,
            hint,
            resume: {
                let updated_0 = phase;
                self.0.phase = updated_0;
                self
            },
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<StringTextStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(StringTextStep::Complete(Completion::Throw(value)));
            }
        };
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "String text conversion returned an object",
            ));
        }
        let realm = self.0.realm;
        let result = match self.0.phase {
            TextPhase::Source => {
                let source = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                match self.0.kind {
                    StringTextKind::Trim(kind) => {
                        runtime.finish_string_trim(realm, kind, source)?
                    }
                    StringTextKind::Case(kind) => {
                        runtime.finish_string_case(realm, kind, source, self.0.limit)?
                    }
                    StringTextKind::Repeat => {
                        let argument = self.0.first.clone();
                        return Ok(self.convert(
                            argument,
                            ToPrimitiveHint::Number,
                            TextPhase::Count(source),
                        ));
                    }
                    StringTextKind::Pad(_) => {
                        let argument = self.0.first.clone();
                        return Ok(self.convert(
                            argument,
                            ToPrimitiveHint::Number,
                            TextPhase::Target(source.linearize()),
                        ));
                    }
                    StringTextKind::Normalize => {
                        if self.0.actual == 0 || matches!(self.0.first, Value::Undefined) {
                            runtime.finish_string_normalize(
                                realm,
                                source,
                                NormalizationForm::Nfc,
                                self.0.limit,
                            )?
                        } else {
                            let argument = self.0.first.clone();
                            return Ok(self.convert(
                                argument,
                                ToPrimitiveHint::String,
                                TextPhase::Form(source),
                            ));
                        }
                    }
                    StringTextKind::LocaleCompare => {
                        let argument = self.0.first.clone();
                        return Ok(self.convert(
                            argument,
                            ToPrimitiveHint::String,
                            TextPhase::Compare(source),
                        ));
                    }
                    StringTextKind::Html(kind) => {
                        let source = source.linearize();
                        let (tag, attribute) = create_html_definition(kind);
                        let buffer = CreateHtmlStringBuffer::new(tag, attribute, self.0.limit);
                        if attribute.is_some() {
                            if matches!(self.0.first, Value::Undefined | Value::Null) {
                                return Ok(StringTextStep::Complete(Completion::Throw(
                                    runtime.new_native_error(
                                        realm,
                                        NativeErrorKind::Type,
                                        "null or undefined are forbidden",
                                    )?,
                                )));
                            }
                            let argument = self.0.first.clone();
                            return Ok(self.convert(
                                argument,
                                ToPrimitiveHint::String,
                                TextPhase::Attribute {
                                    source,
                                    buffer,
                                    tag,
                                },
                            ));
                        }
                        runtime.finish_string_create_html(realm, source, buffer, tag)?
                    }
                }
            }
            TextPhase::Count(source) => {
                let count = match runtime.native_to_int64_sat(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                runtime.finish_string_repeat(realm, source, count, self.0.limit)?
            }
            TextPhase::Target(source) => {
                let target = match runtime.native_to_number(realm, &value)? {
                    NativeConversion::Value(value) => {
                        crate::engine::value::number::to_int32_sat(value)
                    }
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                let source_len = i32::try_from(source.len())
                    .map_err(|_| RuntimeError::Invariant("String length exceeded signed Int32"))?;
                if source_len >= target {
                    Completion::Return(Value::String(source))
                } else if self.0.actual > 1 && !matches!(self.0.second, Value::Undefined) {
                    let argument = self.0.second.clone();
                    return Ok({
                        let updated_0 = TextPhase::Source;
                        self.0.phase = updated_0;
                        self
                    }
                    .convert(
                        argument,
                        ToPrimitiveHint::String,
                        TextPhase::Filler { source, target },
                    ));
                } else {
                    let StringTextKind::Pad(kind) = self.0.kind else {
                        return Err(RuntimeError::Invariant("String pad lost its kind"));
                    };
                    runtime.finish_string_pad(realm, kind, source, target, None, self.0.limit)?
                }
            }
            TextPhase::Filler { source, target } => {
                let filler = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value.linearize(),
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                let StringTextKind::Pad(kind) = self.0.kind else {
                    return Err(RuntimeError::Invariant("String pad lost its kind"));
                };
                runtime.finish_string_pad(
                    realm,
                    kind,
                    source,
                    target,
                    Some(filler),
                    self.0.limit,
                )?
            }
            TextPhase::Form(source) => {
                let form = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                let form = if form.utf16_units().eq("NFC".encode_utf16()) {
                    NormalizationForm::Nfc
                } else if form.utf16_units().eq("NFD".encode_utf16()) {
                    NormalizationForm::Nfd
                } else if form.utf16_units().eq("NFKC".encode_utf16()) {
                    NormalizationForm::Nfkc
                } else if form.utf16_units().eq("NFKD".encode_utf16()) {
                    NormalizationForm::Nfkd
                } else {
                    return Ok(StringTextStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            realm,
                            NativeErrorKind::Range,
                            "bad normalization form",
                        )?,
                    )));
                };
                runtime.finish_string_normalize(realm, source, form, self.0.limit)?
            }
            TextPhase::Compare(source) => {
                let that = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                runtime.finish_string_locale_compare(realm, source, that)?
            }
            TextPhase::Attribute {
                source,
                mut buffer,
                tag,
            } => {
                let attribute = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value.linearize(),
                    NativeConversion::Throw(value) => {
                        return Ok(StringTextStep::Complete(Completion::Throw(value)));
                    }
                };
                buffer.append_escaped_attribute(&attribute);
                runtime.finish_string_create_html(realm, source, buffer, tag)?
            }
        };
        Ok(StringTextStep::Complete(result))
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: StringTextStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            StringTextStep::Complete(result) => return Ok(result),
            StringTextStep::Primitive {
                value,
                hint,
                resume,
            } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, hint)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<StringTextStep>() <= 64);
