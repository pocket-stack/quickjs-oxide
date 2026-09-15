use super::ObjectIteratorStep;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::{ContextId, HeapError, IteratorConcatData, IteratorConcatItem, ObjectData, RawValue},
    object::{CallableRef, ObjectRef, PropertyKey, WellKnownSymbol},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation, NativeInvokeOutcome},
    },
};

impl Runtime {
    pub(crate) fn call_iterator_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match finish(
            self,
            realm,
            ConcatStep::start(self, realm, ConcatKind::Create, &invocation, arguments)?,
        )? {
            NativeInvokeOutcome::Completion(result) => Ok(result),
            NativeInvokeOutcome::IteratorNextRaw { .. } => {
                Err(RuntimeError::Invariant("concat creation returned raw next"))
            }
        }
    }

    fn new_iterator_concat(
        &self,
        realm: ContextId,
        inputs: &[(ObjectRef, Value)],
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.iterator_realm_data(realm)?.concat_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let items = inputs
            .iter()
            .map(|(iterable, method)| {
                Ok(Some(IteratorConcatItem {
                    iterable: iterable.object_id(),
                    method: self.raw_property_value(method)?,
                }))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let retained_atoms =
            match state.retain_raw_value_atoms(items.iter().flatten().map(|item| &item.method)) {
                Ok(atoms) => atoms,
                Err(error) => {
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error);
                }
            };
        let object =
            match state
                .heap
                .allocate_object(ObjectData::iterator_concat(shape, Vec::new(), items))
            {
                Ok(object) => object,
                Err(error) => {
                    state.release_atoms(retained_atoms)?;
                    let cleanup = state.heap.release_shape(shape)?;
                    state.apply_cleanup(cleanup)?;
                    return Err(error.into());
                }
            };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn iterator_concat_snapshot(
        &self,
        realm: ContextId,
        concat: &ObjectRef,
    ) -> Result<NativeConversion<IteratorConcatData>, RuntimeError> {
        let snapshot = {
            let state = self.0.state.borrow();
            state.heap.iterator_concat_state(concat.object_id())
        };
        match snapshot {
            Ok(snapshot) => Ok(NativeConversion::Value(snapshot)),
            Err(HeapError::Invariant(_)) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an Iterator Concat",
            )?)),
            Err(error) => Err(error.into()),
        }
    }

    fn set_iterator_concat_running(
        &self,
        concat: &ObjectRef,
        running: bool,
    ) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .set_iterator_concat_running(concat.object_id(), running)?;
        Ok(())
    }

    fn set_iterator_concat_iterator(
        &self,
        concat: &ObjectRef,
        iterator: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .set_iterator_concat_iterator(concat.object_id(), Some(iterator.object_id()))?;
        state.apply_cleanup(cleanup)
    }

    fn set_iterator_concat_next(
        &self,
        concat: &ObjectRef,
        next: &Value,
    ) -> Result<(), RuntimeError> {
        let raw = self.raw_property_value(next)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atoms = state.retain_raw_value_atoms([&raw])?;
        let cleanup = match state.heap.set_iterator_concat_next(concat.object_id(), raw) {
            Ok(cleanup) => cleanup,
            Err(error) => {
                state.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
        };
        state.apply_cleanup(cleanup)
    }

    fn advance_iterator_concat(&self, concat: &ObjectRef) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.advance_iterator_concat(concat.object_id())?;
        state.apply_cleanup(cleanup)
    }

    fn clear_iterator_concat(&self, concat: &ObjectRef) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.clear_iterator_concat(concat.object_id())?;
        state.apply_cleanup(cleanup)
    }

    pub(crate) fn call_iterator_concat_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_iterator_concat_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    pub(crate) fn call_iterator_concat_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        finish(
            self,
            realm,
            ConcatStep::start(
                self,
                realm,
                ConcatKind::Next,
                &invocation,
                &NativeArguments {
                    actual_arg_count: 0,
                    readable: Vec::new(),
                },
            )?,
        )
    }

    pub(crate) fn call_iterator_concat_return(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match finish(
            self,
            realm,
            ConcatStep::start(
                self,
                realm,
                ConcatKind::Return,
                &invocation,
                &NativeArguments {
                    actual_arg_count: 0,
                    readable: Vec::new(),
                },
            )?,
        )? {
            NativeInvokeOutcome::Completion(result) => Ok(result),
            NativeInvokeOutcome::IteratorNextRaw { .. } => {
                Err(RuntimeError::Invariant("concat return returned raw next"))
            }
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) enum ConcatKind {
    Create,
    Next,
    Return,
}
pub(crate) enum ConcatStep {
    Complete(NativeInvokeOutcome),
    Read { resume: ConcatResume },
    Call { resume: ConcatResume },
    Next { resume: ConcatResume },
}
struct ConcatGuard {
    runtime: Runtime,
    concat: ObjectRef,
    active: bool,
    clear: bool,
}
impl ConcatGuard {
    fn reset(&mut self) -> Result<(), RuntimeError> {
        self.runtime
            .set_iterator_concat_running(&self.concat, false)?;
        if self.clear {
            self.runtime.clear_iterator_concat(&self.concat)?;
        }
        self.active = false;
        Ok(())
    }
}
impl Drop for ConcatGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = self
                .runtime
                .set_iterator_concat_running(&self.concat, false);
            if self.clear {
                let _ = self.runtime.clear_iterator_concat(&self.concat);
            }
        }
    }
}
pub(crate) struct ConcatResume(Box<ConcatResumeState>);
impl std::ops::Deref for ConcatResume {
    type Target = ConcatResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ConcatResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ConcatResume>() <= 8);
pub(crate) struct ConcatResumeState {
    pending_effect: ConcatStepPending,
    realm: ContextId,
    phase: ConcatPhase,
    guard: Option<ConcatGuard>,
}
enum ConcatPhase {
    Input {
        remaining: std::vec::IntoIter<Value>,
        inputs: Vec<(ObjectRef, Value)>,
        current: ObjectRef,
    },
    Iterator,
    Method(ObjectRef),
    Next,
    ReturnMethod(ObjectRef),
    ReturnResult,
}
impl ConcatStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: ConcatKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        if matches!(kind, ConcatKind::Create) {
            if !matches!(invocation, NativeInvocation::Call { .. }) {
                return Err(RuntimeError::Invariant(
                    "Iterator.concat did not receive a generic invocation",
                ));
            }
            return ConcatResume::input(
                runtime,
                realm,
                arguments.readable[..arguments.actual_arg_count]
                    .to_vec()
                    .into_iter(),
                Vec::with_capacity(arguments.actual_arg_count),
            );
        }
        let concat = match runtime.iterator_receiver(realm, invocation.clone())? {
            NativeConversion::Value(concat) => concat,
            NativeConversion::Throw(value) => {
                return Ok(Self::Complete(NativeInvokeOutcome::Completion(
                    Completion::Throw(value),
                )));
            }
        };
        let snapshot = match runtime.iterator_concat_snapshot(realm, &concat)? {
            NativeConversion::Value(snapshot) => snapshot,
            NativeConversion::Throw(value) => {
                return Ok(Self::Complete(NativeInvokeOutcome::Completion(
                    Completion::Throw(value),
                )));
            }
        };
        if snapshot.running {
            return Ok(Self::Complete(NativeInvokeOutcome::Completion(
                Completion::Throw(runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "already running",
                )?),
            )));
        }
        if matches!(kind, ConcatKind::Return) && snapshot.iterator.is_none() {
            runtime.clear_iterator_concat(&concat)?;
            return Ok(Self::Complete(NativeInvokeOutcome::Completion(
                Completion::Return(Value::Undefined),
            )));
        }
        runtime.set_iterator_concat_running(&concat, true)?;
        let guard = ConcatGuard {
            runtime: runtime.clone(),
            concat,
            active: true,
            clear: false,
        };
        let mut resume = ConcatResume(Box::new(ConcatResumeState {
            pending_effect: ConcatStepPending::default(),
            realm,
            phase: ConcatPhase::Next,
            guard: Some(guard),
        }));
        if matches!(kind, ConcatKind::Return) {
            let iterator = ObjectRef::from_borrowed_handle(
                runtime.clone(),
                snapshot
                    .iterator
                    .ok_or(RuntimeError::Invariant("concat return iterator missing"))?,
            )?;
            resume.phase = ConcatPhase::ReturnMethod(iterator.clone());
            return Ok({
                let __pending_field_object = iterator;
                let __pending_field_key = runtime.intern_property_key("return")?;
                let __pending_field_resume = resume;
                Self::request_read(
                    __pending_field_object,
                    __pending_field_key,
                    __pending_field_resume,
                )
            });
        }
        resume.advance(runtime)
    }
}
impl ConcatResume {
    fn input(
        runtime: &Runtime,
        realm: ContextId,
        mut remaining: std::vec::IntoIter<Value>,
        inputs: Vec<(ObjectRef, Value)>,
    ) -> Result<ConcatStep, RuntimeError> {
        let Some(input) = remaining.next() else {
            return Ok(ConcatStep::Complete(NativeInvokeOutcome::Completion(
                Completion::Return(Value::Object(runtime.new_iterator_concat(realm, &inputs)?)),
            )));
        };
        let Value::Object(current) = input else {
            return Ok(ConcatStep::Complete(NativeInvokeOutcome::Completion(
                Completion::Throw(runtime.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?),
            )));
        };
        Ok({
            let __pending_field_object = current.clone();
            let __pending_field_key =
                PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Iterator));
            let __pending_field_resume = Self(Box::new(ConcatResumeState {
                pending_effect: ConcatStepPending::default(),
                realm,
                guard: None,
                phase: ConcatPhase::Input {
                    remaining,
                    inputs,
                    current,
                },
            }));
            ConcatStep::request_read(
                __pending_field_object,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
    fn concat(&self) -> Result<&ObjectRef, RuntimeError> {
        self.0
            .guard
            .as_ref()
            .map(|guard| &guard.concat)
            .ok_or(RuntimeError::Invariant(
                "concat resume has no running owner",
            ))
    }
    fn complete(mut self, result: NativeInvokeOutcome) -> Result<ConcatStep, RuntimeError> {
        if let Some(guard) = &mut self.0.guard {
            guard.reset()?;
        }
        Ok(ConcatStep::Complete(result))
    }
    fn advance(mut self, runtime: &Runtime) -> Result<ConcatStep, RuntimeError> {
        let snapshot = {
            runtime
                .0
                .state
                .borrow()
                .heap
                .iterator_concat_state(self.concat()?.object_id())?
        };
        if snapshot.index >= snapshot.items.len() {
            return self.complete(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Undefined,
                done: true,
            });
        }
        if let Some(iterator) = snapshot.iterator {
            return self.method(
                runtime,
                ObjectRef::from_borrowed_handle(runtime.clone(), iterator)?,
            );
        }
        let item = snapshot
            .items
            .get(snapshot.index)
            .and_then(Option::as_ref)
            .ok_or(RuntimeError::Invariant(
                "Iterator Concat current input was already released",
            ))?;
        let iterable = ObjectRef::from_borrowed_handle(runtime.clone(), item.iterable)?;
        let callable = match runtime
            .iterator_callable_value(self.0.realm, runtime.root_raw_value(&item.method)?)?
        {
            NativeConversion::Value(callable) => callable,
            NativeConversion::Throw(_) => {
                return Err(RuntimeError::Invariant(
                    "Iterator Concat captured method lost its callable brand",
                ));
            }
        };
        self.0.phase = ConcatPhase::Iterator;
        Ok({
            let __pending_field_callable = callable;
            let __pending_field_receiver = Value::Object(iterable);
            let __pending_field_resume = self;
            ConcatStep::request_call(
                __pending_field_callable,
                __pending_field_receiver,
                __pending_field_resume,
            )
        })
    }
    fn method(
        mut self,
        runtime: &Runtime,
        iterator: ObjectRef,
    ) -> Result<ConcatStep, RuntimeError> {
        let snapshot = {
            runtime
                .0
                .state
                .borrow()
                .heap
                .iterator_concat_state(self.concat()?.object_id())?
        };
        if matches!(snapshot.next, RawValue::Undefined) {
            self.0.phase = ConcatPhase::Method(iterator.clone());
            return Ok({
                let __pending_field_object = iterator;
                let __pending_field_key = runtime.intern_property_key("next")?;
                let __pending_field_resume = self;
                ConcatStep::request_read(
                    __pending_field_object,
                    __pending_field_key,
                    __pending_field_resume,
                )
            });
        }
        let method = runtime.root_raw_value(&snapshot.next)?;
        self.0.phase = ConcatPhase::Next;
        Ok({
            let __pending_field_iterator = iterator;
            let __pending_field_method = method;
            let __pending_field_resume = self;
            ConcatStep::request_next(
                __pending_field_iterator,
                __pending_field_method,
                __pending_field_resume,
            )
        })
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<ConcatStep, RuntimeError> {
        let value = match reply {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return self.complete(NativeInvokeOutcome::Completion(Completion::Throw(value)));
            }
        };
        match std::mem::replace(&mut self.0.phase, ConcatPhase::Next) {
            ConcatPhase::Input {
                remaining,
                mut inputs,
                current,
            } => {
                if let NativeConversion::Throw(value) =
                    runtime.iterator_callable_value(self.0.realm, value.clone())?
                {
                    return self
                        .complete(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                }
                inputs.push((current, value));
                Self::input(runtime, self.0.realm, remaining, inputs)
            }
            ConcatPhase::Iterator => {
                let Value::Object(iterator) = value else {
                    let error = runtime.new_native_error(
                        self.0.realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?;
                    return self
                        .complete(NativeInvokeOutcome::Completion(Completion::Throw(error)));
                };
                runtime.set_iterator_concat_iterator(self.concat()?, &iterator)?;
                self.method(runtime, iterator)
            }
            ConcatPhase::Method(iterator) => {
                runtime.set_iterator_concat_next(self.concat()?, &value)?;
                Ok({
                    let __pending_field_iterator = iterator;
                    let __pending_field_method = value;
                    let __pending_field_resume = self;
                    ConcatStep::request_next(
                        __pending_field_iterator,
                        __pending_field_method,
                        __pending_field_resume,
                    )
                })
            }
            ConcatPhase::ReturnMethod(iterator) => {
                self.0
                    .guard
                    .as_mut()
                    .ok_or(RuntimeError::Invariant("concat return owner missing"))?
                    .clear = true;
                let callable = match runtime.iterator_callable_value(self.0.realm, value)? {
                    NativeConversion::Value(callable) => callable,
                    NativeConversion::Throw(value) => {
                        return self
                            .complete(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                    }
                };
                self.0.phase = ConcatPhase::ReturnResult;
                Ok({
                    let __pending_field_callable = callable;
                    let __pending_field_receiver = Value::Object(iterator);
                    let __pending_field_resume = self;
                    ConcatStep::request_call(
                        __pending_field_callable,
                        __pending_field_receiver,
                        __pending_field_resume,
                    )
                })
            }
            ConcatPhase::ReturnResult => {
                self.complete(NativeInvokeOutcome::Completion(Completion::Return(value)))
            }
            ConcatPhase::Next => Err(RuntimeError::Invariant("concat next received completion")),
        }
    }
    pub(crate) fn next(
        self,
        runtime: &Runtime,
        reply: ObjectIteratorStep,
    ) -> Result<ConcatStep, RuntimeError> {
        if !matches!(self.0.phase, ConcatPhase::Next) {
            return Err(RuntimeError::Invariant("concat next reply phase mismatch"));
        }
        match reply {
            ObjectIteratorStep::Yield(value) => {
                self.complete(NativeInvokeOutcome::IteratorNextRaw { value, done: false })
            }
            ObjectIteratorStep::Throw(value) => {
                self.complete(NativeInvokeOutcome::Completion(Completion::Throw(value)))
            }
            ObjectIteratorStep::Done => {
                runtime.advance_iterator_concat(self.concat()?)?;
                self.advance(runtime)
            }
        }
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ConcatStep,
) -> Result<NativeInvokeOutcome, RuntimeError> {
    loop {
        step = match step {
            ConcatStep::Complete(result) => return Ok(result),
            ConcatStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                resume.resume(
                    runtime,
                    runtime.get_property_in_realm(realm, &object, &key)?,
                )?
            }
            ConcatStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                resume.resume(
                    runtime,
                    runtime.call_internal(realm, &callable, receiver, &[])?,
                )?
            }
            ConcatStep::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                resume.next(
                    runtime,
                    super::step::finish_next(
                        runtime,
                        realm,
                        super::step::NextStep::start(runtime, realm, iterator, method)?,
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct ConcatStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    next_iterator: Option<ObjectRef>,
    next_method: Option<Value>,
}
impl ConcatStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        Self::Call { resume }
    }
    pub(crate) fn request_next(
        iterator: ObjectRef,
        method: Value,
        mut resume: ConcatResume,
    ) -> Self {
        resume.0.pending_effect.next_iterator = Some(iterator);
        resume.0.pending_effect.next_method = Some(method);
        Self::Next { resume }
    }
}
impl ConcatResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ConcatStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ConcatStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("ConcatStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ConcatStep Call receiver")
    }
    pub(crate) fn take_next_iterator(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .next_iterator
            .take()
            .expect("ConcatStep Next iterator")
    }
    pub(crate) fn take_next_method(&mut self) -> Value {
        self.0
            .pending_effect
            .next_method
            .take()
            .expect("ConcatStep Next method")
    }
}
const _: () = assert!(std::mem::size_of::<ConcatStep>() <= 56);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ConcatStep>() <= 64);
