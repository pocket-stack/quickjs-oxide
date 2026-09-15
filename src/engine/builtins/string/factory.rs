//! String factories share conversion order and String.raw's latched buffer error.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::StringStaticKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{JsString, JsStringBuilder, Value, conversion::NativeConversion},
    vm::{Completion, call::NativeArguments},
};
#[derive(Clone, Copy)]
pub(crate) enum StringFactoryKind {
    Static(StringStaticKind),
    #[cfg(feature = "test262-host")]
    CodePointRange,
}
impl StringFactoryKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::StringStatic(kind) => Some(Self::Static(kind)),
            #[cfg(feature = "test262-host")]
            NativeFunctionId::StringCodePointRange => Some(Self::CodePointRange),
            _ => None,
        }
    }
}
pub(crate) enum StringFactoryStep {
    Complete(Completion),
    Number {
        value: Value,
        resume: StringFactoryResume,
    },
    String {
        value: Value,
        resume: StringFactoryResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: StringFactoryResume,
    },
}
enum Phase {
    Characters,
    Raw,
    Length,
    Chunk,
    Substitution,
    #[cfg(feature = "test262-host")]
    RangeStart,
    #[cfg(feature = "test262-host")]
    RangeEnd(u32),
}
pub(crate) struct StringFactoryResume(Box<StringFactoryResumeState>);
impl std::ops::Deref for StringFactoryResume {
    type Target = StringFactoryResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for StringFactoryResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<StringFactoryResume>() <= 8);
pub(crate) struct StringFactoryResumeState {
    realm: ContextId,
    kind: StringFactoryKind,
    arguments: Vec<Value>,
    actual: usize,
    cooked: Option<ObjectRef>,
    raw: Option<ObjectRef>,
    chunk: Value,
    length_value: Value,
    length: u64,
    index: u64,
    builder: Option<JsStringBuilder>,
    limit: usize,
    phase: Phase,
}
impl StringFactoryStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: StringFactoryKind,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        Self::with_limit(runtime, realm, kind, arguments, JsString::MAX_LEN)
    }
    pub(crate) fn with_limit(
        runtime: &Runtime,
        realm: ContextId,
        kind: StringFactoryKind,
        arguments: &NativeArguments,
        limit: usize,
    ) -> Result<Self, RuntimeError> {
        let mut resume = StringFactoryResume(Box::new(StringFactoryResumeState {
            realm,
            kind,
            arguments: arguments.readable.clone(),
            actual: arguments.actual_arg_count,
            cooked: None,
            raw: None,
            chunk: Value::Undefined,
            length_value: Value::Undefined,
            length: 0,
            index: 0,
            builder: None,
            limit,
            phase: Phase::Characters,
        }));
        match kind {
            StringFactoryKind::Static(StringStaticKind::Raw) => {
                let template = resume.arguments.first().ok_or(RuntimeError::Invariant(
                    "String.raw template argv was not padded",
                ))?;
                let cooked = match runtime.native_to_object(realm, template.clone())? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(Self::Complete(Completion::Throw(value)));
                    }
                };
                resume.cooked = Some(cooked.clone());
                resume.phase = Phase::Raw;
                Ok(Self::Read {
                    object: cooked,
                    key: runtime.intern_property_key("raw")?,
                    resume,
                })
            }
            StringFactoryKind::Static(_) => {
                resume.builder = Some(JsStringBuilder::new(arguments.actual_arg_count));
                resume.next(runtime)
            }
            #[cfg(feature = "test262-host")]
            StringFactoryKind::CodePointRange => {
                resume.phase = Phase::RangeStart;
                let value = resume
                    .arguments
                    .first()
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "String codePointRange start argv was not padded",
                    ))?;
                Ok(Self::Number { value, resume })
            }
        }
    }
}
impl StringFactoryResume {
    fn abrupt(self, value: Value) -> StringFactoryStep {
        StringFactoryStep::Complete(Completion::Throw(value))
    }
    fn complete(mut self) -> Result<StringFactoryStep, RuntimeError> {
        let builder = self
            .0
            .builder
            .take()
            .ok_or(RuntimeError::Invariant("String factory lost builder"))?;
        Ok(StringFactoryStep::Complete(Completion::Return(
            Value::String(builder.finish()?),
        )))
    }
    fn builder(&mut self) -> Result<&mut JsStringBuilder, RuntimeError> {
        self.0
            .builder
            .as_mut()
            .ok_or(RuntimeError::Invariant("String factory lost builder"))
    }
    fn next(mut self, runtime: &Runtime) -> Result<StringFactoryStep, RuntimeError> {
        if matches!(
            self.0.kind,
            StringFactoryKind::Static(StringStaticKind::Raw)
        ) {
            self.0.chunk = Value::Undefined;
            if self.0.index == self.0.length {
                return self.complete();
            }
            self.0.phase = Phase::Chunk;
            return Ok(StringFactoryStep::Read {
                object: self
                    .0
                    .raw
                    .as_ref()
                    .ok_or(RuntimeError::Invariant("String.raw lost raw object"))?
                    .clone(),
                key: runtime.intern_property_key(&self.0.index.to_string())?,
                resume: self,
            });
        }
        while self.0.index < self.0.actual as u64 {
            let value = self
                .0
                .arguments
                .get(self.0.index as usize)
                .cloned()
                .ok_or(RuntimeError::Invariant("String factory argument missing"))?;
            if matches!(
                self.0.kind,
                StringFactoryKind::Static(StringStaticKind::FromCodePoint)
            ) {
                if let Value::Int(value) = value {
                    if !(0..=0x10_ffff).contains(&value) {
                        let error = runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Range,
                            "invalid code point",
                        )?;
                        return Ok(self.abrupt(error));
                    }
                    self.builder()?.push_code_point(value as u32)?;
                    self.0.index += 1;
                    continue;
                }
            }
            return Ok(StringFactoryStep::Number {
                value,
                resume: self,
            });
        }
        self.complete()
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<StringFactoryStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        };
        match self.0.phase {
            Phase::Characters => {
                let code_point = if matches!(
                    self.0.kind,
                    StringFactoryKind::Static(StringStaticKind::FromCharCode)
                ) {
                    (crate::engine::value::number::to_int32(number) as u32) & 0xffff
                } else {
                    if !number.is_finite()
                        || number < 0.0
                        || number > 0x10_ffff as f64
                        || number.fract() != 0.0
                    {
                        let error = runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Range,
                            "invalid code point",
                        )?;
                        return Ok(self.abrupt(error));
                    }
                    number as u32
                };
                self.builder()?.push_code_point(code_point)?;
                self.0.index += 1;
                self.next(runtime)
            }
            Phase::Length => {
                self.0.length =
                    match runtime.native_to_length(self.0.realm, &Value::number(number))? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                    };
                self.0.builder = Some(JsStringBuilder::with_limit(0, self.0.limit));
                self.next(runtime)
            }
            #[cfg(feature = "test262-host")]
            Phase::RangeStart => {
                self.0.phase = Phase::RangeEnd(Runtime::to_uint32_number(number));
                let value = self
                    .0
                    .arguments
                    .get(1)
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "String codePointRange end argv was not padded",
                    ))?;
                Ok(StringFactoryStep::Number {
                    value,
                    resume: self,
                })
            }
            #[cfg(feature = "test262-host")]
            Phase::RangeEnd(start) => {
                let end = Runtime::to_uint32_number(number).min(0x11_0000);
                let start = start.min(end);
                let length = usize::try_from(end - start + end.saturating_sub(start.max(0x1_0000)))
                    .map_err(|_| {
                        RuntimeError::Invariant("codePointRange length did not fit usize")
                    })?;
                let mut builder = JsStringBuilder::try_with_exact_capacity(length)?;
                for point in start..end {
                    builder.push_code_point(point)?;
                }
                Ok(StringFactoryStep::Complete(Completion::Return(
                    Value::String(builder.finish()?),
                )))
            }
            _ => Err(RuntimeError::Invariant(
                "String factory number phase mismatch",
            )),
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<StringFactoryStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(self.abrupt(value)),
        };
        match self.0.phase {
            Phase::Raw => {
                let raw = match runtime.native_to_object(self.0.realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                };
                self.0.raw = Some(raw.clone());
                self.0.phase = Phase::Length;
                Ok(StringFactoryStep::Read {
                    object: raw,
                    key: runtime.intern_property_key("length")?,
                    resume: self,
                })
            }
            Phase::Length => {
                self.0.length_value = value.clone();
                Ok(StringFactoryStep::Number {
                    value,
                    resume: self,
                })
            }
            Phase::Chunk => {
                self.0.chunk = value.clone();
                Ok(StringFactoryStep::String {
                    value,
                    resume: self,
                })
            }
            _ => Err(RuntimeError::Invariant(
                "String factory read phase mismatch",
            )),
        }
    }
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<StringFactoryStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        };
        match self.0.phase {
            Phase::Chunk => {
                let append = self.builder()?.push_js_string(&value);
                let next = self.0.index + 1;
                let substitution = usize::try_from(next)
                    .ok()
                    .filter(|index| next < self.0.length && *index < self.0.actual);
                let Some(index) = substitution else {
                    // Raw append failure is latched; subsequent Get/ToString
                    // still run and a later user throw can replace that error.
                    let _ = append;
                    self.0.index += 1;
                    return self.next(runtime);
                };
                append?;
                self.0.phase = Phase::Substitution;
                let value = self
                    .0
                    .arguments
                    .get(index)
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "String.raw substitution argv was not readable",
                    ))?;
                Ok(StringFactoryStep::String {
                    value,
                    resume: self,
                })
            }
            Phase::Substitution => {
                self.builder()?.push_js_string(&value)?;
                self.0.index += 1;
                self.next(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "String factory string phase mismatch",
            )),
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: StringFactoryStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            StringFactoryStep::Complete(result) => return Ok(result),
            StringFactoryStep::Number { value, resume } => {
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            StringFactoryStep::String { value, resume } => {
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            StringFactoryStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<StringFactoryStep>() <= 64);
