use super::*;
use crate::heap::{IteratorConcatData, IteratorConcatItem};

impl Runtime {
    pub(in crate::runtime) fn call_iterator_concat(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.concat did not receive a generic invocation",
            ));
        };

        // QuickJS validates every input and snapshots its @@iterator method
        // before creating the concat object. The iterator objects themselves
        // remain lazy and are created one at a time by `next`.
        let iterator_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
        let mut inputs = Vec::with_capacity(arguments.actual_arg_count);
        for input in &arguments.readable[..arguments.actual_arg_count] {
            let Value::Object(iterable) = input else {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            };
            let method = match self.get_property_in_realm(realm, iterable, &iterator_key)? {
                Completion::Return(method) => method,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if let NativeConversion::Throw(value) =
                self.iterator_callable_value(realm, method.clone())?
            {
                return Ok(Completion::Throw(value));
            }
            inputs.push((iterable.clone(), method));
        }

        Ok(Completion::Return(Value::Object(
            self.new_iterator_concat(realm, &inputs)?,
        )))
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

    pub(in crate::runtime) fn call_iterator_concat_next(
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

    pub(in crate::runtime) fn call_iterator_concat_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        let concat = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(concat) => concat,
            NativeConversion::Throw(value) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
            }
        };
        let snapshot = match self.iterator_concat_snapshot(realm, &concat)? {
            NativeConversion::Value(snapshot) => snapshot,
            NativeConversion::Throw(value) => {
                return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
            }
        };
        if snapshot.running {
            return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                self.new_native_error(realm, NativeErrorKind::Type, "already running")?,
            )));
        }
        self.set_iterator_concat_running(&concat, true)?;
        let outcome = self.resume_iterator_concat_next_raw(realm, &concat);
        let reset = self.set_iterator_concat_running(&concat, false);
        match outcome {
            Ok(outcome) => {
                reset?;
                Ok(outcome)
            }
            Err(error) => {
                reset?;
                Err(error)
            }
        }
    }

    fn resume_iterator_concat_next_raw(
        &self,
        realm: ContextId,
        concat: &ObjectRef,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        loop {
            let snapshot = {
                let state = self.0.state.borrow();
                state.heap.iterator_concat_state(concat.object_id())?
            };
            if snapshot.index >= snapshot.items.len() {
                return Ok(NativeInvokeOutcome::IteratorNextRaw {
                    value: Value::Undefined,
                    done: true,
                });
            }

            let iterator = if let Some(iterator) = snapshot.iterator {
                ObjectRef::from_borrowed_handle(self.clone(), iterator)?
            } else {
                let item = snapshot
                    .items
                    .get(snapshot.index)
                    .and_then(Option::as_ref)
                    .ok_or(RuntimeError::Invariant(
                        "Iterator Concat current input was already released",
                    ))?;
                let iterable = ObjectRef::from_borrowed_handle(self.clone(), item.iterable)?;
                let method = self.root_raw_value(&item.method)?;
                let callable = match self.iterator_callable_value(realm, method)? {
                    NativeConversion::Value(callable) => callable,
                    NativeConversion::Throw(_) => {
                        return Err(RuntimeError::Invariant(
                            "Iterator Concat captured method lost its callable brand",
                        ));
                    }
                };
                let result = self.call_internal(realm, &callable, Value::Object(iterable), &[])?;
                let iterator = match result {
                    Completion::Return(Value::Object(iterator)) => iterator,
                    Completion::Return(_) => {
                        return Ok(NativeInvokeOutcome::Completion(Completion::Throw(
                            self.new_native_error(realm, NativeErrorKind::Type, "not an object")?,
                        )));
                    }
                    Completion::Throw(value) => {
                        return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                    }
                };
                self.set_iterator_concat_iterator(concat, &iterator)?;
                iterator
            };

            let snapshot = {
                let state = self.0.state.borrow();
                state.heap.iterator_concat_state(concat.object_id())?
            };
            let next = if matches!(snapshot.next, RawValue::Undefined) {
                let key = self.intern_property_key("next")?;
                let next = match self.get_property_in_realm(realm, &iterator, &key)? {
                    Completion::Return(next) => next,
                    Completion::Throw(value) => {
                        return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                    }
                };
                self.set_iterator_concat_next(concat, &next)?;
                next
            } else {
                self.root_raw_value(&snapshot.next)?
            };

            match self.object_iterator_next(realm, &iterator, next.clone())? {
                ObjectIteratorStep::Yield(value) => {
                    return Ok(NativeInvokeOutcome::IteratorNextRaw { value, done: false });
                }
                ObjectIteratorStep::Throw(value) => {
                    return Ok(NativeInvokeOutcome::Completion(Completion::Throw(value)));
                }
                ObjectIteratorStep::Done => {
                    // Drop temporary roots before releasing the hidden-state
                    // edges in QuickJS's active iterator -> cached next ->
                    // captured method -> captured iterable order.
                    drop(next);
                    drop(iterator);
                    self.advance_iterator_concat(concat)?;
                }
            }
        }
    }

    pub(in crate::runtime) fn call_iterator_concat_return(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let concat = match self.iterator_receiver(realm, invocation)? {
            NativeConversion::Value(concat) => concat,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let snapshot = match self.iterator_concat_snapshot(realm, &concat)? {
            NativeConversion::Value(snapshot) => snapshot,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if snapshot.running {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "already running",
            )?));
        }

        let Some(iterator) = snapshot.iterator else {
            self.clear_iterator_concat(&concat)?;
            return Ok(Completion::Return(Value::Undefined));
        };
        let iterator = ObjectRef::from_borrowed_handle(self.clone(), iterator)?;
        let key = self.intern_property_key("return")?;
        self.set_iterator_concat_running(&concat, true)?;
        let method = match self.get_property_in_realm(realm, &iterator, &key) {
            Ok(Completion::Return(method)) => method,
            Ok(Completion::Throw(value)) => {
                self.set_iterator_concat_running(&concat, false)?;
                return Ok(Completion::Throw(value));
            }
            Err(error) => {
                self.set_iterator_concat_running(&concat, false)?;
                return Err(error);
            }
        };

        // Once the property access succeeds, QuickJS calls whatever value was
        // returned and then drains the whole state even when validation or the
        // call itself throws. The call result is forwarded without requiring
        // an iterator-result object.
        let call_result = (|| {
            let callable = match self.iterator_callable_value(realm, method)? {
                NativeConversion::Value(callable) => callable,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            self.call_internal(realm, &callable, Value::Object(iterator), &[])
        })();
        let reset = self.set_iterator_concat_running(&concat, false);
        let clear = self.clear_iterator_concat(&concat);
        match call_result {
            Ok(completion) => {
                reset?;
                clear?;
                Ok(completion)
            }
            Err(error) => {
                reset?;
                clear?;
                Err(error)
            }
        }
    }
}
