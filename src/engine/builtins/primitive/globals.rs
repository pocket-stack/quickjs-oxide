//! Global parsers and codecs keep their converted input until later argument conversion ends.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{
        GlobalNumberPredicateKind, GlobalUriCodecKind, NumberParseKind, SymbolRegistryKind,
    },
    heap::ContextId,
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum GlobalKind {
    Parse(NumberParseKind),
    Predicate(GlobalNumberPredicateKind),
    Uri(GlobalUriCodecKind),
    SymbolFor,
}
impl GlobalKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::GlobalNumberParse(kind) => Self::Parse(kind),
            NativeFunctionId::GlobalNumberPredicate(kind) => Self::Predicate(kind),
            NativeFunctionId::GlobalUriCodec(kind) => Self::Uri(kind),
            NativeFunctionId::SymbolRegistry(SymbolRegistryKind::For) => Self::SymbolFor,
            _ => return None,
        })
    }
}
pub(crate) enum GlobalStep {
    Complete(Completion),
    String { value: Value, resume: GlobalResume },
    Number { value: Value, resume: GlobalResume },
}
pub(crate) struct GlobalResume(Box<GlobalResumeState>);
impl std::ops::Deref for GlobalResume {
    type Target = GlobalResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for GlobalResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<GlobalResume>() <= 8);
pub(crate) struct GlobalResumeState {
    realm: ContextId,
    kind: GlobalKind,
    radix: Value,
    input: Option<JsString>,
}
impl GlobalStep {
    pub(crate) fn start(
        _runtime: &Runtime,
        realm: ContextId,
        kind: GlobalKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if !matches!(invocation, NativeInvocation::Call { .. }) {
            return Err(RuntimeError::Invariant(
                "global builtin requires generic invocation",
            ));
        }
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "global builtin argv was not padded",
            ))?;
        let resume = GlobalResume(Box::new(GlobalResumeState {
            realm,
            kind,
            radix: arguments
                .readable
                .get(1)
                .cloned()
                .unwrap_or(Value::Undefined),
            input: None,
        }));
        Ok(if matches!(kind, GlobalKind::Predicate(_)) {
            Self::Number { value, resume }
        } else {
            Self::String { value, resume }
        })
    }
    /// Advance only primitive conversion stages, retaining the original request
    /// before an object lookup or callback. Parse input precedes radix conversion.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn advance_primitive(
        mut self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Self, RuntimeError> {
        use crate::engine::value::conversion::number::NumberStep;
        loop {
            self = match self {
                Self::String { value, resume } if !matches!(value, Value::Object(_)) => {
                    resume.string(runtime, runtime.string_from_primitive(realm, &value)?)?
                }
                Self::Number { value, resume } if !matches!(value, Value::Object(_)) => {
                    let NumberStep::Complete(result) = NumberStep::start(runtime, realm, value)?
                    else {
                        return Err(RuntimeError::Invariant("primitive global number suspended"));
                    };
                    resume.number(result)?
                }
                Self::Complete(completion) => {
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_owned_execution_event(
                        "global_completed_without_waiting_state",
                    );
                    return Ok(Self::Complete(completion));
                }
                step => return Ok(step),
            };
        }
    }
}
impl GlobalResume {
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<GlobalStep, RuntimeError> {
        let input = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(GlobalStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.kind {
            GlobalKind::Parse(NumberParseKind::ParseInt) => {
                self.0.input = Some(input);
                Ok(GlobalStep::Number {
                    value: self.0.radix.clone(),
                    resume: self,
                })
            }
            GlobalKind::Parse(NumberParseKind::ParseFloat) => {
                Ok(GlobalStep::Complete(Completion::Return(Value::number(
                    crate::engine::value::number_parse::parse_float(&input),
                ))))
            }
            GlobalKind::Uri(kind) => Ok(GlobalStep::Complete(runtime.finish_global_uri_codec(
                self.0.realm,
                kind,
                input,
            )?)),
            GlobalKind::SymbolFor => Ok(GlobalStep::Complete(Completion::Return(Value::Symbol(
                runtime.symbol_for(&input)?,
            )))),
            _ => Err(RuntimeError::Invariant("global string reply mismatch")),
        }
    }
    pub(crate) fn number(self, result: NativeConversion<f64>) -> Result<GlobalStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(GlobalStep::Complete(Completion::Throw(value)));
            }
        };
        let value = match self.0.kind {
            GlobalKind::Parse(NumberParseKind::ParseInt) => {
                Value::number(crate::engine::value::number_parse::parse_int(
                    &self
                        .0
                        .input
                        .ok_or(RuntimeError::Invariant("parseInt converted input missing"))?,
                    crate::engine::value::number::to_int32(number),
                ))
            }
            GlobalKind::Predicate(kind) => Value::Bool(match kind {
                GlobalNumberPredicateKind::IsNaN => number.is_nan(),
                GlobalNumberPredicateKind::IsFinite => number.is_finite(),
            }),
            _ => return Err(RuntimeError::Invariant("global number reply mismatch")),
        };
        Ok(GlobalStep::Complete(Completion::Return(value)))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: GlobalStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            GlobalStep::Complete(result) => return Ok(result),
            GlobalStep::String { value, resume } => {
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            GlobalStep::Number { value, resume } => {
                resume.number(runtime.native_to_number(realm, &value)?)?
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<GlobalStep>() <= 64);
