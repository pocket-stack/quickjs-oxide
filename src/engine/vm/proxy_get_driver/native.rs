//! Native activation owners stay local when their first shared step completes.
use super::super::call::{NativeInvokeOutcome, PreparedNativeCall};
use super::super::stack::SlotStore;
use super::*;

pub(super) fn finish(
    runtime: &Runtime,
    slots: &mut SlotStore,
    call: PreparedNativeCall,
    mut resume: Resume,
    result: Result<NativeInvokeOutcome, Error>,
) -> Result<Step, Error> {
    let mut output = Step::Complete(Some(Completion::Return(Value::Undefined)));
    finish_into(runtime, slots, call, &mut resume, result, &mut output)?;
    Ok(output)
}

fn finish_into(
    runtime: &Runtime,
    slots: &mut SlotStore,
    call: PreparedNativeCall,
    resume: &mut Resume,
    result: Result<NativeInvokeOutcome, Error>,
    output: &mut Step,
) -> Result<(), Error> {
    let result = finish_result(runtime, slots, call, result)?;
    apply_into(runtime, resume, result, output)
}

pub(super) fn finish_result(
    runtime: &Runtime,
    slots: &mut SlotStore,
    call: PreparedNativeCall,
    result: Result<NativeInvokeOutcome, Error>,
) -> Result<NativeInvokeOutcome, Error> {
    // Materialize ordinary iterator results and native errors while the native
    // activation and its selected realm are still visible.
    let result = result
        .map_err(crate::engine::api::runtime_error::RuntimeError::Engine)
        .and_then(|result| match (call.activation.mode, result) {
            (
                super::super::call::NativeInvokeMode::Ordinary,
                NativeInvokeOutcome::IteratorNextRaw { value, done },
            ) => Ok(NativeInvokeOutcome::Completion(Completion::Return(
                Value::Object(runtime.new_iterator_result(call.activation.realm, value, done)?),
            ))),
            (_, result) => Ok(result),
        });
    let (result, arguments) = call.activation.finish_reusing(result);
    slots.recycle_native_argument_buffer(arguments);
    result.map_err(runtime_error_to_vm_error)
}

pub(super) fn identity_completion(result: NativeInvokeOutcome) -> Result<Completion, Error> {
    let completion =
        Runtime::ordinary_native_completion(result).map_err(runtime_error_to_vm_error)?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "native_identity_completed_in_place",
    );
    Ok(completion)
}

fn apply_into(
    runtime: &Runtime,
    resume: &mut Resume,
    result: NativeInvokeOutcome,
    output: &mut Step,
) -> Result<(), Error> {
    if matches!(resume, Resume::Identity) {
        *output = Step::Complete(Some(identity_completion(result)?));
    } else if let Resume::IteratorNext(next) = resume {
        match next.raw_completion(result) {
            Ok(Ok(result)) => {
                *output = Step::IteratorNextComplete(Some(result));
                #[cfg(feature = "profiling")]
                crate::engine::api::profiling::record_owned_execution_event(
                    "native_iterator_completed_in_place",
                );
            }
            Ok(Err(result)) => {
                *output = std::mem::replace(resume, Resume::Identity)
                    .resume(runtime, result)
                    .map_err(runtime_error_to_vm_error)?;
            }
            Err(error) => {
                // The old consuming raw adapter released a wrong-phase owner
                // before propagating its invariant error. Drop it in place.
                *resume = Resume::Identity;
                return Err(runtime_error_to_vm_error(error));
            }
        }
    } else {
        *output = std::mem::replace(resume, Resume::Identity)
            .native(runtime, result)
            .map_err(runtime_error_to_vm_error)?;
    }
    Ok(())
}

/// Registered synchronous families need no Query identity or waiting buffers.
/// The caller checked budgets before reaching any body; preparation still owns
/// padding, metadata validation and the observable native diagnostic frame.
#[allow(clippy::too_many_arguments)]
pub(super) fn begin_synchronous(
    runtime: &Runtime,
    slots: &mut SlotStore,
    realm: crate::engine::heap::ContextId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    receiver: Value,
    arguments: Vec<Value>,
    kind: crate::engine::builtins::continuation::SynchronousNative,
    selected: Option<super::super::frames::NativeClassification>,
) -> Result<Completion, Error> {
    slots.reserve_native_argument_depth(runtime.0.active_frame_depth.get() + 1)?;
    let native_realm = if target.uses_calling_realm() {
        realm
    } else {
        defining_realm
    };
    let call = runtime
        .prepare_native_continuation_selected(
            callable,
            native_realm,
            target,
            min_readable_args,
            super::super::call::NativeInvocation::Call {
                this_value: receiver,
            },
            arguments,
            super::super::call::NativeInvokeMode::Ordinary,
            selected,
        )
        .map_err(runtime_error_to_vm_error)?;
    let result = (|| {
        let result = match runtime.adapt_native_invocation_borrowed(
            target,
            native_realm,
            &call.invocation,
            &call.activation.arguments,
        )? {
            super::super::call::NativeInvocationAdaptation::Complete(result) => result,
            super::super::call::NativeInvocationAdaptation::Invoke(invocation) => kind.start(
                runtime,
                native_realm,
                &invocation,
                &call.activation.arguments,
                &call.activation.callable,
            )?,
        };
        Ok(result)
    })();
    #[cfg(feature = "profiling")]
    {
        crate::engine::api::profiling::record_owned_execution_event(
            "native_completed_without_waiting_scope",
        );
        crate::engine::api::profiling::record_owned_execution_event("native_synchronous_entry");
    }
    // This ABI can only produce a Completion. It has no raw iterator variant
    // and therefore requires neither the generic outcome wrapper nor an
    // identity resume adapter. Error capture and owner cleanup stay shared.
    let (result, arguments) = call.activation.finish_completion_reusing(result);
    slots.recycle_native_argument_buffer(arguments);
    result.map_err(runtime_error_to_vm_error)
}

pub(super) struct NativeWaitRecord {
    pub(super) call: Option<PreparedNativeCall>,
    pub(super) step: Step,
    // Only a real wait in the selected nested @@replace needs this owner.
    // Ordinary local completion never allocates or transports a parent record.
    pub(super) parents: Vec<ReplaceParent>,
}
pub(super) struct ReplaceParent {
    pub(super) call: Option<PreparedNativeCall>,
    pub(super) resume: crate::engine::builtins::StringReplaceResume,
}

/// The synchronous ABI contains no generic Step/Resume. Their reusable owning
/// record is allocated only after the domain selects an actual waiting effect.
pub(super) enum LocalNativeResult {
    Complete(Completion),
    Waiting(Vec<NativeWaitRecord>),
}

// Keep the generic waiting enum out of begin_local's stack frame. Completed
// non-migrated domains also avoid allocation; only a true effect owns a record.
#[inline(never)]
fn capture_native_step(
    runtime: &Runtime,
    slots: &mut SlotStore,
    storage: &mut storage::QueryStorage,
    realm: crate::engine::heap::ContextId,
    nested_budget: bool,
    step: crate::engine::builtins::continuation::NativeStep,
    pending: &mut Option<Vec<NativeWaitRecord>>,
) -> Result<Option<NativeInvokeOutcome>, Error> {
    use crate::engine::builtins::StringReplaceStep;
    use crate::engine::builtins::continuation::NativeStep;
    use crate::engine::builtins::native::{NativeFunctionId, RegExpNativeKind};
    // Consume exactly the method selected by String.replace's observable Get.
    // Bound, Proxy and custom methods retain their general Call continuation.
    if let NativeStep::StringReplace(StringReplaceStep::Call { mut resume }) = step {
        let target = resume.take_call_target();
        let receiver = resume.take_call_receiver();
        let arguments = resume.take_call_arguments();
        let DirectCallTarget::Callable(callable) = target else {
            return capture_waiting_step(
                storage,
                NativeStep::StringReplace(StringReplaceStep::make_call(
                    target, receiver, arguments, resume,
                )),
                pending,
            );
        };
        if resume.awaits_protocol_result() {
            if let Some(mut selected) =
                super::super::frames::NativeClassification::select(runtime, &callable)
                    .map_err(runtime_error_to_vm_error)?
                    .filter(|selected| {
                        selected.target() == NativeFunctionId::RegExp(RegExpNativeKind::Replace)
                    })
            {
                let result = if !nested_budget || runtime.host_stack_would_overflow() {
                    LocalNativeResult::Complete(overflow(runtime, realm)?)
                } else {
                    let target = selected.target();
                    let defining_realm = selected.defining_realm();
                    let minimum = selected.minimum();
                    let kind = selected
                        .take_operation()
                        .ok_or_else(|| Error::internal("selected replace has no continuation"))?;
                    begin_local(
                        runtime,
                        slots,
                        storage,
                        realm,
                        callable,
                        target,
                        defining_realm,
                        minimum,
                        receiver,
                        arguments,
                        kind,
                        Some(selected),
                        false,
                    )?
                };
                return match result {
                    LocalNativeResult::Complete(result) => {
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "string_replace_protocol_completed_locally",
                        );
                        match resume
                            .resume(runtime, result)
                            .map_err(runtime_error_to_vm_error)?
                        {
                            StringReplaceStep::Complete(result) => {
                                Ok(Some(NativeInvokeOutcome::Completion(result)))
                            }
                            _ => Err(Error::internal(
                                "replace protocol reply selected another effect",
                            )),
                        }
                    }
                    LocalNativeResult::Waiting(mut records) => {
                        if storage::reserve(&mut records[0].parents, 1, "query.replace_parent")
                            .is_err()
                        {
                            let inner = records[0].call.take().expect("nested replace activation");
                            let result = finish_result(
                                runtime,
                                slots,
                                inner,
                                Err(Error::internal("replace parent storage allocation failed")),
                            )
                            .and_then(identity_completion);
                            records[0].step =
                                Step::Complete(Some(Completion::Return(Value::Undefined)));
                            storage.recycle_native_wait(records);
                            let result = result?;
                            return match resume
                                .resume(runtime, result)
                                .map_err(runtime_error_to_vm_error)?
                            {
                                StringReplaceStep::Complete(result) => {
                                    Ok(Some(NativeInvokeOutcome::Completion(result)))
                                }
                                _ => Err(Error::internal(
                                    "replace protocol error selected another effect",
                                )),
                            };
                        }
                        records[0]
                            .parents
                            .push(ReplaceParent { call: None, resume });
                        *pending = Some(records);
                        Ok(None)
                    }
                };
            }
        }
        return capture_waiting_step(
            storage,
            NativeStep::StringReplace(StringReplaceStep::make_call(
                DirectCallTarget::Callable(callable),
                receiver,
                arguments,
                resume,
            )),
            pending,
        );
    }
    capture_waiting_step(storage, step, pending)
}

#[inline(never)]
fn capture_waiting_step(
    storage: &mut storage::QueryStorage,
    step: crate::engine::builtins::continuation::NativeStep,
    pending: &mut Option<Vec<NativeWaitRecord>>,
) -> Result<Option<NativeInvokeOutcome>, Error> {
    // Completed non-migrated domains must also remain allocation-free. Only
    // after their small payload is ruled out acquire the resident wait record.
    let mut step = step.into();
    if let Some(result) = take_immediate(&mut step) {
        return Ok(Some(result));
    }
    let mut records = storage.take_native_wait()?;
    records[0].step = step;
    *pending = Some(records);
    Ok(None)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn begin_local(
    runtime: &Runtime,
    slots: &mut SlotStore,
    storage: &mut storage::QueryStorage,
    realm: crate::engine::heap::ContextId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    receiver: Value,
    arguments: Vec<Value>,
    kind: crate::engine::builtins::continuation::NativeOperation,
    selected: Option<super::super::frames::NativeClassification>,
    nested_budget: bool,
) -> Result<LocalNativeResult, Error> {
    slots.reserve_native_argument_depth(runtime.0.active_frame_depth.get() + 1)?;
    let native_realm = if target.uses_calling_realm() {
        realm
    } else {
        defining_realm
    };
    let call = runtime
        .prepare_native_continuation_selected(
            callable,
            native_realm,
            target,
            min_readable_args,
            super::super::call::NativeInvocation::Call {
                this_value: receiver,
            },
            arguments,
            super::super::call::NativeInvokeMode::Ordinary,
            selected,
        )
        .map_err(runtime_error_to_vm_error)?;
    let mut pending = None;
    let mut pending_error = None;
    let mut transported_completion = None;
    let started = (|| match runtime
        .adapt_native_invocation_borrowed(
            target,
            native_realm,
            &call.invocation,
            &call.activation.arguments,
        )
        .map_err(runtime_error_to_vm_error)?
    {
        super::super::call::NativeInvocationAdaptation::Complete(result) => {
            Ok(Some(NativeInvokeOutcome::Completion(result)))
        }
        super::super::call::NativeInvocationAdaptation::Invoke(invocation) => kind
            .start_into(
                runtime,
                native_realm,
                &invocation,
                &call.activation.arguments,
                &call.activation.callable,
                |step| match capture_native_step(
                    runtime,
                    slots,
                    storage,
                    native_realm,
                    nested_budget,
                    step,
                    &mut pending,
                ) {
                    Ok(result) => transported_completion = result,
                    Err(error) => pending_error = Some(error),
                },
            )
            .map_err(runtime_error_to_vm_error),
    })();
    let immediate = match started {
        Ok(Some(result)) => Some(Ok(result)),
        Err(error) => Some(Err(error)),
        Ok(None) => match pending_error {
            Some(error) => Some(Err(error)),
            None => match transported_completion {
                Some(result) => Some(Ok(result)),
                None if pending.is_some() => None,
                None => Some(Err(Error::internal(
                    "native start omitted its waiting step",
                ))),
            },
        },
    };
    if let Some(mut result) = immediate {
        if let Some(mut records) = pending {
            // start_into currently emits one wait as its final action. Keep
            // cleanup LIFO even if a future adapter fails after publishing it.
            if let Some(inner) = records[0].call.take() {
                result = finish_result(runtime, slots, inner, result);
            }
            records[0].step = Step::Complete(Some(Completion::Return(Value::Undefined)));
            debug_assert!(
                records[0]
                    .parents
                    .iter()
                    .all(|parent| parent.call.is_none())
            );
            records[0].parents.clear();
            storage.recycle_native_wait(records);
        }
        let result = finish_result(runtime, slots, call, result).and_then(identity_completion);
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "native_completed_without_waiting_scope",
        );
        return result.map(LocalNativeResult::Complete);
    }
    let mut records = pending.expect("native waiting output");
    if let Some(parent) = records[0].parents.first_mut() {
        parent.call = Some(call);
    } else {
        records[0].call = Some(call);
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "native_activation_transported_to_wait",
    );
    Ok(LocalNativeResult::Waiting(records))
}

// Non-migrated domains can still adapt an immediate result through NativeStep.
// Take only that small payload, leaving the wide output enum in its destination.
fn take_immediate(output: &mut Step) -> Option<NativeInvokeOutcome> {
    match output {
        Step::Complete(result) => result.take().map(NativeInvokeOutcome::Completion),
        Step::NativeRawComplete(result) => result.take(),
        _ => None,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_into(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    query: &mut Query,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    mode: super::super::call::NativeInvokeMode,
    invocation: super::super::call::NativeInvocation,
    arguments: Vec<Value>,
    resume: Resume,
    output: &mut Step,
) -> Result<(), Error> {
    start_selected_into(
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
        output,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn start_selected_into(
    runtime: &Runtime,
    execution: &mut RunningExecution,
    query: &mut Query,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    mode: super::super::call::NativeInvokeMode,
    invocation: super::super::call::NativeInvocation,
    arguments: Vec<Value>,
    mut resume: Resume,
    output: &mut Step,
    selected: Option<super::super::frames::NativeClassification>,
) -> Result<(), Error> {
    let realm = query.realm;
    let Some(kind) = super::super::frames::native_operation(runtime, &callable)
        .map_err(runtime_error_to_vm_error)?
    else {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_sync_call_bridge();
        let native_realm = if matches!(mode, super::super::call::NativeInvokeMode::IteratorNextRaw)
            || target.uses_calling_realm()
        {
            realm
        } else {
            defining_realm
        };
        let result = runtime
            .invoke_native_function(
                &callable,
                native_realm,
                target,
                min_readable_args,
                invocation,
                &arguments,
                mode,
            )
            .map_err(runtime_error_to_vm_error)?;
        return apply_into(runtime, &mut resume, result, output);
    };
    if !execution
        .frames
        .can_push_with_continuations(query.continuation_depth())
        || runtime.host_stack_would_overflow()
    {
        *output = resume
            .resume(runtime, overflow(runtime, realm)?)
            .map_err(runtime_error_to_vm_error)?;
        return Ok(());
    }
    storage::reserve(&mut query.natives, 1, "query.native_scopes")
        .map_err(|_| Error::internal("native continuation allocation failed"))?;
    storage::reserve(
        &mut query.spare_parents,
        query.natives.len() + 1,
        "query.spare_parents",
    )
    .map_err(|_| Error::internal("native parent storage allocation failed"))?;
    let mut waiting_call = None;
    let immediate = begin_selected_into(
        runtime,
        &mut execution.slots,
        realm,
        callable,
        target,
        defining_realm,
        min_readable_args,
        mode,
        invocation,
        arguments,
        kind,
        output,
        &mut waiting_call,
        selected,
    )?;
    if let Some(result) = immediate {
        return apply_into(runtime, &mut resume, result, output);
    }
    install_waiting(
        query,
        waiting_call.expect("native wait has an activation"),
        resume,
    );
    Ok(())
}

/// Shared first native step. Caller reserves continuation capacity and checks
/// logical/native stack budgets before any invocation effect can occur.
#[allow(clippy::too_many_arguments)]
pub(super) fn begin_into(
    runtime: &Runtime,
    slots: &mut SlotStore,
    realm: crate::engine::heap::ContextId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    mode: super::super::call::NativeInvokeMode,
    invocation: super::super::call::NativeInvocation,
    arguments: Vec<Value>,
    kind: crate::engine::builtins::continuation::NativeOperation,
    output: &mut Step,
    waiting_call: &mut Option<PreparedNativeCall>,
) -> Result<Option<NativeInvokeOutcome>, Error> {
    begin_selected_into(
        runtime,
        slots,
        realm,
        callable,
        target,
        defining_realm,
        min_readable_args,
        mode,
        invocation,
        arguments,
        kind,
        output,
        waiting_call,
        None,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn begin_selected_into(
    runtime: &Runtime,
    slots: &mut SlotStore,
    realm: crate::engine::heap::ContextId,
    callable: crate::engine::object::CallableRef,
    target: crate::engine::builtins::native::NativeFunctionId,
    defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    mode: super::super::call::NativeInvokeMode,
    invocation: super::super::call::NativeInvocation,
    arguments: Vec<Value>,
    kind: crate::engine::builtins::continuation::NativeOperation,
    output: &mut Step,
    waiting_call: &mut Option<PreparedNativeCall>,
    selected: Option<super::super::frames::NativeClassification>,
) -> Result<Option<NativeInvokeOutcome>, Error> {
    debug_assert!(waiting_call.is_none());
    slots.reserve_native_argument_depth(runtime.0.active_frame_depth.get() + 1)?;
    let native_realm = if matches!(mode, super::super::call::NativeInvokeMode::IteratorNextRaw)
        || target.uses_calling_realm()
    {
        realm
    } else {
        defining_realm
    };
    let call = runtime
        .prepare_native_continuation_selected(
            callable,
            native_realm,
            target,
            min_readable_args,
            invocation,
            arguments,
            mode,
            selected,
        )
        .map_err(runtime_error_to_vm_error)?;
    let mut waiting_written = false;
    let prepared = (|| match runtime
        .adapt_native_invocation_borrowed(
            target,
            native_realm,
            &call.invocation,
            &call.activation.arguments,
        )
        .map_err(runtime_error_to_vm_error)?
    {
        super::super::call::NativeInvocationAdaptation::Complete(result) => {
            Ok(Some(NativeInvokeOutcome::Completion(result)))
        }
        super::super::call::NativeInvocationAdaptation::Invoke(invocation) => {
            let mut waiting = |waiting: crate::engine::builtins::continuation::NativeStep| {
                *output = waiting.into();
                waiting_written = true;
            };
            // Known Array-next calls keep every activation and ABI check above,
            // but need not enter the generic native dispatcher's wide frame.
            match kind {
                crate::engine::builtins::continuation::NativeOperation::ArrayNext => {
                    crate::engine::builtins::continuation::start_array_next_into(
                        runtime,
                        native_realm,
                        &invocation,
                        &mut waiting,
                    )
                }
                kind => kind.start_into(
                    runtime,
                    native_realm,
                    &invocation,
                    &call.activation.arguments,
                    &call.activation.callable,
                    &mut waiting,
                ),
            }
            .map_err(runtime_error_to_vm_error)
        }
    })();
    let immediate = match prepared {
        Ok(Some(result)) => Some(Ok(result)),
        Err(error) => Some(Err(error)),
        Ok(None) if waiting_written => take_immediate(output).map(Ok),
        Ok(None) => Some(Err(Error::internal(
            "native start omitted its waiting step",
        ))),
    };
    #[cfg(feature = "profiling")]
    if let Some(result) = &immediate {
        crate::engine::api::profiling::record_owned_execution_event(
            if matches!(result, Ok(NativeInvokeOutcome::IteratorNextRaw { .. })) {
                "native_raw_completed_without_waiting_scope"
            } else {
                "native_completed_without_waiting_scope"
            },
        );
    }
    if let Some(result) = immediate {
        // Keep the activation at its preparation site for immediate returns.
        // Only a real wait transports this owner to a continuation container.
        return finish_result(runtime, slots, call, result).map(Some);
    }
    *waiting_call = Some(call);
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "native_activation_transported_to_wait",
    );
    Ok(None)
}

/// Already selected Array-next, before a PendingIterator/Query is installed.
/// The caller performs logical/host budget checks before entering this function.
/// Only a real selected wait transfers the native activation to its output.
#[allow(clippy::too_many_arguments)]
pub(super) fn compact_array_next_into(
    runtime: &Runtime,
    slots: &mut SlotStore,
    realm: crate::engine::heap::ContextId,
    callable: crate::engine::object::CallableRef,
    _defining_realm: crate::engine::heap::ContextId,
    min_readable_args: u8,
    receiver: Value,
    output: &mut Step,
    waiting_call: &mut Option<PreparedNativeCall>,
) -> Result<Option<NativeInvokeOutcome>, Error> {
    debug_assert!(waiting_call.is_none());
    slots.reserve_native_argument_depth(runtime.0.active_frame_depth.get() + 1)?;
    let call = runtime
        .prepare_array_next_owned(callable, realm, min_readable_args, receiver)
        .map_err(runtime_error_to_vm_error)?;
    let mut waiting_written = false;
    let started = (|| match runtime.adapt_native_invocation_borrowed(
        call.activation.target,
        realm,
        &call.invocation,
        &call.activation.arguments,
    )? {
        super::super::call::NativeInvocationAdaptation::Complete(result) => {
            Ok(Some(NativeInvokeOutcome::Completion(result)))
        }
        super::super::call::NativeInvocationAdaptation::Invoke(invocation) => {
            crate::engine::builtins::continuation::start_array_next_into(
                runtime,
                realm,
                &invocation,
                |step| {
                    *output = step.into();
                    waiting_written = true;
                },
            )
        }
    })()
    .map_err(runtime_error_to_vm_error);
    let immediate = match started {
        Ok(Some(result)) => Some(Ok(result)),
        Err(error) => Some(Err(error)),
        Ok(None) if waiting_written => take_immediate(output).map(Ok),
        Ok(None) => Some(Err(Error::internal("Array-next omitted its waiting step"))),
    };
    if let Some(result) = immediate {
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event(
            "native_raw_completed_without_waiting_scope",
        );
        return finish_result(runtime, slots, call, result).map(Some);
    }
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(
        "native_activation_transported_to_wait",
    );
    *waiting_call = Some(call);
    Ok(None)
}

pub(super) fn install_waiting(query: &mut Query, call: PreparedNativeCall, resume: Resume) {
    let realm = query.realm;
    let native_realm = call.activation.realm;
    query.saved_native_depth += 1 + query.parents.len() as u128;
    query.natives.push(NativeScope {
        call,
        parents: std::mem::replace(
            &mut query.parents,
            query.spare_parents.pop().unwrap_or_default(),
        ),
        resume,
        parent_realm: realm,
    });
    query.realm = native_realm;
}
#[cfg(test)]
mod tests {
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn classified_native_calls_preserve_bound_tail_receiver_and_waiting_reentry() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let map=new Map(), trace='', errors=0;
            const set=Map.prototype.set.bind(map,'key');
            function tail(v){return set(v);}
            function tailMin(v){return Math.min(8,v);}
            for(let i=0;i<8;i++) {
                if(tail(i)!==map || map.get('key')!==i)return false;
                let v={valueOf(){trace+='V';map.set('nested',Math.max(2,4));return 3;}};
                if(tailMin(v)!==3 || map.get('nested')!==4)return false;
                try{Math.min({valueOf(){trace+='T';throw 17;}});}catch(e){if(e===17)errors++;}
                try{Map.prototype.get.call(null,'x');}catch(e){if(e instanceof TypeError)errors++;}
                if(Math.max.bind(null,1).bind(null,2)(3,4)!==4)return false;
            }
            return trace==='VTVTVTVTVTVTVTVT' && errors===16 && tailMin(2)===2;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn classified_native_errors_use_defining_realm_on_cold_and_warm_calls() {
        let runtime = Runtime::new();
        let mut caller = runtime.new_context();
        let mut defining = runtime.new_context();
        let minimum = defining.eval("Math.min").unwrap();
        let type_error = defining.eval("TypeError.prototype").unwrap();
        let global = caller.global_object().unwrap();
        for (name, value) in [("foreignMin", minimum), ("foreignTypeError", type_error)] {
            caller
                .set_property(&global, &runtime.intern_property_key(name).unwrap(), value)
                .unwrap();
        }
        drop(defining);
        assert_eq!(caller.eval(r#"(()=>{
            let errors=0;
            for(let i=0;i<8;i++) {
                if(foreignMin(7,2)!==2)return false;
                try{foreignMin(Symbol());}catch(e){if(Object.getPrototypeOf(e)===foreignTypeError)errors++;}
            }
            return errors===8;
        })()"#).unwrap(),Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[cfg(feature = "profiling")]
    #[test]
    fn classified_native_calls_use_direct_completion_and_install_wait_only_once() {
        use crate::engine::api::profiling::CostProfile;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let calls=0,sum=0,map=new Map();
            for(let i=0;i<12;i++) {
                sum+=Math.min(2,4);
                map.set(i,i);
                sum+=Math.max({valueOf(){calls++;return Math.min(3,5);}},1);
            }
            return calls===12 && sum===60 && map.size===12;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        let costs = profile.snapshot();
        assert!(
            costs
                .owned_execution_events
                .get("native_call_completed_without_query")
                .copied()
                .unwrap_or(0)
                > 20
        );
        assert!(
            costs
                .owned_execution_events
                .get("native_call_direct_wait")
                .copied()
                .unwrap_or(0)
                > 0
        );
        assert!(
            costs
                .owned_execution_events
                .get("query_storage_new")
                .copied()
                .unwrap_or(0)
                > 0
        );
        assert!(
            costs
                .owned_execution_events
                .get("native_activation_transported_to_wait")
                .copied()
                .unwrap_or(0)
                > 0
        );
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn direct_array_next_keeps_getter_conversion_and_reentry_progress() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let trace='', nested=0;
            let source={get length(){trace+='L';return {valueOf(){trace+='N';return 2;}};},
                get 0(){trace+='A';for(let v of [3,4])nested+=v;return 1;},
                get 1(){trace+='B';return 2;}};
            let iterator=Array.prototype.values.call(source),sum=0;
            for(let v of iterator)sum+=v;
            return sum===3 && nested===7 && trace==='LNALNBLN';
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn direct_array_next_keeps_proxy_holes_and_abrupt_record_disabling() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let trace='', values=[];
            let source=new Proxy([1,,3],{get(o,k,r){if(k==='length')trace+='L';if(k==='0'||k==='1'||k==='2')trace+=k;return Reflect.get(o,k,r);}});
            for(let v of source)values.push(v);
            let closed=0, caught=false, reads=0;
            let bad=Array.prototype.values.call({length:2,get 0(){reads++;return 7;},get 1(){reads++;throw 42;}});
            bad.return=function(){closed++;return {};};
            try{for(let v of bad){}}catch(e){caught=e===42;}
            return trace==='L0L1L2L' && values.length===3 && values[1]===undefined
                && caught && reads===2 && closed===0;
        })()"#).unwrap(), Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn direct_array_next_keeps_conversion_throw_and_normal_early_close() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(()=>{
            let calls=0,caught=false,closed=0;
            let bad=Array.prototype.values.call({get length(){return {valueOf(){calls++;throw 99;}};}});
            try{for(let v of bad){}}catch(e){caught=e===99;}
            let good=[1,2].values();good.return=function(){closed++;return {};};
            for(let v of good)break;
            let bound=[3].values();bound.next=bound.next.bind(bound);
            let sum=0;for(let v of bound)sum+=v;
            return calls===1 && caught && closed===1 && sum===3;
        })()"#).unwrap(), Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    #[cfg(feature = "profiling")]
    fn direct_array_next_reports_completion_without_query_and_preserved_waits() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = crate::engine::api::profiling::CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let sum=0;for(let v of [1,2,3])sum+=v;
            let iterator=Array.prototype.values.call({length:1,get 0(){return 4;}});
            for(let v of iterator)sum+=v;
            return sum;
        })()"#
                )
                .unwrap(),
            Value::Int(10)
        );
        let cost = profile.snapshot();
        assert!(
            cost.owned_execution_events
                .get("iterator_native_completed_without_query")
                .copied()
                .unwrap_or(0)
                >= 4
        );
        assert!(
            cost.owned_execution_events
                .get("iterator_native_direct_wait")
                .copied()
                .unwrap_or(0)
                > 0
        );
        assert_eq!(cost.owned_bridge_exits, 0);
        assert_eq!(cost.owned_sync_call_bridges, 0);
    }

    #[test]
    fn immediate_native_errors_and_raw_iterator_results_preserve_activation_lifetimes() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let caught=0;
            for (let i=0;i<20;i++) {
                try { [1].map(()=>Promise.resolve.call(null,1)); }
                catch(e) { if(e instanceof TypeError && e.stack.includes('resolve')) caught++; }
                try { Promise.reject.call(undefined,1); }
                catch(e) { if(e instanceof TypeError) caught++; }
            }
            let iterator=[3,4].values();
            let first=iterator.next(), second=iterator.next(), done=iterator.next();
            let sum=0;for(let value of [3,4])sum+=value;
            return caught===40 && first.value===3 && !first.done && second.value===4
                && !second.done && done.done && sum===7 && Math.min(4,2,3)===2;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
    #[cfg(feature = "profiling")]
    #[test]
    fn immediate_and_waiting_native_outputs_keep_nested_coercion_and_error_order() {
        use crate::engine::api::profiling::CostProfile;
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let reads=0;let errors=0;let result=0;
            for(let i=0;i<10;i++) {
                result+=Math.min(1,{valueOf(){reads++;return Math.max(2,3);}},4);
                result+=parseInt({toString(){reads++;return '11';}},{valueOf(){reads++;return 2;}});
                try {[0].map(()=>Math.min(0,{valueOf(){throw 42;}}));} catch(e){if(e===42)errors++;}
                try {Math.min(1,Symbol());} catch(e){if(e instanceof TypeError)errors++;}
                let entry=[7].values().next();if(entry.value!==7)return false;
            }
            return result===40 && reads===30 && errors===20 && Math.min(4,2)===2;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        let costs = profile.snapshot();
        let count = |name| costs.owned_execution_events.get(name).copied().unwrap_or(0);
        let synchronous = count("native_synchronous_entry");
        let domain = count("native_domain_completed_without_waiting_payload");
        let waiting = count("native_activation_transported_to_wait");
        let identity = count("native_identity_completed_in_place");
        // The ten primitive Math.max calls, ten throwing primitive Math.min
        // calls and final Math.min now use the narrow entry rather than the
        // domain-output adapter. Extra Pure calls (e.g. Symbol) can add hits.
        assert!(synchronous >= 21, "narrow completion: {synchronous}");
        // The ten ordinary iterator.next calls still produce raw domain output
        // before materializing their JS result under the native activation.
        assert!(domain >= 10, "raw domain completion: {domain}");
        // Each loop really waits in the outer Math.min, parseInt, Array.map and
        // callback Math.min. The two parseInt coercions share one activation.
        assert!(waiting >= 40, "native waiting activations: {waiting}");
        assert!(count("native_call_direct_wait") >= 40);
        // Every narrow entry and all ten direct raw-next completions consume
        // the identity result once; resumed waiting native calls add more.
        assert!(
            identity >= synchronous + 10,
            "identity: {identity}, narrow: {synchronous}"
        );
        assert_eq!(costs.owned_bridge_exits, 0);
        assert_eq!(costs.owned_sync_call_bridges, 0);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod array_next_small_entry_tests {
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn array_next_small_entry_preserves_cached_loop_and_ordinary_native_calls() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        #[cfg(feature = "profiling")]
        let profile = crate::engine::api::profiling::CostProfile::start();
        assert_eq!(
            context
                .eval(
                    r#"(function () {
            var total = 0;
            for (var round = 0; round < 8; round++) {
                for (var v of [1, 2, 3]) total += v;
                var it = [4, 5].values(), next = it.next;
                var a = next.call(it), b = it.next(), done = next.call(it);
                if (a.value !== 4 || a.done || b.value !== 5 || b.done || !done.done)
                    return false;
                var entry = [7].entries().next();
                if (entry.done || entry.value[0] !== 0 || entry.value[1] !== 7)
                    return false;
            }
            return total === 48;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        #[cfg(feature = "profiling")]
        {
            let cost = profile.snapshot();
            assert!(
                cost.owned_execution_events
                    .get("native_activation_prepared")
                    .copied()
                    .unwrap_or(0)
                    >= 24
            );
            assert!(
                cost.owned_execution_events
                    .get("array_next_dense_immediate_leaf")
                    .copied()
                    .unwrap_or(0)
                    >= 24
            );
            assert_eq!(cost.owned_bridge_exits, 0);
            assert_eq!(cost.owned_sync_call_bridges, 0);
        }
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn array_next_small_entry_preserves_selected_waits_and_error_recovery() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(function () {
            // Warm native query storage before the for-of Proxy wait.
            for (var warm of [1, 2]) {}
            var trace = '', proxy = new Proxy([9], {get: function(t, k, r) {
                if (k === 'length' || k === '0') trace += k + ',';
                return Reflect.get(t, k, r);
            }}), sum = 0;
            for (var v of proxy) sum += v;
            if (sum !== 9 || trace !== 'length,0,length,') return false;
            var reads = 0, it = Array.prototype.values.call({length: 2,
                get 0() { reads++; throw 31; }, 1: 22});
            var caught;
            try { it.next(); } catch(e) { caught = e; }
            var second = it.next();
            if (caught !== 31 || reads !== 1 || second.value !== 22 || second.done)
                return false;
            var lengthReads = 0, lengthIt = Array.prototype.values.call({
                get length() { if (++lengthReads === 1) throw 41; return 1; }, 0: 11
            });
            try { lengthIt.next(); } catch(e) { caught = e; }
            if (caught !== 41 || lengthIt.next().value !== 11 || lengthReads !== 2)
                return false;
            var next = [].values().next, bad = false;
            try { next.call({}); } catch(e) { bad = e instanceof TypeError; }
            return bad && [6].values().next().value === 6 && it.next().done;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod native_continuation_publication_tests {
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn native_continuation_publication_keeps_waits_nested_calls_and_throw_recovery() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(function () {
            var map = new Map(), reads = 0, errors = 0;
            for (var i = 0; i < 8; i++) {
                var result = map.getOrInsertComputed(i, function(key) {
                    reads++;
                    if (map.set('nested', key) !== map) throw 99;
                    return Math.min(9, {valueOf: function() { reads++; return 3; }});
                });
                if (result !== 3 || map.get('nested') !== i) return false;
                try { map.getOrInsertComputed('bad', function() { throw 27; }); }
                catch(e) { if (e === 27) errors++; }
                if (map.has('bad')) return false;
                try { Map.prototype.get.call(null, i); }
                catch(e) { if (e instanceof TypeError) errors++; }
                var it = Array.prototype.values.call({length:1, get 0() {
                    reads++; return map.get(i);
                }});
                if (it.next().value !== 3 || !it.next().done) return false;
            }
            return reads === 24 && errors === 16 && map.get(7) === 3;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod cached_native_inplace_tests {
    use crate::engine::api::{Runtime, Value};
    use std::{
        cell::{Cell, RefCell},
        rc::Rc,
    };

    #[test]
    fn cached_native_inplace_handles_cold_warm_wait_throw_and_actual_host_reentry() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let host_context = RefCell::new(context.clone());
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        struct ClearTracker(Runtime);
        impl Drop for ClearTracker {
            fn drop(&mut self) {
                self.0.clear_host_promise_rejection_tracker();
            }
        }
        let guard = ClearTracker(runtime.clone());
        runtime.set_host_promise_rejection_tracker(move |event| {
            if event.is_handled() {
                return;
            }
            observed.set(observed.get() + 1);
            assert_eq!(
                host_context
                    .borrow_mut()
                    .eval(
                        r#"(function() {
                var map = new Map();
                for (var i = 0; i < 4; i++) map.set(i, Math.min(7, 2));
                return map.get(3);
            })()"#
                    )
                    .unwrap(),
                Value::Int(2)
            );
        });
        assert_eq!(
            context
                .eval(
                    r#"(function () {
            var reads = 0, failures = 0, map = new Map();
            for (var i = 0; i < 8; i++) {
                if (Math.min(8, 2) !== 2) return false;
                var value = Math.min(8, {valueOf:function() { reads++; return 3; }});
                if (map.set(i, value) !== map || map.get(i) !== 3) return false;
                try { Math.min({valueOf:function() { throw 17; }}); }
                catch(e) { if (e === 17) failures++; }
                Promise.reject(i);
                if (Math.max(1, 4) !== 4) return false;
            }
            return reads === 8 && failures === 8 && map.get(7) === 3;
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert_eq!(calls.get(), 8);
        drop(guard);
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(test)]
mod selected_replace_local_tests {
    use super::*;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn selected_replace_keeps_method_getter_custom_exec_groups_and_callbacks_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(() => {
            const replace = RegExp.prototype[Symbol.replace];
            let methods = 0, flags = 0, execGets = 0, execCalls = 0, groups = 0, names = 0;
            const result = {0:'a', length:1, index:0,
                get groups() { groups++; return {get x(){names++;return 'Z'}}; }};
            const search = {
                get [Symbol.replace]() {
                    methods++;
                    Object.defineProperty(this, Symbol.replace, {value(){throw 99}});
                    return replace;
                },
                get flags() { flags++; return 'g'; },
                get exec() { execGets++; return function() {
                    execCalls++; return execCalls === 1 ? result : null;
                }; }
            };
            if ('a'.replace(search, '$<x>') !== 'Z') return false;
            if (methods!==1 || flags!==1 || execGets!==2 || execCalls!==2 || groups!==1 || names!==1) return false;
            let callbacks=0, original=RegExp.prototype.exec, rx=/a/g;
            let replaced='aa'.replace(rx, function(m,p,s) {
                callbacks++;
                rx.exec=function(){throw 71};
                return String(p);
            });
            if (replaced!=='01' || callbacks!==2) return false;
            let custom=0, read=0;
            const other={get [Symbol.replace]() {read++;return function(s,r) {
                custom++; return this===other && s==='a' && r==='b' ? 'ok' : 'bad';
            }}};
            if ('a'.replace(other,'b')!=='ok' || custom!==1 || read!==1) return false;
            let stringCallbacks=0;
            return 'aba'.replaceAll('a', function(m,p){stringCallbacks++;return p})==='0b2'
                && stringCallbacks===2 && 'aa'.replace(/a/g,'x')==='xx';
        })()"#).unwrap(), Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn selected_replace_inner_budget_rejects_in_parent_realm_before_input_conversion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let input = context
            .eval("globalThis.conversions=0;({toString(){conversions++;return 'a'}})")
            .unwrap();
        let mut foreign = runtime.new_context();
        let search = foreign.eval("/a/g").unwrap();
        let callable = runtime
            .callable_from_value(context.eval("String.prototype.replace").unwrap())
            .unwrap();
        let mut selected =
            super::super::super::frames::NativeClassification::select(&runtime, &callable)
                .unwrap()
                .unwrap();
        let target = selected.target();
        let realm = selected.defining_realm();
        let minimum = selected.minimum();
        let kind = selected.take_operation().unwrap();
        let mut slots = SlotStore::new(32);
        let mut storage = storage::QueryStorage::default();
        let result = begin_local(
            &runtime,
            &mut slots,
            &mut storage,
            context.realm,
            callable,
            target,
            realm,
            minimum,
            input,
            vec![
                search,
                Value::String(crate::engine::value::JsString::from_static("b")),
            ],
            kind,
            Some(selected),
            false,
        )
        .unwrap();
        let LocalNativeResult::Complete(Completion::Throw(Value::Object(error))) = result else {
            panic!("expected nested budget error");
        };
        // The original scheduler rejects before entering the selected native:
        // overflow belongs to query.realm (the outer activation), unlike an
        // error created after entering the foreign RegExp builtin.
        let Value::Object(expected) = runtime
            .new_native_error(
                context.realm,
                crate::engine::api::error::NativeErrorKind::Internal,
                "expected",
            )
            .unwrap()
        else {
            panic!("expected native error object");
        };
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            runtime.get_prototype_of(&expected).unwrap()
        );
        assert_eq!(context.eval("conversions").unwrap(), Value::Int(0));
        assert!(!storage.has_cached_entry());
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn selected_replace_inner_error_uses_defining_realm_and_both_diagnostic_activations() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let mut foreign = runtime.new_context();
        let global = context.global_object().unwrap();
        for (name, value) in [
            (
                "foreignReplace",
                foreign.eval("RegExp.prototype[Symbol.replace]").unwrap(),
            ),
            (
                "foreignTypeError",
                foreign.eval("TypeError.prototype").unwrap(),
            ),
        ] {
            context
                .set_property(&global, &runtime.intern_property_key(name).unwrap(), value)
                .unwrap();
        }
        assert_eq!(
            context
                .eval(
                    r#"(() => {
            const search = {[Symbol.replace]:foreignReplace};
            try { 'a'.replace(search,'b'); return false; }
            catch(e) {
                return Object.getPrototypeOf(e)===foreignTypeError
                    && e.stack.split('replace').length>=3;
            }
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}
