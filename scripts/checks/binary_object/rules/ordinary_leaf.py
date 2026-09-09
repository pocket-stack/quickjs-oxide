"""Ordinary leaf checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import ordinary_leaf as evidence


def check(ctx):
    translate_atom_classes = re.findall(
        r"NativeAtomClass[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
        ctx.translate_atom_item,
    )

    if translate_atom_classes != ["Null", "Index", "String", "Private", "Symbol"]:
        ctx.fail(
            "function-translate-atom-order",
            "borrowed atom projection must preserve all five semantic identity classes in order; "
            f"found {translate_atom_classes}",
        )

    ordinary_leaf_relative = "src/engine/code/binary_object/ordinary_leaf.rs"

    ordinary_leaf_source = ctx.read_source(ordinary_leaf_relative)

    ordinary_leaf_code = ctx.rust_code_only(ordinary_leaf_source)

    ordinary_leaf_production_code = ordinary_leaf_code.split("#[cfg(test)]", 1)[0]

    ordinary_leaf_production_source = ordinary_leaf_source.split("#[cfg(test)]", 1)[0]

    ordinary_visibility = (
        r"pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*\)"
    )

    ordinary_visible_item_pattern = re.compile(
        r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
        r"(?:(?:const|async|unsafe|extern)[ \t\n]+)*"
        r"(?P<kind>enum|struct|union|trait|type|fn|const|static|use|mod)[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )

    ordinary_visible_items = [
        (
            " ".join(match.group("visibility").split()),
            match.group("kind"),
            match.group("name"),
        )
        for match in ordinary_visible_item_pattern.finditer(ordinary_leaf_production_code)
    ]

    expected_ordinary_visible_items = [
        ("pub(in crate::engine::code)", kind, name)
        for kind, name in (
            entry.split(":", 1)
            for entry in '\n        struct:RootFunctionConstantSelector fn:from_zero_based fn:zero_based\n        struct:OrdinaryLeafMetadataDraft fn:argument_count fn:defined_argument_count\n        fn:local_count fn:max_stack fn:is_strict fn:has_simple_parameter_list\n        fn:has_prototype fn:allows_new_target fn:allows_arguments fn:strip_variable_debug\n        enum:DetachedPrimitive struct:DetachedAtomName fn:into_units\n        enum:OrdinaryLeafOp enum:OrdinaryLeafApplyKind enum:OrdinaryLeafStackOp\n        enum:OrdinaryLeafUnaryOp enum:OrdinaryLeafBinaryOp enum:OrdinaryLeafPredicateOp\n        struct:OrdinaryLeafDraft fn:metadata fn:constants fn:code fn:into_parts\n        enum:OrdinaryLeafReadError\n        fn:decode_trusted_ordinary_leaf\n        '.split()
        )
    ]

    if ordinary_visible_items != expected_ordinary_visible_items:
        ctx.fail(
            "ordinary-leaf-visible-item-set",
            "ordinary_leaf.rs may expose only the reviewed selector, owned semantic DTOs, accessors, error, and decoder to runtime; "
            f"found {ordinary_visible_items}",
        )

    ordinary_top_level_item_pattern = re.compile(
        r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
        r"(?P<kind>struct|enum|union|trait|type|mod)[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )

    ordinary_top_level_items = [
        (match.group("kind"), match.group("name"))
        for match in ordinary_top_level_item_pattern.finditer(ordinary_leaf_production_code)
        if ordinary_leaf_production_code[:match.start()].count("{")
        == ordinary_leaf_production_code[:match.start()].count("}")
    ]

    expected_ordinary_top_level_items = [
        tuple(entry.split(":", 1))
        for entry in '\n    struct:RootFunctionConstantSelector struct:OrdinaryLeafMetadataDraft\n    enum:DetachedPrimitive struct:DetachedAtomName enum:OrdinaryLeafOp\n    enum:OrdinaryLeafApplyKind enum:OrdinaryLeafStackOp\n    enum:OrdinaryLeafUnaryOp enum:OrdinaryLeafBinaryOp enum:OrdinaryLeafPredicateOp\n    struct:OrdinaryLeafDraft enum:OrdinaryLeafReadError struct:AdmissionLimits\n    struct:InputAtomLedger\n    '.split()
    ]

    if ordinary_top_level_items != expected_ordinary_top_level_items:
        ctx.fail(
            "ordinary-leaf-top-level-item-set",
            "ordinary_leaf.rs must retain exactly the reviewed DTO and private state types, with no module, trait, alias, union, or helper type escape; "
            f"found {ordinary_top_level_items}",
        )

    ordinary_leaf_op_code, ctx._, ctx._ = ctx.unique_braced_item(
        ordinary_leaf_production_code,
        re.compile(r"\benum[ \t\n]+OrdinaryLeafOp[ \t\n]*\{"),
        "ordinary-leaf-operation-shape",
        "OrdinaryLeafOp enum",
    )

    expected_ordinary_leaf_op_variants = '\n    Nop Object ToObject ToPropKey PushThis PushI32 PushConst PushUndefined PushNull PushBool PushBigIntI32 PushEmptyString\n    Stack Unary PostDec PostInc GetLocal PutLocal SetLocal GetArgument PutArgument\n    SetArgument Binary Predicate IfFalse IfTrue Goto Call TailCall Construct\n    CallMethod TailCallMethod ArrayFrom Apply Return ReturnUndefined Throw ThrowReadOnly\n'.split()

    ordinary_invocation_payloads = {
        name: [
            " ".join(payload.split())
            for payload in re.findall(
                rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", ordinary_leaf_op_code
            )
        ]
        for name in ctx.invocation_variant_names
    }

    if (
        ctx.enum_variant_names(ordinary_leaf_op_code) != expected_ordinary_leaf_op_variants
        or any(ordinary_invocation_payloads[name] != ["u16"]
               for name in ctx.counted_invocation_variant_names)
        or ordinary_invocation_payloads["Apply"] != ["OrdinaryLeafApplyKind"]
    ):
        ctx.fail(
            "ordinary-leaf-operation-shape",
            "OrdinaryLeafOp must retain the exact reviewed inventory with operand-free Nop/Object/ToObject/ToPropKey/PushThis, distinct counted u16 invocation payloads, a typed Apply kind, and operand-free Throw; "
            f"found {ctx.enum_variant_names(ordinary_leaf_op_code)} with invocation payloads {ordinary_invocation_payloads}",
        )

    ordinary_throw_read_only_payloads = [
        " ".join(payload.split())
        for payload in re.findall(
            r"\bThrowReadOnly[ \t\n]*\(([^()]*)\)[ \t\n]*,",
            ordinary_leaf_op_code,
        )
    ]

    if ordinary_throw_read_only_payloads != ["DetachedAtomName"]:
        ctx.fail(
            "ordinary-leaf-operation-shape",
            "OrdinaryLeafOp::ThrowReadOnly must retain exactly one owned DetachedAtomName payload",
        )

    ordinary_apply_kind_code, ctx._, ctx._ = ctx.unique_braced_item(
        ordinary_leaf_production_code,
        re.compile(
            ordinary_visibility
            + r"[ \t\n]+enum[ \t\n]+OrdinaryLeafApplyKind[ \t\n]*\{"
        ),
        "ordinary-leaf-apply-kind",
        "owned ordinary apply kind",
    )

    if ctx.enum_variant_names(ordinary_apply_kind_code) != ["Call", "Construct"]:
        ctx.fail(
            "ordinary-leaf-apply-kind",
            "OrdinaryLeafApplyKind must expose only call and construct semantics; "
            f"found {ctx.enum_variant_names(ordinary_apply_kind_code)}",
        )

    ordinary_top_level_functions = [
        match.group("name")
        for match in re.finditer(
            r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?fn"
            r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
            ordinary_leaf_production_code,
        )
        if ordinary_leaf_production_code[:match.start()].count("{")
        == ordinary_leaf_production_code[:match.start()].count("}")
    ]

    expected_ordinary_top_level_functions = '\n    decode_trusted_ordinary_leaf admit_image preflight_constants project_primitive\n    copy_wire_string copy_bigint lower_code validate_push_this_protocol lower_operation copy_read_only_name lower_constant lower_local\n    lower_argument validate_ir_target unsupported_operation classify_translation_error\n    unadmitted classify_image_error classify_atom_error classify_wire_error\n    classify_data_error classify_envelope_error classify_code_error\n'.split()

    if ordinary_top_level_functions != expected_ordinary_top_level_functions:
        ctx.fail(
            "ordinary-leaf-helper-set",
            "ordinary_leaf.rs production free-function ownership drifted from the reviewed helper set; "
            f"found {ordinary_top_level_functions}",
        )

    ordinary_lower_code, ctx._, ctx._ = ctx.unique_braced_item(
        ordinary_leaf_production_code,
        re.compile(r"\bfn[ \t\n]+lower_code\b[^{};]*\{"),
        "ordinary-leaf-translated-code",
        "sanitized ordinary code consumer",
    )

    ordinary_lower_operation, ctx._, ctx._ = ctx.unique_braced_item(
        ordinary_leaf_production_code,
        re.compile(r"\bfn[ \t\n]+lower_operation\b[^{};]*\{"),
        "ordinary-leaf-translated-code",
        "sanitized ordinary operation lowering",
    )

    ctx.require_normalized_code_sha256(
        "ordinary-leaf-translated-code",
        "ordinary lower_operation must remain one alias-free exhaustive typed handoff",
        ordinary_lower_operation,
        "32c17de1021480b9ac7eaf63113a9575098b4329c6f4261eec12a65b6498816a",
    )

    ordinary_handoff_rows = '\nFunctionOp::Nop @ Ok(OrdinaryLeafOp::Nop)\nFunctionOp::Object @ Ok(OrdinaryLeafOp::Object)\nFunctionOp::ToObject @ Ok(OrdinaryLeafOp::ToObject)\nFunctionOp::ToPropKey @ Ok(OrdinaryLeafOp::ToPropKey)\nFunctionOp::PushThis @ Ok(OrdinaryLeafOp::PushThis)\nFunctionOp::PushI32(value) @ Ok(OrdinaryLeafOp::PushI32(*value))\nFunctionOp::PushConstant(index) @ lower_constant(*index, constant_count)\nFunctionOp::PushUndefined @ Ok(OrdinaryLeafOp::PushUndefined)\nFunctionOp::PushNull @ Ok(OrdinaryLeafOp::PushNull)\nFunctionOp::PushBool(value) @ Ok(OrdinaryLeafOp::PushBool(*value))\nFunctionOp::PushBigIntI32(value) @ Ok(OrdinaryLeafOp::PushBigIntI32(*value))\nFunctionOp::PushEmptyString @ Ok(OrdinaryLeafOp::PushEmptyString)\nFunctionOp::Stack(operation) @ Ok(OrdinaryLeafOp::Stack(match operation { FunctionStackOp::Drop => OrdinaryLeafStackOp::Drop, FunctionStackOp::Nip => OrdinaryLeafStackOp::Nip, FunctionStackOp::Dup => OrdinaryLeafStackOp::Dup, FunctionStackOp::Dup1 => OrdinaryLeafStackOp::Dup1, FunctionStackOp::Dup3 => OrdinaryLeafStackOp::Dup3, FunctionStackOp::Insert2 => OrdinaryLeafStackOp::Insert2, FunctionStackOp::Insert3 => OrdinaryLeafStackOp::Insert3, FunctionStackOp::Insert4 => OrdinaryLeafStackOp::Insert4, FunctionStackOp::Perm3 => OrdinaryLeafStackOp::Perm3, FunctionStackOp::Perm4 => OrdinaryLeafStackOp::Perm4, FunctionStackOp::Perm5 => OrdinaryLeafStackOp::Perm5, FunctionStackOp::Swap => OrdinaryLeafStackOp::Swap, FunctionStackOp::Rot4Left => OrdinaryLeafStackOp::Rot4Left, }))\nFunctionOp::Unary(operation) @ Ok(OrdinaryLeafOp::Unary(match operation { FunctionUnaryOp::Neg => OrdinaryLeafUnaryOp::Neg, FunctionUnaryOp::Plus => OrdinaryLeafUnaryOp::Plus, FunctionUnaryOp::Dec => OrdinaryLeafUnaryOp::Dec, FunctionUnaryOp::Inc => OrdinaryLeafUnaryOp::Inc, FunctionUnaryOp::BitNot => OrdinaryLeafUnaryOp::BitNot, FunctionUnaryOp::LogicalNot => OrdinaryLeafUnaryOp::LogicalNot, FunctionUnaryOp::TypeOf => OrdinaryLeafUnaryOp::TypeOf, }))\nFunctionOp::PostDec @ Ok(OrdinaryLeafOp::PostDec)\nFunctionOp::PostInc @ Ok(OrdinaryLeafOp::PostInc)\nFunctionOp::GetLocal(index) @ lower_local(*index, local_count, OrdinaryLeafOp::GetLocal)\nFunctionOp::PutLocal(index) @ lower_local(*index, local_count, OrdinaryLeafOp::PutLocal)\nFunctionOp::SetLocal(index) @ lower_local(*index, local_count, OrdinaryLeafOp::SetLocal)\nFunctionOp::GetArgument(index) @ { lower_argument(*index, argument_count, OrdinaryLeafOp::GetArgument) }\nFunctionOp::PutArgument(index) @ { lower_argument(*index, argument_count, OrdinaryLeafOp::PutArgument) }\nFunctionOp::SetArgument(index) @ { lower_argument(*index, argument_count, OrdinaryLeafOp::SetArgument) }\nFunctionOp::Binary(operation) @ Ok(OrdinaryLeafOp::Binary(match operation { FunctionBinaryOp::Add => OrdinaryLeafBinaryOp::Add, FunctionBinaryOp::Sub => OrdinaryLeafBinaryOp::Sub, FunctionBinaryOp::Mul => OrdinaryLeafBinaryOp::Mul, FunctionBinaryOp::Div => OrdinaryLeafBinaryOp::Div, FunctionBinaryOp::Mod => OrdinaryLeafBinaryOp::Mod, FunctionBinaryOp::Pow => OrdinaryLeafBinaryOp::Pow, FunctionBinaryOp::Shl => OrdinaryLeafBinaryOp::Shl, FunctionBinaryOp::Sar => OrdinaryLeafBinaryOp::Sar, FunctionBinaryOp::Shr => OrdinaryLeafBinaryOp::Shr, FunctionBinaryOp::LessThan => OrdinaryLeafBinaryOp::LessThan, FunctionBinaryOp::LessThanOrEqual => OrdinaryLeafBinaryOp::LessThanOrEqual, FunctionBinaryOp::GreaterThan => OrdinaryLeafBinaryOp::GreaterThan, FunctionBinaryOp::GreaterThanOrEqual => OrdinaryLeafBinaryOp::GreaterThanOrEqual, FunctionBinaryOp::Equal => OrdinaryLeafBinaryOp::Equal, FunctionBinaryOp::NotEqual => OrdinaryLeafBinaryOp::NotEqual, FunctionBinaryOp::StrictEqual => OrdinaryLeafBinaryOp::StrictEqual, FunctionBinaryOp::StrictNotEqual => OrdinaryLeafBinaryOp::StrictNotEqual, FunctionBinaryOp::BitAnd => OrdinaryLeafBinaryOp::BitAnd, FunctionBinaryOp::BitXor => OrdinaryLeafBinaryOp::BitXor, FunctionBinaryOp::BitOr => OrdinaryLeafBinaryOp::BitOr, }))\nFunctionOp::Predicate(operation) @ Ok(OrdinaryLeafOp::Predicate(match operation { FunctionPredicateOp::IsUndefinedOrNull => OrdinaryLeafPredicateOp::IsUndefinedOrNull, FunctionPredicateOp::IsUndefined => OrdinaryLeafPredicateOp::IsUndefined, FunctionPredicateOp::IsNull => OrdinaryLeafPredicateOp::IsNull, FunctionPredicateOp::TypeOfIsUndefined => OrdinaryLeafPredicateOp::TypeOfIsUndefined, FunctionPredicateOp::TypeOfIsFunction => OrdinaryLeafPredicateOp::TypeOfIsFunction, }))\nFunctionOp::IfFalse(target) @ { validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::IfFalse) }\nFunctionOp::IfTrue(target) @ { validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::IfTrue) }\nFunctionOp::Goto(target) @ { validate_ir_target(*target, instruction_count).map(OrdinaryLeafOp::Goto) }\nFunctionOp::Call(argument_count) @ Ok(OrdinaryLeafOp::Call(*argument_count))\nFunctionOp::TailCall(argument_count) @ Ok(OrdinaryLeafOp::TailCall(*argument_count))\nFunctionOp::Construct(argument_count) @ Ok(OrdinaryLeafOp::Construct(*argument_count))\nFunctionOp::CallMethod(argument_count) @ Ok(OrdinaryLeafOp::CallMethod(*argument_count))\nFunctionOp::TailCallMethod(argument_count) @ { Ok(OrdinaryLeafOp::TailCallMethod(*argument_count)) }\nFunctionOp::ArrayFrom(element_count) @ Ok(OrdinaryLeafOp::ArrayFrom(*element_count))\nFunctionOp::Apply(kind) @ Ok(OrdinaryLeafOp::Apply(match kind { FunctionApplyKind::Call => OrdinaryLeafApplyKind::Call, FunctionApplyKind::Construct => OrdinaryLeafApplyKind::Construct, }))\nFunctionOp::Return @ Ok(OrdinaryLeafOp::Return)\nFunctionOp::ReturnUndefined @ Ok(OrdinaryLeafOp::ReturnUndefined)\nFunctionOp::Throw @ Ok(OrdinaryLeafOp::Throw)\nFunctionOp::ThrowReadOnly(atom) @ { copy_read_only_name(atom).map(OrdinaryLeafOp::ThrowReadOnly) }\n'.strip().splitlines()

    expected_ordinary_handoff = [tuple(row.split(" @ ", 1)) for row in ordinary_handoff_rows]

    found_ordinary_handoff = ctx.rustfmt_match_arms(ordinary_lower_operation, "FunctionOp::")

    if found_ordinary_handoff != expected_ordinary_handoff:
        ctx.fail(
            "ordinary-leaf-translated-code",
            "all 38 sanitized operations must retain their exact ordinary-leaf payload and typed-family mapping; "
            f"found {found_ordinary_handoff}",
        )

    if (
        len(re.findall(r"\binstruction[ \t\n]*\.[ \t\n]*supports_ordinary[ \t\n]*\(", ordinary_lower_code)) != 1
        or len(re.findall(r"\binstruction[ \t\n]*\.[ \t\n]*operation[ \t\n]*\(", ordinary_lower_code)) != 2
        or re.search(r"\b(?:NativeCodePlan|NativeOperands|PinnedOpcode|opcode)\b", ordinary_lower_code)
        or re.search(r"\b(?:OperationDiagnostic|diagnostic|mnemonic)\b", ordinary_lower_operation)
    ):
        ctx.fail(
            "ordinary-leaf-translated-code",
            "ordinary_leaf must filter the sanitized audience before typed lowering and must not consult native opcodes or diagnostics while lowering",
        )

    ordinary_unsupported_item, unsupported_start, unsupported_end = ctx.unique_braced_item(
        ordinary_leaf_production_code,
        re.compile(r"\bfn[ \t\n]+unsupported_operation\b[^{};]*\{"),
        "function-translate-diagnostic-boundary",
        "ordinary rejection-text formatter",
    )

    ordinary_translate_error_classifier, ctx._, ctx._ = ctx.unique_braced_item(
        ordinary_leaf_production_code,
        re.compile(r"\bfn[ \t\n]+classify_translation_error\b[^{};]*\{"),
        "ordinary-leaf-apply-admission",
        "ordinary translation-error classifier",
    )

    normalized_translate_error_classifier = " ".join(
        ordinary_translate_error_classifier.split()
    )

    apply_admission_fragments = deepcopy(evidence.APPLY_ADMISSION_FRAGMENTS)

    apply_admission_offsets = [
        normalized_translate_error_classifier.find(fragment)
        for fragment in apply_admission_fragments
    ]

    if (
        any(offset < 0 for offset in apply_admission_offsets)
        or apply_admission_offsets != sorted(apply_admission_offsets)
        or any(
            normalized_translate_error_classifier.count(fragment) != 1
            for fragment in apply_admission_fragments
        )
    ):
        ctx.fail(
            "ordinary-leaf-apply-admission",
            "noncanonical apply operands must become Unadmitted before draft publication while internal translation failures remain Internal",
        )

    if ordinary_unsupported_item and (
        len(re.findall(r"\bdiagnostic[ \t\n]*\.[ \t\n]*mnemonic[ \t\n]*\(", ordinary_unsupported_item)) != 1
        or len(re.findall(r"\bdiagnostic[ \t\n]*\.[ \t\n]*operand_shape[ \t\n]*\(", ordinary_unsupported_item)) != 1
        or len(re.findall(r"\bOrdinaryLeafReadError[ \t\n]*::[ \t\n]*Unadmitted\b", ordinary_unsupported_item)) != 1
    ):
        ctx.fail(
            "function-translate-diagnostic-boundary",
            "the compatibility diagnostic may be consumed only to format an ordinary-leaf Unadmitted rejection",
        )

    ordinary_rejection_diagnostics = list(
        re.finditer(r"\.[ \t\n]*rejection_diagnostic[ \t\n]*\(", ordinary_leaf_production_code)
    )

    normalized_ordinary_lower_code = " ".join(ordinary_lower_code.split())

    if (
        len(ordinary_rejection_diagnostics) != 1
        or normalized_ordinary_lower_code.count(
            "return Err(unsupported_operation(instruction.rejection_diagnostic()));"
        )
        != 1
    ):
        ctx.fail(
            "function-translate-diagnostic-boundary",
            "ordinary code may read one compatibility diagnostic only in the !supports_ordinary rejection branch",
        )

    for accessor in ("mnemonic", "operand_shape"):
        accessor_uses = list(
            re.finditer(rf"\.[ \t\n]*{accessor}[ \t\n]*\(", ordinary_leaf_production_code)
        )
        if (
            len(accessor_uses) != 1
            or not (unsupported_start <= accessor_uses[0].start() < unsupported_end)
        ):
            ctx.fail(
                "function-translate-diagnostic-boundary",
                "diagnostic mnemonic and operand shape may be read only by the ordinary rejection-text formatter",
            )

    ordinary_raw_dependency = re.search(
        r"\b(?:ImageAtom|PinnedAtomId|PinnedOpcode|NativeAtomRef|NativeCodePlan|"
        r"NativeInstruction|NativeOperands|ImageCode|ImageInstructionSpan|ImageRelocation)\b|"
        r"\.[ \t\n]*(?:as_bytes|atom_relocations)[ \t\n]*\(",
        ordinary_leaf_production_code,
    )

    if ordinary_raw_dependency is not None:
        ctx.fail(
            "ordinary-leaf-native-plan-boundary",
            "ordinary-leaf admission must consume only sanitized function DTOs and authenticated image APIs, never native plans, raw atom identities, code bytes, or relocation sidecars; found "
            + ctx.location(
                ordinary_leaf_relative,
                ordinary_leaf_source,
                ordinary_raw_dependency.start(),
            ),
        )

    ordinary_special_case_pattern = re.compile(
        r"(?i:\btest262\b|\bfixture(?:_[A-Za-z0-9_]+)?\b|"
        r"\b(?:source|input|bytes)_[A-Za-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))\b)|"
        r"\b(?:input|bytes)[ \t\n]*(?:\.[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\([^;\n]*\))*"
        r"\.[ \t\n]*(?:contains|starts_with|ends_with|windows)[ \t\n]*\(",
    )

    ordinary_special_case = ordinary_special_case_pattern.search(ordinary_leaf_production_source)

    if ordinary_special_case is not None:
        ctx.fail(
            "ordinary-leaf-special-casing",
            "ordinary-leaf production admission must remain structural and must not dispatch on Test262, fixture, digest, or exact input-byte identity; found "
            + ctx.location(
                ordinary_leaf_relative,
                ordinary_leaf_source,
                ordinary_special_case.start(),
            ),
        )

    ctx.scalar_script_relative = "src/engine/code/binary_object/scalar_script.rs"

    ctx.scalar_script_source = ctx.read_source(ctx.scalar_script_relative)

    ctx.scalar_script_code = ctx.rust_code_only(ctx.scalar_script_source)

    scalar_visibility = (
        r"pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::[ \t\n]*engine[ \t\n]*::[ \t\n]*code"
        r"[ \t\n]*\)"
    )

    scalar_noncopy_derive = (
        r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
        r"[ \t\n]*Debug[ \t\n]*,[ \t\n]*Eq[ \t\n]*,"
        r"[ \t\n]*PartialEq[ \t\n]*\)[ \t\n]*\]"
    )

    scalar_value_draft_pattern = re.compile(
        rf"{scalar_noncopy_derive}[ \t\n]*{scalar_visibility}"
        r"[ \t\n]+enum[ \t\n]+ScalarValueDraft[ \t\n]*\{"
        r"[ \t\n]*Undefined[ \t\n]*,"
        r"[ \t\n]*Null[ \t\n]*,"
        r"[ \t\n]*Bool[ \t\n]*\([ \t\n]*bool[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Int[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Float64Bits[ \t\n]*\([ \t\n]*u64[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*BigIntI32[ \t\n]*\([ \t\n]*i32[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*BigIntBytes[ \t\n]*\([ \t\n]*Box[ \t\n]*<"
        r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*>[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*EmptyString[ \t\n]*,"
        r"[ \t\n]*ConstantString[ \t\n]*\([ \t\n]*ScalarStringDraft[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*AtomString[ \t\n]*\([ \t\n]*ScalarStringDraft[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*IntegerAtomString[ \t\n]*\([ \t\n]*u32[ \t\n]*\)[ \t\n]*,?[ \t\n]*\}"
    )

    if len(scalar_value_draft_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-draft-shape",
            "ScalarValueDraft must retain exactly the reviewed primitive, BigInt, and provenance-typed String variants, with no precomputed unary result",
        )

    scalar_unary_pattern = re.compile(
        r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
        r"[ \t\n]*Copy[ \t\n]*,[ \t\n]*Debug[ \t\n]*,[ \t\n]*Eq"
        r"[ \t\n]*,[ \t\n]*PartialEq[ \t\n]*\)[ \t\n]*\]"
        rf"[ \t\n]*{scalar_visibility}[ \t\n]+enum[ \t\n]+ScalarUnaryOp"
        r"[ \t\n]*\{[ \t\n]*Neg[ \t\n]*,[ \t\n]*Plus[ \t\n]*,"
        r"[ \t\n]*Dec[ \t\n]*,[ \t\n]*Inc[ \t\n]*,[ \t\n]*BitNot"
        r"[ \t\n]*,[ \t\n]*LogicalNot[ \t\n]*,[ \t\n]*TypeOf"
        r"[ \t\n]*,?[ \t\n]*\}"
    )

    if len(scalar_unary_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-unary-operation-shape",
            "ScalarUnaryOp must retain exactly the seven reviewed ordered, Copy operation tags",
        )

    scalar_unary_impl_code, scalar_unary_impl_start, scalar_unary_impl_end = ctx.unique_braced_item(
        ctx.scalar_script_code,
        re.compile(r"\bimpl[ \t\n]+ScalarUnaryOp[ \t\n]*\{"),
        "scalar-unary-operation-shape",
        "private ScalarUnaryOp implementation",
    )

    scalar_unary_pairs = re.findall(
        r"FunctionUnaryOp[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)"
        r"[ \t\n]*=>[ \t\n]*Self[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
        scalar_unary_impl_code,
    )

    expected_scalar_unary_pairs = [
        ("Neg", "Neg"),
        ("Plus", "Plus"),
        ("Dec", "Dec"),
        ("Inc", "Inc"),
        ("BitNot", "BitNot"),
        ("LogicalNot", "LogicalNot"),
        ("TypeOf", "TypeOf"),
    ]

    if scalar_unary_pairs != expected_scalar_unary_pairs:
        ctx.fail(
            "scalar-unary-operation-shape",
            "ScalarUnaryOp must map the seven sanitized unary operations one-for-one; "
            f"found {scalar_unary_pairs}",
        )

    scalar_string_draft_pattern = re.compile(
        rf"{scalar_noncopy_derive}[ \t\n]*{scalar_visibility}[ \t\n]+struct"
        r"[ \t\n]+ScalarStringDraft[ \t\n]*\([ \t\n]*Box[ \t\n]*<"
        r"[ \t\n]*\[[ \t\n]*u16[ \t\n]*\][ \t\n]*>[ \t\n]*\)[ \t\n]*;"
    )

    if len(scalar_string_draft_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-draft-shape",
            "ScalarStringDraft must be one opaque runtime-visible owned UTF-16 code-unit buffer",
        )

    scalar_error_pattern = re.compile(
        rf"\b{scalar_visibility}[ \t\n]+enum[ \t\n]+ScalarScriptReadError[ \t\n]*\{{"
        r"[ \t\n]*Malformed[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Type[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Range[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*JsInternal[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Unadmitted[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Resource[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,"
        r"[ \t\n]*Internal[ \t\n]*\([ \t\n]*String[ \t\n]*\)[ \t\n]*,?[ \t\n]*\}"
    )

    if len(scalar_error_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-error-shape",
            "ScalarScriptReadError must retain exactly Malformed, Type, Range, JsInternal, Unadmitted, Resource, and Internal String variants",
        )

    scalar_decode_item_pattern = re.compile(
        rf"\b{scalar_visibility}[ \t\n]+fn[ \t\n]+decode_trusted_scalar_script"
        r"[ \t\n]*\([ \t\n]*(?:bytes|input)[ \t\n]*:[ \t\n]*&[ \t\n]*\[u8\]"
        r"[ \t\n]*,?[ \t\n]*\)[ \t\n]*->[ \t\n]*Result[ \t\n]*<"
        r"[ \t\n]*\([ \t\n]*ScalarValueDraft[ \t\n]*,[ \t\n]*Box"
        r"[ \t\n]*<[ \t\n]*\[[ \t\n]*ScalarUnaryOp[ \t\n]*\][ \t\n]*>"
        r"[ \t\n]*\)[ \t\n]*,[ \t\n]*ScalarScriptReadError[ \t\n]*>"
        r"[ \t\n]*\{"
    )

    scalar_decoder_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_script_code,
        scalar_decode_item_pattern,
        "scalar-script-decoder-shape",
        "runtime-visible &[u8] to scalar draft decoder",
    )

    if scalar_decoder_code:
        decoder_steps = (
            list(re.finditer(r"\bdecode_bytecode_image_body[ \t\n]*\(", scalar_decoder_code)),
            list(re.finditer(
                r"\bcursor[ \t\n]*\.[ \t\n]*finish[ \t\n]*\([ \t\n]*\)",
                scalar_decoder_code,
            )),
            list(re.finditer(
                r"\badmit_image[ \t\n]*\([ \t\n]*&[ \t\n]*image[ \t\n]*\)",
                scalar_decoder_code,
            )),
        )
        decoder_step_offsets = tuple(
            step[0].start() if len(step) == 1 else -1 for step in decoder_steps
        )
        if (
            any(len(step) != 1 for step in decoder_steps)
            or decoder_step_offsets != tuple(sorted(decoder_step_offsets))
            or re.search(r"\bOk[ \t\n]*\(", scalar_decoder_code)
            or re.search(
                r"\badmit_image[ \t\n]*\([ \t\n]*&[ \t\n]*image[ \t\n]*\)"
                r"[ \t\n]*\}[ \t\n]*\Z",
                scalar_decoder_code,
            ) is None
        ):
            ctx.fail(
                "scalar-script-decoder-shape",
                "decode_trusted_scalar_script must uniquely complete decode_bytecode_image_body, cursor.finish(), and final admit_image(&image), without an alternate Ok result",
            )

    scalar_opcode_declarations = re.findall(
        r"(?m)^[ \t]*const[ \t]+(OP_[A-Z0-9_]+)\b",
        ctx.scalar_script_code.split("#[cfg(test)]", 1)[0],
    )

    if scalar_opcode_declarations:
        ctx.fail(
            "scalar-script-opcode-set",
            "scalar-script production admission must not retain raw opcode constants after translation centralization; "
            f"found {scalar_opcode_declarations}",
        )

    scalar_push_pattern = re.compile(
        r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
        r"[ \t\n]*Debug[ \t\n]*\)[ \t\n]*\][ \t\n]*enum[ \t\n]+ScalarPush"
        r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        r"[ \t\n]*Direct[ \t\n]*\([ \t\n]*ScalarValueDraft[ \t\n]*\)"
        r"[ \t\n]*,[ \t\n]*Constant[ \t\n]*\([ \t\n]*u32[ \t\n]*\)"
        r"[ \t\n]*,[ \t\n]*AtomValue[ \t\n]*\([ \t\n]*AtomOperand"
        r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\)[ \t\n]*,?[ \t\n]*\}"
    )

    if len(scalar_push_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-push-shape",
            "ScalarPush must retain only a direct draft, constant index, or sanitized atom operand",
        )

    scalar_sequence_pattern = re.compile(
        r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([ \t\n]*Clone[ \t\n]*,"
        r"[ \t\n]*Debug[ \t\n]*\)[ \t\n]*\][ \t\n]*struct[ \t\n]+ScalarSequence"
        r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        r"[ \t\n]*push[ \t\n]*:[ \t\n]*ScalarPush"
        r"[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*,"
        r"[ \t\n]*unary_ops[ \t\n]*:[ \t\n]*Box[ \t\n]*<"
        r"[ \t\n]*\[[ \t\n]*ScalarUnaryOp[ \t\n]*\][ \t\n]*>"
        r"[ \t\n]*,?[ \t\n]*\}"
    )

    if len(scalar_sequence_pattern.findall(ctx.scalar_script_code)) != 1:
        ctx.fail(
            "scalar-script-sequence-shape",
            "ScalarSequence must retain one sanitized value push and one owned ordered unary-operation slice",
        )

    scalar_copy_pattern = re.compile(
        r"#[ \t\n]*\[[ \t\n]*derive[ \t\n]*\([^\]]*\bCopy\b[^\]]*\)"
        r"[ \t\n]*\](?:[ \t\n]*#[ \t\n]*\[[^\]]*\])*[ \t\n]*"
        rf"(?:{scalar_visibility}[ \t\n]+)?enum[ \t\n]+"
        r"(?:ScalarValueDraft|ScalarStringDraft|ScalarPush|ScalarSequence)\b|"
        r"\bimpl[ \t\n]+Copy[ \t\n]+for[ \t\n]+"
        r"(?:ScalarValueDraft|ScalarStringDraft|ScalarPush|ScalarSequence)\b"
    )

    if scalar_copy_pattern.search(ctx.scalar_script_code):
        ctx.fail(
            "scalar-script-draft-shape",
            "the scalar draft, push, and sequence discriminators must not regain Copy semantics around owned BigInt bytes",
        )

    scalar_sequence_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_script_code,
        re.compile(r"\bfn[ \t\n]+decode_scalar_sequence\b[^{};]*\{"),
        "scalar-script-translated-code",
        "sanitized scalar sequence decoder",
    )

    normalized_scalar_sequence = " ".join(scalar_sequence_code.split())

    scalar_sequence_fragments = deepcopy(evidence.SCALAR_SEQUENCE_FRAGMENTS)

    if (
        any(normalized_scalar_sequence.count(fragment) != 1 for fragment in scalar_sequence_fragments)
        or re.search(r"\b(?:NativeCodePlan|NativeOperands|PinnedOpcode|opcode|diagnostic|mnemonic)\b", scalar_sequence_code)
        or re.search(r"\bunary_ops[ \t\n]*\.[ \t\n]*(?:dedup|insert|last|reverse|sort)\b", scalar_sequence_code)
    ):
        ctx.fail(
            "scalar-script-translated-code",
            "scalar sequence admission must filter sanitized audiences, preserve set-local-zero/return and unary order, and consume the owned push without native or diagnostic dispatch",
        )

    scalar_native_production_code = ctx.scalar_script_code.split("#[cfg(test)]", 1)[0]

    raw_scalar_decoder_dependency = re.search(
        r"\b(?:ImageAtom|PinnedAtomId|PinnedOpcode|NativeAtomRef|NativeCodePlan|"
        r"NativeInstruction|NativeOperands|ImageCode|ImageInstructionSpan|ImageRelocation)\b|"
        r"\.[ \t\n]*(?:as_bytes|atom_relocations)[ \t\n]*\(",
        scalar_native_production_code,
    )

    if raw_scalar_decoder_dependency is not None:
        ctx.fail(
            "scalar-script-native-plan-decoder",
            "scalar admission must consume only sanitized function DTOs, never native plans, raw atom identities, archival code bytes, or relocation sidecars; found "
            + ctx.location(
                ctx.scalar_script_relative,
                ctx.scalar_script_source,
                raw_scalar_decoder_dependency.start(),
            ),
        )

    bigint_copy_pattern = re.compile(
        r"\bfn[ \t\n]+copy_bigint_bytes[ \t\n]*\([ \t\n]*bytes[ \t\n]*:"
        r"[ \t\n]*&[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*\)"
        r"[ \t\n]*->[ \t\n]*Result[ \t\n]*<[ \t\n]*Box[ \t\n]*<"
        r"[ \t\n]*\[[ \t\n]*u8[ \t\n]*\][ \t\n]*>[ \t\n]*,"
        r"[ \t\n]*ScalarScriptReadError[ \t\n]*>[ \t\n]*\{",
        re.DOTALL,
    )

    bigint_copy_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.scalar_script_code,
        bigint_copy_pattern,
        "scalar-script-bigint-copy",
        "fallible canonical BigInt byte-copy helper",
    )

    if bigint_copy_code:
        expected_bigint_copy_source = '\n        fn copy_bigint_bytes(bytes: &[u8]) -> Result<Box<[u8]>, ScalarScriptReadError> {\n            let mut copy = Vec::new();\n            copy.try_reserve_exact(bytes.len()).map_err(|_| {\n                ScalarScriptReadError::Internal("could not allocate the scalar BigInt draft".into())\n            })?;\n            copy.extend_from_slice(bytes);\n            Ok(copy.into_boxed_slice())\n        }\n    '
        if (
            " ".join(bigint_copy_code.split())
            != " ".join(ctx.rust_code_only(expected_bigint_copy_source).split())
        ):
            ctx.fail(
                "scalar-script-bigint-copy",
                "copy_bigint_bytes must perform one exact fallible reserve, byte-for-byte copy, and boxed ownership transfer",
            )

    normalized_scalar_script = " ".join(ctx.scalar_script_code.split())

    scalar_string_fragments = deepcopy(evidence.SCALAR_STRING_FRAGMENTS)

    utf16_copy_shape = re.compile(
        r"\bcopy[ \t\n]*\.[ \t\n]*try_reserve_exact[ \t\n]*\([ \t\n]*length"
        r"[ \t\n]*\).*?ScalarScriptReadError[ \t\n]*::[ \t\n]*JsInternal"
        r".*?\bcopy[ \t\n]*\.[ \t\n]*extend[ \t\n]*\([ \t\n]*units"
        r"[ \t\n]*\)[ \t\n]*;[ \t\n]*Ok[ \t\n]*\([ \t\n]*ScalarStringDraft"
        r"[ \t\n]*\([ \t\n]*copy[ \t\n]*\.[ \t\n]*into_boxed_slice",
        re.DOTALL,
    )

    if (
        any(normalized_scalar_script.count(fragment) != 1 for fragment in scalar_string_fragments)
        or len(utf16_copy_shape.findall(ctx.scalar_script_code)) != 1
    ):
        ctx.fail(
            "scalar-script-string-copy",
            "String drafts must cross the reader boundary once as exact, fallibly copied UTF-16 code units",
        )

    if re.search(r"\b(?:try_)?from_utf8\b|\bdecode_utf8\b", ctx.scalar_script_code):
        ctx.fail(
            "scalar-script-string-copy",
            "BC5 narrow Strings are Latin-1 code units and must not pass through a UTF-8 decoder",
        )

    scalar_visible_item_pattern = re.compile(
        r"\b(?P<visibility>pub(?:[ \t\n]*\([^)]*\))?)[ \t\n]+"
        r"(?P<kind>enum|struct|union|trait|type|fn|const|static|use|mod)[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )

    scalar_visible_items = [
        (
            " ".join(match.group("visibility").split()),
            match.group("kind"),
            match.group("name"),
        )
        for match in scalar_visible_item_pattern.finditer(ctx.scalar_script_code)
    ]

    expected_scalar_visible_items = deepcopy(evidence.EXPECTED_SCALAR_VISIBLE_ITEMS)

    if sorted(scalar_visible_items) != sorted(expected_scalar_visible_items):
        ctx.fail(
            "scalar-script-visible-item-set",
            "scalar_script.rs may expose only the reviewed value and unary DTOs, String draft, error, and decoder to runtime; "
            f"found {scalar_visible_items}",
        )

    ctx.scalar_production_code = ctx.scalar_script_code.split("#[cfg(test)]", 1)[0]

    scalar_top_level_item_pattern = re.compile(
        r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
        r"(?P<kind>struct|enum|union|trait|type|mod)[ \t\n]+"
        r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
    )

    ctx.scalar_top_level_items = [
        (match.group("kind"), match.group("name"))
        for match in scalar_top_level_item_pattern.finditer(ctx.scalar_production_code)
        if ctx.scalar_production_code[:match.start()].count("{")
        == ctx.scalar_production_code[:match.start()].count("}")
    ]

    ctx.expected_scalar_top_level_items = deepcopy(evidence.EXPECTED_SCALAR_TOP_LEVEL_ITEMS)
