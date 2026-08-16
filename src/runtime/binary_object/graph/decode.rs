//! Bounded, heap-independent decoder for the first BC5 data-object slice.
//!
//! Containers are assembled through an explicit frame stack. Object and Array
//! nodes enter the reference table before any child is read, which preserves
//! QuickJS's preorder reference numbering while keeping cycles out of the Rust
//! type recursion.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use crate::bigint::BC5_BIGINT_READ_MAX_BYTES;

use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits};
use super::model::{
    ArrayBufferLayoutError, AtomId, GraphError, GraphLimits, GraphResourceKind, NodeId, WireGraph,
    WireKey, WireNode, WireProperty, WireValue, canonical_bigint_length, numeric_atom_index,
    semantic_atom_eq, semantic_atom_hash, validate_array_buffer_layout,
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

        let key = match state.frames.last() {
            Some(Frame::Ordinary { .. }) => Some(read_key(&mut cursor, &header_atoms)?),
            Some(Frame::Array { .. }) | None => None,
        };
        let parsed = state.read_value(&mut cursor)?;

        if let Some(parent) = state.frames.last_mut() {
            parent.attach(key, parsed.value)?;
        } else {
            debug_assert!(root.is_none());
            root = Some(parsed.value);
        }

        if let Some(frame) = parsed.frame {
            state
                .frames
                .try_reserve(1)
                .map_err(|_| GraphError::AllocationFailed)?;
            state.frames.push(frame);
        }

        while state.frames.last().is_some_and(Frame::is_complete) {
            let frame = state
                .frames
                .pop()
                .expect("a completed frame was observed before pop");
            frame.finish(&mut state.nodes)?;
        }
    }

    // This call is unconditional: QuickJsCompatible itself decides to accept
    // trailing bytes, rather than the graph layer bypassing finalization.
    cursor.finish()?;

    Ok(WireGraph {
        atoms: atoms.into_boxed_slice(),
        nodes: state.nodes.into_boxed_slice(),
        ref_table: state.ref_table.into_boxed_slice(),
        root: root.expect("one value is required after a valid BC5 header"),
    })
}

struct DecodeState {
    limits: GraphLimits,
    allow_object_references: bool,
    nodes: Vec<WireNode>,
    ref_table: Vec<NodeId>,
    frames: Vec<Frame>,
    total_container_entries: usize,
    total_bigint_bytes: usize,
    total_array_buffer_bytes: usize,
}

impl DecodeState {
    fn read_value(&mut self, cursor: &mut WireCursor<'_>) -> Result<ParsedValue, DecodeError> {
        let tag_offset = cursor.position();
        let tag = cursor.read_tag()?;
        let primitive = match tag {
            BcTag::Null => Some(WireValue::Null),
            BcTag::Undefined => Some(WireValue::Undefined),
            BcTag::BoolFalse => Some(WireValue::Bool(false)),
            BcTag::BoolTrue => Some(WireValue::Bool(true)),
            BcTag::Int32 => Some(WireValue::Int32(cursor.read_i32()?)),
            BcTag::Float64 => Some(WireValue::Float64Bits(cursor.read_f64()?.to_bits())),
            BcTag::String => Some(WireValue::String(cursor.read_string()?)),
            BcTag::BigInt => Some(self.read_bigint(cursor)?),
            BcTag::Object => return self.begin_container(cursor, ContainerKind::Ordinary),
            BcTag::Array => return self.begin_container(cursor, ContainerKind::Array),
            BcTag::ArrayBuffer => Some(self.read_array_buffer(cursor)?),
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
                Some(WireValue::Node(node))
            }
            BcTag::TemplateObject
            | BcTag::FunctionBytecode
            | BcTag::Module
            | BcTag::TypedArray
            | BcTag::SharedArrayBuffer
            | BcTag::Date
            | BcTag::ObjectValue => {
                return Err(DecodeError::UnsupportedTag {
                    tag,
                    offset: tag_offset,
                });
            }
        };

        Ok(ParsedValue {
            value: primitive.expect("every non-container admitted tag produced a value"),
            frame: None,
        })
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
        let node = self.reserve_node()?;

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
        self.install_node(
            node,
            WireNode::ArrayBuffer {
                bytes: bytes.into_boxed_slice(),
                max_byte_length,
            },
        );
        Ok(WireValue::Node(node))
    }

    fn begin_container(
        &mut self,
        cursor: &mut WireCursor<'_>,
        kind: ContainerKind,
    ) -> Result<ParsedValue, DecodeError> {
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

        let placeholder = match kind {
            ContainerKind::Ordinary => WireNode::Ordinary {
                properties: Box::default(),
            },
            ContainerKind::Array => WireNode::Array {
                elements: Box::default(),
            },
        };
        let node_id = self.allocate_node(placeholder)?;
        let frame = Frame::new(kind, node_id, entry_count)?;
        Ok(ParsedValue {
            value: WireValue::Node(node_id),
            frame: Some(frame),
        })
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

    fn allocate_node(&mut self, node: WireNode) -> Result<NodeId, DecodeError> {
        let node_id = self.reserve_node()?;
        self.install_node(node_id, node);
        Ok(node_id)
    }

    fn reserve_node(&mut self) -> Result<NodeId, DecodeError> {
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

        let node_id = NodeId::from_zero_based(raw_index);
        if self.allow_object_references {
            let requested_references =
                self.ref_table
                    .len()
                    .checked_add(1)
                    .ok_or(GraphError::CountOverflow {
                        kind: GraphResourceKind::ObjectReferences,
                    })?;
            self.limits
                .check(GraphResourceKind::ObjectReferences, requested_references)?;
            self.ref_table
                .try_reserve(1)
                .map_err(|_| GraphError::AllocationFailed)?;
        }

        Ok(node_id)
    }

    fn install_node(&mut self, node_id: NodeId, node: WireNode) {
        debug_assert_eq!(node_id.as_usize(), self.nodes.len());
        self.nodes.push(node);
        // Every admitted node is ready when it enters this table. Containers
        // install an empty placeholder before their first child; ArrayBuffer
        // installs its completed leaf after its payload has been copied.
        if self.allow_object_references {
            self.ref_table.push(node_id);
        }
    }
}

#[derive(Clone, Copy)]
enum ContainerKind {
    Ordinary,
    Array,
}

struct ParsedValue {
    value: WireValue,
    frame: Option<Frame>,
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
        }
        Ok(())
    }

    fn is_complete(&self) -> bool {
        match self {
            Self::Ordinary {
                expected, consumed, ..
            } => consumed == expected,
            Self::Array {
                expected, elements, ..
            } => elements.len() == *expected,
        }
    }

    fn finish(self, nodes: &mut [WireNode]) -> Result<(), GraphError> {
        let (node, replacement) = match self {
            Self::Ordinary {
                node, properties, ..
            } => (
                node,
                WireNode::Ordinary {
                    properties: properties.into_boxed_slice(),
                },
            ),
            Self::Array { node, elements, .. } => (
                node,
                WireNode::Array {
                    elements: elements.into_boxed_slice(),
                },
            ),
        };
        let node_count = nodes.len();
        let slot = nodes
            .get_mut(node.as_usize())
            .ok_or(GraphError::InvalidNodeIndex {
                index: node.zero_based(),
                node_count,
            })?;
        *slot = replacement;
        Ok(())
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

    #[test]
    fn unsupported_data_tags_are_rejected_before_their_payloads() {
        for tag in [
            BcTag::TemplateObject,
            BcTag::FunctionBytecode,
            BcTag::Module,
            BcTag::TypedArray,
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
