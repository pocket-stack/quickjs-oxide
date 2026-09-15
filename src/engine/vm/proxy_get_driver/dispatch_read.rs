//! Bounded native-stack dispatch for read requests.
use super::{
    Completion, DescriptorStep, DirectCallTarget, Error, NativeConversion, Next, OrdinaryRead,
    PreparedHas, ProxyBooleanKind, ProxyBooleanStep, ProxyGetStep, ProxyOwnStep,
    ProxyPrototypeKind, ProxyPrototypeStep, Query, Resume, ReturnOwner, RunningExecution, Runtime,
    Step, Value, overflow, runtime_error_to_vm_error,
};

#[inline(never)]
pub(super) fn prototype(
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
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_read.prototype.visit",
        );
        let realm = query.realm;
        match &mut *step {
            Step::OwnComplete(descriptor) => {
                let descriptor = descriptor.take().expect("selected Step field");

                let parent = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("descriptor reply has no parent operation"))?;
                *step = parent
                    .descriptor(runtime, descriptor)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::BooleanComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("boolean reply has no parent operation"))?;
                *step = resume
                    .boolean(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::GetPrototype { object, resume } => {
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
                            .prototype(runtime, NativeConversion::Throw(value))
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query
                        .parents
                        .push(Resume::PrototypeGetReply(Box::new(resume)));
                    *step =
                        ProxyPrototypeStep::start(runtime, realm, object, ProxyPrototypeKind::Get)
                            .map_err(runtime_error_to_vm_error)?
                            .into();
                } else {
                    let result = runtime
                        .get_prototype_of(&object)
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .prototype(runtime, NativeConversion::Value(result))
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            Step::SetPrototype {
                object,
                prototype,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let prototype = prototype.take().expect("selected Step field");
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
                            .boolean(runtime, NativeConversion::Throw(value))
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query
                        .parents
                        .push(Resume::PrototypeSetReply(Box::new(resume)));
                    *step = ProxyPrototypeStep::start(
                        runtime,
                        realm,
                        object,
                        ProxyPrototypeKind::Set(prototype),
                    )
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                } else {
                    let result = runtime
                        .set_prototype_of(&object, prototype.as_ref())
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .boolean(runtime, NativeConversion::Value(result))
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            _ => return Ok(Next::Continue),
        }
    }
}

#[inline(never)]
pub(super) fn attributes(
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
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_read.attributes.visit",
        );
        let realm = query.realm;
        match &mut *step {
            Step::Delete {
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
                    if !execution
                        .frames
                        .can_push_with_continuations(query.continuation_depth())
                    {
                        let Completion::Throw(value) = overflow(runtime, realm)? else {
                            unreachable!()
                        };
                        *step = resume
                            .boolean(runtime, NativeConversion::Throw(value))
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(resume);
                    *step = ProxyBooleanStep::start(
                        runtime,
                        realm,
                        object,
                        ProxyBooleanKind::Delete(key),
                    )
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                } else {
                    let result = runtime
                        .delete_property(&object, &key)
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .boolean(runtime, NativeConversion::Value(result))
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            Step::PreventExtensions { object, resume } => {
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
                            .boolean(runtime, NativeConversion::Throw(value))
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(resume);
                    *step = ProxyBooleanStep::start(
                        runtime,
                        realm,
                        object,
                        ProxyBooleanKind::PreventExtensions,
                    )
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                } else {
                    let result = runtime
                        .internal_prevent_extensions(realm, &object)
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .boolean(runtime, result)
                        .map_err(runtime_error_to_vm_error)?;
                }
                continue;
            }
            Step::Extensible { object, resume } => {
                let object = object.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if runtime
                    .is_proxy_object(&object)
                    .map_err(runtime_error_to_vm_error)?
                {
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(resume);
                    *step = ProxyBooleanStep::start(
                        runtime,
                        realm,
                        object,
                        ProxyBooleanKind::Extensible,
                    )
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                    continue;
                }
                let result = runtime
                    .is_extensible(&object)
                    .map_err(runtime_error_to_vm_error)?;
                *step = resume
                    .boolean(runtime, NativeConversion::Value(result))
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            _ => return Ok(Next::Continue),
        }
    }
}

#[inline(never)]
pub(super) fn get(
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
        crate::engine::api::profiling::record_owned_execution_event("dispatch_read.get.visit");
        let realm = query.realm;
        let (target, receiver, arguments, resume) = match &mut *step {
            Step::Convert { value, resume } => {
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("property continuation allocation failed"))?;
                query.parents.push(resume);
                *step = DescriptorStep::start(runtime, realm, value)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::Converted(result) => {
                let result = result.take().expect("selected Step field");

                let Some(resume) = query.parents.pop() else {
                    return Err(Error::internal(
                        "converted descriptor has no matching property operation",
                    ));
                };
                *step = resume
                    .converted(runtime, result)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::Has {
                object,
                key,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                let probe = runtime
                    .prepare_has_property(&object, &key)
                    .map_err(runtime_error_to_vm_error)?;
                *step = Step::PreparedHas {
                    probe: Some(probe),
                    key: Some(key),
                    resume: Some(resume),
                };
                continue;
            }
            Step::PreparedHas { probe, key, resume } => {
                let probe = probe.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                match probe {
                    PreparedHas::Complete(value) => {
                        *step = resume
                            .boolean(runtime, NativeConversion::Value(value))
                            .map_err(runtime_error_to_vm_error)?
                    }
                    PreparedHas::Proxy(object) => {
                        query.parents.try_reserve(1).map_err(|_| {
                            Error::internal("property continuation allocation failed")
                        })?;
                        query.parents.push(resume);
                        *step = ProxyBooleanStep::start(
                            runtime,
                            realm,
                            object,
                            ProxyBooleanKind::Has(key),
                        )
                        .map_err(runtime_error_to_vm_error)?
                        .into();
                    }
                }
                continue;
            }
            Step::Read {
                object,
                key,
                receiver,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let receiver = receiver.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                let read = runtime
                    .prepare_ordinary_read(&object, &key, receiver)
                    .map_err(runtime_error_to_vm_error)?;
                *step = Step::PreparedRead {
                    read: Some(read),
                    key: Some(key),
                    resume: Some(resume),
                };
                continue;
            }
            Step::PreparedRead { read, key, resume } => {
                let read = read.take().expect("selected Step field");
                let key = key.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");
                match read {
                    OrdinaryRead::Complete(value) => {
                        *step = resume
                            .resume(
                                runtime,
                                Completion::Return(value.unwrap_or(Value::Undefined)),
                            )
                            .map_err(runtime_error_to_vm_error)?;
                        continue;
                    }
                    OrdinaryRead::Call { getter, receiver } => (
                        DirectCallTarget::Callable(getter),
                        receiver,
                        Vec::new(),
                        resume,
                    ),
                    OrdinaryRead::Special {
                        object, receiver, ..
                    } => {
                        if !execution
                            .frames
                            .can_push_with_continuations(query.continuation_depth())
                        {
                            *step = resume
                                .resume(runtime, overflow(runtime, realm)?)
                                .map_err(runtime_error_to_vm_error)?;
                            continue;
                        }
                        query.parents.try_reserve(1).map_err(|_| {
                            Error::internal("property continuation allocation failed")
                        })?;
                        query.parents.push(resume);
                        *step = ProxyGetStep::start(runtime, realm, object, key, receiver)
                            .map_err(runtime_error_to_vm_error)?
                            .into();
                        continue;
                    }
                }
            }
            Step::Call {
                target,
                receiver,
                arguments,
                resume,
            } => {
                let target = target.take().expect("selected Step field");
                let receiver = receiver.take().expect("selected Step field");
                let arguments = arguments.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");
                (target, receiver, arguments, resume)
            }
            Step::Descriptor {
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
                    query
                        .parents
                        .try_reserve(1)
                        .map_err(|_| Error::internal("property continuation allocation failed"))?;
                    query.parents.push(resume);
                    *step = ProxyOwnStep::start(runtime, realm, object, key)
                        .map_err(runtime_error_to_vm_error)?
                        .into();
                    continue;
                }
                let descriptor = runtime
                    .internal_get_own_property(realm, &object, &key)
                    .map_err(runtime_error_to_vm_error)?;
                *step = resume
                    .descriptor(runtime, descriptor)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            _ => return Ok(Next::Continue),
        };
        *step = Step::Call {
            target: Some(target),
            receiver: Some(receiver),
            arguments: Some(arguments),
            resume: Some(resume),
        };
        return Ok(Next::Invoke);
    }
}
