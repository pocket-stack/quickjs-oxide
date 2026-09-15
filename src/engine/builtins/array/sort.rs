//! Array sorting owns the collected slots, cached strings, and exact rqsort cursor.
use super::{
    ArraySortSlot,
    rqsort::{SortAction, SortMachine},
};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey, operations::InternalSetResult},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum SortStep {
    Complete(Completion),
    Read { resume: SortResume },
    Number { resume: SortResume },
    String { resume: SortResume },
    Has { resume: SortResume },
    Call { resume: SortResume },
    Set { resume: SortResume },
    Delete { resume: SortResume },
}
enum Phase {
    Length,
    LengthNumber,
    CollectHas,
    CollectRead,
    CompareCall,
    CompareNumber,
    LeftString,
    RightString,
    Write,
    Delete,
}
pub(crate) struct SortResume(Box<SortResumeState>);
impl std::ops::Deref for SortResume {
    type Target = SortResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for SortResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<SortResume>() <= 8);
pub(crate) struct SortResumeState {
    pending_effect: SortStepPending,
    scheduler_set_key: Option<PropertyKey>,
    realm: ContextId,
    copying: bool,
    object: ObjectRef,
    comparator: Option<CallableRef>,
    phase: Phase,
    length: u64,
    cursor: u64,
    undefined_count: u64,
    defined_count: u64,
    values: Vec<Value>,
    slots: Vec<ArraySortSlot>,
    logical_capacity: usize,
    // Keep rqsort's fixed partition stack outside each copied domain reply.
    machine: Box<SortMachine>,
    left: usize,
    right: usize,
}
impl SortStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        copying: bool,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let comparator = match runtime.native_sort_comparator(realm, arguments)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Array sort requires generic invocation",
            ));
        };
        let object = match runtime.native_to_object(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        Ok(Self::request_read(
            object.clone(),
            runtime.intern_property_key("length")?,
            SortResume(Box::new(SortResumeState {
                pending_effect: SortStepPending::default(),
                scheduler_set_key: None,
                realm,
                copying,
                object,
                comparator,
                phase: Phase::Length,
                length: 0,
                cursor: 0,
                undefined_count: 0,
                defined_count: 0,
                values: Vec::new(),
                slots: Vec::new(),
                logical_capacity: 0,
                machine: Box::new(SortMachine::new(0)),
                left: 0,
                right: 0,
            })),
        ))
    }
}
impl SortResume {
    pub(crate) fn with_scheduler_set_key(mut self, key: PropertyKey) -> Self {
        self.0.scheduler_set_key = Some(key);
        self
    }
    pub(crate) fn take_scheduler_set_key(&mut self) -> PropertyKey {
        self.0.scheduler_set_key.take().expect("waiting Set key")
    }

    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<SortStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(SortStep::Complete(Completion::Throw(value))),
        };
        match self.0.phase {
            Phase::Length => {
                self.0.phase = Phase::LengthNumber;
                Ok(SortStep::request_number(value, self))
            }
            Phase::CollectRead => {
                self.collect_value(value);
                self.0.cursor += 1;
                self.collect_next(runtime)
            }
            Phase::CompareCall => {
                if let Value::Int(value) = value {
                    return self.compared(runtime, order_from_number(f64::from(value)));
                }
                self.0.phase = Phase::CompareNumber;
                Ok(SortStep::request_number(value, self))
            }
            _ => Err(RuntimeError::Invariant("Array sort value phase mismatch")),
        }
    }
    pub(crate) fn number(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<f64>,
    ) -> Result<SortStep, RuntimeError> {
        let number = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(SortStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::LengthNumber => {
                self.0.length = Runtime::length_from_number(number);
                if self.0.copying {
                    self.0.values = match runtime
                        .native_allocate_fast_array_values(self.0.realm, self.0.length)?
                    {
                        NativeConversion::Value(values) => values,
                        NativeConversion::Throw(value) => {
                            return Ok(SortStep::Complete(Completion::Throw(value)));
                        }
                    };
                }
                self.collect_next(runtime)
            }
            Phase::CompareNumber => self.compared(runtime, order_from_number(number)),
            _ => Err(RuntimeError::Invariant("Array sort number phase mismatch")),
        }
    }
    fn collect_next(mut self, runtime: &Runtime) -> Result<SortStep, RuntimeError> {
        if self.0.cursor == self.0.length {
            if self.0.copying {
                (self.0.slots, self.0.undefined_count) =
                    Runtime::collect_dense_array_sort_slots(&self.0.values)?;
                self.0.object = runtime
                    .new_array_from_values(self.0.realm, std::mem::take(&mut self.0.values))?;
            }
            *self.0.machine = SortMachine::new(self.0.slots.len());
            return self.sort_next(runtime, None);
        }
        if !self.0.copying {
            Runtime::reserve_array_sort_slot_capacity(
                &mut self.0.slots,
                &mut self.0.logical_capacity,
            )?;
        }
        self.0.phase = Phase::CollectHas;
        Ok(SortStep::request_has(
            self.0.object.clone(),
            runtime.property_key_for_index(self.0.cursor)?,
            self,
        ))
    }
    fn collect_value(&mut self, value: Value) {
        if self.0.copying {
            self.0.values[self.0.cursor as usize] = value;
        } else if matches!(value, Value::Undefined) {
            self.0.undefined_count += 1;
        } else {
            self.0.slots.push(ArraySortSlot {
                value,
                cached_string: None,
                original_position: self.0.cursor,
            });
        }
    }
    pub(crate) fn boolean(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<SortStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(SortStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            Phase::CollectHas => {
                if !value {
                    self.0.cursor += 1;
                    return self.collect_next(runtime);
                }
                self.0.phase = Phase::CollectRead;
                Ok(SortStep::request_read(
                    self.0.object.clone(),
                    runtime.property_key_for_index(self.0.cursor)?,
                    self,
                ))
            }
            Phase::Delete => {
                if !value {
                    return Ok(SortStep::Complete(Completion::Throw(
                        runtime.new_native_error(
                            self.0.realm,
                            NativeErrorKind::Type,
                            "could not delete property",
                        )?,
                    )));
                }
                self.0.cursor += 1;
                self.write_next(runtime)
            }
            _ => Err(RuntimeError::Invariant("Array sort boolean phase mismatch")),
        }
    }
    fn sort_next(
        mut self,
        runtime: &Runtime,
        mut reply: Option<std::cmp::Ordering>,
    ) -> Result<SortStep, RuntimeError> {
        loop {
            match self.0.machine.advance(reply.take()) {
                SortAction::Complete => {
                    self.0.cursor = 0;
                    self.0.defined_count = self.0.slots.len() as u64;
                    return self.write_next(runtime);
                }
                SortAction::Swap(left, right) => self.0.slots.swap(left, right),
                SortAction::Compare(left, right) => {
                    self.0.left = left;
                    self.0.right = right;
                    if let Some(callable) = &self.0.comparator {
                        if self.0.slots[left]
                            .value
                            .same_quickjs_representation(&self.0.slots[right].value)
                        {
                            reply = Some(
                                self.0.slots[left]
                                    .original_position
                                    .cmp(&self.0.slots[right].original_position),
                            );
                            continue;
                        }
                        let callable = callable.clone();
                        self.0.phase = Phase::CompareCall;
                        return Ok(SortStep::request_call(
                            callable,
                            vec![
                                self.0.slots[left].value.clone(),
                                self.0.slots[right].value.clone(),
                            ],
                            self,
                        ));
                    }
                    if let (Some(left_string), Some(right_string)) = (
                        &self.0.slots[left].cached_string,
                        &self.0.slots[right].cached_string,
                    ) {
                        let ordering = left_string.utf16_units().cmp(right_string.utf16_units());
                        reply = Some(if ordering.is_eq() {
                            self.0.slots[left]
                                .original_position
                                .cmp(&self.0.slots[right].original_position)
                        } else {
                            ordering
                        });
                        continue;
                    }
                    return self.compare_strings(runtime);
                }
            }
        }
    }
    fn compare_strings(mut self, runtime: &Runtime) -> Result<SortStep, RuntimeError> {
        if self.0.slots[self.0.left].cached_string.is_none() {
            self.0.phase = Phase::LeftString;
            return Ok(SortStep::request_string(
                self.0.slots[self.0.left].value.clone(),
                self,
            ));
        }
        if self.0.slots[self.0.right].cached_string.is_none() {
            self.0.phase = Phase::RightString;
            return Ok(SortStep::request_string(
                self.0.slots[self.0.right].value.clone(),
                self,
            ));
        }
        let ordering = self.0.slots[self.0.left]
            .cached_string
            .as_ref()
            .ok_or(RuntimeError::Invariant(
                "Array sort left string cache missing",
            ))?
            .utf16_units()
            .cmp(
                self.0.slots[self.0.right]
                    .cached_string
                    .as_ref()
                    .ok_or(RuntimeError::Invariant(
                        "Array sort right string cache missing",
                    ))?
                    .utf16_units(),
            );
        self.compared(runtime, ordering)
    }
    pub(crate) fn string(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<JsString>,
    ) -> Result<SortStep, RuntimeError> {
        let value = match result {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => {
                return Ok(SortStep::Complete(Completion::Throw(value)));
            }
        };
        let index = match self.0.phase {
            Phase::LeftString => self.0.left,
            Phase::RightString => self.0.right,
            _ => return Err(RuntimeError::Invariant("Array sort string phase mismatch")),
        };
        self.0.slots[index].cached_string = Some(value);
        self.compare_strings(runtime)
    }
    fn compared(
        self,
        runtime: &Runtime,
        ordering: std::cmp::Ordering,
    ) -> Result<SortStep, RuntimeError> {
        let ordering = if ordering.is_eq() {
            self.0.slots[self.0.left]
                .original_position
                .cmp(&self.0.slots[self.0.right].original_position)
        } else {
            ordering
        };
        self.sort_next(runtime, Some(ordering))
    }
    fn write_next(mut self, runtime: &Runtime) -> Result<SortStep, RuntimeError> {
        while self.0.cursor < self.0.defined_count {
            let slot = &mut self.0.slots[self.0.cursor as usize];
            slot.cached_string.take();
            let value = std::mem::replace(&mut slot.value, Value::Undefined);
            if slot.original_position == self.0.cursor {
                self.0.cursor += 1;
                continue;
            }
            self.0.phase = Phase::Write;
            return Ok(SortStep::request_set(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.cursor)?,
                value,
                self,
            ));
        }
        drop(std::mem::take(&mut self.0.slots));
        if self.0.cursor < self.0.defined_count + self.0.undefined_count {
            self.0.phase = Phase::Write;
            return Ok(SortStep::request_set(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.cursor)?,
                Value::Undefined,
                self,
            ));
        }
        if self.0.cursor < self.0.length {
            self.0.phase = Phase::Delete;
            return Ok(SortStep::request_delete(
                self.0.object.clone(),
                runtime.property_key_for_index(self.0.cursor)?,
                self,
            ));
        }
        Ok(SortStep::Complete(Completion::Return(Value::Object(
            self.0.object,
        ))))
    }
    pub(crate) fn set(
        mut self,
        runtime: &Runtime,
        key: PropertyKey,
        result: NativeConversion<InternalSetResult>,
    ) -> Result<SortStep, RuntimeError> {
        if !matches!(self.0.phase, Phase::Write) {
            return Err(RuntimeError::Invariant("Array sort set phase mismatch"));
        }
        if let Some(value) = runtime.finish_set_property_or_throw(self.0.realm, &key, result)? {
            return Ok(SortStep::Complete(Completion::Throw(value)));
        }
        self.0.cursor += 1;
        self.write_next(runtime)
    }
}
fn order_from_number(number: f64) -> std::cmp::Ordering {
    if number > 0.0 {
        std::cmp::Ordering::Greater
    } else if number < 0.0 {
        std::cmp::Ordering::Less
    } else {
        std::cmp::Ordering::Equal
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: SortStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            SortStep::Complete(result) => return Ok(result),
            SortStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            SortStep::Number { mut resume } => {
                let value = resume.take_number_value();
                resume.number(runtime, runtime.native_to_number(realm, &value)?)?
            }
            SortStep::String { mut resume } => {
                let value = resume.take_string_value();
                resume.string(runtime, runtime.native_to_js_string(realm, &value)?)?
            }
            SortStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                resume.boolean(
                    runtime,
                    runtime.internal_has_property(realm, &object, &key)?,
                )?
            }
            SortStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let arguments = resume.take_call_arguments();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, Value::Undefined, &arguments)?,
                )?
            }
            SortStep::Set { mut resume } => {
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
            SortStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                resume.boolean(
                    runtime,
                    runtime.internal_delete_property(realm, &object, &key)?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct SortStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    number_value: Option<Value>,
    string_value: Option<Value>,
    has_object: Option<ObjectRef>,
    has_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_arguments: Option<Vec<Value>>,
    set_object: Option<ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
    delete_object: Option<ObjectRef>,
    delete_key: Option<PropertyKey>,
}
impl SortStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: SortResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_number(value: Value, mut resume: SortResume) -> Self {
        resume.0.pending_effect.number_value = Some(value);
        Self::Number { resume }
    }
    pub(crate) fn request_string(value: Value, mut resume: SortResume) -> Self {
        resume.0.pending_effect.string_value = Some(value);
        Self::String { resume }
    }
    pub(crate) fn request_has(object: ObjectRef, key: PropertyKey, mut resume: SortResume) -> Self {
        resume.0.pending_effect.has_object = Some(object);
        resume.0.pending_effect.has_key = Some(key);
        Self::Has { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        arguments: Vec<Value>,
        mut resume: SortResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
    pub(crate) fn request_set(
        object: ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: SortResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
    pub(crate) fn request_delete(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: SortResume,
    ) -> Self {
        resume.0.pending_effect.delete_object = Some(object);
        resume.0.pending_effect.delete_key = Some(key);
        Self::Delete { resume }
    }
}
impl SortResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("SortStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("SortStep Read key")
    }
    pub(crate) fn take_number_value(&mut self) -> Value {
        self.0
            .pending_effect
            .number_value
            .take()
            .expect("SortStep Number value")
    }
    pub(crate) fn take_string_value(&mut self) -> Value {
        self.0
            .pending_effect
            .string_value
            .take()
            .expect("SortStep String value")
    }
    pub(crate) fn take_has_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .has_object
            .take()
            .expect("SortStep Has object")
    }
    pub(crate) fn take_has_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .has_key
            .take()
            .expect("SortStep Has key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("SortStep Call callable")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("SortStep Call arguments")
    }
    pub(crate) fn take_set_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("SortStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("SortStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("SortStep Set value")
    }
    pub(crate) fn take_delete_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .delete_object
            .take()
            .expect("SortStep Delete object")
    }
    pub(crate) fn take_delete_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .delete_key
            .take()
            .expect("SortStep Delete key")
    }
}
const _: () = assert!(std::mem::size_of::<SortStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<SortStep>() <= 64);
