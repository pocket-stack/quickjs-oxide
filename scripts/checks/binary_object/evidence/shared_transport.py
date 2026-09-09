"""Pinned data and expected shapes for shared_transport."""

REVIEWED_ENTRYPOINTS = (('decode_graph_with_sab_transport',
  '\n'
  '        pub(in crate::engine::code) fn decode_graph_with_sab_transport(\n'
  "            input: SabTransportInput<'_>,\n"
  '            mode: ReaderMode,\n'
  '            wire_limits: WireLimits,\n'
  '            graph_limits: GraphLimits,\n'
  '            allow_object_references: bool,\n'
  '        ) -> Result<ArchivedWireGraph, DecodeError> {\n'
  '            let cursor = input\n'
  '                .build_cursor(mode, wire_limits, graph_limits)\n'
  '                .map_err(map_sab_archive_error)?;\n'
  '            let (cursor, graph) =\n'
  '                decode_graph_body(cursor, graph_limits, allow_object_references)?;\n'
  '            cursor\n'
  '                .finish_graph_archive(graph)\n'
  '                .map_err(map_sab_archive_error)\n'
  '        }\n'
  '        '),
 ('decode_bytecode_image_with_sab_transport',
  '\n'
  '        pub(in crate::engine::code) fn decode_bytecode_image_with_sab_transport(\n'
  "            input: SabTransportInput<'_>,\n"
  '            mode: ReaderMode,\n'
  '            wire_limits: WireLimits,\n'
  '            limits: BytecodeImageLimits,\n'
  '            allow_object_references: bool,\n'
  '        ) -> Result<ArchivedBytecodeImage, BytecodeImageError> {\n'
  '            let cursor = input.build_cursor(mode, wire_limits, limits.graph())?;\n'
  '            let (cursor, image) =\n'
  '                decode_bytecode_image_body(cursor, limits, allow_object_references)?;\n'
  '            cursor.finish_bytecode_image(image).map_err(Into::into)\n'
  '        }\n'
  '        '))

TRANSPORT_FIELD_COUNTS = (('transport_wire_bytes', 3),
 ('transport_writer_occurrences', 3),
 ('cursor_wire', 18),
 ('cursor_writer_occurrences', 6),
 ('cursor_next_occurrence', 6),
 ('cursor_archive', 4))
