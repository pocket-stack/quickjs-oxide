//! Heap-independent data model for decoded BC5 object graphs.
//!
//! Nodes live in a flat arena so object identity and cycles never require a
//! recursive Rust type. The QuickJS object-reference table is deliberately a
//! separate vector: compatible input can append more than one reference-table
//! entry for the same node (notably through `OBJECT_VALUE`).

use std::fmt;
use std::hash::{Hash, Hasher};

use super::super::wire::WireString;

/// Zero-based index into [`WireGraph::nodes`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct NodeId(pub(in crate::runtime) u32);

impl NodeId {
    #[must_use]
    pub(in crate::runtime) const fn from_zero_based(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub(in crate::runtime) const fn zero_based(self) -> u32 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime) const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// Zero-based index into [`WireGraph::atoms`].
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct AtomId(pub(in crate::runtime) u32);

impl AtomId {
    #[must_use]
    pub(in crate::runtime) const fn from_zero_based(index: u32) -> Self {
        Self(index)
    }

    #[must_use]
    pub(in crate::runtime) const fn zero_based(self) -> u32 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime) const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// A string own-property key accepted by QuickJS's data-object writer.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(in crate::runtime) enum WireKey {
    /// A QuickJS tagged-integer atom, represented by its property index.
    Index(u32),
    /// An entry in the graph's semantic atom table, using a zero-based index.
    /// Header duplicates and tagged decimal spellings are already normalized.
    Atom(AtomId),
}

/// Return the tagged-integer atom represented by a canonical decimal string.
///
/// This is the data-only equivalent of pinned QuickJS's
/// `is_num_string` + `JS_NewAtomStr` path. Narrow and wide strings are both
/// interpreted as sequences of UTF-16 code units; only ASCII decimal digits
/// participate. `JS_ATOM_MAX_INT` is `0x7fff_ffff`.
#[must_use]
pub(in crate::runtime) fn numeric_atom_index(value: &WireString) -> Option<u32> {
    let length = value.len();
    if length == 0 || length > 10 {
        return None;
    }

    let first = atom_code_unit(value, 0);
    if first == u16::from(b'0') {
        return (length == 1).then_some(0);
    }
    if !(u16::from(b'1')..=u16::from(b'9')).contains(&first) {
        return None;
    }

    let mut number = u64::from(first - u16::from(b'0'));
    for index in 1..length {
        let unit = atom_code_unit(value, index);
        if !(u16::from(b'0')..=u16::from(b'9')).contains(&unit) {
            return None;
        }
        number = number * 10 + u64::from(unit - u16::from(b'0'));
        if number > u64::from(i32::MAX as u32) {
            return None;
        }
    }
    Some(number as u32)
}

/// Compare atom strings by JavaScript string code units, independent of their
/// narrow or wide BC5 storage representation.
#[must_use]
pub(in crate::runtime) fn semantic_atom_eq(left: &WireString, right: &WireString) -> bool {
    left.len() == right.len()
        && (0..left.len()).all(|index| atom_code_unit(left, index) == atom_code_unit(right, index))
}

/// Hash an atom with the same width-independent equality used by
/// [`semantic_atom_eq`].
pub(in crate::runtime) fn semantic_atom_hash<H: Hasher>(value: &WireString, state: &mut H) {
    value.len().hash(state);
    for index in 0..value.len() {
        atom_code_unit(value, index).hash(state);
    }
}

fn atom_code_unit(value: &WireString, index: usize) -> u16 {
    match value {
        WireString::Narrow(bytes) => u16::from(bytes[index]),
        WireString::Wide(units) => units[index],
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct WireProperty {
    pub(in crate::runtime) key: WireKey,
    pub(in crate::runtime) value: WireValue,
}

/// A heap-independent JavaScript value held by a wire graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum WireValue {
    Undefined,
    Null,
    Bool(bool),
    Int32(i32),
    /// Exact IEEE-754 payload, preserving `-0` and NaN payload bits.
    Float64Bits(u64),
    String(WireString),
    /// Canonical signed little-endian two's-complement bytes.
    ///
    /// Zero is the empty slice. Non-zero values have no redundant high-order
    /// sign-extension byte. A compatible decoder must normalize accepted
    /// non-minimal input before constructing this variant.
    BigInt(Box<[u8]>),
    Node(NodeId),
}

/// The first admitted data-only BC5 object kinds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum WireNode {
    /// Enumerable data properties in insertion-slot order. Semantic keys are
    /// unique; decoding a later duplicate replaces the first slot's value.
    Ordinary { properties: Box<[WireProperty]> },
    /// Arrays are dense in the semantic graph: the writer normalizes each
    /// source hole to `undefined`, and the reader creates an own property for it.
    Array { elements: Box<[WireValue]> },
}

/// One complete, validated, heap-independent object graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct WireGraph {
    /// Header atoms interned by UTF-16 code units. This can retain unused atoms
    /// from a compatible read; the canonical writer emits only reachable keys
    /// and rebuilds their indices in depth-first encounter order.
    pub(in crate::runtime) atoms: Box<[WireString]>,
    pub(in crate::runtime) nodes: Box<[WireNode]>,
    /// QuickJS object-reference entries in encounter order.
    ///
    /// This is not interchangeable with `nodes`: multiple entries may point to
    /// the same node. The canonical writer treats it as read history and builds
    /// a fresh table from the reachable graph instead of scanning or reusing it.
    pub(in crate::runtime) ref_table: Box<[NodeId]>,
    pub(in crate::runtime) root: WireValue,
}

/// Graph allocations and traversal work controlled independently of framing
/// and string limits in the wire layer.
///
/// There is intentionally no `Default`; each eventual caller must select a
/// policy appropriate for its trust boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct GraphLimits {
    max_nodes: usize,
    max_object_references: usize,
    max_nesting_depth: usize,
    max_container_entries: usize,
    max_total_container_entries: usize,
    max_bigint_bytes: usize,
    max_total_bigint_bytes: usize,
    max_array_buffer_bytes: usize,
    max_total_array_buffer_bytes: usize,
}

impl GraphLimits {
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub(in crate::runtime) const fn new(
        max_nodes: usize,
        max_object_references: usize,
        max_nesting_depth: usize,
        max_container_entries: usize,
        max_total_container_entries: usize,
        max_bigint_bytes: usize,
        max_total_bigint_bytes: usize,
        max_array_buffer_bytes: usize,
        max_total_array_buffer_bytes: usize,
    ) -> Self {
        Self {
            max_nodes,
            max_object_references,
            max_nesting_depth,
            max_container_entries,
            max_total_container_entries,
            max_bigint_bytes,
            max_total_bigint_bytes,
            max_array_buffer_bytes,
            max_total_array_buffer_bytes,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn limit(self, kind: GraphResourceKind) -> usize {
        match kind {
            GraphResourceKind::Nodes => self.max_nodes,
            GraphResourceKind::ObjectReferences => self.max_object_references,
            GraphResourceKind::NestingDepth => self.max_nesting_depth,
            GraphResourceKind::ContainerEntries => self.max_container_entries,
            GraphResourceKind::TotalContainerEntries => self.max_total_container_entries,
            GraphResourceKind::BigIntBytes => self.max_bigint_bytes,
            GraphResourceKind::TotalBigIntBytes => self.max_total_bigint_bytes,
            GraphResourceKind::ArrayBufferBytes => self.max_array_buffer_bytes,
            GraphResourceKind::TotalArrayBufferBytes => self.max_total_array_buffer_bytes,
        }
    }

    pub(in crate::runtime) fn check(
        self,
        kind: GraphResourceKind,
        requested: usize,
    ) -> Result<(), GraphError> {
        let limit = self.limit(kind);
        if requested > limit {
            return Err(GraphError::ResourceLimit {
                kind,
                requested,
                limit,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum GraphResourceKind {
    Nodes,
    ObjectReferences,
    NestingDepth,
    ContainerEntries,
    TotalContainerEntries,
    BigIntBytes,
    TotalBigIntBytes,
    /// Reserved now so admitting `ARRAY_BUFFER` does not weaken the limit API.
    ArrayBufferBytes,
    /// Reserved now so aggregate backing-store bytes remain independently
    /// bounded when `ARRAY_BUFFER` is admitted.
    TotalArrayBufferBytes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum GraphError {
    ResourceLimit {
        kind: GraphResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        kind: GraphResourceKind,
    },
    InvalidAtomIndex {
        index: u32,
        atom_count: usize,
    },
    InvalidNodeIndex {
        index: u32,
        node_count: usize,
    },
    InvalidReferenceIndex {
        index: u32,
        reference_count: usize,
    },
    AllocationFailed,
}

impl fmt::Display for GraphError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimit {
                kind,
                requested,
                limit,
            } => write!(
                formatter,
                "{kind:?} graph resource limit exceeded: requested {requested}, limit {limit}"
            ),
            Self::CountOverflow { kind } => {
                write!(formatter, "{kind:?} graph resource count overflowed")
            }
            Self::InvalidAtomIndex { index, atom_count } => write!(
                formatter,
                "invalid zero-based atom index {index} for {atom_count} graph atoms"
            ),
            Self::InvalidNodeIndex { index, node_count } => write!(
                formatter,
                "invalid zero-based node index {index} for {node_count} graph nodes"
            ),
            Self::InvalidReferenceIndex {
                index,
                reference_count,
            } => write!(
                formatter,
                "invalid object-reference index {index} for {reference_count} graph references"
            ),
            Self::AllocationFailed => formatter.write_str("wire graph allocation failed"),
        }
    }
}

impl std::error::Error for GraphError {}

/// Length of the canonical signed little-endian two's-complement prefix.
///
/// Redundant sign extension lives at the high-order end of the payload, so a
/// compatible decoder can normalize without constructing a BigInt value.
#[must_use]
pub(in crate::runtime) fn canonical_bigint_length(payload: &[u8]) -> usize {
    let mut length = payload.len();
    while length > 1 {
        let high = payload[length - 1];
        let next = payload[length - 2];
        let redundant_positive = high == 0x00 && next & 0x80 == 0;
        let redundant_negative = high == 0xff && next & 0x80 != 0;
        if !redundant_positive && !redundant_negative {
            break;
        }
        length -= 1;
    }
    if length == 1 && payload[0] == 0 {
        0
    } else {
        length
    }
}
