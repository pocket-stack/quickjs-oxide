//! Array.from/of own constructor, iterator and definition replies in source order.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        iterator::step::{CloseStep, NextStep, finish_close, finish_next},
        object::ObjectIteratorStep,
    },
    heap::ContextId,
    object::{
        CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        WellKnownSymbol,
        operations::{InternalDefineResult, InternalSetResult},
    },
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{ConstructorRef, NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum BuildKind {
    From,
    Of,
}
pub(crate) enum BuildStep {
    Complete(Completion),
    Read {
        resume: BuildResume,
    },
    Number {
        resume: BuildResume,
    },
    Call {
        resume: BuildResume,
    },
    Construct {
        resume: BuildResume,
    },
    Parse {
        resume: BuildResume,
    },
    Define {
        resume: BuildResume,
    },
    Set {
        resume: BuildResume,
    },
    Close {
        iterator: ObjectRef,
        completion: Completion,
    },
}
pub(crate) struct BuildResume(Box<BuildResumeState>);
impl std::ops::Deref for BuildResume {
    type Target = BuildResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for BuildResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<BuildResume>() <= 8);
pub(crate) struct BuildResumeState {
    pending_effect: BuildStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    constructor: Value,
    result: Option<ObjectRef>,
    mapfn: Option<CallableRef>,
    map_this: Value,
    mode: Mode,
    index: u64,
    phase: Phase,
}
enum Mode {
    Acquire(Value),
    Iterable {
        items: Value,
        method: CallableRef,
        iterator: Option<ObjectRef>,
        next: Option<CallableRef>,
    },
    ArrayLike {
        source: ObjectRef,
        length: u64,
    },
    Of {
        values: std::vec::IntoIter<Value>,
        length: u32,
    },
}
enum Phase {
    Method,
    Length,
    Number,
    Construct,
    Iterator,
    NextMethod,
    NextCall,
    Parse,
    Value,
    Map,
    Define,
    LengthSet,
}
impl BuildStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: BuildKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array builder did not receive a generic invocation",
            ));
        };
        if matches!(kind, BuildKind::Of) {
            let length = u32::try_from(arguments.actual_arg_count)
                .map_err(|_| RuntimeError::Invariant("Array.of argument count exceeded Uint32"))?;
            let resume = BuildResume(Box::new(BuildResumeState {
                pending_effect: BuildStepPending::default(),
                scheduler_set_key: None,
                realm,
                constructor: this_value.clone(),
                result: None,
                mapfn: None,
                map_this: Value::Undefined,
                mode: Mode::Of {
                    values: arguments.readable[..arguments.actual_arg_count]
                        .to_vec()
                        .into_iter(),
                    length,
                },
                index: 0,
                phase: Phase::Construct,
            }));
            return resume.construct(runtime, Some(Runtime::array_length_value(length)), false);
        }
        let items = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("Array.from argv was not padded"))?;
        let mapfn = if arguments.actual_arg_count > 1
            && !matches!(arguments.readable[1], Value::Undefined)
        {
            let callable = match &arguments.readable[1] {
                Value::Object(object) => runtime.as_callable(object)?,
                _ => None,
            };
            let Some(callable) = callable else {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error(realm, NativeErrorKind::Type, "not a function")?,
                )));
            };
            Some(callable)
        } else {
            None
        };
        let map_this = if arguments.actual_arg_count > 2 {
            arguments.readable[2].clone()
        } else {
            Value::Undefined
        };
        if matches!(items, Value::Null | Value::Undefined) {
            let base = if matches!(items, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'Symbol.iterator' of {base}"),
                )?,
            )));
        }
        Ok(Self::request_read(
            items.clone(),
            PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator)),
            BuildResume(Box::new(BuildResumeState {
                pending_effect: BuildStepPending::default(),
                scheduler_set_key: None,
                realm,
                constructor: this_value.clone(),
                result: None,
                mapfn,
                map_this,
                mode: Mode::Acquire(items),
                index: 0,
                phase: Phase::Method,
            })),
        ))
    }
}
impl BuildResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    fn abrupt(self, value: Value) -> BuildStep {
        if matches!(self.0.phase, Phase::Map | Phase::Define)
            && let Mode::Iterable {
                iterator: Some(iterator),
                ..
            } = self.0.mode
        {
            return BuildStep::Close {
                iterator,
                completion: Completion::Throw(value),
            };
        }
        BuildStep::Complete(Completion::Throw(value))
    }
    fn result(&self) -> Result<ObjectRef, RuntimeError> {
        self.0
            .result
            .clone()
            .ok_or(RuntimeError::Invariant("Array builder result missing"))
    }
    fn construct(
        mut self,
        runtime: &Runtime,
        length: Option<Value>,
        initialize_length: bool,
    ) -> Result<BuildStep, RuntimeError> {
        self.0.phase = Phase::Construct;
        let constructor = std::mem::replace(&mut self.0.constructor, Value::Undefined);
        if let Value::Object(object) = &constructor
            && runtime.is_constructor(object)?
        {
            let target = match runtime.constructor_from_value(self.0.realm, constructor)? {
                NativeConversion::Value(target) => target,
                NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
            };
            return Ok(BuildStep::request_construct(
                target,
                length.into_iter().collect(),
                self,
            ));
        }
        // Numeric length and a new unpublished Array cannot invoke JS. Share
        // the same fallback allocation as ArraySpeciesCreate.
        let result = runtime
            .new_array_with_length(self.0.realm, if initialize_length { length } else { None })?;
        self.resume(runtime, result)
    }
    fn next(mut self, runtime: &Runtime) -> Result<BuildStep, RuntimeError> {
        match &mut self.0.mode {
            Mode::Iterable {
                iterator: Some(iterator),
                next: Some(next),
                ..
            } => {
                let callable = next.clone();
                let receiver = Value::Object(iterator.clone());
                self.0.phase = Phase::NextCall;
                // Array.from uses ordinary Call, not JS_IteratorNext2's raw
                // cproto fast path; parse its actual object result afterwards.
                Ok(BuildStep::request_call(
                    callable,
                    receiver,
                    Vec::new(),
                    self,
                ))
            }
            Mode::ArrayLike { source, length } if self.0.index < *length => {
                let receiver = Value::Object(source.clone());
                self.0.phase = Phase::Value;
                Ok(BuildStep::request_read(
                    receiver,
                    runtime.property_key_for_index(self.0.index)?,
                    self,
                ))
            }
            Mode::Of { values, .. } => {
                if let Some(value) = values.next() {
                    self.define(runtime, value)
                } else {
                    self.set_length(runtime)
                }
            }
            Mode::ArrayLike { .. } => self.set_length(runtime),
            _ => Err(RuntimeError::Invariant(
                "Array builder iteration not initialized",
            )),
        }
    }
    fn set_length(mut self, runtime: &Runtime) -> Result<BuildStep, RuntimeError> {
        let length = match self.0.mode {
            Mode::Of { length, .. } => length as u64,
            Mode::ArrayLike { length, .. } => length,
            _ => self.0.index,
        };
        self.0.phase = Phase::LengthSet;
        Ok(BuildStep::request_set(
            self.result()?,
            runtime.intern_property_key("length")?,
            Value::number(length as f64),
            self,
        ))
    }
    fn map(mut self, runtime: &Runtime, value: Value) -> Result<BuildStep, RuntimeError> {
        if let Some(callable) = self.0.mapfn.clone() {
            self.0.phase = Phase::Map;
            return Ok(BuildStep::request_call(
                callable,
                self.0.map_this.clone(),
                vec![value, Value::number(self.0.index as f64)],
                self,
            ));
        }
        self.define(runtime, value)
    }
    fn define(mut self, runtime: &Runtime, value: Value) -> Result<BuildStep, RuntimeError> {
        self.0.phase = Phase::Define;
        Ok(BuildStep::request_define(
            self.result()?,
            runtime.property_key_for_index(self.0.index)?,
            OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
            self,
        ))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<BuildStep, RuntimeError> {
        if matches!(self.0.phase, Phase::NextCall) {
            self.0.phase = Phase::Parse;
            return Ok(BuildStep::request_parse(reply, self));
        }
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(self.abrupt(value)),
        };
        match self.0.phase {
            Phase::Method => {
                let Mode::Acquire(items) =
                    std::mem::replace(&mut self.0.mode, Mode::Acquire(Value::Undefined))
                else {
                    return Err(RuntimeError::Invariant("Array.from items missing"));
                };
                if matches!(value, Value::Undefined | Value::Null) {
                    let source = match runtime.native_to_object(self.0.realm, items)? {
                        NativeConversion::Value(source) => source,
                        NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
                    };
                    self.0.mode = Mode::ArrayLike {
                        source: source.clone(),
                        length: 0,
                    };
                    self.0.phase = Phase::Length;
                    return Ok(BuildStep::request_read(
                        Value::Object(source),
                        runtime.intern_property_key("length")?,
                        self,
                    ));
                }
                let callable = match value {
                    Value::Object(ref object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(method) = callable else {
                    let error = runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "value is not iterable",
                    )?;
                    return Ok(self.abrupt(error));
                };
                self.0.mode = Mode::Iterable {
                    items,
                    method,
                    iterator: None,
                    next: None,
                };
                self.construct(runtime, None, false)
            }
            Phase::Length => {
                self.0.phase = Phase::Number;
                Ok(BuildStep::request_number(value, self))
            }
            Phase::Construct => {
                let Value::Object(result) = value else {
                    return Err(RuntimeError::Invariant(
                        "Array result constructor returned a primitive",
                    ));
                };
                self.0.result = Some(result);
                if let Mode::Iterable { items, method, .. } = &self.0.mode {
                    let receiver = items.clone();
                    let callable = method.clone();
                    self.0.phase = Phase::Iterator;
                    return Ok(BuildStep::request_call(
                        callable,
                        receiver,
                        Vec::new(),
                        self,
                    ));
                }
                self.next(runtime)
            }
            Phase::Iterator => {
                let Value::Object(iterator) = value else {
                    let error = runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?;
                    return Ok(self.abrupt(error));
                };
                let Mode::Iterable {
                    iterator: target, ..
                } = &mut self.0.mode
                else {
                    return Err(RuntimeError::Invariant("Array.from iterator mode missing"));
                };
                *target = Some(iterator.clone());
                self.0.phase = Phase::NextMethod;
                Ok(BuildStep::request_read(
                    Value::Object(iterator),
                    runtime.intern_property_key("next")?,
                    self,
                ))
            }
            Phase::NextMethod => {
                let callable = match value {
                    Value::Object(ref object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    let error = runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "not a function",
                    )?;
                    return Ok(self.abrupt(error));
                };
                let Mode::Iterable { next, .. } = &mut self.0.mode else {
                    return Err(RuntimeError::Invariant("Array.from iterator mode missing"));
                };
                *next = Some(callable);
                self.next(runtime)
            }
            Phase::Value => self.map(runtime, value),
            Phase::Map => self.define(runtime, value),
            _ => Err(RuntimeError::Invariant(
                "Array builder completion phase mismatch",
            )),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<f64>,
    ) -> Result<BuildStep, RuntimeError> {
        let number = match reply {
            NativeConversion::Value(number) => number,
            NativeConversion::Throw(value) => return Ok(self.abrupt(value)),
        };
        if !matches!(self.0.phase, Phase::Number) {
            return Err(RuntimeError::Invariant(
                "Array builder number phase mismatch",
            ));
        }
        let Mode::ArrayLike { length, .. } = &mut self.0.mode else {
            return Err(RuntimeError::Invariant(
                "Array builder length mode mismatch",
            ));
        };
        *length = Runtime::length_from_number(number);
        let value = Value::number(*length as f64);
        self.construct(runtime, Some(value), true)
    }
    pub(crate) fn parsed(
        self,
        runtime: &Runtime,
        reply: ObjectIteratorStep,
    ) -> Result<BuildStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Parse) {
            return Err(RuntimeError::Invariant(
                "Array builder next result phase mismatch",
            ));
        }
        match reply {
            ObjectIteratorStep::Throw(value) => Ok(self.abrupt(value)),
            ObjectIteratorStep::Done => self.set_length(runtime),
            ObjectIteratorStep::Yield(value) => self.map(runtime, value),
        }
    }
    pub(crate) fn defined(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<InternalDefineResult>,
    ) -> Result<BuildStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Define) {
            return Err(RuntimeError::Invariant(
                "Array builder definition phase mismatch",
            ));
        }
        if let Some(value) =
            runtime.finish_create_indexed_data_property(self.0.realm, self.0.index, reply)?
        {
            return Ok(self.abrupt(value));
        }
        self.0.index = self.0.index.checked_add(1).ok_or(RuntimeError::Invariant(
            "Array.from iterator index overflowed u64",
        ))?;
        self.next(runtime)
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        key: PropertyKey,
        reply: NativeConversion<InternalSetResult>,
    ) -> Result<BuildStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::LengthSet) {
            return Err(RuntimeError::Invariant(
                "Array builder length-set phase mismatch",
            ));
        }
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, reply)? {
            return Ok(self.abrupt(value));
        }
        Ok(BuildStep::Complete(Completion::Return(Value::Object(
            self.result()?,
        ))))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: BuildStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            BuildStep::Complete(result) => return Ok(result),
            BuildStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            BuildStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            BuildStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
            BuildStep::Construct { mut resume } => {
                let target = resume.take_construct_target();
                let arguments = resume.take_construct_arguments();
                resume.resume(
                    runtime,
                    runtime.construct_constructor_internal(realm, &target, &target, &arguments)?,
                )?
            }
            BuildStep::Parse { mut resume } => {
                let result = resume.take_parse_result();
                resume.parsed(
                    runtime,
                    finish_next(
                        runtime,
                        realm,
                        NextStep::parse_result(runtime, realm, result)?,
                    )?,
                )?
            }
            BuildStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
            BuildStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                {
                    let reply = runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?;
                    resume.set(runtime, key, reply)?
                }
            }
            BuildStep::Close {
                iterator,
                completion,
            } => {
                return finish_close(
                    runtime,
                    realm,
                    CloseStep::start(runtime, realm, iterator, completion)?,
                );
            }
        };
    }
}

#[derive(Default)]
struct BuildStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    construct_target: Option<ConstructorRef>,
    construct_arguments: Option<Vec<Value>>,
    parse_result: Option<Completion>,
    define_object: Option<ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
}
impl BuildStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: BuildResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: BuildResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: BuildResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_construct(
        target: ConstructorRef,
        arguments: Vec<Value>,
        mut resume: BuildResume,
    ) -> Self {
        resume.0.pending_effect.construct_target = Some(target);
        resume.0.pending_effect.construct_arguments = Some(arguments);
        Self::Construct { resume }
    }
    pub(crate) fn request_parse(result: Completion, mut resume: BuildResume) -> Self {
        resume.0.pending_effect.parse_result = Some(result);
        Self::Parse { resume }
    }
    pub(crate) fn request_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: BuildResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: BuildResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
}
impl BuildResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("BuildStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("BuildStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("BuildStep Number value")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("BuildStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("BuildStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("BuildStep Call arguments")
    }
    pub(crate) fn take_construct_target(&mut self) -> ConstructorRef {
        self.0
            .pending_effect
            .construct_target
            .take()
            .expect("BuildStep Construct target")
    }
    pub(crate) fn take_construct_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .construct_arguments
            .take()
            .expect("BuildStep Construct arguments")
    }
    pub(crate) fn take_parse_result(&mut self) -> Completion {
        self.0
            .pending_effect
            .parse_result
            .take()
            .expect("BuildStep Parse result")
    }
    pub(crate) fn take_define_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("BuildStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("BuildStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("BuildStep Define descriptor")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("BuildStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("BuildStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("BuildStep Set value")
    }
}
const _: () = assert!(std::mem::size_of::<BuildStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<BuildStep>() <= 64);
