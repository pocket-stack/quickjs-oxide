//! Canonical BC5 writer for a validated, heap-independent [`WireGraph`].

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};

use crate::atom::{ATOM_MAX_INT, ATOM_MAX_TABLE_INDEX};

use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::wire::{BcTag, WireError, WireString, WireWriter};
use super::model::{
    ArrayBufferLayoutError, AtomId, GraphError, GraphLimits, GraphResourceKind, NodeId,
    TypedArrayBackingError, TypedArrayKind, TypedArrayLayoutError, WireGraph, WireKey, WireNode,
    WireValue, canonical_bigint_length, numeric_atom_index, semantic_atom_eq, semantic_atom_hash,
    validate_array_buffer_layout, validate_typed_array_write_layout,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct GraphEncodeOptions {
    allow_object_references: bool,
    max_output_bytes: usize,
    limits: GraphLimits,
}

impl GraphEncodeOptions {
    #[must_use]
    pub(in crate::runtime) const fn new(
        allow_object_references: bool,
        max_output_bytes: usize,
        limits: GraphLimits,
    ) -> Self {
        Self {
            allow_object_references,
            max_output_bytes,
            limits,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum GraphEncodeError {
    Graph(GraphError),
    Wire(WireError),
    AtomCountOverflow {
        atom_count: usize,
    },
    IntegerAtomOutOfRange {
        index: u32,
    },
    DuplicatePropertyKey {
        node: NodeId,
    },
    UnplannedAtom {
        atom: AtomId,
    },
    NonCanonicalBigInt,
    InvalidArrayBuffer {
        node: NodeId,
        reason: ArrayBufferLayoutError,
    },
    InvalidTypedArrayBacking {
        node: NodeId,
        reason: TypedArrayBackingError,
    },
    InvalidTypedArray {
        node: NodeId,
        reason: TypedArrayLayoutError,
    },
    CircularReference {
        node: NodeId,
    },
    AllocationFailed,
}

impl fmt::Display for GraphEncodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Graph(error) => fmt::Display::fmt(error, formatter),
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::AtomCountOverflow { atom_count } => {
                write!(
                    formatter,
                    "BC5 atom count {atom_count} does not fit its atom index space"
                )
            }
            Self::IntegerAtomOutOfRange { index } => {
                write!(formatter, "BC5 integer atom index {index} exceeds 31 bits")
            }
            Self::DuplicatePropertyKey { node } => write!(
                formatter,
                "wire graph node {} contains a duplicate semantic property key",
                node.zero_based()
            ),
            Self::UnplannedAtom { atom } => write!(
                formatter,
                "wire graph atom {} was not encountered by the encode plan",
                atom.zero_based()
            ),
            Self::NonCanonicalBigInt => {
                formatter.write_str("wire graph contains a non-canonical BigInt payload")
            }
            Self::InvalidArrayBuffer { node, reason } => write!(
                formatter,
                "wire graph node {} contains an invalid ArrayBuffer layout: {reason}",
                node.zero_based(),
            ),
            Self::InvalidTypedArrayBacking { node, reason } => write!(
                formatter,
                "wire graph node {} contains an invalid TypedArray backing: {reason}",
                node.zero_based(),
            ),
            Self::InvalidTypedArray { node, reason } => write!(
                formatter,
                "wire graph node {} contains an invalid TypedArray layout: {reason}",
                node.zero_based(),
            ),
            Self::CircularReference { node } => write!(
                formatter,
                "wire graph contains a circular reference through node {}",
                node.zero_based()
            ),
            Self::AllocationFailed => formatter.write_str("wire graph writer allocation failed"),
        }
    }
}

impl std::error::Error for GraphEncodeError {}

impl From<GraphError> for GraphEncodeError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl From<WireError> for GraphEncodeError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

enum EncodeValue<'a> {
    Borrowed(&'a WireValue),
    Node(NodeId),
}

enum EncodeTask<'a> {
    Value(EncodeValue<'a>, usize),
    Key(WireKey),
    LeaveNode(NodeId),
}

#[derive(Clone, Copy)]
enum CanonicalKey {
    Index(u32),
    Header(u32),
}

struct EncodePlan<'a> {
    atoms: Vec<&'a WireString>,
    source_atom_keys: HashMap<AtomId, CanonicalKey>,
}

#[derive(Clone, Copy)]
struct SemanticAtomRef<'a>(&'a WireString);

impl PartialEq for SemanticAtomRef<'_> {
    fn eq(&self, other: &Self) -> bool {
        semantic_atom_eq(self.0, other.0)
    }
}

impl Eq for SemanticAtomRef<'_> {}

impl Hash for SemanticAtomRef<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        semantic_atom_hash(self.0, state);
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum SemanticPropertyKey<'a> {
    Index(u32),
    Atom(SemanticAtomRef<'a>),
}

pub(in crate::runtime) fn encode_graph(
    graph: &WireGraph,
    options: GraphEncodeOptions,
) -> Result<Vec<u8>, GraphEncodeError> {
    let plan = build_encode_plan(graph, options)?;
    let atom_count =
        u32::try_from(plan.atoms.len()).map_err(|_| GraphEncodeError::AtomCountOverflow {
            atom_count: plan.atoms.len(),
        })?;
    let atom_space = AtomIndexSpace::new(BinaryObjectMode::Data, atom_count)?;

    let mut writer = WireWriter::new(options.max_output_bytes);
    writer.write_header(atom_count)?;
    for atom in &plan.atoms {
        writer.write_string(atom)?;
    }

    let mut object_indices = HashMap::new();
    let mut active_nodes = HashSet::new();

    let mut next_object_index = 0_u32;
    let mut total_container_entries = 0_usize;
    let mut total_bigint_bytes = 0_usize;
    let mut total_array_buffer_bytes = 0_usize;
    let mut tasks = Vec::new();
    tasks
        .try_reserve(1)
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    tasks.push(EncodeTask::Value(EncodeValue::Borrowed(&graph.root), 0));

    while let Some(task) = tasks.pop() {
        match task {
            EncodeTask::Key(key) => write_key(&mut writer, &plan, atom_space, key)?,
            EncodeTask::LeaveNode(node) => {
                let was_active = active_nodes.remove(&node);
                debug_assert!(was_active);
            }
            EncodeTask::Value(value, parent_depth) => match value {
                EncodeValue::Borrowed(WireValue::Undefined) => {
                    writer.write_tag(BcTag::Undefined)?;
                }
                EncodeValue::Borrowed(WireValue::Null) => writer.write_tag(BcTag::Null)?,
                EncodeValue::Borrowed(WireValue::Bool(false)) => {
                    writer.write_tag(BcTag::BoolFalse)?;
                }
                EncodeValue::Borrowed(WireValue::Bool(true)) => {
                    writer.write_tag(BcTag::BoolTrue)?;
                }
                EncodeValue::Borrowed(WireValue::Int32(value)) => {
                    writer.write_tag(BcTag::Int32)?;
                    writer.write_i32(*value)?;
                }
                EncodeValue::Borrowed(WireValue::Float64Bits(bits)) => {
                    writer.write_tag(BcTag::Float64)?;
                    writer.write_f64(f64::from_bits(*bits))?;
                }
                EncodeValue::Borrowed(WireValue::String(value)) => {
                    writer.write_tag(BcTag::String)?;
                    writer.write_string(value)?;
                }
                EncodeValue::Borrowed(WireValue::BigInt(payload)) => {
                    total_bigint_bytes = total_bigint_bytes.checked_add(payload.len()).ok_or(
                        GraphError::CountOverflow {
                            kind: GraphResourceKind::TotalBigIntBytes,
                        },
                    )?;
                    options
                        .limits
                        .check(GraphResourceKind::TotalBigIntBytes, total_bigint_bytes)?;
                    writer.write_tag(BcTag::BigInt)?;
                    let length =
                        u32::try_from(payload.len()).map_err(|_| GraphError::CountOverflow {
                            kind: GraphResourceKind::BigIntBytes,
                        })?;
                    writer.write_uleb128(length)?;
                    writer.write_bytes(payload)?;
                }
                EncodeValue::Borrowed(&WireValue::Node(ref node)) | EncodeValue::Node(ref node) => {
                    let node_index = node.as_usize();
                    let node_data =
                        graph
                            .nodes
                            .get(node_index)
                            .ok_or(GraphError::InvalidNodeIndex {
                                index: node.zero_based(),
                                node_count: graph.nodes.len(),
                            })?;

                    if options.allow_object_references {
                        if let Some(index) = object_indices.get(node).copied() {
                            writer.write_tag(BcTag::ObjectReference)?;
                            writer.write_uleb128(index)?;
                            continue;
                        }
                        let requested_references = usize::try_from(next_object_index)
                            .ok()
                            .and_then(|count| count.checked_add(1))
                            .ok_or(GraphError::CountOverflow {
                                kind: GraphResourceKind::ObjectReferences,
                            })?;
                        options
                            .limits
                            .check(GraphResourceKind::ObjectReferences, requested_references)?;
                        object_indices
                            .try_reserve(1)
                            .map_err(|_| GraphEncodeError::AllocationFailed)?;
                        object_indices.insert(*node, next_object_index);
                        next_object_index =
                            next_object_index
                                .checked_add(1)
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::ObjectReferences,
                                })?;
                    } else {
                        if active_nodes.contains(node) {
                            return Err(GraphEncodeError::CircularReference { node: *node });
                        }
                        active_nodes
                            .try_reserve(1)
                            .map_err(|_| GraphEncodeError::AllocationFailed)?;
                        active_nodes.insert(*node);
                    }

                    let depth = parent_depth
                        .checked_add(1)
                        .ok_or(GraphError::CountOverflow {
                            kind: GraphResourceKind::NestingDepth,
                        })?;
                    options
                        .limits
                        .check(GraphResourceKind::NestingDepth, depth)?;

                    match node_data {
                        WireNode::Ordinary { properties } => {
                            total_container_entries = total_container_entries
                                .checked_add(properties.len())
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::TotalContainerEntries,
                                })?;
                            options.limits.check(
                                GraphResourceKind::TotalContainerEntries,
                                total_container_entries,
                            )?;
                            writer.write_tag(BcTag::Object)?;
                            writer.write_uleb128(u32::try_from(properties.len()).map_err(
                                |_| GraphError::CountOverflow {
                                    kind: GraphResourceKind::ContainerEntries,
                                },
                            )?)?;
                            let extra = properties.len().checked_mul(2).and_then(|count| {
                                count.checked_add(usize::from(!options.allow_object_references))
                            });
                            tasks
                                .try_reserve(extra.ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::TotalContainerEntries,
                                })?)
                                .map_err(|_| GraphEncodeError::AllocationFailed)?;
                            if !options.allow_object_references {
                                tasks.push(EncodeTask::LeaveNode(*node));
                            }
                            for property in properties.iter().rev() {
                                tasks.push(EncodeTask::Value(
                                    EncodeValue::Borrowed(&property.value),
                                    depth,
                                ));
                                tasks.push(EncodeTask::Key(property.key));
                            }
                        }
                        WireNode::Array { elements } => {
                            total_container_entries = total_container_entries
                                .checked_add(elements.len())
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::TotalContainerEntries,
                                })?;
                            options.limits.check(
                                GraphResourceKind::TotalContainerEntries,
                                total_container_entries,
                            )?;
                            writer.write_tag(BcTag::Array)?;
                            writer.write_uleb128(u32::try_from(elements.len()).map_err(
                                |_| GraphError::CountOverflow {
                                    kind: GraphResourceKind::ContainerEntries,
                                },
                            )?)?;
                            let extra = elements
                                .len()
                                .checked_add(usize::from(!options.allow_object_references));
                            tasks
                                .try_reserve(extra.ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::TotalContainerEntries,
                                })?)
                                .map_err(|_| GraphEncodeError::AllocationFailed)?;
                            if !options.allow_object_references {
                                tasks.push(EncodeTask::LeaveNode(*node));
                            }
                            for element in elements.iter().rev() {
                                tasks
                                    .push(EncodeTask::Value(EncodeValue::Borrowed(element), depth));
                            }
                        }
                        WireNode::ArrayBuffer {
                            bytes,
                            max_byte_length,
                        } => {
                            let byte_length =
                                validate_array_buffer_node(*node, bytes.len(), *max_byte_length)?;
                            options
                                .limits
                                .check(GraphResourceKind::ArrayBufferBytes, bytes.len())?;
                            total_array_buffer_bytes = total_array_buffer_bytes
                                .checked_add(bytes.len())
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::TotalArrayBufferBytes,
                                })?;
                            options.limits.check(
                                GraphResourceKind::TotalArrayBufferBytes,
                                total_array_buffer_bytes,
                            )?;
                            writer.write_tag(BcTag::ArrayBuffer)?;
                            writer.write_uleb128(byte_length)?;
                            writer.write_uleb128(max_byte_length.unwrap_or(u32::MAX))?;
                            writer.write_bytes(bytes)?;
                            if !options.allow_object_references {
                                let was_active = active_nodes.remove(node);
                                debug_assert!(was_active);
                            }
                        }
                        WireNode::TypedArray {
                            kind,
                            length,
                            byte_offset,
                            buffer,
                        } => {
                            validate_typed_array_node(
                                graph,
                                *node,
                                *kind,
                                *length,
                                *byte_offset,
                                *buffer,
                            )?;
                            writer.write_tag(BcTag::TypedArray)?;
                            writer.write_u8(kind.to_wire_byte())?;
                            writer.write_uleb128(*length)?;
                            writer.write_uleb128(*byte_offset)?;
                            tasks
                                .try_reserve(1 + usize::from(!options.allow_object_references))
                                .map_err(|_| GraphEncodeError::AllocationFailed)?;
                            if !options.allow_object_references {
                                tasks.push(EncodeTask::LeaveNode(*node));
                            }
                            tasks.push(EncodeTask::Value(EncodeValue::Node(*buffer), depth));
                        }
                        WireNode::ObjectValue { primitive } => {
                            writer.write_tag(BcTag::ObjectValue)?;
                            tasks
                                .try_reserve(1 + usize::from(!options.allow_object_references))
                                .map_err(|_| GraphEncodeError::AllocationFailed)?;
                            if !options.allow_object_references {
                                tasks.push(EncodeTask::LeaveNode(*node));
                            }
                            tasks.push(EncodeTask::Value(
                                EncodeValue::Borrowed(primitive.as_wire_value()),
                                depth,
                            ));
                        }
                        WireNode::Date { time_value } => {
                            writer.write_tag(BcTag::Date)?;
                            tasks
                                .try_reserve(1 + usize::from(!options.allow_object_references))
                                .map_err(|_| GraphEncodeError::AllocationFailed)?;
                            if !options.allow_object_references {
                                tasks.push(EncodeTask::LeaveNode(*node));
                            }
                            tasks.push(EncodeTask::Value(
                                EncodeValue::Borrowed(time_value.as_wire_value()),
                                depth,
                            ));
                        }
                    }
                }
            },
        }
    }

    Ok(writer.into_bytes())
}

fn build_encode_plan<'a>(
    graph: &'a WireGraph,
    options: GraphEncodeOptions,
) -> Result<EncodePlan<'a>, GraphEncodeError> {
    let mut plan = EncodePlan {
        atoms: Vec::new(),
        source_atom_keys: HashMap::new(),
    };
    let mut canonical_atoms = HashMap::new();

    // Scratch state is sparse and reachable-only: an unreferenced arena tail
    // must not consume writer memory or semantic node budget.
    let mut visited_nodes = HashSet::new();
    let mut active_nodes = HashSet::new();

    let mut emitted_references = 0_usize;
    let mut total_container_entries = 0_usize;
    let mut total_bigint_bytes = 0_usize;
    let mut total_array_buffer_bytes = 0_usize;
    let mut tasks = Vec::new();
    tasks
        .try_reserve(1)
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    tasks.push(EncodeTask::Value(EncodeValue::Borrowed(&graph.root), 0));

    while let Some(task) = tasks.pop() {
        match task {
            EncodeTask::Key(key) => {
                plan.encounter_key(graph, key, &mut canonical_atoms)?;
            }
            EncodeTask::LeaveNode(node) => {
                let was_active = active_nodes.remove(&node);
                debug_assert!(was_active);
            }
            EncodeTask::Value(value, parent_depth) => match value {
                EncodeValue::Borrowed(WireValue::BigInt(payload)) => {
                    options
                        .limits
                        .check(GraphResourceKind::BigIntBytes, payload.len())?;
                    if canonical_bigint_length(payload) != payload.len() {
                        return Err(GraphEncodeError::NonCanonicalBigInt);
                    }
                    total_bigint_bytes = total_bigint_bytes.checked_add(payload.len()).ok_or(
                        GraphError::CountOverflow {
                            kind: GraphResourceKind::TotalBigIntBytes,
                        },
                    )?;
                    options
                        .limits
                        .check(GraphResourceKind::TotalBigIntBytes, total_bigint_bytes)?;
                }
                EncodeValue::Borrowed(&WireValue::Node(ref node)) | EncodeValue::Node(ref node) => {
                    let node_data =
                        graph
                            .nodes
                            .get(node.as_usize())
                            .ok_or(GraphError::InvalidNodeIndex {
                                index: node.zero_based(),
                                node_count: graph.nodes.len(),
                            })?;
                    let first_visit = !visited_nodes.contains(node);
                    if options.allow_object_references {
                        if !first_visit {
                            continue;
                        }
                        emitted_references =
                            emitted_references
                                .checked_add(1)
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::ObjectReferences,
                                })?;
                        options
                            .limits
                            .check(GraphResourceKind::ObjectReferences, emitted_references)?;
                    } else {
                        if active_nodes.contains(node) {
                            return Err(GraphEncodeError::CircularReference { node: *node });
                        }
                        active_nodes
                            .try_reserve(1)
                            .map_err(|_| GraphEncodeError::AllocationFailed)?;
                        active_nodes.insert(*node);
                    }

                    let depth = parent_depth
                        .checked_add(1)
                        .ok_or(GraphError::CountOverflow {
                            kind: GraphResourceKind::NestingDepth,
                        })?;
                    options
                        .limits
                        .check(GraphResourceKind::NestingDepth, depth)?;

                    if let Some(entry_count) = match node_data {
                        WireNode::Ordinary { properties } => Some(properties.len()),
                        WireNode::Array { elements } => Some(elements.len()),
                        WireNode::ArrayBuffer { .. }
                        | WireNode::TypedArray { .. }
                        | WireNode::ObjectValue { .. }
                        | WireNode::Date { .. } => None,
                    } {
                        options
                            .limits
                            .check(GraphResourceKind::ContainerEntries, entry_count)?;
                        total_container_entries = total_container_entries
                            .checked_add(entry_count)
                            .ok_or(GraphError::CountOverflow {
                                kind: GraphResourceKind::TotalContainerEntries,
                            })?;
                        options.limits.check(
                            GraphResourceKind::TotalContainerEntries,
                            total_container_entries,
                        )?;
                    }

                    if let WireNode::ArrayBuffer { bytes, .. } = node_data {
                        options
                            .limits
                            .check(GraphResourceKind::ArrayBufferBytes, bytes.len())?;
                        total_array_buffer_bytes = total_array_buffer_bytes
                            .checked_add(bytes.len())
                            .ok_or(GraphError::CountOverflow {
                                kind: GraphResourceKind::TotalArrayBufferBytes,
                            })?;
                        options.limits.check(
                            GraphResourceKind::TotalArrayBufferBytes,
                            total_array_buffer_bytes,
                        )?;
                    }

                    if first_visit {
                        let requested_nodes = visited_nodes.len().checked_add(1).ok_or(
                            GraphError::CountOverflow {
                                kind: GraphResourceKind::Nodes,
                            },
                        )?;
                        options
                            .limits
                            .check(GraphResourceKind::Nodes, requested_nodes)?;
                        visited_nodes
                            .try_reserve(1)
                            .map_err(|_| GraphEncodeError::AllocationFailed)?;
                        validate_node(graph, *node, node_data)?;
                        visited_nodes.insert(*node);
                    }

                    let task_count = match node_data {
                        WireNode::Ordinary { properties } => properties.len().checked_mul(2),
                        WireNode::Array { elements } => Some(elements.len()),
                        WireNode::ArrayBuffer { .. } => Some(0),
                        WireNode::TypedArray { .. } => Some(1),
                        WireNode::ObjectValue { .. } => Some(1),
                        WireNode::Date { .. } => Some(1),
                    }
                    .and_then(|count| {
                        count.checked_add(usize::from(!options.allow_object_references))
                    })
                    .ok_or(GraphError::CountOverflow {
                        kind: GraphResourceKind::TotalContainerEntries,
                    })?;
                    tasks
                        .try_reserve(task_count)
                        .map_err(|_| GraphEncodeError::AllocationFailed)?;
                    if !options.allow_object_references {
                        tasks.push(EncodeTask::LeaveNode(*node));
                    }
                    match node_data {
                        WireNode::Ordinary { properties } => {
                            for property in properties.iter().rev() {
                                tasks.push(EncodeTask::Value(
                                    EncodeValue::Borrowed(&property.value),
                                    depth,
                                ));
                                tasks.push(EncodeTask::Key(property.key));
                            }
                        }
                        WireNode::Array { elements } => {
                            for element in elements.iter().rev() {
                                tasks
                                    .push(EncodeTask::Value(EncodeValue::Borrowed(element), depth));
                            }
                        }
                        WireNode::ArrayBuffer { .. } => {}
                        WireNode::TypedArray { buffer, .. } => {
                            tasks.push(EncodeTask::Value(EncodeValue::Node(*buffer), depth));
                        }
                        WireNode::ObjectValue { primitive } => {
                            tasks.push(EncodeTask::Value(
                                EncodeValue::Borrowed(primitive.as_wire_value()),
                                depth,
                            ));
                        }
                        WireNode::Date { time_value } => {
                            tasks.push(EncodeTask::Value(
                                EncodeValue::Borrowed(time_value.as_wire_value()),
                                depth,
                            ));
                        }
                    }
                }
                EncodeValue::Borrowed(
                    WireValue::Undefined
                    | WireValue::Null
                    | WireValue::Bool(_)
                    | WireValue::Int32(_)
                    | WireValue::Float64Bits(_)
                    | WireValue::String(_),
                ) => {}
            },
        }
    }

    Ok(plan)
}

impl<'a> EncodePlan<'a> {
    fn encounter_key(
        &mut self,
        graph: &'a WireGraph,
        key: WireKey,
        canonical_atoms: &mut HashMap<SemanticAtomRef<'a>, u32>,
    ) -> Result<(), GraphEncodeError> {
        let atom = match key {
            WireKey::Index(index) => {
                if index > ATOM_MAX_INT {
                    return Err(GraphEncodeError::IntegerAtomOutOfRange { index });
                }
                return Ok(());
            }
            WireKey::Atom(atom) => atom,
        };
        if self.source_atom_keys.contains_key(&atom) {
            return Ok(());
        }
        let string = graph
            .atoms
            .get(atom.as_usize())
            .ok_or(GraphError::InvalidAtomIndex {
                index: atom.zero_based(),
                atom_count: graph.atoms.len(),
            })?;
        let canonical_key = if let Some(index) = numeric_atom_index(string) {
            CanonicalKey::Index(index)
        } else {
            let semantic = SemanticAtomRef(string);
            let atom_index = if let Some(index) = canonical_atoms.get(&semantic).copied() {
                index
            } else {
                if u32::try_from(self.atoms.len())
                    .map_or(true, |length| length >= ATOM_MAX_TABLE_INDEX)
                {
                    return Err(GraphEncodeError::AtomCountOverflow {
                        atom_count: self.atoms.len().saturating_add(1),
                    });
                }
                let index = u32::try_from(self.atoms.len()).map_err(|_| {
                    GraphEncodeError::AtomCountOverflow {
                        atom_count: self.atoms.len(),
                    }
                })?;
                self.atoms
                    .try_reserve(1)
                    .map_err(|_| GraphEncodeError::AllocationFailed)?;
                canonical_atoms
                    .try_reserve(1)
                    .map_err(|_| GraphEncodeError::AllocationFailed)?;
                self.atoms.push(string);
                canonical_atoms.insert(semantic, index);
                index
            };
            CanonicalKey::Header(atom_index)
        };
        self.source_atom_keys
            .try_reserve(1)
            .map_err(|_| GraphEncodeError::AllocationFailed)?;
        self.source_atom_keys.insert(atom, canonical_key);
        Ok(())
    }
}

fn write_key(
    writer: &mut WireWriter,
    plan: &EncodePlan<'_>,
    atom_space: AtomIndexSpace,
    key: WireKey,
) -> Result<(), GraphEncodeError> {
    let canonical = match key {
        WireKey::Index(index) => CanonicalKey::Index(index),
        WireKey::Atom(atom) => plan
            .source_atom_keys
            .get(&atom)
            .copied()
            .ok_or(GraphEncodeError::UnplannedAtom { atom })?,
    };
    let atom = match canonical {
        CanonicalKey::Index(index) => {
            if index > ATOM_MAX_INT {
                return Err(GraphEncodeError::IntegerAtomOutOfRange { index });
            }
            BinaryAtom::Index(index)
        }
        CanonicalKey::Header(index) => {
            let slot =
                atom_space
                    .header_slot(index)
                    .ok_or(GraphEncodeError::AtomCountOverflow {
                        atom_count: plan.atoms.len(),
                    })?;
            BinaryAtom::Header(slot)
        }
    };
    atom_space.encode_metadata_atom(writer, atom)?;
    Ok(())
}

fn validate_node(
    graph: &WireGraph,
    node: NodeId,
    node_data: &WireNode,
) -> Result<(), GraphEncodeError> {
    match node_data {
        WireNode::Ordinary { properties } => {
            let mut property_keys = HashSet::new();
            property_keys
                .try_reserve(properties.len())
                .map_err(|_| GraphEncodeError::AllocationFailed)?;
            for property in properties {
                if !property_keys.insert(semantic_property_key(graph, property.key)?) {
                    return Err(GraphEncodeError::DuplicatePropertyKey { node });
                }
            }
        }
        WireNode::Array { .. } => {}
        WireNode::ArrayBuffer {
            bytes,
            max_byte_length,
        } => {
            validate_array_buffer_node(node, bytes.len(), *max_byte_length)?;
        }
        WireNode::TypedArray {
            kind,
            length,
            byte_offset,
            buffer,
        } => {
            validate_typed_array_node(graph, node, *kind, *length, *byte_offset, *buffer)?;
        }
        WireNode::ObjectValue { .. } => {}
        WireNode::Date { .. } => {}
    }
    Ok(())
}

fn validate_array_buffer_node(
    node: NodeId,
    byte_length: usize,
    max_byte_length: Option<u32>,
) -> Result<u32, GraphEncodeError> {
    validate_array_buffer_layout(byte_length, max_byte_length)
        .map_err(|reason| GraphEncodeError::InvalidArrayBuffer { node, reason })
}

fn validate_typed_array_node(
    graph: &WireGraph,
    node: NodeId,
    kind: TypedArrayKind,
    length: u32,
    byte_offset: u32,
    buffer: NodeId,
) -> Result<(), GraphEncodeError> {
    let backing = graph
        .nodes
        .get(buffer.as_usize())
        .ok_or(GraphError::InvalidNodeIndex {
            index: buffer.zero_based(),
            node_count: graph.nodes.len(),
        })?;
    let WireNode::ArrayBuffer {
        bytes,
        max_byte_length,
    } = backing
    else {
        return Err(GraphEncodeError::InvalidTypedArrayBacking {
            node,
            reason: TypedArrayBackingError::NotArrayBuffer { node: buffer },
        });
    };
    validate_array_buffer_node(buffer, bytes.len(), *max_byte_length)?;
    validate_typed_array_write_layout(kind, length, byte_offset, bytes.len(), *max_byte_length)
        .map_err(|reason| GraphEncodeError::InvalidTypedArray { node, reason })
}

fn semantic_property_key(
    graph: &WireGraph,
    key: WireKey,
) -> Result<SemanticPropertyKey<'_>, GraphEncodeError> {
    match key {
        WireKey::Index(index) if index > ATOM_MAX_INT => {
            Err(GraphEncodeError::IntegerAtomOutOfRange { index })
        }
        WireKey::Index(index) => Ok(SemanticPropertyKey::Index(index)),
        WireKey::Atom(atom) => {
            let string = graph
                .atoms
                .get(atom.as_usize())
                .ok_or(GraphError::InvalidAtomIndex {
                    index: atom.zero_based(),
                    atom_count: graph.atoms.len(),
                })?;
            Ok(numeric_atom_index(string).map_or_else(
                || SemanticPropertyKey::Atom(SemanticAtomRef(string)),
                SemanticPropertyKey::Index,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::super::wire::WireString;
    use super::super::model::{
        ArrayBufferLayoutError, AtomId, BoxedPrimitive, DateNumber, MAX_ARRAY_BUFFER_BYTE_LENGTH,
        WireProperty, WireValue,
    };
    use super::*;

    const LIMITS: GraphLimits = GraphLimits::new(32, 32, 16, 32, 64, 32, 64, 64, 128);

    fn options(allow_object_references: bool) -> GraphEncodeOptions {
        GraphEncodeOptions::new(allow_object_references, 1024, LIMITS)
    }

    fn options_with_limits(
        allow_object_references: bool,
        limits: GraphLimits,
    ) -> GraphEncodeOptions {
        GraphEncodeOptions::new(allow_object_references, 1024, limits)
    }

    fn array_buffer_graph(bytes: &[u8], max_byte_length: Option<u32>) -> WireGraph {
        WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::ArrayBuffer {
                bytes: bytes.to_vec().into_boxed_slice(),
                max_byte_length,
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        }
    }

    fn typed_array_graph(
        kind: TypedArrayKind,
        length: u32,
        byte_offset: u32,
        bytes: &[u8],
        max_byte_length: Option<u32>,
    ) -> WireGraph {
        WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::TypedArray {
                    kind,
                    length,
                    byte_offset,
                    buffer: NodeId::from_zero_based(1),
                },
                WireNode::ArrayBuffer {
                    bytes: bytes.to_vec().into_boxed_slice(),
                    max_byte_length,
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        }
    }

    fn object_value_graph(value: WireValue) -> WireGraph {
        WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::ObjectValue {
                primitive: BoxedPrimitive::try_from_wire_value(value).unwrap(),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        }
    }

    fn date_graph(value: WireValue) -> WireGraph {
        WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::Date {
                time_value: DateNumber::try_from_wire_value(value).unwrap(),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        }
    }

    #[test]
    fn object_vector_matches_pinned_quickjs() {
        let graph = WireGraph {
            atoms: Box::from([WireString::Narrow(Box::from(*b"x"))]),
            nodes: Box::from([WireNode::Ordinary {
                properties: Box::from([WireProperty {
                    key: WireKey::Atom(AtomId::from_zero_based(0)),
                    value: WireValue::Int32(1),
                }]),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [5, 1, 2, b'x', 8, 1, 2, 5, 2]
        );
    }

    #[test]
    fn shared_nodes_follow_the_reference_flag() {
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([
                        WireValue::Node(NodeId::from_zero_based(1)),
                        WireValue::Node(NodeId::from_zero_based(1)),
                    ]),
                },
                WireNode::Ordinary {
                    properties: Box::from([]),
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [5, 0, 9, 2, 8, 0, 8, 0]
        );
        assert_eq!(
            encode_graph(&graph, options(true)).unwrap(),
            [5, 0, 9, 2, 8, 0, 19, 1]
        );
    }

    #[test]
    fn array_buffer_vectors_match_pinned_quickjs() {
        for (maximum, expected) in [
            (
                None,
                &[5, 0, 15, 3, 0xff, 0xff, 0xff, 0xff, 0x0f, 0xa0, 0xa1, 0xa2][..],
            ),
            (Some(3), &[5, 0, 15, 3, 3, 0xa0, 0xa1, 0xa2][..]),
            (Some(8), &[5, 0, 15, 3, 8, 0xa0, 0xa1, 0xa2][..]),
        ] {
            assert_eq!(
                encode_graph(
                    &array_buffer_graph(&[0xa0, 0xa1, 0xa2], maximum),
                    options(false)
                )
                .unwrap(),
                expected
            );
        }

        assert_eq!(
            encode_graph(&array_buffer_graph(&[], None), options(false)).unwrap(),
            [5, 0, 15, 0, 0xff, 0xff, 0xff, 0xff, 0x0f]
        );
        assert_eq!(
            encode_graph(&array_buffer_graph(&[], Some(0)), options(false)).unwrap(),
            [5, 0, 15, 0, 0]
        );
    }

    #[test]
    fn repeated_array_buffer_nodes_follow_the_reference_flag() {
        let buffer = NodeId::from_zero_based(1);
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([WireValue::Node(buffer), WireValue::Node(buffer)]),
                },
                WireNode::ArrayBuffer {
                    bytes: Box::from([0x12, 0x34]),
                    max_byte_length: None,
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };

        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [
                5, 0, 9, 2, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x12, 0x34, 15, 2, 0xff, 0xff,
                0xff, 0xff, 0x0f, 0x12, 0x34,
            ]
        );
        assert_eq!(
            encode_graph(&graph, options(true)).unwrap(),
            [
                5, 0, 9, 2, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x12, 0x34, 19, 1,
            ]
        );
    }

    #[test]
    fn typed_array_kind_vectors_match_pinned_quickjs() {
        for kind in TypedArrayKind::ALL {
            let byte_length = usize::from(kind.element_byte_length());
            let graph = typed_array_graph(kind, 1, 0, &vec![0; byte_length], None);
            let mut expected = vec![
                5,
                0,
                14,
                kind.to_wire_byte(),
                1,
                0,
                15,
                kind.element_byte_length(),
                0xff,
                0xff,
                0xff,
                0xff,
                0x0f,
            ];
            expected.resize(expected.len() + byte_length, 0);
            assert_eq!(encode_graph(&graph, options(false)).unwrap(), expected);
            assert_eq!(encode_graph(&graph, options(true)).unwrap(), expected);
        }
    }

    #[test]
    fn typed_array_views_share_backing_identity_in_quickjs_preorder() {
        let buffer = NodeId::from_zero_based(3);
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([
                        WireValue::Node(NodeId::from_zero_based(1)),
                        WireValue::Node(NodeId::from_zero_based(2)),
                    ]),
                },
                WireNode::TypedArray {
                    kind: TypedArrayKind::Uint8,
                    length: 2,
                    byte_offset: 0,
                    buffer,
                },
                WireNode::TypedArray {
                    kind: TypedArrayKind::Int16,
                    length: 2,
                    byte_offset: 2,
                    buffer,
                },
                WireNode::ArrayBuffer {
                    bytes: Box::from([0; 8]),
                    max_byte_length: None,
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };

        assert_eq!(
            encode_graph(&graph, options(true)).unwrap(),
            [
                5, 0, 9, 2, 14, 2, 2, 0, 15, 8, 0xff, 0xff, 0xff, 0xff, 0x0f, 0, 0, 0, 0, 0, 0, 0,
                0, 14, 3, 2, 2, 19, 2,
            ]
        );

        let one_backing_copy = GraphLimits::new(4, 4, 3, 2, 2, 0, 0, 8, 8);
        assert!(matches!(
            encode_graph(&graph, options_with_limits(false, one_backing_copy)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalArrayBufferBytes,
                requested: 16,
                limit: 8,
            }))
        ));
        assert!(encode_graph(&graph, options_with_limits(true, one_backing_copy)).is_ok());
    }

    #[test]
    fn typed_array_encoder_rejects_invalid_backings_and_impossible_layouts() {
        let node = NodeId::from_zero_based(0);
        let buffer = NodeId::from_zero_based(1);
        let wrong_backing = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::TypedArray {
                    kind: TypedArrayKind::Uint8,
                    length: 0,
                    byte_offset: 0,
                    buffer,
                },
                WireNode::Ordinary {
                    properties: Box::default(),
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(node),
        };
        assert_eq!(
            encode_graph(&wrong_backing, options(false)),
            Err(GraphEncodeError::InvalidTypedArrayBacking {
                node,
                reason: TypedArrayBackingError::NotArrayBuffer { node: buffer },
            })
        );

        let self_backing = WireGraph {
            nodes: Box::from([WireNode::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 0,
                byte_offset: 0,
                buffer: node,
            }]),
            ..wrong_backing.clone()
        };
        assert_eq!(
            encode_graph(&self_backing, options(true)),
            Err(GraphEncodeError::InvalidTypedArrayBacking {
                node,
                reason: TypedArrayBackingError::NotArrayBuffer { node },
            })
        );

        assert_eq!(
            encode_graph(
                &typed_array_graph(TypedArrayKind::Uint16, 1, 1, &[0; 8], None),
                options(false),
            ),
            Err(GraphEncodeError::InvalidTypedArray {
                node,
                reason: TypedArrayLayoutError::UnalignedByteOffset {
                    byte_offset: 1,
                    element_byte_length: 2,
                },
            })
        );
        assert_eq!(
            encode_graph(
                &typed_array_graph(TypedArrayKind::Uint16, 1, 0, &[0], None),
                options(false),
            ),
            Err(GraphEncodeError::InvalidTypedArray {
                node,
                reason: TypedArrayLayoutError::ViewOutOfBounds {
                    byte_offset: 0,
                    length: 1,
                    element_byte_length: 2,
                    backing_byte_length: 1,
                },
            })
        );
    }

    #[test]
    fn typed_array_writer_preserves_quickjs_oob_resizable_view_asymmetry() {
        // A RAB shrink sets the observable element count to zero but retains the
        // original offset. Pinned QuickJS writes these bytes even though its own
        // reader subsequently reports RangeError: invalid length.
        let graph = typed_array_graph(TypedArrayKind::Uint16, 0, 4, &[0, 0], Some(16));
        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [5, 0, 14, 4, 0, 4, 15, 2, 16, 0, 0]
        );

        let fixed = typed_array_graph(TypedArrayKind::Uint16, 0, 4, &[0, 0], None);
        assert_eq!(
            encode_graph(&fixed, options(false)),
            Err(GraphEncodeError::InvalidTypedArray {
                node: NodeId::from_zero_based(0),
                reason: TypedArrayLayoutError::ViewOutOfBounds {
                    byte_offset: 4,
                    length: 0,
                    element_byte_length: 2,
                    backing_byte_length: 2,
                },
            })
        );
    }

    #[test]
    fn typed_array_encoder_bounds_view_and_backing_traversal() {
        let graph = typed_array_graph(TypedArrayKind::Uint8, 1, 0, &[0], None);
        for (allow_references, limits, expected) in [
            (
                false,
                GraphLimits::new(1, 8, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::Nodes,
                    requested: 2,
                    limit: 1,
                },
            ),
            (
                true,
                GraphLimits::new(8, 1, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::ObjectReferences,
                    requested: 2,
                    limit: 1,
                },
            ),
            (
                false,
                GraphLimits::new(8, 8, 1, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::NestingDepth,
                    requested: 2,
                    limit: 1,
                },
            ),
        ] {
            assert_eq!(
                encode_graph(&graph, options_with_limits(allow_references, limits)),
                Err(GraphEncodeError::Graph(expected))
            );
        }
    }

    #[test]
    fn object_value_kind_vectors_match_pinned_quickjs() {
        for (value, expected) in [
            (WireValue::Bool(false), vec![5, 0, 18, 3]),
            (WireValue::Bool(true), vec![5, 0, 18, 4]),
            (WireValue::Int32(42), vec![5, 0, 18, 5, 84]),
            (
                WireValue::Float64Bits((-0.0_f64).to_bits()),
                vec![5, 0, 18, 6, 0, 0, 0, 0, 0, 0, 0, 128],
            ),
            (
                WireValue::Float64Bits(f64::NAN.to_bits()),
                vec![5, 0, 18, 6, 0, 0, 0, 0, 0, 0, 248, 127],
            ),
            (
                WireValue::Float64Bits(0x7ff8_0000_0000_0042),
                vec![5, 0, 18, 6, 66, 0, 0, 0, 0, 0, 248, 127],
            ),
            (
                WireValue::String(WireString::Narrow(Box::from(*b"abc"))),
                vec![5, 0, 18, 7, 6, b'a', b'b', b'c'],
            ),
            (
                WireValue::String(WireString::Wide(Box::from([0xd800]))),
                vec![5, 0, 18, 7, 3, 0, 0xd8],
            ),
            (WireValue::BigInt(Box::from([1])), vec![5, 0, 18, 10, 1, 1]),
        ] {
            let graph = object_value_graph(value);
            assert_eq!(encode_graph(&graph, options(false)).unwrap(), expected);
            assert_eq!(encode_graph(&graph, options(true)).unwrap(), expected);
        }
    }

    #[test]
    fn object_value_identity_and_payload_work_follow_the_reference_flag() {
        let root = NodeId::from_zero_based(0);
        let wrapper = NodeId::from_zero_based(1);
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([WireValue::Node(wrapper), WireValue::Node(wrapper)]),
                },
                WireNode::ObjectValue {
                    primitive: BoxedPrimitive::try_from_wire_value(WireValue::BigInt(Box::from([
                        1,
                    ])))
                    .unwrap(),
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(root),
        };

        assert_eq!(
            encode_graph(&graph, options(true)).unwrap(),
            [5, 0, 9, 2, 18, 10, 1, 1, 19, 1]
        );
        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [5, 0, 9, 2, 18, 10, 1, 1, 18, 10, 1, 1]
        );

        let one_payload = GraphLimits::new(2, 2, 2, 2, 2, 1, 1, 0, 0);
        assert!(encode_graph(&graph, options_with_limits(true, one_payload)).is_ok());
        assert_eq!(
            encode_graph(&graph, options_with_limits(false, one_payload)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalBigIntBytes,
                requested: 2,
                limit: 1,
            }))
        );
    }

    #[test]
    fn object_value_encoder_bounds_wrapper_identity_and_depth() {
        let graph = object_value_graph(WireValue::Int32(1));
        for (allow_references, limits, expected) in [
            (
                false,
                GraphLimits::new(0, 8, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::Nodes,
                    requested: 1,
                    limit: 0,
                },
            ),
            (
                true,
                GraphLimits::new(8, 0, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::ObjectReferences,
                    requested: 1,
                    limit: 0,
                },
            ),
            (
                false,
                GraphLimits::new(8, 8, 0, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::NestingDepth,
                    requested: 1,
                    limit: 0,
                },
            ),
        ] {
            assert_eq!(
                encode_graph(&graph, options_with_limits(allow_references, limits)),
                Err(GraphEncodeError::Graph(expected))
            );
        }
    }

    #[test]
    fn date_number_vectors_match_pinned_quickjs_and_preserve_reader_only_bits() {
        for (value, expected) in [
            (WireValue::Int32(0), vec![5, 0, 17, 5, 0]),
            (WireValue::Int32(42), vec![5, 0, 17, 5, 84]),
            (WireValue::Int32(-1), vec![5, 0, 17, 5, 1]),
            (
                WireValue::Float64Bits(42.0_f64.to_bits()),
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 69, 64],
            ),
            (
                WireValue::Float64Bits((-0.0_f64).to_bits()),
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 0, 128],
            ),
            (
                WireValue::Float64Bits(f64::INFINITY.to_bits()),
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 240, 127],
            ),
            (
                WireValue::Float64Bits(f64::NEG_INFINITY.to_bits()),
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 240, 255],
            ),
            (
                WireValue::Float64Bits(0x7ff8_0000_0000_0042),
                vec![5, 0, 17, 6, 66, 0, 0, 0, 0, 0, 248, 127],
            ),
            (
                WireValue::Float64Bits(0x7ff0_0000_0000_0001),
                vec![5, 0, 17, 6, 1, 0, 0, 0, 0, 0, 240, 127],
            ),
            (
                WireValue::Float64Bits(1),
                vec![5, 0, 17, 6, 1, 0, 0, 0, 0, 0, 0, 0],
            ),
        ] {
            let graph = date_graph(value);
            assert_eq!(encode_graph(&graph, options(false)).unwrap(), expected);
            assert_eq!(encode_graph(&graph, options(true)).unwrap(), expected);
        }
    }

    #[test]
    fn date_identity_follows_the_reference_flag() {
        let date = NodeId::from_zero_based(1);
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([WireValue::Node(date), WireValue::Node(date)]),
                },
                WireNode::Date {
                    time_value: DateNumber::try_from_wire_value(WireValue::Int32(42)).unwrap(),
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };

        assert_eq!(
            encode_graph(&graph, options(true)).unwrap(),
            [5, 0, 9, 2, 17, 5, 84, 19, 1]
        );
        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [5, 0, 9, 2, 17, 5, 84, 17, 5, 84]
        );
    }

    #[test]
    fn date_encoder_bounds_identity_and_depth_without_counting_unreachable_dates() {
        let graph = date_graph(WireValue::Int32(1));
        for (allow_references, limits, expected) in [
            (
                false,
                GraphLimits::new(0, 8, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::Nodes,
                    requested: 1,
                    limit: 0,
                },
            ),
            (
                true,
                GraphLimits::new(8, 0, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::ObjectReferences,
                    requested: 1,
                    limit: 0,
                },
            ),
            (
                false,
                GraphLimits::new(8, 8, 0, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::NestingDepth,
                    requested: 1,
                    limit: 0,
                },
            ),
        ] {
            assert_eq!(
                encode_graph(&graph, options_with_limits(allow_references, limits)),
                Err(GraphEncodeError::Graph(expected))
            );
        }

        let unreachable = WireGraph {
            atoms: Box::from([]),
            nodes: graph.nodes,
            ref_table: Box::from([]),
            root: WireValue::Int32(7),
        };
        let no_nodes = GraphLimits::new(0, 0, 0, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            encode_graph(&unreachable, options_with_limits(true, no_nodes)).unwrap(),
            [5, 0, 5, 14]
        );
    }

    #[test]
    fn cycles_require_and_use_object_references() {
        let graph = WireGraph {
            atoms: Box::from([WireString::Narrow(Box::from(*b"self"))]),
            nodes: Box::from([WireNode::Ordinary {
                properties: Box::from([WireProperty {
                    key: WireKey::Atom(AtomId::from_zero_based(0)),
                    value: WireValue::Node(NodeId::from_zero_based(0)),
                }]),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        assert_eq!(
            encode_graph(&graph, options(false)),
            Err(GraphEncodeError::CircularReference {
                node: NodeId::from_zero_based(0)
            })
        );
        assert_eq!(
            encode_graph(&graph, options(true)).unwrap(),
            [5, 1, 8, b's', b'e', b'l', b'f', 8, 1, 2, 19, 0]
        );
    }

    #[test]
    fn invalid_indices_and_noncanonical_bigints_are_rejected_before_writing() {
        let bad_node = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        assert!(matches!(
            encode_graph(&bad_node, options(false)),
            Err(GraphEncodeError::Graph(GraphError::InvalidNodeIndex { .. }))
        ));

        let bad_bigint = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([]),
            ref_table: Box::from([]),
            root: WireValue::BigInt(Box::from([0])),
        };
        assert_eq!(
            encode_graph(&bad_bigint, options(false)),
            Err(GraphEncodeError::NonCanonicalBigInt)
        );
    }

    #[test]
    fn reference_budget_counts_indices_emitted_by_this_write() {
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::Ordinary {
                properties: Box::from([]),
            }]),
            // A decoded graph's historical table is intentionally not the
            // writer's freshly assigned preorder table.
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        let no_references = GraphLimits::new(32, 0, 16, 32, 64, 32, 64, 0, 0);
        assert_eq!(
            encode_graph(&graph, options_with_limits(true, no_references)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ObjectReferences,
                requested: 1,
                limit: 0,
            }))
        );

        let graph_with_history = WireGraph {
            ref_table: Box::from([NodeId::from_zero_based(0)]),
            ..graph
        };
        assert_eq!(
            encode_graph(
                &graph_with_history,
                options_with_limits(false, no_references)
            )
            .unwrap(),
            [5, 0, 8, 0]
        );
    }

    #[test]
    fn traversal_budgets_count_each_expansion_without_references() {
        let shared = NodeId::from_zero_based(1);
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([WireValue::Node(shared), WireValue::Node(shared)]),
                },
                WireNode::Array {
                    elements: Box::from([WireValue::BigInt(Box::from([1]))]),
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        let one_expansion = GraphLimits::new(32, 32, 16, 32, 4, 32, 1, 0, 0);

        assert_eq!(
            encode_graph(&graph, options_with_limits(false, one_expansion)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalBigIntBytes,
                requested: 2,
                limit: 1,
            }))
        );
        assert_eq!(
            encode_graph(&graph, options_with_limits(true, one_expansion)).unwrap(),
            [5, 0, 9, 2, 9, 1, 10, 1, 1, 19, 1]
        );

        let too_few_entries = GraphLimits::new(32, 32, 16, 32, 2, 32, 64, 0, 0);
        assert_eq!(
            encode_graph(&graph, options_with_limits(false, too_few_entries)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalContainerEntries,
                requested: 3,
                limit: 2,
            }))
        );
    }

    #[test]
    fn array_buffer_budgets_count_emitted_payloads_not_declared_capacity() {
        let buffer = NodeId::from_zero_based(1);
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([
                WireNode::Array {
                    elements: Box::from([WireValue::Node(buffer), WireValue::Node(buffer)]),
                },
                WireNode::ArrayBuffer {
                    bytes: Box::from([1, 2]),
                    max_byte_length: Some(8),
                },
            ]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        // The shared leaf is expanded twice without references, but only two
        // unique reachable nodes consume the node budget.
        let one_emission = GraphLimits::new(2, 32, 16, 32, 64, 32, 64, 2, 2);

        assert_eq!(
            encode_graph(&graph, options_with_limits(false, one_emission)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalArrayBufferBytes,
                requested: 4,
                limit: 2,
            }))
        );
        assert_eq!(
            encode_graph(&graph, options_with_limits(true, one_emission)).unwrap(),
            [5, 0, 9, 2, 15, 2, 8, 1, 2, 19, 1]
        );

        let too_small_per_buffer = GraphLimits::new(32, 32, 16, 32, 64, 32, 64, 1, 4);
        assert_eq!(
            encode_graph(
                &array_buffer_graph(&[1, 2], None),
                options_with_limits(false, too_small_per_buffer),
            ),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ArrayBufferBytes,
                requested: 2,
                limit: 1,
            }))
        );

        let zero_payload_only = GraphLimits::new(1, 0, 1, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            encode_graph(
                &array_buffer_graph(&[], Some(MAX_ARRAY_BUFFER_BYTE_LENGTH)),
                options_with_limits(false, zero_payload_only),
            )
            .unwrap(),
            [5, 0, 15, 0, 0xff, 0xff, 0xff, 0xff, 0x07]
        );
    }

    #[test]
    fn invalid_reachable_array_buffers_report_the_layout_rule() {
        assert_eq!(
            encode_graph(&array_buffer_graph(&[1, 2], Some(1)), options(false)),
            Err(GraphEncodeError::InvalidArrayBuffer {
                node: NodeId::from_zero_based(0),
                reason: ArrayBufferLayoutError::MaximumTooSmall {
                    byte_length: 2,
                    max_byte_length: 1,
                },
            })
        );
        assert_eq!(
            encode_graph(&array_buffer_graph(&[], Some(0x8000_0000)), options(false),),
            Err(GraphEncodeError::InvalidArrayBuffer {
                node: NodeId::from_zero_based(0),
                reason: ArrayBufferLayoutError::MaximumTooLarge {
                    max_byte_length: 0x8000_0000,
                    maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
                },
            })
        );

        let unreachable_invalid = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::ArrayBuffer {
                bytes: Box::from([1]),
                max_byte_length: Some(0),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Null,
        };
        let root_only = GraphLimits::new(0, 0, 0, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            encode_graph(&unreachable_invalid, options_with_limits(false, root_only),).unwrap(),
            [5, 0, 1]
        );
    }

    #[test]
    fn unreachable_arena_nodes_do_not_consume_emitted_budgets() {
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::Array {
                elements: Box::from([WireValue::BigInt(Box::from([1]))]),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Null,
        };
        let root_only = GraphLimits::new(0, 0, 0, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            encode_graph(&graph, options_with_limits(false, root_only)).unwrap(),
            [5, 0, 1]
        );

        let invalid_typed_array = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 1,
                byte_offset: 0,
                buffer: NodeId::from_zero_based(1),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Null,
        };
        assert_eq!(
            encode_graph(&invalid_typed_array, options_with_limits(false, root_only)).unwrap(),
            [5, 0, 1]
        );
    }

    #[test]
    fn atoms_are_pruned_remapped_and_tagged_by_property_encounter() {
        let graph = WireGraph {
            atoms: Box::from([
                WireString::Narrow(Box::from(*b"unused")),
                WireString::Narrow(Box::from(*b"y")),
                WireString::Narrow(Box::from(*b"x")),
                WireString::Wide(Box::from([u16::from(b'0')])),
            ]),
            nodes: Box::from([WireNode::Ordinary {
                properties: Box::from([
                    WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(2)),
                        value: WireValue::Null,
                    },
                    WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(1)),
                        value: WireValue::Bool(true),
                    },
                    WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(3)),
                        value: WireValue::Undefined,
                    },
                ]),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        assert_eq!(
            encode_graph(&graph, options(false)).unwrap(),
            [5, 2, 2, b'x', 2, b'y', 8, 3, 2, 1, 4, 4, 1, 2]
        );
    }

    #[test]
    fn duplicate_semantic_property_keys_are_not_valid_graphs() {
        let graph = WireGraph {
            atoms: Box::from([WireString::Narrow(Box::from(*b"0"))]),
            nodes: Box::from([WireNode::Ordinary {
                properties: Box::from([
                    WireProperty {
                        key: WireKey::Index(0),
                        value: WireValue::Null,
                    },
                    WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Undefined,
                    },
                ]),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        };
        let one_entry = GraphLimits::new(1, 0, 1, 1, 1, 0, 0, 0, 0);
        assert_eq!(
            encode_graph(&graph, options_with_limits(false, one_entry)),
            Err(GraphEncodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ContainerEntries,
                requested: 2,
                limit: 1,
            }))
        );
        assert_eq!(
            encode_graph(&graph, options(false)),
            Err(GraphEncodeError::DuplicatePropertyKey {
                node: NodeId::from_zero_based(0),
            })
        );
    }
}
