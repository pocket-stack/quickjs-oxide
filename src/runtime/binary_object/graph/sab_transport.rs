//! Pointer-free archive identities for BC5 SharedArrayBuffer records.
//!
//! Native QuickJS writes a process-local backing pointer into every complete
//! SharedArrayBuffer record. The decoder state below may compare those integer
//! bits while one input is being traversed, but neither the raw bits nor a
//! runtime backing capability can enter the completed archive model.

use std::collections::HashMap;
use std::fmt;

use super::super::bytecode_image::{
    BytecodeImage, BytecodeImageError, BytecodeImageLimits, decode_bytecode_image_body,
};
use super::super::wire::{
    BcTag, BinaryObjectHeader, ReaderMode, WireCursor, WireError, WireLimits, WireString,
};
use super::decode::{DecodeError, decode_graph_body, map_sab_archive_error};
use super::model::{
    ArrayBufferLayoutError, GraphError, GraphLimits, GraphResourceKind, WireGraph, WireNode,
    validate_array_buffer_layout,
};

/// Zero-based identity of one shared backing inside a single archived graph.
///
/// This is not a process backing-store identity and is never derived from the
/// numeric value of QuickJS's pointer token. Construction remains private to
/// this module so only [`SabArchiveState`] can allocate canonical identities in
/// first-record encounter order.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct ArchiveBackingId(u32);

impl ArchiveBackingId {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn zero_based(self) -> u32 {
        self.0
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Debug for ArchiveBackingId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("ArchiveBackingId")
            .field(&self.0)
            .finish()
    }
}

/// One decoded SAB node projected to the fields needed by archive
/// finalization. The projection lets the same finalizer authenticate graph and
/// whole-image models without widening either model's private carrier types.
#[derive(Clone, Copy)]
pub(in crate::runtime::binary_object) struct SabArchiveOccurrence {
    byte_length: u32,
    max_byte_length: Option<u32>,
    backing: ArchiveBackingId,
}

impl SabArchiveOccurrence {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn new(
        byte_length: u32,
        max_byte_length: Option<u32>,
        backing: ArchiveBackingId,
    ) -> Self {
        Self {
            byte_length,
            max_byte_length,
            backing,
        }
    }
}

/// Untrusted native BC5 token retained only while one decoder is active.
///
/// Deliberately do not implement `Debug` or `Display`: diagnostics must report
/// offsets and archive-local ordinals, never address-shaped input bits.
#[derive(Clone, Copy, Eq, Hash, PartialEq)]
struct RawSabToken(u64);

impl RawSabToken {
    /// Immediately seal integer bits returned by the fixed-width wire reader.
    /// No inverse conversion is provided.
    #[must_use]
    const fn from_wire_bits(bits: u64) -> Self {
        Self(bits)
    }
}

/// One typed entry retained from QuickJS writer's SAB occurrence side table.
///
/// This is a comparison token, not a pointer or a runtime backing capability.
/// Deliberately do not implement `Debug`, `Display`, or an integer getter: the
/// process-local bits may be checked while decoding one transport, but must not
/// escape through archive state or diagnostics.
pub(in crate::runtime) struct NativeSabToken {
    native_token_bits: u64,
}

#[cfg(test)]
impl NativeSabToken {
    /// Seal test-only integer bits until a dedicated native host bridge owns
    /// the production constructor in a later milestone.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn from_test_bits(bits: u64) -> Self {
        Self {
            native_token_bits: bits,
        }
    }
}

/// Inseparable borrowed view of one QuickJS SAB transport message.
///
/// QuickJS's writer emits one side-table entry for every complete SAB record,
/// including repeated entries when object references are disabled. Keeping the
/// bytes and occurrence table in one input prevents the decoder from exposing
/// either half through a splitting accessor.
pub(in crate::runtime) struct SabTransportInput<'a> {
    transport_wire_bytes: &'a [u8],
    transport_writer_occurrences: &'a [NativeSabToken],
}

impl<'a> SabTransportInput<'a> {
    #[must_use]
    pub(in crate::runtime) const fn new(
        wire: &'a [u8],
        writer_occurrences: &'a [NativeSabToken],
    ) -> Self {
        Self {
            transport_wire_bytes: wire,
            transport_writer_occurrences: writer_occurrences,
        }
    }

    fn build_cursor(
        self,
        mode: ReaderMode,
        wire_limits: WireLimits,
        graph_limits: GraphLimits,
    ) -> Result<SabTransportCursor<'a>, SabArchiveError> {
        Ok(SabTransportCursor {
            cursor_wire: WireCursor::new(self.transport_wire_bytes, mode, wire_limits)?,
            cursor_writer_occurrences: self.transport_writer_occurrences,
            cursor_next_occurrence: 0,
            cursor_archive: SabArchiveState::new(graph_limits),
        })
    }

    #[cfg(test)]
    fn into_cursor_for_test(
        self,
        mode: ReaderMode,
        wire_limits: WireLimits,
        graph_limits: GraphLimits,
    ) -> Result<SabTransportCursor<'a>, SabArchiveError> {
        self.build_cursor(mode, wire_limits, graph_limits)
    }
}

/// Decode one inseparable QuickJS SAB transport into a pointer-free graph
/// archive. Cursor creation and finalization remain owned by this module, so no
/// partially authenticated transport state can escape to another decoder.
pub(in crate::runtime) fn decode_graph_with_sab_transport(
    input: SabTransportInput<'_>,
    mode: ReaderMode,
    wire_limits: WireLimits,
    graph_limits: GraphLimits,
    allow_object_references: bool,
) -> Result<ArchivedWireGraph, DecodeError> {
    let cursor = input
        .build_cursor(mode, wire_limits, graph_limits)
        .map_err(map_sab_archive_error)?;
    let (cursor, graph) = decode_graph_body(cursor, graph_limits, allow_object_references)?;
    cursor
        .finish_graph_archive(graph)
        .map_err(map_sab_archive_error)
}

/// Decode one complete bytecode-mode BC5 image together with its ordered SAB
/// writer side table, returning one inseparable pointer-free archive.
pub(in crate::runtime) fn decode_bytecode_image_with_sab_transport(
    input: SabTransportInput<'_>,
    mode: ReaderMode,
    wire_limits: WireLimits,
    limits: BytecodeImageLimits,
    allow_object_references: bool,
) -> Result<ArchivedBytecodeImage, BytecodeImageError> {
    let cursor = input.build_cursor(mode, wire_limits, limits.graph())?;
    let (cursor, image) = decode_bytecode_image_body(cursor, limits, allow_object_references)?;
    cursor.finish_bytecode_image(image).map_err(Into::into)
}

/// Checked wire and side-table cursor for one SAB-aware decode.
///
/// There is no `Debug`, side-table accessor, bare [`WireCursor`] accessor, or
/// consuming finalizer for only one half. A non-consuming wire-end check exists
/// solely to preserve whole-image diagnostic order; it neither validates the
/// occurrence-table cardinality nor publishes a result. All ordinary reads
/// delegate through this type, while the fixed-width SAB token is interpreted
/// only by the checked record operation below.
pub(in crate::runtime::binary_object) struct SabTransportCursor<'a> {
    cursor_wire: WireCursor<'a>,
    cursor_writer_occurrences: &'a [NativeSabToken],
    cursor_next_occurrence: usize,
    cursor_archive: SabArchiveState,
}

impl<'a> SabTransportCursor<'a> {
    #[must_use]
    pub(in crate::runtime::binary_object) const fn position(&self) -> usize {
        self.cursor_wire.position()
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn mode(&self) -> ReaderMode {
        self.cursor_wire.mode()
    }

    #[must_use]
    pub(in crate::runtime::binary_object) fn remaining(&self) -> usize {
        self.cursor_wire.remaining()
    }

    pub(in crate::runtime::binary_object) fn read_u8(&mut self) -> Result<u8, WireError> {
        self.cursor_wire.read_u8()
    }

    pub(in crate::runtime::binary_object) fn read_u16_le(&mut self) -> Result<u16, WireError> {
        self.cursor_wire.read_u16_le()
    }

    pub(in crate::runtime::binary_object) fn read_bytes(
        &mut self,
        length: usize,
    ) -> Result<&'a [u8], WireError> {
        self.cursor_wire.read_bytes(length)
    }

    pub(in crate::runtime::binary_object) fn read_tag(&mut self) -> Result<BcTag, WireError> {
        self.cursor_wire.read_tag()
    }

    pub(in crate::runtime::binary_object) fn read_uleb128(&mut self) -> Result<u32, WireError> {
        self.cursor_wire.read_uleb128()
    }

    pub(in crate::runtime::binary_object) fn read_i32(&mut self) -> Result<i32, WireError> {
        self.cursor_wire.read_i32()
    }

    pub(in crate::runtime::binary_object) fn read_f64(&mut self) -> Result<f64, WireError> {
        self.cursor_wire.read_f64()
    }

    pub(in crate::runtime::binary_object) fn read_header(
        &mut self,
    ) -> Result<BinaryObjectHeader, WireError> {
        self.cursor_wire.read_header()
    }

    pub(in crate::runtime::binary_object) fn read_string(
        &mut self,
    ) -> Result<WireString, WireError> {
        self.cursor_wire.read_string()
    }

    pub(in crate::runtime::binary_object) fn validate_wire_end(&self) -> Result<(), WireError> {
        self.cursor_wire.validate_wire_end()
    }

    /// Consume and authenticate one complete SAB record's fixed-width token.
    ///
    /// The caller performs QuickJS's immediate `max < current` check first.
    /// The complete eight-byte token and matching writer side-table entry are
    /// then proved before remaining layout and resource validation, so a
    /// truncated or mismatched transport cannot allocate archive identities.
    pub(super) fn record_shared_array_buffer(
        &mut self,
        byte_length: u32,
        max_byte_length: Option<u32>,
    ) -> Result<ArchiveBackingId, SabArchiveError> {
        let offset = self.cursor_wire.position();
        let raw_token = RawSabToken::from_wire_bits(self.cursor_wire.read_u64_le()?);
        let ordinal = self.cursor_next_occurrence;
        let Some(expected) = self.cursor_writer_occurrences.get(ordinal) else {
            return Err(SabArchiveError::SideTableTooShort {
                offset,
                ordinal,
                entry_count: self.cursor_writer_occurrences.len(),
            });
        };
        if raw_token.0 != expected.native_token_bits {
            return Err(SabArchiveError::SideTableTokenMismatch { offset, ordinal });
        }

        let descriptor =
            SharedBackingDescriptor::from_wrapper_layout(byte_length, max_byte_length)?;
        let backing = self
            .cursor_archive
            .record_validated(raw_token, descriptor)?;
        self.cursor_next_occurrence = ordinal + 1;
        Ok(backing)
    }

    /// Finish both transport halves and atomically bind the graph to SAB state.
    ///
    /// Extra writer entries are rejected even in QuickJS-compatible wire mode:
    /// they describe retained native occurrences which the decoded value did
    /// not consume and therefore cannot be silently ignored.
    fn finish_shared_backings<I>(
        self,
        occurrences: I,
    ) -> Result<Box<[SharedBackingDescriptor]>, SabArchiveError>
    where
        I: IntoIterator<Item = SabArchiveOccurrence>,
    {
        self.cursor_wire.finish()?;
        if self.cursor_next_occurrence != self.cursor_writer_occurrences.len() {
            return Err(SabArchiveError::SideTableHasExtra {
                consumed: self.cursor_next_occurrence,
                entry_count: self.cursor_writer_occurrences.len(),
            });
        }
        self.cursor_archive.finish(occurrences)
    }

    fn finish_graph_archive(self, graph: WireGraph) -> Result<ArchivedWireGraph, SabArchiveError> {
        let occurrences = graph.nodes.iter().filter_map(|node| match node {
            WireNode::SharedArrayBuffer {
                byte_length,
                max_byte_length,
                backing,
            } => Some(SabArchiveOccurrence::new(
                *byte_length,
                *max_byte_length,
                *backing,
            )),
            _ => None,
        });
        let shared_backings = self.finish_shared_backings(occurrences)?;
        Ok(ArchivedWireGraph {
            archived_graph_payload: graph,
            archived_graph_shared_backings: shared_backings,
        })
    }

    #[cfg(test)]
    fn finish_graph_archive_for_test(
        self,
        graph: WireGraph,
    ) -> Result<ArchivedWireGraph, SabArchiveError> {
        self.finish_graph_archive(graph)
    }

    /// Finish both transport halves and atomically bind a whole bytecode image
    /// to the SAB descriptors authenticated while traversing that same image.
    fn finish_bytecode_image(
        self,
        image: BytecodeImage,
    ) -> Result<ArchivedBytecodeImage, SabArchiveError> {
        let shared_backings = self.finish_shared_backings(image.sab_archive_occurrences())?;
        Ok(ArchivedBytecodeImage {
            archived_image_payload: image,
            archived_image_shared_backings: shared_backings,
        })
    }
}

/// Backing-wide layout inferred from one or more SAB wrapper records.
///
/// Current byte length remains wrapper-local on each graph node. The backing's
/// committed capacity and fixed/growable class must agree across every record
/// which carried the same native token.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct SharedBackingDescriptor {
    capacity: u32,
    growable: bool,
}

impl SharedBackingDescriptor {
    fn from_wrapper_layout(
        byte_length: u32,
        max_byte_length: Option<u32>,
    ) -> Result<Self, SabArchiveError> {
        validate_array_buffer_layout(byte_length as usize, max_byte_length)
            .map_err(SabArchiveError::InvalidLayout)?;
        Ok(Self {
            capacity: max_byte_length.unwrap_or(byte_length),
            growable: max_byte_length.is_some(),
        })
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn capacity(self) -> u32 {
        self.capacity
    }

    #[must_use]
    pub(in crate::runtime::binary_object) const fn is_growable(self) -> bool {
        self.growable
    }
}

/// Failure while canonicalizing native SAB records into a pointer-free graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum SabArchiveError {
    Wire(WireError),
    Graph(GraphError),
    InvalidLayout(ArrayBufferLayoutError),
    SideTableTooShort {
        offset: usize,
        ordinal: usize,
        entry_count: usize,
    },
    SideTableTokenMismatch {
        offset: usize,
        ordinal: usize,
    },
    SideTableHasExtra {
        consumed: usize,
        entry_count: usize,
    },
    ConflictingBackingDescriptor {
        backing: ArchiveBackingId,
    },
    InvalidBackingIndex {
        backing: ArchiveBackingId,
        backing_count: usize,
    },
    MissingBackingDescriptor {
        backing: ArchiveBackingId,
    },
    OccurrenceCountMismatch {
        recorded: usize,
        archive_nodes: usize,
    },
}

impl From<WireError> for SabArchiveError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<GraphError> for SabArchiveError {
    fn from(error: GraphError) -> Self {
        Self::Graph(error)
    }
}

impl fmt::Display for SabArchiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Graph(error) => fmt::Display::fmt(error, formatter),
            Self::InvalidLayout(error) => {
                write!(formatter, "invalid SharedArrayBuffer layout: {error}")
            }
            Self::SideTableTooShort {
                offset,
                ordinal,
                entry_count,
            } => write!(
                formatter,
                "SharedArrayBuffer token at byte {offset} has occurrence ordinal {ordinal}, but the writer side table has {entry_count} entries"
            ),
            Self::SideTableTokenMismatch { offset, ordinal } => write!(
                formatter,
                "SharedArrayBuffer token at byte {offset} does not match writer side-table occurrence {ordinal}"
            ),
            Self::SideTableHasExtra {
                consumed,
                entry_count,
            } => write!(
                formatter,
                "decoded {consumed} SharedArrayBuffer occurrences, but the writer side table has {entry_count} entries"
            ),
            Self::ConflictingBackingDescriptor { backing } => write!(
                formatter,
                "SharedArrayBuffer backing {} has conflicting capacity or growability",
                backing.zero_based()
            ),
            Self::InvalidBackingIndex {
                backing,
                backing_count,
            } => write!(
                formatter,
                "invalid SharedArrayBuffer backing index {} for {backing_count} archived backings",
                backing.zero_based()
            ),
            Self::MissingBackingDescriptor { backing } => write!(
                formatter,
                "SharedArrayBuffer backing {} has no archive node",
                backing.zero_based()
            ),
            Self::OccurrenceCountMismatch {
                recorded,
                archive_nodes,
            } => write!(
                formatter,
                "SharedArrayBuffer decoder recorded {recorded} occurrences but the archive contains {archive_nodes} SAB nodes"
            ),
        }
    }
}

impl std::error::Error for SabArchiveError {}

/// Decoder-local canonicalization state for native SharedArrayBuffer records.
///
/// There is intentionally no `Debug` implementation. A complete record is
/// charged before its raw token is interned. New backing capacity is charged
/// once, while repeated full records consume only the occurrence budget.
struct SabArchiveState {
    limits: GraphLimits,
    raw_to_backing: HashMap<RawSabToken, ArchiveBackingId>,
    shared_backings: Vec<SharedBackingDescriptor>,
    occurrences: usize,
    total_backing_capacity: usize,
}

impl SabArchiveState {
    #[must_use]
    fn new(limits: GraphLimits) -> Self {
        Self {
            limits,
            raw_to_backing: HashMap::new(),
            shared_backings: Vec::new(),
            occurrences: 0,
            total_backing_capacity: 0,
        }
    }

    /// Record one fully parsed SAB record and return its archive-local backing.
    ///
    /// Callers must perform the pinned `max < current` diagnostic immediately
    /// after reading both lengths, then read the complete eight-byte token
    /// before invoking this method. No state is committed until layout, limits,
    /// identity space, and both fallible allocations have succeeded.
    fn record_validated(
        &mut self,
        raw_token: RawSabToken,
        descriptor: SharedBackingDescriptor,
    ) -> Result<ArchiveBackingId, SabArchiveError> {
        let requested_occurrences =
            self.occurrences
                .checked_add(1)
                .ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::SharedArrayBufferOccurrences,
                })?;
        self.limits.check(
            GraphResourceKind::SharedArrayBufferOccurrences,
            requested_occurrences,
        )?;

        if let Some(backing) = self.raw_to_backing.get(&raw_token).copied() {
            let archived = self.shared_backings.get(backing.as_usize()).ok_or(
                SabArchiveError::InvalidBackingIndex {
                    backing,
                    backing_count: self.shared_backings.len(),
                },
            )?;
            if *archived != descriptor {
                return Err(SabArchiveError::ConflictingBackingDescriptor { backing });
            }
            self.occurrences = requested_occurrences;
            return Ok(backing);
        }

        let requested_backings =
            self.shared_backings
                .len()
                .checked_add(1)
                .ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::SharedArrayBufferBackings,
                })?;
        self.limits.check(
            GraphResourceKind::SharedArrayBufferBackings,
            requested_backings,
        )?;

        let capacity = descriptor.capacity() as usize;
        self.limits
            .check(GraphResourceKind::SharedArrayBufferCapacity, capacity)?;
        let requested_total =
            self.total_backing_capacity
                .checked_add(capacity)
                .ok_or(GraphError::CountOverflow {
                    kind: GraphResourceKind::TotalSharedArrayBufferCapacity,
                })?;
        self.limits.check(
            GraphResourceKind::TotalSharedArrayBufferCapacity,
            requested_total,
        )?;

        let backing_index =
            u32::try_from(self.shared_backings.len()).map_err(|_| GraphError::CountOverflow {
                kind: GraphResourceKind::SharedArrayBufferBackings,
            })?;
        self.raw_to_backing
            .try_reserve(1)
            .map_err(|_| GraphError::AllocationFailed)?;
        self.shared_backings
            .try_reserve(1)
            .map_err(|_| GraphError::AllocationFailed)?;

        let backing = ArchiveBackingId(backing_index);
        self.shared_backings.push(descriptor);
        let previous = self.raw_to_backing.insert(raw_token, backing);
        debug_assert!(previous.is_none());
        self.occurrences = requested_occurrences;
        self.total_backing_capacity = requested_total;
        Ok(backing)
    }

    /// Validate a completed model's SAB projection and return the decoder-owned
    /// descriptor table to the private typed binder which supplied it.
    fn finish<I>(self, occurrences: I) -> Result<Box<[SharedBackingDescriptor]>, SabArchiveError>
    where
        I: IntoIterator<Item = SabArchiveOccurrence>,
    {
        let mut seen = Vec::new();
        seen.try_reserve_exact(self.shared_backings.len())
            .map_err(|_| GraphError::AllocationFailed)?;
        seen.resize(self.shared_backings.len(), false);

        let mut archive_occurrences = 0_usize;
        for SabArchiveOccurrence {
            byte_length,
            max_byte_length,
            backing,
        } in occurrences
        {
            archive_occurrences =
                archive_occurrences
                    .checked_add(1)
                    .ok_or(GraphError::CountOverflow {
                        kind: GraphResourceKind::SharedArrayBufferOccurrences,
                    })?;
            let descriptor = self.shared_backings.get(backing.as_usize()).ok_or(
                SabArchiveError::InvalidBackingIndex {
                    backing,
                    backing_count: self.shared_backings.len(),
                },
            )?;
            let node_descriptor =
                SharedBackingDescriptor::from_wrapper_layout(byte_length, max_byte_length)?;
            if *descriptor != node_descriptor {
                return Err(SabArchiveError::ConflictingBackingDescriptor { backing });
            }
            seen[backing.as_usize()] = true;
        }

        if archive_occurrences != self.occurrences {
            return Err(SabArchiveError::OccurrenceCountMismatch {
                recorded: self.occurrences,
                archive_nodes: archive_occurrences,
            });
        }
        if let Some(index) = seen.iter().position(|seen| !seen) {
            let backing =
                ArchiveBackingId(u32::try_from(index).map_err(|_| GraphError::CountOverflow {
                    kind: GraphResourceKind::SharedArrayBufferBackings,
                })?);
            return Err(SabArchiveError::MissingBackingDescriptor { backing });
        }

        Ok(self.shared_backings.into_boxed_slice())
    }
}

/// Complete bytecode image inseparably bound to its pointer-free SAB archive.
///
/// Native writer tokens and live backing capabilities are absent. This module
/// privately owns both fields and the sole construction site; a future
/// materializer must consume the aggregate here as one authenticated unit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ArchivedBytecodeImage {
    archived_image_payload: BytecodeImage,
    archived_image_shared_backings: Box<[SharedBackingDescriptor]>,
}

impl ArchivedBytecodeImage {
    /// Number of distinct shared backings retained by this archive.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn shared_backing_count(&self) -> usize {
        self.archived_image_shared_backings.len()
    }

    #[cfg(test)]
    pub(in crate::runtime::binary_object) const fn test_image(&self) -> &BytecodeImage {
        &self.archived_image_payload
    }

    #[cfg(test)]
    pub(in crate::runtime::binary_object) fn test_shared_backing_descriptor(
        &self,
        backing: ArchiveBackingId,
    ) -> Option<SharedBackingDescriptor> {
        self.archived_image_shared_backings
            .get(backing.as_usize())
            .copied()
    }
}

/// Complete pointer-free graph plus the descriptor table for every SAB backing.
///
/// Fields, construction, and the contained bare graph remain private. Clone and
/// equality operate on the aggregate, never on independently supplied parts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ArchivedWireGraph {
    archived_graph_payload: WireGraph,
    archived_graph_shared_backings: Box<[SharedBackingDescriptor]>,
}

impl ArchivedWireGraph {
    /// Number of distinct shared backings retained by this archive.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn shared_backing_count(&self) -> usize {
        self.archived_graph_shared_backings.len()
    }

    #[cfg(test)]
    pub(in crate::runtime::binary_object) const fn test_graph(&self) -> &WireGraph {
        &self.archived_graph_payload
    }

    #[cfg(test)]
    pub(super) fn test_shared_backing_descriptor(
        &self,
        backing: ArchiveBackingId,
    ) -> Option<SharedBackingDescriptor> {
        self.archived_graph_shared_backings
            .get(backing.as_usize())
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::super::encode::{GraphEncodeError, GraphEncodeOptions, encode_graph};
    use super::super::model::{NodeId, TypedArrayKind, WireValue};
    use super::*;

    const BASE_LIMITS: GraphLimits = GraphLimits::new(16, 16, 8, 16, 32, 32, 64, 64, 128);
    const WIRE_LIMITS: WireLimits = WireLimits::new(1024, 0, 0, 0);

    fn sab_limits(
        occurrences: usize,
        backings: usize,
        capacity: usize,
        total_capacity: usize,
    ) -> GraphLimits {
        BASE_LIMITS.with_shared_array_buffers(occurrences, backings, capacity, total_capacity)
    }

    fn graph(nodes: Vec<WireNode>) -> WireGraph {
        let root = if nodes.is_empty() {
            WireValue::Undefined
        } else {
            WireValue::Node(NodeId::from_zero_based(0))
        };
        WireGraph {
            atoms: Box::default(),
            nodes: nodes.into_boxed_slice(),
            ref_table: Box::default(),
            root,
        }
    }

    fn sab_node(
        byte_length: u32,
        max_byte_length: Option<u32>,
        backing: ArchiveBackingId,
    ) -> WireNode {
        WireNode::SharedArrayBuffer {
            byte_length,
            max_byte_length,
            backing,
        }
    }

    const fn native(bits: u64) -> NativeSabToken {
        NativeSabToken::from_test_bits(bits)
    }

    fn token_wire(tokens: &[u64]) -> Vec<u8> {
        tokens
            .iter()
            .flat_map(|token| token.to_le_bytes())
            .collect()
    }

    fn cursor<'a>(
        wire: &'a [u8],
        occurrences: &'a [NativeSabToken],
        limits: GraphLimits,
    ) -> SabTransportCursor<'a> {
        SabTransportInput::new(wire, occurrences)
            .into_cursor_for_test(ReaderMode::Strict, WIRE_LIMITS, limits)
            .unwrap()
    }

    #[test]
    fn raw_tokens_intern_to_first_occurrence_ids_and_never_reach_debug() {
        const FIRST_RAW: u64 = 0xfeed_face_dead_beef;
        const SECOND_RAW: u64 = 0x0123_4567_89ab_cdef;
        let wire = token_wire(&[FIRST_RAW, FIRST_RAW, SECOND_RAW]);
        let occurrences = [native(FIRST_RAW), native(FIRST_RAW), native(SECOND_RAW)];
        let mut cursor = cursor(&wire, &occurrences, sab_limits(3, 2, 8, 12));
        let first = cursor.record_shared_array_buffer(4, None).unwrap();
        let repeated = cursor.record_shared_array_buffer(4, None).unwrap();
        let second = cursor.record_shared_array_buffer(2, Some(8)).unwrap();
        assert_eq!(first.zero_based(), 0);
        assert_eq!(repeated, first);
        assert_eq!(second.zero_based(), 1);

        let archive = cursor
            .finish_graph_archive_for_test(graph(vec![
                sab_node(4, None, first),
                sab_node(4, None, first),
                sab_node(2, Some(8), second),
            ]))
            .unwrap();
        assert_eq!(archive.shared_backing_count(), 2);
        let debug = format!("{archive:?}");
        assert!(!debug.contains("feed_face_dead_beef"));
        assert!(!debug.contains(&FIRST_RAW.to_string()));
    }

    #[test]
    fn ordinary_encoder_rejects_reachable_archived_backings_only() {
        let options = GraphEncodeOptions::new(true, 1024, BASE_LIMITS);

        let wire = token_wire(&[0x42]);
        let occurrences = [native(0x42)];
        let mut direct = cursor(&wire, &occurrences, sab_limits(1, 1, 4, 4));
        let backing = direct.record_shared_array_buffer(4, None).unwrap();
        let direct = direct
            .finish_graph_archive_for_test(graph(vec![sab_node(4, None, backing)]))
            .unwrap();
        assert_eq!(
            encode_graph(direct.test_graph(), options),
            Err(GraphEncodeError::ArchivedBackingContextRequired {
                node: NodeId::from_zero_based(0),
            })
        );

        let wire = token_wire(&[0x42]);
        let occurrences = [native(0x42)];
        let mut viewed = cursor(&wire, &occurrences, sab_limits(1, 1, 4, 4));
        let backing = viewed.record_shared_array_buffer(4, None).unwrap();
        let viewed = viewed
            .finish_graph_archive_for_test(graph(vec![
                WireNode::TypedArray {
                    kind: TypedArrayKind::Uint8,
                    length: 4,
                    byte_offset: 0,
                    buffer: NodeId::from_zero_based(1),
                },
                sab_node(4, None, backing),
            ]))
            .unwrap();
        assert_eq!(
            encode_graph(viewed.test_graph(), options),
            Err(GraphEncodeError::ArchivedBackingContextRequired {
                node: NodeId::from_zero_based(1),
            })
        );

        let wire = token_wire(&[0x42]);
        let occurrences = [native(0x42)];
        let mut unreachable = cursor(&wire, &occurrences, sab_limits(1, 1, 4, 4));
        let backing = unreachable.record_shared_array_buffer(4, None).unwrap();
        let unreachable = unreachable
            .finish_graph_archive_for_test(WireGraph {
                atoms: Box::default(),
                nodes: vec![sab_node(4, None, backing)].into_boxed_slice(),
                ref_table: Box::default(),
                root: WireValue::Int32(42),
            })
            .unwrap();
        assert_eq!(
            encode_graph(unreachable.test_graph(), options).unwrap(),
            [5, 0, 5, 84]
        );
    }

    #[test]
    fn one_backing_allows_wrapper_local_growth_but_not_descriptor_drift() {
        let wire = token_wire(&[0x42, 0x42, 0x42]);
        let occurrences = [native(0x42), native(0x42), native(0x42)];
        let mut drift = cursor(&wire, &occurrences, sab_limits(3, 1, 8, 8));
        let backing = drift.record_shared_array_buffer(2, Some(8)).unwrap();
        assert_eq!(drift.record_shared_array_buffer(4, Some(8)), Ok(backing));
        assert_eq!(
            drift.record_shared_array_buffer(2, None),
            Err(SabArchiveError::ConflictingBackingDescriptor { backing })
        );

        let wire = token_wire(&[0x42, 0x42]);
        let occurrences = [native(0x42), native(0x42)];
        let mut valid = cursor(&wire, &occurrences, sab_limits(2, 1, 8, 8));
        let backing = valid.record_shared_array_buffer(2, Some(8)).unwrap();
        assert_eq!(valid.record_shared_array_buffer(4, Some(8)), Ok(backing));
        let archive = valid
            .finish_graph_archive_for_test(graph(vec![
                sab_node(2, Some(8), backing),
                sab_node(4, Some(8), backing),
            ]))
            .unwrap();
        let descriptor = archive.test_shared_backing_descriptor(backing).unwrap();
        assert_eq!(descriptor.capacity(), 8);
        assert!(descriptor.is_growable());
    }

    #[test]
    fn each_shared_array_buffer_resource_has_an_independent_limit() {
        let cases = [
            (
                sab_limits(0, 1, 4, 4),
                GraphResourceKind::SharedArrayBufferOccurrences,
                1,
                0,
            ),
            (
                sab_limits(1, 0, 4, 4),
                GraphResourceKind::SharedArrayBufferBackings,
                1,
                0,
            ),
            (
                sab_limits(1, 1, 3, 4),
                GraphResourceKind::SharedArrayBufferCapacity,
                4,
                3,
            ),
            (
                sab_limits(1, 1, 4, 3),
                GraphResourceKind::TotalSharedArrayBufferCapacity,
                4,
                3,
            ),
        ];

        for (limits, kind, requested, limit) in cases {
            let wire = token_wire(&[0x42]);
            let occurrences = [native(0x42)];
            let mut cursor = cursor(&wire, &occurrences, limits);
            assert_eq!(
                cursor.record_shared_array_buffer(4, None),
                Err(SabArchiveError::Graph(GraphError::ResourceLimit {
                    kind,
                    requested,
                    limit,
                }))
            );
        }
    }

    #[test]
    fn repeated_records_charge_occurrences_but_unique_capacity_once() {
        let wire = token_wire(&[0x42, 0x42, 0x42]);
        let occurrences = [native(0x42), native(0x42), native(0x42)];
        let mut cursor = cursor(&wire, &occurrences, sab_limits(2, 1, 8, 8));
        let backing = cursor.record_shared_array_buffer(2, Some(8)).unwrap();
        assert_eq!(cursor.record_shared_array_buffer(4, Some(8)), Ok(backing));
        assert_eq!(
            cursor.record_shared_array_buffer(6, Some(8)),
            Err(SabArchiveError::Graph(GraphError::ResourceLimit {
                kind: GraphResourceKind::SharedArrayBufferOccurrences,
                requested: 3,
                limit: 2,
            }))
        );
    }

    #[test]
    fn finish_rejects_missing_mismatched_and_out_of_range_backings() {
        let wire = token_wire(&[0x42]);
        let occurrences = [native(0x42)];
        let mut missing = cursor(&wire, &occurrences, sab_limits(1, 1, 4, 4));
        missing.record_shared_array_buffer(4, None).unwrap();
        assert_eq!(
            missing.finish_graph_archive_for_test(graph(Vec::new())),
            Err(SabArchiveError::OccurrenceCountMismatch {
                recorded: 1,
                archive_nodes: 0,
            })
        );

        let empty = cursor(&[], &[], sab_limits(1, 1, 4, 4));
        let invalid = ArchiveBackingId(7);
        assert_eq!(
            empty.finish_graph_archive_for_test(graph(vec![sab_node(4, None, invalid)])),
            Err(SabArchiveError::InvalidBackingIndex {
                backing: invalid,
                backing_count: 0,
            })
        );

        let wire = token_wire(&[0x42]);
        let occurrences = [native(0x42)];
        let mut mismatch = cursor(&wire, &occurrences, sab_limits(1, 1, 8, 8));
        let backing = mismatch.record_shared_array_buffer(4, None).unwrap();
        assert_eq!(
            mismatch.finish_graph_archive_for_test(graph(vec![sab_node(4, Some(8), backing)])),
            Err(SabArchiveError::ConflictingBackingDescriptor { backing })
        );

        let wire = token_wire(&[0x42, 0x99]);
        let occurrences = [native(0x42), native(0x99)];
        let mut missing_descriptor = cursor(&wire, &occurrences, sab_limits(2, 2, 4, 8));
        let first = missing_descriptor
            .record_shared_array_buffer(4, None)
            .unwrap();
        let second = missing_descriptor
            .record_shared_array_buffer(4, None)
            .unwrap();
        assert_eq!(
            missing_descriptor.finish_graph_archive_for_test(graph(vec![
                sab_node(4, None, first),
                sab_node(4, None, first),
            ])),
            Err(SabArchiveError::MissingBackingDescriptor { backing: second })
        );
    }

    #[test]
    fn writer_occurrence_table_is_ordered_exact_and_fully_consumed() {
        const FIRST: u64 = 0xfeed_face_dead_beef;
        const SECOND: u64 = 0x0123_4567_89ab_cdef;

        let wire = token_wire(&[FIRST, SECOND]);
        let occurrences = [native(FIRST), native(FIRST)];
        let mut ordered = cursor(&wire, &occurrences, sab_limits(2, 2, 8, 16));
        ordered.record_shared_array_buffer(4, None).unwrap();
        let mismatch = ordered.record_shared_array_buffer(4, None).unwrap_err();
        assert_eq!(
            mismatch,
            SabArchiveError::SideTableTokenMismatch {
                offset: 8,
                ordinal: 1,
            }
        );
        let diagnostic = format!("{mismatch:?} {mismatch}");
        assert!(!diagnostic.contains("feed_face_dead_beef"));
        assert!(!diagnostic.contains("0123_4567_89ab_cdef"));
        assert!(!diagnostic.contains(&FIRST.to_string()));
        assert!(!diagnostic.contains(&SECOND.to_string()));

        let wire = token_wire(&[FIRST]);
        let mut short = cursor(&wire, &[], sab_limits(1, 1, 8, 8));
        assert_eq!(
            short.record_shared_array_buffer(4, None),
            Err(SabArchiveError::SideTableTooShort {
                offset: 0,
                ordinal: 0,
                entry_count: 0,
            })
        );

        let mut truncated = cursor(&[1, 2, 3], &[], sab_limits(1, 1, 8, 8));
        assert_eq!(
            truncated.record_shared_array_buffer(4, None),
            Err(SabArchiveError::Wire(WireError::Truncated {
                offset: 0,
                needed: 8,
                remaining: 3,
            }))
        );

        let occurrences = [native(FIRST)];
        let extra = cursor(&[], &occurrences, sab_limits(1, 1, 8, 8));
        assert_eq!(
            extra.finish_graph_archive_for_test(graph(Vec::new())),
            Err(SabArchiveError::SideTableHasExtra {
                consumed: 0,
                entry_count: 1,
            })
        );
    }
}
