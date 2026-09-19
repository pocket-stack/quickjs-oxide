use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::value::{JsValue, Value};

impl Runtime {
    /// Internal-value form of [`Runtime::set_pending_exception`]: consumes the
    /// value's edges after the pending-exception root has retained its copy.
    pub(crate) fn set_pending_exception_jsvalue(
        &self,
        value: JsValue,
    ) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        let raw = value.as_raw();
        {
            let mut state = self.0.state.borrow_mut();
            state.retain_raw_root(&raw)?;
            if let Some(previous) = state.pending_exception.replace(raw) {
                state.release_owned_raw_root(previous)?;
            }
        }
        // `raw` owns its own retained occurrence; the consumed value's edges
        // are no longer needed.
        self.release_jsvalue(value)?;
        Ok(())
    }

    pub(crate) fn set_pending_exception(&self, value: Value) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "exception value")?;
        let raw = self.raw_property_value(&value)?;
        // The conversion carries one producer-owned string/BigInt node edge;
        // the pending-exception root retains its own occurrence below, so the
        // producer edge is released on every exit.
        let conversion_edge = raw.conversion_node_edge();
        {
            let mut state = self.0.state.borrow_mut();
            if let Err(error) = state.retain_raw_root(&raw) {
                drop(state);
                if let Some(edge) = conversion_edge {
                    self.release_converted_node_edge(edge);
                }
                return Err(error);
            }
            if let Some(previous) = state.pending_exception.replace(raw) {
                state.release_owned_raw_root(previous)?;
            }
        }
        if let Some(edge) = conversion_edge {
            self.release_converted_node_edge(edge);
        }
        // `raw` now owns its own retained occurrence.
        drop(value);
        Ok(())
    }

    pub(crate) fn take_pending_exception(&self) -> Result<Option<Value>, RuntimeError> {
        let _operation = self.operation();
        let pending = self.0.state.borrow_mut().pending_exception.take();
        pending
            .map(|value| self.take_owned_raw_value(value))
            .transpose()
    }

    pub(crate) fn has_pending_exception(&self) -> bool {
        let _operation = self.operation();
        self.0.state.borrow().pending_exception.is_some()
    }
}

pub(in crate::engine::vm) fn runtime_error_to_vm_error(error: RuntimeError) -> Error {
    match error {
        RuntimeError::Engine(error) => error,
        error => Error::internal(error.to_string()),
    }
}

/// Heap retain/release failures at trusted VM sites carry the same internal
/// diagnostic policy as every other runtime failure.
pub(in crate::engine::vm) fn heap_error_to_vm_error(error: crate::engine::heap::HeapError) -> Error {
    runtime_error_to_vm_error(RuntimeError::from(error))
}

/// Materialize published binding diagnostics outside the resident driver frame.
#[inline(never)]
pub(super) fn binding_error(
    runtime: &Runtime,
    execution: &mut super::execution::RunningExecution,
    id: super::frame::FrameId,
    index: u32,
    redeclaration: bool,
) -> Result<super::Completion, Error> {
    use crate::engine::api::error::{ErrorKind, NativeErrorKind};
    use crate::engine::object::PropertyKey;
    let frame = execution.frames.current_mut(id)?;
    let atom = frame
        .executable
        .property_key_atoms
        .as_ref()
        .and_then(|atoms| atoms.get(index as usize))
        .copied()
        .filter(|atom| !atom.is_null())
        .ok_or_else(|| Error::internal("static name opcode has no linked property key"))?;
    let key = PropertyKey::from_borrowed_atom(runtime.clone(), atom)
        .map_err(|error| Error::internal(error.to_string()))?;
    let (kind, native, prefix, suffix) = if redeclaration {
        (
            ErrorKind::Syntax,
            NativeErrorKind::Syntax,
            "redeclaration of '",
            "'",
        )
    } else {
        (
            ErrorKind::Type,
            NativeErrorKind::Type,
            "'",
            "' is read-only",
        )
    };
    let error = runtime
        .native_atom_error(kind, prefix, &key, suffix)
        .map_err(runtime_error_to_vm_error)?;
    let value = runtime
        .new_native_error_from_error_jsvalue(frame.executable.realm, native, &error)
        .map_err(runtime_error_to_vm_error)?;
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_instruction(execution.slots.depth(&frame.window));
    Ok(super::Completion::Throw(value))
}
