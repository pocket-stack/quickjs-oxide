//! Bounded native-stack dispatch for iteration requests.
use super::{
    DirectCallTarget, Error, Finish, IteratorProgress, Next, Progress, Query, Resume, ReturnOwner,
    RunningExecution, Runtime, Step, Value, continue_iterator, runtime_error_to_vm_error,
};

#[inline(never)]
pub(super) fn advance(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    _owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    // A completed native next reply carries only ObjectIteratorStep. Extract
    // that payload in place before moving any wide generic scheduler state.
    if let Step::IteratorNextComplete(result) = pending {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_iteration.advance.visit",
        );
        let result = result.take().expect("selected Step field");
        if let Some(result) = complete_next(runtime, execution, query, result, pending)? {
            return Ok(Next::Done(Progress::Call(result)));
        }
    }
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_iteration.advance.visit",
        );
        let realm = query.realm;
        match &mut *step {
            Step::RegExpSpecies { regexp, resume } => {
                let regexp = regexp.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("RegExp species continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::RegExpSpeciesStep::start(runtime, realm, regexp)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::RegExpSpeciesComplete(result) => {
                let result = result.take().expect("selected Step field");

                let resume = query
                    .parents
                    .pop()
                    .ok_or_else(|| Error::internal("RegExp species lost parent"))?;
                *step = resume
                    .regexp_species(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
                continue;
            }
            Step::Aggregate { iterable, resume } => {
                let iterable = iterable.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("AggregateError continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::AggregateStep::start(runtime, realm, iterable)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::ArraySpecies {
                source,
                length,
                resume,
            } => {
                let source = source.take().expect("selected Step field");
                let length = length.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("Array species continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::ArraySpeciesStep::start(
                    runtime, realm, &source, length,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::ArrayPush {
                object,
                value,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("Array push continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::ArrayMutationStep::start_values(
                    runtime,
                    realm,
                    crate::engine::builtins::ArrayMutationKind::Push(
                        crate::engine::builtins::native::ArrayPushKind::Push,
                    ),
                    Value::Object(object),
                    vec![value],
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::IteratorNext {
                iterator,
                method,
                resume,
            } => {
                let iterator = iterator.take().expect("selected Step field");
                let method = method.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("iterator continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::IteratorNextStep::start(
                    runtime, realm, iterator, method,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::IteratorNextComplete(result) => {
                let result = result.take().expect("selected Step field");

                *step = Step::IteratorNextComplete(Some(
                    crate::engine::builtins::ObjectIteratorStep::Done,
                ));
                if let Some(result) = complete_next(runtime, execution, query, result, step)? {
                    return Ok(Next::Done(Progress::Call(result)));
                }
                continue;
            }

            Step::IteratorCall {
                callable,
                iterator,
                resume,
            } => {
                let callable = callable.take().expect("selected Step field");
                let iterator = iterator.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                let metadata = runtime
                    .direct_native_callable_metadata(&callable)
                    .map_err(runtime_error_to_vm_error)?;
                if let Some((target, defining_realm, min_readable_args)) = metadata
                    && target.descriptor().cproto
                        == crate::engine::builtins::native::NativeCProto::IteratorNext
                {
                    *step = Step::Complete(Some(super::Completion::Return(Value::Undefined)));
                    super::native_scope(
                        runtime,
                        execution,
                        query,
                        callable,
                        target,
                        defining_realm,
                        min_readable_args,
                        super::super::call::NativeInvokeMode::IteratorNextRaw,
                        super::super::call::NativeInvocation::Call {
                            this_value: Value::Object(iterator),
                        },
                        Vec::new(),
                        Resume::IteratorNext(resume),
                        step,
                    )?;
                } else {
                    *step = Step::Call {
                        target: Some(DirectCallTarget::Callable(callable)),
                        receiver: Some(Value::Object(iterator)),
                        arguments: Some(Vec::new()),
                        resume: Some(Resume::IteratorNext(resume)),
                    };
                }
                continue;
            }
            Step::IteratorClose {
                iterator,
                completion,
            } => {
                let iterator = iterator.take().expect("selected Step field");
                let completion = completion.take().expect("selected Step field");

                *step = crate::engine::builtins::IteratorCloseStep::start(
                    runtime, realm, iterator, completion,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }

            Step::ObjectTag { receiver } => {
                let receiver = receiver.take().expect("selected Step field");

                *step = crate::engine::builtins::ObjectStringStep::start(
                    runtime,
                    realm,
                    crate::engine::builtins::ObjectStringKind::Tag,
                    &super::super::call::NativeInvocation::Call {
                        this_value: receiver,
                    },
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::RegExpExec {
                regexp,
                input,
                resume,
            } => {
                let regexp = regexp.take().expect("selected Step field");
                let input = input.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("RegExp exec continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::RegExpExecStep::abstract_exec(
                    runtime, realm, regexp, input,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::IteratorCloseWithResume {
                iterator,
                completion,
                resume,
            } => {
                let iterator = iterator.take().expect("selected Step field");
                let completion = completion.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("iterator close continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = crate::engine::builtins::IteratorCloseStep::start(
                    runtime, realm, iterator, completion,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }

            Step::OrdinaryInstance {
                constructor,
                value,
                resume,
            } => {
                let constructor = constructor.take().expect("selected Step field");
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("instance continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::InstanceStep::ordinary(
                    runtime,
                    realm,
                    &constructor,
                    value,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::ParseIterator { result, resume } => {
                let result = result.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("iterator parse continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step =
                    crate::engine::builtins::IteratorNextStep::parse_result(runtime, realm, result)
                        .map_err(runtime_error_to_vm_error)?
                        .into();
                continue;
            }
            Step::ArrayCopy {
                object,
                to,
                from,
                count,
                backwards,
                resume,
            } => {
                let object = object.take().expect("selected Step field");
                let to = to.take().expect("selected Step field");
                let from = from.take().expect("selected Step field");
                let count = count.take().expect("selected Step field");
                let backwards = backwards.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("Array copy continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::builtins::ArrayCopyStep::start(
                    runtime, realm, object, to, from, count, backwards,
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

/// The parent reply and root iterator finish paths have one ordering source.
fn complete_next(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    query: &mut Query,
    result: crate::engine::builtins::ObjectIteratorStep,
    output: &mut Step,
) -> Result<Option<super::super::driver::CallStep>, Error> {
    if let Some(parent) = query.parents.pop() {
        *output = parent
            .iterator_next(runtime, result)
            .map_err(runtime_error_to_vm_error)?;
        return Ok(None);
    }
    let Some(Finish::IteratorNext(id)) = query.finish.take() else {
        return Err(Error::internal("iterator lost its continuation"));
    };
    let mut pending = super::take_iterator_finish(execution, id)?;
    let action = pending.next_query(runtime, result)?;
    match continue_iterator(runtime, execution, query, pending, action)? {
        IteratorProgress::Step(next) => {
            *output = next;
            Ok(None)
        }
        IteratorProgress::Done(result) => Ok(Some(result)),
    }
}
