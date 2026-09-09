use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::value::Value;

impl Runtime {
    pub(crate) fn set_pending_exception(&self, value: Value) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        self.validate_value_domain(&value, "exception value")?;
        let raw = self.raw_property_value(&value)?;
        {
            let mut state = self.0.state.borrow_mut();
            state.retain_raw_root(&raw)?;
            if let Some(previous) = state.pending_exception.replace(raw) {
                state.release_owned_raw_root(previous)?;
            }
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
