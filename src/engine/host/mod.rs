//! Embedder-provided services at the runtime's host boundary.
//!
//! QuickJS keeps ECMAScript algorithms inside the engine while delegating a
//! small set of inherently host-dependent values. Keeping those values behind
//! one synchronous interface lets native and WebAssembly embedders provide
//! their own clock, local-time rules, and entropy without changing engine
//! logic.

/// Synchronous host services owned by one runtime.
///
/// `quickjs-oxide` is currently single-threaded, so implementations do not
/// need `Send` or `Sync`. The runtime may call these methods while creating a
/// context or executing JavaScript; implementations must therefore avoid
/// re-entering the same runtime.
pub trait HostServices: std::fmt::Debug {
    /// Receive the qjs helper output after engine-side value formatting.
    fn write_output(&self, _bytes: &[u8], _flush: bool) {}

    /// Return milliseconds since the Unix epoch.
    ///
    /// This is the host value observed by `Date.now()` and zero-argument Date
    /// construction. The engine performs the remaining ECMAScript Date
    /// algorithms itself.
    fn now_millis(&self) -> i64;

    /// Return UTC minus local time, in minutes, at `epoch_millis`.
    ///
    /// The sign matches ECMAScript `Date.prototype.getTimezoneOffset`.
    fn timezone_offset_minutes(&self, epoch_millis: i64) -> i32;

    /// Return the initial state for one context's `Math.random` stream.
    ///
    /// Pinned QuickJS seeds each context from host microsecond time. Embedders
    /// may use another host entropy source. As in QuickJS, the engine replaces
    /// a zero seed with one before producing a value.
    fn random_seed(&self) -> u64;
}

// Unit tests link the native adapter against Cargo's non-test engine instance.
// Its trait identity differs from this unit-test crate's HostServices, so
// forward the same provider here without introducing another implementation.
#[cfg(test)]
impl HostServices for quickjs_oxide_host::SystemHostServices {
    fn write_output(&self, bytes: &[u8], flush: bool) {
        quickjs_oxide_host::engine_test::HostServices::write_output(self, bytes, flush);
    }
    fn now_millis(&self) -> i64 {
        quickjs_oxide_host::engine_test::HostServices::now_millis(self)
    }
    fn timezone_offset_minutes(&self, epoch_millis: i64) -> i32 {
        quickjs_oxide_host::engine_test::HostServices::timezone_offset_minutes(self, epoch_millis)
    }
    fn random_seed(&self) -> u64 {
        quickjs_oxide_host::engine_test::HostServices::random_seed(self)
    }
}

/// Enter only around an actual embedder callback. Domain algorithms keep their
/// JavaScript continuations in the driver and must never use this boundary.
impl crate::engine::api::runtime::Runtime {
    pub(crate) fn with_host_callback<T>(
        &self,
        callback: impl FnOnce() -> T,
    ) -> Result<T, crate::engine::api::runtime_error::RuntimeError> {
        use crate::engine::api::{error::Error, runtime_error::RuntimeError};
        let _stack = crate::engine::vm::native_stack::ModuleHostCallbackGuard::enter(self)
            .map_err(|_| RuntimeError::Engine(Error::internal("stack overflow")))?;
        #[cfg(feature = "stack-vm")]
        let boundary =
            crate::engine::vm::HostBoundaryGuard::enter(self).map_err(RuntimeError::Engine)?;
        let result = callback();
        #[cfg(feature = "stack-vm")]
        boundary.finish(self).map_err(RuntimeError::Engine)?;
        Ok(result)
    }
}
