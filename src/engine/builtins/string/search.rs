//! String search and subrange coercions preserve their distinct position rules.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::{StringIncludesKind, StringIndexOfKind, StringSubrangeKind},
    heap::ContextId,
    object::{ObjectRef, PropertyKey, WellKnownSymbol},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum StringSearchKind {
    Index(StringIndexOfKind),
    Includes(StringIncludesKind),
    Subrange(StringSubrangeKind),
}
impl StringSearchKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::StringPrototypeIndexOf(kind) => Self::Index(kind),
            NativeFunctionId::StringPrototypeIncludes(kind) => Self::Includes(kind),
            NativeFunctionId::StringPrototypeSubrange(kind) => Self::Subrange(kind),
            _ => return None,
        })
    }
}
pub(crate) enum StringSearchStep {
    Complete(Completion),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: StringSearchResume,
    },
    Primitive {
        value: Value,
        hint: ToPrimitiveHint,
        resume: StringSearchResume,
    },
}
pub(crate) struct StringSearchResume(Box<StringSearchResumeState>);
impl std::ops::Deref for StringSearchResume {
    type Target = StringSearchResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for StringSearchResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<StringSearchResume>() <= 8);
pub(crate) struct StringSearchResumeState {
    realm: ContextId,
    kind: StringSearchKind,
    first: Value,
    second: Value,
    actual: usize,
    phase: SearchPhase,
}
enum SearchPhase {
    Source,
    Regexp(JsString),
    Needle(JsString),
    Position { source: JsString, needle: JsString },
    Start(JsString),
    End { source: JsString, start: f64 },
}
impl StringSearchStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: StringSearchKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String search did not receive a call",
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
            resume: StringSearchResume(Box::new(StringSearchResumeState {
                realm,
                kind,
                first: arguments
                    .readable
                    .first()
                    .ok_or(RuntimeError::Invariant(
                        "String search first argument was not padded",
                    ))?
                    .clone(),
                second: arguments
                    .readable
                    .get(1)
                    .cloned()
                    .unwrap_or(Value::Undefined),
                actual: arguments.actual_arg_count,
                phase: SearchPhase::Source,
            })),
        })
    }
}
impl StringSearchResume {
    fn primitive(
        mut self,
        value: Value,
        hint: ToPrimitiveHint,
        phase: SearchPhase,
    ) -> StringSearchStep {
        StringSearchStep::Primitive {
            value,
            hint,
            resume: {
                let updated_0 = phase;
                self.0.phase = updated_0;
                self
            },
        }
    }
    fn needle(self, source: JsString) -> StringSearchStep {
        let value = self.0.first.clone();
        self.primitive(value, ToPrimitiveHint::String, SearchPhase::Needle(source))
    }
    fn finish_search(
        &self,
        runtime: &Runtime,
        source: JsString,
        needle: JsString,
        position: Option<f64>,
    ) -> Result<StringSearchStep, RuntimeError> {
        Ok(StringSearchStep::Complete(match self.0.kind {
            StringSearchKind::Index(kind) => {
                runtime.finish_string_index_of(kind, source, needle, position)?
            }
            StringSearchKind::Includes(kind) => {
                runtime.finish_string_includes(kind, source, needle, position)?
            }
            _ => {
                return Err(RuntimeError::Invariant(
                    "String position reply lost its kind",
                ));
            }
        }))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<StringSearchStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(StringSearchStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            SearchPhase::Source => {
                let source = match string_value(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                match self.0.kind {
                    StringSearchKind::Subrange(_) => {
                        i32::try_from(source.len()).map_err(|_| {
                            RuntimeError::Invariant(
                                "String length exceeded QuickJS's signed index range",
                            )
                        })?;
                        let value = self.0.first.clone();
                        Ok(self.primitive(
                            value,
                            ToPrimitiveHint::Number,
                            SearchPhase::Start(source),
                        ))
                    }
                    StringSearchKind::Includes(_) => {
                        if let Value::Object(object) = &self.0.first {
                            Ok(StringSearchStep::Read {
                                object: object.clone(),
                                key: PropertyKey::from(
                                    runtime.well_known_symbol(WellKnownSymbol::Match),
                                ),
                                resume: {
                                    let updated_0 = SearchPhase::Regexp(source);
                                    self.0.phase = updated_0;
                                    self
                                },
                            })
                        } else {
                            Ok(self.needle(source))
                        }
                    }
                    StringSearchKind::Index(_) => Ok(self.needle(source)),
                }
            }
            SearchPhase::Regexp(source) => {
                let Value::Object(object) = &self.0.first else {
                    return Err(RuntimeError::Invariant("String IsRegExp lost its object"));
                };
                let regexp = runtime.is_regexp_from_match(object, &value)?;
                if regexp {
                    return Ok(StringSearchStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "regexp not supported",
                        )?,
                    )));
                }
                Ok({
                    let updated_0 = SearchPhase::Source;
                    self.0.phase = updated_0;
                    self
                }
                .needle(source))
            }
            SearchPhase::Needle(source) => {
                let needle = match string_value(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                i32::try_from(source.len()).map_err(|_| {
                    RuntimeError::Invariant("String length exceeded QuickJS's signed index range")
                })?;
                i32::try_from(needle.len()).map_err(|_| {
                    RuntimeError::Invariant(
                        "String search length exceeded QuickJS's signed index range",
                    )
                })?;
                let next = {
                    let updated_0 = SearchPhase::Source;
                    self.0.phase = updated_0;
                    self
                };
                if next.actual > 1
                    && !(matches!(next.kind, StringSearchKind::Includes(_))
                        && matches!(next.second, Value::Undefined))
                {
                    let value = next.second.clone();
                    Ok(next.primitive(
                        value,
                        ToPrimitiveHint::Number,
                        SearchPhase::Position { source, needle },
                    ))
                } else {
                    next.finish_search(runtime, source, needle, None)
                }
            }
            SearchPhase::Position { source, needle } => {
                let position = match number_value(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                {
                    let updated_0 = SearchPhase::Source;
                    self.0.phase = updated_0;
                    self
                }
                .finish_search(runtime, source, needle, Some(position))
            }
            SearchPhase::Start(source) => {
                let start = match number_value(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                let next = {
                    let updated_0 = SearchPhase::Source;
                    self.0.phase = updated_0;
                    self
                };
                if matches!(next.second, Value::Undefined) {
                    let StringSearchKind::Subrange(kind) = next.kind else {
                        return Err(RuntimeError::Invariant("String start reply lost its kind"));
                    };
                    Ok(StringSearchStep::Complete(
                        runtime.finish_string_subrange(kind, source, start, None)?,
                    ))
                } else {
                    let value = next.second.clone();
                    Ok(next.primitive(
                        value,
                        ToPrimitiveHint::Number,
                        SearchPhase::End { source, start },
                    ))
                }
            }
            SearchPhase::End { source, start } => {
                let end = match number_value(runtime, realm, value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(StringSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                let StringSearchKind::Subrange(kind) = self.0.kind else {
                    return Err(RuntimeError::Invariant("String end reply lost its kind"));
                };
                Ok(StringSearchStep::Complete(runtime.finish_string_subrange(
                    kind,
                    source,
                    start,
                    Some(end),
                )?))
            }
        }
    }
}
fn string_value(
    runtime: &Runtime,
    realm: ContextId,
    value: Value,
) -> Result<NativeConversion<JsString>, RuntimeError> {
    if matches!(value, Value::Object(_)) {
        return Err(RuntimeError::Invariant(
            "String search conversion returned an object",
        ));
    }
    runtime.native_to_js_string(realm, &value)
}
fn number_value(
    runtime: &Runtime,
    realm: ContextId,
    value: Value,
) -> Result<NativeConversion<f64>, RuntimeError> {
    if matches!(value, Value::Object(_)) {
        return Err(RuntimeError::Invariant(
            "String search position conversion returned an object",
        ));
    }
    runtime.native_to_number(realm, &value)
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: StringSearchStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            StringSearchStep::Complete(result) => return Ok(result),
            StringSearchStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            StringSearchStep::Primitive {
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
const _: () = assert!(std::mem::size_of::<StringSearchStep>() <= 64);
