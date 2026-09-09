"""Native plan checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import native_plan as evidence


def check(ctx):
    native_plan_relative = "src/engine/code/binary_object/bytecode_image/native_plan.rs"

    native_plan_source = ctx.read_source(native_plan_relative)

    native_plan_code = ctx.rust_code_only(native_plan_source)

    native_plan_test_module_pattern = re.compile(
        r"(?m)^[ \t]*#[ \t\n]*\[[ \t\n]*cfg[ \t\n]*\([ \t\n]*test"
        r"[ \t\n]*\)[ \t\n]*\][ \t\n]*mod[ \t\n]+tests[ \t\n]*\{"
    )

    native_plan_test_module, native_plan_test_start, native_plan_test_end = ctx.unique_braced_item(
        native_plan_code,
        native_plan_test_module_pattern,
        "native-plan-test-module",
        "private cfg(test) native-plan test module",
    )

    if native_plan_test_module:
        native_plan_production_code = (
            native_plan_code[:native_plan_test_start]
            + ctx.blank(native_plan_code[native_plan_test_start:native_plan_test_end])
            + native_plan_code[native_plan_test_end:]
        )
        native_plan_production_source = (
            native_plan_source[:native_plan_test_start]
            + ctx.blank(native_plan_source[native_plan_test_start:native_plan_test_end])
            + native_plan_source[native_plan_test_end:]
        )
    else:
        native_plan_production_code = native_plan_code
        native_plan_production_source = native_plan_source

    native_plan_visibility = "pub(in crate::engine::code::binary_object)"

    native_plan_visibility_pattern = r"pub(?:[ \t\n]*\([^)]*\))?"

    native_plan_visible_item_pattern = re.compile(
        rf"(?m)^[ \t]*(?P<visibility>{native_plan_visibility_pattern})[ \t\n]+"
        r"(?:(?:const|async|unsafe|extern)[ \t\n]+)*"
        r"(?P<kind>fn|enum|struct|trait|type|const|static|mod|use)\b"
        r"(?:[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*))?"
    )

    native_plan_visible_matches = list(
        native_plan_visible_item_pattern.finditer(native_plan_production_code)
    )

    native_plan_visible_items = [
        (match.group("kind"), match.group("name"))
        for match in native_plan_visible_matches
    ]

    expected_native_plan_visible_items = deepcopy(evidence.EXPECTED_NATIVE_PLAN_VISIBLE_ITEMS)

    native_plan_visible_tokens = list(
        re.finditer(
            rf"(?<![A-Za-z0-9_]){native_plan_visibility_pattern}(?![A-Za-z0-9_])",
            native_plan_production_code,
        )
    )

    native_plan_visible_details = [
        (
            " ".join(match.group("visibility").split()),
            match.group("kind"),
            match.group("name"),
        )
        for match in native_plan_visible_matches
    ]

    if (
        native_plan_visible_items != expected_native_plan_visible_items
        or len(native_plan_visible_matches) != len(native_plan_visible_tokens)
        or any(
            " ".join(match.group("visibility").split()) != native_plan_visibility
            for match in native_plan_visible_matches
        )
    ):
        ctx.fail(
            "native-plan-visible-surface",
            "the private native plan must expose only the reviewed binary_object-visible semantic DTO accessors; "
            f"found {native_plan_visible_details}",
        )

    native_plan_use_pattern = re.compile(r"(?m)^[ \t]*use[ \t\n]+[^;]+;")

    native_plan_uses = {
        " ".join(match.group(0).split())
        for match in native_plan_use_pattern.finditer(native_plan_production_code)
    }

    expected_native_plan_uses = deepcopy(evidence.EXPECTED_NATIVE_PLAN_USES)

    if native_plan_uses != expected_native_plan_uses:
        ctx.fail(
            "native-plan-dependency-set",
            "native_plan imports must remain the reviewed archive-only dependency set; "
            f"found {sorted(native_plan_uses)}",
        )

    native_plan_all_type_pattern = re.compile(
        r"\b(?P<kind>enum|struct|union)[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)\b"
    )

    native_plan_all_type_items = [
        (match.group("kind"), match.group("name"))
        for match in native_plan_all_type_pattern.finditer(native_plan_production_code)
    ]

    native_plan_type_keyword_count = len(
        re.findall(r"\b(?:enum|struct|union)\b", native_plan_production_code)
    )

    native_plan_type_pattern = re.compile(
        rf"(?m)^[ \t]*(?:{native_plan_visibility_pattern}[ \t\n]+)?"
        r"(?P<kind>enum|struct)[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
        r"[^;{}]*\{"
    )

    native_plan_type_matches = list(native_plan_type_pattern.finditer(native_plan_production_code))

    native_plan_type_items = [
        (match.group("kind"), match.group("name")) for match in native_plan_type_matches
    ]

    expected_native_plan_type_items = deepcopy(evidence.EXPECTED_NATIVE_PLAN_TYPE_ITEMS)

    if (
        native_plan_all_type_items != expected_native_plan_type_items
        or native_plan_type_items != expected_native_plan_type_items
        or native_plan_type_keyword_count != len(expected_native_plan_type_items)
    ):
        ctx.fail(
            "native-plan-type-set",
            "native_plan must contain only the reviewed braced semantic DTO and private decoder types; "
            f"found declarations {native_plan_all_type_items} and braced items {native_plan_type_items}",
        )

    native_plan_function_names = re.findall(
        r"\bfn[ \t\n]+([A-Za-z_][A-Za-z0-9_]*)\b",
        native_plan_production_code,
    )

    native_plan_function_keyword_count = len(
        re.findall(r"\bfn\b", native_plan_production_code)
    )

    expected_native_plan_function_names = deepcopy(evidence.EXPECTED_NATIVE_PLAN_FUNCTION_NAMES)

    if (
        native_plan_function_names != expected_native_plan_function_names
        or native_plan_function_keyword_count != len(expected_native_plan_function_names)
    ):
        ctx.fail(
            "native-plan-function-set",
            "native_plan must contain only the reviewed constructors, accessors, and decoder helpers; "
            f"found {native_plan_function_names}",
        )

    native_plan_non_function_consts = [
        name
        for name in re.findall(
            r"\bconst[ \t\n]+((?:r#)?[A-Za-z_][A-Za-z0-9_]*)\b",
            native_plan_production_code,
        )
        if name != "fn"
    ]

    native_plan_static_items = re.findall(
        r"(?<!')\bstatic[ \t\n]+(?:mut[ \t\n]+)?"
        r"((?:r#)?[A-Za-z_][A-Za-z0-9_]*)\b",
        native_plan_production_code,
    )

    native_plan_const_keyword_count = len(
        re.findall(r"\bconst\b", native_plan_production_code)
    )

    native_plan_static_keyword_count = len(
        re.findall(r"\bstatic\b", native_plan_production_code)
    )

    if (
        native_plan_non_function_consts != ["WIDTH"]
        or native_plan_static_items
        or native_plan_const_keyword_count != 21
        or native_plan_static_keyword_count != 3
    ):
        ctx.fail(
            "native-plan-data-item-set",
            "native_plan must contain no const/static helper items beyond read_array's one const-generic width; "
            f"found const names {native_plan_non_function_consts} and static names {native_plan_static_items}",
        )

    native_plan_stored_forbidden = re.compile(
        r"\b(?:ImageAtom|PinnedAtomId|BytecodeImage|ImageCode|ImageInstructionSpan|"
        r"ImageRelocation|Instruction|JsString|Value|Vm|VmHost|Runtime|Context|"
        r"RawValue|Heap|HeapObject|ObjectRef)\b|"
        r"(?:&[ \t\n]*(?:'[A-Za-z_][A-Za-z0-9_]*[ \t\n]+)?(?:mut[ \t\n]+)?)?"
        r"\[[ \t\n]*u8(?:[ \t\n]*;[^\]]+)?[ \t\n]*\]|"
        r"\b(?:Vec|Box|Arc|Rc|Cow)[ \t\n]*<[^>;{{}}]*\b"
        r"u8\b[^>;{{}}]*>"
    )

    for ctx.match in native_plan_type_matches:
        ctx.item_code, item_start, ctx._ = ctx.braced_item_from_match(
            native_plan_production_code,
            ctx.match,
            "native-plan-facade-representation",
            f"{ctx.match.group('name')} type declaration",
        )
        forbidden = native_plan_stored_forbidden.search(ctx.item_code)
        if forbidden is not None:
            ctx.fail(
                "native-plan-facade-representation",
                "native-plan DTOs must not store raw image identities, native code bytes, or executable runtime representations; found "
                + ctx.location(
                    native_plan_relative,
                    native_plan_source,
                    item_start + forbidden.start(),
                ),
            )

    native_plan_visible_signature_forbidden = re.compile(
        r"\b(?:ImageAtom|PinnedAtomId|ImageCode|ImageInstructionSpan|ImageRelocation|"
        r"Instruction|JsString|Value|Vm|VmHost|Runtime|Context|RawValue|Heap|"
        r"HeapObject|ObjectRef)\b|"
        r"&[ \t\n]*(?:'[A-Za-z_][A-Za-z0-9_]*[ \t\n]+)?(?:mut[ \t\n]+)?"
        r"\[[ \t\n]*u8[ \t\n]*\]|"
        r"\b(?:Vec|Box|Arc|Rc|Cow)[ \t\n]*<[^>;{{}}]*\b"
        r"u8\b[^>;{{}}]*>"
    )

    for ctx.match in native_plan_visible_matches:
        if ctx.match.group("kind") != "fn":
            continue
        signature = native_plan_production_code[ctx.match.start():ctx.match.end()]
        opening_brace = native_plan_production_code.find("{", ctx.match.end())
        semicolon = native_plan_production_code.find(";", ctx.match.end())
        signature_end_candidates = [
            offset for offset in (opening_brace, semicolon) if offset >= 0
        ]
        if signature_end_candidates:
            signature = native_plan_production_code[ctx.match.start():min(signature_end_candidates)]
        forbidden = native_plan_visible_signature_forbidden.search(signature)
        if forbidden is not None:
            ctx.fail(
                "native-plan-visible-representation",
                "native-plan visible functions must not expose raw image identities, native code bytes, or executable runtime representations; found "
                + ctx.location(
                    native_plan_relative,
                    native_plan_source,
                    ctx.match.start() + forbidden.start(),
                ),
            )

    native_plan_runtime_dependency = re.compile(
        r"\bcrate[ \t\n]*::[ \t\n]*(?:engine[ \t\n]*::[ \t\n]*(?:code[ \t\n]*::[ \t\n]*)?)?(?:bytecode|vm|heap|value)\b|"
        r"\b(?:Instruction|JsString|Value|Vm|VmHost|Runtime|Context|RuntimeError|"
        r"RawValue|Heap|HeapObject|ObjectRef)\b"
    )

    runtime_dependency = native_plan_runtime_dependency.search(native_plan_production_code)

    if runtime_dependency is not None:
        ctx.fail(
            "native-plan-runtime-dependency",
            "native_plan must remain archive-only and independent of executable bytecode, VM, heap, and runtime String/Value representations; found "
            + ctx.location(
                native_plan_relative,
                native_plan_source,
                runtime_dependency.start(),
            ),
        )

    native_plan_expansion_pattern = re.compile(
        r"\b(?:mod|trait|union|type)\b|"
        r"\b(?:include|include_bytes|include_str|macro_rules)[ \t\n]*!|"
        r"\bextern[ \t\n]+crate\b"
    )

    expansion = native_plan_expansion_pattern.search(native_plan_production_code)

    if expansion is not None:
        ctx.fail(
            "native-plan-expansion",
            "native_plan must not hide additional modules, traits, aliases, unions, includes, or macro definitions; found "
            + ctx.location(native_plan_relative, native_plan_source, expansion.start()),
        )

    native_atom_ref_pattern = re.compile(
        r"\bstruct[ \t\n]+NativeAtomRef[ \t\n]*<[ \t\n]*'image[ \t\n]*>"
        r"[ \t\n]*\{"
    )

    native_atom_ref_code, ctx._, ctx._ = ctx.unique_braced_item(
        native_plan_production_code,
        native_atom_ref_pattern,
        "native-plan-atom-projection",
        "sealed NativeAtomRef wrapper",
    )

    expected_native_atom_ref_source = "\n    struct NativeAtomRef<'image> {\n        kind: NativeAtomRefKind<'image>,\n        from_input_atom_table: bool,\n    }\n"

    if (
        native_atom_ref_code
        and " ".join(native_atom_ref_code.split())
        != " ".join(expected_native_atom_ref_source.split())
    ):
        ctx.fail(
            "native-plan-atom-projection",
            "NativeAtomRef must remain a sealed wrapper around the sanitized private discriminator",
        )

    native_atom_ref_kind_pattern = re.compile(
        r"\benum[ \t\n]+NativeAtomRefKind[ \t\n]*<[ \t\n]*'image[ \t\n]*>"
        r"[ \t\n]*\{"
    )

    native_atom_ref_kind_code, ctx._, ctx._ = ctx.unique_braced_item(
        native_plan_production_code,
        native_atom_ref_kind_pattern,
        "native-plan-atom-projection",
        "private sanitized NativeAtomRefKind discriminator",
    )

    expected_native_atom_ref_kind_source = deepcopy(evidence.EXPECTED_NATIVE_ATOM_REF_KIND_SOURCE)

    if (
        native_atom_ref_kind_code
        and " ".join(native_atom_ref_kind_code.split())
        != " ".join(expected_native_atom_ref_kind_source.split())
    ):
        ctx.fail(
            "native-plan-atom-projection",
            "NativeAtomRefKind must preserve only null, integer-index, manifest identity class/spelling, and image-borrowed dynamic String projections",
        )

    def enum_variant_names(item_code: str) -> list[str]:
        opening = item_code.find("{")
        closing = item_code.rfind("}")
        if opening < 0 or closing <= opening:
            return []
        body = item_code[opening + 1:closing]
        variants: list[str] = []
        start = 0
        round_depth = 0
        square_depth = 0
        brace_depth = 0
        for offset, character in enumerate(body):
            if character == "(":
                round_depth += 1
            elif character == ")":
                round_depth -= 1
            elif character == "[":
                square_depth += 1
            elif character == "]":
                square_depth -= 1
            elif character == "{":
                brace_depth += 1
            elif character == "}":
                brace_depth -= 1
            elif character == "," and round_depth == square_depth == brace_depth == 0:
                segment = body[start:offset].strip()
                if segment:
                    name = re.match(r"([A-Za-z_][A-Za-z0-9_]*)\b", segment)
                    if name is None:
                        return []
                    variants.append(name.group(1))
                start = offset + 1
        tail = body[start:].strip()
        if tail:
            name = re.match(r"([A-Za-z_][A-Za-z0-9_]*)\b", tail)
            if name is None:
                return []
            variants.append(name.group(1))
        return variants
    ctx.enum_variant_names = enum_variant_names

    native_atom_class_pattern = re.compile(
        rf"\b{native_plan_visibility_pattern}[ \t\n]+enum[ \t\n]+NativeAtomClass"
        r"[ \t\n]*\{"
    )

    native_atom_class_code, ctx._, ctx._ = ctx.unique_braced_item(
        native_plan_production_code,
        native_atom_class_pattern,
        "native-plan-atom-class",
        "NativeAtomClass enum",
    )

    native_atom_classes = ctx.enum_variant_names(native_atom_class_code)

    if native_atom_classes != ["Null", "Index", "String", "Private", "Symbol"]:
        ctx.fail(
            "native-plan-atom-class",
            "NativeAtomClass must preserve null, integer-index, ordinary String, private-name, and Symbol identity classes; "
            f"found {native_atom_classes}",
        )

    native_operands_pattern = re.compile(
        rf"\b{native_plan_visibility_pattern}[ \t\n]+enum[ \t\n]+NativeOperands"
        r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
    )

    native_operands_code, ctx._, ctx._ = ctx.unique_braced_item(
        native_plan_production_code,
        native_operands_pattern,
        "native-plan-operand-formats",
        "NativeOperands enum",
    )

    native_operand_variants = ctx.enum_variant_names(native_operands_code)

    if ctx.is_full_binary_inventory:
        ctx.pinned_opcode_relative = "src/engine/code/binary_object/pinned_opcodes.rs"
        pinned_opcode_code = ctx.binary_code_cache[ctx.root / ctx.pinned_opcode_relative]
        opcode_format_pattern = re.compile(
            rf"\b{native_plan_visibility_pattern.replace('binary_object', 'runtime')}"
            r"[ \t\n]+enum[ \t\n]+OpcodeFormat[ \t\n]*\{"
        )
        opcode_format_code, ctx._, ctx._ = ctx.unique_braced_item(
            pinned_opcode_code,
            opcode_format_pattern,
            "native-plan-operand-formats",
            "pinned OpcodeFormat enum",
        )
        opcode_format_variants = ctx.enum_variant_names(opcode_format_code)
        if native_operand_variants != opcode_format_variants:
            ctx.fail(
                "native-plan-operand-formats",
                "NativeOperands must remain a one-for-one, ordered projection of the pinned OpcodeFormat table; "
                f"found native {native_operand_variants} versus pinned {opcode_format_variants}",
            )

    native_format_method_pattern = re.compile(
        rf"\b{native_plan_visibility_pattern}[ \t\n]+const[ \t\n]+fn"
        r"[ \t\n]+format[ \t\n]*\([ \t\n]*&self[ \t\n]*\)"
        r"[ \t\n]*->[ \t\n]*OpcodeFormat[ \t\n]*\{"
    )

    native_format_method_code, ctx._, ctx._ = ctx.unique_braced_item(
        native_plan_production_code,
        native_format_method_pattern,
        "native-plan-operand-formats",
        "NativeOperands::format mapping",
    )

    native_format_pairs = re.findall(
        r"Self[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)"
        r"[ \t\n]*(?:\([^=]*?\)|\{[^=]*?\})?[ \t\n]*=>"
        r"[ \t\n]*OpcodeFormat[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
        native_format_method_code,
    )

    if (
        [native for native, _ in native_format_pairs] != native_operand_variants
        or any(native != opcode_format for native, opcode_format in native_format_pairs)
    ):
        ctx.fail(
            "native-plan-operand-formats",
            "NativeOperands::format must map every typed operand variant to its identically named pinned format exactly once; "
            f"found {native_format_pairs}",
        )

    native_plan_semantic_seals = [
        (
            "atom projection and accessors",
            re.compile(
                r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
                r"NativeAtomRef[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
            ),
            "378938040432e70209bb37dbce0c1d00ca18097b472d1e04d496a03caa03e710",
        ),
        (
            "label representation",
            re.compile(
                rf"\b{native_plan_visibility_pattern}[ \t\n]+struct"
                r"[ \t\n]+NativeLabel[ \t\n]*\{"
            ),
            "17d62c502c7f9fcff5e0b83cf095c397807967bae3bc30677586c752d57ab4b9",
        ),
        (
            "label accessors",
            re.compile(r"\bimpl[ \t\n]+NativeLabel[ \t\n]*\{"),
            "b4a55d2c8547c8f7c270b21cb476f0424b9f16ab317def40b5d5734a57cc10f5",
        ),
        (
            "typed operand representation",
            re.compile(
                rf"\b{native_plan_visibility_pattern}[ \t\n]+enum"
                r"[ \t\n]+NativeOperands[^{{;]*\{"
            ),
            "6809f5cbecf4da9ea2b8f9d985386c742a9c24cce7d56326be3f56764402f601",
        ),
        (
            "instruction representation",
            re.compile(
                rf"\b{native_plan_visibility_pattern}[ \t\n]+struct"
                r"[ \t\n]+NativeInstruction[^{{;]*\{"
            ),
            "d9e19a9c4abe7d24e21e96109d32b56c7c6300f2374722bb80d75eedf67ab742",
        ),
        (
            "instruction accessors",
            re.compile(
                r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
                r"NativeInstruction[^{{;]*\{"
            ),
            "96dda98087d105b39cda7e56b84ab9f41a36a77836777a262eaa2c644d37256b",
        ),
        (
            "code-plan representation",
            re.compile(
                rf"\b{native_plan_visibility_pattern}[ \t\n]+struct"
                r"[ \t\n]+NativeCodePlan[^{{;]*\{"
            ),
            "3f08b0fbe4e6e606f94f98fa20a404579c412e21577ce4fa62d0708236051909",
        ),
        (
            "code-plan accessors",
            re.compile(
                r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
                r"NativeCodePlan[^{{;]*\{"
            ),
            "c9074e1e3e98fdb0dbd0f8ce39b18fdf23f2077ec25664e936a40852474d61fd",
        ),
        (
            "error representation",
            re.compile(
                rf"\b{native_plan_visibility_pattern}[ \t\n]+enum"
                r"[ \t\n]+NativePlanError[ \t\n]*\{"
            ),
            "da2eecfb29c0b8de9d928fe488f1f2a708a2a7c896361c1b66ee6b07d87ac058",
        ),
        (
            "label-target error classification",
            re.compile(r"\bimpl[ \t\n]+NativePlanError[ \t\n]*\{"),
            "26e0b3f582fe7e09123ac84a16c979b44388403901c4a0a2e36228b2db363e38",
        ),
        (
            "authenticated entrypoint",
            re.compile(
                rf"\b{native_plan_visibility_pattern}[ \t\n]+fn"
                r"[ \t\n]+decode_native_code_plan[^{{;]*\{"
            ),
            "54536fe713b833a61a75e34f1126fef08ca650fcc64b3298d1f57f94c91b6d2d",
        ),
        (
            "code and relocation decoder",
            re.compile(r"\bfn[ \t\n]+decode_code_plan[^{{;]*\{"),
            "2a1fa56db24fc98351ba5035403576b36539cc560b8ebbf5bbda98c00333171e",
        ),
        (
            "instruction-boundary authentication",
            re.compile(r"\bfn[ \t\n]+validate_instruction_boundaries[^{{;]*\{"),
            "4f33d2a1edef051800bf810be6064771d33804b98287bc5b075e3eec514babde",
        ),
        (
            "operand decoder",
            re.compile(r"\bfn[ \t\n]+decode_operands[^{{;]*\{"),
            "3bd2e04c1e7c2d422b9543ba356c5a627600fa23dc45f40c6de923001e2ee393",
        ),
        (
            "implicit integer decoder",
            re.compile(r"\bfn[ \t\n]+implicit_integer[^{{;]*\{"),
            "d5880d502e8c08f31320fc823eacaf65bbc71ef00376385f6235053bdc8bf379",
        ),
        (
            "implicit slot decoder",
            re.compile(r"\bfn[ \t\n]+implicit_slot[^{{;]*\{"),
            "a5aa9080cfe562279addb030926ab547d976c126594f66ccbf2a0496e59991b4",
        ),
        (
            "label decoder",
            re.compile(r"\bfn[ \t\n]+decode_label[^{{;]*\{"),
            "535ab4c806e3fd2536e7bdd2999fabf2766f0f05ed6587a311e570d27ffc8d73",
        ),
        (
            "operand format sizes",
            re.compile(r"\bconst[ \t\n]+fn[ \t\n]+format_size[^{{;]*\{"),
            "7124a7e74681fded01f1f6018132c8fb25e2e4bc76b67b86f3df045b8116dde6",
        ),
    ]

    for ctx.description, ctx.pattern, ctx.expected_hash in native_plan_semantic_seals:
        ctx.item_code, ctx._, ctx._ = ctx.unique_braced_item(
            native_plan_production_code,
            ctx.pattern,
            "native-plan-semantic-seal",
            ctx.description,
        )
        if ctx.item_code and ctx.normalized_code_sha256(ctx.item_code) != ctx.expected_hash:
            ctx.fail(
                "native-plan-semantic-seal",
                f"native_plan {ctx.description} drifted from its reviewed normalized implementation",
            )

    native_plan_implicit_string_patterns = {
        "push_minus1": re.compile(
            r"opcode[ \t\n]*\.[ \t\n]*name[ \t\n]*\([ \t\n]*\)"
            r"[ \t\n]*==[ \t\n]*\"push_minus1\""
        ),
        "push_ prefix": re.compile(
            r"opcode[ \t\n]*\.[ \t\n]*name[ \t\n]*\([ \t\n]*\)"
            r"[ \t\n]*\.[ \t\n]*strip_prefix[ \t\n]*\([ \t\n]*\"push_\""
            r"[ \t\n]*\)"
        ),
        "local short forms": re.compile(
            r"&[ \t\n]*\[[ \t\n]*\"get_loc\"[ \t\n]*,"
            r"[ \t\n]*\"put_loc\"[ \t\n]*,[ \t\n]*\"set_loc\"[ \t\n]*\]"
        ),
        "argument short forms": re.compile(
            r"&[ \t\n]*\[[ \t\n]*\"get_arg\"[ \t\n]*,"
            r"[ \t\n]*\"put_arg\"[ \t\n]*,[ \t\n]*\"set_arg\"[ \t\n]*\]"
        ),
        "variable-reference short forms": re.compile(
            r"&[ \t\n]*\[[ \t\n]*\"get_var_ref\"[ \t\n]*,"
            r"[ \t\n]*\"put_var_ref\"[ \t\n]*,[ \t\n]*\"set_var_ref\""
            r"[ \t\n]*\]"
        ),
        "call short form": re.compile(
            r"OpcodeFormat[ \t\n]*::[ \t\n]*NPopX[^=]*=>[^;]*"
            r"&[ \t\n]*\[[ \t\n]*\"call\"[ \t\n]*\]",
            re.DOTALL,
        ),
    }

    for ctx.description, ctx.pattern in native_plan_implicit_string_patterns.items():
        if len(ctx.pattern.findall(native_plan_production_source)) != 1:
            ctx.fail(
                "native-plan-implicit-opcode-set",
                f"native_plan must retain exactly one reviewed {ctx.description} implicit-opcode spelling",
            )

    native_plan_declaration_pattern = re.compile(
        r"(?m)^[ \t]*mod[ \t]+native_plan[ \t]*;[ \t]*$"
    )

    native_plan_facade_pattern = re.compile(
        rf"\b{native_plan_visibility_pattern}[ \t\n]+use[ \t\n]+native_plan"
        r"[ \t\n]*::[ \t\n]*\{[ \t\n]*NativeAtomClass[ \t\n]*,"
        r"[ \t\n]*NativeAtomRef[ \t\n]*,[ \t\n]*NativeCodePlan[ \t\n]*,"
        r"[ \t\n]*NativeOperands[ \t\n]*,[ \t\n]*decode_native_code_plan"
        r"[ \t\n]*,?[ \t\n]*\}[ \t\n]*;"
    )

    native_plan_declarations = list(native_plan_declaration_pattern.finditer(ctx.image_root_code))

    native_plan_facades = list(native_plan_facade_pattern.finditer(ctx.image_root_code))

    if len(native_plan_declarations) != 1 or len(native_plan_facades) != 1:
        ctx.fail(
            "native-plan-private-stage",
            "native_plan must remain one private bytecode_image child with one exact binary_object-only semantic facade",
        )

    for ctx.path, ctx.code in ctx.binary_code_cache.items():
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        if ctx.relative == native_plan_relative:
            continue
        mentions = list(re.finditer(r"\bnative_plan\b", ctx.code))
        if ctx.relative == ctx.image_root_relative:
            allowed_ranges = [
                (match.start(), match.end())
                for match in (*native_plan_declarations, *native_plan_facades)
            ]
            unexpected = [
                mention
                for mention in mentions
                if not any(start <= mention.start() < end for start, end in allowed_ranges)
            ]
            if unexpected:
                ctx.fail(
                    "native-plan-private-stage",
                    "native_plan module and facade shape drifted; found "
                    + ", ".join(
                        ctx.location(ctx.relative, ctx.binary_source_cache[ctx.path], mention.start())
                        for mention in unexpected
                    ),
                )
        elif ctx.relative == "src/engine/code/binary_object/function_translate/mod.rs":
            continue
        elif mentions:
            ctx.fail(
                "native-plan-private-stage",
                "native_plan may be named only by its private module/facade and the reviewed scalar/ordinary consumers; found "
                + ", ".join(
                    ctx.location(ctx.relative, ctx.binary_source_cache[ctx.path], mention.start())
                    for mention in mentions
                ),
            )

    native_plan_facade_symbols = (
        "NativeAtomClass",
        "NativeAtomRef",
        "NativeCodePlan",
        "NativeOperands",
        "decode_native_code_plan",
    )

    allowed_native_plan_symbol_files = {
        native_plan_relative,
        ctx.image_root_relative,
        "src/engine/code/binary_object/function_translate/mod.rs",
    }

    for ctx.path, ctx.code in ctx.binary_code_cache.items():
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        if ctx.is_test_source(ctx.path) or ctx.relative in allowed_native_plan_symbol_files:
            continue
        for symbol in native_plan_facade_symbols:
            mention = re.search(rf"\b{symbol}\b", ctx.code)
            if mention is not None:
                ctx.fail(
                    "native-plan-consumer-set",
                    "only function_translate may consume the reviewed native-plan facade; found "
                    + ctx.location(ctx.relative, ctx.binary_source_cache[ctx.path], mention.start()),
                )

    bytecode_image_alias_pattern = re.compile(
        r"\btype[ \t\n]+[A-Za-z_][A-Za-z0-9_]*(?:[ \t\n]*<[^;=]*>)?"
        r"[ \t\n]*=[^;]*\bBytecodeImage\b"
        r"|\buse\b[^;]*\bBytecodeImage[ \t\n]+as[ \t\n]+"
        r"[A-Za-z_][A-Za-z0-9_]*"
    )

    for ctx.path, ctx.code in ctx.binary_code_cache.items():
        ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
        if not ctx.relative.startswith("src/engine/code/binary_object/bytecode_image/"):
            continue
        for ctx.match in bytecode_image_alias_pattern.finditer(ctx.code):
            ctx.fail(
                "bytecode-image-alias",
                "BytecodeImage must not acquire a type or import alias that can hide implementation ownership; found "
                + ctx.location(ctx.relative, ctx.binary_source_cache[ctx.path], ctx.match.start()),
            )

    for ctx.path, ctx.code in ctx.binary_code_cache.items():
        for ctx.match in re.finditer(
            r"(?<![A-Za-z0-9_])(?:r#)?include[ \t\n]*!",
            ctx.code,
        ):
            ctx.relative = ctx.path.relative_to(ctx.root).as_posix()
            ctx.fail(
                "forbidden-source-include",
                "binary_object production sources must not splice unscanned Rust source; found "
                + ctx.location(ctx.relative, ctx.binary_source_cache[ctx.path], ctx.match.start()),
            )

    function_translate_root = "src/engine/code/binary_object/function_translate"

    ctx.function_translate_relative = f"{function_translate_root}/mod.rs"

    ctx.function_translate_capability_relative = f"{function_translate_root}/capability.rs"

    ctx.function_translate_dto_relative = f"{function_translate_root}/dto.rs"

    expected_function_translate_sources = {
        ctx.function_translate_relative,
        ctx.function_translate_capability_relative,
        ctx.function_translate_dto_relative,
    }

    found_function_translate_sources = {
        path.relative_to(ctx.root).as_posix()
        for path in ctx.binary_sources
        if path.relative_to(ctx.root).as_posix().startswith(f"{function_translate_root}/")
    }

    if found_function_translate_sources != expected_function_translate_sources:
        ctx.fail(
            "function-translate-module-set",
            "function_translate must contain only the reviewed module root, capability registry, and sanitized DTO; "
            f"found {sorted(found_function_translate_sources)}",
        )
