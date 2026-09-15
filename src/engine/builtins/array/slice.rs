//! Slice and splice keep copied result and completed receiver mutations across replies.
#[cfg(feature = "stack-vm")]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArraySliceKind,
    heap::ContextId,
    object::{
        DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        operations::{InternalDefineResult, InternalSetResult},
    },
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
#[derive(Clone, Copy)]
pub(crate) enum SliceKind {
    Slice,
    Splice,
    ToSpliced,
}
impl SliceKind {
    #[cfg(feature = "stack-vm")]
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        match target {
            NativeFunctionId::ArrayPrototypeSlice(ArraySliceKind::Slice) => Some(Self::Slice),
            NativeFunctionId::ArrayPrototypeSlice(ArraySliceKind::Splice) => Some(Self::Splice),
            NativeFunctionId::ArrayPrototypeToSpliced => Some(Self::ToSpliced),
            _ => None,
        }
    }
}
pub(crate) enum SliceStep {
    Return(Value),
    Throw(Value),
    PreparedRead { resume: SliceResume },
    PreparedHas { resume: SliceResume },
    Read { resume: SliceResume },
    Number { resume: SliceResume },
    Has { resume: SliceResume },
    Species { resume: SliceResume },
    Define { resume: SliceResume },
    Set { resume: SliceResume },
    Delete { resume: SliceResume },
    Copy { resume: SliceResume },
}
// Inline Value completions need 40 bytes; pending effects carry only the
// existing resident pointer, with no allocation on completion.
const _: () = assert!(std::mem::size_of::<SliceStep>() <= 40);
#[derive(Default)]
pub(crate) struct SlicePending {
    read: Option<crate::engine::object::OrdinaryRead>,
    key: Option<PropertyKey>,
    probe: Option<crate::engine::object::PreparedHas>,
    object: Option<ObjectRef>,
    value: Option<Value>,
    source: Option<ObjectRef>,
    length: Option<u64>,
    descriptor: Option<OrdinaryPropertyDescriptor>,
    to: Option<u64>,
    from: Option<u64>,
    count: Option<u64>,
    backwards: Option<bool>,
}

enum Phase {
    Length,
    LengthNumber,
    Start,
    End,
    Species,
    Has,
    Read,
    Define,
    ResultLength,
    Copy,
    Delete,
    Insert,
    FinalLength,
}
pub(crate) struct SliceResume(Box<SliceResumeState>);
impl std::ops::Deref for SliceResume {
    type Target = SliceResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for SliceResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<SliceResume>() <= 8);
pub(crate) struct SliceResumeState {
    scheduler_set_key: Option<PropertyKey>,
    pending: SlicePending,
    realm: ContextId,
    kind: SliceKind,
    phase: Phase,
    object: ObjectRef,
    arguments: Vec<Value>,
    actual: usize,
    length: u64,
    start: u64,
    count: u64,
    items: u64,
    new_length: u64,
    cursor: u64,
    result: Option<ObjectRef>,
    values: Vec<Value>,
}
impl SliceStep {
    fn complete(result: Completion) -> Self {
        match result {
            Completion::Return(value) => Self::Return(value),
            Completion::Throw(value) => Self::Throw(value),
        }
    }
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: SliceKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array slice requires generic invocation",
            ));
        };
        let object = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::complete(Completion::Throw(value))),
        };
        Self::make_read(
            object.clone(),
            runtime.intern_property_key("length")?,
            SliceResume(Box::new(SliceResumeState {
                scheduler_set_key: None,
                pending: SlicePending::default(),
                realm,
                kind,
                phase: Phase::Length,
                object,
                arguments: arguments.readable.clone(),
                actual: arguments.actual_arg_count,
                length: 0,
                start: 0,
                count: 0,
                items: arguments.actual_arg_count.saturating_sub(2) as u64,
                new_length: 0,
                cursor: 0,
                result: None,
                values: Vec::new(),
            })),
        )
        .advance_local(runtime, realm)
    }
}
impl SliceResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<SliceStep, RuntimeError> {
        let realm = self.0.realm;
        self.resume_once(runtime, result)?
            .advance_local(runtime, realm)
    }
    pub(crate) fn number(
        self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<SliceStep, RuntimeError> {
        let realm = self.0.realm;
        self.number_once(runtime, result)?
            .advance_local(runtime, realm)
    }
    pub(crate) fn boolean(
        self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<SliceStep, RuntimeError> {
        let realm = self.0.realm;
        self.boolean_once(runtime, result)?
            .advance_local(runtime, realm)
    }
    pub(crate) fn defined(
        self,
        runtime: &Runtime,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<SliceStep, RuntimeError> {
        let realm = self.0.realm;
        self.defined_once(runtime, result)?
            .advance_local(runtime, realm)
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<SliceStep, RuntimeError> {
        let realm = self.0.realm;
        self.set_once(runtime, key, result)?
            .advance_local(runtime, realm)
    }

    fn argument(&self, index: usize) -> Value {
        self.0
            .arguments
            .get(index)
            .cloned()
            .unwrap_or(Value::Undefined)
    }
    fn result(&self) -> Result<ObjectRef, RuntimeError> {
        self.0
            .result
            .clone()
            .ok_or(RuntimeError::Invariant("Array slice result missing"))
    }
    fn resume_once(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<SliceStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(SliceStep::complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::LengthNumber;
                Ok(SliceStep::make_number(value, self))
            }
            Phase::Species => {
                let Value::Object(object) = value else {
                    return Err(RuntimeError::Invariant(
                        "ArraySpeciesCreate returned primitive",
                    ));
                };
                self.0.result = Some(object);
                self.collect(runtime)
            }
            Phase::Read => {
                if matches!(self.0.kind, SliceKind::ToSpliced) {
                    self.0.values[self.0.cursor as usize] = value;
                    self.0.cursor += 1;
                    return self.collect(runtime);
                }
                self.0.phase = Phase::Define;
                Ok(SliceStep::make_define(
                    self.result()?,
                    runtime.property_key_for_index(self.0.cursor)?,
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
            Phase::Copy => {
                self.0.cursor = self.0.length;
                self.delete_next(runtime)
            }
            _ => Err(RuntimeError::Invariant("Array slice value phase mismatch")),
        }
    }
    fn number_once(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<SliceStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(SliceStep::complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::LengthNumber => {
                self.0.length = Runtime::length_from_number(number);
                if matches!(self.0.kind, SliceKind::ToSpliced) && self.0.actual == 0 {
                    return self.end(runtime);
                }
                self.0.phase = Phase::Start;
                Ok(SliceStep::make_number(self.argument(0), self))
            }
            Phase::Start => {
                let mut index = Runtime::int64_from_number(number);
                if index < 0 {
                    index += self.0.length as i64;
                }
                self.0.start = index.clamp(0, self.0.length as i64) as u64;
                self.end(runtime)
            }
            Phase::End => {
                let mut value = Runtime::int64_from_number(number);
                if matches!(self.0.kind, SliceKind::Slice) {
                    if value < 0 {
                        value += self.0.length as i64;
                    }
                    self.0.count =
                        (value.clamp(0, self.0.length as i64) as u64).saturating_sub(self.0.start);
                } else {
                    self.0.count = value.clamp(0, (self.0.length - self.0.start) as i64) as u64;
                }
                self.allocate(runtime)
            }
            _ => Err(RuntimeError::Invariant("Array slice number phase mismatch")),
        }
    }
    fn end(mut self, runtime: &Runtime) -> Result<SliceStep, RuntimeError> {
        let value = self.argument(1);
        let convert = match self.0.kind {
            SliceKind::Slice => self.0.actual > 1 && !matches!(value, Value::Undefined),
            _ => self.0.actual > 1,
        };
        if convert {
            self.0.phase = Phase::End;
            return Ok(SliceStep::make_number(value, self));
        }
        self.0.count = if !matches!(self.0.kind, SliceKind::Slice) && self.0.actual == 0 {
            0
        } else {
            self.0.length - self.0.start
        };
        self.allocate(runtime)
    }
    fn allocate(mut self, runtime: &Runtime) -> Result<SliceStep, RuntimeError> {
        if !matches!(self.0.kind, SliceKind::Slice) {
            self.0.new_length = (self.0.length - self.0.count).saturating_add(self.0.items);
            if self.0.new_length > (1_u64 << 53) - 1 {
                return Ok(SliceStep::complete(Completion::Throw(
                    runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        if matches!(self.0.kind, SliceKind::ToSpliced) {
                            "invalid array length"
                        } else {
                            "Array loo long"
                        },
                    )?,
                )));
            }
        }
        if matches!(self.0.kind, SliceKind::ToSpliced) {
            self.0.values =
                match runtime.native_allocate_fast_array_values(self.0.realm, self.0.new_length)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(SliceStep::complete(Completion::Throw(value)));
                    }
                };
            self.collect(runtime)
        } else {
            self.0.phase = Phase::Species;
            Ok(SliceStep::make_species(
                self.0.object.clone(),
                self.0.count,
                self,
            ))
        }
    }
    fn source_index(&self) -> u64 {
        if matches!(self.0.kind, SliceKind::ToSpliced) {
            if self.0.cursor < self.0.start {
                self.0.cursor
            } else {
                self.0.cursor - self.0.items + self.0.count
            }
        } else {
            self.0.start + self.0.cursor
        }
    }
    fn collect(mut self, runtime: &Runtime) -> Result<SliceStep, RuntimeError> {
        loop {
            if matches!(self.0.kind, SliceKind::ToSpliced) {
                if self.0.cursor == self.0.start {
                    for index in 0..self.0.items {
                        self.0.values[(self.0.start + index) as usize] =
                            self.argument(index as usize + 2);
                    }
                    self.0.cursor += self.0.items;
                }
                if self.0.cursor == self.0.new_length {
                    return Ok(SliceStep::complete(Completion::Return(Value::Object(
                        runtime.new_array_from_values(self.0.realm, self.0.values)?,
                    ))));
                }
            } else if self.0.cursor == self.0.count {
                self.0.phase = Phase::ResultLength;
                return Ok(SliceStep::make_set(
                    self.result()?,
                    runtime.intern_property_key("length")?,
                    Value::number(self.0.count as f64),
                    self,
                ));
            }
            self.0.phase = Phase::Has;
            let key = runtime.property_key_for_index(self.source_index())?;
            #[cfg(not(feature = "stack-vm"))]
            return Ok(SliceStep::make_has(self.0.object.clone(), key, self));
            #[cfg(feature = "stack-vm")]
            {
                use crate::engine::object::{OrdinaryRead, PreparedHas};
                match runtime.prepare_has_property(&self.0.object, &key)? {
                    PreparedHas::Complete(has) => {
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "array_slice_local_has",
                        );
                        if !has {
                            self.0.cursor += 1;
                            continue;
                        }
                    }
                    probe => {
                        return Ok(SliceStep::make_preparedhas(probe, key, self));
                    }
                }
                self.0.phase = Phase::Read;
                let receiver = Value::Object(self.0.object.clone());
                let value = match runtime.prepare_ordinary_read_borrowed(
                    &self.0.object,
                    &key,
                    &receiver,
                )? {
                    OrdinaryRead::Complete(value) => {
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "array_slice_local_read",
                        );
                        value.unwrap_or(Value::Undefined)
                    }
                    read => {
                        return Ok(SliceStep::make_preparedread(read, key, self));
                    }
                };
                // End the read receiver before the following Define, as in the
                // ordinary Read adapter. The cursor alone keeps source alive.
                drop(receiver);
                drop(key);
                if matches!(self.0.kind, SliceKind::ToSpliced) {
                    self.0.values[self.0.cursor as usize] = value;
                    self.0.cursor += 1;
                    continue;
                }
                self.0.phase = Phase::Define;
                let key = runtime.property_key_for_index(self.0.cursor)?;
                let descriptor = OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                };
                let object = self
                    .0
                    .result
                    .as_ref()
                    .ok_or(RuntimeError::Invariant("Array slice result missing"))?;
                if !local::direct_indexed_target(runtime, object, &key)? {
                    return Ok(SliceStep::make_define(
                        object.clone(),
                        key,
                        descriptor,
                        self,
                    ));
                }
                let result = local::define_local(runtime, self.0.realm, object, &key, &descriptor)?;
                if let Some(value) = runtime.finish_create_indexed_data_property(
                    self.0.realm,
                    self.0.cursor,
                    result,
                )? {
                    return Ok(SliceStep::complete(Completion::Throw(value)));
                }
                self.0.cursor += 1;
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "array_slice_resident_element",
                );
            }
        }
    }
    fn boolean_once(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<SliceStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(SliceStep::complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::Has if value => {
                self.0.phase = Phase::Read;
                Ok(SliceStep::make_read(
                    self.0.object.clone(),
                    runtime.property_key_for_index(self.source_index())?,
                    self,
                ))
            }
            Phase::Has => {
                self.0.cursor += 1;
                self.collect(runtime)
            }
            Phase::Delete => {
                if !value {
                    return Ok(SliceStep::complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "could not delete property",
                        )?,
                    )));
                }
                self.delete_next(runtime)
            }
            _ => Err(RuntimeError::Invariant(
                "Array slice boolean phase mismatch",
            )),
        }
    }
    fn defined_once(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<SliceStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Define) {
            return Err(RuntimeError::Invariant("Array slice define phase mismatch"));
        }
        if let Some(value) =
            runtime.finish_create_indexed_data_property(self.0.realm, self.0.cursor, result)?
        {
            return Ok(SliceStep::complete(Completion::Throw(value)));
        }
        self.0.cursor += 1;
        self.collect(runtime)
    }
    fn mutate(mut self, runtime: &Runtime) -> Result<SliceStep, RuntimeError> {
        if matches!(self.0.kind, SliceKind::Slice) {
            return self.complete();
        }
        if self.0.items != self.0.count {
            self.0.phase = Phase::Copy;
            return Ok(SliceStep::make_copy(
                self.0.object.clone(),
                self.0.start + self.0.items,
                self.0.start + self.0.count,
                self.0.length - self.0.start - self.0.count,
                self.0.items > self.0.count,
                self,
            ));
        }
        self.0.cursor = 0;
        self.insert(runtime)
    }
    fn delete_next(mut self, runtime: &Runtime) -> Result<SliceStep, RuntimeError> {
        if self.0.cursor > self.0.new_length {
            self.0.cursor -= 1;
            self.0.phase = Phase::Delete;
            return Ok(SliceStep::make_delete(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.cursor)?,
                self,
            ));
        }
        self.0.cursor = 0;
        self.insert(runtime)
    }
    fn insert(mut self, runtime: &Runtime) -> Result<SliceStep, RuntimeError> {
        if self.0.cursor < self.0.items {
            self.0.phase = Phase::Insert;
            return Ok(SliceStep::make_set(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.start + self.0.cursor)?,
                self.argument(self.0.cursor as usize + 2),
                self,
            ));
        }
        self.0.phase = Phase::FinalLength;
        Ok(SliceStep::make_set(
            self.0.object.clone(),
            runtime.intern_property_key("length")?,
            Value::number(self.0.new_length as f64),
            self,
        ))
    }
    fn set_once(
        mut self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<SliceStep, RuntimeError> {
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(SliceStep::complete(Completion::Throw(value)));
        }
        match self.0.phase {
            Phase::ResultLength => self.mutate(runtime),
            Phase::Insert => {
                self.0.cursor += 1;
                self.insert(runtime)
            }
            Phase::FinalLength => self.complete(),
            _ => Err(RuntimeError::Invariant("Array slice set phase mismatch")),
        }
    }
    fn complete(self) -> Result<SliceStep, RuntimeError> {
        Ok(SliceStep::complete(Completion::Return(Value::Object(
            self.result()?,
        ))))
    }
}
mod local;
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: SliceStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            SliceStep::Return(value) => return Ok(Completion::Return(value)),
            SliceStep::Throw(value) => return Ok(Completion::Throw(value)),
            SliceStep::PreparedRead { mut resume } => {
                let (read, key) = resume.take_preparedread();
                {
                    let completion = match runtime.finish_prepared_read(realm, &key, read)? {
                        NativeConversion::Value(value) => {
                            Completion::Return(value.unwrap_or(Value::Undefined))
                        }
                        NativeConversion::Throw(value) => Completion::Throw(value),
                    };
                    resume.resume(runtime, completion)?
                }
            }
            SliceStep::PreparedHas { mut resume } => {
                let (probe, key) = resume.take_preparedhas();
                resume.boolean(runtime, runtime.finish_prepared_has(realm, &key, probe)?)?
            }
            SliceStep::Read { mut resume } => {
                let (object, key) = resume.take_read();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            SliceStep::Number { mut resume } => {
                let (value,) = resume.take_number();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            SliceStep::Has { mut resume } => {
                let (object, key) = resume.take_has();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            SliceStep::Species { mut resume } => {
                let (source, length) = resume.take_species();
                resume.resume(
                    runtime,
                    super::species::finish(
                        runtime,
                        realm,
                        super::species::SpeciesStep::start(runtime, realm, &source, length)?,
                    )?,
                )?
            }
            SliceStep::Define { mut resume } => {
                let (object, key, descriptor) = resume.take_define();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
            SliceStep::Set { mut resume } => {
                let (object, key, value) = resume.take_set();
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
            SliceStep::Delete { mut resume } => {
                let (object, key) = resume.take_delete();
                resume.boolean(
                    runtime,
                    runtime.internal_delete_property(realm, &object, &key)?,
                )?
            }
            SliceStep::Copy { mut resume } => {
                let (object, to, from, count, backwards) = resume.take_copy();
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
        };
    }
}

#[cfg(test)]
#[test]
fn slice_resume_keeps_one_resident_owner_across_number_transitions() {
    let runtime = Runtime::new();
    let context = runtime.new_context();
    let object = runtime.new_object(None).unwrap();
    let resume = SliceResume(Box::new(SliceResumeState {
        scheduler_set_key: None,
        pending: SlicePending::default(),
        realm: context.realm,
        kind: SliceKind::Slice,
        phase: Phase::LengthNumber,
        object: object.clone(),
        arguments: vec![Value::Object(object.clone()), Value::Object(object)],
        actual: 2,
        length: 0,
        start: 0,
        count: 0,
        items: 0,
        new_length: 0,
        cursor: 0,
        result: None,
        values: Vec::new(),
    }));
    let address = &*resume.0 as *const SliceResumeState;
    let SliceStep::Number { mut resume, .. } = resume
        .number_once(&runtime, NativeConversion::Value(4.0))
        .unwrap()
    else {
        panic!("start conversion")
    };
    assert_eq!(&*resume.0 as *const SliceResumeState, address);
    let (pending_value,) = resume.take_number();
    assert!(matches!(pending_value, Value::Object(_)));
    assert!(resume.0.pending.value.is_none());
    let SliceStep::Number { mut resume, .. } = resume
        .number_once(&runtime, NativeConversion::Value(1.0))
        .unwrap()
    else {
        panic!("end conversion")
    };
    assert_eq!(&*resume.0 as *const SliceResumeState, address);
    let (pending_value,) = resume.take_number();
    assert!(matches!(pending_value, Value::Object(_)));
    assert!(resume.0.pending.value.is_none());
    assert_eq!(resume.0.start, 1);
}

impl SliceStep {
    fn make_preparedread(
        read: crate::engine::object::OrdinaryRead,
        key: PropertyKey,
        mut resume: SliceResume,
    ) -> Self {
        resume.0.pending.read = Some(read);
        resume.0.pending.key = Some(key);
        Self::PreparedRead { resume }
    }
    fn make_preparedhas(
        probe: crate::engine::object::PreparedHas,
        key: PropertyKey,
        mut resume: SliceResume,
    ) -> Self {
        resume.0.pending.probe = Some(probe);
        resume.0.pending.key = Some(key);
        Self::PreparedHas { resume }
    }
    fn make_read(object: ObjectRef, key: PropertyKey, mut resume: SliceResume) -> Self {
        resume.0.pending.object = Some(object);
        resume.0.pending.key = Some(key);
        Self::Read { resume }
    }
    fn make_number(value: Value, mut resume: SliceResume) -> Self {
        resume.0.pending.value = Some(value);
        Self::Number { resume }
    }

    fn make_species(source: ObjectRef, length: u64, mut resume: SliceResume) -> Self {
        resume.0.pending.source = Some(source);
        resume.0.pending.length = Some(length);
        Self::Species { resume }
    }
    fn make_define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: SliceResume,
    ) -> Self {
        resume.0.pending.object = Some(object);
        resume.0.pending.key = Some(key);
        resume.0.pending.descriptor = Some(descriptor);
        Self::Define { resume }
    }
    fn make_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: SliceResume,
    ) -> Self {
        resume.0.pending.object = Some(object);
        resume.0.pending.key = Some(key);
        resume.0.pending.value = Some(value);
        Self::Set { resume }
    }
    fn make_delete(object: ObjectRef, key: PropertyKey, mut resume: SliceResume) -> Self {
        resume.0.pending.object = Some(object);
        resume.0.pending.key = Some(key);
        Self::Delete { resume }
    }
    fn make_copy(
        object: ObjectRef,
        to: u64,
        from: u64,
        count: u64,
        backwards: bool,
        mut resume: SliceResume,
    ) -> Self {
        resume.0.pending.object = Some(object);
        resume.0.pending.to = Some(to);
        resume.0.pending.from = Some(from);
        resume.0.pending.count = Some(count);
        resume.0.pending.backwards = Some(backwards);
        Self::Copy { resume }
    }
}
impl SliceResume {
    pub(crate) fn take_preparedread(
        &mut self,
    ) -> (crate::engine::object::OrdinaryRead, PropertyKey) {
        (
            self.0
                .pending
                .read
                .take()
                .expect("slice PreparedRead lost read"),
            self.0
                .pending
                .key
                .take()
                .expect("slice PreparedRead lost key"),
        )
    }
    pub(crate) fn take_preparedhas(&mut self) -> (crate::engine::object::PreparedHas, PropertyKey) {
        (
            self.0
                .pending
                .probe
                .take()
                .expect("slice PreparedHas lost probe"),
            self.0
                .pending
                .key
                .take()
                .expect("slice PreparedHas lost key"),
        )
    }
    pub(crate) fn take_read(&mut self) -> (ObjectRef, PropertyKey) {
        (
            self.0
                .pending
                .object
                .take()
                .expect("slice Read lost object"),
            self.0.pending.key.take().expect("slice Read lost key"),
        )
    }
    pub(crate) fn take_number(&mut self) -> (Value,) {
        (self
            .0
            .pending
            .value
            .take()
            .expect("slice Number lost value"),)
    }
    pub(crate) fn take_has(&mut self) -> (ObjectRef, PropertyKey) {
        (
            self.0.pending.object.take().expect("slice Has lost object"),
            self.0.pending.key.take().expect("slice Has lost key"),
        )
    }
    pub(crate) fn take_species(&mut self) -> (ObjectRef, u64) {
        (
            self.0
                .pending
                .source
                .take()
                .expect("slice Species lost source"),
            self.0
                .pending
                .length
                .take()
                .expect("slice Species lost length"),
        )
    }
    pub(crate) fn take_define(&mut self) -> (ObjectRef, PropertyKey, OrdinaryPropertyDescriptor) {
        (
            self.0
                .pending
                .object
                .take()
                .expect("slice Define lost object"),
            self.0.pending.key.take().expect("slice Define lost key"),
            self.0
                .pending
                .descriptor
                .take()
                .expect("slice Define lost descriptor"),
        )
    }
    pub(crate) fn take_set(&mut self) -> (ObjectRef, PropertyKey, Value) {
        (
            self.0.pending.object.take().expect("slice Set lost object"),
            self.0.pending.key.take().expect("slice Set lost key"),
            self.0.pending.value.take().expect("slice Set lost value"),
        )
    }
    pub(crate) fn take_delete(&mut self) -> (ObjectRef, PropertyKey) {
        (
            self.0
                .pending
                .object
                .take()
                .expect("slice Delete lost object"),
            self.0.pending.key.take().expect("slice Delete lost key"),
        )
    }
    pub(crate) fn take_copy(&mut self) -> (ObjectRef, u64, u64, u64, bool) {
        (
            self.0
                .pending
                .object
                .take()
                .expect("slice Copy lost object"),
            self.0.pending.to.take().expect("slice Copy lost to"),
            self.0.pending.from.take().expect("slice Copy lost from"),
            self.0.pending.count.take().expect("slice Copy lost count"),
            self.0
                .pending
                .backwards
                .take()
                .expect("slice Copy lost backwards"),
        )
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<SliceStep>() <= 64);
