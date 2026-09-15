//! Scoped thread-local diagnostics, independent of Runtime ownership.
//!
//! Only the innermost live scope receives events. Guards hold no Runtime or JS
//! values, and dropping a guard (including during unwinding) unregisters it.
//! Timing is inclusive wall time, including nested compilation/host callbacks;
//! phase totals must not be added to obtain exclusive compile time.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
mod buffers;
pub use buffers::CallBufferCost;
pub(crate) use buffers::{
    record_call_buffer_capacity, record_call_buffer_copies, record_call_buffer_initialized,
    record_call_buffer_moves, record_call_buffer_observed, record_call_buffer_share,
    record_call_raw_buffer_copies,
};
mod phases;
pub(crate) use phases::{CompilePhase, PhaseTimer};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PhaseCost {
    /// Entries, including operations which return an error or unwind.
    pub attempts: u64,
    pub inclusive_ns: u128,
    /// Time excluding measured child phases in the same collector.
    /// Uninstrumented work and nested collectors remain charged to this phase.
    pub exclusive_ns: u128,
    /// Boundary snapshots, not allocations or continuous memory sampling.
    pub storage_samples: u64,
    /// Largest partial owned-Vec capacity snapshot in this phase. Excludes
    /// referenced payloads and temporary worklists; not a compiler peak total.
    pub maximum_observed_ir_capacity_bytes: u64,
}

/// Attempt durations, including errors and unwinding. Only the first 4096
/// samples per phase are retained; omitted samples are counted explicitly.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct VmPhaseCost {
    pub cost: PhaseCost,
    /// [inclusive, exclusive] wall-clock nanoseconds, clamped to u64.
    pub samples_ns: Vec<[u64; 2]>,
    pub omitted_samples: u64,
}

/// Partial owned-core costs. Capacity peaks are per store, not process totals.
/// Moves count logical owning transfers, not machine instructions or bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct OwnedStorageCost {
    pub slot_capacity_growths: u64,
    pub maximum_slot_capacity: usize,
    pub frame_capacity_growths: u64,
    pub maximum_frame_capacity: usize,
    pub frames_pushed: u64,
    pub maximum_frame_depth: usize,
    /// Logical frame-slot initialization, including reused inactive slots.
    pub slots_initialized: u64,
    /// Physical writes of None when the initialized backing high-water grows.
    pub physical_none_initializations: u64,
    pub maximum_initialized_slots: usize,
    pub maximum_reserved_slots: usize,
    pub maximum_live_slots: usize,
    pub slot_moves: u64,
    pub slot_clears: u64,
    pub value_copies: u64,
    /// Object/Symbol retains at the slot copy boundary; excludes primitive Rc.
    pub copied_heap_roots: u64,
    pub hot_value_releases: u64,
    /// Object/Symbol releases proven safe by the narrow slot transaction.
    pub hot_heap_root_releases: u64,
}

/// Successful bytecode preparation and owned frame storage only. Capacities
/// are cumulative observations, not live peaks or allocator usable bytes.
/// Argument buffer observations do not count intermediate bound/apply scratch
/// allocations. Root copies cover parameter and preparation callee clones;
/// they are not total Runtime retain/release activity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CallPreparationCost {
    pub frames_prepared: u64,
    pub parameter_buffer_allocations: u64,
    pub parameter_capacity_bytes: u64,
    pub local_buffer_allocations: u64,
    pub local_capacity_bytes: u64,
    pub parameter_slots_initialized: u64,
    pub local_slots_initialized: u64,
    pub parameter_value_copies: u64,
    pub parameter_heap_root_copies: u64,
    pub callee_heap_root_copies: u64,
    pub owned_frame_allocations: u64,
    pub owned_frame_bytes: u64,
    pub owned_captured_reuse_allocations: u64,
    pub owned_captured_reuse_capacity_bytes: u64,
    pub owned_argument_buffers_observed: u64,
    pub owned_argument_capacity_bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CostSnapshot {
    pub parse: PhaseCost,
    pub resolution: PhaseCost,
    pub lowering: PhaseCost,
    pub blocks: PhaseCost,
    pub fusion: PhaseCost,
    pub relocation: PhaseCost,
    pub verify: PhaseCost,
    pub publish: PhaseCost,
    /// Successfully lowered function drafts, including nested functions.
    /// This is not a count of published functions or a unique-code inventory.
    pub lowered_functions: u64,
    pub code_instructions: u64,
    /// Inline typed-instruction storage; excludes boxed operands and metadata.
    pub code_inline_bytes: u64,
    pub maximum_verified_stack: u16,
    /// Optional final-code dumps, in lowering completion order. Disabled by
    /// default; these retain text only, not Runtime roots or IR storage.
    pub code_disassembly: Option<Vec<String>>,
    /// Actual entries to the previous interpreter's instruction dispatch.
    pub legacy_dispatches: u64,
    pub legacy_pc_publications: u64,
    /// Operand depth at dispatch boundaries, excluding locals/arguments.
    pub legacy_max_operand_depth: usize,
    /// Successfully completed instructions in the owned ordinary-call core.
    pub owned_instructions: u64,
    /// Untouched instructions handed to the temporary previous-VM bridge.
    pub owned_bridge_exits: u64,
    /// Selected calls still executed through the transitional synchronous
    /// runtime boundary. Includes callback-free callees; not all nested calls.
    pub owned_sync_call_bridges: u64,
    pub owned_max_operand_depth: usize,
    /// Rust inline layouts as [size bytes, alignment bytes]; excludes owned
    /// allocations and is not a measurement of dynamic payload-copy traffic.
    pub owned_execution_layouts: std::collections::BTreeMap<&'static str, [usize; 2]>,
    /// Observed owned scheduler events. Missing entries are unobserved, not
    /// measurements of legacy behavior; these are not machine copy counts.
    pub owned_execution_events: std::collections::BTreeMap<&'static str, u64>,
    pub owned_storage: OwnedStorageCost,
    pub call_preparation: CallPreparationCost,
    pub call_buffers: std::collections::BTreeMap<&'static str, CallBufferCost>,
    pub vm_phases: std::collections::BTreeMap<&'static str, VmPhaseCost>,
}

type Collector = RefCell<CostSnapshot>;
thread_local! {
    static SCOPES: RefCell<Vec<Weak<Collector>>> = const { RefCell::new(Vec::new()) };
}

/// A diagnostic collection interval on the calling thread. Not Send or Sync.
/// Production builds without the `profiling` feature contain no event hooks.
pub struct CostProfile {
    collector: Rc<Collector>,
}

impl CostProfile {
    #[must_use]
    pub fn start() -> Self {
        let collector = Rc::new(RefCell::new(CostSnapshot::default()));
        SCOPES.with(|scopes| scopes.borrow_mut().push(Rc::downgrade(&collector)));
        Self { collector }
    }

    /// Capture disassembly for subsequent successfully lowered drafts.
    /// Existing captures are preserved. Intended for diagnostics, not timing.
    pub fn capture_disassembly(&self) {
        self.collector
            .borrow_mut()
            .code_disassembly
            .get_or_insert_with(Vec::new);
    }

    #[must_use]
    pub fn snapshot(&self) -> CostSnapshot {
        self.collector.borrow().clone()
    }
}

impl Drop for CostProfile {
    fn drop(&mut self) {
        SCOPES.with(|scopes| {
            scopes
                .borrow_mut()
                .retain(|scope| !scope.ptr_eq(&Rc::downgrade(&self.collector)));
        });
    }
}

fn current() -> Option<Rc<Collector>> {
    SCOPES.with(|scopes| scopes.borrow().last().and_then(Weak::upgrade))
}

pub(crate) fn record_call_preparation(
    parameter_slots: usize,
    parameter_bytes: usize,
    local_slots: usize,
    local_bytes: usize,
    argument_copies: usize,
    argument_root_copies: usize,
    callee_root_copies: usize,
) {
    if let Some(collector) = current() {
        let mut snapshot = collector.borrow_mut();
        let cost = &mut snapshot.call_preparation;
        cost.frames_prepared = cost.frames_prepared.saturating_add(1);
        cost.parameter_buffer_allocations = cost
            .parameter_buffer_allocations
            .saturating_add(u64::from(parameter_bytes != 0));
        cost.parameter_capacity_bytes = cost
            .parameter_capacity_bytes
            .saturating_add(parameter_bytes as u64);
        cost.local_buffer_allocations = cost
            .local_buffer_allocations
            .saturating_add(u64::from(local_bytes != 0));
        cost.local_capacity_bytes = cost.local_capacity_bytes.saturating_add(local_bytes as u64);
        cost.parameter_slots_initialized = cost
            .parameter_slots_initialized
            .saturating_add(parameter_slots as u64);
        cost.local_slots_initialized = cost
            .local_slots_initialized
            .saturating_add(local_slots as u64);
        cost.parameter_value_copies = cost
            .parameter_value_copies
            .saturating_add(argument_copies as u64);
        cost.parameter_heap_root_copies = cost
            .parameter_heap_root_copies
            .saturating_add(argument_root_copies as u64);
        cost.callee_heap_root_copies = cost
            .callee_heap_root_copies
            .saturating_add(callee_root_copies as u64);
    }
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_call_storage(
    frame_bytes: usize,
    reuse_bytes: usize,
    argument_bytes: usize,
) {
    if let Some(collector) = current() {
        let mut snapshot = collector.borrow_mut();
        let cost = &mut snapshot.call_preparation;
        cost.owned_frame_allocations = cost
            .owned_frame_allocations
            .saturating_add(u64::from(frame_bytes != 0));
        cost.owned_frame_bytes = cost.owned_frame_bytes.saturating_add(frame_bytes as u64);
        cost.owned_captured_reuse_allocations = cost
            .owned_captured_reuse_allocations
            .saturating_add(u64::from(reuse_bytes != 0));
        cost.owned_captured_reuse_capacity_bytes = cost
            .owned_captured_reuse_capacity_bytes
            .saturating_add(reuse_bytes as u64);
        cost.owned_argument_buffers_observed = cost
            .owned_argument_buffers_observed
            .saturating_add(u64::from(argument_bytes != 0));
        cost.owned_argument_capacity_bytes = cost
            .owned_argument_capacity_bytes
            .saturating_add(argument_bytes as u64);
    }
}

pub(crate) fn cost_profile_active() -> bool {
    current().is_some()
}

pub(crate) fn record_compiler_storage(phase: CompilePhase, bytes: u64) {
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        let cost = phase.cost_mut(&mut costs);
        cost.storage_samples = cost.storage_samples.saturating_add(1);
        cost.maximum_observed_ir_capacity_bytes =
            cost.maximum_observed_ir_capacity_bytes.max(bytes);
    }
}

pub(crate) fn record_lowered_function(
    code: &[crate::engine::code::bytecode::Instruction],
    max_stack: u16,
) {
    let instructions = code.len();
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        costs.lowered_functions = costs.lowered_functions.saturating_add(1);
        costs.code_instructions = costs.code_instructions.saturating_add(instructions as u64);
        costs.code_inline_bytes = costs.code_inline_bytes.saturating_add(
            instructions.saturating_mul(size_of::<crate::engine::code::bytecode::Instruction>())
                as u64,
        );
        costs.maximum_verified_stack = costs.maximum_verified_stack.max(max_stack);
        if let Some(dumps) = &mut costs.code_disassembly {
            dumps.push(crate::engine::code::instruction::disassemble(code));
        }
    }
}

pub(crate) fn record_legacy_dispatch(operand_depth: usize) {
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        costs.legacy_dispatches = costs.legacy_dispatches.saturating_add(1);
        costs.legacy_max_operand_depth = costs.legacy_max_operand_depth.max(operand_depth);
    }
}

pub(crate) fn record_legacy_pc_publication() {
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        costs.legacy_pc_publications = costs.legacy_pc_publications.saturating_add(1);
    }
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_instruction(operand_depth: usize) {
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        costs.owned_instructions = costs.owned_instructions.saturating_add(1);
        costs.owned_max_operand_depth = costs.owned_max_operand_depth.max(operand_depth);
    }
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_bridge() {
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        costs.owned_bridge_exits = costs.owned_bridge_exits.saturating_add(1);
    }
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_sync_call_bridge() {
    if let Some(collector) = current() {
        let mut costs = collector.borrow_mut();
        costs.owned_sync_call_bridges = costs.owned_sync_call_bridges.saturating_add(1);
    }
}

#[cfg(feature = "stack-vm")]
pub(crate) enum OwnedStorageEvent {
    SlotCapacity { before: usize, after: usize },
    FrameCapacity { before: usize, after: usize },
    FramePush(usize),
    Initialize(usize),
    NoneInitialization { count: usize, high_water: usize },
    Occupancy { reserved: usize, live: usize },
    Move(usize),
    Clear(usize),
    Copy { heap_root: bool },
    HotRelease { heap_root: bool },
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_execution_layout<T>(name: &'static str) {
    if let Some(collector) = current() {
        collector
            .borrow_mut()
            .owned_execution_layouts
            .entry(name)
            .or_insert([size_of::<T>(), align_of::<T>()]);
    }
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_execution_event(name: &'static str) {
    let Some(collector) = current() else {
        return;
    };
    let mut snapshot = collector.borrow_mut();
    let count = snapshot.owned_execution_events.entry(name).or_default();
    *count = count.saturating_add(1);
}

#[cfg(feature = "stack-vm")]
pub(crate) fn record_owned_storage(event: OwnedStorageEvent) {
    let Some(collector) = current() else {
        return;
    };
    let mut snapshot = collector.borrow_mut();
    let cost = &mut snapshot.owned_storage;
    match event {
        OwnedStorageEvent::SlotCapacity { before, after } => {
            cost.slot_capacity_growths = cost
                .slot_capacity_growths
                .saturating_add(u64::from(after > before));
            cost.maximum_slot_capacity = cost.maximum_slot_capacity.max(after);
        }
        OwnedStorageEvent::FrameCapacity { before, after } => {
            cost.frame_capacity_growths = cost
                .frame_capacity_growths
                .saturating_add(u64::from(after > before));
            cost.maximum_frame_capacity = cost.maximum_frame_capacity.max(after);
        }
        OwnedStorageEvent::FramePush(depth) => {
            cost.frames_pushed = cost.frames_pushed.saturating_add(1);
            cost.maximum_frame_depth = cost.maximum_frame_depth.max(depth);
        }
        OwnedStorageEvent::Initialize(count) => {
            cost.slots_initialized = cost.slots_initialized.saturating_add(count as u64)
        }
        OwnedStorageEvent::NoneInitialization { count, high_water } => {
            cost.physical_none_initializations = cost
                .physical_none_initializations
                .saturating_add(count as u64);
            cost.maximum_initialized_slots = cost.maximum_initialized_slots.max(high_water);
        }
        OwnedStorageEvent::Occupancy { reserved, live } => {
            cost.maximum_reserved_slots = cost.maximum_reserved_slots.max(reserved);
            cost.maximum_live_slots = cost.maximum_live_slots.max(live);
        }
        OwnedStorageEvent::Move(count) => {
            cost.slot_moves = cost.slot_moves.saturating_add(count as u64)
        }
        OwnedStorageEvent::Clear(count) => {
            cost.slot_clears = cost.slot_clears.saturating_add(count as u64)
        }
        OwnedStorageEvent::Copy { heap_root } => {
            cost.value_copies = cost.value_copies.saturating_add(1);
            cost.copied_heap_roots = cost.copied_heap_roots.saturating_add(u64::from(heap_root));
        }
        OwnedStorageEvent::HotRelease { heap_root } => {
            cost.hot_value_releases = cost.hot_value_releases.saturating_add(1);
            cost.hot_heap_root_releases = cost
                .hot_heap_root_releases
                .saturating_add(u64::from(heap_root));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CostProfile;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn real_compile_and_execution_are_counted_and_nested_scopes_are_isolated() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let outer = CostProfile::start();
        assert_eq!(
            context.eval("(function(x){return x+1;})(41)").unwrap(),
            Value::Int(42)
        );
        let before = outer.snapshot();
        assert_eq!(before.parse.attempts, 1);
        assert_eq!(before.resolution.attempts, 1);
        assert_eq!(before.lowering.attempts, 1);
        assert_eq!(before.lowered_functions, 2);
        assert_eq!(before.parse.storage_samples, 1);
        assert_eq!(before.resolution.storage_samples, 2);
        assert_eq!(before.lowering.storage_samples, 1);
        assert!(before.parse.maximum_observed_ir_capacity_bytes > 0);
        assert!(before.code_instructions > 0);
        if cfg!(feature = "stack-vm") {
            assert!(before.owned_instructions > 0);
        } else {
            assert!(before.legacy_dispatches > 0);
            assert_eq!(before.owned_instructions, 0);
            assert_eq!(before.owned_bridge_exits, 0);
        }
        assert_eq!(before.legacy_dispatches, before.legacy_pc_publications);
        {
            let inner = CostProfile::start();
            context.eval("1+2").unwrap();
            assert_eq!(inner.snapshot().parse.attempts, 1);
            assert_eq!(outer.snapshot(), before);
        }
        context.eval("3+4").unwrap();
        assert_eq!(outer.snapshot().parse.attempts, 2);
    }

    #[test]
    fn parse_failure_and_unwinding_release_the_collection_scope() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let outer = CostProfile::start();
        assert!(context.eval("let = ;").is_err());
        let failed = outer.snapshot();
        assert_eq!(failed.parse.attempts, 1);
        assert_eq!(failed.resolution.attempts, 0);
        assert_eq!(failed.lowered_functions, 0);
        let unwind = std::panic::catch_unwind(|| {
            let _inner = CostProfile::start();
            panic!("profile unwind probe");
        });
        assert!(unwind.is_err());
        context.eval("42").unwrap();
        assert_eq!(outer.snapshot().parse.attempts, 2);
    }

    #[test]
    fn call_preparation_distinguishes_padding_copies_and_owned_storage() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context.eval("(function(a,b){return a})").unwrap() else {
            panic!("expected function")
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let marker = runtime.new_object(None).unwrap();
        for values in [
            vec![],
            vec![Value::Object(marker.clone())],
            vec![Value::Int(1), Value::Int(2), Value::Int(3)],
        ] {
            let profile = CostProfile::start();
            assert_eq!(
                context.call(&callable, Value::Undefined, &values).unwrap(),
                values.first().cloned().unwrap_or(Value::Undefined)
            );
            let cost = profile.snapshot().call_preparation;
            assert_eq!(cost.frames_prepared, 1);
            assert_eq!(
                cost.parameter_buffer_allocations,
                u64::from(!cfg!(feature = "stack-vm"))
            );
            assert_eq!(cost.parameter_slots_initialized, values.len().max(2) as u64);
            assert_eq!(cost.parameter_value_copies, values.len() as u64);
            assert_eq!(
                cost.parameter_heap_root_copies,
                u64::from(values.len() == 1)
            );
            assert_eq!(cost.callee_heap_root_copies, 1);
            if cfg!(feature = "stack-vm") {
                assert_eq!(cost.owned_frame_allocations, 1);
                assert!(cost.owned_frame_bytes > 0);
                assert_eq!(
                    cost.owned_argument_buffers_observed,
                    u64::from(!values.is_empty())
                );
                assert_eq!(cost.owned_argument_capacity_bytes == 0, values.is_empty());
            } else {
                assert_eq!(cost.owned_frame_allocations, 0);
                assert_eq!(cost.owned_argument_buffers_observed, 0);
            }
        }
    }

    #[test]
    fn throwing_body_still_counts_a_prepared_frame() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(function) = context.eval("(function f(){if(f)throw 42})").unwrap() else {
            panic!("expected function")
        };
        let callable = runtime.as_callable(&function).unwrap().unwrap();
        let profile = CostProfile::start();
        assert!(context.call(&callable, Value::Undefined, &[]).is_err());
        let cost = profile.snapshot().call_preparation;
        assert_eq!(cost.frames_prepared, 1);
        assert_eq!(cost.parameter_buffer_allocations, 0);
        assert_eq!(cost.parameter_value_copies, 0);
        assert_eq!(cost.callee_heap_root_copies, 2);
        assert!(cost.local_slots_initialized > 0);
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
    }
}

#[cfg(test)]
mod disassembly_tests {
    use super::CostProfile;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn optional_disassembly_captures_only_subsequent_final_code() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        context.eval("0").unwrap();
        let before = profile.snapshot();
        assert!(before.code_disassembly.is_none());
        profile.capture_disassembly();
        assert_eq!(
            context.eval("(function(x){return x+1;})(41)").unwrap(),
            Value::Int(42)
        );
        let after = profile.snapshot();
        let dumps = after.code_disassembly.as_ref().unwrap();
        assert_eq!(
            dumps.len() as u64,
            after.lowered_functions - before.lowered_functions
        );
        assert_eq!(
            dumps
                .iter()
                .map(|dump| dump.lines().count() as u64)
                .sum::<u64>(),
            after.code_instructions - before.code_instructions
        );
        let add = dumps
            .iter()
            .flat_map(|dump| dump.lines())
            .find(|line| line.contains(" Add ; "))
            .unwrap();
        assert!(add.contains("may_call_js: true"));
        assert!(add.contains("may_allocate: true"));
        profile.capture_disassembly();
        assert_eq!(profile.snapshot(), after);
    }
}
