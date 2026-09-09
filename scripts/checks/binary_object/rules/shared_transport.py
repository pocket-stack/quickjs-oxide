"""Shared transport checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import shared_transport as evidence


def check(ctx):
    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.source = ctx.binary_source_cache[ctx.path]
        ctx.code = ctx.binary_code_cache[ctx.path]
        for ctx.name in ctx.retired_permit_names:
            ctx.match = re.search(rf"\b{re.escape(ctx.name)}\b", ctx.code)
            if ctx.match is not None:
                ctx.fail(
                    "retired-sab-permit",
                    f"{ctx.name} must not reintroduce forgeable cross-module permits; found "
                    + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
                )

    native_token_struct = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+NativeSabToken[ \t\n]*\{"
        r"[ \t\n]*native_token_bits[ \t\n]*:[ \t\n]*u64[ \t\n]*,?[ \t\n]*\}"
    )

    if len(native_token_struct.findall(ctx.sab_transport_code)) != 1:
        ctx.fail(
            "sab-native-token-shape",
            "NativeSabToken must retain one private named u64 field",
        )

    if re.search(
        r"#[ \t\n]*\[[^\]]*\][ \t\n]*pub[ \t\n]*\([^)]*\)"
        r"[ \t\n]+struct[ \t\n]+NativeSabToken\b",
        ctx.sab_transport_code,
    ):
        ctx.fail(
            "sab-native-token-derive",
            "NativeSabToken must not gain derive or representation attributes",
        )

    native_token_impl_pattern = re.compile(
        r"\bimpl\b[^{};]*\bNativeSabToken\b[^{};]*\{",
        re.DOTALL,
    )

    native_token_impl_sites: list[tuple[str, str, re.Match[str]]] = []

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.code = ctx.binary_code_cache[ctx.path]
        native_token_impl_sites.extend(
            (ctx.relative, ctx.code, match) for match in native_token_impl_pattern.finditer(ctx.code)
        )

    if len(native_token_impl_sites) != 1 or native_token_impl_sites[0][0] != ctx.sab_transport_relative:
        ctx.fail(
            "sab-native-token-implementation-set",
            "NativeSabToken must have one test-only transport-owned implementation",
        )
    else:
        ctx._, ctx.code, ctx.match = native_token_impl_sites[0]
        prefix = ctx.code[max(0, ctx.match.start() - 80):ctx.match.start()]
        if re.search(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]\s*$", prefix) is None:
            ctx.fail(
                "sab-native-token-test-only",
                "NativeSabToken implementation must remain guarded by cfg(test)",
            )
        open_offset = ctx.match.end() - 1
        depth = 0
        close_offset = None
        for offset in range(open_offset, len(ctx.code)):
            character = ctx.code[offset]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    close_offset = offset + 1
                    break
        if close_offset is None:
            ctx.fail(
                "sab-native-token-implementation-set",
                "NativeSabToken implementation has no balanced closing brace",
            )
        else:
            actual_impl = " ".join(ctx.code[ctx.match.start():close_offset].split())
            expected_impl = " ".join(
                '\n            impl NativeSabToken {\n                #[must_use]\n                pub(in crate::engine::code::binary_object) const fn from_test_bits(bits: u64) -> Self {\n                    Self {\n                        native_token_bits: bits,\n                    }\n                }\n            }\n            '.split()
            )
            if actual_impl != expected_impl:
                ctx.fail(
                    "sab-native-token-implementation-set",
                    "NativeSabToken implementation drifted from its reviewed test-only constructor",
                )

    def identifier_paths(name: str) -> list[str]:
        pattern = re.compile(rf"\b{re.escape(name)}\b")
        found: list[str] = []
        for path in ctx.binary_sources:
            if path.is_symlink() or not path.is_file():
                continue
            relative = path.relative_to(ctx.root).as_posix()
            code = ctx.binary_code_cache[path]
            found.extend(relative for _ in pattern.finditer(code))
        return found
    identifier_paths = identifier_paths

    native_token_field_paths = identifier_paths("native_token_bits")

    if native_token_field_paths != [ctx.sab_transport_relative] * 3:
        ctx.fail(
            "sab-native-token-field-escape",
            "native_token_bits must appear only in its field, test constructor, and matcher; "
            f"found {native_token_field_paths}",
        )

    def definition_paths(kind: str, name: str) -> list[str]:
        pattern = re.compile(rf"\b{kind}[ \t\n]+{re.escape(name)}\b")
        found: list[str] = []
        for path in ctx.binary_sources:
            if path.is_symlink() or not path.is_file():
                continue
            relative = path.relative_to(ctx.root).as_posix()
            code = ctx.binary_code_cache[path]
            found.extend(relative for _ in pattern.finditer(code))
        return found
    definition_paths = definition_paths

    owned_definitions = (
        ("fn", "build_cursor", ctx.sab_transport_relative),
        ("fn", "finish_shared_backings", ctx.sab_transport_relative),
        ("fn", "finish_graph_archive", ctx.sab_transport_relative),
        ("fn", "finish_bytecode_image", ctx.sab_transport_relative),
        ("fn", "decode_graph_with_sab_transport", ctx.sab_transport_relative),
        ("fn", "decode_bytecode_image_with_sab_transport", ctx.sab_transport_relative),
        ("struct", "ArchivedBytecodeImage", ctx.sab_transport_relative),
    )

    for kind, ctx.name, expected_path in owned_definitions:
        found = definition_paths(kind, ctx.name)
        if found != [expected_path]:
            ctx.fail(
                "sab-transport-owner",
                f"{kind} {ctx.name} must have one definition owned by {expected_path}; found {found}",
            )

    private_member_patterns = (
        "build_cursor",
        "finish_shared_backings",
        "finish_graph_archive",
        "finish_bytecode_image",
    )

    for ctx.name in private_member_patterns:
        ctx.pattern = re.compile(rf"(?m)^[ \t]*fn[ \t]+{re.escape(ctx.name)}\b")
        if len(ctx.pattern.findall(ctx.sab_transport_code)) != 1:
            ctx.fail(
                "sab-transport-private-member",
                f"{ctx.name} must remain a single module-private SAB transport method",
            )

    entrypoint_patterns = (
        "decode_graph_with_sab_transport",
        "decode_bytecode_image_with_sab_transport",
    )

    for ctx.name in entrypoint_patterns:
        ctx.pattern = re.compile(
            rf"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
            rf"[ \t\n]*\)[ \t\n]+fn[ \t\n]+{re.escape(ctx.name)}\b"
        )
        if len(ctx.pattern.findall(ctx.sab_transport_code)) != 1:
            ctx.fail(
                "sab-transport-entrypoint",
                f"{ctx.name} must remain one runtime-private complete-input entrypoint",
            )

    body_visibility_specs = (
        (
            ctx.graph_decode_code,
            re.compile(
                r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+fn"
                r"[ \t\n]+decode_graph_body\b"
            ),
            "decode_graph_body",
        ),
        (
            ctx.image_decode_code,
            re.compile(
                r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
                r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+fn"
                r"[ \t\n]+decode_bytecode_image_body\b"
            ),
            "decode_bytecode_image_body",
        ),
    )

    for ctx.code, ctx.pattern, ctx.name in body_visibility_specs:
        if len(ctx.pattern.findall(ctx.code)) != 1:
            ctx.fail(
                "sab-decoder-body-visibility",
                f"{ctx.name} must retain its narrow transport-owner visibility",
            )

    call_site_specs = (
        ("build_cursor", [ctx.sab_transport_relative] * 4),
        ("finish_shared_backings", [ctx.sab_transport_relative] * 3),
        ("finish_graph_archive", [ctx.sab_transport_relative] * 3),
        ("finish_bytecode_image", [ctx.sab_transport_relative] * 2),
        ("sab_archive_occurrences", [ctx.image_model_relative, ctx.sab_transport_relative]),
    )

    for ctx.name, expected_paths in call_site_specs:
        found = identifier_paths(ctx.name)
        if sorted(found) != sorted(expected_paths):
            ctx.fail(
                "sab-archive-call-site-set",
                f"{ctx.name} must remain confined to its canonical definition and binders; found {found}",
            )

    cursor_struct = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+struct"
        r"[ \t\n]+SabTransportCursor[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*\{"
        r"[ \t\n]*cursor_wire[ \t\n]*:[ \t\n]*WireCursor[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*,"
        r"[ \t\n]*cursor_writer_occurrences[ \t\n]*:[ \t\n]*&[ \t\n]*'a[ \t\n]*"
        r"\[[ \t\n]*NativeSabToken[ \t\n]*\][ \t\n]*,"
        r"[ \t\n]*cursor_next_occurrence[ \t\n]*:[ \t\n]*usize[ \t\n]*,"
        r"[ \t\n]*cursor_archive[ \t\n]*:[ \t\n]*SabArchiveState[ \t\n]*,?[ \t\n]*\}"
    )

    if len(cursor_struct.findall(ctx.sab_transport_code)) != 1:
        ctx.fail(
            "sab-cursor-shape",
            "SabTransportCursor must retain four private transport-owned fields",
        )

    input_struct = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+SabTransportInput"
        r"[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*\{"
        r"[ \t\n]*transport_wire_bytes[ \t\n]*:[ \t\n]*&[ \t\n]*'a"
        r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*,"
        r"[ \t\n]*transport_writer_occurrences[ \t\n]*:[ \t\n]*&[ \t\n]*'a"
        r"[ \t\n]*\[[ \t\n]*NativeSabToken[ \t\n]*\][ \t\n]*,?[ \t\n]*\}"
    )

    if len(input_struct.findall(ctx.sab_transport_code)) != 1:
        ctx.fail(
            "sab-input-shape",
            "SabTransportInput must retain two private transport-owned fields",
        )

    input_impl_pattern = re.compile(
        r"\bimpl[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*"
        r"SabTransportInput[ \t\n]*<[ \t\n]*'a[ \t\n]*>[ \t\n]*\{"
    )

    input_impl_matches = list(input_impl_pattern.finditer(ctx.sab_transport_code))

    if len(input_impl_matches) != 1:
        ctx.fail(
            "sab-input-implementation-set",
            "SabTransportInput must have one reviewed transport-owned implementation",
        )
    else:
        ctx.match = input_impl_matches[0]
        open_offset = ctx.match.end() - 1
        depth = 0
        close_offset = None
        for offset in range(open_offset, len(ctx.sab_transport_code)):
            character = ctx.sab_transport_code[offset]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    close_offset = offset + 1
                    break
        if close_offset is None:
            ctx.fail(
                "sab-input-implementation-set",
                "SabTransportInput implementation has no balanced closing brace",
            )
        else:
            actual_impl = " ".join(ctx.sab_transport_code[ctx.match.start():close_offset].split())
            expected_impl = " ".join(
                "\n            impl<'a> SabTransportInput<'a> {\n                #[must_use]\n                pub(in crate::engine::code) const fn new(\n                    wire: &'a [u8],\n                    writer_occurrences: &'a [NativeSabToken],\n                ) -> Self {\n                    Self {\n                        transport_wire_bytes: wire,\n                        transport_writer_occurrences: writer_occurrences,\n                    }\n                }\n                fn build_cursor(\n                    self,\n                    mode: ReaderMode,\n                    wire_limits: WireLimits,\n                    graph_limits: GraphLimits,\n                ) -> Result<SabTransportCursor<'a>, SabArchiveError> {\n                    Ok(SabTransportCursor {\n                        cursor_wire: WireCursor::new(self.transport_wire_bytes, mode, wire_limits)?,\n                        cursor_writer_occurrences: self.transport_writer_occurrences,\n                        cursor_next_occurrence: 0,\n                        cursor_archive: SabArchiveState::new(graph_limits),\n                    })\n                }\n                #[cfg(test)]\n                fn into_cursor_for_test(\n                    self,\n                    mode: ReaderMode,\n                    wire_limits: WireLimits,\n                    graph_limits: GraphLimits,\n                ) -> Result<SabTransportCursor<'a>, SabArchiveError> {\n                    self.build_cursor(mode, wire_limits, graph_limits)\n                }\n            }\n            ".split()
            )
            if actual_impl != expected_impl:
                ctx.fail(
                    "sab-input-implementation-set",
                    "SabTransportInput implementation drifted from its reviewed inseparable surface",
                )

    def normalized_function(code: str, name: str) -> str | None:
        pattern = re.compile(
            rf"\bpub[ \t\n]*\([^)]*\)[ \t\n]+fn[ \t\n]+{re.escape(name)}\b"
        )
        matches = list(pattern.finditer(code))
        if len(matches) != 1:
            return None
        match = matches[0]
        open_offset = code.find("{", match.end())
        if open_offset < 0:
            return None
        depth = 0
        close_offset = None
        for offset in range(open_offset, len(code)):
            character = code[offset]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    close_offset = offset + 1
                    break
        if close_offset is None:
            return None
        return " ".join(code[match.start():close_offset].split())
    normalized_function = normalized_function

    reviewed_entrypoints = deepcopy(evidence.REVIEWED_ENTRYPOINTS)

    for ctx.name, expected_source in reviewed_entrypoints:
        actual = normalized_function(ctx.sab_transport_code, ctx.name)
        ctx.expected = " ".join(expected_source.split())
        if actual != ctx.expected:
            ctx.fail(
                "sab-transport-entrypoint-body",
                f"{ctx.name} drifted from its reviewed complete-input decode and finalization path",
            )

    transport_field_counts = deepcopy(evidence.TRANSPORT_FIELD_COUNTS)

    for ctx.name, ctx.expected in transport_field_counts:
        found = identifier_paths(ctx.name)
        if found != [ctx.sab_transport_relative] * ctx.expected:
            ctx.fail(
                "sab-transport-field-escape",
                f"{ctx.name} must appear only in its reviewed transport-owned operations; found {found}",
            )

    cursor_alias = re.compile(
        r"\bSabTransportCursor[ \t\n]+as[ \t\n]+"
        r"|\btype[ \t\n]+(?:r#)?[A-Za-z_][A-Za-z0-9_]*[ \t\n]*"
        r"(?:<[^;=]*>)?[ \t\n]*=[^;]*\bSabTransportCursor\b"
    )

    for ctx.match in cursor_alias.finditer(ctx.sab_transport_code):
        ctx.fail(
            "sab-cursor-alias",
            "the transport owner must not alias its cursor around construction gates; found "
            + ctx.location(ctx.sab_transport_relative, ctx.sab_transport_source, ctx.match.start()),
        )

    cursor_impl_pattern = re.compile(
        r"\bimpl\b[^{};]*\bSabTransportCursor\b[^{};]*\{",
        re.DOTALL,
    )

    function_name_token = re.compile(r"\bfn[ \t\n]+((?:r#)?[^\s(<{]+)")

    cursor_impl_matches = list(cursor_impl_pattern.finditer(ctx.sab_transport_code))

    cursor_impl_headers = [
        " ".join(ctx.sab_transport_code[match.start():match.end()].split())
        for match in cursor_impl_matches
    ]

    if cursor_impl_headers != ["impl<'a> SabTransportCursor<'a> {"]:
        ctx.fail(
            "sab-cursor-implementation-set",
            "SabTransportCursor must have one transport-owned inherent implementation; "
            f"found {cursor_impl_headers}",
        )
    else:
        ctx.match = cursor_impl_matches[0]
        open_offset = ctx.match.end() - 1
        depth = 0
        close_offset = None
        for offset in range(open_offset, len(ctx.sab_transport_code)):
            character = ctx.sab_transport_code[offset]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    close_offset = offset + 1
                    break
        if close_offset is None:
            ctx.fail(
                "sab-cursor-implementation-set",
                "SabTransportCursor implementation has no balanced closing brace",
            )
            cursor_impl_code = ""
        else:
            cursor_impl_code = ctx.sab_transport_code[open_offset:close_offset]

        cursor_methods: list[str] = []
        for method in function_name_token.finditer(cursor_impl_code):
            prefix = cursor_impl_code[:method.start()]
            if prefix.count("{") - prefix.count("}") == 1:
                cursor_methods.append(method.group(1))
        expected_cursor_methods = [
            "position",
            "mode",
            "remaining",
            "read_u8",
            "read_u16_le",
            "read_bytes",
            "read_tag",
            "read_uleb128",
            "read_i32",
            "read_f64",
            "read_header",
            "read_string",
            "validate_wire_end",
            "record_shared_array_buffer",
            "finish_shared_backings",
            "finish_graph_archive",
            "finish_graph_archive_for_test",
            "finish_bytecode_image",
        ]
        if cursor_methods != expected_cursor_methods:
            ctx.fail(
                "sab-cursor-method-set",
                "SabTransportCursor method surface drifted; "
                f"found {cursor_methods}",
            )

        binary_object_cursor_methods = expected_cursor_methods[:13]
        for ctx.name in binary_object_cursor_methods:
            ctx.pattern = re.compile(
                rf"(?m)^[ \t]*pub[ \t]*\([ \t]*in[ \t]+crate::engine::code::binary_object"
                rf"[ \t]*\)[ \t]+(?:const[ \t]+)?fn[ \t]+{re.escape(ctx.name)}\b"
            )
            if len(ctx.pattern.findall(cursor_impl_code)) != 1:
                ctx.fail(
                    "sab-cursor-method-visibility",
                    f"SabTransportCursor::{ctx.name} must remain binary_object-private",
                )
        if len(
            re.findall(
                r"(?m)^[ \t]*pub[ \t]*\([ \t]*super[ \t]*\)[ \t]+fn"
                r"[ \t]+record_shared_array_buffer\b",
                cursor_impl_code,
            )
        ) != 1:
            ctx.fail(
                "sab-cursor-method-visibility",
                "record_shared_array_buffer must remain graph-private",
            )
        for ctx.name in (
            "finish_shared_backings",
            "finish_graph_archive",
            "finish_graph_archive_for_test",
            "finish_bytecode_image",
        ):
            ctx.pattern = re.compile(rf"(?m)^[ \t]*fn[ \t]+{re.escape(ctx.name)}\b")
            if len(ctx.pattern.findall(cursor_impl_code)) != 1:
                ctx.fail(
                    "sab-cursor-method-visibility",
                    f"SabTransportCursor::{ctx.name} must remain module-private",
                )

        test_graph_finalizer = re.compile(
            r"#\[cfg\(test\)\][ \t\n]*fn[ \t\n]+finish_graph_archive_for_test"
            r"[ \t\n]*\([ \t\n]*self[ \t\n]*,[ \t\n]*graph[ \t\n]*:"
            r"[ \t\n]*WireGraph[ \t\n]*,[ \t\n]*\)[ \t\n]*->"
            r"[ \t\n]*Result[ \t\n]*<[ \t\n]*ArchivedWireGraph[ \t\n]*,"
            r"[ \t\n]*SabArchiveError[ \t\n]*>[ \t\n]*\{"
            r"[ \t\n]*self[ \t\n]*\.[ \t\n]*finish_graph_archive"
            r"[ \t\n]*\([ \t\n]*graph[ \t\n]*\)[ \t\n]*\}"
        )
        if len(test_graph_finalizer.findall(cursor_impl_code)) != 1:
            ctx.fail(
                "sab-test-finalizer-body",
                "the graph-finalizer test shim must remain cfg(test) and delegate exactly once",
            )

    top_level_functions: list[str] = []

    for ctx.match in function_name_token.finditer(ctx.sab_transport_code):
        prefix = ctx.sab_transport_code[:ctx.match.start()]
        if prefix.count("{") == prefix.count("}"):
            top_level_functions.append(ctx.match.group(1))

    if top_level_functions != [
        "decode_graph_with_sab_transport",
        "decode_bytecode_image_with_sab_transport",
    ]:
        ctx.fail(
            "sab-transport-free-function-set",
            "SAB transport owner must expose only the two reviewed complete-input free functions; "
            f"found {top_level_functions}",
        )

    forbidden_top_level_items: list[str] = []

    item_keyword = re.compile(r"\b(union|trait|type|const|static|extern)\b")

    for ctx.match in item_keyword.finditer(ctx.sab_transport_code):
        prefix = ctx.sab_transport_code[:ctx.match.start()]
        if prefix.count("{") == prefix.count("}"):
            forbidden_top_level_items.append(ctx.match.group(1))

    if forbidden_top_level_items:
        ctx.fail(
            "sab-transport-top-level-item-set",
            "SAB transport owner gained a forbidden top-level item; "
            f"found {forbidden_top_level_items}",
        )

    owner_public_use = re.compile(
        r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+use\b"
    )

    for ctx.match in owner_public_use.finditer(ctx.sab_transport_code):
        prefix = ctx.sab_transport_code[:ctx.match.start()]
        if prefix.count("{") == prefix.count("}"):
            ctx.fail(
                "sab-transport-top-level-item-set",
                "SAB transport owner must not re-export transport internals",
            )

    graph_archive_struct = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+ArchivedWireGraph[ \t\n]*\{"
        r"[ \t\n]*archived_graph_payload[ \t\n]*:[ \t\n]*WireGraph[ \t\n]*,"
        r"[ \t\n]*archived_graph_shared_backings[ \t\n]*:[ \t\n]*Box[ \t\n]*<"
        r"[ \t\n]*\[[ \t\n]*SharedBackingDescriptor[ \t\n]*\][ \t\n]*>"
        r"[ \t\n]*,?[ \t\n]*\}"
    )

    if len(graph_archive_struct.findall(ctx.sab_transport_code)) != 1:
        ctx.fail(
            "sab-graph-aggregate-shape",
            "ArchivedWireGraph must retain exactly two private transport-owned fields",
        )

    graph_archive_brace_paths: list[str] = []

    graph_archive_brace_pattern = re.compile(r"\bArchivedWireGraph[ \t\n]*\{")

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.code = ctx.binary_code_cache[ctx.path]
        graph_archive_brace_paths.extend(
            ctx.relative for _ in graph_archive_brace_pattern.finditer(ctx.code)
        )

    if graph_archive_brace_paths != [ctx.sab_transport_relative] * 3:
        ctx.fail(
            "sab-graph-construction-set",
            "ArchivedWireGraph must have one declaration, literal, and reviewed implementation; "
            f"found {graph_archive_brace_paths}",
        )

    graph_archive_field_counts = (
        ("archived_graph_payload", 3),
        ("archived_graph_shared_backings", 4),
    )

    for ctx.name, ctx.expected in graph_archive_field_counts:
        found = identifier_paths(ctx.name)
        if found != [ctx.sab_transport_relative] * ctx.expected:
            ctx.fail(
                "sab-graph-field-escape",
                f"{ctx.name} must appear only in the reviewed field, binder, and test projection; "
                f"found {found}",
            )

    graph_archive_impl_pattern = re.compile(
        r"\bimpl\b[^{};]*\bArchivedWireGraph\b[^{};]*\{",
        re.DOTALL,
    )

    graph_archive_impl_sites: list[tuple[str, str, re.Match[str]]] = []

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.code = ctx.binary_code_cache[ctx.path]
        graph_archive_impl_sites.extend(
            (ctx.relative, ctx.code, match) for match in graph_archive_impl_pattern.finditer(ctx.code)
        )

    if len(graph_archive_impl_sites) != 1 or graph_archive_impl_sites[0][0] != ctx.sab_transport_relative:
        ctx.fail(
            "sab-graph-aggregate-escape",
            "ArchivedWireGraph must have one reviewed transport-owned inherent implementation",
        )
    else:
        ctx._, ctx.code, ctx.match = graph_archive_impl_sites[0]
        open_offset = ctx.match.end() - 1
        depth = 0
        close_offset = None
        for offset in range(open_offset, len(ctx.code)):
            character = ctx.code[offset]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    close_offset = offset + 1
                    break
        if close_offset is None:
            ctx.fail(
                "sab-graph-aggregate-escape",
                "ArchivedWireGraph implementation has no balanced closing brace",
            )
        else:
            actual_impl = " ".join(ctx.code[ctx.match.start():close_offset].split())
            expected_impl = " ".join(
                '\n            impl ArchivedWireGraph {\n                #[must_use]\n                pub(in crate::engine::code::binary_object) const fn shared_backing_count(&self) -> usize {\n                    self.archived_graph_shared_backings.len()\n                }\n                #[cfg(test)]\n                pub(in crate::engine::code::binary_object) const fn test_graph(&self) -> &WireGraph {\n                    &self.archived_graph_payload\n                }\n                #[cfg(test)]\n                pub(super) fn test_shared_backing_descriptor(\n                    &self,\n                    backing: ArchiveBackingId,\n                ) -> Option<SharedBackingDescriptor> {\n                    self.archived_graph_shared_backings\n                        .get(backing.as_usize())\n                        .copied()\n                }\n            }\n            '.split()
            )
            if actual_impl != expected_impl:
                ctx.fail(
                    "sab-graph-aggregate-escape",
                    "ArchivedWireGraph implementation drifted from its reviewed non-splitting surface",
                )

    archive_struct = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*\)[ \t\n]+struct[ \t\n]+ArchivedBytecodeImage[ \t\n]*\{"
        r"[ \t\n]*archived_image_payload[ \t\n]*:[ \t\n]*BytecodeImage[ \t\n]*,"
        r"[ \t\n]*archived_image_shared_backings[ \t\n]*:[ \t\n]*Box[ \t\n]*<"
        r"[ \t\n]*\[[ \t\n]*SharedBackingDescriptor[ \t\n]*\][ \t\n]*>"
        r"[ \t\n]*,?[ \t\n]*\}"
    )

    if len(archive_struct.findall(ctx.sab_transport_code)) != 1:
        ctx.fail(
            "sab-image-aggregate-shape",
            "ArchivedBytecodeImage must retain exactly two private transport-owned fields",
        )

    archive_brace_paths: list[str] = []

    archive_brace_pattern = re.compile(r"\bArchivedBytecodeImage[ \t\n]*\{")

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.code = ctx.binary_code_cache[ctx.path]
        archive_brace_paths.extend(ctx.relative for _ in archive_brace_pattern.finditer(ctx.code))

    if archive_brace_paths != [ctx.sab_transport_relative] * 3:
        ctx.fail(
            "sab-image-construction-set",
            "ArchivedBytecodeImage must have one declaration, literal, and reviewed implementation; "
            f"found {archive_brace_paths}",
        )

    archive_field_counts = (
        ("archived_image_payload", 3),
        ("archived_image_shared_backings", 4),
    )

    for ctx.name, ctx.expected in archive_field_counts:
        found = identifier_paths(ctx.name)
        if found != [ctx.sab_transport_relative] * ctx.expected:
            ctx.fail(
                "sab-image-field-escape",
                f"{ctx.name} must appear only in the reviewed field, binder, and test projection; "
                f"found {found}",
            )

    archive_impl_pattern = re.compile(
        r"\bimpl\b[^{};]*\bArchivedBytecodeImage\b[^{};]*\{",
        re.DOTALL,
    )

    archive_impl_sites: list[tuple[str, str, re.Match[str]]] = []

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.code = ctx.binary_code_cache[ctx.path]
        archive_impl_sites.extend((ctx.relative, ctx.code, match) for match in archive_impl_pattern.finditer(ctx.code))

    if len(archive_impl_sites) != 1 or archive_impl_sites[0][0] != ctx.sab_transport_relative:
        ctx.fail(
            "sab-image-aggregate-escape",
            "ArchivedBytecodeImage must have one reviewed transport-owned inherent implementation",
        )
    else:
        ctx._, ctx.code, ctx.match = archive_impl_sites[0]
        open_offset = ctx.match.end() - 1
        depth = 0
        close_offset = None
        for offset in range(open_offset, len(ctx.code)):
            character = ctx.code[offset]
            if character == "{":
                depth += 1
            elif character == "}":
                depth -= 1
                if depth == 0:
                    close_offset = offset + 1
                    break
        if close_offset is None:
            ctx.fail(
                "sab-image-aggregate-escape",
                "ArchivedBytecodeImage implementation has no balanced closing brace",
            )
        else:
            actual_impl = " ".join(ctx.code[ctx.match.start():close_offset].split())
            expected_impl = " ".join(
                '\n            impl ArchivedBytecodeImage {\n                #[must_use]\n                pub(in crate::engine::code::binary_object) const fn shared_backing_count(&self) -> usize {\n                    self.archived_image_shared_backings.len()\n                }\n                #[cfg(test)]\n                pub(in crate::engine::code::binary_object) const fn test_image(&self) -> &BytecodeImage {\n                    &self.archived_image_payload\n                }\n                #[cfg(test)]\n                pub(in crate::engine::code::binary_object) fn test_shared_backing_descriptor(\n                    &self,\n                    backing: ArchiveBackingId,\n                ) -> Option<SharedBackingDescriptor> {\n                    self.archived_image_shared_backings\n                        .get(backing.as_usize())\n                        .copied()\n                }\n            }\n            '.split()
            )
            if actual_impl != expected_impl:
                ctx.fail(
                    "sab-image-aggregate-escape",
                    "ArchivedBytecodeImage implementation drifted from its reviewed non-splitting surface",
                )

    for ctx.path in ctx.binary_sources:
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        if ctx.path.is_symlink() or not ctx.path.is_file():
            ctx.fail("linked-source", f"{ctx.relative} must be a regular file")
            continue
        ctx.source = ctx.binary_source_cache[ctx.path]
        ctx.code = ctx.binary_code_cache[ctx.path]
        forbidden_patterns = (
            (
                "forbidden-vm-dependency",
                re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:engine[ \t\n]*::[ \t\n]*)?(?:r#)?vm\b"),
                "crate::engine::vm",
            ),
            (
                "forbidden-compiler-dependency",
                re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:engine[ \t\n]*::[ \t\n]*)?(?:r#)?compiler\b"),
                "crate::engine::compiler",
            ),
            (
                "forbidden-heap-dependency",
                re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:engine[ \t\n]*::[ \t\n]*)?(?:r#)?heap\b"),
                "crate::engine::heap",
            ),
            (
                "forbidden-runtime-dependency",
                re.compile(
                    r"\buse[ \t\n]+crate[ \t\n]*::[ \t\n]*(?:r#)?runtime\b"
                    r"(?![ \t\n]*::[ \t\n]*binary_object\b)"
                ),
                "crate::engine::heap::runtime",
            ),
            (
                "forbidden-shared-memory-dependency",
                re.compile(r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?shared_memory\b"),
                "crate::engine::heap::shared_memory",
            ),
            (
                "forbidden-parent-dependency",
                re.compile(
                    r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?(?:vm|compiler|heap)\b"
                ),
                "a parent-relative VM/compiler/heap path",
            ),
            (
                "forbidden-shared-memory-dependency",
                re.compile(
                    r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?shared_memory\b"
                ),
                "a parent-relative shared_memory path",
            ),
            (
                "forbidden-shared-memory-runtime-type",
                re.compile(r"\b(?:SharedBufferHandle|SharedBackingStore)\b"),
                "SharedBufferHandle or SharedBackingStore",
            ),
            (
                "forbidden-unsafe-code",
                re.compile(r"\bunsafe\b"),
                "unsafe Rust",
            ),
            (
                "forbidden-non-null-pointer",
                re.compile(r"\bNonNull\b"),
                "NonNull",
            ),
            (
                "forbidden-raw-pointer-type",
                re.compile(r"\*[ \t\n]*(?:const|mut)\b"),
                "a raw pointer type",
            ),
            (
                "forbidden-native-pointer-bridge",
                re.compile(
                    r"\b(?:(?:[A-Za-z_][A-Za-z0-9_]*_)?from_raw_parts(?:_mut|_in)?"
                    r"|into_raw(?:_with_allocator|_parts(?:_with_alloc)?)?)\b"
                ),
                "a native pointer ownership or slice bridge",
            ),
            (
                "forbidden-native-pointer-bridge",
                re.compile(
                    r"\b(?:Box|Vec|Arc|Rc|CString|CStr)"
                    r"[ \t\n]*(?:::[ \t\n]*<[^>{};\n]+>)?"
                    r"[ \t\n]*::[ \t\n]*from_raw(?:_in)?\b"
                ),
                "a native pointer ownership bridge",
            ),
            (
                "forbidden-bytecode-function",
                re.compile(r"\b(?:BytecodeFunction|FunctionBytecodeRef)\b"),
                "BytecodeFunction or FunctionBytecodeRef",
            ),
            (
                "forbidden-runtime-representation",
                re.compile(
                    r"\b(?:Runtime|Context|RuntimeError|FunctionMetadata|FunctionBytecodeData|"
                    r"BytecodeConstant|FunctionBytecodeId|ObjectId|RawValue|UnlinkedFunction|"
                    r"Instruction|Heap|HeapObject|ObjectRef)\b"
                ),
                "a runtime, verifier-draft, VM, or heap representation type",
            ),
            (
                "forbidden-publication-boundary",
                re.compile(
                    r"\b(?:publish_unlinked_function|publish_verified_unlinked_function)\b"
                ),
                "a runtime publication function",
            ),
            (
                "forbidden-crate-alias",
                re.compile(
                    r"\b(?:use[ \t\n]+crate(?:[ \t\n]*::[ \t\n]*engine)?[ \t\n]+as|extern[ \t\n]+crate[ \t\n]+self[ \t\n]+as)\b"
                ),
                "an alias for the crate root",
            ),
        )
        for ctx.code_name, ctx.pattern, ctx.description in forbidden_patterns:
            for ctx.match in ctx.pattern.finditer(ctx.code):
                ctx.fail(
                    ctx.code_name,
                    f"binary_object production sources must not depend on {ctx.description}; found "
                    + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
                )

        grouped_import_pattern = re.compile(
            r"\buse[ \t\n]+crate[ \t\n]*::[ \t\n]*(?:engine[ \t\n]*::[ \t\n]*)?\{(?P<body>.*?)\}[ \t\n]*;",
            re.DOTALL,
        )
        for grouped in grouped_import_pattern.finditer(ctx.code):
            ctx.body = grouped.group("body")
            if re.search(r"\b(?:r#)?vm\b", ctx.body):
                ctx.fail(
                    "forbidden-vm-dependency",
                    "binary_object production sources must not import crate::engine::vm through a grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
            if re.search(r"\b(?:r#)?compiler\b", ctx.body):
                ctx.fail(
                    "forbidden-compiler-dependency",
                    "binary_object production sources must not import crate::engine::compiler through a grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
            if re.search(r"\b(?:r#)?heap\b", ctx.body):
                ctx.fail(
                    "forbidden-heap-dependency",
                    "binary_object production sources must not import crate::engine::heap through a grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
            if re.search(r"\b(?:r#)?runtime\b", ctx.body):
                ctx.fail(
                    "forbidden-runtime-dependency",
                    "binary_object production sources must not import crate::engine::heap::runtime through a grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
            if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?shared_memory\b", ctx.body):
                ctx.fail(
                    "forbidden-shared-memory-dependency",
                    "binary_object production sources must not import crate::engine::heap::shared_memory through a grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
            if re.search(r"\bself[ \t\n]+as\b", ctx.body):
                ctx.fail(
                    "forbidden-crate-alias",
                    "binary_object production sources must not alias the crate root through a grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )

        parent_grouped_import_pattern = re.compile(
            r"\buse[ \t\n]+(?:super[ \t\n]*::[ \t\n]*)+"
            r"\{(?P<body>.*?)\}[ \t\n]*;",
            re.DOTALL,
        )
        for grouped in parent_grouped_import_pattern.finditer(ctx.code):
            grouped_body = grouped.group("body")
            if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?shared_memory\b", grouped_body):
                ctx.fail(
                    "forbidden-shared-memory-dependency",
                    "binary_object production sources must not import shared_memory through a parent-relative grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
            if re.search(r"(?:^|[,{}])[ \t\n]*(?:r#)?heap\b", grouped_body):
                ctx.fail(
                    "forbidden-heap-dependency",
                    "binary_object production sources must not import heap through a parent-relative grouped use; found "
                    + ctx.location(ctx.relative, ctx.source, grouped.start()),
                )
