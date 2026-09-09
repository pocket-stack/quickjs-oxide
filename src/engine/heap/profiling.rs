//! Owner-level storage accounting and safe arena allocation observation.
use super::*;
use crate::engine::api::profiling::{AllocationTrace, MemoryCategory};
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};

// Deliberately exposes a slice, not Vec mutators: every capacity-changing
// operation goes through push and every backing release goes through Drop.
pub(super) struct ArenaStorage {
    values: Vec<ArenaSlot>,
    trace: Option<AllocationTrace>,
}

impl ArenaStorage {
    pub(super) const fn new() -> Self {
        Self {
            values: Vec::new(),
            trace: None,
        }
    }

    pub(super) fn capacity(&self) -> usize {
        self.values.capacity()
    }

    pub(super) fn push(&mut self, value: ArenaSlot) {
        let previous = self.values.capacity();
        self.values.push(value);
        if let Some(trace) = &self.trace {
            trace.record(
                previous * size_of::<ArenaSlot>(),
                self.capacity() * size_of::<ArenaSlot>(),
            );
        }
    }
}

impl Deref for ArenaStorage {
    type Target = [ArenaSlot];
    fn deref(&self) -> &Self::Target {
        &self.values
    }
}

impl DerefMut for ArenaStorage {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.values
    }
}

impl<'a> IntoIterator for &'a ArenaStorage {
    type Item = &'a ArenaSlot;
    type IntoIter = std::slice::Iter<'a, ArenaSlot>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter()
    }
}

impl<'a> IntoIterator for &'a mut ArenaStorage {
    type Item = &'a mut ArenaSlot;
    type IntoIter = std::slice::IterMut<'a, ArenaSlot>;
    fn into_iter(self) -> Self::IntoIter {
        self.values.iter_mut()
    }
}

impl Drop for ArenaStorage {
    fn drop(&mut self) {
        if let Some(trace) = &self.trace {
            let previous = self.capacity() * size_of::<ArenaSlot>();
            // Record F after the backing allocation and all its payloads drop.
            drop(std::mem::take(&mut self.values));
            trace.record(previous, 0);
            trace.finish();
        }
    }
}

impl Heap {
    pub(crate) fn with_allocation_trace(trace: Option<AllocationTrace>) -> Self {
        let mut heap = Self::new();
        heap.slots.trace = trace;
        heap
    }

    pub(crate) fn memory_categories(&self) -> Vec<MemoryCategory> {
        let counts = self.counts();
        let mut result = vec![
            logical("objects", counts.object_nodes),
            logical("shapes", counts.shape_nodes),
            logical("var_refs", counts.var_ref_nodes),
            logical("contexts", counts.context_nodes),
            logical("functions", counts.function_bytecode_nodes),
            storage(
                "arena_slots",
                self.slots.len(),
                self.slots.capacity(),
                size_of::<ArenaSlot>(),
            ),
            storage(
                "arena_free_indices",
                self.free.len(),
                self.free.capacity(),
                size_of::<u32>(),
            ),
            storage(
                "arena_zero_queue",
                self.zero_queue.len(),
                self.zero_queue.capacity(),
                size_of::<RawId>(),
            ),
        ];
        let mut properties = storage("property_slots", 0, 0, 1);
        let mut arrays = logical("arrays", 0);
        let mut elements = storage("dense_array_elements", 0, 0, 1);
        let mut buffers = storage("array_buffer_bytes", 0, 0, 1);
        let mut shared_buffers = logical("shared_array_buffer_wrappers", 0);
        shared_buffers.basis = "wrapper-count-only; shared backing bytes unavailable";
        let mut code = storage("bytecode_instructions", 0, 0, 1);
        code.basis = "deduplicated-Rc-slice-inline-bytes; excludes Rc headers and nested operands";
        let mut seen_code = HashSet::new();
        for slot in &self.slots {
            let node = match &slot.state {
                SlotState::Live(node)
                | SlotState::ZeroQueued(node)
                | SlotState::Finalizing(node) => node,
                _ => continue,
            };
            match &node.data {
                NodeData::Object(object) => {
                    add_storage(
                        &mut properties,
                        object.slots.len(),
                        object.slots.capacity(),
                        size_of::<PropertySlot>(),
                    );
                    match &object.payload {
                        ObjectPayload::Array { dense } => {
                            *arrays.count.as_mut().unwrap() += 1;
                            if let Some(dense) = dense {
                                add_storage(
                                    &mut elements,
                                    dense.len(),
                                    dense.capacity(),
                                    size_of::<RawValue>(),
                                );
                            }
                        }
                        ObjectPayload::ArrayBuffer(data) => {
                            add_storage(&mut buffers, data.bytes.len(), data.bytes.capacity(), 1);
                        }
                        ObjectPayload::SharedArrayBuffer(_) => {
                            *shared_buffers.count.as_mut().unwrap() += 1;
                        }
                        _ => {}
                    }
                }
                NodeData::FunctionBytecode(data) => {
                    // Pointer identity is local to this read-only snapshot and
                    // is never serialized. Shared slices are counted once.
                    if seen_code.insert(Rc::as_ptr(&data.code)) {
                        add_storage(
                            &mut code,
                            data.code.len(),
                            data.code.len(),
                            size_of::<Instruction>(),
                        );
                    }
                }
                _ => {}
            }
        }
        result.extend([properties, arrays, elements, buffers, shared_buffers, code]);
        result
    }
}

fn logical(name: &'static str, count: usize) -> MemoryCategory {
    MemoryCategory {
        name,
        count: Some(count),
        used_bytes: None,
        capacity_bytes: None,
        basis: "logical-node-count; not total bytes",
    }
}

fn storage(name: &'static str, len: usize, capacity: usize, size: usize) -> MemoryCategory {
    MemoryCategory {
        name,
        count: Some(len),
        used_bytes: Some(len * size),
        capacity_bytes: Some(capacity * size),
        basis: "owned-inline-storage; excludes nested allocations and allocator overhead",
    }
}

fn add_storage(category: &mut MemoryCategory, len: usize, capacity: usize, size: usize) {
    *category.count.as_mut().unwrap() += len;
    *category.used_bytes.as_mut().unwrap() += len * size;
    *category.capacity_bytes.as_mut().unwrap() += capacity * size;
}
