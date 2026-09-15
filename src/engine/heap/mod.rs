//! Runtime-owned garbage-collected heap primitives.
//!
//! QuickJS combines explicit reference counts with a trial-deletion cycle
//! collector.  This module keeps the same ownership model while replacing raw
//! pointers with typed generational handles:
//!
//! - every heap edge is a raw, runtime-internal owned handle;
//! - publishing any node retains all of its outgoing heap edges;
//! - zero-reference destruction is driven by an iterative queue;
//! - cycle collection computes external references as
//!   `strong_count - internal_incoming_count`, marks their closure, and then
//!   actively dismantles unreachable object, function-bytecode, and context
//!   anchors;
//! - shapes and variable-reference cells participate in the graph, but
//!   cascade from active anchor destruction rather than acting as cycle
//!   anchors.
//!
//! Atom ownership remains at the runtime boundary.  A shape is expected to
//! arrive with one atom reference for every entry.  Finalization returns those
//! atoms in [`HeapCleanup::atoms`] so the caller can release them without
//! making this low-level arena depend on `AtomTable` mutability or callbacks.

use crate::engine::code::function::metadata::{
    ClassInitializerKind, ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName,
    ConstructorKind, EvalBindingSource, EvalEnvironment, EvalKind, EvalScopeKind,
    EvalVariableEnvironment, FunctionKind, FunctionMetadata, ParameterEnvironmentLayout,
    VariableDefinition,
};
#[cfg(test)]
use crate::engine::code::function::metadata::{EvalBinding, EvalScope, ParameterArgumentCell};

mod buffers;
mod gc;
#[cfg(test)]
use gc::object_atoms;
pub(crate) use gc::{FinalizationJobSink, PreparedFinalizationJob};
pub use gc::{GcStats, HeapCleanup, WeakSymbolGcEvent};
use gc::{
    async_generator_request_edges, context_edges, function_bytecode_edges,
    generator_activation_atoms, generator_activation_edges, object_edges, object_layout_edges,
    promise_reaction_edges, property_slot_atoms, property_slot_edges, raw_module_record_atoms,
    raw_module_record_edges, raw_value_atom, raw_value_edges, raw_value_matches_weak_key,
    shape_edges, var_ref_edges,
};
mod collections;
use crate::engine::code::bytecode_validation;
use bytecode_validation::{
    EvalEnvironmentPhaseContext, parameter_initializer_visible_locals,
    validate_class_initializer_bytecode_layout, validate_derived_constructor_bytecode_layout,
    validate_eval_environment_phase_layout, validate_parameter_bytecode_layout,
    validate_parameter_initializer_scope_layout, validate_pattern_parameter_bytecode_layout,
};
pub(crate) use collections::CollectionIteratorCurrentIndices;
pub use collections::{MapRecord, WeakCollectionKey, WeakCollectionRecords};
mod private_validation;
use crate::engine::api::error::NativeErrorKind;
use crate::engine::atom::Atom;
use crate::engine::builtins::native;
use native::{
    ArrayBufferNativeKind, ArrayIteratorKind, DataViewNativeKind, DynamicFunctionKind,
    GeneratorResumeKind, MapIteratorKind, NativeFunctionData, NativeFunctionId, PrimitiveKind,
    PromiseNativeKind, PromiseResolvingKind, RegExpNativeKind, SetIteratorKind,
    SharedArrayBufferNativeKind, TypedArrayElementKind, TypedArrayNativeKind,
};
use native::{DynamicImportHandlerKind, ModuleEvaluationKind};
use private_validation::validate_published_private_elements;
use std::cell::Cell;

use crate::engine::code::bytecode::{Instruction, MAX_LOCAL_SLOTS, PrivateNameSource};
use crate::engine::code::debug::Pc2LineTable;
use crate::engine::code::module::{
    ModuleImport, ModuleImportCollision, ModuleImportName, ModuleLinkInitializer, ModuleRequest,
    ModuleRequestIndex, ModuleStarExport,
};
use crate::engine::heap::shared_memory::SharedBufferHandle;
use crate::engine::object::shape::{PropertyFlags, PropertyStorageKind, Shape, ShapeError};
use crate::engine::value::JsString;
use crate::engine::value::bigint::JsBigInt;
use crate::regexp::CompiledRegExp;

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};
use std::error::Error;
use std::fmt;
use std::hash::Hash;
use std::rc::Rc;

/// Allocation-complete plan for shortening one fast Array prefix.
///
/// Preparing the plan does not mutate the Array. Once prepared, committing it
/// moves the removed values into already-reserved storage before detaching
/// their heap and atom ownership, so the representation change itself cannot
/// fail because of a container allocation.
pub(crate) struct PreparedArrayDenseTruncation {
    object: ObjectId,
    original_len: usize,
    new_len: usize,
    removed_atom_count: usize,
    removed: Vec<RawValue>,
    cleanup: HeapCleanup,
}

/// Current arena population, split by lifecycle state and node kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct HeapCounts {
    pub object_nodes: usize,
    pub shape_nodes: usize,
    pub var_ref_nodes: usize,
    pub context_nodes: usize,
    pub function_bytecode_nodes: usize,
    pub initializing: usize,
    pub live: usize,
    pub zero_queued: usize,
    pub finalizing: usize,
    pub zombies: usize,
    pub vacant: usize,
    pub retired: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum RawId {
    Object(ObjectId),
    Shape(ShapeId),
    VarRef(VarRefId),
    Context(ContextId),
    FunctionBytecode(FunctionBytecodeId),
}

impl RawId {
    const fn kind(self) -> HeapNodeKind {
        match self {
            Self::Object(_) => HeapNodeKind::Object,
            Self::Shape(_) => HeapNodeKind::Shape,
            Self::VarRef(_) => HeapNodeKind::VarRef,
            Self::Context(_) => HeapNodeKind::Context,
            Self::FunctionBytecode(_) => HeapNodeKind::FunctionBytecode,
        }
    }

    const fn index(self) -> u32 {
        match self {
            Self::Object(id) => id.index,
            Self::Shape(id) => id.index,
            Self::VarRef(id) => id.index,
            Self::Context(id) => id.index,
            Self::FunctionBytecode(id) => id.index,
        }
    }

    const fn generation(self) -> u32 {
        match self {
            Self::Object(id) => id.generation,
            Self::Shape(id) => id.generation,
            Self::VarRef(id) => id.generation,
            Self::Context(id) => id.generation,
            Self::FunctionBytecode(id) => id.generation,
        }
    }
}

// Context nodes intentionally stay inline in the generational arena: boxing
// only this variant would add a second allocator/failure boundary to realm
// publication and collection without shrinking any live Context graph.
#[allow(clippy::large_enum_variant)]
enum NodeData {
    Object(ObjectData),
    Shape(Shape),
    VarRef(VarRefData),
    Context(ContextData),
    FunctionBytecode(FunctionBytecodeData),
}

impl NodeData {
    const fn kind(&self) -> HeapNodeKind {
        match self {
            Self::Object(_) => HeapNodeKind::Object,
            Self::Shape(_) => HeapNodeKind::Shape,
            Self::VarRef(_) => HeapNodeKind::VarRef,
            Self::Context(_) => HeapNodeKind::Context,
            Self::FunctionBytecode(_) => HeapNodeKind::FunctionBytecode,
        }
    }

    fn edges(&self) -> Vec<RawId> {
        match self {
            Self::Object(object) => object_edges(object),
            Self::Shape(shape) => shape_edges(shape),
            Self::VarRef(var_ref) => var_ref_edges(var_ref),
            Self::Context(context) => context_edges(context),
            Self::FunctionBytecode(bytecode) => function_bytecode_edges(bytecode),
        }
    }
}

struct Node {
    strong: u32,
    data: NodeData,
}

enum SlotState {
    Initializing { kind: HeapNodeKind, strong: u32 },
    Live(Node),
    ZeroQueued(Node),
    Finalizing(Node),
    Zombie { kind: HeapNodeKind, strong: u32 },
    Vacant,
    Retired,
}

impl SlotState {
    const fn kind(&self) -> Option<HeapNodeKind> {
        match self {
            Self::Initializing { kind, .. } | Self::Zombie { kind, .. } => Some(*kind),
            Self::Live(node) | Self::ZeroQueued(node) | Self::Finalizing(node) => {
                Some(node.data.kind())
            }
            Self::Vacant | Self::Retired => None,
        }
    }

    const fn strong(&self) -> Option<u32> {
        match self {
            Self::Initializing { strong, .. } | Self::Zombie { strong, .. } => Some(*strong),
            Self::Live(node) | Self::ZeroQueued(node) | Self::Finalizing(node) => Some(node.strong),
            Self::Vacant | Self::Retired => None,
        }
    }
}

struct ArenaSlot {
    generation: u32,
    state: SlotState,
    weak_prev: Option<ObjectId>,
    weak_next: Option<ObjectId>,
}

/// Runtime-local object and shape arena.
///
/// A `Heap` is deliberately not internally synchronized.  The enclosing
/// runtime chooses its single-threaded ownership boundary, as QuickJS does.
pub struct Heap {
    #[cfg(not(feature = "profiling"))]
    slots: Vec<ArenaSlot>,
    #[cfg(feature = "profiling")]
    slots: profiling::ArenaStorage,
    free: Vec<u32>,
    zero_queue: VecDeque<RawId>,
    weak_head: Option<ObjectId>,
    weak_tail: Option<ObjectId>,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

/// Return the occurrence-count difference `left - right` while retaining
/// `left`'s deterministic order. All allocation happens before a caller may
/// publish a record mutation.
fn multiset_difference<T>(
    left: &[T],
    right: &[T],
    operation: &'static str,
) -> Result<Vec<T>, HeapError>
where
    T: Copy + Eq + Hash,
{
    let mut remaining = HashMap::<T, usize>::new();
    remaining
        .try_reserve(right.len())
        .map_err(|_| HeapError::Allocation { operation })?;
    for &item in right {
        let count = remaining.entry(item).or_default();
        *count = count
            .checked_add(1)
            .ok_or(HeapError::Overflow { operation })?;
    }

    let mut difference = Vec::new();
    difference
        .try_reserve(left.len())
        .map_err(|_| HeapError::Allocation { operation })?;
    for &item in left {
        match remaining.get_mut(&item) {
            Some(count) if *count != 0 => *count -= 1,
            Some(_) | None => difference.push(item),
        }
    }
    Ok(difference)
}
const fn is_map_storable_value(value: &RawValue) -> bool {
    !matches!(
        value,
        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
    )
}

fn object_data_is_callable(object: &ObjectData) -> bool {
    matches!(
        &object.payload,
        ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::Proxy(ProxyData {
                is_callable: true,
                ..
            })
    )
}
const fn slot_matches_storage(slot: &PropertySlot, storage: PropertyStorageKind) -> bool {
    matches!(
        (slot, storage),
        (PropertySlot::Data(_), PropertyStorageKind::Data)
            | (PropertySlot::VarRef(_), PropertyStorageKind::Data)
            | (PropertySlot::AutoInit(_), PropertyStorageKind::Data)
            | (PropertySlot::Accessor { .. }, PropertyStorageKind::Accessor)
    )
}

fn increment_kind_count(counts: &mut HeapCounts, kind: HeapNodeKind) {
    match kind {
        HeapNodeKind::Object => counts.object_nodes = counts.object_nodes.saturating_add(1),
        HeapNodeKind::Shape => counts.shape_nodes = counts.shape_nodes.saturating_add(1),
        HeapNodeKind::VarRef => counts.var_ref_nodes = counts.var_ref_nodes.saturating_add(1),
        HeapNodeKind::Context => counts.context_nodes = counts.context_nodes.saturating_add(1),
        HeapNodeKind::FunctionBytecode => {
            counts.function_bytecode_nodes = counts.function_bytecode_nodes.saturating_add(1);
        }
    }
}

#[cfg(test)]
mod tests;

pub mod runtime;
pub mod shared_memory;

pub(crate) mod roots;

pub(crate) mod runtime_gc;

pub(crate) mod ownership;

mod deferred;

#[cfg(test)]
mod release_cleanup_tests;

mod identity;
pub use identity::*;

mod module_records;
pub(crate) use module_records::*;

mod object_records;
pub use object_records::*;

mod realm_records;
pub use realm_records::*;

mod code_records;
pub use code_records::*;

mod binding_records;
pub use binding_records::*;

mod iteration_records;
pub use iteration_records::*;

mod suspension_records;
pub use suspension_records::*;

mod iterator_records;
pub use iterator_records::*;

mod promise_records;
pub use promise_records::*;

mod buffer_records;
pub use buffer_records::*;

mod arena;

mod allocation;

#[cfg(feature = "profiling")]
pub(crate) mod profiling;

mod realm_storage;

mod object_storage;

mod binding_storage;

mod module_storage;

mod promise_storage;

mod iterator_storage;

mod suspension_storage;
