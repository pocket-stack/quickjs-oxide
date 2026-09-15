//! Owned activation protocol (S10).
//!
//! I1: only authenticated OrdinaryCall witnesses install lazy frames.
//! I2: FrameStore fault PCs are authoritative; observation materializes the
//!     new suffix and refreshes its nearest already-registered ancestor.
//! I3: the materialized watermark never exceeds depth. Native argv uses the
//!     registry depth plus the incrementally maintained unregistered depth.
//! I4: unregistered frames own no registry guard; registered guards retire
//!     child-before-parent in both explicit pop and FrameStore::drop.
//! I5: materialization follows domain/operand checks and precedes observers;
//!     a Materialize run exit occurs before any operand is consumed.
//!
//! Generic cold driver operations conservatively observe (they can allocate
//! errors, release owners, or invoke JS). Ordinary Call/Return do not. Native
//! entry, suspend and run-owned release/error paths explicitly observe too.

#[cfg(all(test, feature = "profiling"))]
mod tests {
    use crate::engine::api::{Runtime, Value, profiling::CostProfile};

    #[test]
    fn ordinary_number_calls_do_not_materialize_and_authenticate_once() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("function lazyLeaf(x) { return x; }").unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval("var lazyTotal=0; for(var i=0;i<20;i++) lazyTotal=lazyLeaf(i); lazyTotal")
                .unwrap(),
            Value::number(19.0)
        );
        let cost = profile.snapshot();
        let events = &cost.owned_execution_events;
        assert_eq!(
            events.get("lazy_frame_materialized").copied().unwrap_or(0),
            0,
            "{events:?}"
        );
        assert_eq!(
            events
                .get("ordinary_call_authenticated")
                .copied()
                .unwrap_or(0),
            1,
            "{events:?}"
        );
        assert_eq!(
            events
                .get("ordinary_call_auth_cache_hit")
                .copied()
                .unwrap_or(0),
            19,
            "{events:?}"
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    #[test]
    fn native_error_observes_lazy_ancestors_and_unwind_cleans_registry() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context.eval("function lazyOuter(){return lazyInner()} function lazyInner(){return new Error('observed').stack} var s=lazyOuter(); s.includes('lazyOuter') && s.includes('lazyInner')").unwrap();
        assert_eq!(value, Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
        assert_eq!(context.eval("function a(){return b()} function b(){return Symbol()-1} try{a()}catch(e){e instanceof TypeError && e.stack.includes('a') && e.stack.includes('b')}").unwrap(), Value::Bool(true));
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }
}

#[cfg(all(test, feature = "profiling"))]
mod observation_tests;
