//! Bounded, heap-independent decoder for the first BC5 data-object slice.
//!
//! Containers are assembled through an explicit frame stack. Object and Array
//! identities enter the reference table before any child is read, which
//! preserves QuickJS's preorder reference numbering and cycles. Their values
//! reach the parent or root only after the complete subtree has been consumed.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use crate::bigint::BC5_BIGINT_READ_MAX_BYTES;

use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits};
use super::model::{
    ArrayBufferLayoutError, AtomId, GraphError, GraphLimits, GraphResourceKind, NodeId,
    TypedArrayBackingError, TypedArrayKind, TypedArrayLayoutError, WireGraph, WireKey, WireNode,
    WireProperty, WireValue, canonical_bigint_length, numeric_atom_index, semantic_atom_eq,
    semantic_atom_hash, validate_array_buffer_layout, validate_typed_array_layout,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum DecodeError {
    Wire(WireError),
    Graph(GraphError),
    AtomCountOverflow {
        atom_count: usize,
    },
    ObjectReferencesNotAllowed {
        offset: usize,
    },
    NullPropertyKey {
        offset: usize,
    },
    UnsupportedTag {
        tag: BcTag,
        offset: usize,
    },
    NonCanonicalBigInt {
        offset: usize,
    },
    InvalidArrayBuffer {
        offset: usize,
        reason: ArrayBufferLayoutError,
    },
    InvalidTypedArrayKind {
        offset: usize,
        kind: u8,
    },
    InvalidTypedArrayBacking {
        offset: usize,
        reason: TypedArrayBackingError,
    },
    InvalidTypedArray {
        offset: usize,
        reason: TypedArrayLayoutError,
    },
    InvalidCompletionTarget,
    InvalidNodeState {
        node: NodeId,
    },
}

impl fmt::Display for DecodeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Graph(error) => fmt::Display::fmt(error, formatter),
            Self::AtomCountOverflow { atom_count } => {
                write!(formatter, "wire graph atom count {atom_count} exceeds u32")
            }
            Self::ObjectReferencesNotAllowed { offset } => {
                write!(
                    formatter,
                    "object references are not allowed at byte {offset}"
                )
            }
            Self::NullPropertyKey { offset } => {
                write!(formatter, "null property atom at byte {offset}")
            }
            Self::UnsupportedTag { tag, offset } => {
                write!(
                    formatter,
                    "unsupported data-object tag {tag:?} at byte {offset}"
                )
            }
            Self::NonCanonicalBigInt { offset } => {
                write!(formatter, "non-canonical BigInt payload at byte {offset}")
            }
            Self::InvalidArrayBuffer { offset, reason } => {
                write!(formatter, "invalid ArrayBuffer at byte {offset}: {reason}")
            }
            Self::InvalidTypedArrayKind { offset, kind } => {
                write!(formatter, "invalid TypedArray kind {kind} at byte {offset}")
            }
            Self::InvalidTypedArrayBacking { offset, reason } => {
                write!(
                    formatter,
                    "invalid TypedArray backing at byte {offset}: {reason}"
                )
            }
            Self::InvalidTypedArray { offset, reason } => {
                write!(formatter, "invalid TypedArray at byte {offset}: {reason}")
            }
            Self::InvalidCompletionTarget => {
                formatter.write_str("invalid wire graph completion target")
            }
            Self::InvalidNodeState { node } => write!(
                formatter,
                "wire graph node {} has an invalid decoder state",
                node.zero_based()
            ),
        }
    }
}

impl std::error::Error for DecodeError {}

impl From<WireError> for DecodeError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<GraphError> for DecodeError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

/// Decode one complete BC5 data object into a pure graph.
///
/// `WireCursor::finish` is always called. Its mode decides whether trailing
/// bytes are rejected (strict) or accepted like pinned QuickJS (compatible).
pub(in crate::runtime) fn decode_graph(
    input: &[u8],
    mode: ReaderMode,
    wire_limits: WireLimits,
    graph_limits: GraphLimits,
    allow_object_references: bool,
) -> Result<WireGraph, DecodeError> {
    let mut cursor = WireCursor::new(input, mode, wire_limits)?;
    let header = cursor.read_header()?;
    let atom_count = header.atom_count as usize;
    let mut atoms = Vec::new();
    atoms
        .try_reserve_exact(atom_count)
        .map_err(|_| GraphError::AllocationFailed)?;
    let mut header_atoms = Vec::new();
    header_atoms
        .try_reserve_exact(atom_count)
        .map_err(|_| GraphError::AllocationFailed)?;
    let hash_builder = RandomState::new();
    let mut atoms_by_hash = HashMap::new();
    atoms_by_hash
        .try_reserve(atom_count)
        .map_err(|_| GraphError::AllocationFailed)?;
    for _ in 0..atom_count {
        let value = cursor.read_string()?;
        let key = intern_header_atom(value, &mut atoms, &mut atoms_by_hash, &hash_builder)?;
        header_atoms.push(key);
    }

    let mut state = DecodeState {
        limits: graph_limits,
        allow_object_references,
        nodes: Vec::new(),
        ref_table: Vec::new(),
        frames: Vec::new(),
        total_container_entries: 0,
        total_bigint_bytes: 0,
        total_array_buffer_bytes: 0,
    };
    let mut root = None;

    loop {
        if root.is_some() && state.frames.is_empty() {
            break;
        }

        let return_to = match state.frames.last() {
            Some(active) => {
                let key = active
                    .frame
                    .expects_property_key()
                    .then(|| read_key(&mut cursor, &header_atoms))
                    .transpose()?;
                CompletionTarget::Parent { key }
            }
            None => CompletionTarget::Root,
        };
        match state.read_value(&mut cursor)? {
            ReadStep::Complete(value) => {
                state.deliver_completed(return_to, value, &mut root)?;
            }
            ReadStep::Pending(frame) => {
                state
                    .frames
                    .try_reserve(1)
                    .map_err(|_| GraphError::AllocationFailed)?;
                state.frames.push(ActiveFrame { frame, return_to });
            }
        }

        while state
            .frames
            .last()
            .is_some_and(|active| active.frame.is_complete())
        {
            let active = state
                .frames
                .pop()
                .ok_or(DecodeError::InvalidCompletionTarget)?;
            let value = state.finish_frame(active.frame)?;
            state.deliver_completed(active.return_to, value, &mut root)?;
        }
    }

    // This call is unconditional: QuickJsCompatible itself decides to accept
    // trailing bytes, rather than the graph layer bypassing finalization.
    cursor.finish()?;

    let root = root.ok_or(DecodeError::InvalidCompletionTarget)?;
    let parts = state.into_graph_parts()?;
    Ok(WireGraph {
        atoms: atoms.into_boxed_slice(),
        nodes: parts.nodes,
        ref_table: parts.ref_table,
        root,
    })
}

enum NodeSlot {
    Pending(PendingNodeKind),
    Ready(WireNode),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PendingNodeKind {
    Ordinary,
    Array,
    TypedArray,
}

impl PendingNodeKind {
    fn accepts(self, node: &WireNode) -> bool {
        matches!(
            (self, node),
            (Self::Ordinary, WireNode::Ordinary { .. })
                | (Self::Array, WireNode::Array { .. })
                | (Self::TypedArray, WireNode::TypedArray { .. })
        )
    }
}

#[must_use = "a reserved reference entry must be committed exactly once"]
struct ReferenceReservation {
    expected_count: Option<usize>,
}

#[must_use = "a reserved node must be installed exactly once"]
struct NodeReservation {
    node: NodeId,
    reference: ReferenceReservation,
}

struct DecodedGraphParts {
    nodes: Box<[WireNode]>,
    ref_table: Box<[NodeId]>,
}

struct DecodeState {
    limits: GraphLimits,
    allow_object_references: bool,
    nodes: Vec<NodeSlot>,
    ref_table: Vec<NodeId>,
    frames: Vec<ActiveFrame>,
    total_container_entries: usize,
    total_bigint_bytes: usize,
    total_array_buffer_bytes: usize,
}

impl DecodeState {
    fn read_value(&mut self, cursor: &mut WireCursor<'_>) -> Result<ReadStep, DecodeError> {
        let tag_offset = cursor.position();
        let tag = cursor.read_tag()?;
        let value = match tag {
            BcTag::Null => WireValue::Null,
            BcTag::Undefined => WireValue::Undefined,
            BcTag::BoolFalse => WireValue::Bool(false),
            BcTag::BoolTrue => WireValue::Bool(true),
            BcTag::Int32 => WireValue::Int32(cursor.read_i32()?),
            BcTag::Float64 => WireValue::Float64Bits(cursor.read_f64()?.to_bits()),
            BcTag::String => WireValue::String(cursor.read_string()?),
            BcTag::BigInt => self.read_bigint(cursor)?,
            BcTag::Object => return self.begin_container(cursor, ContainerKind::Ordinary),
            BcTag::Array => return self.begin_container(cursor, ContainerKind::Array),
            BcTag::TypedArray => return self.begin_typed_array(cursor, tag_offset),
            BcTag::ArrayBuffer => self.read_array_buffer(cursor)?,
            BcTag::ObjectReference => {
                if !self.allow_object_references {
                    return Err(DecodeError::ObjectReferencesNotAllowed { offset: tag_offset });
                }
                let index = cursor.read_uleb128()?;
                let node = self.ref_table.get(index as usize).copied().ok_or(
                    GraphError::InvalidReferenceIndex {
                        index,
                        reference_count: self.ref_table.len(),
                    },
                )?;
                WireValue::Node(node)
            }
            BcTag::TemplateObject
            | BcTag::FunctionBytecode
            | BcTag::Module
            | BcTag::SharedArrayBuffer
            | BcTag::Date
            | BcTag::ObjectValue => {
                return Err(DecodeError::UnsupportedTag {
                    tag,
                    offset: tag_offset,
                });
            }
        };

        Ok(ReadStep::Complete(value))
    }

    fn read_bigint(&mut self, cursor: &mut WireCursor<'_>) -> Result<WireValue, DecodeError> {
        let length_offset = cursor.position();
        let byte_length = cursor.read_uleb128()? as usize;
        self.limits
            .check(GraphResourceKind::BigIntBytes, byte_length)?;
        if byte_length > BC5_BIGINT_READ_MAX_BYTES {
            return Err(GraphError::ResourceLimit {
                kind: GraphResourceKind::BigIntBytes,
                requested: byte_length,
                limit: BC5_BIGINT_READ_MAX_BYTES,
            }
            .into());
        }
        self.total_bigint_bytes = checked_total(
            self.total_bigint_bytes,
            byte_length,
            GraphResourceKind::TotalBigIntBytes,
        )?;
        self.limits
            .check(GraphResourceKind::TotalBigIntBytes, self.total_bigint_bytes)?;

        let payload_offset = cursor.position();
        let payload = cursor.read_bytes(byte_length)?;
        let canonical_length = canonical_bigint_length(payload);
        if cursor.mode() == ReaderMode::Strict && canonical_length != payload.len() {
            return Err(DecodeError::NonCanonicalBigInt {
                offset: length_offset,
            });
        }

        let mut canonical = Vec::new();
        canonical
            .try_reserve_exact(canonical_length)
            .map_err(|_| GraphError::AllocationFailed)?;
        canonical.extend_from_slice(&payload[..canonical_length]);
        debug_assert!(canonical_length == 0 || payload_offset < cursor.position());
        Ok(WireValue::BigInt(canonical.into_boxed_slice()))
    }

    fn read_array_buffer(&mut self, cursor: &mut WireCursor<'_>) -> Result<WireValue, DecodeError> {
        let layout_offset = cursor.position();
        let byte_length = cursor.read_uleb128()?;
        let encoded_maximum = cursor.read_uleb128()?;
        let max_byte_length = (encoded_maximum != u32::MAX).then_some(encoded_maximum);
        // QuickJS diagnoses max < current immediately, but performs its 2 GiB
        // constructor-bound checks only after proving the payload is present.
        if let Some(max_byte_length) = max_byte_length {
            if max_byte_length < byte_length {
                return Err(DecodeError::InvalidArrayBuffer {
                    offset: layout_offset,
                    reason: ArrayBufferLayoutError::MaximumTooSmall {
                        byte_length,
                        max_byte_length,
                    },
                });
            }
        }

        let byte_length = byte_length as usize;
        self.limits
            .check(GraphResourceKind::ArrayBufferBytes, byte_length)?;
        self.total_array_buffer_bytes = checked_total(
            self.total_array_buffer_bytes,
            byte_length,
            GraphResourceKind::TotalArrayBufferBytes,
        )?;
        self.limits.check(
            GraphResourceKind::TotalArrayBufferBytes,
            self.total_array_buffer_bytes,
        )?;
        self.check_next_node_depth()?;
        // Preflight the arena/reference work before copying a potentially
        // large payload. The node is installed only after the leaf is complete,
        // matching QuickJS's ArrayBuffer reference-registration point.
        let reservation = self.reserve_node()?;

        let payload = cursor.read_bytes(byte_length)?;
        validate_array_buffer_layout(byte_length, max_byte_length).map_err(|reason| {
            DecodeError::InvalidArrayBuffer {
                offset: layout_offset,
                reason,
            }
        })?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(byte_length)
            .map_err(|_| GraphError::AllocationFailed)?;
        bytes.extend_from_slice(payload);
        let node = self.install_ready_node(
            reservation,
            WireNode::ArrayBuffer {
                bytes: bytes.into_boxed_slice(),
                max_byte_length,
            },
        )?;
        Ok(WireValue::Node(node))
    }

    fn begin_typed_array(
        &mut self,
        cursor: &mut WireCursor<'_>,
        tag_offset: usize,
    ) -> Result<ReadStep, DecodeError> {
        let kind_offset = cursor.position();
        let kind_byte = cursor.read_u8()?;
        let kind = TypedArrayKind::from_wire_byte(kind_byte).ok_or(
            DecodeError::InvalidTypedArrayKind {
                offset: kind_offset,
                kind: kind_byte,
            },
        )?;
        let length = cursor.read_uleb128()?;
        let byte_offset = cursor.read_uleb128()?;

        self.check_next_node_depth()?;
        let reservation = self.reserve_node()?;
        let node = self.install_pending_node(reservation, PendingNodeKind::TypedArray)?;
        Ok(ReadStep::Pending(Frame::TypedArray {
            node,
            offset: tag_offset,
            kind,
            length,
            byte_offset,
            backing: None,
        }))
    }

    fn begin_container(
        &mut self,
        cursor: &mut WireCursor<'_>,
        kind: ContainerKind,
    ) -> Result<ReadStep, DecodeError> {
        let entry_count = cursor.read_uleb128()? as usize;
        self.limits
            .check(GraphResourceKind::ContainerEntries, entry_count)?;
        self.total_container_entries = checked_total(
            self.total_container_entries,
            entry_count,
            GraphResourceKind::TotalContainerEntries,
        )?;
        self.limits.check(
            GraphResourceKind::TotalContainerEntries,
            self.total_container_entries,
        )?;

        self.check_next_node_depth()?;

        let reservation = self.reserve_node()?;
        let node_id = self.install_pending_node(reservation, kind.pending_node_kind())?;
        let frame = Frame::new(kind, node_id, entry_count)?;
        Ok(ReadStep::Pending(frame))
    }

    fn deliver_completed(
        &mut self,
        target: CompletionTarget,
        value: WireValue,
        root: &mut Option<WireValue>,
    ) -> Result<(), DecodeError> {
        match target {
            CompletionTarget::Root => {
                if root.replace(value).is_some() {
                    return Err(DecodeError::InvalidCompletionTarget);
                }
            }
            CompletionTarget::Parent { key } => {
                let parent = self
                    .frames
                    .last_mut()
                    .ok_or(DecodeError::InvalidCompletionTarget)?;
                parent.frame.attach(key, value)?;
            }
        }
        Ok(())
    }

    fn finish_frame(&mut self, frame: Frame) -> Result<WireValue, DecodeError> {
        let (node, replacement) =
            match frame {
                Frame::Ordinary {
                    node, properties, ..
                } => (
                    node,
                    WireNode::Ordinary {
                        properties: properties.into_boxed_slice(),
                    },
                ),
                Frame::Array { node, elements, .. } => (
                    node,
                    WireNode::Array {
                        elements: elements.into_boxed_slice(),
                    },
                ),
                Frame::TypedArray {
                    node,
                    offset,
                    kind,
                    length,
                    byte_offset,
                    backing,
                } => {
                    let buffer = match backing.ok_or(DecodeError::InvalidCompletionTarget)? {
                        WireValue::Node(buffer) => buffer,
                        _ => {
                            return Err(DecodeError::InvalidTypedArrayBacking {
                                offset,
                                reason: TypedArrayBackingError::NotObject,
                            });
                        }
                    };
                    let node_count = self.nodes.len();
                    let backing_byte_length = match self.nodes.get(buffer.as_usize()).ok_or(
                        GraphError::InvalidNodeIndex {
                            index: buffer.zero_based(),
                            node_count,
                        },
                    )? {
                        NodeSlot::Ready(WireNode::ArrayBuffer { bytes, .. }) => bytes.len(),
                        NodeSlot::Ready(_) => {
                            return Err(DecodeError::InvalidTypedArrayBacking {
                                offset,
                                reason: TypedArrayBackingError::NotArrayBuffer { node: buffer },
                            });
                        }
                        NodeSlot::Pending(PendingNodeKind::TypedArray) => {
                            return Err(DecodeError::InvalidTypedArrayBacking {
                                offset,
                                reason: TypedArrayBackingError::Pending { node: buffer },
                            });
                        }
                        NodeSlot::Pending(PendingNodeKind::Ordinary | PendingNodeKind::Array) => {
                            return Err(DecodeError::InvalidTypedArrayBacking {
                                offset,
                                reason: TypedArrayBackingError::NotArrayBuffer { node: buffer },
                            });
                        }
                    };
                    validate_typed_array_layout(kind, length, byte_offset, backing_byte_length)
                        .map_err(|reason| DecodeError::InvalidTypedArray { offset, reason })?;
                    (
                        node,
                        WireNode::TypedArray {
                            kind,
                            length,
                            byte_offset,
                            buffer,
                        },
                    )
                }
            };
        self.complete_node(node, replacement)?;
        Ok(WireValue::Node(node))
    }

    fn check_next_node_depth(&self) -> Result<(), GraphError> {
        let depth = self
            .frames
            .len()
            .checked_add(1)
            .ok_or(GraphError::CountOverflow {
                kind: GraphResourceKind::NestingDepth,
            })?;
        self.limits.check(GraphResourceKind::NestingDepth, depth)
    }

    fn reserve_node(&mut self) -> Result<NodeReservation, DecodeError> {
        let raw_index = u32::try_from(self.nodes.len()).map_err(|_| GraphError::CountOverflow {
            kind: GraphResourceKind::Nodes,
        })?;
        let requested_nodes = self
            .nodes
            .len()
            .checked_add(1)
            .ok_or(GraphError::CountOverflow {
                kind: GraphResourceKind::Nodes,
            })?;
        self.limits
            .check(GraphResourceKind::Nodes, requested_nodes)?;
        self.nodes
            .try_reserve(1)
            .map_err(|_| GraphError::AllocationFailed)?;

        let reference = self.reserve_reference_entry()?;
        Ok(NodeReservation {
            node: NodeId::from_zero_based(raw_index),
            reference,
        })
    }

    fn reserve_reference_entry(&mut self) -> Result<ReferenceReservation, GraphError> {
        if !self.allow_object_references {
            return Ok(ReferenceReservation {
                expected_count: None,
            });
        }

        let expected_count = self.ref_table.len();
        let requested_references =
            expected_count
                .checked_add(1)
                .ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::ObjectReferences,
                })?;
        self.limits
            .check(GraphResourceKind::ObjectReferences, requested_references)?;
        self.ref_table
            .try_reserve(1)
            .map_err(|_| GraphError::AllocationFailed)?;
        Ok(ReferenceReservation {
            expected_count: Some(expected_count),
        })
    }

    fn install_pending_node(
        &mut self,
        reservation: NodeReservation,
        kind: PendingNodeKind,
    ) -> Result<NodeId, DecodeError> {
        self.install_node(reservation, NodeSlot::Pending(kind))
    }

    fn install_ready_node(
        &mut self,
        reservation: NodeReservation,
        node: WireNode,
    ) -> Result<NodeId, DecodeError> {
        self.install_node(reservation, NodeSlot::Ready(node))
    }

    fn install_node(
        &mut self,
        reservation: NodeReservation,
        slot: NodeSlot,
    ) -> Result<NodeId, DecodeError> {
        let node = reservation.node;
        if node.as_usize() != self.nodes.len() {
            return Err(DecodeError::InvalidNodeState { node });
        }
        self.validate_reference_reservation(node, &reservation.reference)?;

        self.nodes.push(slot);
        self.append_reference_entry(node, reservation.reference)?;
        Ok(node)
    }

    fn append_reference_entry(
        &mut self,
        node: NodeId,
        reservation: ReferenceReservation,
    ) -> Result<(), DecodeError> {
        self.validate_node_index(node)?;
        self.validate_reference_reservation(node, &reservation)?;
        if reservation.expected_count.is_some() {
            self.ref_table.push(node);
        }
        Ok(())
    }

    // The first production caller will be ObjectValue. Keeping this as one
    // operation prevents a valid alias from becoming stale between reserve
    // and commit when that tag is admitted.
    #[cfg_attr(not(test), allow(dead_code))]
    fn append_reference_alias(&mut self, node: NodeId) -> Result<(), DecodeError> {
        self.validate_node_index(node)?;
        let reservation = self.reserve_reference_entry()?;
        self.append_reference_entry(node, reservation)
    }

    fn validate_node_index(&self, node: NodeId) -> Result<(), DecodeError> {
        let node_count = self.nodes.len();
        if node.as_usize() >= node_count {
            return Err(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count,
            }
            .into());
        }
        Ok(())
    }

    fn validate_reference_reservation(
        &self,
        node: NodeId,
        reservation: &ReferenceReservation,
    ) -> Result<(), DecodeError> {
        let valid = match reservation.expected_count {
            Some(expected_count) => {
                self.allow_object_references && expected_count == self.ref_table.len()
            }
            None => !self.allow_object_references,
        };
        if !valid {
            return Err(DecodeError::InvalidNodeState { node });
        }
        Ok(())
    }

    fn complete_node(&mut self, node: NodeId, value: WireNode) -> Result<(), DecodeError> {
        let node_count = self.nodes.len();
        let slot = self
            .nodes
            .get_mut(node.as_usize())
            .ok_or(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count,
            })?;
        let NodeSlot::Pending(expected) = slot else {
            return Err(DecodeError::InvalidNodeState { node });
        };
        if !expected.accepts(&value) {
            return Err(DecodeError::InvalidNodeState { node });
        }
        *slot = NodeSlot::Ready(value);
        Ok(())
    }

    fn into_graph_parts(self) -> Result<DecodedGraphParts, DecodeError> {
        let mut ready_nodes = Vec::new();
        ready_nodes
            .try_reserve_exact(self.nodes.len())
            .map_err(|_| GraphError::AllocationFailed)?;
        for (index, slot) in self.nodes.into_iter().enumerate() {
            match slot {
                NodeSlot::Ready(node) => ready_nodes.push(node),
                NodeSlot::Pending(_) => {
                    let index = u32::try_from(index).map_err(|_| GraphError::CountOverflow {
                        kind: GraphResourceKind::Nodes,
                    })?;
                    return Err(DecodeError::InvalidNodeState {
                        node: NodeId::from_zero_based(index),
                    });
                }
            }
        }
        Ok(DecodedGraphParts {
            nodes: ready_nodes.into_boxed_slice(),
            ref_table: self.ref_table.into_boxed_slice(),
        })
    }
}

#[derive(Clone, Copy)]
enum ContainerKind {
    Ordinary,
    Array,
}

impl ContainerKind {
    const fn pending_node_kind(self) -> PendingNodeKind {
        match self {
            Self::Ordinary => PendingNodeKind::Ordinary,
            Self::Array => PendingNodeKind::Array,
        }
    }
}

enum ReadStep {
    Complete(WireValue),
    Pending(Frame),
}

#[derive(Clone, Copy)]
enum CompletionTarget {
    Root,
    Parent { key: Option<DecodedPropertyKey> },
}

struct ActiveFrame {
    frame: Frame,
    return_to: CompletionTarget,
}

enum Frame {
    Ordinary {
        node: NodeId,
        expected: usize,
        consumed: usize,
        properties: Vec<WireProperty>,
        property_indices: HashMap<WireKey, usize>,
    },
    Array {
        node: NodeId,
        expected: usize,
        elements: Vec<WireValue>,
    },
    TypedArray {
        node: NodeId,
        offset: usize,
        kind: TypedArrayKind,
        length: u32,
        byte_offset: u32,
        backing: Option<WireValue>,
    },
}

impl Frame {
    fn new(kind: ContainerKind, node: NodeId, expected: usize) -> Result<Self, GraphError> {
        match kind {
            ContainerKind::Ordinary => {
                let mut properties = Vec::new();
                properties
                    .try_reserve_exact(expected)
                    .map_err(|_| GraphError::AllocationFailed)?;
                let mut property_indices = HashMap::new();
                property_indices
                    .try_reserve(expected)
                    .map_err(|_| GraphError::AllocationFailed)?;
                Ok(Self::Ordinary {
                    node,
                    expected,
                    consumed: 0,
                    properties,
                    property_indices,
                })
            }
            ContainerKind::Array => {
                let mut elements = Vec::new();
                elements
                    .try_reserve_exact(expected)
                    .map_err(|_| GraphError::AllocationFailed)?;
                Ok(Self::Array {
                    node,
                    expected,
                    elements,
                })
            }
        }
    }

    fn attach(
        &mut self,
        key: Option<DecodedPropertyKey>,
        value: WireValue,
    ) -> Result<(), GraphError> {
        match self {
            Self::Ordinary {
                expected,
                consumed,
                properties,
                property_indices,
                ..
            } => {
                let key = key.ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::ContainerEntries,
                })?;
                if *consumed >= *expected {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                *consumed += 1;
                if let DecodedPropertyKey::Define(key) = key {
                    if let Some(index) = property_indices.get(&key).copied() {
                        properties[index].value = value;
                    } else {
                        let index = properties.len();
                        properties.push(WireProperty { key, value });
                        property_indices.insert(key, index);
                    }
                }
            }
            Self::Array {
                expected, elements, ..
            } => {
                if key.is_some() || elements.len() >= *expected {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                elements.push(value);
            }
            Self::TypedArray { backing, .. } => {
                if key.is_some() || backing.is_some() {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                *backing = Some(value);
            }
        }
        Ok(())
    }

    fn expects_property_key(&self) -> bool {
        matches!(self, Self::Ordinary { .. })
    }

    fn is_complete(&self) -> bool {
        match self {
            Self::Ordinary {
                expected, consumed, ..
            } => consumed == expected,
            Self::Array {
                expected, elements, ..
            } => elements.len() == *expected,
            Self::TypedArray { backing, .. } => backing.is_some(),
        }
    }
}

#[derive(Clone, Copy)]
enum DecodedPropertyKey {
    Define(WireKey),
    Ignore,
}

fn read_key(
    cursor: &mut WireCursor<'_>,
    header_atoms: &[WireKey],
) -> Result<DecodedPropertyKey, DecodeError> {
    let offset = cursor.position();
    let encoded = cursor.read_uleb128()?;
    if encoded & 1 != 0 {
        return Ok(DecodedPropertyKey::Define(WireKey::Index(encoded >> 1)));
    }
    if encoded == 0 {
        return match cursor.mode() {
            ReaderMode::Strict => Err(DecodeError::NullPropertyKey { offset }),
            ReaderMode::QuickJsCompatible => Ok(DecodedPropertyKey::Ignore),
        };
    }

    // Data-object mode uses first_atom == 1.
    let table_index = encoded >> 1;
    let zero_based = table_index - 1;
    let key =
        header_atoms
            .get(zero_based as usize)
            .copied()
            .ok_or(GraphError::InvalidAtomIndex {
                index: zero_based,
                atom_count: header_atoms.len(),
            })?;
    Ok(DecodedPropertyKey::Define(key))
}

fn intern_header_atom(
    value: super::super::wire::WireString,
    atoms: &mut Vec<super::super::wire::WireString>,
    atoms_by_hash: &mut HashMap<u64, AtomId>,
    hash_builder: &RandomState,
) -> Result<WireKey, DecodeError> {
    if let Some(index) = numeric_atom_index(&value) {
        return Ok(WireKey::Index(index));
    }

    let mut hasher = hash_builder.build_hasher();
    semantic_atom_hash(&value, &mut hasher);
    let hash = hasher.finish();
    if let Some(first) = atoms_by_hash.get(&hash).copied() {
        if semantic_atom_eq(&atoms[first.as_usize()], &value) {
            return Ok(WireKey::Atom(first));
        }
        // A width-independent hash collision is rare, but must not change atom
        // identity. Colliding entries retain the first hash-table slot and are
        // found by exact code-unit comparison here.
        if let Some((index, _)) = atoms
            .iter()
            .enumerate()
            .find(|(_, candidate)| semantic_atom_eq(candidate, &value))
        {
            let atom_index = u32::try_from(index)
                .map_err(|_| DecodeError::AtomCountOverflow { atom_count: index })?;
            return Ok(WireKey::Atom(AtomId::from_zero_based(atom_index)));
        }
    }

    let atom_index = u32::try_from(atoms.len()).map_err(|_| DecodeError::AtomCountOverflow {
        atom_count: atoms.len(),
    })?;
    let atom = AtomId::from_zero_based(atom_index);
    atoms_by_hash.entry(hash).or_insert(atom);
    atoms.push(value);
    Ok(WireKey::Atom(atom))
}

fn checked_total(
    current: usize,
    additional: usize,
    kind: GraphResourceKind,
) -> Result<usize, GraphError> {
    current
        .checked_add(additional)
        .ok_or(GraphError::CountOverflow { kind })
}

#[cfg(test)]
mod tests {
    use super::super::super::wire::WireString;
    use super::super::model::MAX_ARRAY_BUFFER_BYTE_LENGTH;
    use super::*;

    const WIRE_LIMITS: WireLimits = WireLimits::new(4096, 32, 1024, 2048);
    const GRAPH_LIMITS: GraphLimits =
        GraphLimits::new(64, 64, 32, 128, 256, 1024, 2048, 1024, 2048);

    fn decode(input: &[u8], mode: ReaderMode, references: bool) -> Result<WireGraph, DecodeError> {
        decode_graph(input, mode, WIRE_LIMITS, GRAPH_LIMITS, references)
    }

    fn empty_state(limits: GraphLimits, references: bool) -> DecodeState {
        DecodeState {
            limits,
            allow_object_references: references,
            nodes: Vec::new(),
            ref_table: Vec::new(),
            frames: Vec::new(),
            total_container_entries: 0,
            total_bigint_bytes: 0,
            total_array_buffer_bytes: 0,
        }
    }

    #[test]
    fn pending_nodes_are_referenceable_and_complete_exactly_once() {
        let mut state = empty_state(GRAPH_LIMITS, true);
        let reservation = state.reserve_node().unwrap();
        assert!(state.nodes.is_empty());
        assert!(state.ref_table.is_empty());

        let node = state
            .install_pending_node(reservation, PendingNodeKind::Array)
            .unwrap();
        assert!(matches!(
            state.nodes.as_slice(),
            [NodeSlot::Pending(PendingNodeKind::Array)]
        ));
        assert_eq!(state.ref_table.as_slice(), &[node]);

        // ObjectValue will use the same bounded operation to append an alias
        // even when the referenced identity has not completed yet.
        state.append_reference_alias(node).unwrap();
        assert_eq!(state.ref_table.as_slice(), &[node, node]);
        assert_eq!(state.nodes.len(), 1);

        let completed = WireNode::Array {
            elements: Box::from([WireValue::Node(node)]),
        };
        state.complete_node(node, completed.clone()).unwrap();
        assert!(matches!(
            state.nodes.as_slice(),
            [NodeSlot::Ready(WireNode::Array { .. })]
        ));
        assert_eq!(
            state.complete_node(
                node,
                WireNode::Array {
                    elements: Box::default()
                }
            ),
            Err(DecodeError::InvalidNodeState { node })
        );

        let parts = state.into_graph_parts().unwrap();
        assert_eq!(parts.nodes.as_ref(), &[completed]);
        assert_eq!(parts.ref_table.as_ref(), &[node, node]);
    }

    #[test]
    fn pending_nodes_cannot_escape_decoder_finalization() {
        let mut state = empty_state(GRAPH_LIMITS, false);
        let reservation = state.reserve_node().unwrap();
        let node = state
            .install_pending_node(reservation, PendingNodeKind::Array)
            .unwrap();
        assert!(matches!(
            state.into_graph_parts(),
            Err(DecodeError::InvalidNodeState { node: failed }) if failed == node
        ));
    }

    #[test]
    fn pending_node_kinds_reject_cross_kind_completion_without_mutation() {
        let mut state = empty_state(GRAPH_LIMITS, false);
        let reservation = state.reserve_node().unwrap();
        let node = state
            .install_pending_node(reservation, PendingNodeKind::Ordinary)
            .unwrap();
        assert_eq!(
            state.complete_node(
                node,
                WireNode::Array {
                    elements: Box::default(),
                },
            ),
            Err(DecodeError::InvalidNodeState { node })
        );
        assert!(matches!(
            state.nodes.as_slice(),
            [NodeSlot::Pending(PendingNodeKind::Ordinary)]
        ));
        state
            .complete_node(
                node,
                WireNode::Ordinary {
                    properties: Box::default(),
                },
            )
            .unwrap();
    }

    #[test]
    fn node_and_reference_reservations_reject_stale_or_over_budget_commits() {
        let mut state = empty_state(GRAPH_LIMITS, true);
        let first = state.reserve_node().unwrap();
        let stale_node = state.reserve_node().unwrap();
        let node = state
            .install_ready_node(
                first,
                WireNode::Array {
                    elements: Box::default(),
                },
            )
            .unwrap();
        assert_eq!(
            state.install_ready_node(
                stale_node,
                WireNode::Array {
                    elements: Box::default(),
                }
            ),
            Err(DecodeError::InvalidNodeState { node })
        );

        let first_alias = state.reserve_reference_entry().unwrap();
        let stale_alias = state.reserve_reference_entry().unwrap();
        state.append_reference_entry(node, first_alias).unwrap();
        assert_eq!(
            state.append_reference_entry(node, stale_alias),
            Err(DecodeError::InvalidNodeState { node })
        );

        let mut crossed = empty_state(GRAPH_LIMITS, true);
        let existing = crossed.reserve_node().unwrap();
        let existing = crossed
            .install_ready_node(
                existing,
                WireNode::Array {
                    elements: Box::default(),
                },
            )
            .unwrap();
        let stale_after_alias = crossed.reserve_node().unwrap();
        crossed.append_reference_alias(existing).unwrap();
        let pending = NodeId::from_zero_based(1);
        assert_eq!(
            crossed.install_pending_node(stale_after_alias, PendingNodeKind::Array),
            Err(DecodeError::InvalidNodeState { node: pending })
        );

        let one_reference = GraphLimits::new(4, 1, 4, 4, 4, 4, 4, 4, 4);
        let mut bounded = empty_state(one_reference, true);
        let reservation = bounded.reserve_node().unwrap();
        let bounded_node = bounded
            .install_pending_node(reservation, PendingNodeKind::Array)
            .unwrap();
        assert!(matches!(
            bounded.reserve_reference_entry(),
            Err(GraphError::ResourceLimit {
                kind: GraphResourceKind::ObjectReferences,
                requested: 2,
                limit: 1,
            })
        ));
        assert_eq!(bounded.ref_table.as_slice(), &[bounded_node]);

        let no_references = GraphLimits::new(4, 0, 4, 4, 4, 4, 4, 4, 4);
        let mut disabled = empty_state(no_references, false);
        let reservation = disabled.reserve_node().unwrap();
        let ready = disabled
            .install_ready_node(
                reservation,
                WireNode::Array {
                    elements: Box::default(),
                },
            )
            .unwrap();
        disabled.append_reference_alias(ready).unwrap();
        assert!(disabled.ref_table.is_empty());

        let invalid = NodeId::from_zero_based(1);
        assert_eq!(
            disabled.append_reference_alias(invalid),
            Err(DecodeError::Graph(GraphError::InvalidNodeIndex {
                index: 1,
                node_count: 1,
            }))
        );
    }

    #[test]
    fn pending_array_identities_support_self_and_descendant_cycles() {
        let self_cycle = decode(&[5, 0, 9, 1, 19, 0], ReaderMode::Strict, true).unwrap();
        let root = NodeId::from_zero_based(0);
        assert_eq!(self_cycle.ref_table.as_ref(), &[root]);
        assert_eq!(
            self_cycle.nodes.as_ref(),
            &[WireNode::Array {
                elements: Box::from([WireValue::Node(root)]),
            }]
        );

        let descendant_cycle =
            decode(&[5, 0, 9, 1, 9, 1, 19, 0], ReaderMode::Strict, true).unwrap();
        let child = NodeId::from_zero_based(1);
        assert_eq!(descendant_cycle.ref_table.as_ref(), &[root, child]);
        assert_eq!(
            descendant_cycle.nodes.as_ref(),
            &[
                WireNode::Array {
                    elements: Box::from([WireValue::Node(child)]),
                },
                WireNode::Array {
                    elements: Box::from([WireValue::Node(root)]),
                },
            ]
        );

        // A compatible ignored property still completes its Pending child;
        // the next property recovers that identity through ref 1.
        let recovered = decode(
            &[5, 1, 2, b'x', 8, 2, 0, 9, 1, 5, 0x54, 2, 19, 1],
            ReaderMode::QuickJsCompatible,
            true,
        )
        .unwrap();
        assert_eq!(recovered.ref_table.as_ref(), &[root, child]);
        assert_eq!(
            recovered.nodes.as_ref(),
            &[
                WireNode::Ordinary {
                    properties: Box::from([WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Node(child),
                    }]),
                },
                WireNode::Array {
                    elements: Box::from([WireValue::Int32(42)]),
                },
            ]
        );
    }

    #[test]
    fn decodes_the_exact_quickjs_object_vector() {
        // Pinned qjs bjson.write({ x: 1 }).
        let bytes = [0x05, 0x01, 0x02, b'x', 0x08, 0x01, 0x02, 0x05, 0x02];
        let graph = decode(&bytes, ReaderMode::Strict, false).unwrap();

        assert_eq!(
            graph.atoms.as_ref(),
            &[WireString::Narrow(Vec::from([b'x']).into_boxed_slice())]
        );
        assert!(graph.ref_table.is_empty());
        assert_eq!(graph.root, WireValue::Node(NodeId::from_zero_based(0)));
        assert_eq!(
            graph.nodes.as_ref(),
            &[WireNode::Ordinary {
                properties: Vec::from([WireProperty {
                    key: WireKey::Atom(AtomId::from_zero_based(0)),
                    value: WireValue::Int32(1),
                }])
                .into_boxed_slice(),
            }]
        );
    }

    #[test]
    fn header_numeric_atoms_follow_js_new_atom_str_boundaries() {
        // Pinned JS_ReadObject accepts these two header strings as property
        // keys. JS_NewAtomStr turns 2147483647 into a tagged integer, while
        // 2147483648 remains an interned string atom.
        let bytes = [
            0x05, 0x02, 0x14, b'2', b'1', b'4', b'7', b'4', b'8', b'3', b'6', b'4', b'7', 0x14,
            b'2', b'1', b'4', b'7', b'4', b'8', b'3', b'6', b'4', b'8', 0x08, 0x02, 0x02, 0x05,
            0x02, 0x04, 0x05, 0x04,
        ];
        let graph = decode(&bytes, ReaderMode::Strict, false).unwrap();

        assert_eq!(
            graph.atoms.as_ref(),
            &[WireString::Narrow(
                Vec::from(*b"2147483648").into_boxed_slice()
            )]
        );
        assert_eq!(
            graph.nodes.as_ref(),
            &[WireNode::Ordinary {
                properties: Vec::from([
                    WireProperty {
                        key: WireKey::Index(0x7fff_ffff),
                        value: WireValue::Int32(1),
                    },
                    WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Int32(2),
                    },
                ])
                .into_boxed_slice(),
            }]
        );
    }

    #[test]
    fn duplicate_narrow_and_wide_atoms_keep_first_slot_and_last_value() {
        // Pinned qjs reads this hand-authored vector as { x: 2 } and rewrites
        // it as: 05 01 02 78 08 01 02 05 04. JS_NewAtomStr interns the narrow
        // and wide spellings to the same atom before JS_ReadObjectTag defines
        // the two properties.
        let bytes = [
            0x05, 0x02, 0x02, b'x', 0x03, b'x', 0x00, 0x08, 0x02, 0x02, 0x05, 0x02, 0x04, 0x05,
            0x04,
        ];
        let graph = decode(&bytes, ReaderMode::Strict, false).unwrap();

        assert_eq!(
            graph.atoms.as_ref(),
            &[WireString::Narrow(Vec::from([b'x']).into_boxed_slice())]
        );
        assert_eq!(
            graph.nodes.as_ref(),
            &[WireNode::Ordinary {
                properties: Vec::from([WireProperty {
                    key: WireKey::Atom(AtomId::from_zero_based(0)),
                    value: WireValue::Int32(2),
                }])
                .into_boxed_slice(),
            }]
        );
    }

    #[test]
    fn header_zero_atom_is_strictly_rejected_and_compatibly_ignored() {
        // Pinned qjs consumes the value but exposes no own property, then
        // rewrites this vector as: 05 00 08 00.
        let bytes = [0x05, 0x00, 0x08, 0x01, 0x00, 0x05, 0x54];
        assert_eq!(
            decode(&bytes, ReaderMode::Strict, false),
            Err(DecodeError::NullPropertyKey { offset: 4 })
        );

        let graph = decode(&bytes, ReaderMode::QuickJsCompatible, false).unwrap();
        assert_eq!(
            graph.nodes.as_ref(),
            &[WireNode::Ordinary {
                properties: Box::default(),
            }]
        );
    }

    #[test]
    fn ignored_and_kept_nested_empty_values_attach_to_the_right_parent() {
        // Pinned qjs consumes the first empty object under atom zero, retains
        // the following x:[] property, and rewrites the reachable value as:
        // 05 01 02 78 08 01 02 09 00.
        let bytes = [
            0x05, 0x01, 0x02, b'x', 0x08, 0x02, 0x00, 0x08, 0x00, 0x02, 0x09, 0x00,
        ];
        let graph = decode(&bytes, ReaderMode::QuickJsCompatible, false).unwrap();

        assert_eq!(graph.root, WireValue::Node(NodeId::from_zero_based(0)));
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Ordinary {
                    properties: Vec::from([WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Node(NodeId::from_zero_based(2)),
                    }])
                    .into_boxed_slice(),
                },
                WireNode::Ordinary {
                    properties: Box::default(),
                },
                WireNode::Array {
                    elements: Box::default(),
                },
            ]
        );
    }

    #[test]
    fn nested_container_values_keep_ignored_and_duplicate_property_routing() {
        // The ignored array is fully decoded, then x receives the following
        // primitive. This freezes the routing needed by deferred completion.
        let ignored = [5, 1, 2, b'x', 8, 2, 0, 9, 1, 5, 0x54, 2, 5, 0x0e];
        let graph = decode(&ignored, ReaderMode::QuickJsCompatible, false).unwrap();
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Ordinary {
                    properties: Box::from([WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Int32(7),
                    }]),
                },
                WireNode::Array {
                    elements: Box::from([WireValue::Int32(42)]),
                },
            ]
        );

        // Duplicate x retains its first property slot but the second completed
        // container supplies the value.
        let duplicate = [5, 1, 2, b'x', 8, 2, 2, 9, 1, 5, 2, 2, 8, 0];
        let graph = decode(&duplicate, ReaderMode::Strict, false).unwrap();
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Ordinary {
                    properties: Box::from([WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Node(NodeId::from_zero_based(2)),
                    }]),
                },
                WireNode::Array {
                    elements: Box::from([WireValue::Int32(1)]),
                },
                WireNode::Ordinary {
                    properties: Box::default(),
                },
            ]
        );
    }

    #[test]
    fn nested_cycles_and_ignored_buffers_keep_preorder_reference_identity() {
        // Pinned QuickJS graph: o.a = [o].
        let cycle = [5, 1, 2, b'a', 8, 1, 2, 9, 1, 19, 0];
        let graph = decode(&cycle, ReaderMode::Strict, true).unwrap();
        let root = NodeId::from_zero_based(0);
        let array = NodeId::from_zero_based(1);
        assert_eq!(graph.ref_table.as_ref(), &[root, array]);
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Ordinary {
                    properties: Box::from([WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Node(array),
                    }]),
                },
                WireNode::Array {
                    elements: Box::from([WireValue::Node(root)]),
                },
            ]
        );

        // A compatible null key discards the first attachment, but its buffer
        // remains ref 1 and a later property can reach it through that history.
        let recovered = [
            5, 1, 2, b'x', 8, 2, 0, 15, 1, 0xff, 0xff, 0xff, 0xff, 0x0f, 0xaa, 2, 19, 1,
        ];
        let graph = decode(&recovered, ReaderMode::QuickJsCompatible, true).unwrap();
        let buffer = NodeId::from_zero_based(1);
        assert_eq!(graph.ref_table.as_ref(), &[root, buffer]);
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Ordinary {
                    properties: Box::from([WireProperty {
                        key: WireKey::Atom(AtomId::from_zero_based(0)),
                        value: WireValue::Node(buffer),
                    }]),
                },
                WireNode::ArrayBuffer {
                    bytes: Box::from([0xaa]),
                    max_byte_length: None,
                },
            ]
        );
    }

    #[test]
    fn empty_frames_drain_in_order_and_nested_key_truncation_still_wins() {
        let graph = decode(&[5, 0, 9, 2, 9, 0, 8, 0], ReaderMode::Strict, true).unwrap();
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Array {
                    elements: Box::from([
                        WireValue::Node(NodeId::from_zero_based(1)),
                        WireValue::Node(NodeId::from_zero_based(2)),
                    ]),
                },
                WireNode::Array {
                    elements: Box::default(),
                },
                WireNode::Ordinary {
                    properties: Box::default(),
                },
            ]
        );

        assert_eq!(
            decode(&[5, 0, 9, 1, 8, 1], ReaderMode::Strict, false),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 6,
                needed: 1,
                remaining: 0,
            }))
        );
    }

    #[test]
    fn decodes_the_exact_quickjs_primitive_array_vector() {
        // Pinned qjs bjson.write([
        //   undefined, null, false, true, -1, 1.5, "abc", 257n
        // ]).
        let bytes = [
            0x05, 0x00, 0x09, 0x08, 0x02, 0x01, 0x03, 0x04, 0x05, 0x01, 0x06, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0xf8, 0x3f, 0x07, 0x06, b'a', b'b', b'c', 0x0a, 0x02, 0x01, 0x01,
        ];
        let graph = decode(&bytes, ReaderMode::Strict, false).unwrap();
        assert_eq!(
            graph.nodes.as_ref(),
            &[WireNode::Array {
                elements: Vec::from([
                    WireValue::Undefined,
                    WireValue::Null,
                    WireValue::Bool(false),
                    WireValue::Bool(true),
                    WireValue::Int32(-1),
                    WireValue::Float64Bits(1.5_f64.to_bits()),
                    WireValue::String(WireString::Narrow(Vec::from(*b"abc").into_boxed_slice(),)),
                    WireValue::BigInt(Vec::from([0x01, 0x01]).into_boxed_slice()),
                ])
                .into_boxed_slice(),
            }]
        );
    }

    #[test]
    fn reference_table_preserves_shared_identity_and_preorder() {
        // Pinned qjs bjson.write([o, o], true).
        let bytes = [0x05, 0x00, 0x09, 0x02, 0x08, 0x00, 0x13, 0x01];
        let graph = decode(&bytes, ReaderMode::Strict, true).unwrap();
        let outer = NodeId::from_zero_based(0);
        let shared = NodeId::from_zero_based(1);

        assert_eq!(graph.root, WireValue::Node(outer));
        assert_eq!(graph.ref_table.as_ref(), &[outer, shared]);
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Array {
                    elements: Vec::from([WireValue::Node(shared), WireValue::Node(shared)])
                        .into_boxed_slice(),
                },
                WireNode::Ordinary {
                    properties: Box::default(),
                },
            ]
        );
    }

    #[test]
    fn reference_table_allows_the_exact_quickjs_cycle_vector() {
        // Pinned qjs bjson.write(o, true), where o.self = o.
        let bytes = [
            0x05, 0x01, 0x08, b's', b'e', b'l', b'f', 0x08, 0x01, 0x02, 0x13, 0x00,
        ];
        let graph = decode(&bytes, ReaderMode::Strict, true).unwrap();
        let root = NodeId::from_zero_based(0);

        assert_eq!(graph.root, WireValue::Node(root));
        assert_eq!(graph.ref_table.as_ref(), &[root]);
        assert_eq!(
            graph.nodes.as_ref(),
            &[WireNode::Ordinary {
                properties: Vec::from([WireProperty {
                    key: WireKey::Atom(AtomId::from_zero_based(0)),
                    value: WireValue::Node(root),
                }])
                .into_boxed_slice(),
            }]
        );
    }

    #[test]
    fn object_reference_errors_match_the_flag_and_table_boundary() {
        let bytes = [0x05, 0x00, 0x13, 0x00];
        assert_eq!(
            decode(&bytes, ReaderMode::Strict, false),
            Err(DecodeError::ObjectReferencesNotAllowed { offset: 2 })
        );
        assert_eq!(
            decode(&bytes, ReaderMode::Strict, true),
            Err(DecodeError::Graph(GraphError::InvalidReferenceIndex {
                index: 0,
                reference_count: 0,
            }))
        );

        // The root Array is already ref 0 while Pending, but ref 1 remains a
        // forward reference and must report the live table boundary.
        assert_eq!(
            decode(&[5, 0, 9, 1, 19, 1], ReaderMode::Strict, true),
            Err(DecodeError::Graph(GraphError::InvalidReferenceIndex {
                index: 1,
                reference_count: 1,
            }))
        );
    }

    #[test]
    fn top_level_finish_is_mode_dependent_but_never_bypassed() {
        let bytes = [0x05, 0x00, 0x01, 0xaa];
        assert_eq!(
            decode(&bytes, ReaderMode::Strict, false),
            Err(DecodeError::Wire(WireError::TrailingBytes {
                offset: 3,
                remaining: 1,
            }))
        );
        assert_eq!(
            decode(&bytes, ReaderMode::QuickJsCompatible, false)
                .unwrap()
                .root,
            WireValue::Null
        );
    }

    #[test]
    fn bigint_canonical_policy_comes_only_from_reader_mode() {
        let redundant_zero = [0x05, 0x00, 0x0a, 0x01, 0x00];
        assert_eq!(
            decode(&redundant_zero, ReaderMode::Strict, false),
            Err(DecodeError::NonCanonicalBigInt { offset: 3 })
        );
        assert_eq!(
            decode(&redundant_zero, ReaderMode::QuickJsCompatible, false)
                .unwrap()
                .root,
            WireValue::BigInt(Box::default())
        );

        let redundant_negative = [0x05, 0x00, 0x0a, 0x02, 0xff, 0xff];
        assert_eq!(
            decode(&redundant_negative, ReaderMode::QuickJsCompatible, false)
                .unwrap()
                .root,
            WireValue::BigInt(Vec::from([0xff]).into_boxed_slice())
        );
    }

    #[test]
    fn exact_quickjs_array_buffer_vectors_preserve_fixed_and_resizable_state() {
        for (bytes, expected_maximum) in [
            (
                &[5, 0, 15, 4, 0xff, 0xff, 0xff, 0xff, 0x0f, 1, 2, 3, 4][..],
                None,
            ),
            (&[5, 0, 15, 4, 4, 1, 2, 3, 4][..], Some(4)),
            (&[5, 0, 15, 4, 8, 1, 2, 3, 4][..], Some(8)),
        ] {
            let graph = decode(bytes, ReaderMode::Strict, true).unwrap();
            let root = NodeId::from_zero_based(0);
            assert_eq!(graph.root, WireValue::Node(root));
            assert_eq!(graph.ref_table.as_ref(), &[root]);
            assert_eq!(
                graph.nodes.as_ref(),
                &[WireNode::ArrayBuffer {
                    bytes: Box::from([1, 2, 3, 4]),
                    max_byte_length: expected_maximum,
                }]
            );
        }

        for (bytes, expected_maximum) in [
            (&[5, 0, 15, 0, 0][..], 0),
            (
                &[5, 0, 15, 0, 0xff, 0xff, 0xff, 0xff, 0x07][..],
                MAX_ARRAY_BUFFER_BYTE_LENGTH,
            ),
        ] {
            let graph = decode(bytes, ReaderMode::Strict, false).unwrap();
            assert_eq!(
                graph.nodes.as_ref(),
                &[WireNode::ArrayBuffer {
                    bytes: Box::default(),
                    max_byte_length: Some(expected_maximum),
                }]
            );
        }
    }

    #[test]
    fn array_buffer_reference_registration_matches_quickjs_preorder() {
        // Pinned qjs bjson.write([buffer, buffer], true).
        let bytes = [
            5, 0, 9, 2, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x12, 0x34, 19, 1,
        ];
        let graph = decode(&bytes, ReaderMode::Strict, true).unwrap();
        let root = NodeId::from_zero_based(0);
        let buffer = NodeId::from_zero_based(1);

        assert_eq!(graph.ref_table.as_ref(), &[root, buffer]);
        assert_eq!(
            graph.nodes.as_ref(),
            &[
                WireNode::Array {
                    elements: Box::from([WireValue::Node(buffer), WireValue::Node(buffer)]),
                },
                WireNode::ArrayBuffer {
                    bytes: Box::from([0x12, 0x34]),
                    max_byte_length: None,
                },
            ]
        );
    }

    #[test]
    fn array_buffer_layout_failures_keep_the_quickjs_reason() {
        assert_eq!(
            decode(&[5, 0, 15, 4, 3], ReaderMode::Strict, false,),
            Err(DecodeError::InvalidArrayBuffer {
                offset: 3,
                reason: ArrayBufferLayoutError::MaximumTooSmall {
                    byte_length: 4,
                    max_byte_length: 3,
                },
            })
        );

        assert_eq!(
            decode(
                &[5, 0, 15, 0, 0x80, 0x80, 0x80, 0x80, 0x08],
                ReaderMode::Strict,
                false,
            ),
            Err(DecodeError::InvalidArrayBuffer {
                offset: 3,
                reason: ArrayBufferLayoutError::MaximumTooLarge {
                    max_byte_length: 0x8000_0000,
                    maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
                },
            })
        );

        // Pinned QuickJS checks payload availability before the constructor's
        // finite maximum and current-length upper bounds.
        assert_eq!(
            decode(
                &[5, 0, 15, 1, 0x80, 0x80, 0x80, 0x80, 0x08],
                ReaderMode::Strict,
                false,
            ),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 9,
                needed: 1,
                remaining: 0,
            }))
        );

        let oversized_payload = [
            5, 0, 15, 0x80, 0x80, 0x80, 0x80, 0x08, 0xff, 0xff, 0xff, 0xff, 0x0f,
        ];
        let oracle_order =
            GraphLimits::new(8, 8, 8, 8, 8, 8, 8, u32::MAX as usize, u32::MAX as usize);
        assert_eq!(
            decode_graph(
                &oversized_payload,
                ReaderMode::Strict,
                WIRE_LIMITS,
                oracle_order,
                false,
            ),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 13,
                needed: 0x8000_0000,
                remaining: 0,
            }))
        );

        assert_eq!(
            decode(
                &[5, 0, 15, 0x80, 0x00, 0xff, 0xff, 0xff, 0xff, 0x0f],
                ReaderMode::Strict,
                false,
            ),
            Err(DecodeError::Wire(WireError::NonCanonicalUleb128 {
                offset: 3,
            }))
        );
    }

    #[test]
    fn array_buffer_payload_depth_and_copy_work_are_bounded() {
        let truncated = [5, 0, 15, 4, 0xff, 0xff, 0xff, 0xff, 0x0f, 1, 2, 3];
        assert_eq!(
            decode(&truncated, ReaderMode::Strict, false),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 9,
                needed: 4,
                remaining: 3,
            }))
        );

        let root_buffer = [5, 0, 15, 3, 3, 1, 2, 3];
        let per_buffer = GraphLimits::new(8, 8, 8, 8, 8, 8, 8, 2, 8);
        assert_eq!(
            decode_graph(
                &root_buffer,
                ReaderMode::Strict,
                WIRE_LIMITS,
                per_buffer,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ArrayBufferBytes,
                requested: 3,
                limit: 2,
            }))
        );

        let no_depth = GraphLimits::new(8, 8, 0, 0, 0, 8, 8, 3, 3);
        assert_eq!(
            decode_graph(
                &root_buffer,
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_depth,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::NestingDepth,
                requested: 1,
                limit: 0,
            }))
        );

        let two_buffers = [5, 0, 9, 2, 15, 2, 2, 1, 2, 15, 2, 2, 3, 4];
        let aggregate = GraphLimits::new(8, 8, 8, 8, 8, 8, 8, 2, 3);
        assert_eq!(
            decode_graph(
                &two_buffers,
                ReaderMode::Strict,
                WIRE_LIMITS,
                aggregate,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalArrayBufferBytes,
                requested: 4,
                limit: 3,
            }))
        );
    }

    fn typed_array_vector(kind: TypedArrayKind) -> Vec<u8> {
        let byte_length = kind.element_byte_length();
        let mut bytes = vec![
            5,
            0,
            BcTag::TypedArray.to_byte(),
            kind.to_wire_byte(),
            1,
            0,
            BcTag::ArrayBuffer.to_byte(),
            byte_length,
            0xff,
            0xff,
            0xff,
            0xff,
            0x0f,
        ];
        bytes.resize(bytes.len() + usize::from(byte_length), 0);
        bytes
    }

    #[test]
    fn typed_array_kinds_and_fresh_backings_match_pinned_quickjs() {
        for kind in TypedArrayKind::ALL {
            let bytes = typed_array_vector(kind);
            let graph = decode(&bytes, ReaderMode::Strict, true).unwrap();
            let typed_array = NodeId::from_zero_based(0);
            let buffer = NodeId::from_zero_based(1);
            assert_eq!(graph.root, WireValue::Node(typed_array));
            assert_eq!(graph.ref_table.as_ref(), &[typed_array, buffer]);
            assert_eq!(
                graph.nodes.as_ref(),
                &[
                    WireNode::TypedArray {
                        kind,
                        length: 1,
                        byte_offset: 0,
                        buffer,
                    },
                    WireNode::ArrayBuffer {
                        bytes: vec![0; usize::from(kind.element_byte_length())].into_boxed_slice(),
                        max_byte_length: None,
                    },
                ]
            );

            let without_references = decode(&bytes, ReaderMode::Strict, false).unwrap();
            assert!(without_references.ref_table.is_empty());
            assert_eq!(without_references.nodes, graph.nodes);
        }
    }

    #[test]
    fn typed_array_reference_order_preserves_view_and_buffer_identity() {
        // Pinned qjs bjson.write([buffer, new Uint8Array(buffer, 1, 1)], true).
        let earlier_buffer = [
            5, 0, 9, 2, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x11, 0x22, 14, 2, 1, 1, 19, 1,
        ];
        let graph = decode(&earlier_buffer, ReaderMode::Strict, true).unwrap();
        let root = NodeId::from_zero_based(0);
        let buffer = NodeId::from_zero_based(1);
        let view = NodeId::from_zero_based(2);
        assert_eq!(graph.ref_table.as_ref(), &[root, buffer, view]);
        assert_eq!(
            graph.nodes[view.as_usize()],
            WireNode::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 1,
                byte_offset: 1,
                buffer,
            }
        );

        // Pinned qjs bjson.write([view, view], true): view is ref 1 and its
        // freshly nested backing buffer is ref 2.
        let repeated_view = [
            5, 0, 9, 2, 14, 2, 1, 1, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x11, 0x22, 19, 1,
        ];
        let graph = decode(&repeated_view, ReaderMode::Strict, true).unwrap();
        let view = NodeId::from_zero_based(1);
        let buffer = NodeId::from_zero_based(2);
        assert_eq!(graph.ref_table.as_ref(), &[root, view, buffer]);
        assert_eq!(
            graph.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(view), WireValue::Node(view)]),
            }
        );
    }

    #[test]
    fn typed_array_errors_follow_quickjs_read_and_constructor_order() {
        assert_eq!(
            decode(&[5, 0, 14, 12], ReaderMode::Strict, true),
            Err(DecodeError::InvalidTypedArrayKind {
                offset: 3,
                kind: 12,
            })
        );
        for (bytes, offset) in [
            (&[5, 0, 14][..], 3),
            (&[5, 0, 14, 2][..], 4),
            (&[5, 0, 14, 2, 1][..], 5),
            // Even an unaligned view does not reach constructor validation
            // until its recursively encoded backing value is complete.
            (&[5, 0, 14, 3, 1, 1][..], 6),
        ] {
            assert_eq!(
                decode(bytes, ReaderMode::Strict, true),
                Err(DecodeError::Wire(WireError::Truncated {
                    offset,
                    needed: 1,
                    remaining: 0,
                }))
            );
        }

        // Backing class validation happens before alignment.
        assert_eq!(
            decode(&[5, 0, 14, 3, 1, 1, 1], ReaderMode::Strict, true),
            Err(DecodeError::InvalidTypedArrayBacking {
                offset: 2,
                reason: TypedArrayBackingError::NotObject,
            })
        );
        assert_eq!(
            decode(
                &[5, 0, 14, 3, 1, 1, 15, 0, 0xff, 0xff, 0xff, 0xff, 0x0f],
                ReaderMode::Strict,
                true,
            ),
            Err(DecodeError::InvalidTypedArray {
                offset: 2,
                reason: TypedArrayLayoutError::UnalignedByteOffset {
                    byte_offset: 1,
                    element_byte_length: 2,
                },
            })
        );
        assert_eq!(
            decode(
                &[5, 0, 14, 3, 1, 0, 15, 1, 0xff, 0xff, 0xff, 0xff, 0x0f, 0,],
                ReaderMode::Strict,
                true,
            ),
            Err(DecodeError::InvalidTypedArray {
                offset: 2,
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
    fn typed_array_pending_and_wrong_backings_are_safely_rejected() {
        let root = NodeId::from_zero_based(0);
        // Pinned QuickJS 2026-06-04 exits 139 on this malicious self-backing
        // placeholder. Rust preserves registration order but rejects it safely.
        assert_eq!(
            decode(&[5, 0, 14, 2, 1, 0, 19, 0], ReaderMode::Strict, true),
            Err(DecodeError::InvalidTypedArrayBacking {
                offset: 2,
                reason: TypedArrayBackingError::Pending { node: root },
            })
        );

        // Pinned qjs reports `TypeError: ArrayBuffer object expected` when a
        // view points at a still-open Array or Ordinary ancestor. Those
        // containers already have their real class; only a TypedArray's
        // temporary NULL identity needs the safe Pending diagnostic above.
        for (bytes, offset) in [
            (&[5, 0, 9, 1, 14, 2, 0, 0, 19, 0][..], 4),
            (&[5, 0, 8, 1, 1, 14, 2, 0, 0, 19, 0][..], 5),
        ] {
            assert_eq!(
                decode(bytes, ReaderMode::Strict, true),
                Err(DecodeError::InvalidTypedArrayBacking {
                    offset,
                    reason: TypedArrayBackingError::NotArrayBuffer { node: root },
                })
            );
        }

        let ordinary = NodeId::from_zero_based(1);
        assert_eq!(
            decode(
                &[5, 0, 9, 2, 8, 0, 14, 2, 0, 0, 19, 1],
                ReaderMode::Strict,
                true,
            ),
            Err(DecodeError::InvalidTypedArrayBacking {
                offset: 6,
                reason: TypedArrayBackingError::NotArrayBuffer { node: ordinary },
            })
        );

        assert_eq!(
            decode(&[5, 0, 14, 2, 0, 0, 19], ReaderMode::Strict, false),
            Err(DecodeError::ObjectReferencesNotAllowed { offset: 6 })
        );
        assert_eq!(
            decode(&[5, 0, 14, 2, 0, 0, 19, 1], ReaderMode::Strict, true),
            Err(DecodeError::Graph(GraphError::InvalidReferenceIndex {
                index: 1,
                reference_count: 1,
            }))
        );

        assert_eq!(
            decode(&[5, 0, 14, 2, 0, 0, 16], ReaderMode::Strict, true),
            Err(DecodeError::UnsupportedTag {
                tag: BcTag::SharedArrayBuffer,
                offset: 6,
            })
        );
    }

    #[test]
    fn typed_array_identity_and_backing_depth_are_bounded_independently() {
        let bytes = typed_array_vector(TypedArrayKind::Uint8);
        for (limits, expected) in [
            (
                GraphLimits::new(1, 8, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::Nodes,
                    requested: 2,
                    limit: 1,
                },
            ),
            (
                GraphLimits::new(8, 1, 8, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::ObjectReferences,
                    requested: 2,
                    limit: 1,
                },
            ),
            (
                GraphLimits::new(8, 8, 1, 8, 8, 8, 8, 8, 8),
                GraphError::ResourceLimit {
                    kind: GraphResourceKind::NestingDepth,
                    requested: 2,
                    limit: 1,
                },
            ),
        ] {
            assert_eq!(
                decode_graph(&bytes, ReaderMode::Strict, WIRE_LIMITS, limits, true),
                Err(DecodeError::Graph(expected))
            );
        }
    }

    #[test]
    fn unsupported_data_tags_are_rejected_before_their_payloads() {
        for tag in [
            BcTag::TemplateObject,
            BcTag::FunctionBytecode,
            BcTag::Module,
            BcTag::SharedArrayBuffer,
            BcTag::Date,
            BcTag::ObjectValue,
        ] {
            assert_eq!(
                decode(&[0x05, 0x00, tag.to_byte()], ReaderMode::Strict, false),
                Err(DecodeError::UnsupportedTag { tag, offset: 2 })
            );
        }
    }

    #[test]
    fn nesting_and_aggregate_container_work_are_bounded() {
        let shallow = GraphLimits::new(8, 8, 1, 8, 8, 8, 8, 0, 0);
        let nested = [0x05, 0x00, 0x09, 0x01, 0x09, 0x01, 0x01];
        assert_eq!(
            decode_graph(&nested, ReaderMode::Strict, WIRE_LIMITS, shallow, false,),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::NestingDepth,
                requested: 2,
                limit: 1,
            }))
        );

        let aggregate = GraphLimits::new(8, 8, 8, 8, 1, 8, 8, 0, 0);
        assert_eq!(
            decode_graph(&nested, ReaderMode::Strict, WIRE_LIMITS, aggregate, false,),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalContainerEntries,
                requested: 2,
                limit: 1,
            }))
        );
    }
}
