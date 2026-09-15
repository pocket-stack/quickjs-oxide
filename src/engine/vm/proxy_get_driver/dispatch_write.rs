//! Bounded native-stack dispatch for write requests.
use super::{
    Completion, DirectCallTarget, Error, Finish, NativeConversion, Next, Query, Resume,
    ReturnOwner, RunningExecution, Runtime, Step, overflow, runtime_error_to_vm_error,
};

#[inline(never)]
pub(super) fn keys(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    _owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("dispatch_write.keys.visit");
        let realm = query.realm;
        match &mut *step {
            Step::SnapshotEnumerable {
                object,
                key,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if runtime
                    .is_proxy_object(&object)
                    .map_err(runtime_error_to_vm_error)?
                {
                    *step = Step::Descriptor {
                        object: Some(object),
                        key: Some(key),
                        resume: Some(Resume::OwnFlagReply {
                            enumerable: true,
                            resume: Box::new(resume),
                        }),
                    };
                } else {
                    let result = runtime
                        .internal_snapshot_own_property_is_enumerable(realm, &object, &key)
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .boolean(runtime, result)
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            Step::OwnFlag {
                object,
                key,
                enumerable,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let enumerable = enumerable.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if runtime
                    .is_proxy_object(&object)
                    .map_err(runtime_error_to_vm_error)?
                {
                    *step = Step::Descriptor {
                        object: Some(object),
                        key: Some(key),
                        resume: Some(Resume::OwnFlagReply {
                            enumerable,
                            resume: Box::new(resume),
                        }),
                    };
                } else {
                    let result = if enumerable {
                        runtime.internal_own_property_is_enumerable(realm, &object, &key)
                    } else {
                        runtime.internal_has_own_property(realm, &object, &key)
                    }
                    .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .boolean(runtime, result)
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            Step::Keys { object, resume } => {
                let object = object.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if runtime
                    .is_proxy_object(&object)
                    .map_err(runtime_error_to_vm_error)?
                {
                    if !execution
                        .frames
                        .can_push_with_continuations(query.continuation_depth())
                    {
                        let Completion::Throw(value) = overflow(runtime, realm)? else {
                            unreachable!()
                        };
                        *step = resume
                            .keys(runtime, NativeConversion::Throw(value))
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("ownKeys continuation allocation failed"))?;
                    query.parents.push(resume);
                    *step = crate::engine::object::KeysStep::start(runtime, realm, object)
                        .map_err(runtime_error_to_vm_error)?
                        .into();
                } else {
                    let result = runtime
                        .own_property_keys(&object)
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .keys(runtime, NativeConversion::Value(result))
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            Step::KeysComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("ownKeys result has no parent"))?;
                *step = resume
                    .keys(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::ReadValue {
                receiver,
                key,
                resume,
            } => {
                let receiver = receiver.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                *step = match runtime
                    .prepare_value_property_read_completion(realm, receiver, &key)
                    .map_err(runtime_error_to_vm_error)?
                {
                    NativeConversion::Value(read) => Step::PreparedRead {
                        read: Some(read),
                        key: Some(key),
                        resume: Some(resume),
                    },
                    NativeConversion::Throw(reason) => resume
                        .resume(runtime, Completion::Throw(reason))
                        .map_err(runtime_error_to_vm_error)?,
                };
                continue;
            }
            _ => {
                return Ok(Next::Continue);
            }
        }
    }
}

#[inline(never)]
pub(super) fn set(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    _owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("dispatch_write.set.visit");
        let realm = query.realm;
        match &mut *step {
            Step::PreparedSet {
                step: selected,
                resume,
            } => {
                let selected = selected.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if !execution
                    .frames
                    .can_push_with_continuations(query.continuation_depth())
                {
                    let Completion::Throw(value) = overflow(runtime, realm)? else {
                        unreachable!()
                    };
                    *step = resume
                        .set(
                            runtime,
                            crate::engine::object::operations::PropertySetAction::Throw(value),
                        )
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("property continuation allocation failed"))?;
                query.parents.push(resume);
                *step = (*selected).into();
                continue;
            }
            Step::SetContinue(resume) => {
                let resume = resume.take().expect("selected Step field");

                *step = resume
                    .advance(runtime)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::SetLength { value, resume } => {
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("property continuation allocation failed"))?;
                query.parents.push(Resume::SetLength(resume));
                *step = crate::engine::object::ArrayLengthStep::start(runtime, Some(realm), value)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::SetSpecial {
                object,
                key,
                value,
                receiver,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let value = value.take().expect("selected Step field");
                let receiver = receiver.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                match runtime
                    .prepare_typed_array_set(&object, &key, &value, &receiver)
                    .map_err(runtime_error_to_vm_error)?
                {
                    None => {
                        *step = resume
                            .special(runtime, None)
                            .map_err(runtime_error_to_vm_error)?
                            .into()
                    }
                    Some(request) => {
                        query.parents.try_reserve(1).map_err(|_| {
                            Error::internal("property continuation allocation failed")
                        })?;
                        query.parents.push(Resume::SetTyped(resume));
                        *step = request.into();
                    }
                }
                continue;
            }
            Step::SetComplete(action) => {
                let action = action.take().expect("selected Step field");

                if let crate::engine::object::operations::PropertySetAction::Call { payload } =
                    action
                {
                    let crate::engine::object::operations::PropertySetterCall {
                        setter,
                        receiver,
                        argument,
                    } = *payload;

                    *step = Step::Call {
                        target: Some(DirectCallTarget::Callable(setter)),
                        receiver: Some(receiver),
                        arguments: Some(vec![argument]),
                        resume: Some(Resume::Setter),
                    };
                    continue;
                }
                if let Some(resume) = query.parents.pop() {
                    *step = resume
                        .set(runtime, action)
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                let Some(Finish::Write { key, strict, .. }) = query.finish.as_ref() else {
                    return Err(Error::internal("Set result has no assignment owner"));
                };
                *step = Step::Complete(Some(
                    runtime
                        .finish_property_set(
                            super::request::set_result(action)
                                .map_err(runtime_error_to_vm_error)?,
                            key,
                            *strict,
                        )
                        .map_err(runtime_error_to_vm_error)?,
                ));
                continue;
            }
            Step::Set {
                object,
                key,
                value,
                receiver,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let value = value.take().expect("selected Step field");
                let receiver = receiver.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if !execution
                    .frames
                    .can_push_with_continuations(query.continuation_depth())
                {
                    let Completion::Throw(value) = overflow(runtime, realm)? else {
                        unreachable!()
                    };
                    *step = resume
                        .set(
                            runtime,
                            crate::engine::object::operations::PropertySetAction::Throw(value),
                        )
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("property continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::object::SetStep::start(
                    runtime,
                    Some(realm),
                    object,
                    key,
                    value,
                    receiver,
                )
                .map_err(runtime_error_to_vm_error)?
                // The budget check and parent reservation above must precede
                // any storage effects. Only shared no-callback phases advance;
                // selected setters and exotic waits keep their exact state.
                .advance_without_callback(runtime)
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::SetProxy {
                object,
                key,
                value,
                receiver,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let value = value.take().expect("selected Step field");
                let receiver = receiver.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if !execution
                    .frames
                    .can_push_with_continuations(query.continuation_depth())
                {
                    let Completion::Throw(value) = overflow(runtime, realm)? else {
                        unreachable!()
                    };
                    *step = resume
                        .set(
                            runtime,
                            crate::engine::object::operations::PropertySetAction::Throw(value),
                        )
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("property continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::object::ProxySetStep::start(
                    runtime, realm, object, key, value, receiver,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            _ => {
                return Ok(Next::Continue);
            }
        }
    }
}

#[inline(never)]
pub(super) fn define(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    _owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        {
            crate::engine::api::profiling::record_owned_execution_event(
                "dispatch_write.define.visit",
            );
            crate::engine::api::profiling::record_owned_execution_event(match &step {
                Step::Define { .. } => "dispatch_write.define.stage.request",
                Step::DefineOrdinary { .. } => "dispatch_write.define.stage.ordinary",
                Step::Defined { .. } => "dispatch_write.define.stage.reply",
                _ => "dispatch_write.define.stage.leave",
            });
        }
        let realm = query.realm;
        match &mut *step {
            Step::Defined(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("Define result has no parent"))?;
                *step = resume
                    .defined(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::Define {
                object,
                key,
                descriptor,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let descriptor = descriptor.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if runtime
                    .is_proxy_object(&object)
                    .map_err(runtime_error_to_vm_error)?
                {
                    if !execution
                        .frames
                        .can_push_with_continuations(query.continuation_depth())
                    {
                        let Completion::Throw(value) = overflow(runtime, realm)? else {
                            unreachable!()
                        };
                        *step = resume
                            .defined(runtime, NativeConversion::Throw(value))
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(resume);
                    *step = crate::engine::object::ProxyDefineStep::start(
                        runtime, realm, object, key, descriptor,
                    )
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                    continue;
                }
                *step = Step::DefineOrdinary {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(resume),
                };
                continue;
            }
            Step::DefineOrdinary {
                object,
                key,
                descriptor,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let descriptor = descriptor.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if let Some(length) = runtime
                    .prepare_array_length_definition(Some(realm), &object, &key, &descriptor)
                    .map_err(runtime_error_to_vm_error)?
                {
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(Resume::DefineLength {
                        payload: Box::new(super::request::DefineLengthPayload {
                            object,
                            key,
                            descriptor,
                            resume: Box::new(resume),
                        }),
                    });
                    *step = length.into();
                    continue;
                }
                if let Some(request) = runtime
                    .prepare_typed_array_definition(&object, &key, &descriptor)
                    .map_err(runtime_error_to_vm_error)?
                {
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(Resume::DefineTyped {
                        payload: Box::new(super::request::DefineTypedPayload {
                            object,
                            _descriptor: descriptor,
                            resume: Box::new(resume),
                        }),
                    });
                    *step = request.into();
                    continue;
                }
                let result = match runtime
                        .define_own_property_in_realm(Some(realm), &object, &key, &descriptor)
                        .map_err(runtime_error_to_vm_error)? {
                        crate::engine::object::operations::PropertyDefineOutcome::Defined(true) => NativeConversion::Value(crate::engine::object::operations::InternalDefineResult::Defined),
                        crate::engine::object::operations::PropertyDefineOutcome::Defined(false) => NativeConversion::Value(crate::engine::object::operations::InternalDefineResult::RejectedOrdinary(object)),
                        crate::engine::object::operations::PropertyDefineOutcome::Throw(value) => NativeConversion::Throw(value),
                    };
                *step = resume
                    .defined(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            _ => {
                return Ok(Next::Continue);
            }
        }
    }
}

#[cfg(test)]
mod local_set_tests {
    use super::super::Parents;
    use super::*;
    use crate::engine::api::Value;
    use crate::engine::vm::execution::ExecutionLimits;

    #[test]
    fn local_set_dispatch_budget_failure_precedes_array_write() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(array) = context.eval("[]").unwrap() else {
            panic!("expected array");
        };
        let key = runtime.property_key_for_index(0).unwrap();
        let mut query = Query {
            #[cfg(feature = "profiling")]
            had_callback: false,
            realm: context.realm,
            parents: Parents::default(),
            natives: Vec::new(),
            saved_native_depth: 0,
            spare_parents: Vec::new(),
            finish: None,
        };
        let mut pending = Step::Set {
            object: Some(array.clone()),
            key: Some(key.clone()),
            value: Some(Value::Int(7)),
            receiver: Some(Value::Object(array.clone())),
            resume: Some(Resume::RootSet),
        };
        let mut execution = RunningExecution::new(
            &runtime,
            ExecutionLimits {
                frames: 0,
                slots: 0,
            },
        )
        .unwrap();
        assert!(matches!(
            set(
                &runtime,
                &mut execution,
                ReturnOwner::Root,
                1,
                &mut query,
                &mut pending
            )
            .unwrap(),
            Next::Continue
        ));
        assert!(matches!(
            pending,
            Step::Complete(Some(Completion::Throw(_)))
        ));
        assert!(query.parents.is_empty());
        assert!(runtime.get_own_property(&array, &key).unwrap().is_none());
        assert_eq!(
            runtime.array_length_state_if_genuine(&array).unwrap(),
            Some((0, true))
        );
    }

    #[test]
    fn local_set_dispatch_keeps_setters_proxy_and_array_length_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(function () {
            var setterCalls = 0, setterLength = -1, stored;
            var proto = Object.create(Array.prototype);
            Object.defineProperty(proto, '0', { set: function (value) {
                setterCalls++; setterLength = this.length; stored = value;
            }});
            var a = []; Object.setPrototypeOf(a, proto);
            if (a.push(7) !== 1 || setterCalls !== 1 || setterLength !== 0 ||
                stored !== 7 || Object.hasOwn(a, '0')) return false;
            var trace = [], target = [];
            var proxy = new Proxy(target, {set: function (t, k, v, r) {
                trace.push(k + ':' + t.length); return Reflect.set(t, k, v, r);
            }});
            if (Array.prototype.push.call(proxy, 9) !== 1 || target[0] !== 9 ||
                trace.join(',') !== '0:0,length:1') return false;
            var b = [];
            Object.defineProperty(b, 'length', {writable: false});
            var rejected = false;
            try { b.push(1); } catch (e) { rejected = e instanceof TypeError; }
            if (!rejected || b.length !== 0 || Object.hasOwn(b, '0')) return false;
            var order = '', length = 0, receiver = {
                get length() { order += 'g'; return length; },
                set length(v) { order += 'l'; length = v; },
                set 0(v) { order += 'a'; },
                set 1(v) { order += 'b'; throw 23; }
            };
            var failure;
            try { Array.prototype.push.call(receiver, 1, 2); } catch(e) { failure = e; }
            return failure === 23 && order === 'gab' && length === 0;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}
