//! Proposal Set methods retain size-dependent traversal and pinned close policy.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::{
        iterator::step::{NextStep, finish_next},
        native::SetNativeKind,
        object::ObjectIteratorStep,
    },
    heap::{ContextId, ObjectPayload},
    object::{CallableRef, ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
        frames::{ActiveCollectionRecord, ActiveCollectionRecordGuard},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum SetOperation {
    Disjoint,
    Subset,
    Superset,
    Intersection,
    Difference,
    SymmetricDifference,
    Union,
}
impl SetOperation {
    pub(crate) fn from_native(kind: SetNativeKind) -> Option<Self> {
        Some(match kind {
            SetNativeKind::IsDisjointFrom => Self::Disjoint,
            SetNativeKind::IsSubsetOf => Self::Subset,
            SetNativeKind::IsSupersetOf => Self::Superset,
            SetNativeKind::Intersection => Self::Intersection,
            SetNativeKind::Difference => Self::Difference,
            SetNativeKind::SymmetricDifference => Self::SymmetricDifference,
            SetNativeKind::Union => Self::Union,
            _ => return None,
        })
    }
}
pub(crate) enum SetStep {
    Complete(Completion),
    Read { resume: SetResume },
    Number { resume: SetResume },
    Call { resume: SetResume },
    Parse { resume: SetResume },
}
pub(crate) struct SetResume(Box<SetResumeState>);
impl std::ops::Deref for SetResume {
    type Target = SetResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for SetResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<SetResume>() <= 8);
pub(crate) struct SetResumeState {
    pending_effect: SetStepPending,
    phase: Phase,
    realm: ContextId,
    kind: SetOperation,
    set: ObjectRef,
    target: Value,
    size: i64,
    has: Option<CallableRef>,
    keys: Option<CallableRef>,
    result: Option<ObjectRef>,
    index: usize,
    iterator: Value,
    next: Value,
}
enum Phase {
    Size,
    Number,
    Has,
    Keys,
    Iterator,
    NextMethod,
    NextCall,
    Parse,
    Probe {
        record: Option<ActiveCollectionRecordGuard>,
        value: Value,
    },
    CloseMethod,
    CloseCall,
}
impl SetStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: SetOperation,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let set = match runtime.set_receiver(realm, invocation.clone(), false)? {
            NativeConversion::Value(set) => set,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Set method operand argv was not padded",
            ))?;
        if matches!(target, Value::Null | Value::Undefined) {
            let base = if matches!(target, Value::Null) {
                "null"
            } else {
                "undefined"
            };
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    &format!("cannot read property 'size' of {base}"),
                )?,
            )));
        }
        let genuine_size = if let Value::Object(object) = &target {
            if !object.belongs_to(runtime) {
                return Err(RuntimeError::WrongRuntime("set-like object"));
            }
            let state = runtime.0.state.borrow();
            if matches!(
                state.heap.object(object.object_id())?.payload,
                ObjectPayload::Set { .. }
            ) {
                Some(state.heap.set_size(object.object_id())?)
            } else {
                None
            }
        } else {
            None
        };
        let mut resume = SetResume(Box::new(SetResumeState {
            pending_effect: SetStepPending::default(),
            realm,
            kind,
            set,
            target,
            size: 0,
            has: None,
            keys: None,
            result: None,
            index: 0,
            iterator: Value::Undefined,
            next: Value::Undefined,
            phase: Phase::Size,
        }));
        if let Some(size) = genuine_size {
            resume.size = i64::try_from(size).map_err(|_| {
                RuntimeError::Invariant("genuine Set size exceeded signed 64-bit range")
            })?;
            resume.phase = Phase::Has;
            return resume.read(runtime, "has");
        }
        resume.read(runtime, "size")
    }
}
impl SetResume {
    fn read(self, runtime: &Runtime, name: &str) -> Result<SetStep, RuntimeError> {
        Ok(SetStep::request_read(
            self.0.target.clone(),
            runtime.intern_property_key(name)?,
            self,
        ))
    }
    fn result(&self) -> Result<ObjectRef, RuntimeError> {
        self.0
            .result
            .clone()
            .ok_or(RuntimeError::Invariant("Set operation result missing"))
    }
    fn complete(self) -> Result<SetStep, RuntimeError> {
        let value = match self.0.kind {
            SetOperation::Disjoint | SetOperation::Subset | SetOperation::Superset => {
                Value::Bool(true)
            }
            _ => Value::Object(self.result()?),
        };
        Ok(SetStep::Complete(Completion::Return(value)))
    }
    fn selected(mut self, runtime: &Runtime) -> Result<SetStep, RuntimeError> {
        if matches!(self.0.kind, SetOperation::Difference) {
            self.0.result = Some(runtime.copy_set_in_realm(self.0.realm, &self.0.set)?);
        }
        let size = i64::try_from(runtime.set_size_value(&self.0.set)?).unwrap_or(i64::MAX);
        if matches!(self.0.kind, SetOperation::Subset) && size > self.0.size
            || matches!(self.0.kind, SetOperation::Superset) && size < self.0.size
        {
            return Ok(SetStep::Complete(Completion::Return(Value::Bool(false))));
        }
        let own = match self.0.kind {
            SetOperation::Subset => true,
            SetOperation::Disjoint | SetOperation::Intersection | SetOperation::Difference => {
                size <= self.0.size
            }
            _ => false,
        };
        if own {
            if matches!(self.0.kind, SetOperation::Intersection) {
                self.0.result = Some(runtime.new_set_in_realm(self.0.realm)?);
            }
            return self.probe(runtime);
        }
        if matches!(
            self.0.kind,
            SetOperation::SymmetricDifference | SetOperation::Union
        ) {
            self.0.has = None;
        }
        self.0.phase = Phase::Iterator;
        Ok(SetStep::request_call(
            self.0
                .keys
                .clone()
                .ok_or(RuntimeError::Invariant("Set operation keys missing"))?,
            self.0.target.clone(),
            Vec::new(),
            self,
        ))
    }
    fn probe(mut self, runtime: &Runtime) -> Result<SetStep, RuntimeError> {
        let source = if matches!(self.0.kind, SetOperation::Difference) {
            self.result()?
        } else {
            self.0.set.clone()
        };
        let Some((record_index, value)) =
            runtime.next_live_set_record(&source, &mut self.0.index)?
        else {
            return self.complete();
        };
        let record = runtime.push_active_collection_record(ActiveCollectionRecord::Set {
            object: source.object_id(),
            index: record_index,
        });
        let arguments = vec![value.clone()];
        self.0.phase = Phase::Probe {
            record: Some(record),
            value,
        };
        Ok(SetStep::request_call(
            self.0
                .has
                .clone()
                .ok_or(RuntimeError::Invariant("Set operation has missing"))?,
            self.0.target.clone(),
            arguments,
            self,
        ))
    }
    fn next_step(mut self, runtime: &Runtime) -> Result<SetStep, RuntimeError> {
        let callable = match &self.0.next {
            Value::Object(object) => runtime.as_callable(object)?,
            _ => None,
        };
        let Some(callable) = callable else {
            return Ok(SetStep::Complete(Completion::Throw(
                runtime.new_native_error(self.0.realm, NativeErrorKind::Type, "not a function")?,
            )));
        };
        self.0.phase = Phase::NextCall;
        Ok(SetStep::request_call(
            callable,
            self.0.iterator.clone(),
            Vec::new(),
            self,
        ))
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<SetStep, RuntimeError> {
        if matches!(self.0.phase, Phase::NextCall) {
            self.0.phase = Phase::Parse;
            return Ok(SetStep::request_parse(reply, self));
        }
        if let Phase::Probe { record, .. } = &mut self.0.phase
            && let Some(record) = record.take()
        {
            record.finish()?;
        }
        if matches!(self.0.phase, Phase::CloseCall)
            || matches!(self.0.phase, Phase::CloseMethod) && matches!(reply, Completion::Throw(_))
        {
            return Ok(SetStep::Complete(Completion::Return(Value::Bool(false))));
        }
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(SetStep::Complete(Completion::Throw(value))),
        };
        match std::mem::replace(&mut self.0.phase, Phase::Parse) {
            Phase::Size => {
                self.0.phase = Phase::Number;
                Ok(SetStep::request_number(value, self))
            }
            phase @ (Phase::Has | Phase::Keys) => {
                let has = matches!(phase, Phase::Has);
                let name = if has { "has" } else { "keys" };
                if matches!(value, Value::Undefined) {
                    return Ok(SetStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            &format!(".{name} is undefined"),
                        )?,
                    )));
                }
                let callable = match value {
                    Value::Object(ref object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(SetStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            &format!(".{name} is not a function"),
                        )?,
                    )));
                };
                if has {
                    self.0.has = Some(callable);
                    self.0.phase = Phase::Keys;
                    self.read(runtime, "keys")
                } else {
                    self.0.keys = Some(callable);
                    self.selected(runtime)
                }
            }
            Phase::Iterator => {
                if matches!(value, Value::Null | Value::Undefined) {
                    let base = if matches!(value, Value::Null) {
                        "null"
                    } else {
                        "undefined"
                    };
                    return Ok(SetStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            &format!("cannot read property 'next' of {base}"),
                        )?,
                    )));
                }
                self.0.iterator = value.clone();
                self.0.phase = Phase::NextMethod;
                Ok(SetStep::request_read(
                    value,
                    runtime.intern_property_key("next")?,
                    self,
                ))
            }
            Phase::NextMethod => {
                self.0.next = value;
                match self.0.kind {
                    SetOperation::Intersection => {
                        self.0.result = Some(runtime.new_set_in_realm(self.0.realm)?)
                    }
                    SetOperation::SymmetricDifference | SetOperation::Union => {
                        self.0.result = Some(runtime.copy_set_in_realm(self.0.realm, &self.0.set)?)
                    }
                    _ => {}
                }
                self.next_step(runtime)
            }
            Phase::Probe { value: item, .. } => {
                let present = runtime.value_to_boolean(&value)?;
                match self.0.kind {
                    SetOperation::Disjoint if present => {
                        return Ok(SetStep::Complete(Completion::Return(Value::Bool(false))));
                    }
                    SetOperation::Subset if !present => {
                        return Ok(SetStep::Complete(Completion::Return(Value::Bool(false))));
                    }
                    SetOperation::Intersection if present => {
                        runtime.insert_set_record(&self.result()?, item)?;
                    }
                    SetOperation::Difference if present => {
                        runtime.delete_set_record(&self.result()?, &item)?;
                    }
                    _ => {}
                }
                self.probe(runtime)
            }
            Phase::CloseMethod => {
                let callable = match value {
                    Value::Object(ref object) => runtime.as_callable(object)?,
                    _ => None,
                };
                let Some(callable) = callable else {
                    return Ok(SetStep::Complete(Completion::Return(Value::Bool(false))));
                };
                self.0.phase = Phase::CloseCall;
                Ok(SetStep::request_call(
                    callable,
                    self.0.iterator.clone(),
                    Vec::new(),
                    self,
                ))
            }
            _ => Err(RuntimeError::Invariant(
                "Set operation completion phase mismatch",
            )),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        reply: NativeConversion<f64>,
    ) -> Result<SetStep, RuntimeError> {
        let size = match reply {
            NativeConversion::Value(size) => size,
            NativeConversion::Throw(value) => {
                return Ok(SetStep::Complete(Completion::Throw(value)));
            }
        };
        if size.is_nan() {
            return Ok(SetStep::Complete(Completion::Throw(
                runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Type,
                    ".size is not a number",
                )?,
            )));
        }
        let size = if size < i64::MIN as f64 {
            i64::MIN
        } else if size >= 2f64.powi(63) {
            i64::MAX
        } else {
            size as i64
        };
        if size < 0 {
            return Ok(SetStep::Complete(Completion::Throw(
                runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Range,
                    ".size must be positive",
                )?,
            )));
        }
        self.0.size = size;
        self.0.phase = Phase::Has;
        self.read(runtime, "has")
    }
    pub(crate) fn parsed(
        mut self,
        runtime: &Runtime,
        reply: ObjectIteratorStep,
    ) -> Result<SetStep, RuntimeError> {
        let value = match reply {
            ObjectIteratorStep::Throw(value) => {
                return Ok(SetStep::Complete(Completion::Throw(value)));
            }
            ObjectIteratorStep::Done => return self.complete(),
            ObjectIteratorStep::Yield(value) => value,
        };
        let value = Runtime::normalized_set_key(value);
        match self.0.kind {
            SetOperation::Disjoint | SetOperation::Superset => {
                let present = runtime.find_set_record(&self.0.set, &value)?.is_some();
                drop(value);
                if present == matches!(self.0.kind, SetOperation::Disjoint) {
                    self.0.phase = Phase::CloseMethod;
                    return Ok(SetStep::request_read(
                        self.0.iterator.clone(),
                        runtime.intern_property_key("return")?,
                        self,
                    ));
                }
            }
            SetOperation::Intersection => {
                if runtime.find_set_record(&self.0.set, &value)?.is_some() {
                    runtime.insert_set_record(&self.result()?, value)?;
                }
            }
            SetOperation::Difference => {
                runtime.delete_set_record(&self.result()?, &value)?;
            }
            SetOperation::SymmetricDifference => {
                if runtime.find_set_record(&self.0.set, &value)?.is_some() {
                    runtime.delete_set_record(&self.result()?, &value)?;
                } else {
                    runtime.insert_set_record(&self.result()?, value)?;
                }
            }
            SetOperation::Union => {
                runtime.insert_set_record(&self.result()?, value)?;
            }
            SetOperation::Subset => {
                return Err(RuntimeError::Invariant("Set subset used foreign iterator"));
            }
        }
        self.next_step(runtime)
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: SetStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            SetStep::Complete(result) => return Ok(result),
            SetStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_value_property_in_realm(realm, receiver, &key)?,
                )?
            }
            SetStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            SetStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &arguments)?,
                )?
            }
            SetStep::Parse { mut resume } => {
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
        };
    }
}

#[derive(Default)]
struct SetStepPending {
    read_receiver: Option<Value>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
    parse_result: Option<Completion>,
}
impl SetStep {
    pub(crate) fn request_read(receiver: Value, key: PropertyKey, mut resume: SetResume) -> Self {
        resume.0.pending_effect.read_receiver = Some(receiver);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: SetResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: SetResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_parse(result: Completion, mut resume: SetResume) -> Self {
        resume.0.pending_effect.parse_result = Some(result);
        Self::Parse { resume }
    }
}
impl SetResume {
    pub(crate) fn take_read_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .read_receiver
            .take()
            .expect("SetStep Read receiver")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("SetStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("SetStep Number value")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("SetStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("SetStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("SetStep Call arguments")
    }
    pub(crate) fn take_parse_result(&mut self) -> Completion {
        self.0
            .pending_effect
            .parse_result
            .take()
            .expect("SetStep Parse result")
    }
}
const _: () = assert!(std::mem::size_of::<SetStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<SetStep>() <= 64);
