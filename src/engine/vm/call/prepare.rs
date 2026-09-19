//! Bytecode frame preparation owned by the call layer.
//! Publication remains authoritative; parameter padding, lexical initialization
//! and the named-function binding retain their existing ordering and semantics.

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::code::runtime::{PublishedFunctionData, PublishedFunctionSnapshot};
use crate::engine::object::CallableRef;
use crate::engine::value::{JsValue, Value};
use crate::engine::vm::CallInput;
use crate::engine::vm::bindings::FrameBinding;
use crate::engine::vm::frames::ActiveFrameGuard;

#[cfg(test)]
pub(in crate::engine::vm) struct PreparedBytecodeFrame {
    pub executable: PublishedFunctionSnapshot,
    pub active_frame: ActiveFrameGuard,
    pub input: CallInput,
    pub arguments: Vec<FrameBinding>,
    pub locals: Vec<FrameBinding>,
}

/// Common validated call owners, without legacy binding-buffer headers.
pub(in crate::engine::vm) struct PreparedBytecodeHeader {
    pub executable: PublishedFunctionSnapshot,
    pub active_frame: ActiveFrameGuard,
    pub input: CallInput,
}

impl Runtime {
    #[cfg(test)]
    pub(in crate::engine::vm) fn prepare_bytecode_frame(
        &self,
        callable: &CallableRef,
        this_value: Value,
        new_target: Value,
        arguments: &[Value],
        bytecode: FunctionBytecodeRef,
    ) -> Result<PreparedBytecodeFrame, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _profile_phase =
            crate::engine::api::profiling::PhaseTimer::start_vm("bytecode.prepare");
        let PreparedBytecodeHeader {
            executable,
            active_frame,
            input,
        } = self.prepare_bytecode_header(callable, this_value, new_target, bytecode)?;
        let local_definitions = &executable.local_definitions;
        let metadata = executable.metadata;
        let argument_slots = executable.frame_layout().argument_slots(arguments.len());
        let mut frame_arguments = Vec::new();
        let mut frame_locals = Vec::new();
        frame_arguments.reserve(argument_slots);
        frame_arguments.extend(arguments.iter().cloned().map(FrameBinding::Direct));
        frame_arguments.resize_with(argument_slots, || FrameBinding::Direct(Value::Undefined));
        frame_locals.reserve(local_definitions.len());
        frame_locals.extend(
            local_definitions
                .iter()
                .enumerate()
                .map(|(index, definition)| {
                    initial_local_binding(
                        self,
                        definition.is_lexical,
                        metadata.function_name_local == Some(index as u16),
                        callable.as_object(),
                    )
                })
                .collect::<Result<Vec<_>, _>>()?,
        );
        #[cfg(feature = "profiling")]
        if crate::engine::api::profiling::cost_profile_active() {
            crate::engine::api::profiling::record_call_preparation(
                argument_slots,
                frame_arguments.capacity() * size_of::<FrameBinding>(),
                local_definitions.len(),
                frame_locals.capacity() * size_of::<FrameBinding>(),
                arguments.len(),
                arguments
                    .iter()
                    .filter(|value| matches!(value, Value::Object(_) | Value::Symbol(_)))
                    .count(),
                1 + usize::from(metadata.function_name_local.is_some()),
            );
        }
        Ok(PreparedBytecodeFrame {
            executable,
            active_frame,
            input,
            arguments: frame_arguments,
            locals: frame_locals,
        })
    }

    pub(in crate::engine::vm) fn prepare_owned_bytecode_frame(
        &self,
        callable: &CallableRef,
        this_value: JsValue,
        new_target: JsValue,
        bytecode: FunctionBytecodeRef,
    ) -> Result<PreparedBytecodeHeader, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _profile_phase =
            crate::engine::api::profiling::PhaseTimer::start_vm("bytecode.prepare");
        self.prepare_bytecode_header(callable, this_value, new_target, bytecode)
    }

    fn prepare_bytecode_header(
        &self,
        callable: &CallableRef,
        this_value: JsValue,
        new_target: JsValue,
        bytecode: FunctionBytecodeRef,
    ) -> Result<PreparedBytecodeHeader, RuntimeError> {
        let executable = self.snapshot_function_bytecode(&bytecode)?;
        let PublishedFunctionData {
            local_definitions,
            metadata,
            realm,
            ..
        } = &*executable;
        let metadata = *metadata;
        let realm = *realm;
        let callee_global = self.global_object_for_realm(realm)?;
        let active_frame = self.push_bytecode_active_frame(
            callable.as_object().clone(),
            bytecode,
            realm,
            metadata.strict,
        )?;
        if metadata
            .function_name_local
            .is_some_and(|index| usize::from(index) >= local_definitions.len())
        {
            return Err(RuntimeError::Invariant(
                "function-name local is outside the frame",
            ));
        }
        Ok(PreparedBytecodeHeader {
            executable,
            active_frame,
            input: CallInput {
                this_value,
                new_target,
                callee_global: Some(callee_global),
            },
        })
    }
}

/// Shared initial binding shape for both legacy vectors and owned windows.
/// The named-function binding duplicates the callable's edge for the frame.
pub(in crate::engine::vm) fn initial_local_binding(
    runtime: &Runtime,
    lexical: bool,
    function_name: bool,
    callable: &crate::engine::object::ObjectRef,
) -> Result<FrameBinding, RuntimeError> {
    if function_name {
        let id = callable.object_id();
        runtime.retain_object_handle(id)?;
        Ok(FrameBinding::Direct(JsValue::Object(id)))
    } else if lexical {
        FrameBinding::Uninitialized
    } else {
        FrameBinding::Direct(Value::Undefined)
    }
}
