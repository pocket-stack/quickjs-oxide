//! One executable owner and one exclusive storage window per running frame.

mod storage;
pub(in crate::engine::vm) use storage::{CallStorage, ColdFrame, FrameBody};

use crate::engine::api::error::Error;
use crate::engine::code::runtime::PublishedFunctionSnapshot;
use crate::engine::heap::ContextId;
use crate::engine::object::ObjectRef;
use crate::engine::value::Value;
use crate::engine::vm::CallInput;
use crate::engine::vm::frames::{ActiveFrameGuard, ActiveFrameToken};
use crate::engine::vm::stack::FrameStorage;

#[derive(Clone, Copy)]
pub(super) enum ReturnValue {
    Push,
    Discard,
}

#[derive(Clone, Copy)]
pub(super) struct ReturnTarget {
    pub value_use: ReturnValue,
    pub owner: ReturnOwner,
    pub tail: bool,
    pub operation: Option<OperationTarget>,
}

/// A continuation may belong to a bytecode frame or a root native/job request.
#[derive(Clone, Copy)]
pub(super) enum ReturnOwner {
    Frame(FrameId),
    Root,
}
impl ReturnOwner {
    pub(super) fn frame(self) -> Result<FrameId, Error> {
        match self {
            Self::Frame(id) => Ok(id),
            Self::Root => Err(Error::internal(
                "root request used a bytecode-only operation",
            )),
        }
    }
}
impl ReturnTarget {
    pub(super) fn frame(self) -> Result<FrameId, Error> {
        self.owner.frame()
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationTarget {
    Conversion(u64),
    PropertyGet(u64),
    Iterator(u64),
    Eval(u16),
}

pub(super) enum ConstructorReturn {
    Base(crate::engine::value::Value),
    Derived,
}

#[derive(Default)]
pub(super) struct FrameRare {
    pub normalized_this: Option<Value>,
    property_wait: Option<Box<super::proxy_get_driver::PendingProxyGet>>,
    pub iterator_wait: Option<crate::engine::vm::iterator_driver::PendingIterator>,
    pub resume_throw: Option<Value>,
    pub regions: Vec<crate::engine::vm::VmUnwindRegion>,
    pub eval_arguments: Option<Vec<crate::engine::value::Value>>,
    pub constructor_return: Option<ConstructorReturn>,
    pub conversion: Option<crate::engine::vm::conversion_driver::ConversionWait>,
}

pub(super) struct FrameCold {
    pub rare: std::cell::OnceCell<Box<FrameRare>>,
    pub return_to: Option<ReturnTarget>,
    pub entry_guard: Option<ActiveFrameGuard>,
    pub function: storage::Resident<ObjectRef>,
    pub closure_slots: crate::engine::vm::closure::ClosureSlots,
    pub reusable_captured_locals: Vec<bool>,
    pub input: storage::Resident<CallInput>,
}

/// Owners crossing the driver boundary before installation or after detachment.
pub(super) struct FrameEntry {
    pub property_generation: u64,
    pub iterator_generation: u64,
    pub caller_realm: ContextId,
    pub active_frame: ActiveFrameToken,

    pub initialize_bindings: bool,
    pub executable: PublishedFunctionSnapshot,
    pub cold: ColdFrame,
    pub storage: FrameStorage,
}

/// Only the dispatch header moves on frame-stack push/pop. The executable and
/// affine window live in the already pooled cold allocation, whose address is
/// stable across vector growth and reuse. No extra per-call allocation exists.
pub(super) struct Frame {
    pub property_generation: u64,
    pub iterator_generation: u64,
    pub caller_realm: ContextId,
    pub active_frame: ActiveFrameToken,

    pub fault_pc: usize,
    pub resume_pc: usize,
    pub cold: ColdFrame,
}
impl std::ops::Deref for Frame {
    type Target = storage::FrameBody;
    fn deref(&self) -> &storage::FrameBody {
        &self.cold
    }
}
impl std::ops::DerefMut for Frame {
    fn deref_mut(&mut self) -> &mut storage::FrameBody {
        &mut self.cold
    }
}
const _: () = assert!(size_of::<Frame>() <= 64);
#[cfg(target_pointer_width = "64")]
const _: () = assert!(size_of::<Frame>() == 56);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FrameId {
    execution: u64,
    generation: u64,
}

pub(super) struct FrameStore {
    frames: Vec<(FrameId, Frame)>,
    execution: u64,
    next_generation: u64,
    limit: usize,
    // Exact sum: at most usize::MAX frames, each charged at most usize::MAX.
    // Wider accounting preserves overflow recovery without rescanning ancestors.
    installed_wait_depth: u128,
    materialized_watermark: usize,
    unmaterialized_depth: usize,
}

impl FrameStore {
    pub(super) fn new(execution: u64, limit: usize) -> Self {
        Self {
            frames: Vec::new(),
            execution,
            next_generation: 1,
            limit,
            installed_wait_depth: 0,
            materialized_watermark: 0,
            unmaterialized_depth: 0,
        }
    }

    pub(super) fn depth(&self) -> usize {
        self.frames.len()
    }

    /// Register only the newly observable suffix; ancestors have been frozen
    /// at their call PC since the previous suffix was materialized.
    pub(super) fn materialize(
        &mut self,
        runtime: &crate::engine::api::runtime::Runtime,
    ) -> Result<(), Error> {
        use super::exception::runtime_error_to_vm_error;
        let depth = self.frames.len();
        let start = self.materialized_watermark.saturating_sub(1);
        for (offset, (_, frame)) in self.frames[start..].iter_mut().enumerate() {
            if frame.active_frame.is_materialized() {
                runtime
                    .publish_materialized_pc(
                        frame.active_frame,
                        frame
                            .cold
                            .entry_guard
                            .as_ref()
                            .map(|guard| guard.registry_depth()),
                        super::BytecodePc::new(frame.fault_pc),
                    )
                    .map_err(runtime_error_to_vm_error)?;
            } else {
                let guard = runtime
                    .materialize_owned_frame(frame)
                    .map_err(runtime_error_to_vm_error)?;
                frame.active_frame = guard.token();
                self.unmaterialized_depth -= 1;
                frame.cold.entry_guard = Some(guard);
            }
            self.materialized_watermark = start + offset + 1;
        }
        self.materialized_watermark = depth;
        Ok(())
    }

    pub(super) fn logical_active_depth(
        &self,
        runtime: &crate::engine::api::runtime::Runtime,
    ) -> usize {
        runtime
            .0
            .active_frame_depth
            .get()
            .saturating_add(self.unmaterialized_depth)
    }

    pub(super) fn can_push(&self) -> bool {
        self.frames.len() < self.limit
    }

    /// Domain continuations replace recursive calls and share the existing
    /// execution depth ceiling, including calls that install no bytecode frame.
    pub(super) fn can_push_with_continuations(&self, pending: usize) -> bool {
        self.frames
            .len()
            .checked_add(pending)
            .and_then(|depth| {
                usize::try_from(self.installed_wait_depth)
                    .ok()
                    .and_then(|wait| depth.checked_add(wait))
            })
            .is_some_and(|depth| depth < self.limit)
    }

    fn remove_installed_wait_depth(&mut self, removed: usize) {
        self.installed_wait_depth -= removed as u128;
    }

    pub(super) fn take_pending(
        &mut self,
        id: FrameId,
    ) -> Result<Box<super::proxy_get_driver::PendingProxyGet>, Error> {
        let pending = self
            .current_mut(id)?
            .cold
            .property_wait
            .take()
            .ok_or_else(|| Error::internal("request reply has no pending operation"))?;
        self.remove_installed_wait_depth(pending.continuation_depth());
        Ok(pending)
    }

    pub(super) fn put_pending(
        &mut self,
        id: FrameId,
        pending: Box<super::proxy_get_driver::PendingProxyGet>,
    ) -> Result<(), Error> {
        let frame = self.current_mut(id)?;
        if frame.cold.property_wait.is_some() {
            return Err(Error::internal("request overwrote a pending reply"));
        }
        let depth = pending.continuation_depth();
        frame.cold.property_wait = Some(pending);
        self.installed_wait_depth += depth as u128;
        Ok(())
    }

    pub(super) fn current_id(&self) -> Option<FrameId> {
        self.frames.last().map(|(id, _)| *id)
    }

    /// Reserve identity and capacity while retaining exclusive access to this
    /// store. Slot publication may then fail, but installing a frame cannot.
    pub(super) fn prepare_push(&mut self) -> Result<FramePush<'_>, Error> {
        if self.frames.len() >= self.limit {
            return Err(Error::internal("execution frame limit exceeded"));
        }
        let next = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| Error::internal("execution frame identity exhausted"))?;
        #[cfg(feature = "profiling")]
        let before = self.frames.capacity();
        self.frames
            .try_reserve(1)
            .map_err(|_| Error::internal("execution frame allocation failed"))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_storage(
            crate::engine::api::profiling::OwnedStorageEvent::FrameCapacity {
                before,
                after: self.frames.capacity(),
            },
        );
        Ok(FramePush { store: self, next })
    }

    #[cfg(test)]
    pub(super) fn push(&mut self, frame: Frame) -> Result<FrameId, Error> {
        Ok(self.prepare_push()?.install(frame))
    }

    pub(super) fn pop_current(&mut self) -> Option<Frame> {
        let (_, frame) = self.frames.pop()?;
        self.materialized_watermark = self.materialized_watermark.min(self.frames.len());
        self.unmaterialized_depth -= usize::from(!frame.active_frame.is_materialized());
        self.remove_installed_wait_depth(frame.cold.pending_depth());
        Some(frame)
    }

    pub(super) fn current_mut(&mut self, id: FrameId) -> Result<&mut Frame, Error> {
        match self.frames.last_mut() {
            Some((current, frame)) if *current == id => Ok(frame),
            _ => Err(Error::internal(
                "frame identity is not the current execution frame",
            )),
        }
    }

    pub(super) fn pop(&mut self, id: FrameId) -> Result<Frame, Error> {
        self.current_mut(id)?;
        Ok(self.pop_current().unwrap())
    }
}

/// The borrow prevents another push/pop from invalidating reserved capacity.
pub(super) struct FramePush<'a> {
    store: &'a mut FrameStore,
    next: u64,
}
impl FramePush<'_> {
    pub(super) fn current_mut(&mut self, id: FrameId) -> Result<&mut Frame, Error> {
        self.store.current_mut(id)
    }
    pub(super) fn install(self, frame: Frame) -> FrameId {
        let id = FrameId {
            execution: self.store.execution,
            generation: self.store.next_generation,
        };
        self.store.next_generation = self.next;
        let depth = frame.cold.pending_depth();
        self.store.unmaterialized_depth += usize::from(!frame.active_frame.is_materialized());
        self.store.frames.push((id, frame));
        self.store.installed_wait_depth += depth as u128;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_storage(
            crate::engine::api::profiling::OwnedStorageEvent::FramePush(self.store.frames.len()),
        );
        id
    }
}
impl Drop for FrameStore {
    fn drop(&mut self) {
        // Child query/activation owners must disappear before their parents.
        while let Some(frame) = self.pop_current() {
            drop(frame);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::Runtime;
    use crate::engine::value::Value;
    use crate::engine::vm::stack::{FrameStorage, SlotStore};

    fn assert_wait_depth_matches_scan(frames: &FrameStore) {
        let sum = frames.frames.iter().try_fold(0usize, |sum, (_, frame)| {
            sum.checked_add(frame.cold.pending_depth())
        });
        assert_eq!(usize::try_from(frames.installed_wait_depth).ok(), sum);
        for pending in [0, 1, 2, 3, 7, usize::MAX - 1, usize::MAX] {
            let original = frames.frames.len().checked_add(pending).and_then(|depth| {
                frames.frames.iter().try_fold(depth, |depth, (_, frame)| {
                    depth.checked_add(frame.cold.pending_depth())
                })
            });
            assert_eq!(
                frames.can_push_with_continuations(pending),
                original.is_some_and(|depth| depth < frames.limit),
            );
        }
    }

    #[test]
    fn installed_wait_cache_matches_scan_across_install_take_and_both_pops() {
        use super::super::proxy_get_driver::PendingProxyGet;
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut frames = FrameStore::new(1, 9);
        assert_wait_depth_matches_scan(&frames);
        let (first, _first_slots) = frame(&runtime, context.realm);
        let first_id = frames.push(first).unwrap();
        frames
            .put_pending(
                first_id,
                PendingProxyGet::with_parent_depth_for_test(context.realm, 3),
            )
            .unwrap();
        assert_wait_depth_matches_scan(&frames);
        assert!(frames.can_push_with_continuations(4));
        assert!(!frames.can_push_with_continuations(5));
        let (mut second, _second_slots) = frame(&runtime, context.realm);
        second.cold.property_wait = Some(PendingProxyGet::with_parent_depth_for_test(
            context.realm,
            4,
        ));
        let second_id = frames.prepare_push().unwrap().install(second);
        assert_wait_depth_matches_scan(&frames);
        assert!(!frames.can_push_with_continuations(0));
        let pending = frames.take_pending(second_id).unwrap();
        assert_eq!(pending.continuation_depth(), 4);
        assert_wait_depth_matches_scan(&frames);
        frames.put_pending(second_id, pending).unwrap();
        assert_wait_depth_matches_scan(&frames);
        drop(frames.pop(second_id).unwrap());
        assert_wait_depth_matches_scan(&frames);
        drop(frames.pop_current().unwrap());
        assert_wait_depth_matches_scan(&frames);
        assert!(frames.pop_current().is_none());
        assert_wait_depth_matches_scan(&frames);
    }

    #[test]
    fn pending_errors_preserve_cache_and_put_does_not_add_a_budget_rejection() {
        use super::super::proxy_get_driver::PendingProxyGet;
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut frames = FrameStore::new(1, 1);
        let (first, _slots) = frame(&runtime, context.realm);
        let id = frames.push(first).unwrap();
        let invalid = FrameId {
            execution: 2,
            generation: id.generation,
        };
        assert!(frames.take_pending(id).is_err());
        assert!(frames.take_pending(invalid).is_err());
        assert!(
            frames
                .put_pending(
                    invalid,
                    PendingProxyGet::with_parent_depth_for_test(context.realm, 2)
                )
                .is_err()
        );
        assert!(frames.pop(invalid).is_err());
        assert_wait_depth_matches_scan(&frames);
        frames
            .put_pending(
                id,
                PendingProxyGet::with_parent_depth_for_test(context.realm, 3),
            )
            .unwrap();
        assert!(!frames.can_push_with_continuations(0));
        assert!(
            frames
                .put_pending(
                    id,
                    PendingProxyGet::with_parent_depth_for_test(context.realm, 5)
                )
                .is_err()
        );
        assert_wait_depth_matches_scan(&frames);
        assert_eq!(frames.take_pending(id).unwrap().continuation_depth(), 3);
        assert_wait_depth_matches_scan(&frames);
        drop(frames.pop_current().unwrap());
        assert!(frames.take_pending(id).is_err());
        assert_wait_depth_matches_scan(&frames);
    }

    #[test]
    fn numeric_installed_wait_overflow_recovers_without_scanning() {
        let mut frames = FrameStore::new(1, usize::MAX);
        frames.installed_wait_depth = usize::MAX as u128 + 1;
        assert!(!frames.can_push_with_continuations(0));
        frames.remove_installed_wait_depth(1);
        assert_eq!(frames.installed_wait_depth, usize::MAX as u128);
        frames.remove_installed_wait_depth(usize::MAX);
        assert!(frames.can_push_with_continuations(0));
    }

    #[test]
    fn cached_cold_storage_reuses_empty_capacity_without_retaining_runtime() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut cache = CallStorage::default();
        cache.reserve().unwrap();
        let (mut first, mut first_slots) = frame(&runtime, context.realm);
        first.cold.reusable_captured_locals = vec![true; 23];
        let address = &*first.cold as *const FrameBody;
        first_slots.clear_frame(first.window.take()).unwrap();
        cache.recycle(first.cold);
        let (flags, grown) = cache.capture_flags(23).unwrap();
        assert_eq!(grown, 0);
        assert_eq!(flags, vec![false; 23]);
        let (mut second, mut second_slots) = frame(&runtime, context.realm);
        second_slots.clear_frame(second.window.take()).unwrap();
        let mut contents = second.cold.into_inner();
        contents.reusable_captured_locals = flags;
        let (cold, allocated) = cache.install(contents);
        assert_eq!(allocated, 0);
        assert_eq!(&*cold as *const FrameBody, address);
        cache.recycle(cold);
        drop((first_slots, second_slots, context, runtime));
        assert!(weak.upgrade().is_none());
        drop(cache);
    }

    fn frame(runtime: &Runtime, realm: ContextId) -> (Frame, SlotStore) {
        let executable = PublishedFunctionSnapshot::empty_for_test(realm);
        let mut slots = SlotStore::new(0);
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
        let mut cold = super::ColdFrame::new(FrameCold {
            rare: std::cell::OnceCell::new(),
            return_to: None,
            entry_guard: None,
            input: (CallInput {
                this_value: Value::Undefined,
                new_target: Value::Undefined,
                callee_global: Some(function.clone()),
            })
            .into(),
            function: (function).into(),
            closure_slots: Default::default(),
            reusable_captured_locals: Vec::new(),
        });
        cold.executable = executable.into();
        cold.window = window.into();
        (
            Frame {
                property_generation: 0,
                iterator_generation: 0,
                caller_realm: realm,
                active_frame: ActiveFrameToken(0),

                fault_pc: 0,
                resume_pc: 0,
                cold,
            },
            slots,
        )
    }

    #[test]
    fn limits_and_stale_ids_preserve_the_active_frame_and_release_rejected_owners() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut frames = FrameStore::new(1, 1);
        let (first, _first_slots) = frame(&runtime, context.realm);
        let first_id = frames.push(first).unwrap();
        let capacity = frames.frames.capacity();
        let (rejected, _rejected_slots) = frame(&runtime, context.realm);
        let rejected_object = rejected.cold.function.object_id();
        assert!(frames.push(rejected).is_err());
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(rejected_object)
                .is_err()
        );
        assert!(frames.current_mut(first_id).is_ok());
        assert_eq!(frames.frames.capacity(), capacity);
        drop(frames.pop(first_id).unwrap());
        let (replacement, _replacement_slots) = frame(&runtime, context.realm);
        let replacement_id = frames.push(replacement).unwrap();
        assert_ne!(first_id, replacement_id);
        assert!(frames.current_mut(first_id).is_err());
        assert!(
            frames
                .current_mut(FrameId {
                    execution: 2,
                    generation: replacement_id.generation
                })
                .is_err()
        );
        assert!(frames.current_mut(replacement_id).is_ok());
        assert_eq!(frames.frames.capacity(), capacity);
    }

    #[test]
    fn exhausted_frame_identity_rejects_before_installing_ownership() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let mut frames = FrameStore::new(1, 1);
        frames.next_generation = u64::MAX;
        let (rejected, _slots) = frame(&runtime, context.realm);
        let object = rejected.cold.function.object_id();
        assert!(frames.push(rejected).is_err());
        assert!(frames.frames.is_empty());
        assert_eq!(frames.frames.capacity(), 0);
        assert!(runtime.0.state.borrow().heap.object(object).is_err());
    }
    fn entry(runtime: &Runtime, realm: ContextId) -> FrameEntry {
        let (mut frame, _slots) = frame(runtime, realm);
        FrameEntry {
            initialize_bindings: false,
            property_generation: frame.property_generation,
            iterator_generation: frame.iterator_generation,
            caller_realm: frame.caller_realm,
            active_frame: frame.active_frame,
            executable: frame.executable.take(),
            cold: frame.cold,
            storage: FrameStorage {
                original_arguments: Vec::new(),
                parameters: Vec::new(),
                locals: Vec::new(),
                operands: Vec::new(),
            },
        }
    }

    #[test]
    fn rejected_child_push_preserves_parent_window_and_releases_child_owners() {
        use crate::engine::vm::{
            driver::push_frame,
            execution::{ExecutionLimits, RunningExecution},
        };
        for exhausted_identity in [false, true] {
            let runtime = Runtime::new();
            let context = runtime.new_context();
            let mut execution = RunningExecution::new(
                &runtime,
                ExecutionLimits {
                    frames: 1,
                    slots: 16,
                },
            )
            .unwrap();
            let parent = push_frame(&mut execution, entry(&runtime, context.realm)).unwrap();
            let generation = execution.frames.next_generation;
            if exhausted_identity {
                execution.frames.limit = 2;
                execution.frames.next_generation = u64::MAX;
            }
            let mut child = entry(&runtime, context.realm);
            let child_object = child.cold.function.object_id();
            child.storage.original_arguments.push(Value::Int(42));
            child
                .storage
                .parameters
                .push(super::super::bindings::FrameBinding::Direct(Value::Int(42)));
            let error = push_frame(&mut execution, child).unwrap_err();
            assert!(error.to_string().contains(if exhausted_identity {
                "identity exhausted"
            } else {
                "frame limit"
            }));
            assert_eq!(execution.frames.current_id(), Some(parent));
            let frame = execution.frames.current_mut(parent).unwrap();
            assert_eq!(
                execution.slots.binding_counts(&frame.window).unwrap(),
                (0, 0)
            );
            assert!(runtime.0.state.borrow().heap.object(child_object).is_err());
            execution.frames.limit = 2;
            execution.frames.next_generation = generation;
            let replacement = push_frame(&mut execution, entry(&runtime, context.realm)).unwrap();
            let mut frame = execution.frames.pop(replacement).unwrap();
            execution.slots.clear_frame(frame.window.take()).unwrap();
            let parent = execution.frames.current_mut(parent).unwrap();
            assert_eq!(
                execution.slots.binding_counts(&parent.window).unwrap(),
                (0, 0)
            );
        }
    }

    struct DropLog(
        &'static str,
        std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    );
    impl Drop for DropLog {
        fn drop(&mut self) {
            self.1.borrow_mut().push(self.0);
        }
    }
    fn tracked_runtime(
        name: &'static str,
        events: &std::rc::Rc<std::cell::RefCell<Vec<&'static str>>>,
    ) -> Runtime {
        let runtime = Runtime::new();
        let log = DropLog(name, events.clone());
        runtime.set_host_promise_rejection_tracker(move |_| {
            let _ = &log;
        });
        runtime
    }
    #[test]
    fn frame_store_abandon_releases_inner_runtime_owner_first() {
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let mut frames = FrameStore::new(1, 2);
        for name in ["parent", "child"] {
            let runtime = tracked_runtime(name, &events);
            let context = runtime.new_context();
            let (frame, _slots) = frame(&runtime, context.realm);
            frames.push(frame).unwrap();
        }
        assert!(events.borrow().is_empty());
        drop(frames);
        assert_eq!(*events.borrow(), ["child", "parent"]);
    }
    #[test]
    fn execution_abandon_clears_child_slots_before_parent_frame_owner() {
        use crate::engine::vm::{
            driver::push_frame,
            execution::{ExecutionLimits, RunningExecution},
        };
        let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let parent_runtime = tracked_runtime("parent", &events);
        let parent_context = parent_runtime.new_context();
        let mut execution = RunningExecution::new(
            &parent_runtime,
            ExecutionLimits {
                frames: 2,
                slots: 16,
            },
        )
        .unwrap();
        push_frame(&mut execution, entry(&parent_runtime, parent_context.realm)).unwrap();
        {
            let runtime = tracked_runtime("child", &events);
            let context = runtime.new_context();
            let captured_runtime = tracked_runtime("child-slot", &events);
            let capture = captured_runtime.new_object(None).unwrap();
            let mut child = entry(&runtime, context.realm);
            child
                .storage
                .original_arguments
                .push(Value::Object(capture));
            child
                .storage
                .parameters
                .push(super::super::bindings::FrameBinding::Direct(
                    Value::Undefined,
                ));
            push_frame(&mut execution, child).unwrap();
        }
        drop(parent_context);
        drop(parent_runtime);
        assert!(events.borrow().is_empty());
        drop(execution);
        assert_eq!(*events.borrow(), ["child-slot", "child", "parent"]);
    }
}

impl FrameCold {
    pub(super) fn has_pending_query(&self) -> bool {
        self.rare
            .get()
            .is_some_and(|rare| rare.property_wait.is_some())
    }
    fn pending_depth(&self) -> usize {
        self.rare
            .get()
            .and_then(|rare| rare.property_wait.as_ref())
            .map_or(0, |wait| wait.continuation_depth())
    }
    pub(super) fn ordinary_return(&self) -> Option<ReturnTarget> {
        let target = self.return_to?;
        if target.tail
            || target.operation.is_some()
            || !matches!(target.owner, ReturnOwner::Frame(_))
        {
            return None;
        }
        if self.rare.get().is_some_and(|rare| {
            rare.constructor_return.is_some()
                || rare.property_wait.is_some()
                || rare.iterator_wait.is_some()
                || rare.conversion.is_some()
                || !rare.regions.is_empty()
                || rare.resume_throw.is_some()
        }) {
            return None;
        }
        Some(target)
    }
}
impl std::ops::Deref for FrameCold {
    type Target = FrameRare;
    fn deref(&self) -> &FrameRare {
        self.rare.get_or_init(Default::default)
    }
}
impl std::ops::DerefMut for FrameCold {
    fn deref_mut(&mut self) -> &mut FrameRare {
        if self.rare.get().is_none() {
            self.rare.set(Default::default()).ok();
        }
        self.rare.get_mut().unwrap()
    }
}
