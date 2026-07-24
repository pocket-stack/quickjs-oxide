//! Async-from-Sync iterator acquisition and Promise continuation.
//!
//! Pinned QuickJS gives the fallback from `@@asyncIterator` to `@@iterator`
//! its own branded class. Each public resume method allocates its result
//! Promise before touching the receiver, calls the retained synchronous
//! iterator, intrinsically assimilates the iterator result's `value`, and
//! completes the outer capability through private Promise reactions.

use super::*;
use crate::heap::InternalCallableData;
use crate::runtime::intrinsics::promise::RootedPromiseCapability;

impl Runtime {
    pub(in crate::runtime) fn get_async_iterator_record(
        &self,
        realm: ContextId,
        iterable: Value,
    ) -> Result<NativeConversion<(Value, Value)>, RuntimeError> {
        let async_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::AsyncIterator));
        let async_method =
            match self.get_value_property_in_realm(realm, iterable.clone(), &async_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };

        let iterator = if matches!(async_method, Value::Undefined | Value::Null) {
            let sync_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Iterator));
            let sync_method =
                match self.get_value_property_in_realm(realm, iterable.clone(), &sync_key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
            let sync_method =
                match self.async_from_sync_callable(realm, sync_method, "not a function")? {
                    NativeConversion::Value(method) => method,
                    NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
            let sync_iterator = match self.call_internal(realm, &sync_method, iterable, &[])? {
                Completion::Return(Value::Object(iterator)) => iterator,
                Completion::Return(_) => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?));
                }
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            let next_key = self.intern_property_key("next")?;
            let next = match self.get_property_in_realm(realm, &sync_iterator, &next_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            Value::Object(self.new_async_from_sync_iterator(realm, &sync_iterator, &next)?)
        } else {
            let async_method = match self.async_from_sync_callable(
                realm,
                async_method,
                "value is not iterable",
            )? {
                NativeConversion::Value(method) => method,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            match self.call_internal(realm, &async_method, iterable, &[])? {
                Completion::Return(Value::Object(iterator)) => Value::Object(iterator),
                Completion::Return(_) => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "not an object",
                    )?));
                }
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        };

        let next_key = self.intern_property_key("next")?;
        let next = match self.get_value_property_in_realm(realm, iterator.clone(), &next_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        Ok(NativeConversion::Value((iterator, next)))
    }

    fn async_from_sync_callable(
        &self,
        realm: ContextId,
        value: Value,
        message: &'static str,
    ) -> Result<NativeConversion<CallableRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                message,
            )?));
        };
        let Some(callable) = self.as_callable(&object)? else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                message,
            )?));
        };
        Ok(NativeConversion::Value(callable))
    }

    fn new_async_from_sync_iterator(
        &self,
        realm: ContextId,
        sync_iterator: &ObjectRef,
        next: &Value,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .async_generator
            .ok_or(RuntimeError::Invariant(
                "realm has no AsyncGenerator intrinsics",
            ))?
            .async_from_sync_iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        let raw_next = self.raw_property_value(next)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let retained_atoms = match state.retain_raw_value_atoms(std::iter::once(&raw_next)) {
            Ok(atoms) => atoms,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error);
            }
        };
        let object = match state
            .heap
            .allocate_object(ObjectData::async_from_sync_iterator(
                shape,
                Vec::new(),
                sync_iterator.object_id(),
                raw_next,
            )) {
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

    pub(in crate::runtime) fn call_async_from_sync_iterator_resume(
        &self,
        realm: ContextId,
        kind: GeneratorResumeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let capability = self.new_default_promise_capability(realm)?;
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Async-from-Sync resume did not receive a call invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Async-from-Sync resume argv was not padded",
            ))?;
        let receiver = match this_value {
            Value::Object(receiver) => receiver,
            _ => {
                let reason = self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an Async-from-Sync Iterator",
                )?;
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
        };
        let state = self
            .0
            .state
            .borrow()
            .heap
            .async_from_sync_iterator_state(receiver.object_id());
        let (sync_iterator, cached_next) = match state {
            Ok(state) => state,
            Err(HeapError::Invariant(_)) => {
                let reason = self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an Async-from-Sync Iterator",
                )?;
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
            Err(error) => return Err(error.into()),
        };
        let sync_iterator = ObjectRef::from_borrowed_handle(self.clone(), sync_iterator)?;

        let method = match kind {
            GeneratorResumeKind::Next => self.root_raw_value(&cached_next)?,
            GeneratorResumeKind::Return | GeneratorResumeKind::Throw => {
                let name = match kind {
                    GeneratorResumeKind::Return => "return",
                    GeneratorResumeKind::Throw => "throw",
                    GeneratorResumeKind::Next => unreachable!(),
                };
                let key = self.intern_property_key(name)?;
                match self.get_property_in_realm(realm, &sync_iterator, &key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(reason) => {
                        return self.reject_async_from_sync_capability(realm, capability, reason);
                    }
                }
            }
        };

        if matches!(method, Value::Undefined | Value::Null) {
            return match kind {
                GeneratorResumeKind::Return => {
                    let result = Value::Object(self.new_iterator_result(realm, argument, true)?);
                    self.resolve_async_from_sync_capability(realm, capability, result)
                }
                GeneratorResumeKind::Throw => {
                    match self.close_async_from_sync_iterator_normally(realm, &sync_iterator)? {
                        NativeConversion::Value(()) => {
                            let reason = self.new_native_error(
                                realm,
                                NativeErrorKind::Type,
                                "throw is not a method",
                            )?;
                            self.reject_async_from_sync_capability(realm, capability, reason)
                        }
                        NativeConversion::Throw(reason) => {
                            self.reject_async_from_sync_capability(realm, capability, reason)
                        }
                    }
                }
                GeneratorResumeKind::Next => {
                    let reason =
                        self.new_native_error(realm, NativeErrorKind::Type, "not a function")?;
                    self.reject_async_from_sync_capability(realm, capability, reason)
                }
            };
        }

        let method = match self.async_from_sync_callable(realm, method, "not a function")? {
            NativeConversion::Value(method) => method,
            NativeConversion::Throw(reason) => {
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
        };
        let call_arguments = if arguments.actual_arg_count == 0 {
            &[][..]
        } else {
            std::slice::from_ref(&argument)
        };
        let result = match self.call_internal(
            realm,
            &method,
            Value::Object(sync_iterator.clone()),
            call_arguments,
        )? {
            Completion::Return(value) => value,
            Completion::Throw(reason) => {
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
        };
        let Value::Object(result) = result else {
            let reason = self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "iterator must return an object",
            )?;
            return self.reject_async_from_sync_capability(realm, capability, reason);
        };

        let done_key = self.intern_property_key("done")?;
        let done = match self.get_property_in_realm(realm, &result, &done_key)? {
            Completion::Return(value) => value.to_boolean(),
            Completion::Throw(reason) => {
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
        };
        // Unlike ordinary IteratorStep, AsyncFromSyncIteratorContinuation
        // observes `value` even when `done` is already true.
        let value_key = self.intern_property_key("value")?;
        let value = match self.get_property_in_realm(realm, &result, &value_key)? {
            Completion::Return(value) => value,
            Completion::Throw(reason) => {
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
        };

        let value_promise = match self.promise_resolve_intrinsic(realm, value)? {
            Completion::Return(Value::Object(promise)) => promise,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "intrinsic PromiseResolve returned a non-object",
                ));
            }
            Completion::Throw(reason) => {
                if kind != GeneratorResumeKind::Return && !done {
                    self.close_iterator_preserving_throw(realm, &sync_iterator)?;
                }
                return self.reject_async_from_sync_capability(realm, capability, reason);
            }
        };

        let unwrap = self.new_internal_promise_function(
            realm,
            NativeFunctionId::AsyncFromSyncIteratorUnwrap,
            1,
            1,
            InternalCallableData::AsyncFromSyncIteratorUnwrap { done },
        )?;
        let close = if kind != GeneratorResumeKind::Return && !done {
            Some(self.new_internal_promise_function(
                realm,
                NativeFunctionId::AsyncFromSyncIteratorClose,
                1,
                1,
                InternalCallableData::AsyncFromSyncIteratorClose {
                    sync_iterator: sync_iterator.object_id(),
                },
            )?)
        } else {
            None
        };
        self.perform_promise_then_with_capability(
            realm,
            &value_promise,
            Some(&unwrap),
            close.as_ref(),
            &capability,
        )?;
        Ok(Completion::Return(Value::Object(capability.promise)))
    }

    fn close_async_from_sync_iterator_normally(
        &self,
        realm: ContextId,
        iterator: &ObjectRef,
    ) -> Result<NativeConversion<()>, RuntimeError> {
        let key = self.intern_property_key("return")?;
        let method = match self.get_property_in_realm(realm, iterator, &key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(method, Value::Undefined | Value::Null) {
            return Ok(NativeConversion::Value(()));
        }
        let method = match self.async_from_sync_callable(realm, method, "not a function")? {
            NativeConversion::Value(method) => method,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        match self.call_internal(realm, &method, Value::Object(iterator.clone()), &[])? {
            Completion::Return(Value::Object(_)) => Ok(NativeConversion::Value(())),
            Completion::Return(_) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?)),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
        }
    }

    fn resolve_async_from_sync_capability(
        &self,
        realm: ContextId,
        capability: RootedPromiseCapability,
        value: Value,
    ) -> Result<Completion, RuntimeError> {
        self.settle_async_from_sync_capability(realm, capability, true, value)
    }

    fn reject_async_from_sync_capability(
        &self,
        realm: ContextId,
        capability: RootedPromiseCapability,
        reason: Value,
    ) -> Result<Completion, RuntimeError> {
        self.settle_async_from_sync_capability(realm, capability, false, reason)
    }

    fn settle_async_from_sync_capability(
        &self,
        realm: ContextId,
        capability: RootedPromiseCapability,
        resolve: bool,
        value: Value,
    ) -> Result<Completion, RuntimeError> {
        let promise = capability.promise.clone();
        let target = if resolve {
            &capability.resolve
        } else {
            &capability.reject
        };
        match self.call_internal(realm, target, Value::Undefined, &[value])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Object(promise))),
            Completion::Throw(_) => Err(RuntimeError::Invariant(
                "intrinsic Promise resolving function threw",
            )),
        }
    }

    pub(in crate::runtime) fn call_async_from_sync_iterator_unwrap(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Async-from-Sync unwrap did not receive a call invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Async-from-Sync unwrap had no internal capture",
            ))?;
        let InternalCallableData::AsyncFromSyncIteratorUnwrap { done } = internal else {
            return Err(RuntimeError::Invariant(
                "Async-from-Sync unwrap had the wrong internal capture",
            ));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Async-from-Sync unwrap argv was not padded",
            ))?;
        Ok(Completion::Return(Value::Object(
            self.new_iterator_result(realm, value, done)?,
        )))
    }

    pub(in crate::runtime) fn call_async_from_sync_iterator_close(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Async-from-Sync close did not receive a call invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Async-from-Sync close had no internal capture",
            ))?;
        let InternalCallableData::AsyncFromSyncIteratorClose { sync_iterator } = internal else {
            return Err(RuntimeError::Invariant(
                "Async-from-Sync close had the wrong internal capture",
            ));
        };
        let reason = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Async-from-Sync close argv was not padded",
            ))?;
        let sync_iterator = ObjectRef::from_borrowed_handle(self.clone(), sync_iterator)?;
        self.close_iterator_preserving_throw(realm, &sync_iterator)?;
        Ok(Completion::Throw(reason))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::PromiseState;

    fn acquire(runtime: &Runtime, context: &mut Context, source: &str) -> (ObjectRef, CallableRef) {
        let iterable = context.eval(source).unwrap();
        let NativeConversion::Value((Value::Object(iterator), next)) = runtime
            .get_async_iterator_record(context.realm, iterable)
            .unwrap()
        else {
            panic!("GetAsyncIterator did not produce an iterator record");
        };
        let Value::Object(next) = next else {
            panic!("Async iterator next was not an object");
        };
        let next = runtime
            .as_callable(&next)
            .unwrap()
            .expect("Async iterator next was not callable");
        (iterator, next)
    }

    fn method(
        runtime: &Runtime,
        context: &mut Context,
        object: &ObjectRef,
        name: &str,
    ) -> CallableRef {
        let key = runtime.intern_property_key(name).unwrap();
        let Value::Object(method) = context.get_property(object, &key).unwrap() else {
            panic!("Async iterator {name} was not an object");
        };
        runtime
            .as_callable(&method)
            .unwrap()
            .unwrap_or_else(|| panic!("Async iterator {name} was not callable"))
    }

    fn promise(value: Value) -> ObjectRef {
        let Value::Object(promise) = value else {
            panic!("Async-from-Sync method did not return a Promise");
        };
        promise
    }

    fn promise_result(runtime: &Runtime, promise: &ObjectRef) -> (PromiseState, Value) {
        let snapshot = runtime
            .0
            .state
            .borrow()
            .heap
            .promise_snapshot(promise.object_id())
            .unwrap();
        (
            snapshot.state,
            runtime.root_raw_value(&snapshot.result).unwrap(),
        )
    }

    fn drain_jobs(runtime: &Runtime) {
        while runtime.execute_pending_job().unwrap() {}
    }

    fn property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> Value {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(object, &key).unwrap()
    }

    #[test]
    fn get_async_iterator_uses_nullish_fallback_and_branded_wrapper() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, _next) = acquire(
            &runtime,
            &mut context,
            r#"
                globalThis.trace = "";
                ({
                    get [Symbol.asyncIterator]() {
                        trace += "a";
                        return null;
                    },
                    get [Symbol.iterator]() {
                        trace += "s";
                        return function () {
                            trace += "i";
                            return {
                                get next() {
                                    trace += "n";
                                    return function () { return { done: true }; };
                                }
                            };
                        };
                    }
                })
            "#,
        );
        assert_eq!(
            context.eval("trace").unwrap(),
            Value::String(JsString::from_static("asin"))
        );
        assert!(matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(iterator.object_id())
                .unwrap()
                .payload,
            ObjectPayload::AsyncFromSyncIterator(_)
        ));
    }

    #[test]
    fn async_from_sync_distinguishes_argument_count_and_reads_done_then_value() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, next) = acquire(
            &runtime,
            &mut context,
            r#"
                globalThis.trace = "";
                Promise.resolve = function () { trace += "bad-resolve:"; };
                Promise.prototype.then = function () { trace += "bad-then:"; };
                ({
                    [Symbol.iterator]() {
                        return {
                            next(value) {
                                trace += arguments.length + ":";
                                return {
                                    get done() { trace += "done:"; return true; },
                                    get value() { trace += "value"; return 9; }
                                };
                            }
                        };
                    }
                })
            "#,
        );

        let first = promise(
            context
                .call(&next, Value::Object(iterator.clone()), &[])
                .unwrap(),
        );
        drain_jobs(&runtime);
        let (state, Value::Object(result)) = promise_result(&runtime, &first) else {
            panic!("first Async-from-Sync result was not an object");
        };
        assert_eq!(state, PromiseState::Fulfilled);
        assert_eq!(
            property(&runtime, &mut context, &result, "done"),
            Value::Bool(true)
        );
        assert_eq!(
            property(&runtime, &mut context, &result, "value"),
            Value::Int(9)
        );
        assert_eq!(
            context.eval("trace").unwrap(),
            Value::String(JsString::from_static("0:done:value"))
        );

        let trace_key = runtime.intern_property_key("trace").unwrap();
        let global = context.global_object().unwrap();
        assert!(
            context
                .set_property(
                    &global,
                    &trace_key,
                    Value::String(JsString::from_static("")),
                )
                .unwrap()
        );
        let second = promise(
            context
                .call(&next, Value::Object(iterator), &[Value::Undefined])
                .unwrap(),
        );
        drain_jobs(&runtime);
        assert_eq!(promise_result(&runtime, &second).0, PromiseState::Fulfilled);
        assert_eq!(
            context.eval("trace").unwrap(),
            Value::String(JsString::from_static("1:done:value"))
        );
    }

    #[test]
    fn missing_return_resolves_directly_without_assimilating_argument() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, _next) = acquire(
            &runtime,
            &mut context,
            r#"
                globalThis.touched = false;
                ({ [Symbol.iterator]() { return { next() { return { done: true }; } }; } })
            "#,
        );
        let return_method = method(&runtime, &mut context, &iterator, "return");
        let argument = context
            .eval(
                r#"({
                    get then() {
                        touched = true;
                        return function () {};
                    }
                })"#,
            )
            .unwrap();
        let result_promise = promise(
            context
                .call(
                    &return_method,
                    Value::Object(iterator),
                    std::slice::from_ref(&argument),
                )
                .unwrap(),
        );
        let (state, Value::Object(result)) = promise_result(&runtime, &result_promise) else {
            panic!("missing return did not resolve an iterator result");
        };
        assert_eq!(state, PromiseState::Fulfilled);
        assert_eq!(
            property(&runtime, &mut context, &result, "done"),
            Value::Bool(true)
        );
        assert!(property(&runtime, &mut context, &result, "value").same_value(&argument));
        assert_eq!(context.eval("touched").unwrap(), Value::Bool(false));
    }

    #[test]
    fn receiver_and_synchronous_failures_return_rejected_promises() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, next) = acquire(
            &runtime,
            &mut context,
            r#"({ [Symbol.iterator]() { return { next() { throw "sync"; } }; } })"#,
        );

        let wrong_receiver = promise(context.call(&next, Value::Undefined, &[]).unwrap());
        assert_eq!(
            promise_result(&runtime, &wrong_receiver).0,
            PromiseState::Rejected
        );

        let sync_throw = promise(context.call(&next, Value::Object(iterator), &[]).unwrap());
        assert_eq!(
            promise_result(&runtime, &sync_throw),
            (
                PromiseState::Rejected,
                Value::String(JsString::from_static("sync"))
            )
        );
    }

    #[test]
    fn rejected_value_closes_non_return_and_preserves_original_reason_across_gc() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, next) = acquire(
            &runtime,
            &mut context,
            r#"
                globalThis.closed = 0;
                ({
                    [Symbol.iterator]() {
                        return {
                            next() {
                                return { done: false, value: Promise.reject("original") };
                            },
                            return() {
                                closed++;
                                throw "close";
                            }
                        };
                    }
                })
            "#,
        );
        let result_promise = promise(
            context
                .call(&next, Value::Object(iterator.clone()), &[])
                .unwrap(),
        );
        drop(iterator);
        drop(next);
        runtime.run_gc().unwrap();
        drain_jobs(&runtime);
        assert_eq!(
            promise_result(&runtime, &result_promise),
            (
                PromiseState::Rejected,
                Value::String(JsString::from_static("original"))
            )
        );
        assert_eq!(context.eval("closed").unwrap(), Value::Int(1));
    }

    #[test]
    fn return_mode_and_completed_steps_do_not_close_rejected_values() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, next) = acquire(
            &runtime,
            &mut context,
            r#"
                globalThis.returns = 0;
                ({
                    [Symbol.iterator]() {
                        return {
                            next() {
                                return { done: true, value: Promise.reject("done") };
                            },
                            return() {
                                returns++;
                                return { done: false, value: Promise.reject("return") };
                            }
                        };
                    }
                })
            "#,
        );

        let done_promise = promise(
            context
                .call(&next, Value::Object(iterator.clone()), &[])
                .unwrap(),
        );
        drain_jobs(&runtime);
        assert_eq!(
            promise_result(&runtime, &done_promise),
            (
                PromiseState::Rejected,
                Value::String(JsString::from_static("done"))
            )
        );
        assert_eq!(context.eval("returns").unwrap(), Value::Int(0));

        let return_method = method(&runtime, &mut context, &iterator, "return");
        let return_promise = promise(
            context
                .call(&return_method, Value::Object(iterator), &[])
                .unwrap(),
        );
        drain_jobs(&runtime);
        assert_eq!(
            promise_result(&runtime, &return_promise),
            (
                PromiseState::Rejected,
                Value::String(JsString::from_static("return"))
            )
        );
        assert_eq!(context.eval("returns").unwrap(), Value::Int(1));
    }

    #[test]
    fn missing_throw_closes_normally_before_rejecting() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (iterator, _next) = acquire(
            &runtime,
            &mut context,
            r#"
                globalThis.closed = 0;
                ({
                    [Symbol.iterator]() {
                        return {
                            next() { return { done: false, value: 1 }; },
                            return() { closed++; return {}; }
                        };
                    }
                })
            "#,
        );
        let throw_method = method(&runtime, &mut context, &iterator, "throw");
        let result_promise = promise(
            context
                .call(&throw_method, Value::Object(iterator), &[Value::Int(7)])
                .unwrap(),
        );
        assert_eq!(
            promise_result(&runtime, &result_promise).0,
            PromiseState::Rejected
        );
        assert_eq!(context.eval("closed").unwrap(), Value::Int(1));
    }
}
