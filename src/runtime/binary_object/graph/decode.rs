//! Bounded, heap-independent decoder for the first BC5 data-object slice.
//!
//! Containers are assembled through an explicit frame stack. Object, Array,
//! and TemplateObject identities enter the reference table before any child is
//! read, which preserves QuickJS's preorder reference numbering and cycles.
//! Their values reach the parent or root only after the complete subtree has
//! been consumed.

use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::convert::Infallible;
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::bigint::BC5_BIGINT_READ_MAX_BYTES;

use super::super::atoms::{AtomIndexSpace, BinaryAtom, BinaryObjectMode};
use super::super::bytecode_image::{ImageOpaque, ImageValue};
use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits};
use super::arena::{ArenaError, NodeState, ObjectArena, PendingNodeKind};
use super::model::{
    ArrayBufferLayoutError, AtomId, BoxedPrimitive, BoxedPrimitiveError, DateNumber,
    DateNumberError, GraphError, GraphLimits, GraphResourceKind, NodeId, TypedArrayBackingError,
    TypedArrayKind, TypedArrayLayoutError, WireGraph, WireKey, WireNodeCarrier,
    WirePropertyCarrier, WireValue, canonical_bigint_length, numeric_atom_index, semantic_atom_eq,
    semantic_atom_hash, validate_array_buffer_layout, validate_typed_array_layout,
};
#[cfg(test)]
use super::model::{WireNode, WireProperty};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum DecodeError<Opaque = Infallible> {
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
    OpaqueTypedArrayBacking {
        offset: usize,
        value: Opaque,
    },
    InvalidObjectValue {
        offset: usize,
        reason: BoxedPrimitiveError,
    },
    OpaqueObjectValue {
        offset: usize,
        value: Opaque,
    },
    InvalidObjectValueAlias {
        offset: usize,
        node: NodeId,
    },
    InvalidDate {
        offset: usize,
        reason: DateNumberError,
    },
    OpaqueDateValue {
        offset: usize,
        value: Opaque,
    },
    MachineIdExhausted,
    InvalidCompletionTarget,
    InvalidNodeState {
        node: NodeId,
    },
}

impl<Opaque: fmt::Debug> fmt::Display for DecodeError<Opaque> {
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
            Self::OpaqueTypedArrayBacking { offset, value } => write!(
                formatter,
                "invalid TypedArray backing at byte {offset}: opaque value {value:?} is not an object"
            ),
            Self::InvalidObjectValue { offset, reason } => {
                write!(formatter, "invalid ObjectValue at byte {offset}: {reason}")
            }
            Self::OpaqueObjectValue { offset, value } => write!(
                formatter,
                "invalid ObjectValue at byte {offset}: opaque value {value:?} cannot be converted to an object"
            ),
            Self::InvalidObjectValueAlias { offset, node } => write!(
                formatter,
                "invalid ObjectValue alias at byte {offset}: TypedArray node {} is not complete",
                node.zero_based()
            ),
            Self::InvalidDate { offset, reason } => {
                write!(formatter, "invalid Date at byte {offset}: {reason}")
            }
            Self::OpaqueDateValue { offset, value } => write!(
                formatter,
                "invalid Date at byte {offset}: opaque value {value:?} is not a number"
            ),
            Self::MachineIdExhausted => {
                formatter.write_str("data-machine identity space is exhausted")
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

impl<Opaque: fmt::Debug> std::error::Error for DecodeError<Opaque> {}

impl<Opaque> From<WireError> for DecodeError<Opaque> {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl<Opaque> From<GraphError> for DecodeError<Opaque> {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl<Opaque> From<ArenaError> for DecodeError<Opaque> {
    fn from(error: ArenaError) -> Self {
        match error {
            ArenaError::Graph(error) => Self::Graph(error),
            ArenaError::InvalidNodeState { node } => Self::InvalidNodeState { node },
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
struct MachineId(u64);

/// Opaque identity of one data-machine traversal.
///
/// The raw counter is never exposed. Whole-image opaque identities retain this
/// token so a value originating in one decoder cannot be rebranded by another.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime::binary_object) struct MachineSource(MachineId);

static NEXT_MACHINE_ID: AtomicU64 = AtomicU64::new(1);

impl MachineId {
    fn allocate<Opaque>() -> Result<Self, DecodeError<Opaque>> {
        NEXT_MACHINE_ID
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |next| {
                next.checked_add(1)
            })
            .map(Self)
            .map_err(|_| DecodeError::MachineIdExhausted)
    }
}

mod sealed {
    pub trait Sealed {}
}

/// Adapter between the concrete BC5 data values assembled here and a caller's
/// wider whole-image value type.
///
/// A plain data graph uses [`WireValue`] directly and therefore has no opaque
/// values. A bytecode-image reader can wrap those same values while retaining
/// a separate function identity variant. The opaque discriminator is returned
/// only when a data-only tag (TypedArray, ObjectValue, or Date) needs to inspect
/// such a wider value.
///
/// This is a sealed, trusted internal adapter contract. Every value returned by
/// `from_wire(wire)` must return `Ok` from both `as_wire` and `into_wire`, with
/// the same `WireValue`. Conversely, `as_wire` and `into_wire` must agree on
/// which variants are opaque, and neither an opaque variant nor its discriminator
/// may contain or re-encode a raw `WireValue` or `NodeId`. Every opaque variant
/// must return the machine source retained by its identity; `wrap_opaque_value`
/// rejects a missing or foreign source. Violating these laws can defeat the
/// arena-provenance checks and is an internal implementation bug.
#[allow(private_bounds)]
pub(in crate::runtime::binary_object) trait DataValue:
    sealed::Sealed + Sized
{
    type Opaque: Copy + fmt::Debug;

    fn from_wire(value: WireValue) -> Self;
    fn as_wire(&self) -> Result<&WireValue, Self::Opaque>;
    fn into_wire(self) -> Result<WireValue, Self::Opaque>;
    fn opaque_source(&self) -> Option<MachineSource>;
}

impl sealed::Sealed for WireValue {}

impl DataValue for WireValue {
    type Opaque = Infallible;

    fn from_wire(value: WireValue) -> Self {
        value
    }

    fn as_wire(&self) -> Result<&WireValue, Self::Opaque> {
        Ok(self)
    }

    fn into_wire(self) -> Result<WireValue, Self::Opaque> {
        Ok(self)
    }

    fn opaque_source(&self) -> Option<MachineSource> {
        None
    }
}

impl sealed::Sealed for ImageValue {}

impl DataValue for ImageValue {
    type Opaque = ImageOpaque;

    fn from_wire(value: WireValue) -> Self {
        Self::from_wire(value)
    }

    fn as_wire(&self) -> Result<&WireValue, Self::Opaque> {
        self.as_wire()
    }

    fn into_wire(self) -> Result<WireValue, Self::Opaque> {
        self.into_wire()
    }

    fn opaque_source(&self) -> Option<MachineSource> {
        self.opaque().map(ImageOpaque::source)
    }
}

/// One completed value tied linearly to the machine whose arena gave meaning
/// to any contained [`NodeId`]. For lawful [`DataValue`] adapters, private
/// fields prevent sibling drivers from rebranding values or splicing identities
/// across whole-image decoders.
pub(in crate::runtime::binary_object) struct DataCompletion<V> {
    owner: MachineId,
    value: V,
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
    let atom_space = AtomIndexSpace::new(BinaryObjectMode::Data, header.atom_count)?;
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

    let mut state = DataMachine::new(graph_limits, allow_object_references)?;
    let mut frames: Vec<ActiveFrame> = Vec::new();
    let mut root = None;

    loop {
        if root.is_some() && frames.is_empty() {
            break;
        }

        let return_to = match frames.last() {
            Some(active) => {
                let key = active
                    .frame
                    .expects_property_key()
                    .then(|| read_key(&mut cursor, atom_space, &header_atoms))
                    .transpose()?;
                CompletionTarget::Parent { key }
            }
            None => CompletionTarget::Root,
        };
        let tag_offset = cursor.position();
        let tag = cursor.read_tag()?;
        match state.read_value_after_tag(&mut cursor, tag, tag_offset, frames.len())? {
            DataReadStep::Complete(value) => {
                deliver_completed(&state, &mut frames, return_to, value, &mut root)?;
            }
            DataReadStep::Pending(frame) => {
                frames
                    .try_reserve(1)
                    .map_err(|_| GraphError::AllocationFailed)?;
                frames.push(ActiveFrame { frame, return_to });
            }
        }

        while frames
            .last()
            .is_some_and(|active| active.frame.is_complete())
        {
            let active = frames.pop().ok_or(DecodeError::InvalidCompletionTarget)?;
            let value = state.finish_frame(active.frame)?;
            deliver_completed(&state, &mut frames, active.return_to, value, &mut root)?;
        }
    }

    // This call is unconditional: QuickJsCompatible itself decides to accept
    // trailing bytes, rather than the graph layer bypassing finalization.
    cursor.finish()?;

    let root = state.unwrap_completion(root.ok_or(DecodeError::InvalidCompletionTarget)?)?;
    let parts = state.finish()?;
    Ok(WireGraph {
        atoms: atoms.into_boxed_slice(),
        nodes: parts.nodes,
        ref_table: parts.ref_table,
        root,
    })
}

pub(in crate::runtime::binary_object) struct DataMachine<V, K> {
    id: MachineId,
    limits: GraphLimits,
    arena: ObjectArena<V, K>,
    total_container_entries: usize,
    total_bigint_bytes: usize,
    total_array_buffer_bytes: usize,
}

impl<V, K> DataMachine<V, K>
where
    V: DataValue,
    K: Copy + Eq + std::hash::Hash,
{
    pub(in crate::runtime::binary_object) fn new(
        limits: GraphLimits,
        allow_object_references: bool,
    ) -> Result<Self, DecodeError<V::Opaque>> {
        Ok(Self {
            id: MachineId::allocate()?,
            limits,
            arena: ObjectArena::new(limits, allow_object_references),
            total_container_entries: 0,
            total_bigint_bytes: 0,
            total_array_buffer_bytes: 0,
        })
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn source(&self) -> MachineSource {
        MachineSource(self.id)
    }

    /// Bind a caller-produced opaque value, such as a completed function or
    /// module record, to this machine. For a lawful [`DataValue`] adapter, concrete
    /// wire values are rejected; in particular, a raw NodeId cannot be
    /// rebranded into another arena. Opaque values must also carry this exact
    /// machine's source token. The whole-image record tables separately prove
    /// that a same-source opaque identity names one of their reserved slots.
    pub(in crate::runtime::binary_object) fn wrap_opaque_value(
        &self,
        value: V,
    ) -> Result<DataCompletion<V>, DecodeError<V::Opaque>> {
        if value.as_wire().is_ok() {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        if value.opaque_source() != Some(self.source()) {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        Ok(self.complete(value))
    }

    /// Remove provenance inside the data decoder only. Whole-image siblings
    /// must instead consume the machine through [`Self::finish_output`].
    fn unwrap_completion(
        &self,
        completion: DataCompletion<V>,
    ) -> Result<V, DecodeError<V::Opaque>> {
        self.validate_completion(&completion)?;
        Ok(completion.value)
    }

    fn complete(&self, value: V) -> DataCompletion<V> {
        DataCompletion {
            owner: self.id,
            value,
        }
    }

    fn validate_completion(
        &self,
        completion: &DataCompletion<V>,
    ) -> Result<(), DecodeError<V::Opaque>> {
        if completion.owner != self.id {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        Ok(())
    }

    /// Attach a completed child only when the value and destination frame were
    /// both issued by this machine. Both owner checks precede frame mutation.
    pub(in crate::runtime::binary_object) fn attach_to_frame(
        &self,
        frame: &mut DataFrame<V, K>,
        key: Option<PropertyDisposition<K>>,
        completion: DataCompletion<V>,
    ) -> Result<(), DecodeError<V::Opaque>> {
        if frame.owner != self.id {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        self.validate_completion(&completion)?;
        frame.attach_raw(key, completion.value)?;
        Ok(())
    }

    pub(in crate::runtime::binary_object) fn read_value_after_tag(
        &mut self,
        cursor: &mut WireCursor<'_>,
        tag: BcTag,
        tag_offset: usize,
        active_depth: usize,
    ) -> Result<DataReadStep<V, K>, DecodeError<V::Opaque>> {
        let value = match tag {
            BcTag::Null => WireValue::Null,
            BcTag::Undefined => WireValue::Undefined,
            BcTag::BoolFalse => WireValue::Bool(false),
            BcTag::BoolTrue => WireValue::Bool(true),
            BcTag::Int32 => WireValue::Int32(cursor.read_i32()?),
            BcTag::Float64 => WireValue::Float64Bits(cursor.read_f64()?.to_bits()),
            BcTag::String => WireValue::String(cursor.read_string()?),
            BcTag::BigInt => self.read_bigint(cursor)?,
            BcTag::Object => {
                return self.begin_container(cursor, ContainerKind::Ordinary, active_depth);
            }
            BcTag::Array => {
                return self.begin_container(cursor, ContainerKind::Array, active_depth);
            }
            BcTag::TemplateObject => {
                return self.begin_container(cursor, ContainerKind::TemplateObject, active_depth);
            }
            BcTag::TypedArray => {
                return self.begin_typed_array(cursor, tag_offset, active_depth);
            }
            BcTag::ObjectValue => return self.begin_object_value(tag_offset, active_depth),
            BcTag::Date => return self.begin_date(tag_offset, active_depth),
            BcTag::ArrayBuffer => return self.read_array_buffer(cursor, active_depth),
            BcTag::ObjectReference => {
                if !self.arena.allows_references() {
                    return Err(DecodeError::ObjectReferencesNotAllowed { offset: tag_offset });
                }
                let index = cursor.read_uleb128()?;
                let node = self.arena.resolve_reference(index)?;
                WireValue::Node(node)
            }
            BcTag::FunctionBytecode | BcTag::Module | BcTag::SharedArrayBuffer => {
                return Err(DecodeError::UnsupportedTag {
                    tag,
                    offset: tag_offset,
                });
            }
        };

        Ok(DataReadStep::Complete(self.complete(V::from_wire(value))))
    }

    fn read_bigint(
        &mut self,
        cursor: &mut WireCursor<'_>,
    ) -> Result<WireValue, DecodeError<V::Opaque>> {
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

    fn read_array_buffer(
        &mut self,
        cursor: &mut WireCursor<'_>,
        active_depth: usize,
    ) -> Result<DataReadStep<V, K>, DecodeError<V::Opaque>> {
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
        self.check_next_node_depth(active_depth)?;
        // Preflight the arena/reference work before copying a potentially
        // large payload. The node is installed only after the leaf is complete,
        // matching QuickJS's ArrayBuffer reference-registration point.
        let reservation = self.arena.reserve_node()?;

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
        let node = reservation.install_ready_node(WireNodeCarrier::ArrayBuffer {
            bytes: bytes.into_boxed_slice(),
            max_byte_length,
        })?;
        Ok(DataReadStep::Complete(
            self.complete(V::from_wire(WireValue::Node(node))),
        ))
    }

    fn begin_typed_array(
        &mut self,
        cursor: &mut WireCursor<'_>,
        tag_offset: usize,
        active_depth: usize,
    ) -> Result<DataReadStep<V, K>, DecodeError<V::Opaque>> {
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

        self.check_next_node_depth(active_depth)?;
        let reservation = self.arena.reserve_node()?;
        let node = reservation.install_pending_node(PendingNodeKind::TypedArray)?;
        Ok(DataReadStep::Pending(DataFrame {
            owner: self.id,
            kind: DataFrameKind::TypedArray {
                node,
                offset: tag_offset,
                kind,
                length,
                byte_offset,
                backing: None,
            },
        }))
    }

    fn begin_object_value(
        &self,
        tag_offset: usize,
        active_depth: usize,
    ) -> Result<DataReadStep<V, K>, DecodeError<V::Opaque>> {
        self.check_next_node_depth(active_depth)?;
        Ok(DataReadStep::Pending(DataFrame {
            owner: self.id,
            kind: DataFrameKind::ObjectValue {
                offset: tag_offset,
                value: None,
            },
        }))
    }

    fn begin_date(
        &self,
        tag_offset: usize,
        active_depth: usize,
    ) -> Result<DataReadStep<V, K>, DecodeError<V::Opaque>> {
        self.check_next_node_depth(active_depth)?;
        Ok(DataReadStep::Pending(DataFrame {
            owner: self.id,
            kind: DataFrameKind::Date {
                offset: tag_offset,
                value: None,
            },
        }))
    }

    fn begin_container(
        &mut self,
        cursor: &mut WireCursor<'_>,
        kind: ContainerKind,
        active_depth: usize,
    ) -> Result<DataReadStep<V, K>, DecodeError<V::Opaque>> {
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

        self.check_next_node_depth(active_depth)?;

        let reservation = self.arena.reserve_node()?;
        let node_id = reservation.install_pending_node(kind.pending_node_kind())?;
        let frame = DataFrame::new(self.id, kind, node_id, entry_count)?;
        Ok(DataReadStep::Pending(frame))
    }

    pub(in crate::runtime::binary_object) fn finish_frame(
        &mut self,
        frame: DataFrame<V, K>,
    ) -> Result<DataCompletion<V>, DecodeError<V::Opaque>> {
        if frame.owner != self.id {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        if !frame.is_complete() {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        let (node, replacement) = match frame.kind {
            DataFrameKind::ObjectValue { offset, value } => {
                return self.finish_object_value(offset, value);
            }
            DataFrameKind::Date { offset, value } => {
                return self.finish_date(offset, value);
            }
            DataFrameKind::Ordinary {
                node, properties, ..
            } => (
                node,
                WireNodeCarrier::Ordinary {
                    properties: properties.into_boxed_slice(),
                },
            ),
            DataFrameKind::Array { node, elements, .. } => (
                node,
                WireNodeCarrier::Array {
                    elements: elements.into_boxed_slice(),
                },
            ),
            DataFrameKind::TemplateObject {
                node,
                elements,
                raw,
                ..
            } => (
                node,
                WireNodeCarrier::TemplateObject {
                    elements: elements.into_boxed_slice(),
                    raw: raw.ok_or(DecodeError::InvalidCompletionTarget)?,
                },
            ),
            DataFrameKind::TypedArray {
                node,
                offset,
                kind,
                length,
                byte_offset,
                backing,
            } => {
                let backing = backing.ok_or(DecodeError::InvalidCompletionTarget)?;
                let buffer = match backing.as_wire() {
                    Ok(WireValue::Node(buffer)) => *buffer,
                    Ok(_) => {
                        return Err(DecodeError::InvalidTypedArrayBacking {
                            offset,
                            reason: TypedArrayBackingError::NotObject,
                        });
                    }
                    Err(value) => {
                        return Err(DecodeError::OpaqueTypedArrayBacking { offset, value });
                    }
                };
                let backing_byte_length = match self.arena.node_state(buffer)? {
                    NodeState::Ready(WireNodeCarrier::ArrayBuffer { bytes, .. }) => bytes.len(),
                    NodeState::Ready(_) => {
                        return Err(DecodeError::InvalidTypedArrayBacking {
                            offset,
                            reason: TypedArrayBackingError::NotArrayBuffer { node: buffer },
                        });
                    }
                    NodeState::Pending(PendingNodeKind::TypedArray) => {
                        return Err(DecodeError::InvalidTypedArrayBacking {
                            offset,
                            reason: TypedArrayBackingError::Pending { node: buffer },
                        });
                    }
                    NodeState::Pending(
                        PendingNodeKind::Ordinary
                        | PendingNodeKind::Array
                        | PendingNodeKind::TemplateObject,
                    ) => {
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
                    WireNodeCarrier::TypedArray {
                        kind,
                        length,
                        byte_offset,
                        buffer,
                    },
                )
            }
        };
        self.arena.complete_node(node, replacement)?;
        Ok(self.complete(V::from_wire(WireValue::Node(node))))
    }

    fn finish_object_value(
        &mut self,
        offset: usize,
        value: Option<V>,
    ) -> Result<DataCompletion<V>, DecodeError<V::Opaque>> {
        let value = value.ok_or(DecodeError::InvalidCompletionTarget)?;
        if let Ok(WireValue::Node(node)) = value.as_wire() {
            let node = *node;
            if matches!(
                self.arena.node_state(node)?,
                NodeState::Pending(PendingNodeKind::TypedArray)
            ) {
                return Err(DecodeError::InvalidObjectValueAlias { offset, node });
            }
            self.arena.append_reference_alias(node)?;
            return Ok(self.complete(V::from_wire(WireValue::Node(node))));
        }

        let value = value
            .into_wire()
            .map_err(|value| DecodeError::OpaqueObjectValue { offset, value })?;
        let primitive = BoxedPrimitive::try_from_wire_value(value)
            .map_err(|reason| DecodeError::InvalidObjectValue { offset, reason })?;
        let reservation = self.arena.reserve_node()?;
        let node = reservation.install_ready_node(WireNodeCarrier::ObjectValue { primitive })?;
        Ok(self.complete(V::from_wire(WireValue::Node(node))))
    }

    fn finish_date(
        &mut self,
        offset: usize,
        value: Option<V>,
    ) -> Result<DataCompletion<V>, DecodeError<V::Opaque>> {
        let value = value.ok_or(DecodeError::InvalidCompletionTarget)?;
        let value = value
            .into_wire()
            .map_err(|value| DecodeError::OpaqueDateValue { offset, value })?;
        let time_value = DateNumber::try_from_wire_value(value)
            .map_err(|reason| DecodeError::InvalidDate { offset, reason })?;
        // Pinned QuickJS creates and registers the Date identity only after its
        // complete child has been read and proved numeric.
        let reservation = self.arena.reserve_node()?;
        let node = reservation.install_ready_node(WireNodeCarrier::Date { time_value })?;
        Ok(self.complete(V::from_wire(WireValue::Node(node))))
    }

    pub(in crate::runtime::binary_object) fn finish(
        self,
    ) -> Result<super::arena::ObjectArenaParts<V, K>, DecodeError<V::Opaque>> {
        self.arena.finish().map_err(Into::into)
    }

    pub(in crate::runtime::binary_object) fn finish_output(
        self,
    ) -> Result<DataMachineOutput<V, K>, DecodeError<V::Opaque>> {
        Ok(DataMachineOutput {
            owner: self.id,
            parts: self.arena.finish()?,
        })
    }

    fn check_next_node_depth(&self, active_depth: usize) -> Result<(), GraphError> {
        let depth = active_depth
            .checked_add(1)
            .ok_or(GraphError::CountOverflow {
                kind: GraphResourceKind::NestingDepth,
            })?;
        self.limits.check(GraphResourceKind::NestingDepth, depth)
    }
}

/// Consuming finalization capability for one completed data machine.
///
/// The source machine no longer exists, so values unwrapped here cannot be
/// attached back into it. The capability retains the source identity long
/// enough to unwrap every root and whole-image nested value exactly once by
/// move.
pub(in crate::runtime::binary_object) struct DataMachineOutput<V, K> {
    owner: MachineId,
    parts: super::arena::ObjectArenaParts<V, K>,
}

impl<V, K> DataMachineOutput<V, K>
where
    V: DataValue,
{
    pub(in crate::runtime::binary_object) fn unwrap_completion(
        &self,
        completion: DataCompletion<V>,
    ) -> Result<V, DecodeError<V::Opaque>> {
        if completion.owner != self.owner {
            return Err(DecodeError::InvalidCompletionTarget);
        }
        Ok(completion.value)
    }

    pub(in crate::runtime::binary_object) fn into_parts(
        self,
    ) -> super::arena::ObjectArenaParts<V, K> {
        self.parts
    }
}

#[derive(Clone, Copy)]
enum ContainerKind {
    Ordinary,
    Array,
    TemplateObject,
}

impl ContainerKind {
    const fn pending_node_kind(self) -> PendingNodeKind {
        match self {
            Self::Ordinary => PendingNodeKind::Ordinary,
            Self::Array => PendingNodeKind::Array,
            Self::TemplateObject => PendingNodeKind::TemplateObject,
        }
    }
}

pub(in crate::runtime::binary_object) enum DataReadStep<V, K> {
    Complete(DataCompletion<V>),
    Pending(DataFrame<V, K>),
}

#[derive(Clone, Copy)]
enum CompletionTarget {
    Root,
    Parent {
        key: Option<PropertyDisposition<WireKey>>,
    },
}

struct ActiveFrame {
    frame: DataFrame<WireValue, WireKey>,
    return_to: CompletionTarget,
}

fn deliver_completed(
    state: &DataMachine<WireValue, WireKey>,
    frames: &mut [ActiveFrame],
    target: CompletionTarget,
    value: DataCompletion<WireValue>,
    root: &mut Option<DataCompletion<WireValue>>,
) -> Result<(), DecodeError> {
    match target {
        CompletionTarget::Root => {
            state.validate_completion(&value)?;
            if root.replace(value).is_some() {
                return Err(DecodeError::InvalidCompletionTarget);
            }
        }
        CompletionTarget::Parent { key } => {
            let parent = frames
                .last_mut()
                .ok_or(DecodeError::InvalidCompletionTarget)?;
            state.attach_to_frame(&mut parent.frame, key, value)?;
        }
    }
    Ok(())
}

/// One in-progress data container tied to the machine which reserved its node
/// identities. The private representation prevents sibling drivers from
/// destructuring and recombining provenance with another frame's contents.
pub(in crate::runtime::binary_object) struct DataFrame<V, K> {
    owner: MachineId,
    kind: DataFrameKind<V, K>,
}

enum DataFrameKind<V, K> {
    Ordinary {
        node: NodeId,
        expected: usize,
        consumed: usize,
        properties: Vec<WirePropertyCarrier<V, K>>,
        property_indices: HashMap<K, usize>,
    },
    Array {
        node: NodeId,
        expected: usize,
        elements: Vec<V>,
    },
    TemplateObject {
        node: NodeId,
        expected: usize,
        elements: Vec<V>,
        raw: Option<V>,
    },
    TypedArray {
        node: NodeId,
        offset: usize,
        kind: TypedArrayKind,
        length: u32,
        byte_offset: u32,
        backing: Option<V>,
    },
    ObjectValue {
        offset: usize,
        value: Option<V>,
    },
    Date {
        offset: usize,
        value: Option<V>,
    },
}

impl<V, K> DataFrame<V, K>
where
    K: Copy + Eq + std::hash::Hash,
{
    fn new(
        owner: MachineId,
        kind: ContainerKind,
        node: NodeId,
        expected: usize,
    ) -> Result<Self, GraphError> {
        Ok(Self {
            owner,
            kind: DataFrameKind::new(kind, node, expected)?,
        })
    }

    fn attach_raw(
        &mut self,
        key: Option<PropertyDisposition<K>>,
        value: V,
    ) -> Result<(), GraphError> {
        self.kind.attach(key, value)
    }

    pub(in crate::runtime::binary_object) fn expects_property_key(&self) -> bool {
        self.kind.expects_property_key()
    }

    pub(in crate::runtime::binary_object) fn is_complete(&self) -> bool {
        self.kind.is_complete()
    }
}

impl<V, K> DataFrameKind<V, K>
where
    K: Copy + Eq + std::hash::Hash,
{
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
            ContainerKind::TemplateObject => {
                let mut elements = Vec::new();
                elements
                    .try_reserve_exact(expected)
                    .map_err(|_| GraphError::AllocationFailed)?;
                Ok(Self::TemplateObject {
                    node,
                    expected,
                    elements,
                    raw: None,
                })
            }
        }
    }

    fn attach(&mut self, key: Option<PropertyDisposition<K>>, value: V) -> Result<(), GraphError> {
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
                if let PropertyDisposition::Define(key) = key {
                    if let Some(index) = property_indices.get(&key).copied() {
                        properties[index].value = value;
                    } else {
                        let index = properties.len();
                        properties.push(WirePropertyCarrier { key, value });
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
            Self::TemplateObject {
                expected,
                elements,
                raw,
                ..
            } => {
                if key.is_some() {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                if elements.len() < *expected {
                    elements.push(value);
                } else if raw.is_none() {
                    *raw = Some(value);
                } else {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
            }
            Self::TypedArray { backing, .. } => {
                if key.is_some() || backing.is_some() {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                *backing = Some(value);
            }
            Self::ObjectValue {
                value: boxed_value, ..
            } => {
                if key.is_some() || boxed_value.is_some() {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                *boxed_value = Some(value);
            }
            Self::Date {
                value: time_value, ..
            } => {
                if key.is_some() || time_value.is_some() {
                    return Err(GraphError::CountOverflow {
                        kind: GraphResourceKind::ContainerEntries,
                    });
                }
                *time_value = Some(value);
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
            Self::TemplateObject {
                expected,
                elements,
                raw,
                ..
            } => elements.len() == *expected && raw.is_some(),
            Self::TypedArray { backing, .. } => backing.is_some(),
            Self::ObjectValue { value, .. } => value.is_some(),
            Self::Date { value, .. } => value.is_some(),
        }
    }
}

#[derive(Clone, Copy)]
pub(in crate::runtime::binary_object) enum PropertyDisposition<K> {
    Define(K),
    Ignore,
}

fn read_key(
    cursor: &mut WireCursor<'_>,
    atom_space: AtomIndexSpace,
    header_atoms: &[WireKey],
) -> Result<PropertyDisposition<WireKey>, DecodeError> {
    let offset = cursor.position();
    match atom_space.decode_metadata_atom(cursor)? {
        BinaryAtom::Null => match cursor.mode() {
            ReaderMode::Strict => Err(DecodeError::NullPropertyKey { offset }),
            ReaderMode::QuickJsCompatible => Ok(PropertyDisposition::Ignore),
        },
        BinaryAtom::Index(index) => Ok(PropertyDisposition::Define(WireKey::Index(index))),
        BinaryAtom::Header(slot) => {
            let key = header_atoms.get(slot.index() as usize).copied().ok_or(
                GraphError::InvalidAtomIndex {
                    index: slot.index(),
                    atom_count: header_atoms.len(),
                },
            )?;
            Ok(PropertyDisposition::Define(key))
        }
        BinaryAtom::Predefined(atom) => Err(WireError::InvalidAtomIndex {
            offset: cursor.position(),
            index: atom.raw(),
            first_atom: atom_space.first_atom(),
            atom_count: atom_space.header_count(),
        }
        .into()),
    }
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
    use crate::atom::ATOM_MAX_TABLE_INDEX;

    const WIRE_LIMITS: WireLimits = WireLimits::new(4096, 32, 1024, 2048);
    const GRAPH_LIMITS: GraphLimits =
        GraphLimits::new(64, 64, 32, 128, 256, 1024, 2048, 1024, 2048);

    fn decode(input: &[u8], mode: ReaderMode, references: bool) -> Result<WireGraph, DecodeError> {
        decode_graph(input, mode, WIRE_LIMITS, GRAPH_LIMITS, references)
    }

    #[derive(Clone, Debug, Eq, PartialEq)]
    enum WiderValue {
        Data(WireValue),
        Function(MachineSource, u32),
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum WiderOpaque {
        Function(u32),
    }

    impl sealed::Sealed for WiderValue {}

    impl DataValue for WiderValue {
        type Opaque = WiderOpaque;

        fn from_wire(value: WireValue) -> Self {
            Self::Data(value)
        }

        fn as_wire(&self) -> Result<&WireValue, Self::Opaque> {
            match self {
                Self::Data(value) => Ok(value),
                Self::Function(_, function) => Err(WiderOpaque::Function(*function)),
            }
        }

        fn into_wire(self) -> Result<WireValue, Self::Opaque> {
            match self {
                Self::Data(value) => Ok(value),
                Self::Function(_, function) => Err(WiderOpaque::Function(function)),
            }
        }

        fn opaque_source(&self) -> Option<MachineSource> {
            match self {
                Self::Data(_) => None,
                Self::Function(source, _) => Some(*source),
            }
        }
    }

    fn pending_wire_frame(
        machine: &mut DataMachine<WireValue, WireKey>,
        tag: BcTag,
        body: &[u8],
        tag_offset: usize,
    ) -> DataFrame<WireValue, WireKey> {
        let mut cursor = WireCursor::new(body, ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let step = machine
            .read_value_after_tag(&mut cursor, tag, tag_offset, 0)
            .unwrap();
        cursor.finish().unwrap();
        let DataReadStep::Pending(frame) = step else {
            panic!("test tag must produce a pending data frame");
        };
        frame
    }

    fn pending_wider_frame(
        machine: &mut DataMachine<WiderValue, WireKey>,
        tag: BcTag,
        body: &[u8],
        tag_offset: usize,
    ) -> DataFrame<WiderValue, WireKey> {
        let mut cursor = WireCursor::new(body, ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let step = machine
            .read_value_after_tag(&mut cursor, tag, tag_offset, 0)
            .unwrap();
        cursor.finish().unwrap();
        let DataReadStep::Pending(frame) = step else {
            panic!("test tag must produce a pending wider-value frame");
        };
        frame
    }

    #[test]
    fn incomplete_container_frames_cannot_publish_pending_nodes() {
        for (tag, key) in [
            (
                BcTag::Object,
                Some(PropertyDisposition::Define(WireKey::Index(0))),
            ),
            (BcTag::Array, None),
        ] {
            let mut early = DataMachine::<WireValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
            let early_frame = pending_wire_frame(&mut early, tag, &[1], 5);
            assert!(!early_frame.is_complete());
            assert!(matches!(
                early.finish_frame(early_frame),
                Err(DecodeError::InvalidCompletionTarget)
            ));
            assert!(matches!(
                early.finish(),
                Err(DecodeError::InvalidNodeState { node })
                    if node == NodeId::from_zero_based(0)
            ));

            // The same incomplete shape remains lawful when its child is
            // attached before finish. This second frame avoids consuming the
            // one used to exercise the public early-finish rejection above.
            let mut lawful = DataMachine::<WireValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
            let mut lawful_frame = pending_wire_frame(&mut lawful, tag, &[1], 6);
            let mut value_cursor =
                WireCursor::new(&[0x54], ReaderMode::Strict, WIRE_LIMITS).unwrap();
            let DataReadStep::Complete(value) = lawful
                .read_value_after_tag(&mut value_cursor, BcTag::Int32, 7, 1)
                .unwrap()
            else {
                panic!("Int32 must complete immediately");
            };
            value_cursor.finish().unwrap();
            lawful
                .attach_to_frame(&mut lawful_frame, key, value)
                .unwrap();
            assert!(lawful_frame.is_complete());
            let root = lawful.finish_frame(lawful_frame).unwrap();
            assert_eq!(
                lawful.unwrap_completion(root).unwrap(),
                WireValue::Node(NodeId::from_zero_based(0))
            );
            let parts = lawful.finish().unwrap();
            match tag {
                BcTag::Object => assert_eq!(
                    parts.nodes.as_ref(),
                    &[WireNode::Ordinary {
                        properties: Box::from([WireProperty {
                            key: WireKey::Index(0),
                            value: WireValue::Int32(42),
                        }]),
                    }]
                ),
                BcTag::Array => assert_eq!(
                    parts.nodes.as_ref(),
                    &[WireNode::Array {
                        elements: Box::from([WireValue::Int32(42)]),
                    }]
                ),
                _ => unreachable!("test table contains only container tags"),
            }
        }
    }

    #[test]
    fn data_machine_provenance_rejects_foreign_frames_and_completions() {
        let mut machine_a = DataMachine::<WireValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let mut machine_b = DataMachine::<WireValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        // Both frames are the first Array in their arena, hence both carry the
        // same kind and NodeId(0). Provenance, not shape or index, rejects the
        // swap before machine A's pending slot can be completed.
        let frame_a = pending_wire_frame(&mut machine_a, BcTag::Array, &[0], 10);
        let frame_b = pending_wire_frame(&mut machine_b, BcTag::Array, &[0], 20);
        assert!(matches!(
            machine_a.finish_frame(frame_b),
            Err(DecodeError::InvalidCompletionTarget)
        ));
        let completion_a = machine_a.finish_frame(frame_a).unwrap();
        assert_eq!(
            machine_a.unwrap_completion(completion_a).unwrap(),
            WireValue::Node(NodeId::from_zero_based(0))
        );
        let parts_a = machine_a.finish().unwrap();
        assert_eq!(
            parts_a.nodes.as_ref(),
            &[WireNode::Array {
                elements: Box::new([]),
            }]
        );
        assert!(matches!(
            machine_b.finish(),
            Err(DecodeError::InvalidNodeState { node })
                if node == NodeId::from_zero_based(0)
        ));

        let mut source = DataMachine::<WireValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let mut source_cursor = WireCursor::new(&[0x54], ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let DataReadStep::Complete(foreign_value) = source
            .read_value_after_tag(&mut source_cursor, BcTag::Int32, 30, 0)
            .unwrap()
        else {
            panic!("Int32 must complete immediately");
        };
        source_cursor.finish().unwrap();

        let mut target = DataMachine::<WireValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let mut target_frame = pending_wire_frame(&mut target, BcTag::Array, &[1], 40);
        assert!(matches!(
            target.attach_to_frame(&mut target_frame, None, foreign_value),
            Err(DecodeError::InvalidCompletionTarget)
        ));
        assert!(!target_frame.is_complete());

        let mut target_cursor = WireCursor::new(&[0x54], ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let DataReadStep::Complete(local_value) = target
            .read_value_after_tag(&mut target_cursor, BcTag::Int32, 41, 1)
            .unwrap()
        else {
            panic!("Int32 must complete immediately");
        };
        target_cursor.finish().unwrap();
        target
            .attach_to_frame(&mut target_frame, None, local_value)
            .unwrap();
        let target_root = target.finish_frame(target_frame).unwrap();
        assert_eq!(
            target.unwrap_completion(target_root).unwrap(),
            WireValue::Node(NodeId::from_zero_based(0))
        );
        let target_parts = target.finish().unwrap();
        assert_eq!(
            target_parts.nodes.as_ref(),
            &[WireNode::Array {
                elements: Box::from([WireValue::Int32(42)]),
            }]
        );
    }

    #[test]
    fn opaque_function_values_store_safely_but_wire_nodes_cannot_be_rebranded() {
        let mut ordinary = DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let ordinary_source = ordinary.source();
        let mut ordinary_frame = pending_wider_frame(&mut ordinary, BcTag::Object, &[1], 50);
        let function = ordinary
            .wrap_opaque_value(WiderValue::Function(ordinary_source, 1))
            .unwrap();
        ordinary
            .attach_to_frame(
                &mut ordinary_frame,
                Some(PropertyDisposition::Define(WireKey::Index(7))),
                function,
            )
            .unwrap();
        let ordinary_root = ordinary.finish_frame(ordinary_frame).unwrap();
        assert_eq!(
            ordinary.unwrap_completion(ordinary_root).unwrap(),
            WiderValue::Data(WireValue::Node(NodeId::from_zero_based(0)))
        );
        let ordinary_parts = ordinary.finish().unwrap();
        assert_eq!(
            ordinary_parts.nodes.as_ref(),
            &[WireNodeCarrier::Ordinary {
                properties: Box::from([WirePropertyCarrier {
                    key: WireKey::Index(7),
                    value: WiderValue::Function(ordinary_source, 1),
                }]),
            }]
        );

        let mut array = DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let array_source = array.source();
        let mut array_frame = pending_wider_frame(&mut array, BcTag::Array, &[1], 60);
        let function = array
            .wrap_opaque_value(WiderValue::Function(array_source, 2))
            .unwrap();
        array
            .attach_to_frame(&mut array_frame, None, function)
            .unwrap();
        let array_root = array.finish_frame(array_frame).unwrap();
        assert_eq!(
            array.unwrap_completion(array_root).unwrap(),
            WiderValue::Data(WireValue::Node(NodeId::from_zero_based(0)))
        );
        let array_parts = array.finish().unwrap();
        assert_eq!(
            array_parts.nodes.as_ref(),
            &[WireNodeCarrier::Array {
                elements: Box::from([WiderValue::Function(array_source, 2)]),
            }]
        );

        let mut template = DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let template_source = template.source();
        let mut template_frame =
            pending_wider_frame(&mut template, BcTag::TemplateObject, &[0], 70);
        let function = template
            .wrap_opaque_value(WiderValue::Function(template_source, 3))
            .unwrap();
        template
            .attach_to_frame(&mut template_frame, None, function)
            .unwrap();
        let template_root = template.finish_frame(template_frame).unwrap();
        assert_eq!(
            template.unwrap_completion(template_root).unwrap(),
            WiderValue::Data(WireValue::Node(NodeId::from_zero_based(0)))
        );
        let template_parts = template.finish().unwrap();
        assert_eq!(
            template_parts.nodes.as_ref(),
            &[WireNodeCarrier::TemplateObject {
                elements: Box::new([]),
                raw: WiderValue::Function(template_source, 3),
            }]
        );

        let mut node_machine = DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let node_frame = pending_wider_frame(&mut node_machine, BcTag::Array, &[0], 80);
        let node = node_machine.finish_frame(node_frame).unwrap();
        let node = node_machine.unwrap_completion(node).unwrap();
        assert!(matches!(
            node_machine.wrap_opaque_value(node),
            Err(DecodeError::InvalidCompletionTarget)
        ));
        let node_parts = node_machine.finish().unwrap();
        assert_eq!(
            node_parts.nodes.as_ref(),
            &[WireNodeCarrier::Array {
                elements: Box::new([]),
            }]
        );
    }

    #[test]
    fn shared_data_machine_accepts_wider_values_and_types_opaque_child_errors() {
        let mut primitive_machine =
            DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let mut primitive_cursor =
            WireCursor::new(&[0x54], ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let primitive = primitive_machine
            .read_value_after_tag(&mut primitive_cursor, BcTag::Int32, 12, 0)
            .unwrap();
        let DataReadStep::Complete(primitive) = primitive else {
            panic!("an Int32 must complete without a frame");
        };
        let primitive = primitive_machine.unwrap_completion(primitive).unwrap();
        assert_eq!(primitive, WiderValue::Data(WireValue::Int32(42)));
        primitive_cursor.finish().unwrap();
        let primitive_parts = primitive_machine.finish().unwrap();
        assert!(primitive_parts.nodes.is_empty());
        assert!(primitive_parts.ref_table.is_empty());

        let mut object_machine =
            DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let object_source = object_machine.source();
        let mut empty_cursor = WireCursor::new(&[], ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let object_step = object_machine
            .read_value_after_tag(&mut empty_cursor, BcTag::ObjectValue, 21, 0)
            .unwrap();
        let DataReadStep::Pending(mut object_frame) = object_step else {
            panic!("ObjectValue must wait for its child");
        };
        let function = object_machine
            .wrap_opaque_value(WiderValue::Function(object_source, 1))
            .unwrap();
        object_machine
            .attach_to_frame(&mut object_frame, None, function)
            .unwrap();
        assert!(object_frame.is_complete());
        assert!(matches!(
            object_machine.finish_frame(object_frame),
            Err(DecodeError::OpaqueObjectValue {
                offset: 21,
                value: WiderOpaque::Function(1),
            })
        ));

        let mut date_machine = DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let date_source = date_machine.source();
        let date_step = date_machine
            .read_value_after_tag(&mut empty_cursor, BcTag::Date, 34, 0)
            .unwrap();
        let DataReadStep::Pending(mut date_frame) = date_step else {
            panic!("Date must wait for its child");
        };
        let function = date_machine
            .wrap_opaque_value(WiderValue::Function(date_source, 2))
            .unwrap();
        date_machine
            .attach_to_frame(&mut date_frame, None, function)
            .unwrap();
        assert!(matches!(
            date_machine.finish_frame(date_frame),
            Err(DecodeError::OpaqueDateValue {
                offset: 34,
                value: WiderOpaque::Function(2),
            })
        ));
        empty_cursor.finish().unwrap();

        let typed_body = [TypedArrayKind::Uint8.to_wire_byte(), 1, 0];
        let mut typed_cursor =
            WireCursor::new(&typed_body, ReaderMode::Strict, WIRE_LIMITS).unwrap();
        let mut typed_machine =
            DataMachine::<WiderValue, WireKey>::new(GRAPH_LIMITS, true).unwrap();
        let typed_source = typed_machine.source();
        let typed_step = typed_machine
            .read_value_after_tag(&mut typed_cursor, BcTag::TypedArray, 55, 0)
            .unwrap();
        let DataReadStep::Pending(mut typed_frame) = typed_step else {
            panic!("TypedArray must wait for its backing value");
        };
        let function = typed_machine
            .wrap_opaque_value(WiderValue::Function(typed_source, 3))
            .unwrap();
        typed_machine
            .attach_to_frame(&mut typed_frame, None, function)
            .unwrap();
        assert!(matches!(
            typed_machine.finish_frame(typed_frame),
            Err(DecodeError::OpaqueTypedArrayBacking {
                offset: 55,
                value: WiderOpaque::Function(3),
            })
        ));
        typed_cursor.finish().unwrap();
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
    fn data_atom_namespace_rejects_missing_header_slots_at_the_wire_offset() {
        // The object key consumes ULEB(2), which denotes raw atom one. Data
        // mode has no header atoms here, so the diagnostic position follows
        // pinned QuickJS and points after the consumed atom value.
        assert_eq!(
            decode(&[5, 0, 8, 1, 2], ReaderMode::Strict, false),
            Err(DecodeError::Wire(WireError::InvalidAtomIndex {
                offset: 5,
                index: 1,
                first_atom: 1,
                atom_count: 0,
            }))
        );

        // Index-space validation happens after the header count but before
        // atom-string allocation or payload reads.
        let limits = WireLimits::new(64, u32::MAX, 16, 16);
        assert_eq!(
            decode_graph(
                &[5, 0x80, 0x80, 0x80, 0x80, 0x04],
                ReaderMode::Strict,
                limits,
                GRAPH_LIMITS,
                false,
            ),
            Err(DecodeError::Wire(WireError::AtomIndexSpaceOverflow {
                first_atom: 1,
                atom_count: ATOM_MAX_TABLE_INDEX + 1,
                maximum: ATOM_MAX_TABLE_INDEX,
            }))
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
    fn object_value_primitives_match_pinned_quickjs_vectors() {
        let vectors = [
            (vec![5, 0, 18, 3], WireValue::Bool(false)),
            (vec![5, 0, 18, 4], WireValue::Bool(true)),
            (vec![5, 0, 18, 5, 84], WireValue::Int32(42)),
            (
                vec![5, 0, 18, 6, 0, 0, 0, 0, 0, 0, 0, 128],
                WireValue::Float64Bits((-0.0_f64).to_bits()),
            ),
            (
                vec![5, 0, 18, 6, 0, 0, 0, 0, 0, 0, 248, 127],
                WireValue::Float64Bits(f64::NAN.to_bits()),
            ),
            (
                vec![5, 0, 18, 6, 66, 0, 0, 0, 0, 0, 248, 127],
                WireValue::Float64Bits(0x7ff8_0000_0000_0042),
            ),
            (
                vec![5, 0, 18, 7, 6, b'a', b'b', b'c'],
                WireValue::String(WireString::Narrow(Box::from(*b"abc"))),
            ),
            (
                vec![5, 0, 18, 7, 3, 0, 0xd8],
                WireValue::String(WireString::Wide(Box::from([0xd800]))),
            ),
            (vec![5, 0, 18, 10, 1, 1], WireValue::BigInt(Box::from([1]))),
        ];

        let wrapper = NodeId::from_zero_based(0);
        for (bytes, value) in vectors {
            let primitive = BoxedPrimitive::try_from_wire_value(value).unwrap();
            let graph = decode(&bytes, ReaderMode::Strict, true).unwrap();
            assert_eq!(graph.root, WireValue::Node(wrapper));
            assert_eq!(graph.ref_table.as_ref(), &[wrapper]);
            assert_eq!(
                graph.nodes.as_ref(),
                &[WireNode::ObjectValue {
                    primitive: primitive.clone(),
                }]
            );

            let without_references = decode(&bytes, ReaderMode::Strict, false).unwrap();
            assert!(without_references.ref_table.is_empty());
            assert_eq!(without_references.nodes, graph.nodes);
        }
    }

    #[test]
    fn object_value_reference_aliases_follow_reader_completion_order() {
        let root = NodeId::from_zero_based(0);
        let wrapper = NodeId::from_zero_based(1);

        // Pinned qjs bjson.write([wrapper, wrapper], true).
        let repeated = decode(&[5, 0, 9, 2, 18, 5, 84, 19, 1], ReaderMode::Strict, true).unwrap();
        assert_eq!(repeated.ref_table.as_ref(), &[root, wrapper]);
        assert_eq!(
            repeated.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(wrapper), WireValue::Node(wrapper)]),
            }
        );

        // Without references, the writer expands the wrapper twice and the
        // reader therefore creates two distinct identities.
        let copied = decode(
            &[5, 0, 9, 2, 18, 5, 84, 18, 5, 84],
            ReaderMode::Strict,
            false,
        )
        .unwrap();
        let second_wrapper = NodeId::from_zero_based(2);
        assert!(copied.ref_table.is_empty());
        assert_eq!(
            copied.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(wrapper), WireValue::Node(second_wrapper),]),
            }
        );

        // Reader-only ObjectValue(object) inputs return the child object and
        // append another reference-table entry for the same NodeId.
        let fresh_alias = decode(&[5, 0, 18, 8, 0], ReaderMode::Strict, true).unwrap();
        assert_eq!(fresh_alias.root, WireValue::Node(root));
        assert_eq!(fresh_alias.ref_table.as_ref(), &[root, root]);
        assert_eq!(fresh_alias.nodes.len(), 1);

        let fresh_without_references =
            decode(&[5, 0, 18, 8, 0], ReaderMode::Strict, false).unwrap();
        assert_eq!(fresh_without_references.root, WireValue::Node(root));
        assert!(fresh_without_references.ref_table.is_empty());
        assert_eq!(fresh_without_references.nodes.len(), 1);

        let object = NodeId::from_zero_based(1);
        let array_alias = decode(&[5, 0, 9, 2, 18, 8, 0, 19, 2], ReaderMode::Strict, true).unwrap();
        assert_eq!(array_alias.ref_table.as_ref(), &[root, object, object]);
        assert_eq!(
            array_alias.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(object), WireValue::Node(object)]),
            }
        );

        let buffer = NodeId::from_zero_based(1);
        let buffer_alias = decode(
            &[5, 0, 9, 2, 18, 15, 0, 255, 255, 255, 255, 15, 19, 2],
            ReaderMode::Strict,
            true,
        )
        .unwrap();
        assert_eq!(buffer_alias.ref_table.as_ref(), &[root, buffer, buffer]);
        assert_eq!(
            buffer_alias.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(buffer), WireValue::Node(buffer)]),
            }
        );
        assert_eq!(
            buffer_alias.nodes[buffer.as_usize()],
            WireNode::ArrayBuffer {
                bytes: Box::default(),
                max_byte_length: None,
            }
        );

        // The inner ObjectValue creates the wrapper after its primitive; the
        // outer ObjectValue then aliases that completed wrapper as ref 2.
        let nested = decode(&[5, 0, 9, 2, 18, 18, 5, 2, 19, 2], ReaderMode::Strict, true).unwrap();
        assert_eq!(nested.ref_table.as_ref(), &[root, wrapper, wrapper]);
        assert_eq!(
            nested.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(wrapper), WireValue::Node(wrapper)]),
            }
        );

        // Ordinary and Array identities are real before their children, so an
        // ObjectValue reference can alias a still-open ancestor and form a cycle.
        let pending = decode(&[5, 0, 9, 2, 18, 19, 0, 19, 1], ReaderMode::Strict, true).unwrap();
        assert_eq!(pending.ref_table.as_ref(), &[root, root]);
        assert_eq!(
            pending.nodes.as_ref(),
            &[WireNode::Array {
                elements: Box::from([WireValue::Node(root), WireValue::Node(root)]),
            }]
        );

        let pending_object = decode(
            &[5, 1, 2, b'x', 8, 1, 2, 18, 19, 0],
            ReaderMode::Strict,
            true,
        )
        .unwrap();
        assert_eq!(pending_object.ref_table.as_ref(), &[root, root]);
        assert_eq!(
            pending_object.nodes.as_ref(),
            &[WireNode::Ordinary {
                properties: Box::from([WireProperty {
                    key: WireKey::Atom(AtomId::from_zero_based(0)),
                    value: WireValue::Node(root),
                }]),
            }]
        );
    }

    #[test]
    fn object_value_errors_follow_child_read_and_to_object_order() {
        assert_eq!(
            decode(&[5, 0, 18], ReaderMode::Strict, true),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 3,
                needed: 1,
                remaining: 0,
            }))
        );
        for tag in [BcTag::Null, BcTag::Undefined] {
            assert_eq!(
                decode(&[5, 0, 18, tag.to_byte()], ReaderMode::Strict, true),
                Err(DecodeError::InvalidObjectValue {
                    offset: 2,
                    reason: BoxedPrimitiveError::NullOrUndefined,
                })
            );
        }
        assert_eq!(
            decode(&[5, 0, 18, 19, 0], ReaderMode::Strict, false),
            Err(DecodeError::ObjectReferencesNotAllowed { offset: 3 })
        );
        assert_eq!(
            decode(&[5, 0, 18, 19, 0], ReaderMode::Strict, true),
            Err(DecodeError::Graph(GraphError::InvalidReferenceIndex {
                index: 0,
                reference_count: 0,
            }))
        );
        assert_eq!(
            decode(&[5, 0, 18, 16], ReaderMode::Strict, true),
            Err(DecodeError::UnsupportedTag {
                tag: BcTag::SharedArrayBuffer,
                offset: 3,
            })
        );

        // Wrapper node/reference reservations happen only after the child is
        // complete, so child wire/reference failures win even with no identity
        // budget available.
        let no_identity = GraphLimits::new(0, 0, 8, 8, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 18],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_identity,
                true,
            ),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 3,
                needed: 1,
                remaining: 0,
            }))
        );
        assert_eq!(
            decode_graph(
                &[5, 0, 18, 19, 0],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_identity,
                true,
            ),
            Err(DecodeError::Graph(GraphError::InvalidReferenceIndex {
                index: 0,
                reference_count: 0,
            }))
        );

        // A pending TypedArray reference denotes QuickJS's temporary NULL
        // placeholder, not a real object identity. Reject it without following
        // the pinned native crash path.
        let typed_array = NodeId::from_zero_based(0);
        assert_eq!(
            decode(&[5, 0, 14, 2, 0, 0, 18, 19, 0], ReaderMode::Strict, true,),
            Err(DecodeError::InvalidObjectValueAlias {
                offset: 6,
                node: typed_array,
            })
        );
    }

    #[test]
    fn object_value_identity_alias_and_depth_budgets_are_independent() {
        let primitive = [5, 0, 18, 5, 2];
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
                decode_graph(
                    &primitive,
                    ReaderMode::Strict,
                    WIRE_LIMITS,
                    limits,
                    allow_references,
                ),
                Err(DecodeError::Graph(expected))
            );
        }

        let one_level = GraphLimits::new(8, 8, 1, 8, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 18, 18, 5, 2],
                ReaderMode::Strict,
                WIRE_LIMITS,
                one_level,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::NestingDepth,
                requested: 2,
                limit: 1,
            }))
        );

        // The root Array already consumed the only node and reference slot.
        // Its ObjectValue child aliases that node, so only the reference budget
        // grows after the child reference has been read.
        let one_alias = GraphLimits::new(1, 1, 2, 1, 1, 0, 0, 0, 0);
        assert_eq!(
            decode_graph(
                &[5, 0, 9, 1, 18, 19, 0],
                ReaderMode::Strict,
                WIRE_LIMITS,
                one_alias,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ObjectReferences,
                requested: 2,
                limit: 1,
            }))
        );

        // A ready ArrayBuffer child consumes its own identity before
        // ObjectValue appends the alias entry.
        let ready_leaf_alias = GraphLimits::new(2, 2, 3, 1, 1, 0, 0, 0, 0);
        assert_eq!(
            decode_graph(
                &[5, 0, 9, 1, 18, 15, 0, 255, 255, 255, 255, 15],
                ReaderMode::Strict,
                WIRE_LIMITS,
                ready_leaf_alias,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ObjectReferences,
                requested: 3,
                limit: 2,
            }))
        );
    }

    #[test]
    fn date_numbers_match_pinned_quickjs_and_preserve_reader_only_bits() {
        let vectors = [
            (vec![5, 0, 17, 5, 0], WireValue::Int32(0)),
            (vec![5, 0, 17, 5, 84], WireValue::Int32(42)),
            (vec![5, 0, 17, 5, 1], WireValue::Int32(-1)),
            (
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 69, 64],
                WireValue::Float64Bits(42.0_f64.to_bits()),
            ),
            (
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 0, 128],
                WireValue::Float64Bits((-0.0_f64).to_bits()),
            ),
            (
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 240, 127],
                WireValue::Float64Bits(f64::INFINITY.to_bits()),
            ),
            (
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 240, 255],
                WireValue::Float64Bits(f64::NEG_INFINITY.to_bits()),
            ),
            (
                vec![5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 248, 127],
                WireValue::Float64Bits(f64::NAN.to_bits()),
            ),
            (
                vec![5, 0, 17, 6, 66, 0, 0, 0, 0, 0, 248, 127],
                WireValue::Float64Bits(0x7ff8_0000_0000_0042),
            ),
            (
                vec![5, 0, 17, 6, 1, 0, 0, 0, 0, 0, 240, 127],
                WireValue::Float64Bits(0x7ff0_0000_0000_0001),
            ),
            (
                vec![5, 0, 17, 6, 1, 0, 0, 0, 0, 0, 0, 0],
                WireValue::Float64Bits(1),
            ),
        ];

        let date = NodeId::from_zero_based(0);
        for (bytes, value) in vectors {
            let time_value = DateNumber::try_from_wire_value(value).unwrap();
            let graph = decode(&bytes, ReaderMode::Strict, true).unwrap();
            assert_eq!(graph.root, WireValue::Node(date));
            assert_eq!(graph.ref_table.as_ref(), &[date]);
            assert_eq!(
                graph.nodes.as_ref(),
                &[WireNode::Date {
                    time_value: time_value.clone(),
                }]
            );

            let without_references = decode(&bytes, ReaderMode::Strict, false).unwrap();
            assert!(without_references.ref_table.is_empty());
            assert_eq!(without_references.nodes, graph.nodes);
        }
    }

    #[test]
    fn date_identity_and_object_value_aliases_follow_completion_order() {
        let root = NodeId::from_zero_based(0);
        let date = NodeId::from_zero_based(1);
        let repeated = decode(&[5, 0, 9, 2, 17, 5, 84, 19, 1], ReaderMode::Strict, true).unwrap();
        assert_eq!(repeated.ref_table.as_ref(), &[root, date]);
        assert_eq!(
            repeated.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(date), WireValue::Node(date)]),
            }
        );

        let copied = decode(
            &[5, 0, 9, 2, 17, 5, 84, 17, 5, 84],
            ReaderMode::Strict,
            false,
        )
        .unwrap();
        assert!(copied.ref_table.is_empty());
        assert_eq!(
            copied.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([
                    WireValue::Node(date),
                    WireValue::Node(NodeId::from_zero_based(2)),
                ]),
            }
        );

        let aliased = decode(
            &[5, 0, 9, 2, 18, 17, 5, 84, 19, 2],
            ReaderMode::Strict,
            true,
        )
        .unwrap();
        assert_eq!(aliased.ref_table.as_ref(), &[root, date, date]);
        assert_eq!(
            aliased.nodes[root.as_usize()],
            WireNode::Array {
                elements: Box::from([WireValue::Node(date), WireValue::Node(date)]),
            }
        );
    }

    #[test]
    fn date_errors_follow_complete_child_then_number_validation_order() {
        assert_eq!(
            decode(&[5, 0, 17], ReaderMode::Strict, true),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 3,
                needed: 1,
                remaining: 0,
            }))
        );
        assert_eq!(
            decode(&[5, 0, 17, 6, 0, 0], ReaderMode::Strict, true),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 4,
                needed: 8,
                remaining: 2,
            }))
        );

        let non_numbers: &[&[u8]] = &[
            &[5, 0, 17, 1],
            &[5, 0, 17, 2],
            &[5, 0, 17, 3],
            &[5, 0, 17, 7, 0],
            &[5, 0, 17, 10, 0],
            &[5, 0, 17, 8, 0],
            &[5, 0, 17, 9, 0],
            &[5, 0, 17, 15, 0, 255, 255, 255, 255, 15],
            &[5, 0, 17, 17, 5, 0],
            &[5, 0, 17, 18, 5, 0],
        ];
        for bytes in non_numbers {
            assert_eq!(
                decode(bytes, ReaderMode::Strict, true),
                Err(DecodeError::InvalidDate {
                    offset: 2,
                    reason: DateNumberError::NotNumber,
                })
            );
        }

        assert_eq!(
            decode(&[5, 0, 17, 19, 0], ReaderMode::Strict, false),
            Err(DecodeError::ObjectReferencesNotAllowed { offset: 3 })
        );
        assert_eq!(
            decode(&[5, 0, 17, 19, 0], ReaderMode::Strict, true),
            Err(DecodeError::Graph(GraphError::InvalidReferenceIndex {
                index: 0,
                reference_count: 0,
            }))
        );
        assert_eq!(
            decode(&[5, 0, 17, 16], ReaderMode::Strict, true),
            Err(DecodeError::UnsupportedTag {
                tag: BcTag::SharedArrayBuffer,
                offset: 3,
            })
        );

        let no_bigint = GraphLimits::new(8, 8, 8, 8, 8, 0, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 17, 10, 1, 1],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_bigint,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::BigIntBytes,
                requested: 1,
                limit: 0,
            }))
        );

        // The root Array is already reference 0, but a Date never aliases its
        // object child: number validation still fails after resolving it.
        assert_eq!(
            decode(&[5, 0, 9, 1, 17, 19, 0], ReaderMode::Strict, true),
            Err(DecodeError::InvalidDate {
                offset: 4,
                reason: DateNumberError::NotNumber,
            })
        );

        // Pinned QuickJS represents a TypedArray under construction with a
        // temporary NULL entry. Date performs only number validation, so Rust
        // rejects the placeholder deterministically instead of entering the
        // native crash path.
        assert_eq!(
            decode(&[5, 0, 14, 0, 0, 0, 17, 19, 0], ReaderMode::Strict, true),
            Err(DecodeError::InvalidDate {
                offset: 6,
                reason: DateNumberError::NotNumber,
            })
        );

        let no_identity = GraphLimits::new(0, 0, 8, 8, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 17],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_identity,
                true,
            ),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 3,
                needed: 1,
                remaining: 0,
            }))
        );

        let two_references = GraphLimits::new(2, 2, 3, 8, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 9, 2, 18, 17, 5, 84, 19, 2],
                ReaderMode::Strict,
                WIRE_LIMITS,
                two_references,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ObjectReferences,
                requested: 3,
                limit: 2,
            }))
        );
        assert_eq!(
            decode_graph(
                &[5, 0, 17, 3],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_identity,
                true,
            ),
            Err(DecodeError::InvalidDate {
                offset: 2,
                reason: DateNumberError::NotNumber,
            })
        );
        assert_eq!(
            decode_graph(
                &[5, 0, 17, 8, 0],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_identity,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::Nodes,
                requested: 1,
                limit: 0,
            }))
        );
    }

    #[test]
    fn date_identity_and_depth_budgets_apply_after_numeric_validation() {
        let number = [5, 0, 17, 5, 2];
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
                decode_graph(
                    &number,
                    ReaderMode::Strict,
                    WIRE_LIMITS,
                    limits,
                    allow_references,
                ),
                Err(DecodeError::Graph(expected))
            );
        }

        let no_references = GraphLimits::new(1, 0, 1, 8, 8, 8, 8, 8, 8);
        assert!(
            decode_graph(
                &number,
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_references,
                false,
            )
            .is_ok()
        );

        let one_level = GraphLimits::new(8, 8, 1, 8, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 17, 17, 5, 2],
                ReaderMode::Strict,
                WIRE_LIMITS,
                one_level,
                true,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::NestingDepth,
                requested: 2,
                limit: 1,
            }))
        );
    }

    #[test]
    fn template_objects_decode_pinned_quickjs_vectors_and_required_raw_child() {
        // Pinned QuickJS accepts TEMPLATE_OBJECT in the reader even without
        // the bytecode flag. The payload is a dense element sequence followed
        // by exactly one raw child. Undefined is still consumed here even
        // though QuickJS then omits the observable `.raw` property.
        let empty = decode(&[5, 0, 11, 0, 2], ReaderMode::Strict, false).unwrap();
        let root = NodeId::from_zero_based(0);
        assert_eq!(empty.root, WireValue::Node(root));
        assert_eq!(
            empty.nodes.as_ref(),
            &[WireNode::TemplateObject {
                elements: Box::default(),
                raw: WireValue::Undefined,
            }]
        );

        let populated = decode(
            &[5, 0, 11, 2, 5, 2, 7, 2, b'x', 2],
            ReaderMode::Strict,
            false,
        )
        .unwrap();
        assert_eq!(
            populated.nodes.as_ref(),
            &[WireNode::TemplateObject {
                elements: Box::from([
                    WireValue::Int32(1),
                    WireValue::String(WireString::Narrow(Box::from(*b"x"))),
                ]),
                raw: WireValue::Undefined,
            }]
        );

        // Element count zero does not complete the frame: raw remains a
        // mandatory child on the wire.
        assert_eq!(
            decode(&[5, 0, 11, 0], ReaderMode::Strict, false),
            Err(DecodeError::Wire(WireError::Truncated {
                offset: 4,
                needed: 1,
                remaining: 0,
            }))
        );
    }

    #[test]
    fn template_identity_is_registered_before_elements_and_raw() {
        let root = NodeId::from_zero_based(0);
        for (bytes, expected_elements, expected_raw) in [
            (
                &[5, 0, 11, 1, 19, 0, 2][..],
                Box::from([WireValue::Node(root)]),
                WireValue::Undefined,
            ),
            (
                &[5, 0, 11, 0, 19, 0][..],
                Box::default(),
                WireValue::Node(root),
            ),
        ] {
            let graph = decode(bytes, ReaderMode::Strict, true).unwrap();
            assert_eq!(graph.ref_table.as_ref(), &[root]);
            assert_eq!(
                graph.nodes.as_ref(),
                &[WireNode::TemplateObject {
                    elements: expected_elements,
                    raw: expected_raw,
                }]
            );
        }

        // A TypedArray sees the still-open template as an Array-class object,
        // not the temporary NULL identity used for a pending TypedArray.
        assert_eq!(
            decode(
                &[5, 0, 11, 1, 14, 2, 0, 0, 19, 0, 2],
                ReaderMode::Strict,
                true,
            ),
            Err(DecodeError::InvalidTypedArrayBacking {
                offset: 4,
                reason: TypedArrayBackingError::NotArrayBuffer { node: root },
            })
        );
    }

    #[test]
    fn template_elements_use_container_budgets_but_raw_is_a_fixed_child() {
        let two_elements = [5, 0, 11, 2, 2, 2, 2];
        let per_container = GraphLimits::new(8, 8, 8, 1, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &two_elements,
                ReaderMode::Strict,
                WIRE_LIMITS,
                per_container,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ContainerEntries,
                requested: 2,
                limit: 1,
            }))
        );

        let aggregate = GraphLimits::new(8, 8, 8, 8, 1, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &two_elements,
                ReaderMode::Strict,
                WIRE_LIMITS,
                aggregate,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::TotalContainerEntries,
                requested: 2,
                limit: 1,
            }))
        );

        let no_entries = GraphLimits::new(2, 0, 2, 0, 0, 8, 8, 8, 8);
        assert!(
            decode_graph(
                &[5, 0, 11, 0, 2],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_entries,
                false,
            )
            .is_ok()
        );

        // The fixed raw slot itself is not a container entry. A container
        // stored in that slot still contributes its own element work.
        assert_eq!(
            decode_graph(
                &[5, 0, 11, 0, 9, 1, 2],
                ReaderMode::Strict,
                WIRE_LIMITS,
                no_entries,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::ContainerEntries,
                requested: 1,
                limit: 0,
            }))
        );

        let one_level = GraphLimits::new(2, 0, 1, 8, 8, 8, 8, 8, 8);
        assert_eq!(
            decode_graph(
                &[5, 0, 11, 0, 9, 0],
                ReaderMode::Strict,
                WIRE_LIMITS,
                one_level,
                false,
            ),
            Err(DecodeError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::NestingDepth,
                requested: 2,
                limit: 1,
            }))
        );
    }

    #[test]
    fn unsupported_data_tags_are_rejected_before_their_payloads() {
        for tag in [
            BcTag::FunctionBytecode,
            BcTag::Module,
            BcTag::SharedArrayBuffer,
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
