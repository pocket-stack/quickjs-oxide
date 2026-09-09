"""Publication checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import publication as evidence


def check(ctx):
    if ctx.consumer_exists:
        consumer_production_code = ctx.consumer_code.split("#[cfg(test)]", 1)[0]
        consumer_top_level_item_pattern = re.compile(
            r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
            r"(?P<kind>struct|enum|union|trait|type|mod)[ \t\n]+"
            r"(?P<name>[A-Za-z_][A-Za-z0-9_]*)"
        )
        consumer_top_level_items = [
            (match.group("kind"), match.group("name"))
            for match in consumer_top_level_item_pattern.finditer(consumer_production_code)
            if consumer_production_code[:match.start()].count("{")
            == consumer_production_code[:match.start()].count("}")
        ]
        if consumer_top_level_items != [("enum", "LoweredScalar")]:
            ctx.fail(
                "binary-object-consumer-top-level-item-set",
                f"{ctx.consumer_relative} may own only the reviewed LoweredScalar type and no module, trait, alias, union, or helper type escape; "
                f"found {consumer_top_level_items}",
            )

        consumer_top_level_functions = [
            match.group("name")
            for match in re.finditer(
                r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?"
                r"(?:const[ \t\n]+)?fn"
                r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
                consumer_production_code,
            )
            if consumer_production_code[:match.start()].count("{")
            == consumer_production_code[:match.start()].count("}")
        ]
        expected_consumer_top_level_functions = [
            "lower_scalar_value",
            "lower_scalar_string",
            "lower_bigint_constant",
            "decode_bigint_constant",
            "lower_primitive_constant",
            "lower_detached_primitive",
            "lower_ordinary_leaf_op",
            "map_ordinary_leaf_verification_error",
            "map_ordinary_leaf_read_error",
            "map_read_error",
        ]
        if consumer_top_level_functions != expected_consumer_top_level_functions:
            ctx.fail(
                "binary-object-consumer-helper-set",
                f"{ctx.consumer_relative} production free-function ownership drifted from the reviewed helper set; "
                f"found {consumer_top_level_functions}",
            )

        consumer_impl_headers = [
            " ".join(match.group("header").split())
            for match in re.finditer(
                r"(?m)^[ \t]*impl\b(?P<header>[^{};]*)\{",
                consumer_production_code,
            )
            if consumer_production_code[:match.start()].count("{")
            == consumer_production_code[:match.start()].count("}")
        ]
        if consumer_impl_headers != ["Runtime"]:
            ctx.fail(
                "binary-object-consumer-implementation-set",
                f"{ctx.consumer_relative} must retain exactly one inherent Runtime implementation and no trait or alternate owner; "
                f"found {consumer_impl_headers}",
            )

        consumer_macro_invocations = re.findall(
            r"(?<![A-Za-z0-9_])((?:r#)?[A-Za-z_][A-Za-z0-9_]*)"
            r"[ \t\n]*![ \t\n]*[([{]",
            consumer_production_code,
        )
        if consumer_macro_invocations != [
            "matches",
            "vec",
            "vec",
            "format",
            "format",
            "format",
            "format",
            "format",
            "format",
            "format",
        ]:
            ctx.fail(
                "binary-object-consumer-macro-set",
                f"{ctx.consumer_relative} may invoke only its reviewed constant-vector and diagnostic macros; "
                f"found {consumer_macro_invocations}",
            )
        for ctx.match in re.finditer(
            r"(?<![A-Za-z0-9_])(?:r#)?include[ \t\n]*!",
            consumer_production_code,
        ):
            ctx.fail(
                "binary-object-consumer-source-include",
                f"{ctx.consumer_relative} must not splice unscanned Rust source; found "
                + ctx.location(ctx.consumer_relative, ctx.consumer_source, ctx.match.start()),
            )

        if len(ctx.consumer_facade_imports) != 1:
            ctx.fail(
                "binary-object-consumer-import",
                f"{ctx.consumer_relative} must contain exactly one reviewed scalar/ordinary facade import",
            )
        else:
            consumer_import_items = [
                item.strip()
                for item in ctx.consumer_facade_imports[0].group("body").split(",")
                if item.strip()
            ]
            expected_consumer_facade_names = ctx.expected_scalar_facade_names | {
                "DetachedPrimitive",
                "OrdinaryLeafApplyKind",
                "OrdinaryLeafBinaryOp",
                "OrdinaryLeafOp",
                "OrdinaryLeafPredicateOp",
                "OrdinaryLeafReadError",
                "OrdinaryLeafStackOp",
                "OrdinaryLeafUnaryOp",
                "RootFunctionConstantSelector",
                "decode_trusted_ordinary_leaf",
            }
            if (
                len(consumer_import_items) != len(expected_consumer_facade_names)
                or set(consumer_import_items)
                != expected_consumer_facade_names
            ):
                ctx.fail(
                    "binary-object-consumer-import",
                    f"{ctx.consumer_relative} may import only the reviewed scalar/ordinary facades; "
                    f"found {consumer_import_items}",
                )

        consumer_binary_mentions = list(re.finditer(r"\bbinary_object\b", ctx.consumer_code))
        if len(consumer_binary_mentions) != 1:
            ctx.fail(
                "binary-object-consumer-import",
                f"{ctx.consumer_relative} may name binary_object only in its one reviewed facade import",
            )

        safe_publication_calls = re.findall(
            r"\b(?:self|runtime)[ \t\n]*\.[ \t\n]*publish_unlinked_function[ \t\n]*\(",
            ctx.consumer_code,
        )
        if len(safe_publication_calls) != 1:
            ctx.fail(
                "binary-object-consumer-publication",
                f"{ctx.consumer_relative} must enter publish_unlinked_function exactly once",
            )
        verified_publication_calls = re.findall(
            r"\b(?:self|runtime)[ \t\n]*\.[ \t\n]*publish_verified_unlinked_function"
            r"[ \t\n]*\(",
            ctx.consumer_code,
        )
        if len(verified_publication_calls) != 1:
            ctx.fail(
                "binary-object-consumer-publication",
                f"{ctx.consumer_relative} must enter publish_verified_unlinked_function exactly once after the dedicated ordinary-leaf verifier",
            )

        lowered_scalar_pattern = re.compile(
            r"(?m)^[ \t]*enum[ \t]+LoweredScalar[ \t\n]*\{"
            r"[ \t\n]*Direct[ \t\n]*\([ \t\n]*Instruction[ \t\n]*\)[ \t\n]*,"
            r"[ \t\n]*Constant[ \t\n]*\([ \t\n]*UnlinkedConstant[ \t\n]*\)[ \t\n]*,"
            r"[ \t\n]*AtomString[ \t\n]*\([ \t\n]*UnlinkedConstant[ \t\n]*\)[ \t\n]*,"
            r"[ \t\n]*IntegerAtomString[ \t\n]*\([ \t\n]*u32[ \t\n]*\)"
            r"[ \t\n]*,?[ \t\n]*\}"
        )
        if len(lowered_scalar_pattern.findall(ctx.consumer_code)) != 1:
            ctx.fail(
                "binary-object-consumer-scalar-mapping",
                "LoweredScalar must preserve only direct, primitive-cpool, atom-cpool, and fresh integer-atom value provenance",
            )

        publication_bridge_pattern = re.compile(
            r"\bpub[ \t\n]*\([ \t\n]*crate[ \t\n]*\)[ \t\n]+fn"
            r"[ \t\n]+read_trusted_scalar_script_in_realm[ \t\n]*\([^{};]*\)"
            r"[ \t\n]*->[^{;]+\{",
            re.DOTALL,
        )
        publication_bridge_code, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.consumer_code,
            publication_bridge_pattern,
            "binary-object-consumer-publication",
            "trusted scalar publication bridge",
        )
        expected_publication_bridge_source = '\n        pub(crate) fn read_trusted_scalar_script_in_realm(\n            &self,\n            realm: ContextId,\n            bytes: &[u8],\n        ) -> Result<FunctionBytecodeRef, RuntimeError> {\n            let (value, unary_ops) = decode_trusted_scalar_script(bytes).map_err(map_read_error)?;\n            let (push, constants) = match lower_scalar_value(value)? {\n                LoweredScalar::Direct(push) => (push, Vec::new()),\n                LoweredScalar::Constant(constant) | LoweredScalar::AtomString(constant) => {\n                    (Instruction::PushConst(0), vec![constant])\n                }\n                LoweredScalar::IntegerAtomString(value) => {\n                    (Instruction::PushAtomValueIndex(value), Vec::new())\n                }\n            };\n            let instruction_capacity = unary_ops.len().checked_add(3).ok_or_else(|| {\n                RuntimeError::Engine(Error::internal(\n                    "trusted scalar instruction count overflowed",\n                ))\n            })?;\n            let mut instructions = Vec::new();\n            instructions\n                .try_reserve_exact(instruction_capacity)\n                .map_err(|_| {\n                    RuntimeError::Engine(Error::internal(\n                        "could not allocate trusted scalar instruction draft",\n                    ))\n                })?;\n            instructions.push(push);\n            for operation in unary_ops {\n                instructions.push(match operation {\n                    ScalarUnaryOp::Neg => Instruction::Neg,\n                    ScalarUnaryOp::Plus => Instruction::Plus,\n                    ScalarUnaryOp::Dec => Instruction::Dec,\n                    ScalarUnaryOp::Inc => Instruction::Inc,\n                    ScalarUnaryOp::BitNot => Instruction::BitNot,\n                    ScalarUnaryOp::LogicalNot => Instruction::Not,\n                    ScalarUnaryOp::TypeOf => Instruction::TypeOf,\n                });\n            }\n            instructions.push(Instruction::SetLocal(0));\n            instructions.push(Instruction::Return);\n            let function = UnlinkedFunction::new(\n                instructions,\n                constants,\n                FunctionMetadata {\n                    local_count: 1,\n                    max_stack: 1,\n                    strip_variable_debug: true,\n                    ..FunctionMetadata::default()\n                },\n                Vec::new(),\n                vec![crate::engine::code::function::UnlinkedVariableDefinition::ordinary(None)],\n                Vec::new(),\n            );\n\n            self.publish_unlinked_function(realm, function)\n        }\n    '
        if (
            " ".join(publication_bridge_code.split())
            != " ".join(ctx.rust_code_only(expected_publication_bridge_source).split())
        ):
            ctx.fail(
                "binary-object-consumer-publication",
                f"{ctx.consumer_relative} must publish one checked push, every authenticated unary operation in order, completion, and return before entering the ordinary verifier/publication boundary",
            )

        ordinary_publication_bridge_code, ctx._, ctx._ = ctx.unique_braced_item(
            consumer_production_code,
            re.compile(
                r"\bpub[ \t\n]*\([ \t\n]*crate[ \t\n]*\)[ \t\n]+fn"
                r"[ \t\n]+read_trusted_ordinary_function_in_realm\b[^{};]*\{"
            ),
            "ordinary-leaf-consumer-publication",
            "trusted ordinary-leaf publication bridge",
        )
        ordinary_publication_steps = (
            re.compile(r"\bdecode_trusted_ordinary_leaf[ \t\n]*\("),
            re.compile(r"\bdraft[ \t\n]*\.[ \t\n]*into_parts[ \t\n]*\("),
            re.compile(r"\bUnlinkedFunction[ \t\n]*::[ \t\n]*new[ \t\n]*\("),
            re.compile(
                r"\bbytecode_publish[ \t\n]*::[ \t\n]*verify_unlinked_ordinary_leaf"
                r"[ \t\n]*\("
            ),
            re.compile(
                r"\bself[ \t\n]*\.[ \t\n]*publish_verified_unlinked_function"
                r"[ \t\n]*\("
            ),
            re.compile(r"\bself[ \t\n]*\.[ \t\n]*new_bytecode_closure[ \t\n]*\("),
        )
        ordinary_publication_matches = [
            list(pattern.finditer(ordinary_publication_bridge_code))
            for pattern in ordinary_publication_steps
        ]
        ordinary_publication_offsets = [
            matches[0].start() if len(matches) == 1 else -1
            for matches in ordinary_publication_matches
        ]
        if (
            any(len(matches) != 1 for matches in ordinary_publication_matches)
            or ordinary_publication_offsets != sorted(ordinary_publication_offsets)
            or any(
                ordinary_publication_bridge_code[:offset].count("{")
                - ordinary_publication_bridge_code[:offset].count("}") != 1
                for offset in ordinary_publication_offsets
            )
        ):
            ctx.fail(
                "ordinary-leaf-consumer-publication",
                "the ordinary-leaf bridge must decode and detach before constructing the draft, then run the dedicated verifier before verified publication and closure allocation",
            )
        normalized_ordinary_publication = " ".join(ordinary_publication_bridge_code.split())
        synthetic_publication_fragments = (
            "let original_constant_count = detached_constants.len();",
            "let synthetic_constant_count = detached_code .iter() .filter(",
            "let total_constant_count = original_constant_count .checked_add(synthetic_constant_count)",
            "u32::try_from(total_constant_count)",
            "for constant in detached_constants { constants.push(lower_detached_primitive(constant)?); }",
            "constants .try_reserve_exact(synthetic_constant_count)",
            "for operation in &detached_code {",
            "OrdinaryLeafOp::PushBigIntI32(value) => constants.push(lower_primitive_constant( Value::BigInt(JsBigInt::from(*value)), )?)",
            "OrdinaryLeafOp::PushEmptyString => { constants.push(UnlinkedConstant::atom_string(JsString::from_static(",
            "let mut next_synthetic_index = u32::try_from(original_constant_count)",
            "for operation in detached_code { instructions.push(lower_ordinary_leaf_op(",
            "if next_synthetic_index as usize != total_constant_count",
        )
        synthetic_publication_offsets = [
            normalized_ordinary_publication.find(fragment)
            for fragment in synthetic_publication_fragments
        ]
        if (
            any(offset < 0 for offset in synthetic_publication_offsets)
            or synthetic_publication_offsets != sorted(synthetic_publication_offsets)
            or any(
                normalized_ordinary_publication.count(fragment) != 1
                for fragment in synthetic_publication_fragments
            )
        ):
            ctx.fail(
                "ordinary-leaf-consumer-publication",
                "original constants must publish before code-ordered synthetic BigInts/empty atoms and the matching stable PushConst index pass",
            )

        consumer_runtime_impl_code, ctx._, ctx._ = ctx.unique_braced_item(
            consumer_production_code,
            re.compile(r"(?m)^[ \t]*impl[ \t\n]+Runtime[ \t\n]*\{"),
            "binary-object-consumer-implementation-set",
            "sole Runtime publication implementation",
        )
        consumer_runtime_methods = [
            match.group("name")
            for match in re.finditer(
                r"(?m)^[ \t]*(?:pub(?:[ \t\n]*\([^)]*\))?[ \t\n]+)?fn"
                r"[ \t\n]+(?P<name>[A-Za-z_][A-Za-z0-9_]*)",
                consumer_runtime_impl_code,
            )
            if consumer_runtime_impl_code[:match.start()].count("{")
            - consumer_runtime_impl_code[:match.start()].count("}") == 1
        ]
        if consumer_runtime_methods != [
            "read_trusted_ordinary_function_in_realm",
            "read_trusted_scalar_script_in_realm",
        ]:
            ctx.fail(
                "binary-object-consumer-implementation-set",
                f"{ctx.consumer_relative} Runtime implementation must contain only the reviewed ordinary-leaf and scalar publication bridges; found {consumer_runtime_methods}",
            )

        scalar_lowering_pattern = re.compile(
            r"\bfn[ \t\n]+lower_scalar_value[ \t\n]*\([^{};]*\)"
            r"[ \t\n]*->[^{;]+\{",
            re.DOTALL,
        )
        scalar_lowering_code, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.consumer_code,
            scalar_lowering_pattern,
            "binary-object-consumer-scalar-mapping",
            "lower_scalar_value function",
        )
        bigint_lowering_pattern = re.compile(
            r"\bfn[ \t\n]+lower_bigint_constant[ \t\n]*\("
            r"[ \t\n]*bytes[ \t\n]*:[ \t\n]*&[ \t\n]*\[[ \t\n]*u8"
            r"[ \t\n]*\][ \t\n]*,?[ \t\n]*\)[ \t\n]*->[^{;]+\{",
            re.DOTALL,
        )
        bigint_lowering_code, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.consumer_code,
            bigint_lowering_pattern,
            "binary-object-consumer-bigint",
            "lower_bigint_constant function",
        )
        bigint_decoder_pattern = re.compile(
            r"\bfn[ \t\n]+decode_bigint_constant[ \t\n]*\("
            r"[ \t\n]*bytes[ \t\n]*:[ \t\n]*&[ \t\n]*\[[ \t\n]*u8"
            r"[ \t\n]*\][ \t\n]*,?[ \t\n]*\)[ \t\n]*->[^{;]+\{",
            re.DOTALL,
        )
        bigint_decoder_code, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.consumer_code,
            bigint_decoder_pattern,
            "binary-object-consumer-bigint",
            "decode_bigint_constant function",
        )
        scalar_constant_pattern = re.compile(
            r"\bfn[ \t\n]+lower_primitive_constant[ \t\n]*\([^{};]*\)"
            r"[ \t\n]*->[^{;]+\{",
            re.DOTALL,
        )
        scalar_constant_code, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.consumer_code,
            scalar_constant_pattern,
            "binary-object-consumer-scalar-mapping",
            "lower_primitive_constant function",
        )
        scalar_string_pattern = re.compile(
            r"\bfn[ \t\n]+lower_scalar_string[ \t\n]*\([^{};]*\)"
            r"[ \t\n]*->[^{;]+\{",
            re.DOTALL,
        )
        scalar_string_code, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.consumer_code,
            scalar_string_pattern,
            "binary-object-consumer-string",
            "lower_scalar_string function",
        )
        expected_scalar_lowering_source = '\n        fn lower_scalar_value(value: ScalarValueDraft) -> Result<LoweredScalar, RuntimeError> {\n            match value {\n                ScalarValueDraft::Undefined => Ok(LoweredScalar::Direct(Instruction::Undefined)),\n                ScalarValueDraft::Null => Ok(LoweredScalar::Direct(Instruction::Null)),\n                ScalarValueDraft::Bool(false) => Ok(LoweredScalar::Direct(Instruction::PushFalse)),\n                ScalarValueDraft::Bool(true) => Ok(LoweredScalar::Direct(Instruction::PushTrue)),\n                ScalarValueDraft::Int(value) => Ok(LoweredScalar::Direct(Instruction::PushI32(value))),\n                ScalarValueDraft::Float64Bits(bits) => {\n                    lower_primitive_constant(Value::Float(f64::from_bits(bits)))\n                        .map(LoweredScalar::Constant)\n                }\n                ScalarValueDraft::BigIntI32(value) => {\n                    lower_primitive_constant(Value::BigInt(JsBigInt::from(value)))\n                        .map(LoweredScalar::Constant)\n                }\n                ScalarValueDraft::BigIntBytes(bytes) => {\n                    lower_bigint_constant(&bytes).map(LoweredScalar::Constant)\n                }\n                ScalarValueDraft::EmptyString => Ok(LoweredScalar::AtomString(\n                    UnlinkedConstant::atom_string(JsString::from_static("")),\n                )),\n                ScalarValueDraft::ConstantString(value) => lower_scalar_string(value)\n                    .and_then(|value| lower_primitive_constant(Value::String(value)))\n                    .map(LoweredScalar::Constant),\n                ScalarValueDraft::AtomString(value) => Ok(LoweredScalar::AtomString(\n                    UnlinkedConstant::atom_string(lower_scalar_string(value)?),\n                )),\n                ScalarValueDraft::IntegerAtomString(value) => Ok(LoweredScalar::IntegerAtomString(value)),\n            }\n        }\n    '
        expected_scalar_string_source = '\n        fn lower_scalar_string(value: ScalarStringDraft) -> Result<JsString, RuntimeError> {\n            JsString::try_from_utf16(value.into_units()).map_err(|error| RuntimeError::Engine(error.into()))\n        }\n    '
        expected_bigint_lowering_source = '\n        fn lower_bigint_constant(bytes: &[u8]) -> Result<UnlinkedConstant, RuntimeError> {\n            lower_primitive_constant(Value::BigInt(decode_bigint_constant(bytes)?))\n        }\n    '
        expected_bigint_decoder_source = '\n        fn decode_bigint_constant(bytes: &[u8]) -> Result<JsBigInt, RuntimeError> {\n            let (value, consumed) =\n                JsBigInt::decode_bc5_signed_le(bytes, bytes.len(), bytes.len(), true)\n            .map_err(|error| {\n                RuntimeError::Engine(Error::internal(format!(\n                    "trusted binary-object draft contained invalid canonical BigInt bytes: {error:?}"\n                )))\n            })?;\n            if consumed != bytes.len() {\n                return Err(RuntimeError::Engine(Error::internal(\n                    "trusted scalar BigInt draft was not consumed exactly",\n                )));\n            }\n            Ok(value)\n        }\n    '
        expected_scalar_constant_source = '\n        fn lower_primitive_constant(value: Value) -> Result<UnlinkedConstant, RuntimeError> {\n            UnlinkedConstant::primitive(value).map_err(|error| {\n                RuntimeError::Engine(Error::internal(format!(\n                    "trusted binary-object draft produced an invalid primitive constant: {error}"\n                )))\n            })\n        }\n    '
        if (
            " ".join(scalar_lowering_code.split())
            != " ".join(ctx.rust_code_only(expected_scalar_lowering_source).split())
            or re.findall(
                r"\bScalarValueDraft[ \t\n]*::[ \t\n]*([A-Za-z_][A-Za-z0-9_]*)",
                ctx.consumer_code.split("#[cfg(test)]", 1)[0],
            ) != [
                "Undefined",
                "Null",
                "Bool",
                "Bool",
                "Int",
                "Float64Bits",
                "BigIntI32",
                "BigIntBytes",
                "EmptyString",
                "ConstantString",
                "AtomString",
                "IntegerAtomString",
            ]
            or " ".join(scalar_string_code.split())
            != " ".join(ctx.rust_code_only(expected_scalar_string_source).split())
        ):
            ctx.fail(
                "binary-object-consumer-scalar-mapping",
                f"{ctx.consumer_relative} must retain the reviewed primitive, BigInt, and UTF-16 String provenance mapping",
            )
        if (
            " ".join(scalar_constant_code.split())
            != " ".join(ctx.rust_code_only(expected_scalar_constant_source).split())
            or " ".join(bigint_lowering_code.split())
            != " ".join(ctx.rust_code_only(expected_bigint_lowering_source).split())
            or " ".join(bigint_decoder_code.split())
            != " ".join(ctx.rust_code_only(expected_bigint_decoder_source).split())
            or len(re.findall(r"\blower_bigint_constant\b", ctx.consumer_code)) != 2
            or len(re.findall(r"\bdecode_bigint_constant\b", ctx.consumer_code)) != 3
            or len(re.findall(r"\bJsBigInt[ \t\n]*::[ \t\n]*decode_bc5_signed_le\b", ctx.consumer_code)) != 1
        ):
            ctx.fail(
                "binary-object-consumer-bigint",
                f"{ctx.consumer_relative} must decode BigIntBytes exactly once through the unique canonical BigInt helper and lower all primitive pushes through reviewed helpers",
            )

        ordinary_detached_lowering, ctx._, ctx._ = ctx.unique_braced_item(
            consumer_production_code,
            re.compile(r"\bfn[ \t\n]+lower_detached_primitive\b[^{};]*\{"),
            "ordinary-leaf-consumer-lowering",
            "detached primitive lowering",
        )
        ordinary_instruction_lowering, ctx._, ctx._ = ctx.unique_braced_item(
            consumer_production_code,
            re.compile(r"\bfn[ \t\n]+lower_ordinary_leaf_op\b[^{};]*\{"),
            "ordinary-leaf-consumer-lowering",
            "typed instruction lowering",
        )
        ctx.require_normalized_code_sha256(
            "ordinary-leaf-consumer-lowering",
            "lower_ordinary_leaf_op must remain one alias-free exhaustive typed publisher match",
            ordinary_instruction_lowering,
            "0fe1f20d09e3441228acc24c6f93c88693c858afe5f7b8d61662eaa99bc14aaa",
        )
        detached_variants = re.findall(
            r"\bDetachedPrimitive[ \t\n]*::[ \t\n]*(\w+)",
            ordinary_detached_lowering,
        )
        normalized_detached_lowering = " ".join(ordinary_detached_lowering.split())
        if detached_variants != [
            "Undefined", "Null", "Bool", "Int", "Float64Bits", "String",
            "BigIntSignedLeCanonical",
        ] or any(
            normalized_detached_lowering.count(fragment) != 1
            for fragment in (
                "DetachedPrimitive::Float64Bits(bits) => Value::Float(f64::from_bits(bits))",
                "DetachedPrimitive::String(units) => Value::String( JsString::try_from_utf16(units.into_vec())",
                "DetachedPrimitive::BigIntSignedLeCanonical(bytes) => { Value::BigInt(decode_bigint_constant(&bytes)?) }",
                "lower_primitive_constant(value)",
            )
        ):
            ctx.fail(
                "ordinary-leaf-consumer-lowering",
                "detached primitives must retain bit-preserving Float64, canonical BigInt, UTF-16 String, and primitive publication",
            )
        published_variants = re.findall(
            r"\bOrdinaryLeafOp[ \t\n]*::[ \t\n]*(\w+)",
            ordinary_instruction_lowering,
        )
        expected_published_variants = '\n        Nop Object ToObject ToPropKey PushThis PushI32 PushConst PushUndefined PushNull PushBool PushBool PushBigIntI32\n        PushEmptyString Stack Unary PostDec PostInc GetLocal PutLocal SetLocal\n        GetArgument PutArgument SetArgument Binary Predicate IfFalse IfTrue Goto\n        Call TailCall Construct CallMethod TailCallMethod ArrayFrom Apply Return\n        ReturnUndefined Throw ThrowReadOnly\n    '.split()
        found_publisher_arms = ctx.rustfmt_match_arms(
            ordinary_instruction_lowering, "OrdinaryLeafOp::"
        )
        publisher_families = {
            "Stack": ("Drop", "Nip", "Dup", "Dup1", "Dup3", "Insert2", "Insert3", "Insert4", "Perm3", "Perm4", "Perm5", "Swap", "Rot4Left"),
            "Unary": ("Neg", "Plus", "Dec", "Inc", "BitNot", "LogicalNot", "TypeOf"),
            "Binary": ("Add", "Sub", "Mul", "Div", "Mod", "Pow", "Shl", "Sar", "Shr", "LessThan", "LessThanOrEqual", "GreaterThan", "GreaterThanOrEqual", "Equal", "NotEqual", "StrictEqual", "StrictNotEqual", "BitAnd", "BitXor", "BitOr"),
            "Predicate": ("IsUndefinedOrNull", "IsUndefined", "IsNull", "TypeOfIsUndefined", "TypeOfIsFunction"),
        }
        engine_renames = {
            "LogicalNot": "Not", "LessThan": "Lt", "LessThanOrEqual": "Lte",
            "GreaterThan": "Gt", "GreaterThanOrEqual": "Gte", "Equal": "Eq",
            "NotEqual": "Neq", "StrictEqual": "StrictEq", "StrictNotEqual": "StrictNeq",
        }
        publisher_arm_map = dict(found_publisher_arms)
        for family, variants in publisher_families.items():
            expected_body = "match operation { " + " ".join(
                f"OrdinaryLeaf{family}Op::{variant} => "
                f"Instruction::{engine_renames.get(variant, variant)},"
                for variant in variants
            ) + " }"
            if publisher_arm_map.get(f"OrdinaryLeafOp::{family}(operation)") != expected_body:
                ctx.fail(
                    "ordinary-leaf-consumer-lowering",
                    f"the {family.lower()} publisher mapping drifted",
                )
        publisher_direct_rows = '\nOrdinaryLeafOp::Nop @ Instruction::Nop\nOrdinaryLeafOp::Object @ Instruction::Object\nOrdinaryLeafOp::ToObject @ Instruction::ToObject\nOrdinaryLeafOp::ToPropKey @ Instruction::ToPropKey\nOrdinaryLeafOp::PushThis @ Instruction::PushThis\nOrdinaryLeafOp::PushI32(value) @ Instruction::PushI32(value)\nOrdinaryLeafOp::PushConst(index) @ Instruction::PushConst(index)\nOrdinaryLeafOp::PushUndefined @ Instruction::Undefined\nOrdinaryLeafOp::PushNull @ Instruction::Null\nOrdinaryLeafOp::PushBool(false) @ Instruction::PushFalse\nOrdinaryLeafOp::PushBool(true) @ Instruction::PushTrue\nOrdinaryLeafOp::PostDec @ Instruction::PostDec\nOrdinaryLeafOp::PostInc @ Instruction::PostInc\nOrdinaryLeafOp::GetLocal(index) @ Instruction::GetLocal(index)\nOrdinaryLeafOp::PutLocal(index) @ Instruction::PutLocal(index)\nOrdinaryLeafOp::SetLocal(index) @ Instruction::SetLocal(index)\nOrdinaryLeafOp::GetArgument(index) @ Instruction::GetArg(index)\nOrdinaryLeafOp::PutArgument(index) @ Instruction::PutArg(index)\nOrdinaryLeafOp::SetArgument(index) @ Instruction::SetArg(index)\nOrdinaryLeafOp::IfFalse(target) @ Instruction::IfFalse(target)\nOrdinaryLeafOp::IfTrue(target) @ Instruction::IfTrue(target)\nOrdinaryLeafOp::Goto(target) @ Instruction::Goto(target)\nOrdinaryLeafOp::Call(argument_count) @ Instruction::Call(argument_count)\nOrdinaryLeafOp::TailCall(argument_count) @ Instruction::TailCall(argument_count)\nOrdinaryLeafOp::Construct(argument_count) @ Instruction::Construct(argument_count)\nOrdinaryLeafOp::CallMethod(argument_count) @ Instruction::CallMethod(argument_count)\nOrdinaryLeafOp::TailCallMethod(argument_count) @ { Instruction::TailCallMethod(argument_count) }\nOrdinaryLeafOp::ArrayFrom(element_count) @ Instruction::ArrayFrom(element_count)\nOrdinaryLeafOp::Apply(kind) @ Instruction::Apply(match kind { OrdinaryLeafApplyKind::Call => ApplyKind::Call, OrdinaryLeafApplyKind::Construct => ApplyKind::Construct, })\nOrdinaryLeafOp::Return @ Instruction::Return\nOrdinaryLeafOp::ReturnUndefined @ Instruction::ReturnUndefined\nOrdinaryLeafOp::Throw @ Instruction::Throw\n'.strip().splitlines()
        excluded_publisher_arms = {
            *(f"OrdinaryLeafOp::{family}(operation)" for family in publisher_families),
            "OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString",
            "OrdinaryLeafOp::ThrowReadOnly(_)",
        }
        found_publisher_direct = [
            arm for arm in found_publisher_arms if arm[0] not in excluded_publisher_arms
        ]
        expected_publisher_direct = [
            tuple(row.split(" @ ", 1)) for row in publisher_direct_rows
        ]
        normalized_instruction_lowering = " ".join(ordinary_instruction_lowering.split())
        synthetic_index_arm, ctx._, ctx._ = ctx.unique_braced_item(
            ordinary_instruction_lowering,
            re.compile(
                r"OrdinaryLeafOp[ \t\n]*::[ \t\n]*PushBigIntI32[ \t\n]*\([^)]*\)"
                r"[ \t\n]*\|[ \t\n]*OrdinaryLeafOp[ \t\n]*::[ \t\n]*PushEmptyString"
                r"[ \t\n]*=>[ \t\n]*\{"
            ),
            "ordinary-leaf-consumer-lowering",
            "synthetic constant index arm",
        )
        normalized_synthetic_index = " ".join(synthetic_index_arm.split())
        throw_read_only_publisher_arm, ctx._, ctx._ = ctx.unique_braced_item(
            ordinary_instruction_lowering,
            re.compile(
                r"OrdinaryLeafOp[ \t\n]*::[ \t\n]*ThrowReadOnly"
                r"[ \t\n]*\([^)]*\)[ \t\n]*=>[ \t\n]*\{"
            ),
            "ordinary-leaf-consumer-lowering",
            "ThrowReadOnly synthetic constant index arm",
        )
        normalized_throw_read_only_publisher = " ".join(
            throw_read_only_publisher_arm.split()
        )
        synthetic_index_fragments = (
            "let index = *next_synthetic_index;",
            "*next_synthetic_index = next_synthetic_index.checked_add(1)",
            "Instruction::PushConst(index)",
        )
        synthetic_index_offsets = [
            normalized_synthetic_index.find(fragment) for fragment in synthetic_index_fragments
        ]
        if (
            len(found_publisher_arms) != 38
            or found_publisher_direct != expected_publisher_direct
            or published_variants != expected_published_variants
            or any(
            normalized_instruction_lowering.count(fragment) != 1
            for fragment in (
                "OrdinaryLeafOp::PushBigIntI32(_) | OrdinaryLeafOp::PushEmptyString =>",
            )
            )
            or normalized_instruction_lowering.count("Instruction::PushConst(index)") != 2
            or normalized_throw_read_only_publisher != (
                "OrdinaryLeafOp::ThrowReadOnly(_) => { "
                "let index = *next_synthetic_index; "
                "*next_synthetic_index = next_synthetic_index.checked_add(1).ok_or_else(|| { "
                "RuntimeError::Engine(Error::internal( , )) })?; "
                "Instruction::ThrowReadOnly(index) }"
            )
            or (
                any(offset < 0 for offset in synthetic_index_offsets)
                or synthetic_index_offsets != sorted(synthetic_index_offsets)
                or any(
                    normalized_synthetic_index.count(fragment) != 1
                    for fragment in synthetic_index_fragments
                )
            )
        ):
            ctx.fail(
                "ordinary-leaf-consumer-lowering",
                "ordinary operations must retain their complete typed publisher mapping, one-for-one Nop/Object/ToObject/ToPropKey/PushThis publication, and stable synthetic indices",
            )

        ordinary_consumer_seals = (
            ("typed-verifier error classification", "map_ordinary_leaf_verification_error", "43d7c7fd3f77c74c9a4ba88cfa14f7d7e8394f7be1100214e8b08a424a8b59ee"),
            ("archive error classification", "map_ordinary_leaf_read_error", "5bb4c99694272a03044d353e48e3f3848ade1e1bfbad2b87a552e1bafa45ba74"),
        )
        for ctx.description, ctx.function_name, ctx.expected_hash in ordinary_consumer_seals:
            ctx.item_code, ctx._, ctx._ = ctx.unique_braced_item(
                consumer_production_code,
                re.compile(
                    rf"(?m)^[ \t]*(?:const[ \t\n]+)?fn[ \t\n]+"
                    rf"{ctx.function_name}\b[^{{}};]*\{{"
                ),
                "ordinary-leaf-consumer-lowering",
                ctx.description,
            )
            if ctx.item_code and ctx.normalized_code_sha256(ctx.item_code) != ctx.expected_hash:
                ctx.fail(
                    "ordinary-leaf-consumer-lowering",
                    f"ordinary-leaf {ctx.description} drifted from its reviewed normalized implementation",
                )

        consumer_raw_archive_dependency = re.search(
            r"\b(?:BytecodeImage|ImageCode|ImageInstructionSpan|ImageRelocation|"
            r"NativeCodePlan|NativeOperands|NativeInstruction|FunctionId|ImageAtom|"
            r"PinnedAtomId)\b|\.[ \t\n]*(?:as_bytes|atom_relocations)[ \t\n]*\(",
            consumer_production_code,
        )
        if consumer_raw_archive_dependency is not None:
            ctx.fail(
                "ordinary-leaf-consumer-import",
                "the runtime publisher may consume only owned ordinary-leaf DTOs, never archival images, raw code, native-plan operands, identities, or sidecars; found "
                + ctx.location(
                    ctx.consumer_relative,
                    ctx.consumer_source,
                    consumer_raw_archive_dependency.start(),
                ),
            )

        consumer_special_case_pattern = re.compile(
            r"(?i:\btest262\b|\bfixture(?:_[A-Za-z0-9_]+)?\b|"
            r"\b(?:source|input|bytes)_[A-Za-z0-9_]*(?:hash|digest|sha_?(?:1|256|512))\b)|"
            r"\bbytes[ \t\n]*(?:\.[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\([^;\n]*\))*"
            r"\.[ \t\n]*(?:contains|starts_with|ends_with|windows)[ \t\n]*\(",
        )
        consumer_special_case = consumer_special_case_pattern.search(
            ctx.consumer_source.split("#[cfg(test)]", 1)[0]
        )
        if consumer_special_case is not None:
            ctx.fail(
                "ordinary-leaf-consumer-special-casing",
                "the ordinary-leaf publisher must not dispatch on Test262, fixture, digest, or exact input-byte identity; found "
                + ctx.location(
                    ctx.consumer_relative,
                    ctx.consumer_source,
                    consumer_special_case.start(),
                ),
            )

        for ctx.match in re.finditer(r"\b(?:r#)?number[ \t\n]*\(", ctx.consumer_code):
            ctx.fail(
                "binary-object-consumer-float64",
                f"{ctx.consumer_relative} must not normalize an authenticated Float64 tag through Value::number or an alias; found "
                + ctx.location(ctx.consumer_relative, ctx.consumer_source, ctx.match.start()),
            )

        if (
            len(re.findall(r"\bUnlinkedConstant[ \t\n]*::[ \t\n]*atom_string[ \t\n]*\(", ctx.consumer_code)) != 3
            or len(re.findall(r"\batom_string\b", ctx.consumer_code)) != 3
        ):
            ctx.fail(
                "binary-object-consumer-atom-string",
                "only scalar direct/ordinary atom Strings and the ordinary direct-empty synthetic constant may use the atom-string publication marker",
            )

        consumer_forbidden_patterns = (
            (
                "binary-object-consumer-bigint-eager-negation",
                re.compile(
                    r"\b(?:std|core)[ \t\n]*::[ \t\n]*ops[ \t\n]*::"
                    r"[ \t\n]*Neg\b|"
                    r"\bJsBigInt[ \t\n]*::[ \t\n]*(?:neg|negate)\b|"
                    r"\b(?:value|bigint)[ \t\n]*\.[ \t\n]*"
                    r"(?:neg|negate|checked_neg|wrapping_neg)[ \t\n]*\(|"
                    r"-[ \t\n]*(?:value|bigint)\b"
                ),
                "eager BigInt negation; authenticated unary negation must remain Instruction::Neg execution semantics",
            ),
            (
                "binary-object-consumer-alternate-entrypoint",
                re.compile(
                    r"\b(?:(?:self|runtime)[ \t\n]*\.[ \t\n]*|Runtime[ \t\n]*::[ \t\n]*)"
                    r"(?!(?:publish_unlinked_function|publish_verified_unlinked_function)\b)"
                    r"(?:compile|publish)_[A-Za-z_][A-Za-z0-9_]*[ \t\n]*\("
                ),
                "an alternate runtime compilation or publication entry point",
            ),
            (
                "binary-object-consumer-heap-type",
                re.compile(
                    r"\b(?:FunctionBytecodeData|BytecodeConstant|FunctionBytecodeId|ObjectId|"
                    r"RawValue|Heap|HeapObject|ObjectRef)\b"
                ),
                "a direct runtime heap representation type",
            ),
            (
                "binary-object-consumer-root-forge",
                re.compile(
                    r"\bFunctionBytecodeRef[ \t\n]*\{|\bFunctionBytecodeRef[ \t\n]*::"
                    r"[ \t\n]*(?:from_owned_handle|from_borrowed_handle)\b"
                ),
                "a direct FunctionBytecodeRef constructor",
            ),
            (
                "binary-object-consumer-atom-interning",
                re.compile(r"\bintern(?:_[A-Za-z0-9_]+)?\b"),
                "an atom-interning identifier",
            ),
            (
                "binary-object-consumer-vm-dependency",
                re.compile(
                    r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?vm\b|"
                    r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?vm\b"
                ),
                "the VM module",
            ),
            (
                "binary-object-consumer-compiler-dependency",
                re.compile(
                    r"\bcrate[ \t\n]*::[ \t\n]*(?:r#)?compiler\b|"
                    r"\b(?:super[ \t\n]*::[ \t\n]*)+(?:r#)?compiler\b"
                ),
                "the source compiler",
            ),
            (
                "binary-object-consumer-unsafe",
                re.compile(r"\bunsafe\b|\bNonNull\b|\*[ \t\n]*(?:const|mut)\b"),
                "unsafe code or native pointers",
            ),
        )
        for ctx.code_name, ctx.pattern, ctx.description in consumer_forbidden_patterns:
            for ctx.match in ctx.pattern.finditer(ctx.consumer_code):
                ctx.fail(
                    ctx.code_name,
                    f"{ctx.consumer_relative} must not use {ctx.description}; found "
                    + ctx.location(ctx.consumer_relative, ctx.consumer_source, ctx.match.start()),
                )

    ctx.bytecode_publish_relative = "src/engine/code/bytecode_publish.rs"

    bytecode_publish_source = ctx.read_source(ctx.bytecode_publish_relative)

    bytecode_publish_code = ctx.rust_code_only(bytecode_publish_source)

    if (
        len(
            re.findall(
                r"(?m)^[ \t]*TrustedOrdinaryLeaf[ \t]*,[ \t]*$",
                bytecode_publish_code,
            )
        )
        != 1
        or len(re.findall(r"\bTrustedOrdinaryLeaf\b", bytecode_publish_code)) != 8
    ):
        ctx.fail(
            "ordinary-leaf-verifier-role",
            "RootPublication must declare one distinct TrustedOrdinaryLeaf role with the reviewed generic-verifier use set",
        )

    ordinary_verifier_entry_code, ctx._, ctx._ = ctx.unique_braced_item(
        bytecode_publish_code,
        re.compile(
            r"(?m)^[ \t]*pub[ \t\n]*\([ \t\n]*in[ \t\n]+crate[ \t\n]*::"
            r"[ \t\n]*engine[ \t\n]*::[ \t\n]*code[ \t\n]*\)[ \t\n]+fn[ \t\n]+"
            r"verify_unlinked_ordinary_leaf\b[^{};]*\{"
        ),
        "ordinary-leaf-verifier-entrypoint",
        "dedicated ordinary-leaf verifier entry point",
    )

    expected_ordinary_verifier_entry = deepcopy(evidence.EXPECTED_ORDINARY_VERIFIER_ENTRY)

    if " ".join(ordinary_verifier_entry_code.split()) != " ".join(
        expected_ordinary_verifier_entry.split()
    ):
        ctx.fail(
            "ordinary-leaf-verifier-entrypoint",
            "verify_unlinked_ordinary_leaf must enter the generic verifier through only the distinct TrustedOrdinaryLeaf role",
        )

    ordinary_verifier_arm_pattern = re.compile(
        r"RootPublication[ \t\n]*::[ \t\n]*TrustedOrdinaryLeaf"
        r"[ \t\n]*=>[ \t\n]*\{"
    )

    ordinary_verifier_arms = []

    for arm_match in ordinary_verifier_arm_pattern.finditer(bytecode_publish_code):
        arm_code, ctx._, ctx._ = ctx.braced_item_from_match(
            bytecode_publish_code,
            arm_match,
            "ordinary-leaf-verifier-role",
            "TrustedOrdinaryLeaf verifier arm",
        )
        ordinary_verifier_arms.append(arm_code)

    expected_ordinary_closure_arm = 'RootPublication::TrustedOrdinaryLeaf => { return Err(RuntimeError::Engine(Error::internal("trusted ordinary leaf retained a closure descriptor", ))); }'

    if (
        len(ordinary_verifier_arms) != 2
        or ctx.normalized_code_sha256(ordinary_verifier_arms[0])
            != "78509ea6396da4c20a0ee2b34304ac0224552e20e3b89e4346ab8173810c8c7e"
        or " ".join(ordinary_verifier_arms[1].split())
        != " ".join(ctx.rust_code_only(expected_ordinary_closure_arm).split())
    ):
        ctx.fail(
            "ordinary-leaf-verifier-role",
            "the dedicated verifier must retain its exact fail-closed metadata/debug/parameter/local/primitive-only checks and closure rejection; "
            f"found {len(ordinary_verifier_arms)} role arms",
        )

    function_relative = "src/engine/code/function.rs"

    function_source = ctx.read_source(function_relative)

    function_code = ctx.rust_code_only(function_source)

    plain_primitive_code, ctx._, ctx._ = ctx.unique_braced_item(
        function_code,
        re.compile(
            r"(?m)^[ \t]*pub[ \t\n]+const"
            r"[ \t\n]+fn[ \t\n]+is_plain_primitive\b[^{};]*\{"
        ),
        "ordinary-leaf-plain-primitive",
        "UnlinkedConstant plain-primitive discriminator",
    )

    expected_plain_primitive = "pub const fn is_plain_primitive(&self) -> bool { matches!(self.0, UnlinkedConstantKind::Primitive(_)) }"

    if " ".join(plain_primitive_code.split()) != " ".join(expected_plain_primitive.split()):
        ctx.fail(
            "ordinary-leaf-plain-primitive",
            "ordinary-leaf verification must classify only UnlinkedConstantKind::Primitive as a plain primitive",
        )

    empty_atom_code, ctx._, ctx._ = ctx.unique_braced_item(
        function_code,
        re.compile(
            r"(?m)^[ \t]*pub[ \t\n]+fn"
            r"[ \t\n]+is_empty_atom_string\b[^{};]*\{"
        ),
        "ordinary-leaf-plain-primitive",
        "exact empty atom-String discriminator",
    )

    expected_empty_atom = "pub fn is_empty_atom_string(&self) -> bool { matches!( &self.0, UnlinkedConstantKind::AtomString(PrimitiveValue::String(value)) if value.is_empty() ) }"

    if " ".join(empty_atom_code.split()) != expected_empty_atom:
        ctx.fail(
            "ordinary-leaf-plain-primitive",
            "ordinary-leaf verification may admit only the exact empty atom String beside plain primitives",
        )

    context_relative = "src/engine/api/context/bytecode.rs"

    context_source = ctx.read_source(context_relative)

    ctx.context_code = ctx.rust_code_only(context_source)

    ordinary_public_api_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.context_code,
        re.compile(
            r"(?m)^[ \t]*pub[ \t\n]+fn[ \t\n]+read_trusted_ordinary_function"
            r"\b[^{};]*\{"
        ),
        "ordinary-leaf-public-api",
        "Context ordinary-leaf public API",
    )

    expected_ordinary_public_api = deepcopy(evidence.EXPECTED_ORDINARY_PUBLIC_API)

    if " ".join(ordinary_public_api_code.split()) != " ".join(
        expected_ordinary_public_api.split()
    ):
        ctx.fail(
            "ordinary-leaf-public-api",
            "Context::read_trusted_ordinary_function must retain its exact selector, realm bridge, and trusted-read error finishing flow",
        )

    trusted_read_finish_code, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.context_code,
        re.compile(
            r"(?m)^[ \t]*fn[ \t\n]+finish_trusted_bytecode_read\b[^{};]*\{"
        ),
        "ordinary-leaf-public-api",
        "shared trusted-bytecode read finisher",
    )

    if trusted_read_finish_code and ctx.normalized_code_sha256(
        trusted_read_finish_code
    ) != "398f8677cddce39a30934cca10dfcde5fef30279c69f56e1f83f937aee7f745e":
        ctx.fail(
            "ordinary-leaf-public-api",
            "trusted bytecode reads must convert only JavaScript-visible errors into pending exceptions and preserve Unsupported/Internal directly",
        )

    bytecode_source = ctx.read_source("src/engine/code/bytecode.rs")

    ctx.bytecode_code = ctx.rust_code_only(bytecode_source)

    bytecode_production_code = ctx.bytecode_code.split('#[cfg(test)]\nmod tests', 1)[0]

    ctx.vm_code = ctx.rust_code_only(ctx.read_source("src/engine/vm/mod.rs"))

    value_code = ctx.rust_code_only(ctx.read_source("src/engine/value/primitive.rs"))

    atom_code = ctx.rust_code_only(ctx.read_source("src/engine/atom/mod.rs"))

    engine_string_fragments = (
        (ctx.bytecode_code, "PushAtomValueIndex(u32),"),
        (ctx.bytecode_code, "Self::PushI32(_) | Self::PushAtomValueIndex(_) | Self::PushConst(_)"),
        (ctx.bytecode_code, "Instruction::PushAtomValueIndex(index) if *index > crate::engine::atom::ATOM_MAX_INT"),
        (ctx.vm_code, "Instruction::PushAtomValueIndex(value) => self.stack.push(Value::String( crate::engine::value::JsString::from_fresh_decimal_u32(*value), ))"),
        (atom_code, "AtomSpelling::Integer(value) => Ok(JsString::from_fresh_decimal_u32(value))"),
        (value_code, "pub fn from_fresh_decimal_u32(mut value: u32) -> Self"),
        (value_code, "digits[start] = b'0' + (value % 10) as u8;"),
        (value_code, "Self(Rc::new(StringRepr::Latin1( digits[start..].to_vec().into_boxed_slice(), )))"),
    )

    if any(" ".join(code.split()).count(fragment) != 1 for code, fragment in engine_string_fragments):
        ctx.fail(
            "scalar-string-engine-path",
            "tagged integer atoms must verify within JS_ATOM_MAX_INT and execute through one fresh narrow String instruction",
        )

    normalized_bytecode = " ".join(ctx.bytecode_code.split())

    if normalized_bytecode.count("| Self::ReturnUndefined | Self::ThrowRedeclaration(_)" ) != 1:
        ctx.fail(
            "ordinary-leaf-engine-semantics",
            "ReturnUndefined must remain a zero-pop, zero-push terminal instruction",
        )

    instruction_code, ctx._, ctx._ = ctx.unique_braced_item(
        bytecode_production_code,
        re.compile(r"\bpub[ \t\n]+enum[ \t\n]+Instruction[ \t\n]*\{"),
        "stage3c-instruction-shape",
        "engine Instruction enum",
    )

    tail_instruction_payloads = {
        name: [
            " ".join(payload.split())
            for payload in re.findall(
                rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", instruction_code
            )
        ]
        for name in ("TailCall", "TailCallMethod")
    }

    if tail_instruction_payloads != {"TailCall": ["u16"], "TailCallMethod": ["u16"]}:
        ctx.fail(
            "stage3c-instruction-shape",
            "Instruction must retain distinct TailCall(u16) and TailCallMethod(u16) terminal payloads; "
            f"found {tail_instruction_payloads}",
        )

    if (
        ctx.enum_variant_names(instruction_code).count("Throw") != 1
        or re.search(r"\bThrow[ \t\n]*[({]", instruction_code)
    ):
        ctx.fail(
            "stage3d-instruction-shape",
            "Instruction must retain exactly one operand-free Throw completion",
        )

    predicate_requirements = deepcopy(evidence.PREDICATE_REQUIREMENTS)

    for instruction, (required, uses_html_dda) in predicate_requirements.items():
        ctx.arm, ctx._, ctx._ = ctx.unique_braced_item(
            ctx.vm_code,
            re.compile(rf"\bInstruction[ \t\n]*::[ \t\n]*{instruction}[ \t\n]*=>[ \t\n]*\{{"),
            "ordinary-leaf-engine-semantics",
            f"VM {instruction} arm",
        )
        normalized_arm = " ".join(ctx.arm.split())
        if (
            normalized_arm.count("let value = self.pop()?;") != 1
            or normalized_arm.count(required) != 1
            or ("host.is_html_dda" in normalized_arm) != uses_html_dda
        ):
            ctx.fail(
                "ordinary-leaf-engine-semantics",
                f"VM {instruction} must retain its exact QuickJS tag/HTMLDDA predicate",
            )

    return_undefined_arm, ctx._, ctx._ = ctx.unique_braced_item(
        ctx.vm_code,
        re.compile(r"\bInstruction[ \t\n]*::[ \t\n]*ReturnUndefined[ \t\n]*=>[ \t\n]*\{"),
        "ordinary-leaf-engine-semantics",
        "VM ReturnUndefined arm",
    )

    if " ".join(return_undefined_arm.split()).count(
        "return Ok(Some(Completion::Return(Value::Undefined)));"
    ) != 1:
        ctx.fail(
            "ordinary-leaf-engine-semantics",
            "ReturnUndefined must complete directly with undefined without reading the operand stack",
        )
