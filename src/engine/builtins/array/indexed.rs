//! Indexed Array algorithms expose property and numeric conversion boundaries.

use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArraySearchKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum IndexedKind {
    At,
    With,
    Fill,
    CopyWithin,
    Search(ArraySearchKind),
    ToReversed,
}
impl IndexedKind {
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::ArrayPrototypeAt => Self::At,
            NativeFunctionId::ArrayPrototypeWith => Self::With,
            NativeFunctionId::ArrayPrototypeFill => Self::Fill,
            NativeFunctionId::ArrayPrototypeCopyWithin => Self::CopyWithin,
            NativeFunctionId::ArrayPrototypeSearch(kind) => Self::Search(kind),
            NativeFunctionId::ArrayPrototypeToReversed => Self::ToReversed,
            _ => return None,
        })
    }
}
pub(crate) enum IndexedStep {
    Complete(Completion),
    Copy { resume: IndexedResume },
    Read { resume: IndexedResume },
    Number { resume: IndexedResume },
    Has { resume: IndexedResume },
    Set { resume: IndexedResume },
}
enum Phase {
    Length,
    LengthNumber,
    Bound(usize),
    Has,
    Read,
    Write,
    Copy,
}
pub(crate) struct IndexedResume(Box<IndexedResumeState>);
impl std::ops::Deref for IndexedResume {
    type Target = IndexedResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for IndexedResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<IndexedResume>() <= 8);
pub(crate) struct IndexedResumeState {
    pending_effect: IndexedStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    kind: IndexedKind,
    object: ObjectRef,
    arguments: Vec<Value>,
    actual: usize,
    phase: Phase,
    length: i64,
    bounds: [i64; 3],
    index: i64,
    end: i64,
    direction: i64,
    values: Vec<Value>,
    replacement: i64,
}
impl IndexedStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: IndexedKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array indexed method requires generic invocation",
            ));
        };
        let object = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        Ok(Self::request_read(
            object.clone(),
            runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
            IndexedResume(Box::new(IndexedResumeState {
                pending_effect: IndexedStepPending::default(),
                scheduler_set_key: None,
                realm,
                kind,
                object,
                arguments: arguments.readable.clone(),
                actual: arguments.actual_arg_count,
                phase: Phase::Length,
                length: 0,
                bounds: [0; 3],
                index: 0,
                end: 0,
                direction: 1,
                values: Vec::new(),
                replacement: -1,
            })),
        ))
    }
}
impl IndexedResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    fn argument(&self, index: usize) -> Value {
        self.0
            .arguments
            .get(index)
            .cloned()
            .unwrap_or(Value::Undefined)
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<IndexedStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(IndexedStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::LengthNumber;
                Ok(IndexedStep::request_number(value, self))
            }
            Phase::Read => match self.0.kind {
                IndexedKind::At => Ok(IndexedStep::Complete(Completion::Return(value))),
                IndexedKind::With | IndexedKind::ToReversed => {
                    let output = if matches!(self.0.kind, IndexedKind::ToReversed) {
                        self.0.length - self.0.index - 1
                    } else {
                        self.0.index
                    };
                    self.0.values[output as usize] = value;
                    self.advance(runtime)
                }
                IndexedKind::Search(kind) => {
                    let search = self.argument(0);
                    let found = if kind == ArraySearchKind::Includes {
                        search.same_value_zero(&value)
                    } else {
                        search.strict_equal(&value)
                    };
                    if found {
                        Ok(IndexedStep::Complete(Completion::Return(
                            if kind == ArraySearchKind::Includes {
                                Value::Bool(true)
                            } else {
                                Value::number(self.0.index as f64)
                            },
                        )))
                    } else {
                        self.advance(runtime)
                    }
                }
                _ => Err(RuntimeError::Invariant("Array indexed read kind mismatch")),
            },
            Phase::Copy => self.complete(runtime),
            _ => Err(RuntimeError::Invariant(
                "Array indexed value phase mismatch",
            )),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<IndexedStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(IndexedStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::LengthNumber => {
                self.0.length = Runtime::length_from_number(number) as i64;
                self.0.bounds[2] = self.0.length;
                if matches!(self.0.kind, IndexedKind::Search(_)) && self.0.length == 0 {
                    return self.complete(runtime);
                }
                self.bound(runtime, 0)
            }
            Phase::Bound(index) => {
                let mut value = Runtime::int64_from_number(number);
                if !matches!(self.0.kind, IndexedKind::At | IndexedKind::With) {
                    if value < 0 {
                        value += self.0.length;
                    }
                    value = if matches!(
                        self.0.kind,
                        IndexedKind::Search(ArraySearchKind::LastIndexOf)
                    ) {
                        value.clamp(-1, self.0.length - 1)
                    } else {
                        value.clamp(0, self.0.length)
                    };
                }
                self.0.bounds[index] = value;
                self.bound(runtime, index + 1)
            }
            _ => Err(RuntimeError::Invariant(
                "Array indexed number phase mismatch",
            )),
        }
    }
    fn bound(mut self, runtime: &Runtime, index: usize) -> Result<IndexedStep, RuntimeError> {
        let argument = match self.0.kind {
            IndexedKind::At | IndexedKind::With if index == 0 => Some(0),
            IndexedKind::Fill if index < 2 => Some(index + 1),
            IndexedKind::CopyWithin if index < 3 => Some(index),
            IndexedKind::Search(_) if index == 0 => Some(1),
            _ => None,
        };
        if let Some(argument) = argument {
            let value = self.argument(argument);
            let omitted = match self.0.kind {
                IndexedKind::Fill => argument >= self.0.actual || matches!(value, Value::Undefined),
                IndexedKind::CopyWithin if argument == 2 => {
                    argument >= self.0.actual || matches!(value, Value::Undefined)
                }
                IndexedKind::Search(_) => argument >= self.0.actual,
                _ => false,
            };
            if omitted {
                self.0.bounds[index] = match self.0.kind {
                    IndexedKind::Fill if index == 1 => self.0.length,
                    IndexedKind::CopyWithin => self.0.length,
                    IndexedKind::Search(ArraySearchKind::LastIndexOf) => self.0.length - 1,
                    _ => 0,
                };
                return self.bound(runtime, index + 1);
            }
            self.0.phase = Phase::Bound(index);
            return Ok(IndexedStep::request_number(value, self));
        }
        self.0.index = self.0.bounds[0];
        self.0.end = self.0.length;
        match self.0.kind {
            IndexedKind::At | IndexedKind::With => {
                if self.0.index < 0 {
                    self.0.index += self.0.length;
                }
                if self.0.index < 0 || self.0.index >= self.0.length {
                    return Ok(IndexedStep::Complete(
                        if matches!(self.0.kind, IndexedKind::At) {
                            Completion::Return(Value::Undefined)
                        } else {
                            Completion::Throw(runtime.new_native_error_jsvalue(
                                self.0.realm,
                                NativeErrorKind::Range,
                                &format!("invalid array index: {}", self.0.index),
                            )?)
                        },
                    ));
                }
                if matches!(self.0.kind, IndexedKind::With) {
                    self.0.replacement = self.0.index;
                    self.0.index = 0;
                    if let Some(value) = self.allocate(runtime)? {
                        return Ok(IndexedStep::Complete(Completion::Throw(value)));
                    }
                }
            }
            IndexedKind::Fill => self.0.end = self.0.bounds[1].max(self.0.index),
            IndexedKind::CopyWithin => {
                let to = self.0.bounds[0];
                let from = self.0.bounds[1];
                let count = (self.0.bounds[2] - from).min(self.0.length - to).max(0);
                self.0.phase = Phase::Copy;
                return Ok(IndexedStep::request_copy(
                    self.0.object.clone(),
                    to as u64,
                    from as u64,
                    count as u64,
                    from < to && to < from + count,
                    self,
                ));
            }
            IndexedKind::Search(ArraySearchKind::LastIndexOf) => {
                self.0.end = -1;
                self.0.direction = -1;
            }
            IndexedKind::ToReversed => {
                if let Some(value) = self.allocate(runtime)? {
                    return Ok(IndexedStep::Complete(Completion::Throw(value)));
                }
                self.0.index = self.0.length - 1;
                self.0.end = -1;
                self.0.direction = -1;
            }
            _ => {}
        }
        self.next(runtime)
    }
    fn allocate(&mut self, runtime: &Runtime) -> Result<Option<Value>, RuntimeError> {
        match runtime.native_allocate_fast_array_values(self.0.realm, self.0.length as u64)? {
            NativeConversion::Value(values) => {
                self.0.values = values;
                Ok(None)
            }
            NativeConversion::Throw(value) => Ok(Some(value)),
        }
    }
    fn advance(mut self, runtime: &Runtime) -> Result<IndexedStep, RuntimeError> {
        self.0.index += self.0.direction;
        self.next(runtime)
    }
    fn next(mut self, runtime: &Runtime) -> Result<IndexedStep, RuntimeError> {
        loop {
            if self.0.index == self.0.end {
                return self.complete(runtime);
            }
            if matches!(self.0.kind, IndexedKind::With) && self.0.index == self.0.replacement {
                self.0.values[self.0.index as usize] = self.argument(1);
                self.0.index += 1;
                continue;
            }
            let key = runtime.property_key_for_index(self.0.index as u64)?;
            if matches!(self.0.kind, IndexedKind::Fill) {
                self.0.phase = Phase::Write;
                return Ok(IndexedStep::request_set(
                    self.0.object.clone(),
                    key,
                    self.argument(0),
                    self,
                ));
            }
            if matches!(self.0.kind, IndexedKind::Search(ArraySearchKind::Includes)) {
                self.0.phase = Phase::Read;
                return Ok(IndexedStep::request_read(self.0.object.clone(), key, self));
            }
            self.0.phase = Phase::Has;
            return Ok(IndexedStep::request_has(self.0.object.clone(), key, self));
        }
    }
    fn complete(self, runtime: &Runtime) -> Result<IndexedStep, RuntimeError> {
        let value = match self.0.kind {
            IndexedKind::With | IndexedKind::ToReversed => {
                Value::Object(runtime.new_array_from_values(self.0.realm, self.0.values)?)
            }
            IndexedKind::Search(ArraySearchKind::Includes) => Value::Bool(false),
            IndexedKind::Search(_) => Value::Int(-1),
            IndexedKind::At => Value::Undefined,
            _ => Value::Object(self.0.object),
        };
        Ok(IndexedStep::Complete(Completion::Return(value)))
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<IndexedStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(IndexedStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Has => {
                if value {
                    self.0.phase = Phase::Read;
                    return Ok(IndexedStep::request_read(
                        self.0.object.clone(),
                        runtime.property_key_for_index(self.0.index as u64)?,
                        self,
                    ));
                }
                if matches!(self.0.kind, IndexedKind::At) {
                    return self.complete(runtime);
                }
                self.advance(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "Array indexed boolean phase mismatch",
            )),
        }
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<IndexedStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Write) {
            return Err(RuntimeError::Invariant("Array indexed set phase mismatch"));
        }
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(IndexedStep::Complete(Completion::Throw(value)));
        }
        self.advance(runtime)
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: IndexedStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            IndexedStep::Complete(result) => return Ok(result),
            IndexedStep::Copy { mut resume } => {
                let object = resume.take_copy_object();
                let to = resume.take_copy_to();
                let from = resume.take_copy_from();
                let count = resume.take_copy_count();
                let backwards = resume.take_copy_backwards();
                resume.resume(
                    runtime,
                    super::copy::finish(
                        runtime,
                        realm,
                        super::copy::CopyStep::start(
                            runtime, realm, object, to, from, count, backwards,
                        )?,
                    )?,
                )?
            }
            IndexedStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            IndexedStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            IndexedStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            IndexedStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                {
                    let result = runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?;
                    resume.set(runtime, key, result)?
                }
            }
        };
    }
}

#[derive(Default)]
struct IndexedStepPending {
    copy_object: Option<ObjectRef>,
    copy_to: Option<u64>,
    copy_from: Option<u64>,
    copy_count: Option<u64>,
    copy_backwards: Option<bool>,
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
}
impl IndexedStep {
    pub(crate) fn request_copy(
        object: ObjectRef,
        to: u64,
        from: u64,
        count: u64,
        backwards: bool,
        mut resume: IndexedResume,
    ) -> Self {
        resume.0.pending_effect.copy_object = Some(object);
        resume.0.pending_effect.copy_to = Some(to);
        resume.0.pending_effect.copy_from = Some(from);
        resume.0.pending_effect.copy_count = Some(count);
        resume.0.pending_effect.copy_backwards = Some(backwards);
        Self::Copy { resume }
    }
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: IndexedResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: IndexedResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_has(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: IndexedResume,
    ) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: IndexedResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
}
impl IndexedResume {
    pub(crate) fn take_copy_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .copy_object
            .take()
            .expect("IndexedStep Copy object")
    }
    pub(crate) fn take_copy_to(&mut self) -> u64 {
        self.0
            .pending_effect
            .copy_to
            .take()
            .expect("IndexedStep Copy to")
    }
    pub(crate) fn take_copy_from(&mut self) -> u64 {
        self.0
            .pending_effect
            .copy_from
            .take()
            .expect("IndexedStep Copy from")
    }
    pub(crate) fn take_copy_count(&mut self) -> u64 {
        self.0
            .pending_effect
            .copy_count
            .take()
            .expect("IndexedStep Copy count")
    }
    pub(crate) fn take_copy_backwards(&mut self) -> bool {
        self.0
            .pending_effect
            .copy_backwards
            .take()
            .expect("IndexedStep Copy backwards")
    }
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("IndexedStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("IndexedStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("IndexedStep Number value")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("IndexedStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("IndexedStep Has key")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("IndexedStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("IndexedStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("IndexedStep Set value")
    }
}
const _: () = assert!(std::mem::size_of::<IndexedStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<IndexedStep>() <= 64);
