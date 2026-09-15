//! `RegExp.prototype[Symbol.split]`.

use super::match_protocol::advance_string_index;
use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::ContextId;
use crate::engine::object::{ObjectRef, PropertyKey, operations::InternalSetResult};
use crate::engine::value::conversion::NativeConversion;

use crate::engine::value::{JsString, Value};
use crate::engine::vm::call::{ConstructorRef, NativeArguments, NativeInvocation};
use crate::engine::vm::{Completion, ToPrimitiveHint};

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_split`.
    pub(crate) fn call_regexp_symbol_split(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            RegExpSplitStep::start(self, realm, &invocation, arguments)?,
        )
    }

    fn append_regexp_split_value(
        &self,
        result: &ObjectRef,
        length: &mut u32,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let index = *length;
        let next = index.checked_add(1).ok_or(RuntimeError::Invariant(
            "RegExp split output index exceeded Uint32",
        ))?;
        // The output is an intrinsic fresh Array, never the species-created
        // splitter and never exposed to exec/capture callbacks. Preserve the
        // original allocation and append timing using the shared constructor
        // kernel, which defines own C/W/E data without inherited setters.
        self.append_fresh_array_value(result, value)?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("regexp_result.split_append");
        *length = next;
        Ok(())
    }
}

pub(crate) enum RegExpSplitStep {
    Complete(Completion),
    Primitive { resume: RegExpSplitResume },
    Read { resume: RegExpSplitResume },
    Species { resume: RegExpSplitResume },
    Construct { resume: RegExpSplitResume },
    Set { resume: RegExpSplitResume },
    Exec { resume: RegExpSplitResume },
}
pub(crate) struct RegExpSplitResume(Box<RegExpSplitResumeState>);
impl std::ops::Deref for RegExpSplitResume {
    type Target = RegExpSplitResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpSplitResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpSplitResume>() <= 8);
pub(crate) struct RegExpSplitResumeState {
    step_pending: RegExpSplitStepPending,
    realm: ContextId,
    phase: Phase,
}
enum Phase {
    Input {
        regexp: ObjectRef,
        limit: Value,
    },
    Species {
        regexp: ObjectRef,
        input: JsString,
        limit: Value,
    },
    Flags {
        regexp: ObjectRef,
        input: JsString,
        limit: Value,
        constructor: ConstructorRef,
    },
    FlagsPrimitive {
        regexp: ObjectRef,
        input: JsString,
        limit: Value,
        constructor: ConstructorRef,
    },
    Construct {
        input: JsString,
        limit: Value,
        unicode: bool,
    },
    Limit(SplitState),
    Empty(SplitState),
    Set(SplitState),
    Exec(SplitState),
    End {
        state: SplitState,
        matched: ObjectRef,
    },
    EndPrimitive {
        state: SplitState,
        matched: ObjectRef,
    },
    Count {
        state: SplitState,
        matched: ObjectRef,
    },
    CountPrimitive {
        state: SplitState,
        matched: ObjectRef,
    },
    Capture {
        state: SplitState,
        matched: ObjectRef,
        index: u64,
        count: u64,
    },
}
struct SplitState {
    input: JsString,
    splitter: ObjectRef,
    result: ObjectRef,
    unicode: bool,
    limit: u32,
    length: u32,
    p: usize,
    q: usize,
}
impl SplitState {
    fn complete(self) -> RegExpSplitStep {
        RegExpSplitStep::Complete(Completion::Return(Value::Object(self.result)))
    }
    fn append(&mut self, runtime: &Runtime, value: Value) -> Result<(), RuntimeError> {
        runtime.append_regexp_split_value(&self.result, &mut self.length, value)
    }
    fn advance(&mut self) -> Result<(), RuntimeError> {
        self.q = usize::try_from(advance_string_index(
            &self.input,
            self.q as u64,
            self.unicode,
        ))
        .map_err(|_| RuntimeError::Invariant("advanced split index did not fit usize"))?;
        Ok(())
    }
    fn next(
        mut self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<RegExpSplitStep, RuntimeError> {
        if self.q >= self.input.len() {
            let value = Value::String(
                self.input
                    .sub_string(self.p.min(self.input.len()), self.input.len()),
            );
            self.append(runtime, value)?;
            return Ok(self.complete());
        }
        let value = Value::Int(i32::try_from(self.q).map_err(|_| {
            RuntimeError::Invariant("RegExp split index exceeded signed String range")
        })?);
        Ok(RegExpSplitStep::make_set(
            self.splitter.clone(),
            runtime.intern_property_key("lastIndex")?,
            value,
            RegExpSplitResume(Box::new(RegExpSplitResumeState {
                step_pending: RegExpSplitStepPending::default(),
                realm,
                phase: Phase::Set(self),
            })),
        ))
    }
    fn execute(self, realm: ContextId, empty: bool) -> RegExpSplitStep {
        RegExpSplitStep::make_exec(
            Value::Object(self.splitter.clone()),
            Value::String(self.input.clone()),
            RegExpSplitResume(Box::new(RegExpSplitResumeState {
                step_pending: RegExpSplitStepPending::default(),
                realm,
                phase: if empty {
                    Phase::Empty(self)
                } else {
                    Phase::Exec(self)
                },
            })),
        )
    }
    fn captures(
        mut self,
        runtime: &Runtime,
        realm: ContextId,
        matched: ObjectRef,
        index: u64,
        count: u64,
    ) -> Result<RegExpSplitStep, RuntimeError> {
        if index >= count {
            self.q = self.p;
            return self.next(runtime, realm);
        }
        Ok(RegExpSplitStep::make_read(
            matched.clone(),
            runtime.intern_property_key(&index.to_string())?,
            RegExpSplitResume(Box::new(RegExpSplitResumeState {
                step_pending: RegExpSplitStepPending::default(),
                realm,
                phase: Phase::Capture {
                    state: self,
                    matched,
                    index,
                    count,
                },
            })),
        ))
    }
}
impl RegExpSplitStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@split did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = this_value else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(realm, NativeErrorKind::Type, "not an object")?,
            )));
        };
        let input = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "RegExp @@split input argv was not padded",
            ))?
            .clone();
        let limit = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant(
                "RegExp @@split limit argv was not padded",
            ))?
            .clone();
        Ok(Self::make_primitive(
            input,
            ToPrimitiveHint::String,
            RegExpSplitResume(Box::new(RegExpSplitResumeState {
                step_pending: RegExpSplitStepPending::default(),
                realm,
                phase: Phase::Input {
                    regexp: regexp.clone(),
                    limit,
                },
            })),
        ))
    }
}
impl RegExpSplitResume {
    pub(crate) fn species(
        self,
        runtime: &Runtime,
        result: NativeConversion<ConstructorRef>,
    ) -> Result<RegExpSplitStep, RuntimeError> {
        let constructor = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
            }
        };
        let Phase::Species {
            regexp,
            input,
            limit,
        } = self.0.phase
        else {
            return Err(RuntimeError::Invariant(
                "RegExp split species reply in wrong phase",
            ));
        };
        Ok(RegExpSplitStep::make_read(
            regexp.clone(),
            runtime.intern_property_key("flags")?,
            Self(Box::new(RegExpSplitResumeState {
                step_pending: RegExpSplitStepPending::default(),
                realm: self.0.realm,
                phase: Phase::Flags {
                    regexp,
                    input,
                    limit,
                    constructor,
                },
            })),
        ))
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<RegExpSplitStep, RuntimeError> {
        let key = runtime.intern_property_key("lastIndex")?;
        let result = match runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            Some(value) => Completion::Throw(value),
            None => Completion::Return(Value::Undefined),
        };
        self.resume(runtime, result)
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpSplitStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm;
        match self.0.phase {
            Phase::Input { regexp, limit } => {
                let input = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok(RegExpSplitStep::make_species(
                    regexp.clone(),
                    Self(Box::new(RegExpSplitResumeState {
                        step_pending: RegExpSplitStepPending::default(),
                        realm,
                        phase: Phase::Species {
                            regexp,
                            input,
                            limit,
                        },
                    })),
                ))
            }
            Phase::Flags {
                regexp,
                input,
                limit,
                constructor,
            } => Ok(RegExpSplitStep::make_primitive(
                value,
                ToPrimitiveHint::String,
                Self(Box::new(RegExpSplitResumeState {
                    step_pending: RegExpSplitStepPending::default(),
                    realm,
                    phase: Phase::FlagsPrimitive {
                        regexp,
                        input,
                        limit,
                        constructor,
                    },
                })),
            )),
            Phase::FlagsPrimitive {
                regexp,
                input,
                limit,
                constructor,
            } => {
                let flags = match runtime.native_to_js_string(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                let unicode = flags
                    .utf16_units()
                    .any(|unit| unit == u16::from(b'u') || unit == u16::from(b'v'));
                let flags = if flags.utf16_units().any(|unit| unit == u16::from(b'y')) {
                    flags
                } else {
                    flags.try_concat(&JsString::from_static("y"))?
                };
                let mut arguments = Vec::new();
                if arguments.try_reserve_exact(2).is_err() {
                    return Ok(RegExpSplitStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            realm,
                            NativeErrorKind::Internal,
                            "out of memory",
                        )?,
                    )));
                }
                arguments.push(Value::Object(regexp));
                arguments.push(Value::String(flags));
                Ok(RegExpSplitStep::make_construct(
                    constructor,
                    arguments,
                    Self(Box::new(RegExpSplitResumeState {
                        step_pending: RegExpSplitStepPending::default(),
                        realm,
                        phase: Phase::Construct {
                            input,
                            limit,
                            unicode,
                        },
                    })),
                ))
            }
            Phase::Construct {
                input,
                limit,
                unicode,
            } => {
                let Value::Object(splitter) = value else {
                    return Err(RuntimeError::Invariant(
                        "RegExp species constructor returned a primitive",
                    ));
                };
                let state = SplitState {
                    input,
                    splitter,
                    result: runtime.new_array(realm)?,
                    unicode,
                    limit: u32::MAX,
                    length: 0,
                    p: 0,
                    q: 0,
                };
                if matches!(limit, Value::Undefined) {
                    return Self::after_limit(state, runtime, realm);
                }
                Ok(RegExpSplitStep::make_primitive(
                    limit,
                    ToPrimitiveHint::Number,
                    Self(Box::new(RegExpSplitResumeState {
                        step_pending: RegExpSplitStepPending::default(),
                        realm,
                        phase: Phase::Limit(state),
                    })),
                ))
            }
            Phase::Limit(mut state) => {
                let number = match runtime.native_to_number(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                state.limit = Runtime::to_uint32_number(number);
                Self::after_limit(state, runtime, realm)
            }
            Phase::Empty(mut state) => {
                match value {
                    Value::Null => {
                        let input = Value::String(state.input.clone());
                        state.append(runtime, input)?;
                    }
                    Value::Object(_) => {}
                    _ => {
                        return Err(RuntimeError::Invariant(
                            "RegExpExec returned neither an object nor null",
                        ));
                    }
                }
                Ok(state.complete())
            }
            Phase::Set(state) => Ok(state.execute(realm, false)),
            Phase::Exec(mut state) => match value {
                Value::Null => {
                    state.advance()?;
                    state.next(runtime, realm)
                }
                Value::Object(matched) => Ok(RegExpSplitStep::make_read(
                    state.splitter.clone(),
                    runtime.intern_property_key("lastIndex")?,
                    Self(Box::new(RegExpSplitResumeState {
                        step_pending: RegExpSplitStepPending::default(),
                        realm,
                        phase: Phase::End { state, matched },
                    })),
                )),
                _ => Err(RuntimeError::Invariant(
                    "RegExpExec returned neither an object nor null",
                )),
            },
            Phase::End { state, matched } => Ok(RegExpSplitStep::make_primitive(
                value,
                ToPrimitiveHint::Number,
                Self(Box::new(RegExpSplitResumeState {
                    step_pending: RegExpSplitStepPending::default(),
                    realm,
                    phase: Phase::EndPrimitive { state, matched },
                })),
            )),
            Phase::EndPrimitive { mut state, matched } => {
                let end = match runtime.native_to_length(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                let end = usize::try_from(end.min(state.input.len() as u64))
                    .map_err(|_| RuntimeError::Invariant("split end index did not fit usize"))?;
                if end == state.p {
                    state.advance()?;
                    return state.next(runtime, realm);
                }
                let part = Value::String(state.input.sub_string(state.p, state.q));
                state.append(runtime, part)?;
                if state.length == state.limit {
                    return Ok(state.complete());
                }
                state.p = end;
                Ok(RegExpSplitStep::make_read(
                    matched.clone(),
                    runtime.intern_property_key("length")?,
                    Self(Box::new(RegExpSplitResumeState {
                        step_pending: RegExpSplitStepPending::default(),
                        realm,
                        phase: Phase::Count { state, matched },
                    })),
                ))
            }
            Phase::Count { state, matched } => Ok(RegExpSplitStep::make_primitive(
                value,
                ToPrimitiveHint::Number,
                Self(Box::new(RegExpSplitResumeState {
                    step_pending: RegExpSplitStepPending::default(),
                    realm,
                    phase: Phase::CountPrimitive { state, matched },
                })),
            )),
            Phase::CountPrimitive { state, matched } => {
                let count = match runtime.native_to_length(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpSplitStep::Complete(Completion::Throw(value)));
                    }
                };
                state.captures(runtime, realm, matched, 1, count)
            }
            Phase::Capture {
                mut state,
                matched,
                index,
                count,
            } => {
                state.append(runtime, value)?;
                if state.length == state.limit {
                    return Ok(state.complete());
                }
                state.captures(runtime, realm, matched, index + 1, count)
            }
            Phase::Species { .. } => Err(RuntimeError::Invariant(
                "RegExp split completion in species phase",
            )),
        }
    }
    fn after_limit(
        state: SplitState,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<RegExpSplitStep, RuntimeError> {
        if state.limit == 0 {
            return Ok(state.complete());
        }
        if state.input.is_empty() {
            return Ok(state.execute(realm, true));
        }
        // Preserve key allocation before the first observable splitter write.
        runtime.intern_property_key("lastIndex")?;
        runtime.intern_property_key("length")?;
        state.next(runtime, realm)
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: RegExpSplitStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            RegExpSplitStep::Complete(result) => return Ok(result),
            RegExpSplitStep::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                let hint = resume.take_primitive_hint();
                {
                    let result = if matches!(value, Value::Object(_)) {
                        runtime.to_primitive(realm, value, hint)?
                    } else {
                        Completion::Return(value)
                    };
                    resume.resume(runtime, result)?
                }
            }
            RegExpSplitStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            RegExpSplitStep::Species { mut resume } => {
                let regexp = resume.take_species_regexp();
                resume.species(runtime, runtime.regexp_species_constructor(realm, &regexp)?)?
            }
            RegExpSplitStep::Construct { mut resume } => {
                let constructor = resume.take_construct_constructor();
                let arguments = resume.take_construct_arguments();
                resume.resume(
                    runtime,
                    runtime.construct_constructor_internal(
                        realm,
                        &constructor,
                        &constructor,
                        &arguments,
                    )?,
                )?
            }
            RegExpSplitStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                resume.set(
                    runtime,
                    runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?,
                )?
            }
            RegExpSplitStep::Exec { mut resume } => {
                let regexp = resume.take_exec_regexp();
                let input = resume.take_exec_input();
                resume.resume(runtime, runtime.regexp_exec_abstract(realm, regexp, input)?)?
            }
        }
    }
}

#[derive(Default)]
pub(crate) struct RegExpSplitStepPending {
    value: Option<Value>,
    hint: Option<ToPrimitiveHint>,
    object: Option<ObjectRef>,
    key: Option<PropertyKey>,
    regexp: Option<ObjectRef>,
    constructor: Option<ConstructorRef>,
    arguments: Option<Vec<Value>>,
    exec_regexp: Option<Value>,
    input: Option<Value>,
}
impl RegExpSplitStep {
    pub(crate) fn make_primitive(
        value: Value,
        hint: ToPrimitiveHint,
        mut resume: RegExpSplitResume,
    ) -> Self {
        resume.0.step_pending.value = Some(value);
        resume.0.step_pending.hint = Some(hint);
        Self::Primitive { resume }
    }
    pub(crate) fn make_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: RegExpSplitResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn make_species(regexp: ObjectRef, mut resume: RegExpSplitResume) -> Self {
        resume.0.step_pending.regexp = Some(regexp);
        Self::Species { resume }
    }
    pub(crate) fn make_construct(
        constructor: ConstructorRef,
        arguments: Vec<Value>,
        mut resume: RegExpSplitResume,
    ) -> Self {
        resume.0.step_pending.constructor = Some(constructor);
        resume.0.step_pending.arguments = Some(arguments);
        Self::Construct { resume }
    }
    pub(crate) fn make_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: RegExpSplitResume,
    ) -> Self {
        resume.0.step_pending.object = Some(object);
        resume.0.step_pending.key = Some(key);
        resume.0.step_pending.value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn make_exec(regexp: Value, input: Value, mut resume: RegExpSplitResume) -> Self {
        resume.0.step_pending.exec_regexp = Some(regexp);
        resume.0.step_pending.input = Some(input);
        Self::Exec { resume }
    }
}
impl RegExpSplitResume {
    pub(crate) fn take_primitive_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpSplitStep::Primitive lost value")
    }
    pub(crate) fn take_primitive_hint(&mut self) -> ToPrimitiveHint {
        self.0
            .step_pending
            .hint
            .take()
            .expect("RegExpSplitStep::Primitive lost hint")
    }

    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpSplitStep::Read lost object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpSplitStep::Read lost key")
    }

    pub(crate) fn take_species_regexp(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .regexp
            .take()
            .expect("RegExpSplitStep::Species lost regexp")
    }

    pub(crate) fn take_construct_constructor(&mut self) -> ConstructorRef {
        self.0
            .step_pending
            .constructor
            .take()
            .expect("RegExpSplitStep::Construct lost constructor")
    }
    pub(crate) fn take_construct_arguments(&mut self) -> Vec<Value> {
        self.0
            .step_pending
            .arguments
            .take()
            .expect("RegExpSplitStep::Construct lost arguments")
    }

    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .step_pending
            .object
            .take()
            .expect("RegExpSplitStep::Set lost object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .step_pending
            .key
            .take()
            .expect("RegExpSplitStep::Set lost key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .step_pending
            .value
            .take()
            .expect("RegExpSplitStep::Set lost value")
    }

    pub(crate) fn take_exec_regexp(&mut self) -> Value {
        self.0
            .step_pending
            .exec_regexp
            .take()
            .expect("RegExpSplitStep::Exec lost regexp")
    }
    pub(crate) fn take_exec_input(&mut self) -> Value {
        self.0
            .step_pending
            .input
            .take()
            .expect("RegExpSplitStep::Exec lost input")
    }
}

const _: () = assert!(std::mem::size_of::<RegExpSplitStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpSplitStep>() <= 64);
