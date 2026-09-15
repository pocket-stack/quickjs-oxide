//! Reusable empty cold allocations. A cached allocation contains no JS owner.
use super::FrameCold;
use crate::engine::api::error::Error;
use std::ops::{Deref, DerefMut};

/// Keep owners first: abort drops activation/function/input before releasing
/// the executable publication, matching the prior separate-frame drop order.
/// Run splits these three fields once, so its slot borrow never aliases owners.
pub(in crate::engine::vm) struct FrameBody {
    pub owners: FrameCold,
    pub executable: Resident<crate::engine::code::runtime::PublishedFunctionSnapshot>,
    pub window: Resident<crate::engine::vm::stack::FrameWindow>,
}
impl Deref for FrameBody {
    type Target = FrameCold;
    fn deref(&self) -> &FrameCold {
        &self.owners
    }
}
impl DerefMut for FrameBody {
    fn deref_mut(&mut self) -> &mut FrameCold {
        &mut self.owners
    }
}
pub(in crate::engine::vm) struct ColdFrame(Box<FrameBody>);
impl ColdFrame {
    pub(in crate::engine::vm) fn new(frame: FrameCold) -> Self {
        let cold = Self(Box::new(FrameBody {
            executable: Default::default(),
            window: Default::default(),
            owners: frame,
        }));
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "cold.frame_box",
            0,
            1,
            size_of::<FrameBody>(),
        );
        cold
    }
    pub(in crate::engine::vm) fn into_inner(self) -> FrameCold {
        self.0.owners
    }
}
impl Deref for ColdFrame {
    type Target = FrameBody;
    fn deref(&self) -> &FrameBody {
        self.0.as_ref()
    }
}
impl DerefMut for ColdFrame {
    fn deref_mut(&mut self) -> &mut FrameBody {
        self.0.as_mut()
    }
}

#[derive(Default)]
pub(in crate::engine::vm) struct CallStorage {
    prepared_depth: usize,
    empty_frames: Vec<Box<FrameBody>>,
    capture_flags: Vec<Vec<bool>>,
    regions: Vec<Vec<crate::engine::vm::VmUnwindRegion>>,
}
impl CallStorage {
    /// Reserve recycler metadata while all source owners are still available.
    pub(in crate::engine::vm) fn reserve(&mut self) -> Result<(), Error> {
        self.reserve_depth(1)
    }
    /// The high-water mark is simultaneous frame depth, not cumulative calls.
    pub(in crate::engine::vm) fn reserve_depth(&mut self, depth: usize) -> Result<(), Error> {
        if depth <= self.prepared_depth {
            return Ok(());
        }
        #[cfg(feature = "profiling")]
        let before = self.empty_frames.capacity();
        self.empty_frames
            .try_reserve(depth.saturating_sub(self.empty_frames.len()))
            .map_err(|_| Error::internal("cold frame recycler allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "cold.empty_pool",
            before,
            self.empty_frames.capacity(),
            size_of::<Box<FrameBody>>(),
        );
        #[cfg(feature = "profiling")]
        let before = self.capture_flags.capacity();
        self.capture_flags
            .try_reserve(depth.saturating_sub(self.capture_flags.len()))
            .map_err(|_| Error::internal("capture recycler allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "cold.capture_pool",
            before,
            self.capture_flags.capacity(),
            size_of::<Vec<bool>>(),
        );
        #[cfg(feature = "profiling")]
        let before = self.regions.capacity();
        self.regions
            .try_reserve(depth.saturating_sub(self.regions.len()))
            .map_err(|_| Error::internal("region recycler allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "cold.region_pool",
            before,
            self.regions.capacity(),
            size_of::<Vec<crate::engine::vm::VmUnwindRegion>>(),
        );
        self.prepared_depth = depth;
        Ok(())
    }
    pub(in crate::engine::vm) fn capture_flags(
        &mut self,
        count: usize,
    ) -> Result<(Vec<bool>, usize), Error> {
        let mut flags = self.capture_flags.pop().unwrap_or_default();
        let before = flags.capacity();
        flags
            .try_reserve(count)
            .map_err(|_| Error::internal("capture flags allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_call_buffer_capacity(
            "cold.capture_flags",
            before,
            flags.capacity(),
            size_of::<bool>(),
        );
        flags.resize(count, false);
        let grown = if flags.capacity() > before {
            flags.capacity()
        } else {
            0
        };
        #[cfg(feature = "profiling")]
        if grown != 0 {
            crate::engine::api::profiling::record_owned_execution_event(
                "call_capture_flags_capacity_growth",
            );
        }
        Ok((flags, grown))
    }
    pub(in crate::engine::vm) fn install(&mut self, mut frame: FrameCold) -> (ColdFrame, usize) {
        if let Some(rare) = frame.rare.get_mut() {
            if rare.regions.is_empty() {
                if let Some(regions) = self.regions.pop() {
                    if regions.capacity() > rare.regions.capacity() {
                        rare.regions = regions;
                        #[cfg(feature = "profiling")]
                        crate::engine::api::profiling::record_owned_execution_event(
                            "call_region_buffer_reused",
                        );
                    }
                }
            }
        }
        if let Some(mut empty) = self.empty_frames.pop() {
            if frame.rare.get().is_none() {
                frame.rare = std::mem::take(&mut empty.rare);
            }
            empty.owners = frame;
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event("call_cold_frame_reused");
            (ColdFrame(empty), 0)
        } else {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "call_cold_frame_allocated",
            );
            (ColdFrame::new(frame), size_of::<FrameBody>())
        }
    }
    pub(in crate::engine::vm) fn recycle(&mut self, mut cold: ColdFrame) {
        let mut flags = std::mem::take(&mut cold.reusable_captured_locals);
        flags.clear();

        // Drop owners in their resident allocation; never take the whole frame
        // onto the native stack just to destroy it.
        if let Some(rare) = cold.rare.get_mut() {
            rare.property_wait = None;
            rare.iterator_wait = None;
            rare.resume_throw = None;
            rare.regions.clear();
            rare.eval_arguments = None;
            rare.constructor_return = None;
            rare.conversion = None;
            rare.normalized_this = None;
        }

        cold.window.0 = None;
        cold.return_to = None;
        cold.entry_guard = None;
        cold.function.0 = None;
        cold.closure_slots = Default::default();
        cold.input.0 = None;
        cold.executable.0 = None;
        if flags.capacity() != 0 && self.capture_flags.len() < self.capture_flags.capacity() {
            self.capture_flags.push(flags);
        }

        if self.empty_frames.len() < self.empty_frames.capacity() {
            self.empty_frames.push(cold.0);
        }
    }
}

#[cfg(all(test, feature = "profiling"))]
mod tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn narrow_header_reuses_body_allocation_without_retaining_previous_owners() {
        use super::{CallStorage, FrameBody};
        use crate::engine::vm::{
            CallInput,
            stack::{FrameStorage, SlotStore},
        };
        assert!(size_of::<crate::engine::vm::frame::Frame>() <= 64);
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut storage = CallStorage::default();
        storage.reserve_depth(1).unwrap();
        let (mut cold, _) = storage.vacant(context.realm_id());
        let address = (&*cold) as *const FrameBody;
        let executable = crate::engine::code::runtime::PublishedFunctionSnapshot::empty_for_test(
            context.realm_id(),
        );
        let mut slots = SlotStore::new(16);
        let window = slots
            .push_frame(
                &executable.frame_layout(),
                FrameStorage {
                    original_arguments: Vec::new(),
                    parameters: Vec::new(),
                    locals: Vec::new(),
                    operands: Vec::new(),
                },
            )
            .unwrap();
        let function = runtime.new_object(None).unwrap();
        let id = function.object_id();
        cold.function = function.into();
        cold.input = CallInput {
            this_value: Value::Undefined,
            new_target: Value::Undefined,
            callee_global: None,
        }
        .into();
        cold.executable = executable.into();
        cold.window = window.into();
        slots.clear_frame(cold.window.take()).unwrap();
        storage.recycle(cold);
        assert!(runtime.0.state.borrow().heap.object(id).is_err());
        let cached = &storage.empty_frames[0];
        assert!(cached.executable.0.is_none() && cached.window.0.is_none());
        assert!(cached.owners.function.0.is_none() && cached.owners.input.0.is_none());
        let (reused, bytes) = storage.vacant(context.realm_id());
        assert_eq!((&*reused) as *const FrameBody, address);
        assert_eq!(bytes, 0);
    }

    #[test]
    fn repeated_calls_reuse_empty_buffers_at_stable_depth() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval("function inner(a,b){var c=a+b;return c} function outer(a){return inner(a,1)}")
            .unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval("var sum=0;for(var i=0;i<1000;i++)sum+=outer(i);sum")
                .unwrap(),
            Value::Int(500500)
        );
        let snapshot = profile.snapshot();
        assert_eq!(snapshot.call_preparation.parameter_buffer_allocations, 0);
        assert!(snapshot.call_preparation.owned_frame_allocations <= 3);
        assert_eq!(
            snapshot.call_preparation.owned_captured_reuse_allocations,
            0
        );
        assert!(snapshot.owned_execution_events["call_cold_frame_reused"] >= 1998);
        for name in [
            "cold.empty_pool",
            "cold.capture_pool",
            "cold.region_pool",
            "cold.frame_box",
        ] {
            let cost = &snapshot.call_buffers[name];
            assert!(cost.capacity_growths > 0, "{name}");
            assert!(cost.capacity_growth_bytes > 0, "{name}");
        }
        assert!(!snapshot.call_buffers.contains_key("cold.capture_flags"));
        assert!(snapshot.call_buffers["cold.frame_box"].capacity_growths <= 3);
        let metadata = &snapshot.call_buffers["executable.published_data_rc"];
        assert!(metadata.capacity_growths <= 3);
        assert!(snapshot.owned_execution_events["ordinary_call_auth_cache_hit"] >= 1998);

        assert!(snapshot.owned_execution_events["call_bindings_initialized_in_window"] >= 2000);
        assert!(
            snapshot
                .owned_execution_events
                .get("call_outgoing_buffer_capacity_growth")
                .copied()
                .unwrap_or(0)
                <= 2
        );
        assert!(snapshot.owned_execution_events["call_outgoing_tail_transferred"] >= 2000);
        assert_eq!(snapshot.legacy_dispatches, 0);
    }
    #[test]
    fn repeated_deep_calls_reuse_the_peak_cold_capacity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context
            .eval("function descend(n){var marker=n;if(n)return descend(n-1);return marker}")
            .unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval("var total=0;for(var i=0;i<20;i++)total+=descend(20);total")
                .unwrap(),
            Value::Int(0)
        );
        let snapshot = profile.snapshot();
        assert!(snapshot.call_preparation.owned_frame_allocations <= 22);
        assert!(snapshot.owned_execution_events["call_cold_frame_reused"] >= 399);
        assert_eq!(snapshot.call_preparation.parameter_buffer_allocations, 0);
    }
    #[test]
    fn repeated_try_finally_calls_reuse_unwind_region_capacity() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        context.eval("function guarded(n){try {if(n%2)throw n;return n;}catch(e){return e;}finally{n++;}}").unwrap();
        let profile = CostProfile::start();
        assert_eq!(
            context
                .eval("var total=0;for(var i=0;i<100;i++)total+=guarded(i);total")
                .unwrap(),
            Value::Int(4950)
        );
        let snapshot = profile.snapshot();
        assert!(snapshot.owned_execution_events["call_region_buffer_reused"] >= 99);
        assert!(snapshot.call_preparation.owned_frame_allocations <= 2);
    }
}

/// An empty cached allocation has no roots. Running frames have initialized
/// owners; field-level replacement avoids constructing/moving the whole frame.
pub(in crate::engine::vm) struct Resident<T>(Option<T>);
impl<T> Default for Resident<T> {
    fn default() -> Self {
        Self(None)
    }
}
impl<T> From<T> for Resident<T> {
    fn from(value: T) -> Self {
        Self(Some(value))
    }
}
impl<T> Resident<T> {
    pub(in crate::engine::vm) fn take(&mut self) -> T {
        self.0.take().expect("resident owner already taken")
    }
    pub(in crate::engine::vm) fn into_inner(self) -> T {
        self.0.unwrap()
    }
}
impl<T> Deref for Resident<T> {
    type Target = T;
    fn deref(&self) -> &T {
        self.0
            .as_ref()
            .expect("empty cached frame is not executable")
    }
}
impl<T> DerefMut for Resident<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.0
            .as_mut()
            .expect("empty cached frame is not executable")
    }
}
impl CallStorage {
    pub(in crate::engine::vm) fn vacant(
        &mut self,
        _realm: crate::engine::heap::ContextId,
    ) -> (ColdFrame, usize) {
        if let Some(frame) = self.empty_frames.pop() {
            #[cfg(feature = "profiling")]
            if frame
                .rare
                .get()
                .is_some_and(|rare| rare.regions.capacity() > 0)
            {
                crate::engine::api::profiling::record_owned_execution_event(
                    "call_region_buffer_reused",
                );
            }
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event("call_cold_frame_reused");
            (ColdFrame(frame), 0)
        } else {
            #[cfg(feature = "profiling")]
            crate::engine::api::profiling::record_owned_execution_event(
                "call_cold_frame_allocated",
            );
            (
                ColdFrame::new(FrameCold {
                    rare: Default::default(),
                    return_to: None,
                    entry_guard: None,
                    function: Resident(None),
                    input: Resident(None),
                    closure_slots: Default::default(),
                    reusable_captured_locals: Vec::new(),
                }),
                size_of::<FrameBody>(),
            )
        }
    }
}

#[cfg(test)]
mod lazy_tests {
    use super::*;
    use crate::engine::api::Runtime;
    use crate::engine::value::Value;
    use crate::engine::vm::CallInput;

    #[test]
    fn reserve_watermark_reuses_all_three_pools_without_touching_contents() {
        let mut storage = CallStorage::default();
        storage.reserve_depth(7).unwrap();
        let capacities = (
            storage.empty_frames.capacity(),
            storage.capture_flags.capacity(),
            storage.regions.capacity(),
        );
        storage.capture_flags.push(vec![true; 3]);
        for depth in [0, 1, 7, 2] {
            storage.reserve_depth(depth).unwrap();
        }
        assert_eq!(storage.prepared_depth, 7);
        assert_eq!(
            capacities,
            (
                storage.empty_frames.capacity(),
                storage.capture_flags.capacity(),
                storage.regions.capacity()
            )
        );
        assert_eq!(storage.capture_flags, [vec![true; 3]]);
        storage.reserve_depth(11).unwrap();
        assert_eq!(storage.prepared_depth, 11);
        assert!(
            storage.empty_frames.capacity() >= 11
                && storage.capture_flags.capacity() >= 11
                && storage.regions.capacity() >= 11
        );
    }

    #[test]
    fn vacant_frame_and_unused_global_keep_lazy_storage_empty() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut storage = CallStorage::default();
        storage.reserve().unwrap();
        let (mut cold, _) = storage.vacant(context.realm);
        cold.input = CallInput {
            this_value: Value::Undefined,
            new_target: Value::Undefined,
            callee_global: None,
        }
        .into();
        assert!(cold.rare.get().is_none());
        assert!(cold.input.callee_global.is_none());
        let expected = runtime.global_object_for_realm(context.realm).unwrap();
        let first = cold
            .input
            .callee_global(&runtime, context.realm)
            .unwrap()
            .object_id();
        assert_eq!(first, expected.object_id());
        assert_eq!(
            cold.input
                .callee_global(&runtime, context.realm)
                .unwrap()
                .object_id(),
            first
        );
        assert!(cold.rare.get().is_none());
        storage.recycle(cold);
        assert!(storage.capture_flags.is_empty());
        let (cold, _) = storage.vacant(context.realm);
        assert!(cold.rare.get().is_none());
    }

    #[test]
    fn lazy_this_preserves_strict_sloppy_eval_and_captured_scope_behavior() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval("function sloppy(){return this === globalThis} function strict(){'use strict';return this===undefined} function direct(){return eval('this===globalThis')} sloppy() && strict() && direct()").unwrap(), Value::Bool(true));
        assert_eq!(context.eval("function scoped(){let a=[];for(let i=0;i<3;i++){let x=i;a.push(()=>x)}return a[0]()+a[1]()+a[2]()} scoped()+scoped()").unwrap(), Value::Int(6));
        assert_eq!(context.eval("function plain(){let sum=0;for(let i=0;i<3;i++){let x=i;sum+=x}return sum} plain()+plain()").unwrap(), Value::Int(6));
    }
}
