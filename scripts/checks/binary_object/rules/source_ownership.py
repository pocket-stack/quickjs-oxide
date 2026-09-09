"""Source ownership checks, in the ordered boundary scan."""
from __future__ import annotations

from pathlib import Path
import re
from copy import deepcopy

from ..evidence import source_ownership as evidence


def check(ctx):
    production_sources: list[Path] = []
    for relative in ("src", "apps", "adapters", "conformance", "examples", "tests"):
        src_root = ctx.root / relative
        if src_root.is_symlink() or not src_root.is_dir():
            if not ctx.self_test_marker_authorized:
                ctx.fail("missing-source", f"{relative} must be a regular directory")
        else:
            production_sources.extend(sorted(src_root.rglob("*.rs")))

    allowed_assertion_namespace_imports = {
        "use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};",
        "use std::panic::{self, AssertUnwindSafe};",
    }

    for ctx.path in production_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.code = ctx.rust_code_only(ctx.path.read_text(encoding="utf-8"))
        if re.search(
            r"\bmacro_rules[ \t\n]*![ \t\n]*(?:r#)?(?:assert|assert_eq|assert_ne|matches|panic)\b",
            ctx.code,
        ):
            ctx.fail(
                "stage3e-runtime-evidence",
                f"{ctx.relative} must not define an assertion macro that can enter Stage3E test scope",
            )
        for statement in re.findall(
            r"(?ms)^[ \t]*(?:(?:pub(?:[ \t]*\([^)]*\))?)[ \t]+)?use\b.*?;",
            ctx.code,
        ):
            if (
                ctx.assertion_shadow_pattern.search(statement)
                and " ".join(statement.split()) not in allowed_assertion_namespace_imports
            ):
                ctx.fail(
                    "stage3e-runtime-evidence",
                    f"{ctx.relative} must not import an assertion macro that can shadow Stage3E test evidence",
                )

    facade_name_pattern = re.compile(
        r"\b(?:ScalarValueDraft|ScalarUnaryOp|ScalarScriptReadError|ScalarStringDraft|"
        r"decode_trusted_scalar_script|DetachedAtomName|DetachedPrimitive|OrdinaryLeafDraft|"
        r"OrdinaryLeafApplyKind|OrdinaryLeafMetadataDraft|OrdinaryLeafOp|OrdinaryLeafReadError|"
        r"RootFunctionConstantSelector|decode_trusted_ordinary_leaf)\b"
    )

    for ctx.path in production_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        if ctx.relative.startswith("src/engine/code/binary_object/"):
            continue
        ctx.source = ctx.path.read_text(encoding="utf-8")
        ctx.code = ctx.rust_code_only(ctx.source)
        binary_mentions = list(re.finditer(r"\bbinary_object\b", ctx.code))
        if ctx.relative == "src/engine/code/mod.rs":
            if len(binary_mentions) != 1:
                ctx.fail(
                    "binary-object-consumer-set",
                    "src/engine/code/mod.rs may name binary_object only in its private module declaration",
                )
        elif ctx.relative == ctx.consumer_relative and ctx.consumer_exists:
            if len(binary_mentions) != 1:
                ctx.fail(
                    "binary-object-consumer-set",
                    f"{ctx.consumer_relative} must remain the sole reviewed codec consumer",
                )
        elif binary_mentions:
            for ctx.match in binary_mentions:
                ctx.fail(
                    "binary-object-consumer-set",
                    "only binary_object_publish.rs may consume binary_object; found "
                    + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
                )

        if ctx.relative not in {ctx.binary_root_relative, ctx.consumer_relative}:
            for ctx.match in facade_name_pattern.finditer(ctx.code):
                ctx.fail(
                    "binary-object-facade-consumer-set",
                    "only binary_object_publish.rs may name the scalar-script or ordinary-leaf facade; found "
                    + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
                )

        if ctx.relative not in {ctx.bytecode_publish_relative, ctx.consumer_relative}:
            for ctx.match in re.finditer(r"\bverify_unlinked_ordinary_leaf\b", ctx.code):
                ctx.fail(
                    "ordinary-leaf-verifier-consumer-set",
                    "only binary_object_publish.rs may call the dedicated ordinary-leaf verifier; found "
                    + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
                )

        path_attribute = re.compile(
            r"#[ \t\n]*\[[ \t\n]*path[ \t\n]*=[ \t\n]*[^\]]*binary_object[^\]]*\]"
        )
        for ctx.match in path_attribute.finditer(ctx.source):
            ctx.fail(
                "binary-object-consumer-set",
                "path attributes must not create an alternate binary_object consumer; found "
                + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
            )

    cursor_relative = "src/engine/code/binary_object/read_cursor.rs"

    cursor_source = ctx.read_source(cursor_relative)

    cursor_code = ctx.rust_code_only(cursor_source)

    sealed_modules = re.findall(r"(?m)^[ \t]*mod[ \t]+sealed[ \t]*\{", cursor_code)

    if len(sealed_modules) != 1 or re.search(
        r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+mod[ \t\n]+sealed\b",
        cursor_code,
    ):
        ctx.fail(
            "common-cursor-unsealed",
            f"{cursor_relative} must contain exactly one private `mod sealed`",
        )

    checked_trait_pattern = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+trait[ \t\n]+"
        r"CheckedReadCursor[ \t\n]*<[ \t\n]*'input[ \t\n]*>[ \t\n]*:"
        r"[ \t\n]*sealed[ \t\n]*::[ \t\n]*Sealed\b"
    )

    if len(checked_trait_pattern.findall(cursor_code)) != 1:
        ctx.fail(
            "common-cursor-unsealed",
            f"{cursor_relative} must declare one binary_object-private CheckedReadCursor sealed by sealed::Sealed",
        )

    forbidden_cursor_capability = re.compile(
        r"\bu64\b|\ballows_shared_array_buffers\b|\brecord_shared_array_buffer\b"
    )

    for ctx.match in forbidden_cursor_capability.finditer(cursor_code):
        ctx.fail(
            "forbidden-common-cursor-capability",
            "CheckedReadCursor must not expose raw u64 or SAB capability hooks; found "
            + ctx.location(cursor_relative, cursor_source, ctx.match.start()),
        )

    sealed_cursor_alias = re.compile(r"\bSealed[ \t\n]+as[ \t\n]+")

    for ctx.match in sealed_cursor_alias.finditer(cursor_code):
        ctx.fail(
            "common-cursor-seal-alias",
            "the common cursor seal must not be renamed before an implementation; found "
            + ctx.location(cursor_relative, cursor_source, ctx.match.start()),
        )

    checked_impl_pattern = re.compile(
        r"\bimpl\b(?P<header>[^{};]*\bCheckedReadCursor\b"
        r"[ \t\n]*(?:(?:::[ \t\n]*)?<[^{};>]*>)?[ \t\n]+for\b[^{};]*)\{",
        re.DOTALL,
    )

    checked_cursor_alias = re.compile(r"\bCheckedReadCursor[ \t\n]+as[ \t\n]+")

    checked_impl_headers: list[tuple[str, str]] = []

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.source = ctx.binary_source_cache[ctx.path]
        ctx.code = ctx.binary_code_cache[ctx.path]
        for ctx.match in checked_cursor_alias.finditer(ctx.code):
            ctx.fail(
                "common-cursor-trait-alias",
                "CheckedReadCursor must not be renamed before an implementation; found "
                + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
            )
        for ctx.match in checked_impl_pattern.finditer(ctx.code):
            header = " ".join(("impl" + ctx.match.group("header") + " {").split())
            checked_impl_headers.append((ctx.relative, header))

    expected_checked_impl_headers = [
        (
            cursor_relative,
            "impl<'input> CheckedReadCursor<'input> for SabTransportCursor<'input> {",
        ),
        (
            cursor_relative,
            "impl<'input> CheckedReadCursor<'input> for WireCursor<'input> {",
        ),
    ]

    if sorted(checked_impl_headers) != sorted(expected_checked_impl_headers):
        ctx.fail(
            "common-cursor-implementation-set",
            "CheckedReadCursor must have only the two canonical implementations in read_cursor.rs; "
            f"found {checked_impl_headers}",
        )

    sealed_impl_pattern = re.compile(
        r"\bimpl\b(?P<header>[^{};]*\bSealed\b"
        r"[ \t\n]*(?:::[ \t\n]*<[^{};>]*>)?[ \t\n]+for\b[^{};]*)\{",
        re.DOTALL,
    )

    sealed_impl_headers = [
        " ".join(("impl" + match.group("header") + " {").split())
        for match in sealed_impl_pattern.finditer(cursor_code)
    ]

    expected_sealed_impl_headers = [
        "impl Sealed for SabTransportCursor<'_> {",
        "impl Sealed for WireCursor<'_> {",
    ]

    if sorted(sealed_impl_headers) != sorted(expected_sealed_impl_headers):
        ctx.fail(
            "common-cursor-seal-implementation-set",
            "the private common cursor seal must have only the two canonical implementations; "
            f"found {sealed_impl_headers}",
        )

    graph_decode_relative = "src/engine/code/binary_object/graph/decode.rs"

    image_decode_relative = "src/engine/code/binary_object/bytecode_image/decode/mod.rs"

    ctx.sab_transport_relative = "src/engine/code/binary_object/graph/sab_transport.rs"

    ctx.image_model_relative = "src/engine/code/binary_object/bytecode_image/model.rs"

    image_atoms_relative = "src/engine/code/binary_object/bytecode_image/atoms.rs"

    graph_decode_source = ctx.read_source(graph_decode_relative)

    ctx.graph_decode_code = ctx.rust_code_only(graph_decode_source)

    image_decode_source = ctx.read_source(image_decode_relative)

    ctx.image_decode_code = ctx.rust_code_only(image_decode_source)

    ctx.sab_transport_source = ctx.read_source(ctx.sab_transport_relative)

    ctx.sab_transport_code = ctx.rust_code_only(ctx.sab_transport_source)

    image_model_source = ctx.read_source(ctx.image_model_relative)

    image_model_code = ctx.rust_code_only(image_model_source)

    image_atoms_source = ctx.read_source(image_atoms_relative)

    image_atoms_code = ctx.rust_code_only(image_atoms_source)

    if (
        ctx.is_full_binary_inventory
        and ctx.normalized_code_sha256(image_model_code)
        != "78c0c5f66234f50549d91ebc8bc8bf249701a514fa6465c14f77ec1f263205a5"
    ):
        ctx.fail(
            "bytecode-image-model-seal",
            "the atom-bearing bytecode image model drifted from its reviewed normalized implementation",
        )

    image_atom_declaration = re.compile(
        r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+enum"
        r"[ \t\n]+ImageAtom\b"
    )

    all_image_atom_declarations = re.compile(
        r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+enum[ \t\n]+ImageAtom\b"
    )

    if (
        len(image_atom_declaration.findall(image_atoms_code)) != 1
        or len(all_image_atom_declarations.findall(image_atoms_code)) != 1
    ):
        ctx.fail(
            "image-atom-visibility",
            "ImageAtom must remain visible only to its bytecode_image parent module",
        )

    eval_atom_constant = re.compile(
        r"(?m)^[ \t]*const[ \t]+PINNED_EVAL_ATOM_RAW[ \t]*:[ \t]*u32"
        r"[ \t]*=[ \t]*84[ \t]*;[ \t]*$"
    )

    if len(eval_atom_constant.findall(image_model_code)) != 1:
        ctx.fail(
            "scalar-script-atom-predicate",
            "the pinned <eval> identity must remain one private model constant with raw value 84",
        )

    binary_object_visibility = (
        r"pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)"
    )

    bytecode_image_impl_pattern = re.compile(
        r"\bimpl[ \t\n]+BytecodeImage[ \t\n]*\{"
    )

    model_bytecode_image_impl_code, ctx._, ctx._ = ctx.unique_braced_item(
        image_model_code,
        bytecode_image_impl_pattern,
        "bytecode-image-visible-method-set",
        "model-owned BytecodeImage implementation",
    )

    bytecode_image_impl_paths = []

    for ctx.path, ctx.code in ctx.binary_code_cache.items():
        bytecode_image_impl_paths.extend(
            ctx.path.relative_to(ctx.root).as_posix()
            for _ in bytecode_image_impl_pattern.finditer(ctx.code)
        )

    if bytecode_image_impl_paths != [ctx.image_model_relative]:
        ctx.fail(
            "bytecode-image-visible-method-set",
            "BytecodeImage implementations must remain in the reviewed model owner; "
            f"found {bytecode_image_impl_paths}",
        )

    bytecode_image_visible_method_pattern = re.compile(
        r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
        r"(?:const[ \t\n]+)?fn[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )

    def visible_method_set(code: str) -> list[tuple[str, str]]:
        return [
            (
                " ".join(match.group("visibility").split()),
                match.group("name"),
            )
            for match in bytecode_image_visible_method_pattern.finditer(code)
        ]
    visible_method_set = visible_method_set

    expected_model_bytecode_image_methods = deepcopy(evidence.EXPECTED_MODEL_BYTECODE_IMAGE_METHODS)

    model_bytecode_image_methods = visible_method_set(model_bytecode_image_impl_code)

    if model_bytecode_image_methods not in (expected_model_bytecode_image_methods, []):
        ctx.fail(
            "bytecode-image-visible-method-set",
            "BytecodeImage may expose only its reviewed model accessors",
        )

    null_name_predicate = re.compile(
        rf"\b{binary_object_visibility}[ \t\n]+const[ \t\n]+fn"
        r"[ \t\n]+name_is_null[ \t\n]*\([ \t\n]*&self[ \t\n]*\)"
        r"[ \t\n]*->[ \t\n]*bool[ \t\n]*\{[ \t\n]*matches!"
        r"[ \t\n]*\([ \t\n]*self[ \t\n]*\.[ \t\n]*name[ \t\n]*,"
        r"[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Null[ \t\n]*\)"
        r"[ \t\n]*\}"
    )

    eval_name_predicate = re.compile(
        rf"\b{binary_object_visibility}[ \t\n]+const[ \t\n]+fn"
        r"[ \t\n]+name_is_pinned_eval[ \t\n]*\([ \t\n]*&self[ \t\n]*\)"
        r"[ \t\n]*->[ \t\n]*bool[ \t\n]*\{[ \t\n]*match"
        r"[ \t\n]+self[ \t\n]*\.[ \t\n]*name[ \t\n]*\{"
        r"[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Predefined[ \t\n]*\("
        r"[ \t\n]*atom[ \t\n]*\)[ \t\n]*=>[ \t\n]*atom[ \t\n]*\."
        r"[ \t\n]*raw[ \t\n]*\([ \t\n]*\)[ \t\n]*==[ \t\n]*PINNED_EVAL_ATOM_RAW"
        r"[ \t\n]*,[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Null[ \t\n]*\|"
        r"[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Index[ \t\n]*\([ \t\n]*_[ \t\n]*\)"
        r"[ \t\n]*\|[ \t\n]*ImageAtom[ \t\n]*::[ \t\n]*Dynamic"
        r"[ \t\n]*\([ \t\n]*_[ \t\n]*\)[ \t\n]*=>[ \t\n]*false[ \t\n]*,?"
        r"[ \t\n]*\}[ \t\n]*\}"
    )

    if (
        len(null_name_predicate.findall(image_model_code)) != (2 if ctx.is_full_binary_inventory else 1)
        or len(eval_name_predicate.findall(image_model_code)) != 1
    ):
        ctx.fail(
            "scalar-script-atom-predicate",
            "the model must expose only the reviewed null-local, null-function-name, and pinned-<eval> boolean predicates",
        )

    image_atom_export = re.compile(
        r"\bpub(?:[ \t\n]*\([^)]*\))?[ \t\n]+use\b[^;]*"
        r"\b(?:ImageAtom|PinnedAtomId)\b",
        re.DOTALL,
    )

    raw_atom_return = re.compile(
        rf"\b{binary_object_visibility}[^;{{}}]*\bfn\b[^;{{}}]*->[ \t\n]*"
        r"(?:ImageAtom|PinnedAtomId)\b"
    )

    visible_function_pattern = re.compile(
        r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
        r"(?:(?:const|async|unsafe|extern)[ \t\n]+)*fn[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b[^;{}]*\{"
    )

    atom_sensitive_visible_sites: list[tuple[str, str, str]] = []

    for ctx.path in ctx.binary_sources:
        if ctx.path.is_symlink() or not ctx.path.is_file():
            continue
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        ctx.source = ctx.binary_source_cache[ctx.path]
        ctx.code = ctx.binary_code_cache[ctx.path]
        for ctx.match in image_atom_export.finditer(ctx.code):
            ctx.fail(
                "image-atom-reexport",
                "ImageAtom and PinnedAtomId must not cross the bytecode_image boundary; found "
                + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
            )
        for ctx.match in raw_atom_return.finditer(ctx.code):
            ctx.fail(
                "image-atom-escape",
                "scalar admission may consume boolean atom predicates, not raw atom identities; found "
                + ctx.location(ctx.relative, ctx.source, ctx.match.start()),
            )
        if not ctx.relative.startswith("src/engine/code/binary_object/bytecode_image/") or ctx.is_test_source(ctx.path):
            continue
        for ctx.match in visible_function_pattern.finditer(ctx.code):
            visibility = " ".join(ctx.match.group("visibility").split())
            if visibility in ("pub(super)", "pub(self)"):
                continue
            ctx.item_code, ctx._, ctx._ = ctx.braced_item_from_match(
                ctx.code,
                ctx.match,
                "image-atom-visible-capability",
                "runtime-visible bytecode_image function",
            )
            if re.search(
                r"\b(?:ImageAtom|PinnedAtomId)\b|\.[ \t\n]*(?:atom|raw)[ \t\n]*\(",
                ctx.item_code,
            ):
                atom_sensitive_visible_sites.append(
                    (ctx.relative, visibility, ctx.match.group("name"))
                )

    expected_atom_sensitive_visible_sites = deepcopy(evidence.EXPECTED_ATOM_SENSITIVE_VISIBLE_SITES)

    if ctx.is_full_binary_inventory:
        expected_atom_sensitive_visible_sites.append(
            (
                "src/engine/code/binary_object/bytecode_image/model.rs",
                "pub(in crate::engine::code::binary_object)",
                "name_is_null",
            )
        )

    if atom_sensitive_visible_sites != expected_atom_sensitive_visible_sites:
        ctx.fail(
            "image-atom-visible-capability",
            "only the reviewed boolean atom predicates may expose an atom-sensitive bytecode-image method; "
            f"found {atom_sensitive_visible_sites}",
        )

    ctx.retired_permit_names = (
        "GraphSabDecodePermit",
        "BytecodeImageSabDecodePermit",
        "SabDecodePermit",
        "sab_decode_permit_sealed",
    )
