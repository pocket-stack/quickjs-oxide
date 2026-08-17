//! Heap-independent data model for decoded BC5 object graphs.
//!
//! Nodes live in a flat arena so object identity and cycles never require a
//! recursive Rust type. The QuickJS object-reference table is deliberately a
//! separate vector: compatible input can append more than one reference-table
//! entry for the same node (notably through `OBJECT_VALUE`).

use std::fmt;
use std::hash::{Hash, Hasher};

use super::super::wire::WireString;
pub(in crate::runtime) use super::sab_transport::ArchiveBackingId;

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

/// Zero-based index into a validated semantic string-atom arena.
///
/// Data graphs use this for [`WireGraph::atoms`]; a whole bytecode image uses
/// the same strong index type for its image-local dynamic atom arena.
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
pub(in crate::runtime) struct WirePropertyCarrier<V, K> {
    pub(in crate::runtime) key: K,
    pub(in crate::runtime) value: V,
}

/// One property in the concrete data-object graph.
pub(in crate::runtime) type WireProperty = WirePropertyCarrier<WireValue, WireKey>;

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

/// One primitive payload that pinned QuickJS can store in `JS_CLASS_NUMBER`,
/// `JS_CLASS_STRING`, `JS_CLASS_BOOLEAN`, or `JS_CLASS_BIG_INT`.
///
/// The private field makes null, undefined, symbols, and object identities
/// unrepresentable as a boxed node. In particular, the BC5 reader treats an
/// `OBJECT_VALUE` whose child is already an object as a reference-table alias,
/// not as a second wrapper identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct BoxedPrimitive(WireValue);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BoxedPrimitiveError {
    NullOrUndefined,
    Object,
}

impl fmt::Display for BoxedPrimitiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullOrUndefined => {
                formatter.write_str("null or undefined cannot be converted to an object")
            }
            Self::Object => formatter.write_str(
                "an object payload aliases its existing identity instead of creating a wrapper",
            ),
        }
    }
}

impl BoxedPrimitive {
    pub(in crate::runtime) fn try_from_wire_value(
        value: WireValue,
    ) -> Result<Self, BoxedPrimitiveError> {
        match value {
            value @ (WireValue::Bool(_)
            | WireValue::Int32(_)
            | WireValue::Float64Bits(_)
            | WireValue::String(_)
            | WireValue::BigInt(_)) => Ok(Self(value)),
            WireValue::Null | WireValue::Undefined => Err(BoxedPrimitiveError::NullOrUndefined),
            WireValue::Node(_) => Err(BoxedPrimitiveError::Object),
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn as_wire_value(&self) -> &WireValue {
        &self.0
    }
}

/// One numeric payload stored in a pinned QuickJS `JS_CLASS_DATE` object.
///
/// The reader installs the decoded number without applying `TimeClip`, so the
/// private field preserves both the Int32-versus-Float64 tag choice and every
/// IEEE-754 bit (including `-0`, infinities, and NaN payloads).
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct DateNumber(WireValue);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum DateNumberError {
    NotNumber,
}

impl fmt::Display for DateNumberError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotNumber => formatter.write_str("Number tag expected for date"),
        }
    }
}

impl DateNumber {
    pub(in crate::runtime) fn try_from_wire_value(
        value: WireValue,
    ) -> Result<Self, DateNumberError> {
        match value {
            value @ (WireValue::Int32(_) | WireValue::Float64Bits(_)) => Ok(Self(value)),
            WireValue::Undefined
            | WireValue::Null
            | WireValue::Bool(_)
            | WireValue::String(_)
            | WireValue::BigInt(_)
            | WireValue::Node(_) => Err(DateNumberError::NotNumber),
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn as_wire_value(&self) -> &WireValue {
        &self.0
    }
}

/// Exact `class_id - JS_CLASS_UINT8C_ARRAY` order used by BC5.
///
/// This deliberately remains a wire type rather than depending on the runtime
/// heap's TypedArray enum. The eventual materializer must convert between the
/// two with an exhaustive match.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(in crate::runtime) enum TypedArrayKind {
    Uint8Clamped = 0,
    Int8 = 1,
    Uint8 = 2,
    Int16 = 3,
    Uint16 = 4,
    Int32 = 5,
    Uint32 = 6,
    BigInt64 = 7,
    BigUint64 = 8,
    Float16 = 9,
    Float32 = 10,
    Float64 = 11,
}

impl TypedArrayKind {
    pub(in crate::runtime) const COUNT: usize = 12;
    pub(in crate::runtime) const ALL: [Self; Self::COUNT] = [
        Self::Uint8Clamped,
        Self::Int8,
        Self::Uint8,
        Self::Int16,
        Self::Uint16,
        Self::Int32,
        Self::Uint32,
        Self::BigInt64,
        Self::BigUint64,
        Self::Float16,
        Self::Float32,
        Self::Float64,
    ];

    #[must_use]
    pub(in crate::runtime) const fn from_wire_byte(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Uint8Clamped),
            1 => Some(Self::Int8),
            2 => Some(Self::Uint8),
            3 => Some(Self::Int16),
            4 => Some(Self::Uint16),
            5 => Some(Self::Int32),
            6 => Some(Self::Uint32),
            7 => Some(Self::BigInt64),
            8 => Some(Self::BigUint64),
            9 => Some(Self::Float16),
            10 => Some(Self::Float32),
            11 => Some(Self::Float64),
            _ => None,
        }
    }

    #[must_use]
    pub(in crate::runtime) const fn to_wire_byte(self) -> u8 {
        self as u8
    }

    #[must_use]
    pub(in crate::runtime) const fn element_byte_length(self) -> u8 {
        match self {
            Self::Uint8Clamped | Self::Int8 | Self::Uint8 => 1,
            Self::Int16 | Self::Uint16 | Self::Float16 => 2,
            Self::Int32 | Self::Uint32 | Self::Float32 => 4,
            Self::BigInt64 | Self::BigUint64 | Self::Float64 => 8,
        }
    }
}

/// The first admitted BC5 object kinds.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum WireNodeCarrier<V, K> {
    /// Enumerable data properties in insertion-slot order. Semantic keys are
    /// unique; decoding a later duplicate replaces the first slot's value.
    Ordinary {
        properties: Box<[WirePropertyCarrier<V, K>]>,
    },
    /// Arrays are dense in the semantic graph: the writer normalizes each
    /// source hole to `undefined`, and the reader creates an own property for it.
    Array { elements: Box<[V]> },
    /// A BC5 `TEMPLATE_OBJECT` identity.
    ///
    /// Like ordinary arrays, the indexed elements are dense after wire
    /// normalization. The `raw` child is always present in the wire payload.
    /// Pinned QuickJS consumes `WireValue::Undefined` but then omits the own
    /// `.raw` property; a later heap materializer must preserve that distinction.
    /// The graph deliberately does not reuse the language-level template
    /// materializer: the BC5 reader leaves the Array's `length` writable.
    /// Either child position may refer back to this node because the identity is
    /// registered before its children are traversed.
    TemplateObject { elements: Box<[V]>, raw: V },
    /// An owned ArrayBuffer backing store.
    ///
    /// `None` is the fixed-length `UINT32_MAX` wire sentinel. `Some(max)` is a
    /// resizable buffer even when `max == bytes.len()`; the distinction is
    /// observable through the JavaScript ArrayBuffer API.
    ArrayBuffer {
        bytes: Box<[u8]>,
        max_byte_length: Option<u32>,
    },
    /// One SharedArrayBuffer wrapper whose live bytes remain outside the
    /// archive model.
    ///
    /// The wrapper identity is this graph node. `backing` is a separate,
    /// archive-local identity so distinct wrappers may retain shared-backing
    /// aliasing without storing QuickJS's process-local pointer token. A bare
    /// [`WireGraph`] containing this variant is intentionally incomplete;
    /// only the transport-aware decoder may return it inside its inseparable
    /// archived-graph aggregate.
    SharedArrayBuffer {
        byte_length: u32,
        max_byte_length: Option<u32>,
        backing: ArchiveBackingId,
    },
    /// A fixed-length view over one graph-owned backing buffer identity.
    ///
    /// BC5 does not preserve the length-tracking bit of a view over a resizable
    /// buffer: the writer emits the current element count and the reader always
    /// supplies that count explicitly to the constructor.
    TypedArray {
        kind: TypedArrayKind,
        length: u32,
        byte_offset: u32,
        buffer: NodeId,
    },
    /// One genuine primitive wrapper identity.
    ///
    /// Reader-only `OBJECT_VALUE(object)` inputs canonicalize to the existing
    /// object node and append a reference-table alias instead of constructing
    /// this variant.
    ObjectValue { primitive: BoxedPrimitive },
    /// A Date identity with the exact number representation accepted by the
    /// BC5 reader. Enumerable own properties are not part of this wire class.
    Date { time_value: DateNumber },
}

/// One node in the concrete data-object graph.
pub(in crate::runtime) type WireNode = WireNodeCarrier<WireValue, WireKey>;

/// Pinned QuickJS currently rejects ArrayBuffer lengths above 2 GiB - 1.
pub(in crate::runtime) const MAX_ARRAY_BUFFER_BYTE_LENGTH: u32 = i32::MAX as u32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ArrayBufferLayoutError {
    ByteLengthTooLarge {
        byte_length: usize,
        maximum: u32,
    },
    MaximumTooSmall {
        byte_length: u32,
        max_byte_length: u32,
    },
    MaximumTooLarge {
        max_byte_length: u32,
        maximum: u32,
    },
}

impl fmt::Display for ArrayBufferLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ByteLengthTooLarge {
                byte_length,
                maximum,
            } => write!(
                formatter,
                "byte length {byte_length} exceeds pinned QuickJS maximum {maximum}"
            ),
            Self::MaximumTooSmall {
                byte_length,
                max_byte_length,
            } => write!(
                formatter,
                "max byte length {max_byte_length} is smaller than byte length {byte_length}"
            ),
            Self::MaximumTooLarge {
                max_byte_length,
                maximum,
            } => write!(
                formatter,
                "max byte length {max_byte_length} exceeds pinned QuickJS maximum {maximum}"
            ),
        }
    }
}

/// Validate and return an ArrayBuffer byte length accepted by pinned QuickJS.
pub(in crate::runtime) fn validate_array_buffer_layout(
    byte_length: usize,
    max_byte_length: Option<u32>,
) -> Result<u32, ArrayBufferLayoutError> {
    let Ok(byte_length) = u32::try_from(byte_length) else {
        return Err(ArrayBufferLayoutError::ByteLengthTooLarge {
            byte_length,
            maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
        });
    };
    if let Some(max_byte_length) = max_byte_length {
        if max_byte_length < byte_length {
            return Err(ArrayBufferLayoutError::MaximumTooSmall {
                byte_length,
                max_byte_length,
            });
        }
    }
    if byte_length > MAX_ARRAY_BUFFER_BYTE_LENGTH {
        return Err(ArrayBufferLayoutError::ByteLengthTooLarge {
            byte_length: byte_length as usize,
            maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
        });
    }
    if let Some(max_byte_length) = max_byte_length {
        if max_byte_length > MAX_ARRAY_BUFFER_BYTE_LENGTH {
            return Err(ArrayBufferLayoutError::MaximumTooLarge {
                max_byte_length,
                maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
            });
        }
    }
    Ok(byte_length)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum TypedArrayLayoutError {
    UnalignedByteOffset {
        byte_offset: u32,
        element_byte_length: u8,
    },
    ViewOutOfBounds {
        byte_offset: u32,
        length: u32,
        element_byte_length: u8,
        backing_byte_length: usize,
    },
}

impl fmt::Display for TypedArrayLayoutError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnalignedByteOffset {
                byte_offset,
                element_byte_length,
            } => write!(
                formatter,
                "byte offset {byte_offset} is not aligned to {element_byte_length}-byte elements"
            ),
            Self::ViewOutOfBounds {
                byte_offset,
                length,
                element_byte_length,
                backing_byte_length,
            } => write!(
                formatter,
                "view at byte offset {byte_offset} with {length} {element_byte_length}-byte elements exceeds backing byte length {backing_byte_length}"
            ),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum TypedArrayBackingError {
    NotObject,
    Pending { node: NodeId },
    NotArrayBuffer { node: NodeId },
}

impl fmt::Display for TypedArrayBackingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotObject => formatter.write_str("backing value is not an object"),
            Self::Pending { node } => write!(
                formatter,
                "backing node {} is not complete",
                node.zero_based()
            ),
            Self::NotArrayBuffer { node } => write!(
                formatter,
                "backing node {} is not an ArrayBuffer",
                node.zero_based()
            ),
        }
    }
}

/// Validate the constructor checks performed by the pinned BC5 reader.
pub(in crate::runtime) fn validate_typed_array_layout(
    kind: TypedArrayKind,
    length: u32,
    byte_offset: u32,
    backing_byte_length: usize,
) -> Result<(), TypedArrayLayoutError> {
    let element_byte_length = kind.element_byte_length();
    if byte_offset % u32::from(element_byte_length) != 0 {
        return Err(TypedArrayLayoutError::UnalignedByteOffset {
            byte_offset,
            element_byte_length,
        });
    }

    let view_end = u128::from(byte_offset) + u128::from(length) * u128::from(element_byte_length);
    if view_end > backing_byte_length as u128 {
        return Err(TypedArrayLayoutError::ViewOutOfBounds {
            byte_offset,
            length,
            element_byte_length,
            backing_byte_length,
        });
    }
    Ok(())
}

/// Validate a graph state that the pinned writer can observe.
///
/// Shrinking a resizable ArrayBuffer can leave a TypedArray out of bounds.
/// QuickJS then writes a zero element count plus the original offset, even
/// though its own reader rejects the result. Preserve that writer asymmetry;
/// all other impossible graph layouts remain rejected.
pub(in crate::runtime) fn validate_typed_array_write_layout(
    kind: TypedArrayKind,
    length: u32,
    byte_offset: u32,
    backing_byte_length: usize,
    max_byte_length: Option<u32>,
) -> Result<(), TypedArrayLayoutError> {
    match validate_typed_array_layout(kind, length, byte_offset, backing_byte_length) {
        Ok(()) => Ok(()),
        Err(TypedArrayLayoutError::ViewOutOfBounds { .. })
            if length == 0 && max_byte_length.is_some_and(|maximum| byte_offset <= maximum) =>
        {
            Ok(())
        }
        Err(error) => Err(error),
    }
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
    max_shared_array_buffer_occurrences: usize,
    max_shared_array_buffer_backings: usize,
    max_shared_array_buffer_capacity: usize,
    max_total_shared_array_buffer_capacity: usize,
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
            // SharedArrayBuffer admission is a separate capability. Existing
            // policies keep it disabled until they opt into all four limits.
            max_shared_array_buffer_occurrences: 0,
            max_shared_array_buffer_backings: 0,
            max_shared_array_buffer_capacity: 0,
            max_total_shared_array_buffer_capacity: 0,
        }
    }

    /// Admit SharedArrayBuffer archive records under four independent bounds.
    ///
    /// Occurrences follow actual full SAB records and therefore include
    /// duplicate records when object references are disabled. Backing counts
    /// and aggregate capacity are charged once per archive-local backing.
    #[must_use]
    pub(in crate::runtime) const fn with_shared_array_buffers(
        mut self,
        max_occurrences: usize,
        max_backings: usize,
        max_backing_capacity: usize,
        max_total_backing_capacity: usize,
    ) -> Self {
        self.max_shared_array_buffer_occurrences = max_occurrences;
        self.max_shared_array_buffer_backings = max_backings;
        self.max_shared_array_buffer_capacity = max_backing_capacity;
        self.max_total_shared_array_buffer_capacity = max_total_backing_capacity;
        self
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
            GraphResourceKind::SharedArrayBufferOccurrences => {
                self.max_shared_array_buffer_occurrences
            }
            GraphResourceKind::SharedArrayBufferBackings => self.max_shared_array_buffer_backings,
            GraphResourceKind::SharedArrayBufferCapacity => self.max_shared_array_buffer_capacity,
            GraphResourceKind::TotalSharedArrayBufferCapacity => {
                self.max_total_shared_array_buffer_capacity
            }
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
    /// Nodes allocated while decoding, or unique reachable nodes while writing.
    Nodes,
    ObjectReferences,
    NestingDepth,
    ContainerEntries,
    TotalContainerEntries,
    BigIntBytes,
    TotalBigIntBytes,
    /// Bytes copied into one ArrayBuffer's current backing store.
    ArrayBufferBytes,
    /// Bytes copied into all emitted or decoded current ArrayBuffer backing
    /// stores. A resizable buffer's unallocated maximum is not charged here.
    TotalArrayBufferBytes,
    /// Complete SharedArrayBuffer records encountered on wire. ObjectReference
    /// tags do not count; repeated full records do.
    SharedArrayBufferOccurrences,
    /// Distinct archive-local shared backing identities.
    SharedArrayBufferBackings,
    /// Maximum capacity declared by one shared backing.
    SharedArrayBufferCapacity,
    /// Sum of capacities across distinct shared backing identities.
    TotalSharedArrayBufferCapacity,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn graph_carriers_are_generic_while_concrete_names_remain_inference_safe() {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        enum TestKey {
            Pinned(u32),
        }

        fn accepts_default_property(_: WireProperty) {}
        fn accepts_default_node(_: WireNode) {}
        fn accepts_concrete_graph(_: WireGraph) {}
        fn accepts_generic_node(_: WireNodeCarrier<u8, TestKey>) {}

        accepts_default_property(WireProperty {
            key: WireKey::Index(0),
            value: WireValue::Int32(42),
        });
        let inferred_array = WireNode::Array {
            elements: Box::from([WireValue::Int32(42)]),
        };
        let inferred_array_buffer = WireNode::ArrayBuffer {
            bytes: Box::default(),
            max_byte_length: None,
        };
        assert!(matches!(
            inferred_array,
            WireNode::Array { elements } if elements.as_ref() == [WireValue::Int32(42)]
        ));
        assert!(matches!(
            inferred_array_buffer,
            WireNode::ArrayBuffer { bytes, max_byte_length: None } if bytes.is_empty()
        ));
        accepts_default_node(WireNode::Date {
            time_value: DateNumber::try_from_wire_value(WireValue::Int32(42)).unwrap(),
        });
        accepts_concrete_graph(WireGraph {
            atoms: Box::default(),
            nodes: Box::from([WireNode::Array {
                elements: Box::from([WireValue::Int32(42)]),
            }]),
            ref_table: Box::default(),
            root: WireValue::Node(NodeId::from_zero_based(0)),
        });

        accepts_generic_node(WireNodeCarrier::Ordinary {
            properties: Box::from([WirePropertyCarrier {
                key: TestKey::Pinned(4),
                value: 40,
            }]),
        });
        accepts_generic_node(WireNodeCarrier::Array {
            elements: Box::from([41]),
        });
        accepts_generic_node(WireNodeCarrier::TemplateObject {
            elements: Box::from([42]),
            raw: 43,
        });
        accepts_generic_node(WireNodeCarrier::ArrayBuffer {
            bytes: Box::default(),
            max_byte_length: None,
        });
    }

    #[test]
    fn array_buffer_layout_validation_tracks_quickjs_constructor_bounds() {
        assert_eq!(validate_array_buffer_layout(4, None), Ok(4));
        assert_eq!(validate_array_buffer_layout(4, Some(4)), Ok(4));
        assert_eq!(
            validate_array_buffer_layout(4, Some(3)),
            Err(ArrayBufferLayoutError::MaximumTooSmall {
                byte_length: 4,
                max_byte_length: 3,
            })
        );
        assert_eq!(
            validate_array_buffer_layout(0, Some(0x8000_0000)),
            Err(ArrayBufferLayoutError::MaximumTooLarge {
                max_byte_length: 0x8000_0000,
                maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
            })
        );
        assert_eq!(
            validate_array_buffer_layout(0x8000_0000, None),
            Err(ArrayBufferLayoutError::ByteLengthTooLarge {
                byte_length: 0x8000_0000,
                maximum: MAX_ARRAY_BUFFER_BYTE_LENGTH,
            })
        );
    }

    #[test]
    fn typed_array_kind_mapping_matches_the_pinned_class_range() {
        let widths = [1, 1, 1, 2, 2, 4, 4, 8, 8, 2, 4, 8];
        for (index, (kind, width)) in TypedArrayKind::ALL.into_iter().zip(widths).enumerate() {
            let wire_byte = u8::try_from(index).unwrap();
            assert_eq!(TypedArrayKind::from_wire_byte(wire_byte), Some(kind));
            assert_eq!(kind.to_wire_byte(), wire_byte);
            assert_eq!(kind.element_byte_length(), width);
        }
        assert_eq!(TypedArrayKind::from_wire_byte(12), None);
        assert_eq!(TypedArrayKind::from_wire_byte(u8::MAX), None);
    }

    #[test]
    fn typed_array_layout_validation_tracks_reader_and_writer_asymmetry() {
        assert_eq!(
            validate_typed_array_layout(TypedArrayKind::Uint16, 2, 4, 8),
            Ok(())
        );
        assert_eq!(
            validate_typed_array_layout(TypedArrayKind::Uint16, 1, 1, 8),
            Err(TypedArrayLayoutError::UnalignedByteOffset {
                byte_offset: 1,
                element_byte_length: 2,
            })
        );
        let out_of_bounds = TypedArrayLayoutError::ViewOutOfBounds {
            byte_offset: 4,
            length: 0,
            element_byte_length: 2,
            backing_byte_length: 2,
        };
        assert_eq!(
            validate_typed_array_layout(TypedArrayKind::Uint16, 0, 4, 2),
            Err(out_of_bounds)
        );
        assert_eq!(
            validate_typed_array_write_layout(TypedArrayKind::Uint16, 0, 4, 2, Some(16)),
            Ok(())
        );
        assert_eq!(
            validate_typed_array_write_layout(TypedArrayKind::Uint16, 0, 4, 2, None),
            Err(out_of_bounds)
        );
        assert_eq!(
            validate_typed_array_write_layout(TypedArrayKind::Uint16, 1, 4, 2, Some(16)),
            Err(TypedArrayLayoutError::ViewOutOfBounds {
                byte_offset: 4,
                length: 1,
                element_byte_length: 2,
                backing_byte_length: 2,
            })
        );
    }

    #[test]
    fn boxed_primitive_excludes_values_that_do_not_create_wrapper_identity() {
        for value in [
            WireValue::Bool(true),
            WireValue::Int32(42),
            WireValue::Float64Bits((-0.0_f64).to_bits()),
            WireValue::String(WireString::Narrow(Box::from(*b"value"))),
            WireValue::BigInt(Box::from([1])),
        ] {
            let expected = value.clone();
            let primitive = BoxedPrimitive::try_from_wire_value(value).unwrap();
            assert_eq!(primitive.as_wire_value(), &expected);
        }

        for value in [WireValue::Null, WireValue::Undefined] {
            assert_eq!(
                BoxedPrimitive::try_from_wire_value(value),
                Err(BoxedPrimitiveError::NullOrUndefined)
            );
        }
        assert_eq!(
            BoxedPrimitive::try_from_wire_value(WireValue::Node(NodeId::from_zero_based(0))),
            Err(BoxedPrimitiveError::Object)
        );
    }

    #[test]
    fn date_number_preserves_only_exact_numeric_wire_representations() {
        for value in [
            WireValue::Int32(42),
            WireValue::Float64Bits((-0.0_f64).to_bits()),
            WireValue::Float64Bits(0x7ff8_0000_0000_0042),
        ] {
            let expected = value.clone();
            let number = DateNumber::try_from_wire_value(value).unwrap();
            assert_eq!(number.as_wire_value(), &expected);
        }

        for value in [
            WireValue::Undefined,
            WireValue::Null,
            WireValue::Bool(true),
            WireValue::String(WireString::Narrow(Box::from(*b"42"))),
            WireValue::BigInt(Box::from([42])),
            WireValue::Node(NodeId::from_zero_based(0)),
        ] {
            assert_eq!(
                DateNumber::try_from_wire_value(value),
                Err(DateNumberError::NotNumber)
            );
        }
    }
}
