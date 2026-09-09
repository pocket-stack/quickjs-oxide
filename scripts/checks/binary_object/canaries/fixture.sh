tmp_dir=$(mktemp -d "${TMPDIR:-/tmp}/quickjs-oxide-binary-boundary.XXXXXX")
trap 'rm -rf -- "$tmp_dir"' EXIT HUP INT TERM

fixture=$tmp_dir/fixture
mkdir -p "$fixture/src/engine/vm" "$fixture/src/engine/atom" "$fixture/src/engine/heap/runtime"
boundary_self_test_token="generated-boundary-self-test-$PPID-$RANDOM-$RANDOM"
mkdir -p "$fixture/src/engine/api" "$fixture/src/engine/code" "$fixture/src/engine/value" "$fixture/src/engine/code/binary_object/bytecode_image/decode" \
    "$fixture/src/engine/code/binary_object/function_translate" \
    "$fixture/src/engine/code/binary_object/graph"
printf '%s\n' "$boundary_self_test_token" > "$fixture/.boundary-self-test"
printf '%s\n' 'pub mod runtime;' > "$fixture/src/lib.rs"
printf '%s\n' '// shared runtime ownership' > "$fixture/src/engine/heap/runtime/mod.rs"
printf '%s\n' 'mod binary_object;' > "$fixture/src/engine/code/mod.rs"
cp -- "$repository_root/src/engine/code/bytecode.rs" "$fixture/src/engine/code/bytecode.rs"
cp -- "$repository_root/src/engine/vm/mod.rs" "$fixture/src/engine/vm/mod.rs"
cp -- "$repository_root/src/engine/value/mod.rs" "$fixture/src/engine/value/mod.rs"
cp -- "$repository_root/src/engine/atom/mod.rs" "$fixture/src/engine/atom/mod.rs"
cp -- "$repository_root/src/engine/code/function.rs" "$fixture/src/engine/code/function.rs"
cp -- "$repository_root/src/engine/value/primitive.rs" "$fixture/src/engine/value/primitive.rs"
cp -R -- "$repository_root/src/engine/api/context" "$fixture/src/engine/api/context"
cp -- "$repository_root/src/engine/code/bytecode_publish.rs" \
    "$fixture/src/engine/code/bytecode_publish.rs"
printf '%s\n' \
    'mod atoms;' \
    'mod code;' \
    'mod function_envelope;' \
    'mod function_translate;' \
    'mod bytecode_image;' \
    'mod graph;' \
    'mod pinned_atoms;' \
    'mod pinned_opcodes;' \
    'mod read_cursor;' \
    'mod ordinary_leaf;' \
    'mod scalar_script;' \
    'mod wire;' \
    'pub(super) use scalar_script::{ScalarScriptReadError, ScalarStringDraft, ScalarUnaryOp, ScalarValueDraft, decode_trusted_scalar_script};' \
    'pub(super) use ordinary_leaf::{DetachedAtomName, DetachedPrimitive, OrdinaryLeafApplyKind, OrdinaryLeafBinaryOp, OrdinaryLeafDraft, OrdinaryLeafMetadataDraft, OrdinaryLeafOp, OrdinaryLeafPredicateOp, OrdinaryLeafReadError, OrdinaryLeafStackOp, OrdinaryLeafUnaryOp, RootFunctionConstantSelector, decode_trusted_ordinary_leaf};' \
    > "$fixture/src/engine/code/binary_object/mod.rs"
cp -- "$repository_root/src/engine/code/binary_object/function_translate/mod.rs" \
    "$fixture/src/engine/code/binary_object/function_translate/mod.rs"
cp -- "$repository_root/src/engine/code/binary_object/function_translate/capability.rs" \
    "$fixture/src/engine/code/binary_object/function_translate/capability.rs"
cp -- "$repository_root/src/engine/code/binary_object/function_translate/dto.rs" \
    "$fixture/src/engine/code/binary_object/function_translate/dto.rs"
python3 "$boundary_dir/canaries/scalar_fixture.py" \
    "$repository_root/src/engine/code/binary_object/scalar_script.rs" \
    "$fixture/src/engine/code/binary_object/scalar_script.rs"
python3 "$boundary_dir/canaries/ordinary_fixture.py" \
    "$repository_root/src/engine/code/binary_object/ordinary_leaf.rs" \
    "$fixture/src/engine/code/binary_object/ordinary_leaf.rs"
printf '%s\n' '// no alternate binary-object consumers' \
    > "$fixture/src/engine/heap/runtime/other.rs"
printf '%s\n' \
    'fn retained_from_raw(raw: u32) {' \
    '    let _ = PinnedAtomId::from_raw(raw);' \
    '}' \
    > "$fixture/src/engine/code/binary_object/atoms.rs"
printf '%s\n' \
    'mod sealed {' \
    '    pub trait Sealed {}' \
    "    impl Sealed for WireCursor<'_> {}" \
    "    impl Sealed for SabTransportCursor<'_> {}" \
    '}' \
    "pub(in crate::engine::code::binary_object) trait CheckedReadCursor<'input>: sealed::Sealed {}" \
    "impl<'input> CheckedReadCursor<'input> for WireCursor<'input> {}" \
    "impl<'input> CheckedReadCursor<'input> for SabTransportCursor<'input> {}" \
    > "$fixture/src/engine/code/binary_object/read_cursor.rs"
printf '%s\n' \
    'mod atoms;' \
    'mod budget;' \
    'mod decode;' \
    'mod encode;' \
    'mod model;' \
    'mod native_plan;' \
    '#[cfg(test)]' \
    'mod tests;' \
    'pub(in crate::engine::code::binary_object) use native_plan::{NativeAtomClass, NativeAtomRef, NativeCodePlan, NativeOperands, decode_native_code_plan};' \
    > "$fixture/src/engine/code/binary_object/bytecode_image/mod.rs"
cp -- "$repository_root/src/engine/code/binary_object/bytecode_image/native_plan.rs" \
    "$fixture/src/engine/code/binary_object/bytecode_image/native_plan.rs"
cp -- "$repository_root/src/engine/code/binary_object/pinned_opcodes.rs" \
    "$fixture/src/engine/code/binary_object/pinned_opcodes.rs"
printf '%s\n' \
    'pub(in crate::engine::code::binary_object) fn decode_bytecode_image_body() {}' \
    > "$fixture/src/engine/code/binary_object/bytecode_image/decode/mod.rs"
printf '%s\n' \
    'pub(super) enum ImageAtom {' \
    '    Null,' \
    '    Index(u32),' \
    '    Predefined(PinnedAtomId),' \
    '    Dynamic(AtomId),' \
    '}' \
    > "$fixture/src/engine/code/binary_object/bytecode_image/atoms.rs"
printf '%s\n' \
    'const PINNED_EVAL_ATOM_RAW: u32 = 84;' \
    'struct ImageLocalVariable { name: ImageAtom }' \
    'impl ImageLocalVariable {' \
    '    pub(in crate::engine::code::binary_object) const fn name_is_null(&self) -> bool {' \
    '        matches!(self.name, ImageAtom::Null)' \
    '    }' \
    '}' \
    'struct ImageFunctionEnvelope { name: ImageAtom }' \
    'impl ImageFunctionEnvelope {' \
    '    pub(in crate::engine::code::binary_object) const fn name_is_pinned_eval(&self) -> bool {' \
    '        match self.name {' \
    '            ImageAtom::Predefined(atom) => atom.raw() == PINNED_EVAL_ATOM_RAW,' \
    '            ImageAtom::Null | ImageAtom::Index(_) | ImageAtom::Dynamic(_) => false,' \
    '        }' \
    '    }' \
    '}' \
    'impl BytecodeImage {' \
    '    fn sab_archive_occurrences(&self) {}' \
    '}' \
    > "$fixture/src/engine/code/binary_object/bytecode_image/model.rs"
printf '%s\n' \
    'pub(super) fn decode_graph_body() {}' \
    > "$fixture/src/engine/code/binary_object/graph/decode.rs"
printf '%s\n' \
    'pub(in crate::engine::code) struct NativeSabToken {' \
    '    native_token_bits: u64,' \
    '}' \
    '#[cfg(test)]' \
    'impl NativeSabToken {' \
    '    #[must_use]' \
    '    pub(in crate::engine::code::binary_object) const fn from_test_bits(bits: u64) -> Self {' \
    '        Self {' \
    '            native_token_bits: bits,' \
    '        }' \
    '    }' \
    '}' \
    'pub(in crate::engine::code) struct SabTransportInput<'"'"'a> {' \
    '    transport_wire_bytes: &'"'"'a [u8],' \
    '    transport_writer_occurrences: &'"'"'a [NativeSabToken],' \
    '}' \
    "impl<'a> SabTransportInput<'a> {" \
    '    #[must_use]' \
    '    pub(in crate::engine::code) const fn new(' \
    '        wire: &'"'"'a [u8],' \
    '        writer_occurrences: &'"'"'a [NativeSabToken],' \
    '    ) -> Self {' \
    '        Self {' \
    '            transport_wire_bytes: wire,' \
    '            transport_writer_occurrences: writer_occurrences,' \
    '        }' \
    '    }' \
    '    fn build_cursor(' \
    '        self,' \
    '        mode: ReaderMode,' \
    '        wire_limits: WireLimits,' \
    '        graph_limits: GraphLimits,' \
    '    ) -> Result<SabTransportCursor<'"'"'a>, SabArchiveError> {' \
    '        Ok(SabTransportCursor {' \
    '            cursor_wire: WireCursor::new(self.transport_wire_bytes, mode, wire_limits)?,' \
    '            cursor_writer_occurrences: self.transport_writer_occurrences,' \
    '            cursor_next_occurrence: 0,' \
    '            cursor_archive: SabArchiveState::new(graph_limits),' \
    '        })' \
    '    }' \
    '    #[cfg(test)]' \
    '    fn into_cursor_for_test(' \
    '        self,' \
    '        mode: ReaderMode,' \
    '        wire_limits: WireLimits,' \
    '        graph_limits: GraphLimits,' \
    '    ) -> Result<SabTransportCursor<'"'"'a>, SabArchiveError> {' \
    '        self.build_cursor(mode, wire_limits, graph_limits)' \
    '    }' \
    '}' \
    'pub(in crate::engine::code) fn decode_graph_with_sab_transport(' \
    '    input: SabTransportInput<'"'"'_>,' \
    '    mode: ReaderMode,' \
    '    wire_limits: WireLimits,' \
    '    graph_limits: GraphLimits,' \
    '    allow_object_references: bool,' \
    ') -> Result<ArchivedWireGraph, DecodeError> {' \
    '    let cursor = input' \
    '        .build_cursor(mode, wire_limits, graph_limits)' \
    '        .map_err(map_sab_archive_error)?;' \
    '    let (cursor, graph) =' \
    '        decode_graph_body(cursor, graph_limits, allow_object_references)?;' \
    '    cursor' \
    '        .finish_graph_archive(graph)' \
    '        .map_err(map_sab_archive_error)' \
    '}' \
    'pub(in crate::engine::code) fn decode_bytecode_image_with_sab_transport(' \
    '    input: SabTransportInput<'"'"'_>,' \
    '    mode: ReaderMode,' \
    '    wire_limits: WireLimits,' \
    '    limits: BytecodeImageLimits,' \
    '    allow_object_references: bool,' \
    ') -> Result<ArchivedBytecodeImage, BytecodeImageError> {' \
    '    let cursor = input.build_cursor(mode, wire_limits, limits.graph())?;' \
    '    let (cursor, image) =' \
    '        decode_bytecode_image_body(cursor, limits, allow_object_references)?;' \
    '    cursor.finish_bytecode_image(image).map_err(Into::into)' \
    '}' \
    'pub(in crate::engine::code::binary_object) struct SabTransportCursor<'"'"'a> {' \
    '    cursor_wire: WireCursor<'"'"'a>,' \
    '    cursor_writer_occurrences: &'"'"'a [NativeSabToken],' \
    '    cursor_next_occurrence: usize,' \
    '    cursor_archive: SabArchiveState,' \
    '}' \
    "impl<'a> SabTransportCursor<'a> {" \
    '    pub(in crate::engine::code::binary_object) const fn position(&self) -> usize {' \
    '        let _ = &self.cursor_wire;' \
    '        0' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) const fn mode(&self) -> ReaderMode {' \
    '        let _ = &self.cursor_wire;' \
    '        mode' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn remaining(&self) -> usize {' \
    '        let _ = &self.cursor_wire;' \
    '        0' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_u8(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_u16_le(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_bytes(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_tag(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_uleb128(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_i32(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_f64(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_header(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn read_string(&mut self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(in crate::engine::code::binary_object) fn validate_wire_end(&self) {' \
    '        let _ = &self.cursor_wire;' \
    '    }' \
    '    pub(super) fn record_shared_array_buffer(&mut self, expected: &NativeSabToken) {' \
    '        let _ = &self.cursor_wire;' \
    '        let _ = &self.cursor_wire;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = &self.cursor_archive;' \
    '        let _ = expected.native_token_bits;' \
    '    }' \
    '    fn finish_shared_backings(&self) -> Box<[SharedBackingDescriptor]> {' \
    '        let _ = &self.cursor_wire;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = &self.cursor_writer_occurrences;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = self.cursor_next_occurrence;' \
    '        let _ = &self.cursor_archive;' \
    '        shared_backings' \
    '    }' \
    '    fn finish_graph_archive(self, graph: WireGraph) -> Result<ArchivedWireGraph, Error> {' \
    '        let shared_backings = self.finish_shared_backings();' \
    '        ArchivedWireGraph {' \
    '            archived_graph_payload: graph,' \
    '            archived_graph_shared_backings: shared_backings,' \
    '        }' \
    '    }' \
    '    #[cfg(test)]' \
    '    fn finish_graph_archive_for_test(' \
    '        self,' \
    '        graph: WireGraph,' \
    '    ) -> Result<ArchivedWireGraph, SabArchiveError> {' \
    '        self.finish_graph_archive(graph)' \
    '    }' \
    '    fn finish_bytecode_image(self, image: BytecodeImage) -> Result<ArchivedBytecodeImage, Error> {' \
    '        let shared_backings = self.finish_shared_backings();' \
    '        image.sab_archive_occurrences();' \
    '        ArchivedBytecodeImage {' \
    '            archived_image_payload: image,' \
    '            archived_image_shared_backings: shared_backings,' \
    '        }' \
    '    }' \
    '}' \
    'pub(in crate::engine::code) struct ArchivedWireGraph {' \
    '    archived_graph_payload: WireGraph,' \
    '    archived_graph_shared_backings: Box<[SharedBackingDescriptor]>,' \
    '}' \
    'impl ArchivedWireGraph {' \
    '    #[must_use]' \
    '    pub(in crate::engine::code::binary_object) const fn shared_backing_count(&self) -> usize {' \
    '        self.archived_graph_shared_backings.len()' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(in crate::engine::code::binary_object) const fn test_graph(&self) -> &WireGraph {' \
    '        &self.archived_graph_payload' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(super) fn test_shared_backing_descriptor(' \
    '        &self,' \
    '        backing: ArchiveBackingId,' \
    '    ) -> Option<SharedBackingDescriptor> {' \
    '        self.archived_graph_shared_backings' \
    '            .get(backing.as_usize())' \
    '            .copied()' \
    '    }' \
    '}' \
    'pub(in crate::engine::code) struct ArchivedBytecodeImage {' \
    '    archived_image_payload: BytecodeImage,' \
    '    archived_image_shared_backings: Box<[SharedBackingDescriptor]>,' \
    '}' \
    'impl ArchivedBytecodeImage {' \
    '    #[must_use]' \
    '    pub(in crate::engine::code::binary_object) const fn shared_backing_count(&self) -> usize {' \
    '        self.archived_image_shared_backings.len()' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(in crate::engine::code::binary_object) const fn test_image(&self) -> &BytecodeImage {' \
    '        &self.archived_image_payload' \
    '    }' \
    '    #[cfg(test)]' \
    '    pub(in crate::engine::code::binary_object) fn test_shared_backing_descriptor(' \
    '        &self,' \
    '        backing: ArchiveBackingId,' \
    '    ) -> Option<SharedBackingDescriptor> {' \
    '        self.archived_image_shared_backings' \
    '            .get(backing.as_usize())' \
    '            .copied()' \
    '    }' \
    '}' \
    > "$fixture/src/engine/code/binary_object/graph/sab_transport.rs"

# VM evidence is split across its registered implementation owners.
cp -- "$repository_root/src/engine/vm/protocol.rs" "$fixture/src/engine/vm/protocol.rs"
cp -- "$repository_root/src/engine/vm/completion.rs" "$fixture/src/engine/vm/completion.rs"
cp -- "$repository_root/src/engine/vm/numeric.rs" "$fixture/src/engine/vm/numeric.rs"
cp -- "$repository_root/src/engine/vm/detached.rs" "$fixture/src/engine/vm/detached.rs"
cp -- "$repository_root/src/engine/vm/activation.rs" "$fixture/src/engine/vm/activation.rs"
cp -- "$repository_root/src/engine/vm/dispatch.rs" "$fixture/src/engine/vm/dispatch.rs"
cp -- "$repository_root/src/engine/vm/numeric_execution.rs" "$fixture/src/engine/vm/numeric_execution.rs"
cp -- "$repository_root/src/engine/vm/unwind.rs" "$fixture/src/engine/vm/unwind.rs"
cp -- "$repository_root/src/engine/vm/frame_execution.rs" "$fixture/src/engine/vm/frame_execution.rs"
cp -- "$repository_root/src/engine/vm/tests.rs" "$fixture/src/engine/vm/tests.rs"

scan_root "$fixture" "$boundary_self_test_token" \
    || die "binary-object boundary rejected its clean no-consumer self-test fixture"

printf '%s\n' 'mod binary_object_publish;' >> "$fixture/src/engine/code/mod.rs"
cp -- "$repository_root/src/engine/code/binary_object_publish.rs" \
    "$fixture/src/engine/code/binary_object_publish.rs"

scan_root "$fixture" "$boundary_self_test_token" \
    || die "binary-object boundary rejected its clean sole-consumer self-test fixture"
