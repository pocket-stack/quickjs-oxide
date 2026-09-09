"""Scalar checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import scalar as evidence


def check(ctx):
    if ctx.scalar_top_level_items != ctx.expected_scalar_top_level_items:
        ctx.fail(
            "scalar-script-top-level-item-set",
            "scalar_script.rs must retain exactly the reviewed production DTO and private state types, with no module, trait, alias, union, or helper type escape; "
            f"found {ctx.scalar_top_level_items}",
        )

    scalar_top_level_function_pattern = re.compile(
        r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?fn"
        r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )

    scalar_top_level_functions = [
        match.group("name")
        for match in scalar_top_level_function_pattern.finditer(ctx.scalar_production_code)
        if ctx.scalar_production_code[:match.start()].count("{")
        == ctx.scalar_production_code[:match.start()].count("}")
    ]

    expected_scalar_top_level_functions = deepcopy(evidence.EXPECTED_SCALAR_TOP_LEVEL_FUNCTIONS)

    if scalar_top_level_functions != expected_scalar_top_level_functions:
        ctx.fail(
            "scalar-script-helper-set",
            "scalar_script.rs production free-function ownership drifted from the reviewed helper set; "
            f"found {scalar_top_level_functions}",
        )

    scalar_impl_header_pattern = re.compile(r"(?m)^[ \t]*impl\b(?P<header>[^{};]*)\{")

    scalar_impl_headers = [
        " ".join(match.group("header").split())
        for match in scalar_impl_header_pattern.finditer(ctx.scalar_production_code)
        if ctx.scalar_production_code[:match.start()].count("{")
        == ctx.scalar_production_code[:match.start()].count("}")
    ]

    expected_scalar_impl_headers = [
        "ScalarUnaryOp",
        "ScalarStringDraft",
        "fmt::Display for ScalarScriptReadError",
        "std::error::Error for ScalarScriptReadError",
        "AdmissionLimits",
    ]

    if scalar_impl_headers != expected_scalar_impl_headers:
        ctx.fail(
            "scalar-script-implementation-set",
            "scalar_script.rs must retain exactly the reviewed inherent and error-trait implementations; "
            f"found {scalar_impl_headers}",
        )

    scalar_macro_invocations = re.findall(
        r"(?<![A-Za-z0-9_])((?:r#)?[A-Za-z_][A-Za-z0-9_]*)"
        r"[ \t\n]*![ \t\n]*[([{]",
        ctx.scalar_production_code,
    )

    if scalar_macro_invocations != [
        "write",
        "write",
        "write",
        "write",
        "write",
        "write",
        "write",
        "format",
        "matches",
        "format",
        "matches",
        "matches",
        "matches",
        "format",
        "format",
    ]:
        ctx.fail(
            "scalar-script-macro-set",
            "scalar_script.rs may invoke only the reviewed diagnostic and atom-predicate macros; "
            f"found {scalar_macro_invocations}",
        )

    scalar_display_pattern = re.compile(
        r"\bimpl\b[^{};]*\b(?:fmt[ \t\n]*::[ \t\n]*)?Display[ \t\n]+for"
        r"[ \t\n]+ScalarScriptReadError[ \t\n]*\{",
        re.DOTALL,
    )

    if len(scalar_display_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-error-display",
            "ScalarScriptReadError must have exactly one direct Display implementation",
        )

    if len(re.findall(r"\bReaderMode[ \t\n]*::[ \t\n]*QuickJsCompatible\b", ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-reader-mode",
            "scalar_script.rs must select QuickJsCompatible exactly once",
        )

    for ctx.match in re.finditer(r"\bReaderMode[ \t\n]*::[ \t\n]*Strict\b", ctx.scalar_script_code):
        ctx.fail(
            "scalar-script-reader-mode",
            "scalar-script admission must not use Strict or Strict-plus-fallback; found "
            + ctx.location(ctx.scalar_script_relative, ctx.scalar_script_source, ctx.match.start()),
        )

    scalar_admission_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_production_code,
        re.compile(r"\bfn[ \t\n]+admit_image\b[^{};]*\{"),
        "scalar-script-translated-admission",
        "sanitized scalar admission function",
    )

    normalized_scalar_admission = " ".join(scalar_admission_code.split())

    scalar_admission_fragments = deepcopy(evidence.SCALAR_ADMISSION_FRAGMENTS)

    scalar_admission_offsets = [
        normalized_scalar_admission.find(fragment) for fragment in scalar_admission_fragments
    ]

    if (
        any(offset < 0 for offset in scalar_admission_offsets)
        or scalar_admission_offsets != sorted(scalar_admission_offsets)
        or any(normalized_scalar_admission.count(fragment) != 1 for fragment in scalar_admission_fragments)
        or re.search(r"\breturn[ \t\n]+Ok[ \t\n]*\(", scalar_admission_code)
    ):
        ctx.fail(
            "scalar-script-translated-admission",
            "scalar admission must translate once, validate the scalar shape and atom-table boundary, pair constants, then project an admitted atom before returning",
        )

    normalized_scalar_production = " ".join(ctx.scalar_production_code.split())

    expected_scalar_translate_import = " ".join(
        ctx.rust_code_only(
            '\n        use super::function_translate::{\n            AtomOperand, AtomOperandClass, FunctionCode, FunctionOp, FunctionTranslateError,\n            FunctionUnaryOp, TranslationTarget, translate_function,\n        };\n        '
        ).split()
    )

    if normalized_scalar_production.count(expected_scalar_translate_import) != 1:
        ctx.fail(
            "scalar-script-translated-import",
            "scalar_script must import exactly the reviewed sanitized translation facade",
        )

    direct_native_plan_import = re.search(
        r"\b(?:bytecode_image[ \t\n]*::[ \t\n]*native_plan|NativeAtomClass|"
        r"NativeAtomRef|NativeCodePlan|NativeInstruction|NativeOperands|PinnedOpcode)\b",
        ctx.scalar_production_code,
    )

    if direct_native_plan_import is not None:
        ctx.fail(
            "native-plan-consumer-set",
            "scalar_script must consume only sanitized translation semantics, never the native plan or pinned opcode catalog; found "
            + ctx.location(
                ctx.scalar_script_relative,
                ctx.scalar_script_source,
                direct_native_plan_import.start(),
            ),
        )

    if re.findall(
        r"\bWireValue[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
        ctx.scalar_production_code,
    ) != ["Float64Bits", "BigInt", "String"]:
        ctx.fail(
            "scalar-script-constant-pairing",
            "the scalar-script path may name only the reviewed Float64, BigInt, and String pool variants",
        )

    if re.search(
        r"\b(?:ImageAtom|PinnedAtomId|NativePlanError|OperationDiagnostic)\b|"
        r"\.[ \t\n]*(?:rejection_diagnostic|mnemonic|operand_shape)[ \t\n]*\(",
        ctx.scalar_production_code,
    ):
        ctx.fail(
            "scalar-script-translated-admission",
            "the scalar consumer may use only sanitized semantic operands and translation errors, never raw atom identities, private native-plan errors, or compatibility diagnostics",
        )

    scalar_atom_classes = re.findall(
        r"\bAtomOperandClass[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
        ctx.scalar_production_code,
    )

    scalar_atom_accessor_counts = {
        accessor: len(
            re.findall(
                rf"\batom[ \t\n]*\.[ \t\n]*{accessor}[ \t\n]*\(",
                ctx.scalar_production_code,
            )
        )
        for accessor in (
            "originates_from_input_atom_table",
            "class",
            "index_value",
            "string_utf16_len",
            "string_utf16_units",
        )
    }

    scalar_atom_projection_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_production_code,
        re.compile(r"\bfn[ \t\n]+project_atom_string\b[^{};]*\{"),
        "scalar-native-atom-consumer",
        "sanitized atom class and provenance consumer",
    )

    normalized_atom_projection = " ".join(scalar_atom_projection_code.split())

    atom_projection_fragments = deepcopy(evidence.ATOM_PROJECTION_FRAGMENTS)

    if any(
        normalized_atom_projection.count(fragment) != 1
        for fragment in atom_projection_fragments
    ):
        ctx.fail(
            "scalar-native-atom-consumer",
            "scalar atom admission must retain input-slot provenance and reject Null/Private/Symbol before Index/String projection",
        )

    if (
        scalar_atom_classes != ["Null", "Private", "Symbol", "Index", "String"]
        or scalar_atom_accessor_counts
        != {
            "originates_from_input_atom_table": 2,
            "class": 1,
            "index_value": 1,
            "string_utf16_len": 1,
            "string_utf16_units": 1,
        }
    ):
        ctx.fail(
            "scalar-native-atom-consumer",
            "scalar admission must preserve the exact sanitized input-slot provenance and Null/Private/Symbol/Index/String identity-class boundary; "
            f"found classes {scalar_atom_classes} and accessors {scalar_atom_accessor_counts}",
        )

    scalar_atom_spelling_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_production_code,
        re.compile(r"\bfn[ \t\n]+project_atom_string_spelling\b[^{};]*\{"),
        "scalar-native-atom-consumer",
        "sanitized atom String spelling consumer",
    )

    normalized_atom_spelling = " ".join(scalar_atom_spelling_code.split())

    atom_spelling_fragments = deepcopy(evidence.ATOM_SPELLING_FRAGMENTS)

    atom_spelling_offsets = [
        normalized_atom_spelling.find(fragment) for fragment in atom_spelling_fragments
    ]

    if (
        any(offset < 0 for offset in atom_spelling_offsets)
        or atom_spelling_offsets != sorted(atom_spelling_offsets)
        or any(normalized_atom_spelling.count(fragment) != 1 for fragment in atom_spelling_fragments)
    ):
        ctx.fail(
            "scalar-native-atom-consumer",
            "atom String admission must inspect the sealed UTF-16 length and iterator before the single fallible scalar copy",
        )

    scalar_translation_error_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_production_code,
        re.compile(r"\bfn[ \t\n]+classify_translation_error\b[^{};]*\{"),
        "scalar-script-translated-admission",
        "translation error classifier",
    )

    normalized_translation_error = " ".join(scalar_translation_error_code.split())

    if (
        normalized_translation_error.count("if error.is_label_target_error()") != 1
        or normalized_translation_error.count("ScalarScriptReadError::Unadmitted(") != 1
    ):
        ctx.fail(
            "scalar-script-translated-admission",
            "invalid translated labels must remain an ordinary scalar-cohort rejection",
        )

    ctx.consumer_relative = "src/engine/code/binary_object_publish.rs"

    consumer_path = ctx.root / ctx.consumer_relative

    ctx.consumer_exists = consumer_path.exists() or consumer_path.is_symlink()

    consumer_module_declarations = re.findall(
        r"(?m)^[ \t]*mod[ \t]+binary_object_publish[ \t]*;[ \t]*$",
        ctx.rust_code_only(ctx.read_source("src/engine/code/mod.rs")),
    )

    consumer_public_module_declarations = re.findall(
        r"(?m)^[ \t]*pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+mod"
        r"[ \t\n]+binary_object_publish[ \t\n]*;",
        ctx.rust_code_only(ctx.read_source("src/engine/code/mod.rs")),
    )

    if consumer_public_module_declarations:
        ctx.fail(
            "binary-object-consumer-module",
            "binary_object_publish must remain a private runtime module",
        )

    if ctx.consumer_exists:
        if consumer_path.is_symlink() or not consumer_path.is_file():
            ctx.fail("linked-source", f"{ctx.consumer_relative} must be a regular file")
            ctx.consumer_source = ""
        else:
            ctx.consumer_source = consumer_path.read_text(encoding="utf-8")
        if len(consumer_module_declarations) != 1:
            ctx.fail(
                "binary-object-consumer-module",
                "an existing binary_object_publish.rs requires exactly one private runtime module declaration",
            )
    else:
        ctx.consumer_source = ""
        if consumer_module_declarations:
            ctx.fail(
                "binary-object-consumer-module",
                "runtime must not declare binary_object_publish before its reviewed source exists",
            )

    ctx.consumer_code = ctx.rust_code_only(ctx.consumer_source)

    consumer_facade_import_pattern = re.compile(
        r"(?m)^[ \t]*use[ \t\n]+super[ \t\n]*::[ \t\n]*binary_object"
        r"[ \t\n]*::[ \t\n]*\{(?P<body>[^{}]*)\}[ \t\n]*;"
    )

    ctx.consumer_facade_imports = list(consumer_facade_import_pattern.finditer(ctx.consumer_code))
