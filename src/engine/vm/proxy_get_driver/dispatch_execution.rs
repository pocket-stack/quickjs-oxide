//! Bounded native-stack dispatch for execution requests.
use super::{
    CallStep, Completion, DirectCallTarget, Error, Finish, IteratorProgress, Next, OperationTarget,
    Progress, Query, ReturnOwner, ReturnTarget, ReturnValue, RunningExecution, Runtime, Step,
    Value, construct, continue_iterator, native_scope, overflow, runtime_error_to_vm_error,
};

#[inline(never)]
pub(super) fn finish(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    _identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_execution.finish.visit",
        );
        match &mut *step {
            Step::RootDescriptor(result) => {
                let result = result.take().expect("selected Step field");

                if !matches!(owner, ReturnOwner::Root)
                    || !query.parents.is_empty()
                    || !query.natives.is_empty()
                    || !matches!(query.finish.take(), Some(Finish::Root))
                    || execution.root_descriptor.is_some()
                {
                    return Err(Error::internal(
                        "root descriptor reached a non-root continuation",
                    ));
                }
                execution.root_descriptor = Some(result);
                return Ok(Next::Done(Progress::Call(CallStep::Entered)));
            }

            Step::Complete(completion) => {
                let completion = completion.take().expect("selected Step field");

                if let Some(parent) = query.parents.pop() {
                    *step = parent
                        .resume(runtime, completion)
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                if !query.natives.is_empty() {
                    *step = query.finish_native(runtime, &mut execution.slots, Ok(completion))?;
                    continue;
                }
                let (_depth, push) = match query
                    .finish
                    .take()
                    .ok_or_else(|| Error::internal("query lost its final continuation"))?
                {
                    Finish::Root => {
                        return Ok(Next::Done(Progress::Call(CallStep::Complete(completion))));
                    }
                    Finish::Class(pending) => {
                        return super::super::construct_driver::finish_class_reply(
                            runtime, execution, *pending, completion,
                        )
                        .map(Progress::Call)
                        .map(Next::Done);
                    }
                    Finish::VmCall(value_use) => {
                        return match completion {
                            Completion::Return(value) => {
                                if matches!(value_use, ReturnValue::Push) {
                                    let parent = execution.frames.current_mut(owner.frame()?)?;
                                    execution.slots.push(&mut parent.window, value)?;
                                }
                                Ok(Next::Done(Progress::Call(CallStep::Entered)))
                            }
                            completion => {
                                Ok(Next::Done(Progress::Call(CallStep::Complete(completion))))
                            }
                        };
                    }
                    Finish::Iterator(id) => {
                        let mut pending = super::take_iterator_finish(execution, id)?;
                        let action = pending.advance_query(runtime, Some(completion))?;
                        match continue_iterator(runtime, execution, query, pending, action)? {
                            IteratorProgress::Step(next) => {
                                *step = next;
                                continue;
                            }
                            IteratorProgress::Done(result) => {
                                return Ok(Next::Done(Progress::Call(result)));
                            }
                        }
                    }
                    Finish::IteratorNext(_) => {
                        return Err(Error::internal("iterator next received untyped completion"));
                    }
                    Finish::ForIn(depth)
                    | Finish::Numeric(depth)
                    | Finish::Write { depth, .. }
                    | Finish::Discard(depth) => (depth, false),
                    Finish::PropertyRead(depth) => (depth, true),
                    Finish::Call { depth, tail } => {
                        return super::finish_call_instruction(
                            execution, owner, completion, depth, tail,
                        )
                        .map(Next::Done);
                    }
                    Finish::Conversion(wait) => {
                        return super::super::conversion_driver::ConversionTask::from_wait(
                            runtime,
                            owner.frame()?,
                            wait,
                            completion,
                        )
                        .map(Progress::Conversion)
                        .map(Next::Done);
                    }
                };
                return super::finish_instruction(execution, owner, completion, push, _depth)
                    .map(Next::Done);
            }
            Step::ForInComplete { value, done } => {
                let value = value.take().expect("selected Step field");
                let done = done.take().expect("selected Step field");

                let Some(Finish::ForIn(_depth)) = query.finish.take() else {
                    return Err(Error::internal("for-in result lost its instruction"));
                };
                return super::finish_for_in(execution, owner.frame()?, value, done, _depth)
                    .map(Progress::Call)
                    .map(Next::Done);
            }
            Step::NumericComplete { value, previous } => {
                let value = value.take().expect("selected Step field");
                let previous = previous.take().expect("selected Step field");

                let Some(Finish::Numeric(_depth)) = query.finish.take() else {
                    return Err(Error::internal("numeric result lost its instruction"));
                };
                return super::finish_numeric(execution, owner.frame()?, value, previous, _depth)
                    .map(Progress::Call)
                    .map(Next::Done);
            }
            Step::NativeRawComplete(result) => {
                let result = result.take().expect("selected Step field");

                if !query.parents.is_empty() {
                    return Err(Error::internal(
                        "raw native result escaped a child operation",
                    ));
                }
                *step = query.finish_native_outcome(runtime, &mut execution.slots, Ok(result))?;
                continue;
            }
            _ => return Ok(Next::Continue),
        }
    }
}

#[inline(never)]
pub(super) fn activation(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_execution.activation.visit",
        );
        let realm = query.realm;
        match &mut *step {
            Step::ResumeFrame {
                activation,
                input,
                resume,
            } => {
                let activation = activation.take().expect("selected Step field");
                let input = input.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if !execution
                    .frames
                    .can_push_with_continuations(query.continuation_depth())
                    || runtime.bytecode_call_would_overflow()
                {
                    *step = resume
                        .resume(runtime, overflow(runtime, realm)?)
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                let mut prepared = activation
                    .prepare_owned(runtime, input)
                    .map_err(runtime_error_to_vm_error)?;
                prepared.entry.cold.return_to = Some(ReturnTarget {
                    owner,
                    value_use: ReturnValue::Push,
                    tail: false,
                    operation: Some(OperationTarget::PropertyGet(identity)),
                });
                return Ok(Next::Call {
                    entry: Box::new(prepared.entry),
                    pc: prepared.pc,
                    resume,
                });
            }
            Step::ConstructorReady {
                request,
                receiver,
                derived,
                resume,
            } => {
                let request = request.take().expect("selected Step field");
                let receiver = receiver.take().expect("selected Step field");
                let derived = derived.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                match construct::ready(
                    runtime, execution, query, request, receiver, derived, resume,
                )? {
                    Ok(next) => return Ok(next),
                    Err(next) => {
                        *step = next;
                        continue;
                    }
                }
            }
            Step::Native {
                callable,
                target,
                defining_realm,
                min_readable_args,
                mode,
                invocation,
                arguments,
                resume,
            } => {
                let callable = callable.take().expect("selected Step field");
                let target = target.take().expect("selected Step field");
                let defining_realm = defining_realm.take().expect("selected Step field");
                let min_readable_args = min_readable_args.take().expect("selected Step field");
                let mode = mode.take().expect("selected Step field");
                let invocation = invocation.take().expect("selected Step field");
                let arguments = arguments.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                *step = Step::Complete(Some(Completion::Return(Value::Undefined)));
                native_scope(
                    runtime,
                    execution,
                    query,
                    callable,
                    target,
                    defining_realm,
                    min_readable_args,
                    mode,
                    invocation,
                    arguments,
                    resume,
                    step,
                )?;
                continue;
            }

            _ => return Ok(Next::Continue),
        }
    }
}

#[inline(never)]
pub(super) fn prepare(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    owner: ReturnOwner,
    identity: u64,
    query: &mut Query,
    pending: &mut Step,
) -> Result<Next, Error> {
    let step = pending;
    loop {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "dispatch_execution.prepare.visit",
        );
        let realm = query.realm;
        match &mut *step {
            Step::ModuleCallbackOperation {
                step: callback,
                resume,
            } => {
                let callback = callback.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("module callback continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = (*callback).into();
            }

            Step::ModuleBodyOperation { step: body, resume } => {
                let body = body.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("module body continuation allocation failed"))?;
                query.parents.push(resume);
                *step = (*body).into();
            }

            Step::ModuleLink {
                realm,
                callable,
                resume,
            } => {
                let realm = realm.take().expect("selected Step field");
                let callable = callable.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                let super::CallableExecution::Bytecode {
                    bytecode,
                    closure_slots,
                } = runtime
                    .bytecode_for_callable(&callable)
                    .map_err(runtime_error_to_vm_error)?
                else {
                    return Err(Error::internal("module link prefix is not bytecode"));
                };
                if !runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .function_bytecode(bytecode.bytecode_id())
                    .map_err(|e| Error::internal(e.to_string()))?
                    .metadata
                    .is_module
                {
                    return Err(Error::internal(
                        "module link prefix is not a module executable",
                    ));
                }
                if !execution
                    .frames
                    .can_push_with_continuations(query.continuation_depth())
                    || runtime.bytecode_call_would_overflow()
                {
                    let completion = runtime
                        .bytecode_stack_overflow_completion(realm, &bytecode)
                        .map_err(runtime_error_to_vm_error)?;
                    *step = resume
                        .resume(runtime, completion)
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                let entry = super::BytecodeCallRequest {
                    callable,
                    receiver: Value::Bool(true),
                    new_target: Value::Undefined,
                    arguments: Vec::new(),
                    bytecode,
                    closure_slots,
                    caller_realm: realm,
                    return_to: ReturnTarget {
                        owner,
                        value_use: ReturnValue::Push,
                        tail: false,
                        operation: Some(OperationTarget::PropertyGet(identity)),
                    },
                }
                .prepare(runtime, &mut execution.call_storage)?;
                return Ok(Next::Call {
                    entry: Box::new(entry),
                    pc: 0,
                    resume,
                });
            }

            Step::PromiseOperation {
                step: operation,
                resume,
            } => {
                let operation = operation.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("Promise continuation allocation failed"))?;
                query.parents.push(resume);
                *step = (*operation).into();
                continue;
            }
            Step::IntrinsicPromiseResolve {
                value,
                realm: resolve_realm,
                resume,
            } => {
                let value = value.take().expect("selected Step field");
                let resolve_realm = resolve_realm.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                query.parents.try_reserve(1).map_err(|_| {
                    Error::internal("await resolution continuation allocation failed")
                })?;
                query.parents.push(resume);
                *step = runtime
                    .prepare_intrinsic_promise_resolve(resolve_realm, value)
                    .map_err(runtime_error_to_vm_error)?
                    .into();
                continue;
            }
            Step::Construct {
                target,
                new_target,
                arguments,
                resume,
            } => {
                let target = target.take().expect("selected Step field");
                let new_target = new_target.take().expect("selected Step field");
                let arguments = arguments.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                *step = construct::start(
                    runtime, owner, identity, realm, target, new_target, arguments, resume,
                )?;
                continue;
            }
            Step::ConstructProxy {
                target,
                new_target,
                arguments,
                resume,
            } => {
                let target = target.take().expect("selected Step field");
                let new_target = new_target.take().expect("selected Step field");
                let arguments = arguments.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                if !execution
                    .frames
                    .can_push_with_continuations(query.continuation_depth())
                {
                    *step = resume
                        .resume(runtime, overflow(runtime, realm)?)
                        .map_err(runtime_error_to_vm_error)?;
                    continue;
                }
                query
                    .parents
                    .try_reserve(1)
                    .map_err(|_| Error::internal("constructor continuation allocation failed"))?;
                query.parents.push(resume);
                *step = crate::engine::object::ProxyConstructStep::start(
                    runtime, realm, target, new_target, arguments,
                )
                .map_err(runtime_error_to_vm_error)?
                .into();
                continue;
            }
            Step::IndirectEval { source, resume } => {
                let source = source.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                *step = match runtime
                    .prepare_indirect_string_eval(realm, &source)
                    .map_err(runtime_error_to_vm_error)?
                {
                    crate::engine::builtins::DirectEvalPreparation::Complete(completion) => resume
                        .resume(runtime, completion)
                        .map_err(runtime_error_to_vm_error)?,
                    crate::engine::builtins::DirectEvalPreparation::Ready {
                        callable,
                        this_value,
                    } => Step::Call {
                        target: Some(DirectCallTarget::Callable(callable)),
                        receiver: Some(this_value),
                        arguments: Some(Vec::new()),
                        resume: Some(resume),
                    },
                };
                continue;
            }
            Step::NumericHtmlDda { value, resume } => {
                let value = value.take().expect("selected Step field");
                let resume = resume.take().expect("selected Step field");

                *step = resume
                    .html_dda(
                        runtime
                            .value_is_html_dda(&value)
                            .map_err(runtime_error_to_vm_error)?,
                    )?
                    .into();
                continue;
            }
            _ => return Ok(Next::Continue),
        }
    }
}
