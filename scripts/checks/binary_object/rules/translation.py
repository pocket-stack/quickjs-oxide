"""Translation checks, in the ordered boundary scan."""
from __future__ import annotations

import hashlib
import re
from copy import deepcopy

from ..evidence import translation as evidence


def check(ctx):
    function_translate_source = ctx.read_source(ctx.function_translate_relative)

    ctx.function_translate_code = ctx.rust_code_only(function_translate_source)

    function_translate_production_code = ctx.function_translate_code.split("#[cfg(test)]", 1)[0]

    function_translate_production_source = function_translate_source.split("#[cfg(test)]", 1)[0]

    function_translate_modules = re.findall(
        r"(?m)^[ \t]*mod[ \t]+([A-Za-z_][A-Za-z0-9_]*)[ \t]*;[ \t]*$",
        function_translate_production_code,
    )

    if function_translate_modules != ["capability", "dto"]:
        ctx.fail(
            "function-translate-module-set",
            "function_translate must retain exactly the private capability and dto children; "
            f"found {function_translate_modules}",
        )

    for ctx.match in ctx.public_module_pattern.finditer(function_translate_production_code):
        ctx.fail(
            "function-translate-module-visibility",
            "function_translate children must remain private; found "
            + ctx.location(ctx.function_translate_relative, function_translate_source, ctx.match.start()),
        )

    capability_source = ctx.read_source(ctx.function_translate_capability_relative)

    capability_production_source = capability_source.split("#[cfg(test)]", 1)[0]

    capability_production_code = ctx.rust_code_only(capability_production_source)

    dto_source = ctx.read_source(ctx.function_translate_dto_relative)

    dto_production_source = dto_source.split("#[cfg(test)]", 1)[0]

    dto_production_code = ctx.rust_code_only(dto_production_source)

    recipe_code, ctx._, ctx._ = ctx.unique_braced_item(
        capability_production_code,
        re.compile(r"\benum[ \t\n]+Recipe[ \t\n]*\{"),
        "function-translate-recipe-shape",
        "Recipe enum",
    )

    expected_recipe_variants = '\n    Nop Object ToObject ToPropKey PushThis PushI32 PushConstant PushAtom PushUndefined PushNull PushFalse PushTrue\n    PushBigIntI32 PushEmptyString Stack Unary PostDec PostInc GetLocal PutLocal\n    SetLocal GetArgument PutArgument SetArgument Binary Predicate IfFalse IfTrue\n    Goto Call TailCall Construct CallMethod TailCallMethod ArrayFrom Apply Return\n    ReturnUndefined Throw ThrowReadOnly\n'.split()

    ctx.counted_invocation_variant_names = (
        "Call", "TailCall", "Construct", "CallMethod", "TailCallMethod", "ArrayFrom"
    )

    ctx.invocation_variant_names = (*ctx.counted_invocation_variant_names, "Apply")

    unit_recipe_variant_names = ("Nop", "Object", "ToObject", "ToPropKey", "PushThis")

    unit_recipe_shapes = {
        name: (
            len(re.findall(rf"\b{name}[ \t\n]*,", recipe_code)),
            bool(re.search(rf"\b{name}[ \t\n]*[({{]", recipe_code)),
        )
        for name in unit_recipe_variant_names
    }

    recipe_invocation_shapes = {
        name: (
            len(re.findall(rf"\b{name}[ \t\n]*,", recipe_code)),
            bool(re.search(rf"\b{name}[ \t\n]*[({{]", recipe_code)),
        )
        for name in ctx.invocation_variant_names
    }

    if (
        ctx.enum_variant_names(recipe_code) != expected_recipe_variants
        or any(unit_recipe_shapes[name] != (1, False) for name in unit_recipe_variant_names)
        or any(
            recipe_invocation_shapes[name] != (1, False)
            for name in ctx.invocation_variant_names
        )
    ):
        ctx.fail(
            "function-translate-recipe-shape",
            "Recipe must retain the exact reviewed inventory with unit Nop/Object/ToObject/ToPropKey/PushThis recipes and explicit terminal completions; "
            f"found {ctx.enum_variant_names(recipe_code)} with unit shapes {unit_recipe_shapes} "
            f"and invocation shapes {recipe_invocation_shapes}",
        )

    dto_function_op_code, ctx._, ctx._ = ctx.unique_braced_item(
        dto_production_code,
        re.compile(
            r"\benum[ \t\n]+FunctionOp[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        ),
        "function-translate-dto-shape",
        "FunctionOp enum",
    )

    expected_function_op_variants = '\n    Blocked OutsideTarget Nop Object ToObject ToPropKey PushThis PushI32 PushConstant PushAtom PushUndefined PushNull\n    PushBool PushBigIntI32 PushEmptyString Stack Unary PostDec PostInc GetLocal\n    PutLocal SetLocal GetArgument PutArgument SetArgument Binary Predicate IfFalse\n    IfTrue Goto Call TailCall Construct CallMethod TailCallMethod ArrayFrom Apply\n    Return ReturnUndefined Throw ThrowReadOnly\n'.split()

    function_invocation_payloads = {
        name: [
            " ".join(payload.split())
            for payload in re.findall(
                rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", dto_function_op_code
            )
        ]
        for name in ctx.invocation_variant_names
    }

    if (
        ctx.enum_variant_names(dto_function_op_code) != expected_function_op_variants
        or any(function_invocation_payloads[name] != ["u16"]
               for name in ctx.counted_invocation_variant_names)
        or function_invocation_payloads["Apply"] != ["FunctionApplyKind"]
    ):
        ctx.fail(
            "function-translate-dto-shape",
            "FunctionOp must retain the exact reviewed inventory with operand-free Nop/Object/ToObject/ToPropKey/PushThis, distinct counted u16 invocation payloads, a typed Apply kind, and an operand-free Throw completion; "
            f"found {ctx.enum_variant_names(dto_function_op_code)} with invocation payloads {function_invocation_payloads}",
        )

    function_throw_read_only_payloads = [
        " ".join(payload.split())
        for payload in re.findall(
            r"\bThrowReadOnly[ \t\n]*\(([^()]*)\)[ \t\n]*,",
            dto_function_op_code,
        )
    ]

    if function_throw_read_only_payloads != ["AtomOperand<'image>"]:
        ctx.fail(
            "function-translate-dto-shape",
            "FunctionOp::ThrowReadOnly must retain exactly one borrowed sanitized AtomOperand payload",
        )

    dto_apply_kind_code, ctx._, ctx._ = ctx.unique_braced_item(
        dto_production_code,
        re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
            r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+enum"
            r"[ \t\n]+FunctionApplyKind[ \t\n]*\{"
        ),
        "function-translate-apply-kind",
        "typed sanitized apply kind",
    )

    if ctx.enum_variant_names(dto_apply_kind_code) != ["Call", "Construct"]:
        ctx.fail(
            "function-translate-apply-kind",
            "FunctionApplyKind must expose only canonical call and construct semantics; "
            f"found {ctx.enum_variant_names(dto_apply_kind_code)}",
        )

    ctx.pinned_opcode_relative = "src/engine/code/binary_object/pinned_opcodes.rs"

    pinned_opcode_source = ctx.read_source(ctx.pinned_opcode_relative)

    pinned_opcode_production_source = pinned_opcode_source.split("#[cfg(test)]", 1)[0]

    pinned_descriptor_pattern = re.compile(
        r"PinnedOpcodeInfo[ \t\n]*::[ \t\n]*new[ \t\n]*\("
        r"[ \t\n]*\"([^\"]+)\"[ \t\n]*,[ \t\n]*(\d+)"
        r"[ \t\n]*,[ \t\n]*(\d+)[ \t\n]*,[ \t\n]*(\d+)"
        r"[ \t\n]*,[ \t\n]*OpcodeFormat[ \t\n]*::[ \t\n]*"
        r"([A-Za-z_][A-Za-z0-9_]*)[ \t\n]*\)"
    )

    pinned_descriptors = [
        (name, int(size), int(n_pop), int(n_push), operand_format)
        for name, size, n_pop, n_push, operand_format in pinned_descriptor_pattern.findall(
            pinned_opcode_production_source
        )
    ]

    if len(pinned_descriptors) != 244:
        ctx.fail(
            "function-translate-registry-descriptor",
            "the pinned descriptor table must retain exactly 244 ordered entries; "
            f"found {len(pinned_descriptors)}",
        )

    registry_row_pattern = re.compile(
        r"(?m)^[ \t]*row![ \t]*\([ \t]*(\d+)[ \t]*,[ \t]*"
        r"([A-Za-z_][A-Za-z0-9_]*)[ \t]*,[ \t]*"
        r"(Blocked|ScalarOnly|OrdinaryOnly|Shared)[ \t]*,[ \t]*"
        r"([^,\n]+)[ \t]*\)[ \t]*,[ \t]*$"
    )

    registry_rows = [
        (int(raw), operand_format, audience, " ".join(detail.split()))
        for raw, operand_format, audience, detail in registry_row_pattern.findall(
            capability_production_source
        )
    ]

    registry_raws = [raw for raw, _, _, _ in registry_rows]

    if registry_raws != list(range(244)):
        ctx.fail(
            "function-translate-registry-raw",
            "the capability registry must contain one contiguous raw-indexed row for each opcode 0 through 243; "
            f"found {registry_raws}",
        )

    if len(pinned_descriptors) == 244:
        format_mismatches = [
            (raw, operand_format, pinned_descriptors[raw][4])
            for raw, operand_format, _, _ in registry_rows
            if raw >= len(pinned_descriptors)
            or operand_format != pinned_descriptors[raw][4]
        ]
        if format_mismatches:
            ctx.fail(
                "function-translate-registry-descriptor",
                "each raw-indexed capability row must retain the operand format of its pinned descriptor; "
                f"found {format_mismatches}",
            )

    registry_audience_counts = {
        audience: sum(row_audience == audience for _, _, row_audience, _ in registry_rows)
        for audience in ("Blocked", "ScalarOnly", "OrdinaryOnly", "Shared")
    }

    expected_registry_audience_counts = dict(zip(
        ("Blocked", "ScalarOnly", "OrdinaryOnly", "Shared"), (110, 1, 104, 29)
    ))

    derived_registry_counts = (
        registry_audience_counts["ScalarOnly"] + registry_audience_counts["Shared"],
        registry_audience_counts["OrdinaryOnly"] + registry_audience_counts["Shared"],
        244 - registry_audience_counts["Blocked"],
    )

    if (
        registry_audience_counts != expected_registry_audience_counts
        or derived_registry_counts != (30, 133, 134)
    ):
        ctx.fail(
            "function-translate-registry-audience",
            "the centralized registry must preserve the exact final stage-three-J physical cohorts; "
            f"found {registry_audience_counts} with scalar/ordinary/union {derived_registry_counts}",
        )

    expected_admitted_registry: dict[int, tuple[str, str]] = {}

    def expect_admitted(audience: str, recipe: str, raws: tuple[int, ...]) -> None:
        for raw in raws:
            if raw in expected_admitted_registry:
                ctx.fail(
                    "function-translate-registry-policy",
                    f"internal gate expectation names admitted raw opcode {raw} twice",
                )
            expected_admitted_registry[raw] = (audience, recipe)
    expect_admitted = expect_admitted

    expect_admitted("Shared", "Recipe::PushI32", (1, *range(178, 189)))

    expect_admitted("Shared", "Recipe::PushConstant", (2, 189))

    expect_admitted("Shared", "Recipe::PushUndefined", (6,))

    expect_admitted("Shared", "Recipe::PushNull", (7,))

    expect_admitted("Shared", "Recipe::PushFalse", (9,))

    expect_admitted("Shared", "Recipe::PushTrue", (10,))

    expect_admitted("Shared", "Recipe::Return", (40,))

    for raw, operation in zip((138, 139, 140, 141, 147, 148, 149),
                              ("Neg", "Plus", "Dec", "Inc", "BitNot", "LogicalNot", "TypeOf")):
        expect_admitted("Shared", f"Recipe::Unary(FunctionUnaryOp::{operation})", (raw,))

    expect_admitted("Shared", "Recipe::PushBigIntI32", (176,))

    expect_admitted("Shared", "Recipe::PushEmptyString", (191,))

    expect_admitted("Shared", "Recipe::SetLocal", (203,))

    expect_admitted("ScalarOnly", "Recipe::PushAtom", (4,))

    for raw, recipe in zip(
        range(14, 33),
        (
            "Direct(FunctionStackOp::Drop)", "Direct(FunctionStackOp::Nip)", "Nip1",
            "Direct(FunctionStackOp::Dup)", "Direct(FunctionStackOp::Dup1)", "Dup2",
            "Direct(FunctionStackOp::Dup3)", "Direct(FunctionStackOp::Insert2)",
            "Direct(FunctionStackOp::Insert3)", "Direct(FunctionStackOp::Insert4)",
            "Direct(FunctionStackOp::Perm3)", "Direct(FunctionStackOp::Perm4)",
            "Direct(FunctionStackOp::Perm5)", "Direct(FunctionStackOp::Swap)", "Swap2",
            "Rot3Left", "Rot3Right", "Direct(FunctionStackOp::Rot4Left)", "Rot5Left",
        ),
    ):
        expect_admitted("OrdinaryOnly", f"Recipe::Stack(StackRecipe::{recipe})", (raw,))

    expect_admitted("OrdinaryOnly", "Recipe::ReturnUndefined", (41,))

    expect_admitted("OrdinaryOnly", "Recipe::GetLocal", (85, 192, 195, 196, 197, 198))

    expect_admitted("OrdinaryOnly", "Recipe::PutLocal", (86, 193, 199, 200, 201, 202))

    expect_admitted("OrdinaryOnly", "Recipe::SetLocal", (87, 194, 204, 205, 206))

    expect_admitted("OrdinaryOnly", "Recipe::GetArgument", (88, 207, 208, 209, 210))

    expect_admitted("OrdinaryOnly", "Recipe::PutArgument", (89, 211, 212, 213, 214))

    expect_admitted("OrdinaryOnly", "Recipe::SetArgument", (90, 215, 216, 217, 218))

    expect_admitted("OrdinaryOnly", "Recipe::IfFalse", (104, 232))

    expect_admitted("OrdinaryOnly", "Recipe::IfTrue", (105, 233))

    expect_admitted("OrdinaryOnly", "Recipe::Goto", (106, 234, 235))

    expect_admitted("OrdinaryOnly", "Recipe::Call", (34, 236, 237, 238, 239))

    expect_admitted("OrdinaryOnly", "Recipe::TailCall", (35,))

    expect_admitted("OrdinaryOnly", "Recipe::Construct", (33,))

    expect_admitted("OrdinaryOnly", "Recipe::CallMethod", (36,))

    expect_admitted("OrdinaryOnly", "Recipe::TailCallMethod", (37,))

    expect_admitted("OrdinaryOnly", "Recipe::ArrayFrom", (38,))

    expect_admitted("OrdinaryOnly", "Recipe::Apply", (39,))

    expect_admitted("OrdinaryOnly", "Recipe::Throw", (48,))

    expect_admitted("OrdinaryOnly", "Recipe::ThrowReadOnly", (49,))

    expect_admitted("OrdinaryOnly", "Recipe::Nop", (177,))

    expect_admitted("OrdinaryOnly", "Recipe::Object", (11,))

    expect_admitted("OrdinaryOnly", "Recipe::ToObject", (111,))

    expect_admitted("OrdinaryOnly", "Recipe::ToPropKey", (112,))

    expect_admitted("OrdinaryOnly", "Recipe::PushThis", (8,))

    expect_admitted("OrdinaryOnly", "Recipe::PostDec", (142,))

    expect_admitted("OrdinaryOnly", "Recipe::PostInc", (143,))

    for raw, operation in (
        (152, "Mul"), (153, "Div"), (154, "Mod"), (155, "Add"), (156, "Sub"),
        (157, "Pow"), (158, "Shl"), (159, "Sar"), (160, "Shr"),
        (161, "LessThan"), (162, "LessThanOrEqual"), (163, "GreaterThan"),
        (164, "GreaterThanOrEqual"), (167, "Equal"), (168, "NotEqual"),
        (169, "StrictEqual"), (170, "StrictNotEqual"), (171, "BitAnd"),
        (172, "BitXor"), (173, "BitOr"),
    ):
        expect_admitted("OrdinaryOnly", f"Recipe::Binary(FunctionBinaryOp::{operation})", (raw,))

    for raw, predicate in (
        (174, "IsUndefinedOrNull"), (240, "IsUndefined"), (241, "IsNull"),
        (242, "TypeOfIsUndefined"), (243, "TypeOfIsFunction"),
    ):
        expect_admitted("OrdinaryOnly", f"Recipe::Predicate(FunctionPredicateOp::{predicate})", (raw,))

    found_admitted_registry = {
        raw: (audience, detail)
        for raw, _, audience, detail in registry_rows
        if audience != "Blocked"
    }

    if found_admitted_registry != expected_admitted_registry:
        ctx.fail(
            "function-translate-registry-policy",
            "the registry must preserve every admitted raw opcode, audience, and semantic recipe; "
            f"found {found_admitted_registry}",
        )

    expected_stage_boundaries = deepcopy(evidence.EXPECTED_STAGE_BOUNDARIES)

    found_stage_boundaries = {
        raw: (pinned_descriptors[raw], registry_rows[raw][2], registry_rows[raw][3])
        for raw in expected_stage_boundaries
        if raw < len(pinned_descriptors) and raw < len(registry_rows)
    }

    if found_stage_boundaries != expected_stage_boundaries:
        ctx.fail(
            "function-translate-stage-boundary",
            "reviewed invocation, return-undefined, explicit-throw, and plain-call boundaries must retain their pinned descriptors and policy rows; "
            f"found {found_stage_boundaries}",
        )

    stage_one_ordinary_rows = deepcopy(evidence.STAGE_ONE_ORDINARY_ROWS)

    former_scalar_rows = {6, 7, 9, 10, 138, 139, 140, 141, 147, 148, 149, 176, 191}

    found_stage_one_rows = tuple(
        raw
        for raw in stage_one_ordinary_rows
        if registry_rows[raw][2]
        == ("Shared" if raw in former_scalar_rows else "OrdinaryOnly")
    )

    if found_stage_one_rows != stage_one_ordinary_rows:
        ctx.fail(
            "function-translate-stage-one-set",
            "the reviewed stage-one 57-row ordinary cohort must remain admitted with its exact audiences; "
            f"found {found_stage_one_rows}",
        )

    stage_two_plain_call_rows = (34, 236, 237, 238, 239)

    found_stage_two_plain_call_rows = tuple(
        raw
        for raw, _, audience, detail in registry_rows
        if audience == "OrdinaryOnly" and detail == "Recipe::Call"
    )

    if found_stage_two_plain_call_rows != stage_two_plain_call_rows:
        ctx.fail(
            "function-translate-stage-two-set",
            "stage two must admit exactly raw call plus call0 through call3 as ordinary plain calls; "
            f"found {found_stage_two_plain_call_rows}",
        )

    stage_three_a_invocation_rows = (
        (33, "NPop", "Recipe::Construct"),
        (36, "NPop", "Recipe::CallMethod"),
        (38, "NPop", "Recipe::ArrayFrom"),
    )

    stage_three_a_recipe_names = {detail for _, _, detail in stage_three_a_invocation_rows}

    found_stage_three_a_invocation_rows = tuple(
        (raw, operand_format, detail)
        for raw, operand_format, audience, detail in registry_rows
        if audience == "OrdinaryOnly" and detail in stage_three_a_recipe_names
    )

    if found_stage_three_a_invocation_rows != stage_three_a_invocation_rows:
        ctx.fail(
            "function-translate-stage-three-a-set",
            "stage three A must admit exactly Construct raw 33, CallMethod raw 36, and ArrayFrom raw 38 with NPop operands; "
            f"found {found_stage_three_a_invocation_rows}",
        )

    stage_three_b_apply_rows = ((39, "U16", "Recipe::Apply"),)

    found_stage_three_b_apply_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_b_apply_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_b_apply_rows != stage_three_b_apply_rows:
        ctx.fail(
            "function-translate-stage-three-b-set",
            "stage three B must admit exactly raw 39 apply with its U16 operand and typed Apply recipe; "
            f"found {found_stage_three_b_apply_rows}",
        )

    stage_three_c_tail_rows = (
        (35, "NPop", "Recipe::TailCall"),
        (37, "NPop", "Recipe::TailCallMethod"),
    )

    found_stage_three_c_tail_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_c_tail_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_c_tail_rows != stage_three_c_tail_rows:
        ctx.fail(
            "function-translate-stage-three-c-set",
            "stage three C must admit exactly raw 35 TailCall and raw 37 TailCallMethod as distinct OrdinaryOnly NPop recipes; "
            f"found {found_stage_three_c_tail_rows}",
        )

    stage_three_d_throw_rows = ((48, "None", "Recipe::Throw"),)

    found_stage_three_d_throw_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_d_throw_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_d_throw_rows != stage_three_d_throw_rows:
        ctx.fail(
            "function-translate-stage-three-d-set",
            "stage three D must admit exactly operand-free raw 48 Throw as an OrdinaryOnly recipe; "
            f"found {found_stage_three_d_throw_rows}",
        )

    stage_three_e_throw_error_rows = ((49, "AtomU8", "Recipe::ThrowReadOnly"),)

    found_stage_three_e_throw_error_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_e_throw_error_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_e_throw_error_rows != stage_three_e_throw_error_rows:
        ctx.fail(
            "function-translate-stage-three-e-set",
            "stage three E must admit exactly raw 49 throw_error through its AtomU8 subtype-checked ThrowReadOnly recipe; "
            f"found {found_stage_three_e_throw_error_rows}",
        )

    stage_three_f_nop_rows = ((177, "None", "Recipe::Nop"),)

    found_stage_three_f_nop_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_f_nop_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_f_nop_rows != stage_three_f_nop_rows:
        ctx.fail(
            "function-translate-stage-three-f-set",
            "stage three F must admit exactly operand-free raw 177 nop as an OrdinaryOnly Nop recipe; "
            f"found {found_stage_three_f_nop_rows}",
        )

    stage_three_g_object_rows = ((11, "None", "Recipe::Object"),)

    found_stage_three_g_object_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_g_object_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_g_object_rows != stage_three_g_object_rows:
        ctx.fail(
            "function-translate-stage-three-g-set",
            "stage three G must admit exactly operand-free raw 11 object as an OrdinaryOnly Object recipe; "
            f"found {found_stage_three_g_object_rows}",
        )

    stage_three_h_to_object_rows = ((111, "None", "Recipe::ToObject"),)

    found_stage_three_h_to_object_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_h_to_object_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_h_to_object_rows != stage_three_h_to_object_rows:
        ctx.fail(
            "function-translate-stage-three-h-set",
            "stage three H must admit exactly operand-free raw 111 to_object as an OrdinaryOnly ToObject recipe; "
            f"found {found_stage_three_h_to_object_rows}",
        )

    stage_three_i_push_this_rows = ((8, "None", "Recipe::PushThis"),)

    found_stage_three_i_push_this_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_i_push_this_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_i_push_this_rows != stage_three_i_push_this_rows:
        ctx.fail(
            "function-translate-stage-three-i-set",
            "stage three I must admit exactly operand-free raw 8 push_this as an OrdinaryOnly PushThis recipe; "
            f"found {found_stage_three_i_push_this_rows}",
        )

    stage_three_j_to_propkey_rows = ((112, "None", "Recipe::ToPropKey"),)

    found_stage_three_j_to_propkey_rows = tuple(
        (raw, registry_rows[raw][1], registry_rows[raw][3])
        for raw, _, _ in stage_three_j_to_propkey_rows
        if registry_rows[raw][2] == "OrdinaryOnly"
    )

    if found_stage_three_j_to_propkey_rows != stage_three_j_to_propkey_rows:
        ctx.fail(
            "function-translate-stage-three-j-set",
            "stage three J must admit exactly operand-free raw 112 to_propkey as an OrdinaryOnly ToPropKey recipe; "
            f"found {found_stage_three_j_to_propkey_rows}",
        )

    stage_three_j_deferred_rows = {
        47: (("return_async", 1, 1, 0, "None"), "Blocked", "Completion"),
    }

    found_stage_three_j_deferred_rows = {
        raw: (pinned_descriptors[raw], registry_rows[raw][2], registry_rows[raw][3])
        for raw in stage_three_j_deferred_rows
        if raw < len(pinned_descriptors) and raw < len(registry_rows)
    }

    if found_stage_three_j_deferred_rows != stage_three_j_deferred_rows:
        ctx.fail(
            "function-translate-stage-three-j-set",
            "raw 47 return_async must remain blocked as Completion outside the Stage3J admission; "
            f"found {found_stage_three_j_deferred_rows}",
        )

    blocker_count_tokens = '\n    InvalidSentinel 1 ValueConstruction 4 FunctionGraph 2 Completion 1\n    EvalOrModule 3 Binding 7 Property 16 ObjectConstruction 15\n    LexicalEnvironment 25 ControlFlow 4 DynamicScope 9 Iteration 11 Suspension 5\n    Operator 4 Specialized 3\n'.split()

    expected_blocker_counts = dict(
        zip(blocker_count_tokens[::2], map(int, blocker_count_tokens[1::2]))
    )

    found_blocker_counts = {
        blocker: sum(
            audience == "Blocked" and detail == blocker
            for _, _, audience, detail in registry_rows
        )
        for blocker in expected_blocker_counts
    }

    unexpected_blockers = sorted(
        {
            detail
            for _, _, audience, detail in registry_rows
            if audience == "Blocked" and detail not in expected_blocker_counts
        }
    )

    if found_blocker_counts != expected_blocker_counts or unexpected_blockers:
        ctx.fail(
            "function-translate-registry-blockers",
            "the blocked frontier must retain all 15 nonempty typed categories and their exact counts, with the retired Invocation and Exception buckets absent; "
            f"found {found_blocker_counts} with unexpected {unexpected_blockers}",
        )

    blocked_registry_mapping = '\n'.join(
        f"{raw}:{detail}"
        for raw, _, audience, detail in registry_rows
        if audience == "Blocked"
    )

    blocked_registry_mapping_hash = hashlib.sha256(
        blocked_registry_mapping.encode("utf-8")
    ).hexdigest()

    if blocked_registry_mapping_hash != (
        "66222091ea38b5c6c11cb800df576d3f048379fe79b282f2ea5c4b42e90a9fa2"
    ):
        ctx.fail(
            "function-translate-registry-blockers",
            "every blocked raw opcode must retain its reviewed typed blocker, not only the aggregate category counts; "
            f"mapping sha256 {blocked_registry_mapping_hash}",
        )

    dto_blocker_code, ctx._, ctx._ = ctx.unique_braced_item(
        dto_production_code,
        re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
            r"[ \t\n]*::[ \t\n]*binary_object[ \t\n]*\)[ \t\n]+enum"
            r"[ \t\n]+TranslationBlocker[ \t\n]*\{"
        ),
        "function-translate-dto-shape",
        "TranslationBlocker enum",
    )

    if ctx.enum_variant_names(dto_blocker_code) != list(expected_blocker_counts):
        ctx.fail(
            "function-translate-dto-shape",
            "TranslationBlocker must retain the 15 reviewed ordered nonempty categories and no retired Invocation or Exception bucket; "
            f"found {ctx.enum_variant_names(dto_blocker_code)}",
        )

    dto_forbidden = re.compile(
        r"\b(?:FunctionId|ImageAtom|PinnedAtomId|PinnedOpcode|NativeAtomRef|"
        r"NativeCodePlan|NativeInstruction|NativeOperands|WireString|ImageCode|"
        r"ImageInstructionSpan|ImageRelocation|byte_pc|native_pc|operand_pc|"
        r"target_pc|source_pc|Runtime|Context|RuntimeError|Instruction|JsString|"
        r"Value|Vm|VmHost|RawValue|Heap|HeapObject|ObjectRef|UnlinkedFunction)\b|"
        r"\b(?:raw(?:_opcode)?|opcode_(?:raw|byte)|(?:atom|function|image|instruction)_id)"
        r"[ \t\n]*:"
    )

    for ctx.match in dto_forbidden.finditer(dto_production_code):
        ctx.fail(
            "function-translate-dto-representation",
            "sanitized translation DTOs must not retain native PCs, image identities, wire strings, or executable runtime types; found "
            + ctx.location(ctx.function_translate_dto_relative, dto_source, ctx.match.start()),
        )

    instruction_new_item, ctx._, ctx._ = ctx.unique_braced_item(
        dto_production_code,
        re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*super[ \t\n]*\)[ \t\n]+const[ \t\n]+fn"
            r"[ \t\n]+new[ \t\n]*\([ \t\n]*audience[ \t\n]*:"
        ),
        "function-translate-dto-representation",
        "FunctionInstruction constructor",
    )

    expected_instruction_new = deepcopy(evidence.EXPECTED_INSTRUCTION_NEW)

    if " ".join(instruction_new_item.split()) != expected_instruction_new:
        ctx.fail(
            "function-translate-dto-representation",
            "FunctionInstruction::new must store its sanitized audience, diagnostic, and operation unchanged",
        )

    forbidden_dto_traits = deepcopy(evidence.FORBIDDEN_DTO_TRAITS)

    dto_attribute_pattern = re.compile(
        r"(?P<attributes>(?:#[ \t\n]*\[[^]]*\][ \t\n]*)+)"
        r"(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?(?:enum|struct)"
        r"[ \t\n]+(?P<name>" + "|".join(forbidden_dto_traits) + r")\b"
    )

    for ctx.item in dto_attribute_pattern.finditer(dto_production_code):
        ctx.name = ctx.item.group("name")
        leaked = forbidden_dto_traits[ctx.name] & set(
            re.findall(r"\b[A-Za-z_][A-Za-z0-9_]*\b", ctx.item.group("attributes"))
        )
        if leaked:
            ctx.fail(
                "function-translate-dto-representation",
                f"{ctx.name} must not derive representation-revealing traits {sorted(leaked)}",
            )

    for ctx.name, traits in forbidden_dto_traits.items():
        for trait in traits:
            if re.search(
                rf"\bimpl\b[^{{;]*\b(?:fmt[ \t\n]*::[ \t\n]*)?{trait}\b[^{{;]*\bfor[ \t\n]+{ctx.name}\b",
                dto_production_code,
            ):
                ctx.fail(
                    "function-translate-dto-representation",
                    f"{ctx.name} must not implement representation-revealing trait {trait}",
                )

    translate_special_case_pattern = re.compile(
        r"(?i:\btest262\b|\bfixture(?:_[A-Za-z0-9_]+)?\b|"
        r"\b(?:source|input|bytes)_[A-Za-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))\b)|"
        r"\b(?:input|bytes)[ \t\n]*(?:\.[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\([^;\n]*\))*"
        r"\.[ \t\n]*(?:contains|starts_with|ends_with|windows)[ \t\n]*\("
    )

    for ctx.relative, ctx.source in (
        (ctx.function_translate_relative, function_translate_production_source),
        (ctx.function_translate_capability_relative, capability_production_source),
        (ctx.function_translate_dto_relative, dto_production_source),
    ):
        special_case = translate_special_case_pattern.search(ctx.source)
        if special_case is not None:
            ctx.fail(
                "function-translate-special-casing",
                "function translation must not admit by Test262 path, fixture identity, source bytes, or digest; found "
                + ctx.location(ctx.relative, ctx.source, special_case.start()),
            )

    ctx.translate_native_item, translate_native_start, translate_native_end = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\bfn[ \t\n]+translate_native_plan\b[^{};]*\{"),
        "function-translate-control-flow",
        "native-plan translation function",
    )

    translate_lower_item, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\bfn[ \t\n]+lower_operation\b[^{};]*\{"),
        "function-translate-semantic-dispatch",
        "recipe-based semantic lowering function",
    )

    translate_error_kind_code, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\benum[ \t\n]+FunctionTranslateErrorKind[ \t\n]*\{"),
        "function-translate-apply-admission",
        "translation error kind",
    )

    if ctx.enum_variant_names(translate_error_kind_code) != [
        "NativePlan",
        "RegistryDrift",
        "AllocationFailed",
        "InstructionCountOverflow",
        "InvalidBranchTarget",
        "AtomProjectionInvariant",
        "NonCanonicalApplyMagic",
        "UnadmittedThrowErrorSubtype",
    ]:
        ctx.fail(
            "function-translate-apply-admission",
            "translation errors must retain dedicated noncanonical-Apply and throw_error-subtype operand classes",
        )

    translate_target_item, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\bfn[ \t\n]+operation_for_target\b[^{};]*\{"),
        "function-translate-atom-order",
        "target-filtered operand materializer",
    )

    pending_expansion_item, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\bstruct[ \t\n]+PendingExpansion\b[^{};]*\{"),
        "function-translate-expansion",
        "fixed-capacity pending expansion carrier",
    )

    if " ".join(pending_expansion_item.split()) != (
        "struct PendingExpansion<'image> { "
        "operations: [Option<PendingOperation<'image>>; 4], len: u8, }"
    ):
        ctx.fail(
            "function-translate-expansion",
            "pending expansions must remain a fixed four-slot carrier with no per-op allocation",
        )

    ctx.pending_expansion_impl, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(
            r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+PendingExpansion"
            r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        ),
        "function-translate-expansion",
        "pending expansion implementation",
    )

    normalized_pending_expansion = " ".join(ctx.pending_expansion_impl.split())

    found_pending_initializers = re.findall(
        r"operations: \[([^]]+)\], len: ([1-4]),",
        normalized_pending_expansion,
    )

    expected_pending_initializers = deepcopy(evidence.EXPECTED_PENDING_INITIALIZERS)

    pending_helper_fragments = deepcopy(evidence.PENDING_HELPER_FRAGMENTS)

    if (
        found_pending_initializers != expected_pending_initializers
        or any(
            normalized_pending_expansion.count(fragment) != 1
            for fragment in pending_helper_fragments
        )
    ):
        ctx.fail(
            "function-translate-expansion",
            "pending expansion constructors, length, and iterator must preserve exact source order",
        )

    normalized_translate_target = " ".join(translate_target_item.split())

    if (
        normalized_translate_target.count(
            "if target.accepts(audience) { lower_operation(recipe, operands) } else { "
            "Ok(PendingExpansion::one(PendingOperation::Ready( FunctionOp::OutsideTarget, ))) }"
        )
        != 1
    ):
        ctx.fail(
            "function-translate-atom-order",
            "target audience rejection must precede all operand materialization",
        )

    normalized_translate_lower = " ".join(translate_lower_item.split())

    ctx.require_normalized_code_sha256(
        "function-translate-semantic-dispatch",
        "lower_operation must remain one alias-free typed Recipe/operand match with its unique ready publisher",
        translate_lower_item,
        "80936a383d38195b24c64242e118ef35944e4a22286da867eeddffe438327ba8",
    )

    if normalized_translate_lower.count(
        "let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));"
    ) != 1:
        ctx.fail(
            "function-translate-semantic-dispatch",
            "the single-step lowering closure must pass its typed operation through unchanged",
        )

    lower_match = re.search(
        r"match \(recipe, operands\) \{ (.*) _ => Err\(",
        normalized_translate_lower,
    )

    lowering_arm_matches = [] if lower_match is None else list(re.finditer(
        r"\( *(Recipe::.*?), (NativeOperands::.*?)(?:,)? *\) => (.*?)(?= \( *Recipe::|$)",
        lower_match.group(1).rstrip(", "),
    ))

    found_single_step_arms = []

    for ctx.arm in lowering_arm_matches:
        recipe_text, operands_text, ctx.body = ctx.arm.groups()
        if "StackRecipe::" in recipe_text and "StackRecipe::Direct" not in recipe_text:
            continue
        found_single_step_arms.append((recipe_text, operands_text, ctx.body.rstrip(", ")))

    single_step_rows = '\nRecipe::Nop @ NativeOperands::None @ ready(FunctionOp::Nop)\nRecipe::Object @ NativeOperands::None @ ready(FunctionOp::Object)\nRecipe::ToObject @ NativeOperands::None @ ready(FunctionOp::ToObject)\nRecipe::ToPropKey @ NativeOperands::None @ ready(FunctionOp::ToPropKey)\nRecipe::PushThis @ NativeOperands::None @ ready(FunctionOp::PushThis)\nRecipe::PushI32 @ NativeOperands::I32(value) | NativeOperands::NoneInt(value) @ { ready(FunctionOp::PushI32(*value)) }\nRecipe::PushI32 @ NativeOperands::I8(value) @ { ready(FunctionOp::PushI32(i32::from(*value))) }\nRecipe::PushI32 @ NativeOperands::I16(value) @ { ready(FunctionOp::PushI32(i32::from(*value))) }\nRecipe::PushConstant @ NativeOperands::Const(index) @ { ready(FunctionOp::PushConstant(*index)) }\nRecipe::PushConstant @ NativeOperands::Const8(index) @ { ready(FunctionOp::PushConstant(u32::from(*index))) }\nRecipe::PushAtom @ NativeOperands::Atom(atom) @ { ready(FunctionOp::PushAtom(project_atom(*atom)?)) }\nRecipe::PushUndefined @ NativeOperands::None @ ready(FunctionOp::PushUndefined)\nRecipe::PushNull @ NativeOperands::None @ ready(FunctionOp::PushNull)\nRecipe::PushFalse @ NativeOperands::None @ ready(FunctionOp::PushBool(false))\nRecipe::PushTrue @ NativeOperands::None @ ready(FunctionOp::PushBool(true))\nRecipe::PushBigIntI32 @ NativeOperands::I32(value) @ { ready(FunctionOp::PushBigIntI32(*value)) }\nRecipe::PushEmptyString @ NativeOperands::None @ ready(FunctionOp::PushEmptyString)\nRecipe::Stack(capability::StackRecipe::Direct(operation)) @ NativeOperands::None @ { ready(FunctionOp::Stack(operation)) }\nRecipe::Unary(operation) @ NativeOperands::None @ ready(FunctionOp::Unary(operation))\nRecipe::PostDec @ NativeOperands::None @ ready(FunctionOp::PostDec)\nRecipe::PostInc @ NativeOperands::None @ ready(FunctionOp::PostInc)\nRecipe::GetLocal @ NativeOperands::Loc(index) | NativeOperands::NoneLoc(index) @ { ready(FunctionOp::GetLocal(*index)) }\nRecipe::GetLocal @ NativeOperands::Loc8(index) @ { ready(FunctionOp::GetLocal(u16::from(*index))) }\nRecipe::PutLocal @ NativeOperands::Loc(index) | NativeOperands::NoneLoc(index) @ { ready(FunctionOp::PutLocal(*index)) }\nRecipe::PutLocal @ NativeOperands::Loc8(index) @ { ready(FunctionOp::PutLocal(u16::from(*index))) }\nRecipe::SetLocal @ NativeOperands::Loc(index) | NativeOperands::NoneLoc(index) @ { ready(FunctionOp::SetLocal(*index)) }\nRecipe::SetLocal @ NativeOperands::Loc8(index) @ { ready(FunctionOp::SetLocal(u16::from(*index))) }\nRecipe::GetArgument @ NativeOperands::Arg(index) | NativeOperands::NoneArg(index) @ { ready(FunctionOp::GetArgument(*index)) }\nRecipe::PutArgument @ NativeOperands::Arg(index) | NativeOperands::NoneArg(index) @ { ready(FunctionOp::PutArgument(*index)) }\nRecipe::SetArgument @ NativeOperands::Arg(index) | NativeOperands::NoneArg(index) @ { ready(FunctionOp::SetArgument(*index)) }\nRecipe::Binary(operation) @ NativeOperands::None @ ready(FunctionOp::Binary(operation))\nRecipe::Predicate(operation) @ NativeOperands::None @ { ready(FunctionOp::Predicate(operation)) }\nRecipe::IfFalse @ NativeOperands::Label(label) @ Ok(PendingExpansion::one( PendingOperation::IfFalse(label.target_instruction()), ))\nRecipe::IfFalse @ NativeOperands::Label8(label) @ Ok(PendingExpansion::one( PendingOperation::IfFalse(label.target_instruction()), ))\nRecipe::IfTrue @ NativeOperands::Label(label) @ Ok(PendingExpansion::one( PendingOperation::IfTrue(label.target_instruction()), ))\nRecipe::IfTrue @ NativeOperands::Label8(label) @ Ok(PendingExpansion::one( PendingOperation::IfTrue(label.target_instruction()), ))\nRecipe::Goto @ NativeOperands::Label(label) @ Ok(PendingExpansion::one( PendingOperation::Goto(label.target_instruction()), ))\nRecipe::Goto @ NativeOperands::Label8(label) @ Ok(PendingExpansion::one( PendingOperation::Goto(label.target_instruction()), ))\nRecipe::Goto @ NativeOperands::Label16(label) @ Ok(PendingExpansion::one( PendingOperation::Goto(label.target_instruction()), ))\nRecipe::Call @ NativeOperands::NPop(argument_count) | NativeOperands::NPopX(argument_count) @ ready(FunctionOp::Call(*argument_count))\nRecipe::TailCall @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::TailCall(*argument_count)) }\nRecipe::Construct @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::Construct(*argument_count)) }\nRecipe::CallMethod @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::CallMethod(*argument_count)) }\nRecipe::TailCallMethod @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::TailCallMethod(*argument_count)) }\nRecipe::ArrayFrom @ NativeOperands::NPop(argument_count) @ { ready(FunctionOp::ArrayFrom(*argument_count)) }\nRecipe::Apply @ NativeOperands::U16(0) @ { ready(FunctionOp::Apply(FunctionApplyKind::Call)) }\nRecipe::Apply @ NativeOperands::U16(1) @ { ready(FunctionOp::Apply(FunctionApplyKind::Construct)) }\nRecipe::Apply @ NativeOperands::U16(magic) @ { Err(FunctionTranslateError::non_canonical_apply_magic(*magic)) }\nRecipe::Return @ NativeOperands::None @ ready(FunctionOp::Return)\nRecipe::ReturnUndefined @ NativeOperands::None @ ready(FunctionOp::ReturnUndefined)\nRecipe::Throw @ NativeOperands::None @ ready(FunctionOp::Throw)\nRecipe::ThrowReadOnly @ NativeOperands::AtomU8 { atom, value: 0 } @ { ready(FunctionOp::ThrowReadOnly(project_atom(*atom)?)) }\nRecipe::ThrowReadOnly @ NativeOperands::AtomU8 { value, .. } @ Err( FunctionTranslateError::unadmitted_throw_error_subtype(*value), )\n'.strip().splitlines()

    expected_single_step_arms = [
        tuple(row.split(" @ ", 2)) for row in single_step_rows
    ]

    if len(lowering_arm_matches) != 59 or found_single_step_arms != expected_single_step_arms:
        ctx.fail(
            "function-translate-semantic-dispatch",
            "lower_operation must retain all 59 reviewed Recipe/operand arms, including operand-free Nop, Object, ToObject, ToPropKey, and PushThis, terminal tail invocations, explicit throw, and subtype-0 ThrowReadOnly, with each exact normalized RHS payload expression; "
            f"found {found_single_step_arms}",
        )

    apply_magic_error_contracts = deepcopy(evidence.APPLY_MAGIC_ERROR_CONTRACTS)

    for ctx.function_name, ctx.expected in apply_magic_error_contracts:
        ctx.item, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.function_translate_code,
            re.compile(rf"\bfn[ \t\n]+{ctx.function_name}\b[^{{}};]*\{{"),
            "function-translate-apply-admission",
            ctx.function_name,
        )
        if " ".join(ctx.item.split()) != ctx.expected:
            ctx.fail(
                "function-translate-apply-admission",
                f"{ctx.function_name} must retain the exact noncanonical-Apply and throw_error-subtype error classification",
            )

    stack_expansion_pattern = re.compile(
        r"\(Recipe::Stack\(capability::StackRecipe::(\w+)\), NativeOperands::None\)"
        r" => \{ Ok\(PendingExpansion::(two|three|four)\((.*?)\)\) \}"
    )

    stack_expansion_matches = stack_expansion_pattern.findall(normalized_translate_lower)

    found_stack_expansions = {
        recipe: (arity, tuple(re.findall(r"FunctionStackOp::(\w+)", body)))
        for recipe, arity, body in stack_expansion_matches
    }

    expected_stack_expansions = deepcopy(evidence.EXPECTED_STACK_EXPANSIONS)

    if (
        len(stack_expansion_matches) != 6
        or found_stack_expansions != expected_stack_expansions
        or normalized_translate_lower.count(
            "(Recipe::ReturnUndefined, NativeOperands::None) => ready(FunctionOp::ReturnUndefined),"
        ) != 1
    ):
        ctx.fail(
            "function-translate-expansion",
            "the six stack expansions and single zero-stack ReturnUndefined lowering must retain their exact allocation-free shape; "
            f"found {found_stack_expansions}",
        )

    if re.search(
        r"\b(?:OperationDiagnostic|opcode|diagnostic|mnemonic)\b|\.[ \t\n]*name[ \t\n]*\(",
        translate_lower_item,
    ):
        ctx.fail(
            "function-translate-semantic-dispatch",
            "semantic lowering must dispatch only on typed recipes and operands, never opcode names or diagnostics",
        )

    opcode_name_uses = list(
        re.finditer(r"\bopcode[ \t\n]*\.[ \t\n]*name[ \t\n]*\(", function_translate_production_code)
    )

    if len(opcode_name_uses) != 2 or any(
        not (translate_native_start <= match.start() < translate_native_end)
        for match in opcode_name_uses
    ):
        ctx.fail(
            "function-translate-diagnostic-boundary",
            "opcode.name() may appear only in the two compatibility-diagnostic constructions inside translate_native_plan",
        )

    normalized_translate_native = " ".join(ctx.translate_native_item.split())

    diagnostic_fragments = deepcopy(evidence.DIAGNOSTIC_FRAGMENTS)

    if any(normalized_translate_native.count(fragment) != 1 for fragment in diagnostic_fragments):
        ctx.fail(
            "function-translate-diagnostic-boundary",
            "the opcode mnemonic may feed only registry-drift and rejection-text diagnostics",
        )

    branch_fragments = deepcopy(evidence.BRANCH_FRAGMENTS)

    if any(normalized_translate_native.count(fragment) != 1 for fragment in branch_fragments):
        ctx.fail(
            "function-translate-control-flow",
            "translation must retain one source-to-output map and resolve all three branch kinds through it in the second pass",
        )

    source_map_fragments = deepcopy(evidence.SOURCE_MAP_FRAGMENTS)

    source_map_offsets = [
        normalized_translate_native.find(fragment) for fragment in source_map_fragments
    ]

    output_len_writes = re.findall(
        r"\boutput_len[ \t\n]*(?:[+*/%&|^-]?=)(?!=)",
        ctx.translate_native_item,
    )

    output_index_writes = re.findall(
        r"\boutput_index[ \t\n]*(?:[+*/%&|^-]?=)(?!=)",
        ctx.translate_native_item,
    )

    if (
        any(offset < 0 for offset in source_map_offsets)
        or source_map_offsets != sorted(source_map_offsets)
        or any(normalized_translate_native.count(fragment) != 1 for fragment in source_map_fragments)
        or len(output_len_writes) != 2
        or len(output_index_writes) != 1
        or normalized_translate_native.count(
            "pending .try_reserve_exact(plan.instructions().len())"
        ) != 1
    ):
        ctx.fail(
            "function-translate-control-flow",
            "each physical source must reserve one pending slot and map to the cumulative output length before its expansion",
        )

    resolve_target_item, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\bfn[ \t\n]+resolve_target\b[^{};]*\{"),
        "function-translate-control-flow",
        "sanitized branch-target resolver",
    )

    normalized_resolve_target = " ".join(resolve_target_item.split())

    expected_resolve_target = deepcopy(evidence.EXPECTED_RESOLVE_TARGET)

    if normalized_resolve_target != expected_resolve_target:
        ctx.fail(
            "function-translate-control-flow",
            "branch targets must remain bounds-checked instruction indexes in the sanitized output map",
        )

    ctx.translate_atom_item, ctx._, ctx._ = ctx.unique_braced_item(
        function_translate_production_code,
        re.compile(r"\bfn[ \t\n]+project_atom\b[^{};]*\{"),
        "function-translate-atom-order",
        "borrowed semantic atom projection",
    )

    ctx.require_normalized_code_sha256(
        "function-translate-atom-order",
        "project_atom must preserve String spelling and input-table provenance without allocation or class aliasing",
        ctx.translate_atom_item,
        "b61b799820c1c462fc2000f1ecd8b9e0698446da2e9b70f0bbb9f13de099b58c",
    )

    if re.search(
        r"\b(?:Vec|Box|String)[ \t\n]*(?:::|<)|\b(?:try_reserve|reserve|collect|"
        r"to_owned|to_vec|into_boxed_slice)[ \t\n]*\(",
        ctx.translate_atom_item,
    ):
        ctx.fail(
            "function-translate-atom-order",
            "atom translation must remain allocation-free so scalar rejection and OOM ordering stay unchanged",
        )
