//! Direct published root-call preparation for the sole execution core.
use super::{
    Completion,
    frame::{FrameCold, FrameEntry},
    stack::{FrameStorage, copy_value},
};
use crate::engine::api::{Error, runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::code::function::metadata::FunctionKind;
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::ContextId;
use crate::engine::object::CallableRef;
use crate::engine::value::Value;
impl Runtime {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn execute_bytecode_callable(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
        this_value: Value,
        new_target: Value,
        arguments: &[Value],
        bytecode: FunctionBytecodeRef,
        closure_slots: crate::engine::vm::closure::ClosureSlots,
    ) -> Result<Completion, RuntimeError> {
        if self.bytecode_call_would_overflow() {
            return self.bytecode_stack_overflow_completion(caller_realm, &bytecode);
        }
        if !bytecode.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let entry = prepare_call(
            self,
            caller_realm,
            callable,
            this_value,
            new_target,
            arguments,
            bytecode,
            closure_slots,
        )?;
        let metadata = entry.executable.metadata;
        let module_link = metadata.is_module
            && matches!(
                entry.cold.input.this_value,
                crate::engine::value::JsValue::Bool(true)
            );
        if metadata.function_kind == FunctionKind::Async && !module_link {
            return self.start_async_bytecode_callable(caller_realm, entry);
        }
        let result = super::driver::execute(
            self.clone(),
            entry,
            super::execution::ExecutionLimits::for_runtime(self),
        )
        .map_err(RuntimeError::Engine)?;
        if matches!(
            metadata.function_kind,
            FunctionKind::Generator | FunctionKind::AsyncGenerator
        ) {
            return super::suspend::creation::GeneratorCreation {
                realm: caller_realm,
                callable: callable.clone(),
                asynchronous: metadata.function_kind == FunctionKind::AsyncGenerator,
            }
            .initial(
                self,
                result
                    .finish_suspending(self.clone())
                    .map_err(RuntimeError::Engine)?,
            )?
            .finish(self, caller_realm);
        }
        result.finish(self.clone()).map_err(RuntimeError::Engine)
    }
}

/// Normal root entries use the same direct window initialization as child
/// calls. The public borrowed argv needs an independent snapshot, but no
/// parameter/local binding vectors are built.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(in crate::engine::vm) fn prepare_call(
    runtime: &Runtime,
    caller_realm: crate::engine::heap::ContextId,
    callable: &crate::engine::object::CallableRef,
    receiver: Value,
    new_target: Value,
    arguments: &[Value],
    bytecode: crate::engine::code::rooted::FunctionBytecodeRef,
    closure_slots: crate::engine::vm::closure::ClosureSlots,
) -> Result<FrameEntry, crate::engine::api::runtime_error::RuntimeError> {
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
    let active_token = prepared.active_frame.token();
    let cold = crate::engine::vm::frame::ColdFrame::new(FrameCold {
        rare: std::cell::OnceCell::new(),
        return_to: None,
        entry_guard: Some(prepared.active_frame),
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
        caller_realm,
        active_frame: active_token,

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
    Ok(entry)
}
