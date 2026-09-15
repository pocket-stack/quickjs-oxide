//! Producer-local temporary buffer diagnostics. Counts are cumulative; Value
//! copies, rooted promotions and raw edges are separate, never additive totals.
use super::current;
use crate::engine::{heap::RawValue, value::Value};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CallBufferCost {
    pub capacity_growths: u64,
    pub capacity_growth_bytes: u64,
    pub allocated_capacity_bytes: u64,
    pub buffers_observed: u64,
    pub shared_storage_clones: u64,
    pub observed_capacity_bytes: u64,
    pub slots_initialized: u64,
    pub values_copied: u64,
    pub heap_root_copies: u64,
    pub primitive_rc_copies: u64,
    pub immediate_copies: u64,
    pub values_moved: u64,
    pub raw_values_copied: u64,
    pub raw_heap_edges_copied: u64,
    pub raw_primitive_rc_copies: u64,
}

impl CallBufferCost {
    pub fn fields(&self) -> [(&'static str, u64); 15] {
        [
            ("capacity_growths", self.capacity_growths),
            ("capacity_growth_bytes", self.capacity_growth_bytes),
            ("allocated_capacity_bytes", self.allocated_capacity_bytes),
            ("buffers_observed", self.buffers_observed),
            ("shared_storage_clones", self.shared_storage_clones),
            ("observed_capacity_bytes", self.observed_capacity_bytes),
            ("slots_initialized", self.slots_initialized),
            ("values_copied", self.values_copied),
            ("heap_root_copies", self.heap_root_copies),
            ("primitive_rc_copies", self.primitive_rc_copies),
            ("immediate_copies", self.immediate_copies),
            ("values_moved", self.values_moved),
            ("raw_values_copied", self.raw_values_copied),
            ("raw_heap_edges_copied", self.raw_heap_edges_copied),
            ("raw_primitive_rc_copies", self.raw_primitive_rc_copies),
        ]
    }
}

/// Call only at an observed successful reserve/new-allocation site. `after`
/// is Rust Vec capacity, not allocator usable size or physical realloc bytes.
/// Named `*_box`/`*_rc` producers use 0 -> 1 only after successful fixed-size
/// allocation; element_size is payload size, excluding allocator/Rc headers.
pub(crate) fn record_call_buffer_capacity(
    name: &'static str,
    before: usize,
    after: usize,
    element_size: usize,
) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    if after > before {
        cost.capacity_growths = cost.capacity_growths.saturating_add(1);
        cost.capacity_growth_bytes = cost
            .capacity_growth_bytes
            .saturating_add(((after - before) * element_size) as u64);
        cost.allocated_capacity_bytes = cost
            .allocated_capacity_bytes
            .saturating_add((after * element_size) as u64);
    }
}

/// Observation only: use when an incoming/collected buffer may reuse backing
/// storage. This deliberately does not increment allocation/growth counters.
pub(crate) fn record_call_buffer_observed(
    name: &'static str,
    capacity: usize,
    element_size: usize,
) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    cost.buffers_observed = cost.buffers_observed.saturating_add(1);
    cost.observed_capacity_bytes = cost
        .observed_capacity_bytes
        .saturating_add((capacity * element_size) as u64);
}

/// An immutable Rc storage owner was cloned; payload and allocation are unchanged.
pub(crate) fn record_call_buffer_share(name: &'static str, length: usize, element_size: usize) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    cost.shared_storage_clones = cost.shared_storage_clones.saturating_add(1);
    cost.buffers_observed = cost.buffers_observed.saturating_add(1);
    cost.observed_capacity_bytes = cost
        .observed_capacity_bytes
        .saturating_add((length * element_size) as u64);
}

pub(crate) fn record_call_buffer_initialized(name: &'static str, count: usize) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    cost.slots_initialized = cost.slots_initialized.saturating_add(count as u64);
}

pub(crate) fn record_call_buffer_copies(name: &'static str, values: &[Value]) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    cost.values_copied = cost.values_copied.saturating_add(values.len() as u64);
    cost.slots_initialized = cost.slots_initialized.saturating_add(values.len() as u64);
    for value in values {
        let counter = match value {
            Value::Object(_) | Value::Symbol(_) => &mut cost.heap_root_copies,
            Value::String(_) => &mut cost.primitive_rc_copies,
            Value::BigInt(value) if value.as_i64().is_none() => &mut cost.primitive_rc_copies,
            _ => &mut cost.immediate_copies,
        };
        *counter = counter.saturating_add(1);
    }
}

pub(crate) fn record_call_buffer_moves(name: &'static str, count: usize) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    cost.values_moved = cost.values_moved.saturating_add(count as u64);
    cost.slots_initialized = cost.slots_initialized.saturating_add(count as u64);
}

pub(crate) fn record_call_raw_buffer_copies(name: &'static str, values: &[RawValue]) {
    let Some(collector) = current() else {
        return;
    };
    let mut costs = collector.borrow_mut();
    let cost = costs.call_buffers.entry(name).or_default();
    cost.raw_values_copied = cost.raw_values_copied.saturating_add(values.len() as u64);
    for value in values {
        match value {
            RawValue::Object(_) | RawValue::Symbol(_) | RawValue::Private(_) => {
                cost.raw_heap_edges_copied = cost.raw_heap_edges_copied.saturating_add(1);
            }
            RawValue::String(_) => {
                cost.raw_primitive_rc_copies = cost.raw_primitive_rc_copies.saturating_add(1);
            }
            RawValue::BigInt(value) if value.as_i64().is_none() => {
                cost.raw_primitive_rc_copies = cost.raw_primitive_rc_copies.saturating_add(1);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::api::profiling::CostProfile;
    use crate::engine::api::{Runtime, Value};

    #[test]
    fn bound_and_apply_producers_separate_edges_roots_and_moves() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(context.eval("function f(x,y){return x.value+y;}var marker={value:4};var bound=f.bind(null,marker);bound(3)+f.apply(null,{length:2,0:marker,1:5})+f(...[marker,6])").unwrap(), Value::Int(26));
        let costs = profile.snapshot();
        let raw = &costs.call_buffers["bound.raw_snapshot"];
        assert!(raw.buffers_observed > 0);
        assert_eq!(raw.shared_storage_clones, raw.buffers_observed);
        assert_eq!(raw.capacity_growths, 0);
        assert_eq!(raw.raw_values_copied, 0);
        assert_eq!(raw.heap_root_copies, 0);
        assert!(costs.call_buffers["bound.rooted_snapshot"].heap_root_copies > 0);
        assert!(costs.call_buffers["bound.merge"].heap_root_copies > 0);
        assert_eq!(costs.call_buffers["apply.indexed"].values_moved, 2);
        assert!(costs.call_buffers["arguments.fast_rooted"].buffers_observed > 0);
        assert!(costs.call_buffers["invoke.argv_carrier"].buffers_observed > 0);
    }

    #[test]
    fn generator_diagnostics_measure_encode_and_decode_without_retaining_owners() {
        let runtime = Runtime::new();
        let weak = std::rc::Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let profile = CostProfile::start();
        assert_eq!(context.eval("function* g(){try{yield {value:7};}finally{}}var i=g();var n=i.next().value.value;i.next();i=null;n").unwrap(), Value::Int(7));
        let costs = profile.snapshot();
        for name in ["freeze.encode", "thaw.decode"] {
            let phase = &costs.vm_phases[name];
            assert!(phase.cost.attempts > 0);
            assert_eq!(phase.cost.attempts as usize, phase.samples_ns.len());
            assert_eq!(phase.omitted_samples, 0);
            assert!(
                phase
                    .samples_ns
                    .iter()
                    .all(|[inclusive, exclusive]| inclusive >= exclusive)
            );
        }
        drop((context, runtime));
        assert!(weak.upgrade().is_none());
        drop((profile, costs));
    }
}
