//! Temporary S03 ordinary-call entry and one-way handoff to the previous VM.
//! Only this adapter knows both frame representations. Hot execution owns the
//! executable and all slots; unsupported instructions hand back existing owners
//! at their untouched PC. This bridge is removed after S04–S07 cover its exits.

use super::RuntimeVmHost;
use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::value::Value;
use crate::engine::vm::frame::{FrameCold, FrameEntry};
use crate::engine::vm::stack::{FrameStorage, copy_value};
use crate::engine::vm::{CallInput, Completion, VmActivation};

/// Normal root entries use the same direct window initialization as child
/// calls. The public borrowed argv needs an independent snapshot, but no
/// parameter/local binding vectors or temporary RuntimeVmHost are built.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(in crate::engine::vm) fn execute_call(
    runtime: &Runtime,
    caller_realm: crate::engine::heap::ContextId,
    callable: &crate::engine::object::CallableRef,
    receiver: Value,
    new_target: Value,
    arguments: &[Value],
    bytecode: crate::engine::code::rooted::FunctionBytecodeRef,
    closure_slots: crate::engine::vm::closure::ClosureSlots,
) -> Result<Completion, crate::engine::api::runtime_error::RuntimeError> {
    use crate::engine::api::runtime_error::RuntimeError;
    let prepared =
        runtime.prepare_owned_bytecode_frame(callable, receiver, new_target, bytecode)?;
    if closure_slots.len() != usize::from(prepared.executable.metadata.closure_count) {
        return Err(RuntimeError::Engine(Error::internal(
            "function object closure slot count does not match bytecode metadata",
        )));
    }
    let mut original_arguments = Vec::new();
    original_arguments
        .try_reserve_exact(arguments.len())
        .map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "original argument snapshot allocation failed",
            ))
        })?;
    for value in arguments {
        original_arguments.push(copy_value(value).map_err(RuntimeError::Engine)?);
    }
    let local_count = if prepared.executable.has_captured_locals {
        prepared.executable.local_definitions.len()
    } else {
        0
    };
    let cold = crate::engine::vm::frame::ColdFrame::new(FrameCold {
        rare: std::cell::OnceCell::new(),
        return_to: None,
        entry_guard: None,
        function: (callable.as_object().clone()).into(),
        closure_slots,
        reusable_captured_locals: vec![false; local_count],
        input: (prepared.input).into(),
    });
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_call_storage(
        size_of::<crate::engine::vm::frame::FrameBody>(),
        cold.reusable_captured_locals.capacity(),
        original_arguments.capacity() * size_of::<Value>(),
    );
    let entry = FrameEntry {
        property_generation: 0,
        iterator_generation: 0,
        caller_realm: caller_realm,
        active_frame: prepared.active_frame.token(),

        initialize_bindings: true,
        executable: prepared.executable,
        cold,
        storage: FrameStorage {
            original_arguments,
            parameters: Vec::new(),
            locals: Vec::new(),
            operands: Vec::new(),
        },
    };
    let result = crate::engine::vm::driver::execute(
        runtime.clone(),
        entry,
        crate::engine::vm::execution::ExecutionLimits::default(),
    )
    .and_then(|exit| exit.finish(runtime.clone()));
    prepared.active_frame.finish()?;
    result.map_err(RuntimeError::Engine)
}

pub(in crate::engine::vm) fn execute(
    host: RuntimeVmHost,
    input: CallInput,
    original_arguments: &[Value],
) -> Result<crate::engine::vm::driver::RunningExit, Error> {
    let (runtime, entry) = prepare(host, input, original_arguments)?;
    crate::engine::vm::driver::execute(
        runtime,
        entry,
        crate::engine::vm::execution::ExecutionLimits::default(),
    )
}

// Preparation does not remain on the native stack during callback reentry.
#[inline(never)]
pub(in crate::engine::vm) fn prepare(
    host: RuntimeVmHost,
    input: CallInput,
    original_arguments: &[Value],
) -> Result<(Runtime, FrameEntry), Error> {
    // Preserve the production entry's dynamic authentication, independently
    // of static publication and of dormant-activation resume validation.
    if host.executable.root().is_none() {
        return Err(Error::internal(
            "unpublished host cannot execute published code",
        ));
    }
    if host.closure_slots.len() != usize::from(host.executable.metadata.closure_count) {
        return Err(Error::internal(
            "function object closure slot count does not match bytecode metadata",
        ));
    }
    if host.actual_argument_count != original_arguments.len() {
        return Err(Error::internal(
            "owned original arguments disagree with call arity",
        ));
    }
    let mut original_snapshot = Vec::new();
    original_snapshot
        .try_reserve_exact(original_arguments.len())
        .map_err(|_| Error::internal("original argument snapshot allocation failed"))?;
    for argument in original_arguments {
        original_snapshot.push(copy_value(argument)?);
    }
    let original_arguments = original_snapshot;
    let RuntimeVmHost {
        runtime,
        active_frame_token,
        current_realm: _,
        caller_realm,
        executable,
        current_function,
        actual_argument_count: _,
        closure_slots,
        arguments,
        locals,
        reusable_captured_locals,
    } = host;
    let function = current_function
        .ok_or_else(|| Error::internal("published frame has no current function"))?;
    let entry = FrameEntry {
        initialize_bindings: false,
        executable,
        property_generation: 0,
        iterator_generation: 0,
        caller_realm: caller_realm,
        active_frame: active_frame_token,
        cold: crate::engine::vm::frame::ColdFrame::new(FrameCold {
            rare: std::cell::OnceCell::new(),
            return_to: None,
            entry_guard: None,
            function: (function).into(),
            closure_slots,
            reusable_captured_locals,
            input: (input).into(),
        }),
        storage: FrameStorage {
            original_arguments,
            parameters: arguments,
            locals,
            operands: Vec::new(),
        },
    };
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_call_storage(
        size_of::<crate::engine::vm::frame::FrameBody>(),
        entry.cold.reusable_captured_locals.capacity() * size_of::<bool>(),
        entry.storage.original_arguments.capacity() * size_of::<Value>(),
    );
    Ok((runtime, entry))
}

/// Execute only the detached frame supplied by the driver. This adapter does
/// not own or clear the caller's RunningExecution or any parent window.
pub(in crate::engine::vm) fn execute_frame(
    runtime: Runtime,
    entry: FrameEntry,
    resume_pc: usize,
) -> Result<Completion, Error> {
    let (mut host, activation, _original_arguments) = detach_frame(runtime, entry, resume_pc)?;
    let code = host.executable.code.clone();
    activation.execute(&code, &mut host)
}

/// The remaining S07 opcode bridge must return either terminal form. It may
/// not turn an authored await into an execute-to-completion invariant error.
pub(in crate::engine::vm) fn run_frame(
    runtime: Runtime,
    entry: FrameEntry,
    resume_pc: usize,
) -> Result<super::super::suspend::VmRunOutcome, Error> {
    let (mut host, activation, originals) = detach_frame(runtime, entry, resume_pc)?;
    let code = host.executable.code.clone();
    match activation.run(&code, &mut host)? {
        super::super::VmExit::Complete(completion) => {
            Ok(super::super::suspend::VmRunOutcome::Complete(completion))
        }
        super::super::VmExit::Suspend(suspension) => {
            super::super::suspend::finish_suspension(host, suspension, originals)
                .map_err(super::super::exception::runtime_error_to_vm_error)
        }
    }
}

pub(in crate::engine::vm) fn detach_frame(
    runtime: Runtime,
    entry: FrameEntry,
    resume_pc: usize,
) -> Result<(RuntimeVmHost, VmActivation, Vec<Value>), Error> {
    let FrameEntry {
        property_generation: _,
        iterator_generation: _,
        caller_realm,
        active_frame,
        initialize_bindings: _,
        executable,
        cold,
        storage,
    } = entry;
    executable
        .ensure_root(&runtime)
        .map_err(super::super::exception::runtime_error_to_vm_error)?;
    let has_pending_query = cold.has_pending_query();
    let FrameCold {
        rare,
        function,
        closure_slots,
        mut reusable_captured_locals,
        input,
        ..
    } = cold.into_inner();
    let crate::engine::vm::frame::FrameRare {
        regions,
        normalized_this,
        resume_throw,
        iterator_wait,
        eval_arguments,
        conversion,
        ..
    } = *rare.into_inner().unwrap_or_default();
    if resume_throw.is_some()
        || has_pending_query
        || conversion.is_some()
        || iterator_wait.is_some()
        || eval_arguments.is_some()
    {
        return Err(Error::internal(
            "pending conversion cannot hand off its frame",
        ));
    }
    let function = function.into_inner();
    let mut input = input.into_inner();
    let callee_global = input.callee_global(&runtime, executable.realm)?.clone();
    let mut activation = VmActivation::new_in_realm(
        executable.frame_layout(),
        caller_realm,
        executable.realm,
        function.clone(),
        input.this_value,
        input.new_target,
        callee_global,
    );
    activation.regions = regions;
    activation.normalized_this = normalized_this;
    activation.stack = storage.operands;
    activation.pc = resume_pc;
    // RuntimeVmHost is a cold fully materialized representation. Freeze and
    // legacy host helpers require one flag per local even when ordinary frames
    // proved that no local can be captured and omitted the vector entirely.
    if reusable_captured_locals.is_empty() && !executable.has_captured_locals {
        reusable_captured_locals.resize(storage.locals.len(), false);
    }
    let host = RuntimeVmHost {
        runtime,
        active_frame_token: active_frame,
        current_realm: executable.realm,
        caller_realm,
        executable: executable,
        current_function: Some(function),
        actual_argument_count: storage.original_arguments.len(),
        closure_slots,
        arguments: storage.parameters,
        locals: storage.locals,
        reusable_captured_locals,
    };
    Ok((host, activation, storage.original_arguments))
}
