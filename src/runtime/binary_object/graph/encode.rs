//! Canonical BC5 writer for a validated, heap-independent [`WireGraph`].

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::hash::{Hash, Hasher};

use super::super::wire::{BcTag, WireError, WireString, WireWriter};
use super::model::{
    AtomId, GraphError, GraphLimits, GraphResourceKind, NodeId, WireGraph, WireKey, WireNode,
    WireValue, canonical_bigint_length, numeric_atom_index, semantic_atom_eq, semantic_atom_hash,
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
    AtomCountOverflow { atom_count: usize },
    IntegerAtomOutOfRange { index: u32 },
    DuplicatePropertyKey { node: NodeId },
    UnplannedAtom { atom: AtomId },
    NonCanonicalBigInt,
    CircularReference { node: NodeId },
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

enum EncodeTask<'a> {
    Value(&'a WireValue, usize),
    Key(WireKey),
    LeaveNode(NodeId),
}

#[derive(Clone, Copy)]
enum CanonicalKey {
    Index(u32),
    Atom(u32),
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
    validate_graph(graph, options.limits)?;
    let plan = build_encode_plan(graph, options)?;
    let atom_count =
        u32::try_from(plan.atoms.len()).map_err(|_| GraphEncodeError::AtomCountOverflow {
            atom_count: plan.atoms.len(),
        })?;
    // Non-integer BC5 atoms occupy the 31-bit QuickJS atom index space after
    // the data-mode `first_atom == 1` offset.
    if atom_count > 0x7fff_ffff {
        return Err(GraphEncodeError::AtomCountOverflow {
            atom_count: plan.atoms.len(),
        });
    }

    let mut writer = WireWriter::new(options.max_output_bytes);
    writer.write_header(atom_count)?;
    for atom in &plan.atoms {
        writer.write_string(atom)?;
    }

    let mut object_indices = Vec::new();
    object_indices
        .try_reserve_exact(graph.nodes.len())
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    object_indices.resize(graph.nodes.len(), None::<u32>);

    let mut active_nodes = Vec::new();
    active_nodes
        .try_reserve_exact(graph.nodes.len())
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    active_nodes.resize(graph.nodes.len(), false);

    let mut next_object_index = 0_u32;
    let mut total_container_entries = 0_usize;
    let mut total_bigint_bytes = 0_usize;
    let mut tasks = Vec::new();
    tasks
        .try_reserve(1)
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    tasks.push(EncodeTask::Value(&graph.root, 0));

    while let Some(task) = tasks.pop() {
        match task {
            EncodeTask::Key(key) => write_key(&mut writer, &plan, key)?,
            EncodeTask::LeaveNode(node) => {
                active_nodes[node.as_usize()] = false;
            }
            EncodeTask::Value(value, parent_depth) => match value {
                WireValue::Undefined => writer.write_tag(BcTag::Undefined)?,
                WireValue::Null => writer.write_tag(BcTag::Null)?,
                WireValue::Bool(false) => writer.write_tag(BcTag::BoolFalse)?,
                WireValue::Bool(true) => writer.write_tag(BcTag::BoolTrue)?,
                WireValue::Int32(value) => {
                    writer.write_tag(BcTag::Int32)?;
                    writer.write_i32(*value)?;
                }
                WireValue::Float64Bits(bits) => {
                    writer.write_tag(BcTag::Float64)?;
                    writer.write_f64(f64::from_bits(*bits))?;
                }
                WireValue::String(value) => {
                    writer.write_tag(BcTag::String)?;
                    writer.write_string(value)?;
                }
                WireValue::BigInt(payload) => {
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
                WireValue::Node(node) => {
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
                        if let Some(index) = object_indices[node_index] {
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
                        object_indices[node_index] = Some(next_object_index);
                        next_object_index =
                            next_object_index
                                .checked_add(1)
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::ObjectReferences,
                                })?;
                    } else {
                        if active_nodes[node_index] {
                            return Err(GraphEncodeError::CircularReference { node: *node });
                        }
                        active_nodes[node_index] = true;
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
                                tasks.push(EncodeTask::Value(&property.value, depth));
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
                                tasks.push(EncodeTask::Value(element, depth));
                            }
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

    let mut seen_nodes = Vec::new();
    seen_nodes
        .try_reserve_exact(graph.nodes.len())
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    seen_nodes.resize(graph.nodes.len(), false);

    let mut active_nodes = Vec::new();
    active_nodes
        .try_reserve_exact(graph.nodes.len())
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    active_nodes.resize(graph.nodes.len(), false);

    let mut validated_nodes = Vec::new();
    validated_nodes
        .try_reserve_exact(graph.nodes.len())
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    validated_nodes.resize(graph.nodes.len(), false);

    let mut emitted_references = 0_usize;
    let mut total_container_entries = 0_usize;
    let mut total_bigint_bytes = 0_usize;
    let mut tasks = Vec::new();
    tasks
        .try_reserve(1)
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    tasks.push(EncodeTask::Value(&graph.root, 0));

    while let Some(task) = tasks.pop() {
        match task {
            EncodeTask::Key(key) => {
                plan.encounter_key(graph, key, &mut canonical_atoms)?;
            }
            EncodeTask::LeaveNode(node) => active_nodes[node.as_usize()] = false,
            EncodeTask::Value(value, parent_depth) => {
                match value {
                    WireValue::BigInt(payload) => {
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
                    WireValue::Node(node) => {
                        let node_data = graph.nodes.get(node.as_usize()).ok_or(
                            GraphError::InvalidNodeIndex {
                                index: node.zero_based(),
                                node_count: graph.nodes.len(),
                            },
                        )?;
                        if options.allow_object_references {
                            if seen_nodes[node.as_usize()] {
                                continue;
                            }
                            emitted_references = emitted_references.checked_add(1).ok_or(
                                GraphError::CountOverflow {
                                    kind: GraphResourceKind::ObjectReferences,
                                },
                            )?;
                            options
                                .limits
                                .check(GraphResourceKind::ObjectReferences, emitted_references)?;
                            seen_nodes[node.as_usize()] = true;
                        } else {
                            if active_nodes[node.as_usize()] {
                                return Err(GraphEncodeError::CircularReference { node: *node });
                            }
                            active_nodes[node.as_usize()] = true;
                        }

                        let depth =
                            parent_depth
                                .checked_add(1)
                                .ok_or(GraphError::CountOverflow {
                                    kind: GraphResourceKind::NestingDepth,
                                })?;
                        options
                            .limits
                            .check(GraphResourceKind::NestingDepth, depth)?;

                        let entry_count = match node_data {
                            WireNode::Ordinary { properties } => properties.len(),
                            WireNode::Array { elements } => elements.len(),
                        };
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

                        if !validated_nodes[node.as_usize()] {
                            validate_node_properties(graph, *node, node_data)?;
                            validated_nodes[node.as_usize()] = true;
                        }

                        let task_count = match node_data {
                            WireNode::Ordinary { properties } => properties.len().checked_mul(2),
                            WireNode::Array { elements } => Some(elements.len()),
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
                                    tasks.push(EncodeTask::Value(&property.value, depth));
                                    tasks.push(EncodeTask::Key(property.key));
                                }
                            }
                            WireNode::Array { elements } => {
                                for element in elements.iter().rev() {
                                    tasks.push(EncodeTask::Value(element, depth));
                                }
                            }
                        }
                    }
                    WireValue::Undefined
                    | WireValue::Null
                    | WireValue::Bool(_)
                    | WireValue::Int32(_)
                    | WireValue::Float64Bits(_)
                    | WireValue::String(_) => {}
                }
            }
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
                if index > 0x7fff_ffff {
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
                if self.atoms.len() >= 0x7fff_ffff {
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
            CanonicalKey::Atom(atom_index)
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
    let encoded = match canonical {
        CanonicalKey::Index(index) => {
            if index > 0x7fff_ffff {
                return Err(GraphEncodeError::IntegerAtomOutOfRange { index });
            }
            (index << 1) | 1
        }
        CanonicalKey::Atom(index) => {
            index
                .checked_add(1)
                .filter(|index| *index <= 0x7fff_ffff)
                .ok_or(GraphEncodeError::AtomCountOverflow {
                    atom_count: plan.atoms.len(),
                })?
                << 1
        }
    };
    writer.write_uleb128(encoded)?;
    Ok(())
}

fn validate_graph(graph: &WireGraph, limits: GraphLimits) -> Result<(), GraphEncodeError> {
    limits.check(GraphResourceKind::Nodes, graph.nodes.len())?;
    Ok(())
}

fn validate_node_properties(
    graph: &WireGraph,
    node: NodeId,
    node_data: &WireNode,
) -> Result<(), GraphEncodeError> {
    let WireNode::Ordinary { properties } = node_data else {
        return Ok(());
    };
    let mut property_keys = HashSet::new();
    property_keys
        .try_reserve(properties.len())
        .map_err(|_| GraphEncodeError::AllocationFailed)?;
    for property in properties {
        if !property_keys.insert(semantic_property_key(graph, property.key)?) {
            return Err(GraphEncodeError::DuplicatePropertyKey { node });
        }
    }
    Ok(())
}

fn semantic_property_key(
    graph: &WireGraph,
    key: WireKey,
) -> Result<SemanticPropertyKey<'_>, GraphEncodeError> {
    match key {
        WireKey::Index(index) if index > 0x7fff_ffff => {
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
    use super::super::model::{AtomId, WireProperty, WireValue};
    use super::*;

    const LIMITS: GraphLimits = GraphLimits::new(32, 32, 16, 32, 64, 32, 64, 0, 0);

    fn options(allow_object_references: bool) -> GraphEncodeOptions {
        GraphEncodeOptions::new(allow_object_references, 1024, LIMITS)
    }

    fn options_with_limits(
        allow_object_references: bool,
        limits: GraphLimits,
    ) -> GraphEncodeOptions {
        GraphEncodeOptions::new(allow_object_references, 1024, limits)
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
    fn unreachable_arena_nodes_do_not_consume_emitted_budgets() {
        let graph = WireGraph {
            atoms: Box::from([]),
            nodes: Box::from([WireNode::Array {
                elements: Box::from([WireValue::BigInt(Box::from([1]))]),
            }]),
            ref_table: Box::from([]),
            root: WireValue::Null,
        };
        let root_only = GraphLimits::new(1, 0, 0, 0, 0, 0, 0, 0, 0);
        assert_eq!(
            encode_graph(&graph, options_with_limits(false, root_only)).unwrap(),
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
        assert_eq!(
            encode_graph(&graph, options(false)),
            Err(GraphEncodeError::DuplicatePropertyKey {
                node: NodeId::from_zero_based(0),
            })
        );
    }
}
