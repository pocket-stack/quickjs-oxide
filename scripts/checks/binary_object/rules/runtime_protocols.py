"""Runtime protocols checks, in the ordered boundary scan."""
from __future__ import annotations

import re
from copy import deepcopy

from ..evidence import runtime_protocols as evidence


def check(ctx):
    if ctx.self_test_marker_authorized:
        return

    stage3b_sources = {
        "src/engine/heap/runtime/mod.rs": ctx.runtime_code,
        "src/engine/vm/mod.rs": ctx.vm_code,
        "src/engine/code/bytecode.rs": ctx.bytecode_code,
        "src/engine/api/context/bytecode.rs": ctx.context_code,
    }

    stage3b_items: dict[tuple[str, str], str] = {}

    def stage3b_code(relative: str) -> str:
        if relative not in stage3b_sources:
            stage3b_sources[relative] = ctx.rust_code_only(ctx.read_source(relative))
        return stage3b_sources[relative]
    ctx.stage3b_code = stage3b_code

    def stage3b_function(relative: str, name: str, diagnostic: str) -> str:
        key = (relative, name)
        if key not in stage3b_items:
            stage3b_items[key] = ctx.unique_braced_item(
                ctx.stage3b_code(relative),
                re.compile(rf"\bfn[ \t\n]+{re.escape(name)}\b[^{{}};]*\{{"),
                diagnostic,
                f"{relative}::{name}",
            )[0]
        return stage3b_items[key]
    ctx.stage3b_function = stage3b_function

    def stage3j_source_function(relative: str, name: str, diagnostic: str) -> str:
        source = ctx.read_source(relative)
        code = ctx.rust_code_only(source)
        _, start, end = ctx.unique_braced_item(
            code,
            re.compile(rf"\bfn[ \t\n]+{re.escape(name)}\b[^{{}};]*\{{"),
            diagnostic,
            f"{relative}::{name}",
        )
        if start < 0 or end < 0:
            return ""
        return source[start:end]
    ctx.stage3j_source_function = stage3j_source_function

    if len(re.findall(
        r"\bstruct[ \t\n]+ConstructorRef[ \t\n]*\([ \t\n]*ObjectRef[ \t\n]*\)[ \t\n]*;",
        ctx.runtime_code,
    )) != 1:
        ctx.fail("stage3b-constructor-capability", "ConstructorRef must remain the private [[Construct]] capability")

    construct_new_target_code = ctx.unique_braced_item(
        ctx.runtime_code,
        re.compile(r"\benum[ \t\n]+ConstructNewTarget[ \t\n]*\{"),
        "stage3b-new-target-capability",
        "validated-or-raw newTarget carrier",
    )[0]

    construct_new_target_payloads = {
        name: [" ".join(value.split()) for value in re.findall(
            rf"\b{name}[ \t\n]*\(([^()]*)\)[ \t\n]*,", construct_new_target_code
        )]
        for name in ("Validated", "Raw")
    }

    if (
        ctx.enum_variant_names(construct_new_target_code) != ["Validated", "Raw"]
        or construct_new_target_payloads != {"Validated": ["ConstructorRef"], "Raw": ["Value"]}
    ):
        ctx.fail(
            "stage3b-new-target-capability",
            f"ConstructNewTarget must remain Validated(ConstructorRef) or Raw(Value); found {construct_new_target_payloads}",
        )

    constructor_from_value = ctx.stage3b_function(
        "src/engine/heap/runtime/mod.rs", "constructor_from_value", "stage3b-constructor-capability"
    )

    normalized_constructor_from_value = " ".join(constructor_from_value.split())

    if (
        "Result<NativeConversion<ConstructorRef>, RuntimeError>" not in normalized_constructor_from_value
        or normalized_constructor_from_value.count("object_data.is_constructor") != 1
        or normalized_constructor_from_value.count("ConstructorRef::from_validated_object(object)") != 1
        or re.search(r"\b(?:CallableRef|is_callable|as_callable|callable_from_value)\b", constructor_from_value)
    ):
        ctx.fail(
            "stage3b-constructor-capability",
            "constructor conversion must validate only [[Construct]], without CallableRef narrowing",
        )

    vm_apply_arm = ctx.unique_braced_item(
        ctx.vm_code,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Apply[ \t\n]*\([ \t\n]*kind"
            r"[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3b-apply-stack",
        "VM Apply dispatch arm",
    )[0]

    ctx.require_ordered_fragments(
        "stage3b-apply-stack",
        "Apply must pop list, receiver-or-newTarget, and function before one typed host call",
        vm_apply_arm,
        (
            "let argument_array = self.pop()?;",
            "let this_or_new_target = self.pop()?;",
            "let function = self.pop()?;",
            "host.apply(function, this_or_new_target, argument_array, *kind)?",
        ),
    )

    if ctx.bytecode_code.count("Self::Apply(_) | Self::ApplySuper => (3, 1),") != 1:
        ctx.fail("stage3b-apply-stack", "Apply must retain its exact three-pop/one-push verifier effect")

    tail_stack_effects = (
        "Self::TailCall(argument_count) => (*argument_count as usize + 1, 0),",
        "Self::TailCallMethod(argument_count) => (*argument_count as usize + 2, 0),",
    )

    stack_effect_item = ctx.stage3b_function(
        "src/engine/code/bytecode.rs", "stack_effect", "stage3c-tail-verifier"
    )

    ctx.require_normalized_code_sha256(
        "stage3c-tail-verifier",
        "Instruction::stack_effect must remain the reviewed alias-free exhaustive stack model",
        stack_effect_item,
        "a6b0111cc4ec1e4316e8206d6cd75ccc66d7e910c248015b02de358894454538",
    )

    normalized_stack_effect = " ".join(stack_effect_item.split())

    if any(normalized_stack_effect.count(fragment) != 1 for fragment in tail_stack_effects):
        ctx.fail(
            "stage3c-tail-verifier",
            "TailCall and TailCallMethod must preserve argc+1/argc+2 pops and zero pushes",
        )

    verify_parts_item = ctx.stage3b_function(
        "src/engine/code/bytecode.rs", "verify_parts", "stage3c-tail-verifier"
    )

    normalized_verify_parts = " ".join(verify_parts_item.split())

    verifier_terminal_guard = deepcopy(evidence.VERIFIER_TERMINAL_GUARD)

    verifier_terminal_offset = normalized_verify_parts.find(verifier_terminal_guard)

    if verifier_terminal_offset < 0:
        ctx.fail(
            "stage3c-tail-verifier",
            "the reviewed terminal verifier corridor is missing",
        )
    else:
        ctx.require_normalized_code_sha256(
            "stage3c-tail-verifier",
            "the iterator guard, maximum-depth check, terminal dispatch, and successor enqueue corridor must remain alias-free and ordered",
            normalized_verify_parts[verifier_terminal_offset:],
            "5f18313dcb4ee5192b9fc3a53c76e1fe7e63bfba3eb3e5761033ecf5ad694370",
        )

    tail_terminal_prefix = "Instruction::TailCall(_) | Instruction::TailCallMethod(_)"

    tail_terminal_dispatch = deepcopy(evidence.TAIL_TERMINAL_DISPATCH)

    if (
        normalized_verify_parts.count(tail_terminal_prefix) != 2
        or normalized_verify_parts.count(tail_terminal_dispatch) != 1
        or normalized_verify_parts.count("enqueue_target(") != 4
        or normalized_verify_parts.count("enqueue_fallthrough(") != 4
    ):
        ctx.fail(
            "stage3c-tail-verifier",
            "both tail invocation instructions must be terminal verifier nodes and must not enqueue fallthrough",
        )

    take_call_arguments_item = ctx.stage3b_function(
        "src/engine/vm/mod.rs", "take_call_arguments", "stage3c-tail-vm"
    )

    ctx.require_normalized_code_sha256(
        "stage3c-tail-vm",
        "take_call_arguments must retain the exact checked suffix split without argument or fixed-value shadowing",
        take_call_arguments_item,
        "e8f523f68ef01df927a8761c6dc92ddafb0471fadee17ab9ead81f71d50287f4",
    )

    call_dispatch_item = ctx.stage3b_function(
        "src/engine/vm/mod.rs", "execute_call_instruction", "stage3c-tail-vm"
    )

    ctx.require_normalized_code_sha256(
        "stage3c-tail-vm",
        "execute_call_instruction must retain its alias-free exhaustive call-family dispatch",
        call_dispatch_item,
        "f6b13bdb02ab06e2177beba9cd3bda2f626c6baa97fa09420cbfa2459ef5a97d",
    )

    tail_call_arm = ctx.unique_braced_item(
        call_dispatch_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*TailCall[ \t\n]*\("
            r"[ \t\n]*argument_count[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3c-tail-vm",
        "TailCall VM arm",
    )[0]

    tail_method_arm = ctx.unique_braced_item(
        call_dispatch_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*TailCallMethod[ \t\n]*\("
            r"[ \t\n]*argument_count[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3c-tail-vm",
        "TailCallMethod VM arm",
    )[0]

    expected_tail_call_arm = deepcopy(evidence.EXPECTED_TAIL_CALL_ARM)

    expected_tail_method_arm = deepcopy(evidence.EXPECTED_TAIL_METHOD_ARM)

    if (
        " ".join(tail_call_arm.split()) != expected_tail_call_arm
        or " ".join(tail_method_arm.split()) != expected_tail_method_arm
    ):
        ctx.fail(
            "stage3c-tail-vm",
            "tail VM dispatch must preserve undefined/plain and receiver/function/method argument order, then return the host completion directly",
        )

    activation_execute_item = ctx.unique_braced_item(
        ctx.vm_code,
        re.compile(
            r"\bfn[ \t\n]+execute[ \t\n]*\([^{};]*\)[ \t\n]*->[ \t\n]*"
            r"Result[ \t\n]*<[ \t\n]*Completion[ \t\n]*,[ \t\n]*Error"
            r"[ \t\n]*>[ \t\n]*\{"
        ),
        "stage3c-tail-completion",
        "execute-to-completion activation driver",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3c-tail-completion",
        "the execute-to-completion driver must retain its unique Return terminal and Throw raise flow",
        activation_execute_item,
        "65d316cc1950e983ffc111de71f62e9f9cabb1833acddf93a0980b554df62368",
    )

    ctx.require_ordered_fragments(
        "stage3c-tail-completion",
        "tail Return must finish the current frame while Throw still enters the current activation raise path",
        activation_execute_item,
        (
            "Ok(InterpreterExit::Complete(Completion::Return(value))) => { return Ok(Completion::Return(value)); }",
            "Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,",
            "Err(error) if NativeErrorKind::from_javascript_error(error.kind()).is_some() => { host.materialize_error(error)? }",
            "if let Some(completion) = self.raise(raised, host, code.len())? { return Ok(completion); }",
        ),
    )

    activation_run_item = ctx.unique_braced_item(
        ctx.vm_code,
        re.compile(
            r"\bfn[ \t\n]+run[ \t\n]*\([^{};]*\)[ \t\n]*->[ \t\n]*"
            r"Result[ \t\n]*<[ \t\n]*VmExit[ \t\n]*,[ \t\n]*Error"
            r"[ \t\n]*>[ \t\n]*\{"
        ),
        "stage3c-tail-completion",
        "suspendable activation driver",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3c-tail-completion",
        "the suspendable driver must retain its unique Return terminal and Throw raise flow",
        activation_run_item,
        "d31f35156ced4fa99836ce08152eafd6e535d2eb40e3f0406362550a82269ed2",
    )

    raise_item = ctx.stage3b_function("src/engine/vm/mod.rs", "raise", "stage3c-tail-completion")

    ctx.require_normalized_code_sha256(
        "stage3c-tail-completion",
        "raise must retain one backtrace-first catch/iterator unwind loop without guarded bypasses",
        raise_item,
        "52a25a382122f09433bd29178885bc10c38222aee15f0d5482def7b15b45caab",
    )

    ctx.require_ordered_fragments(
        "stage3c-tail-completion",
        "tail Throw must retain backtrace attachment, catch transfer, and iterator unwind on the same activation",
        raise_item,
        (
            "host.ensure_backtrace(&value)?;",
            "let Some(region) = self.regions.pop() else { return Ok(Some(Completion::Throw(value))); };",
            "VmUnwindRegion::Catch { target, stack_depth, } => {",
            "host.prepare_captured_local_reuse()?;",
            "self.stack.truncate(stack_depth);",
            "self.stack.push(value);",
            "self.pc = checked_target(",
            "return Ok(None);",
            "VmUnwindRegion::Iterator { record_base, enabled, .. } => {",
            "match host.iterator_close(iterator, true)? {",
        ),
    )

    execute_inner_item = ctx.stage3b_function(
        "src/engine/vm/mod.rs", "execute_inner", "stage3c-tail-vm"
    )

    normalized_execute_inner = " ".join(execute_inner_item.split())

    call_route = deepcopy(evidence.CALL_ROUTE)

    call_route_offset = normalized_execute_inner.find(call_route)

    if call_route_offset < 0 or normalized_execute_inner.count(call_route) != 1:
        ctx.fail(
            "stage3c-tail-vm",
            "execute_inner must route the complete call family, including both tail variants, exactly once",
        )
    else:
        call_route_end = call_route_offset + len(call_route)
        ctx.require_normalized_code_sha256(
            "stage3c-tail-vm",
            "the execute_inner prefix through call-family routing must not intercept, alias, or remove tail completion",
            normalized_execute_inner[:call_route_end],
            "e425f4d42ef9a3a7b6552994a6f7da31009e70451f5b48e3312581929a54e54c",
        )

    capability_relative = "src/engine/code/binary_object/function_translate/capability.rs"

    capability_code = ctx.stage3b_code(capability_relative)

    capability_row_new = ctx.stage3b_function(
        capability_relative, "new", "stage3d-throw-capability-route"
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-capability-route",
        "CapabilityRow::new must preserve the raw, format, and policy fields without rewriting raw48",
        capability_row_new,
        "9e4be6620a97b5136ea400f536be3db0ef7f5cd95c74279dc30969278e19b4fc",
    )

    capability_row_macro = ctx.unique_braced_item(
        capability_code,
        re.compile(r"\bmacro_rules[ \t\n]*![ \t\n]*row[ \t\n]*\{"),
        "stage3d-throw-capability-route",
        "capability row macro",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3d-throw-capability-route",
        "the capability row macro must construct the declared audience and recipe directly",
        capability_row_macro,
        "96eac354655d5c9e47c266be4e31d6d1cbfbecc36fe70083bc90e1a59210a59b",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-capability-route",
        "row_for must select only the physical raw opcode's registry row",
        ctx.stage3b_function(
            capability_relative, "row_for", "stage3d-throw-capability-route"
        ),
        "e05d664bfc0b3bdd581c293111a0a81440e7227aa75b46baab209c061ce3f131",
    )

    dto_relative = "src/engine/code/binary_object/function_translate/dto.rs"

    for ctx.name, ctx.description, ctx.expected_hash in (
        (
            "includes_ordinary",
            "ordinary audience membership must remain OrdinaryOnly or Shared",
            "eac985e03d25991731ee08d4964ed8f621886bd9e6d6783dde9f117075346be8",
        ),
        (
            "supports_ordinary",
            "FunctionInstruction must consult its retained audience for ordinary admission",
            "d0f6a6a6678edbe91a19fdf00d9deab39d21d203fd38f57590b4d41abe8d8d8f",
        ),
        (
            "operation",
            "FunctionInstruction must expose the retained typed operation without substitution",
            "903997a7bf00f7594ba961f08a431af59ad1bd42ca678390d39944b8b8c11c0a",
        ),
        (
            "instructions",
            "FunctionCode must expose its retained instruction sequence without filtering",
            "ae989b36159707bc008c1f1b7144b6c0c0b11497ce8b80046157d5a22db99378",
        ),
    ):
        ctx.require_normalized_code_sha256(
            "stage3d-throw-translation-route",
            ctx.description,
            ctx.stage3b_function(dto_relative, ctx.name, "stage3d-throw-translation-route"),
            ctx.expected_hash,
        )

    translate_relative = "src/engine/code/binary_object/function_translate/mod.rs"

    translate_code = ctx.stage3b_code(translate_relative)

    ctx.require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "TranslationTarget must delegate ordinary admission to the retained audience",
        ctx.stage3b_function(
            translate_relative, "accepts", "stage3d-throw-translation-route"
        ),
        "90579d0e77fd5b936444117085b9b1bb8246364a9f8bc3878c95d7e3e271d16c",
    )

    ctx.pending_expansion_impl = ctx.unique_braced_item(
        translate_code,
        re.compile(
            r"\bimpl[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]+"
            r"PendingExpansion[ \t\n]*<[ \t\n]*'image[ \t\n]*>[ \t\n]*\{"
        ),
        "stage3d-throw-translation-route",
        "PendingExpansion implementation",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "PendingExpansion must retain each ready operation exactly once and in order",
        ctx.pending_expansion_impl,
        "9f43f69140aababa9f85f6845ce991b6133a52693feb7dbb83177e98e3c6001e",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "operation_for_target must lower admitted ordinary operations and reject outside-target aliases",
        ctx.stage3b_function(
            translate_relative,
            "operation_for_target",
            "stage3d-throw-translation-route",
        ),
        "124ecc9366407ebfa14448710b3795c2ee74137aea4f099076fdd13e0b32aec1",
    )

    translate_native_plan_item = ctx.stage3b_function(
        translate_relative, "translate_native_plan", "stage3d-throw-translation-route"
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-translation-route",
        "translate_native_plan must preserve the physical row, target audience, ready operation, and final FunctionCode without a post-lowering remap",
        translate_native_plan_item,
        "fe149677e125ffef44ebac61b8b9799eff3166eafd7efa93e086b84571eb9867",
    )

    ctx.require_normalized_corridor_sha256(
        "stage3d-throw-translation-route",
        "translate_native_plan must carry the physical row through its audience expansion into pending code",
        translate_native_plan_item,
        "let row = row_for(opcode);",
        "pending.push(PendingInstruction { audience, diagnostic, expansion, });",
        "de451567beafe6477081e0466f2cdc2b1cb302b8a8202130ef53226509f39524",
    )

    ctx.require_normalized_corridor_sha256(
        "stage3d-throw-translation-route",
        "translate_native_plan must publish every ready operation through FunctionInstruction::new without remapping",
        translate_native_plan_item,
        "for instruction in pending {",
        "output.push(FunctionInstruction::new( instruction.audience, instruction.diagnostic, operation, ));",
        "154a0d0ab86cab0cb75aebcc4bb31e8cd6e7b8015e02a4f61dd32eadf17842ae",
    )

    ordinary_relative = "src/engine/code/binary_object/ordinary_leaf.rs"

    ctx.require_normalized_code_sha256(
        "stage3d-throw-ordinary-route",
        "ordinary lower_code must require ordinary audience support and lower every typed operation once",
        ctx.stage3b_function(ordinary_relative, "lower_code", "stage3d-throw-ordinary-route"),
        "802efb137202fe4c4b7380359e627b9e97d676ed0e19b6041a58230e03820557",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-ordinary-route",
        "OrdinaryLeafDraft::into_parts must retain metadata, constants, and code without substitution",
        ctx.stage3b_function(ordinary_relative, "into_parts", "stage3d-throw-ordinary-route"),
        "cb258f4d808ff5458a2be60fe0050ebc734736ec811b2232f9ff6c031e7e1cf9",
    )

    ctx.require_normalized_code_sha256(
        "stage3e-read-only-ordinary-route",
        "admit_image must pass the authenticated input-atom slot count into the typed ordinary lowering without bypass",
        ctx.stage3b_function(ordinary_relative, "admit_image", "stage3e-read-only-ordinary-route"),
        "afec36c89046ac4ddb822a58c95bb6d1f8683ce6cedfea0b2718c4d9bf4434ae",
    )

    input_atom_ledger_impl = ctx.unique_braced_item(
        ctx.stage3b_code(ordinary_relative),
        re.compile(r"\bimpl[ \t\n]+InputAtomLedger[ \t\n]*\{"),
        "stage3e-read-only-atom-ledger",
        "InputAtomLedger implementation",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3e-read-only-atom-ledger",
        "the input-atom ledger must admit at most one slot, require any declared slot to be consumed by raw49, and preserve provenance",
        input_atom_ledger_impl,
        "3b2000fa311f9de95eca3794884907f8b565f2cad4f52b2a2ce0369226799057",
    )

    detached_atom_name_impl = ctx.unique_braced_item(
        ctx.stage3b_code(ordinary_relative),
        re.compile(r"\bimpl[ \t\n]+DetachedAtomName[ \t\n]*\{"),
        "stage3e-read-only-atom-ledger",
        "DetachedAtomName implementation",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3e-read-only-atom-ledger",
        "DetachedAtomName must release only its owned UTF-16 units into publication",
        detached_atom_name_impl,
        "fe30418c8de3040b9177db6f452ab0c95a961a658407bc83a3b20177077377b0",
    )

    ctx.require_normalized_code_sha256(
        "stage3e-read-only-atom-ledger",
        "copy_read_only_name must admit only String atoms and preserve every UTF-16 unit in an owned payload",
        ctx.stage3b_function(ordinary_relative, "copy_read_only_name", "stage3e-read-only-atom-ledger"),
        "b5f7bc69b10a77a23b94db94558816a972c477459aab01c2939368eab2d7e8e0",
    )

    ctx.require_normalized_code_sha256(
        "stage3e-read-only-publication",
        "the ordinary publisher must synthesize exactly one verified String constant per ThrowReadOnly and publish its matching typed index before verification",
        ctx.stage3b_function(
            ctx.consumer_relative,
            "read_trusted_ordinary_function_in_realm",
            "stage3e-read-only-publication",
        ),
        "7bce6b697724b3bf4e6cd3b4747887d7e3a4a3376bf940907650d44ad7261f4e",
    )

    if normalized_stack_effect.count("| Self::Throw => (1, 0),") != 1:
        ctx.fail(
            "stage3d-throw-verifier",
            "Throw must consume exactly one value and produce no fallthrough value",
        )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-verifier",
        "the full typed verifier must keep Throw terminal without a guarded or aliased fallthrough path",
        verify_parts_item,
        "dc38a575344a31719e9923b1cf0412c01d3b2932d954d5243a93e826a97e5c8d",
    )

    if normalized_verify_parts.count(tail_terminal_dispatch) != 1:
        ctx.fail(
            "stage3d-throw-verifier",
            "Throw must remain in the unique terminal verifier arm",
        )

    if (
        normalized_stack_effect.count("| Self::ThrowReadOnly(_)") != 1
        or "Self::ThrowReadOnly(_) => (1, 0)" in normalized_stack_effect
        or normalized_verify_parts.count(
            "Instruction::ThrowReadOnly(_) | Instruction::ThrowRedeclaration(_) | Instruction::ThrowDeleteSuper | Instruction::ThrowIteratorMissingThrow => {}"
        ) != 1
    ):
        ctx.fail(
            "stage3e-read-only-verifier",
            "ThrowReadOnly must remain a zero-pop, zero-push terminal verifier node with no fallthrough",
        )

    if (
        normalized_stack_effect.count("Self::Nop | Self::CheckCtor") != 1
        or normalized_verify_parts.count("Instruction::Nop") != 0
        or normalized_verify_parts.count("_ => enqueue_fallthrough(") != 1
    ):
        ctx.fail(
            "stage3f-nop-verifier",
            "Instruction::Nop must remain an existing zero-pop, zero-push node that reaches the verifier's unique ordinary fallthrough path",
        )

    if (
        normalized_stack_effect.count("Self::Object => (0, 1)") != 1
        or normalized_verify_parts.count("Instruction::Object") != 0
        or normalized_verify_parts.count("_ => enqueue_fallthrough(") != 1
    ):
        ctx.fail(
            "stage3g-object-verifier",
            "Instruction::Object must remain an existing zero-pop, one-push node that reaches the verifier's unique ordinary fallthrough path",
        )

    if (
        normalized_stack_effect.count(
            "Self::SetName(_) | Self::ToObject | Self::IteratorCheckObject => (1, 1)"
        )
        != 1
        or normalized_verify_parts.count("Instruction::ToObject") != 0
        or normalized_verify_parts.count("_ => enqueue_fallthrough(") != 1
    ):
        ctx.fail(
            "stage3h-to-object-verifier",
            "Instruction::ToObject must remain an existing one-pop, one-push node that reaches the verifier's unique ordinary fallthrough path",
        )

    execute_hot_item = ctx.stage3b_function(
        "src/engine/vm/mod.rs", "execute_hot_instruction", "stage3d-throw-completion"
    )

    throw_vm_arm = ctx.unique_braced_item(
        execute_hot_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Throw[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3d-throw-completion",
        "VM Throw dispatch arm",
    )[0]

    if " ".join(throw_vm_arm.split()) != (
        "Instruction::Throw => { return self.pop().map(|value| "
        "Some(Completion::Throw(value))); }"
    ):
        ctx.fail(
            "stage3d-throw-completion",
            "VM Throw must pop the original value directly into Completion::Throw",
        )

    throw_read_only_vm_arm = ctx.unique_braced_item(
        execute_hot_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*ThrowReadOnly[ \t\n]*\("
            r"[ \t\n]*index[ \t\n]*\)[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3e-read-only-completion",
        "VM ThrowReadOnly dispatch arm",
    )[0]

    if " ".join(throw_read_only_vm_arm.split()) != (
        "Instruction::ThrowReadOnly(index) => { "
        "return Err(host.read_only_error(*index)?); }"
    ):
        ctx.fail(
            "stage3e-read-only-completion",
            "VM ThrowReadOnly must call the read-only error hook directly without popping or returning",
        )

    nop_vm_arm = ctx.unique_braced_item(
        execute_hot_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Nop[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3f-nop-vm",
        "VM Nop dispatch arm",
    )[0]

    if " ".join(nop_vm_arm.split()) != "Instruction::Nop => {}":
        ctx.fail(
            "stage3f-nop-vm",
            "VM Nop must remain the existing empty no-effect dispatch arm",
        )

    object_vm_cold_item = ctx.stage3b_function(
        "src/engine/vm/mod.rs", "execute_cold_instruction", "stage3g-object-vm"
    )

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-vm",
        "execute_cold_instruction must retain its complete reviewed dispatch so ToObject cannot be diverted by a direct, aliased, or helper-mediated pre-match path",
        object_vm_cold_item,
        "1d8fd1a51a5c2e349b2a1055c408a5c877c76661d60373bf22239dae60716cdb",
    )

    object_vm_arm = ctx.unique_braced_item(
        object_vm_cold_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*Object[ \t\n]*=>[ \t\n]*"
            r"match[ \t\n]+host[ \t\n]*\.[ \t\n]*object[ \t\n]*\("
            r"[ \t\n]*\)[ \t\n]*\?[ \t\n]*\{"
        ),
        "stage3g-object-vm",
        "VM Object dispatch arm",
    )[0]

    if " ".join(object_vm_arm.split()) != (
        "Instruction::Object => match host.object()? { "
        "Completion::Return(object) => self.stack.push(object), "
        "Completion::Throw(value) => return Ok(Some(Completion::Throw(value))), }"
    ):
        ctx.fail(
            "stage3g-object-vm",
            "VM Object must delegate once to the host, push only its returned fresh Object, and propagate a host throw",
        )

    ctx.require_normalized_code_sha256(
        "stage3g-object-realm",
        "the runtime VM host must allocate Object through the executing bytecode's current defining realm",
        ctx.stage3b_function(
            "src/engine/vm/host_bridge.rs", "object", "stage3g-object-realm"
        ),
        "90cbeb40094a4266ebba996ce790be75ce46b2ab8959a4996265ddcc656924ce",
    )

    to_object_vm_arm = ctx.unique_braced_item(
        object_vm_cold_item,
        re.compile(
            r"\bInstruction[ \t\n]*::[ \t\n]*ToObject[ \t\n]*=>[ \t\n]*\{"
        ),
        "stage3h-to-object-vm",
        "VM ToObject dispatch arm",
    )[0]

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-vm",
        "VM ToObject must preserve Object identity, reject nullish values with TypeError, and box only primitives without a coercion hook",
        to_object_vm_arm,
        "7ea7f87dd0d4c20abc0d4148c7e7b2b2b4c4cdbe06be6ac396130138b4123122",
    )

    if re.search(
        r"\b(?:to_primitive|to_property_key|value_of|to_string)[ \t\n]*\(",
        to_object_vm_arm,
    ):
        ctx.fail(
            "stage3h-to-object-vm",
            "VM ToObject must not invoke user coercion while preserving Objects or boxing primitives",
        )

    ctx.require_normalized_code_sha256(
        "stage3h-to-object-realm",
        "the runtime VM host must allocate every primitive wrapper through the executing bytecode's current defining realm",
        ctx.stage3b_function(
            "src/engine/vm/host_bridge.rs", "box_primitive", "stage3h-to-object-realm"
        ),
        "47f1cf4db70f24b86c09ea669b93a0f0a9780ae35a119c5b2f7698f959984ffa",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "execute_inner must carry raw48 from fetch through the hot dispatcher without a guarded completion alias",
        execute_inner_item,
        "fa323bad632c685546d3efadbe860a77f540b1066559744ea23c333958036358",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "execute_hot_instruction must enter its unique match before handling Throw and retain the exact dispatch body",
        execute_hot_item,
        "2fab69bd24de64e6ab0149f4c267b8312e1cebecb6064c4f8c81a44a74156304",
    )

    execute_published_item = ctx.stage3b_function(
        "src/engine/vm/mod.rs", "execute_published", "stage3d-throw-critical-route"
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "execute_published must return the activation's Completion directly without post-processing Throw",
        execute_published_item,
        "b2743fde8341d22bb2592d3810e10030ecce6f812befe9be150a80ccd982a0a7",
    )

    runtime_vm_host_relative = "src/engine/vm/host_bridge.rs"

    execute_bytecode_callable_item = ctx.stage3b_function(
        runtime_vm_host_relative,
        "execute_bytecode_callable",
        "stage3d-throw-critical-route",
    )

    ctx.require_normalized_corridor_sha256(
        "stage3d-throw-critical-route",
        "the module-link bytecode bridge must finish its frame and return execute_published without completion remapping",
        execute_bytecode_callable_item,
        "if is_module_link_entry {",
        "return result.map_err(RuntimeError::Engine);",
        "cc840e26ee0461e8d8951e15459568f47c87e2c5f21e32bc878f38a314c0b57a",
    )

    ctx.require_normalized_corridor_sha256(
        "stage3d-throw-critical-route",
        "the normal bytecode bridge must finish its frame and return execute_published without completion remapping",
        execute_bytecode_callable_item,
        "FunctionKind::Normal => {}",
        "result.map_err(RuntimeError::Engine) }",
        "4c6141e7e3aaabf76b78abf7274c2db6864a7feb516ed02c56bac8ebf6af2b60",
    )

    call_internal_item = ctx.stage3b_function(
        "src/engine/builtins/dispatch.rs",
        "call_internal",
        "stage3d-throw-critical-route",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-critical-route",
        "call_internal must preserve the callable completion before, during, and after forwarded-frame cleanup",
        call_internal_item,
        "a94d89cf8db9fb9658f867c9d6af1115c979e571165ff953909d0dbb86e74714",
    )

    ctx.require_normalized_corridor_sha256(
        "stage3d-throw-critical-route",
        "call_internal must preserve the callable completion across forwarded-frame cleanup",
        call_internal_item,
        "let result = (|| loop {",
        "frame_error.map_or(result, Err)",
        "3177d4ccf8210565de65aa1534e23c483ae98e25c5b527055b70386d44307735",
    )

    for ctx.relative, ctx.name, ctx.description, ctx.expected_hash in (
        (
            runtime_vm_host_relative,
            "call",
            "RuntimeVmHost::call must forward nested callable completions without remapping Throw before caller catch",
            "c1970423cb9f5a75f26e5c309dcc724ee92f74bd48a6e8d24311a3fc94bb6a14",
        ),
        (
            "src/engine/object/internal_methods.rs",
            "call_value_internal",
            "call_value_internal must preserve callable and Proxy completions for the current activation",
            "d7564209dc646e4a18641690161eefba207110f7a244377489a96510bb9d66b5",
        ),
        (
            "src/engine/api/context/calls.rs",
            "call",
            "Context::call must pass call_internal's completion directly to finish_completion",
            "bf80579858f0ce24fdb44408eb43a1b3a7263bba07ef972026a22a2b1ff0fa89",
        ),
        (
            runtime_vm_host_relative,
            "ensure_backtrace",
            "RuntimeVmHost::ensure_backtrace must delegate explicit Throw values to the runtime backtrace hook",
            "532bdb791b4b0a3e0d4bc1b8bd9658a5c58e321bf864c3786684bbb22006b0d2",
        ),
        (
            runtime_vm_host_relative,
            "iterator_close",
            "RuntimeVmHost::iterator_close must retain getter, call, pending-exception, and result precedence",
            "242106effd28c2885dd94c0cdbb4f85312b650291dc97d7593ff028e83c02aae",
        ),
        (
            runtime_vm_host_relative,
            "read_only_error",
            "RuntimeVmHost::read_only_error must resolve the verified String constant and build the native TypeError in the bytecode realm",
            "b6f792f0992f3b857dc97f0521470557b22593cf86c61f1c079152245ad39f7e",
        ),
        (
            runtime_vm_host_relative,
            "materialize_error",
            "RuntimeVmHost::materialize_error must allocate the native TypeError in the current defining realm before the existing raise path",
            "46fe1d5c192e13c6d1e2f4ae69b6ab04c0b56b0187387c09d2e25a0dab7a0a1f",
        ),
    ):
        ctx.require_normalized_code_sha256(
            "stage3d-throw-critical-route",
            ctx.description,
            ctx.stage3b_function(ctx.relative, ctx.name, "stage3d-throw-critical-route"),
            ctx.expected_hash,
        )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-completion",
        "execute must route every Throw completion through the current activation's raise path",
        activation_execute_item,
        "65d316cc1950e983ffc111de71f62e9f9cabb1833acddf93a0980b554df62368",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-completion",
        "the suspendable activation driver must share the same Throw raise path",
        activation_run_item,
        "d31f35156ced4fa99836ce08152eafd6e535d2eb40e3f0406362550a82269ed2",
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-completion",
        "raise must attach backtraces before ordered catch and iterator unwinding",
        raise_item,
        "52a25a382122f09433bd29178885bc10c38222aee15f0d5482def7b15b45caab",
    )

    ctx.require_ordered_fragments(
        "stage3d-throw-completion",
        "explicit Throw must attach a backtrace, prefer the innermost catch/iterator region order, and preserve the original value across iterator close",
        raise_item,
        (
            "host.ensure_backtrace(&value)?;",
            "let Some(region) = self.regions.pop() else { return Ok(Some(Completion::Throw(value))); };",
            "match region {",
            "VmUnwindRegion::Catch { target, stack_depth, } => {",
            "self.stack.push(value);",
            "VmUnwindRegion::Iterator { record_base, enabled, .. } => {",
            "match host.iterator_close(iterator, true)? {",
            "IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}",
        ),
    )

    for ctx.name, ctx.description, ctx.expected_hash in (
        (
            "set_pending_exception",
            "the pending-exception writer must retain the original value as an owned runtime root",
            "128c592e4525a60a1bd79dffff836195d8269c54be97a7f17cb98bc5a00a14f8",
        ),
        (
            "take_pending_exception",
            "the pending-exception reader must take and reconstruct the owned original value",
            "8b9051265509db89853144e02a180da15b0851f4d6fbe13bce2b8f3b5743d6de",
        ),
        (
            "has_pending_exception",
            "the pending-exception observer must report the actual pending slot",
            "1812d935455fea3c9027d72b60c252701cc783baa4433a1364050ea4b42e4956",
        ),
    ):
        ctx.require_normalized_code_sha256(
            "stage3d-throw-pending",
            ctx.description,
            ctx.stage3b_function("src/engine/heap/runtime/mod.rs", ctx.name, "stage3d-throw-pending"),
            ctx.expected_hash,
        )

    for ctx.name, ctx.description, ctx.expected_hash in (
        (
            "has_exception",
            "Context::has_exception must observe the runtime pending slot directly",
            "81a8f856db04b94fd5dd58141a1ed0763d3132fc3ebe3d19dec1645e5367de77",
        ),
        (
            "take_exception",
            "Context::take_exception must return the value taken from the runtime pending slot",
            "691d43e5f989578873bf0ae60896055ed031a9c031c4e4ed74cb908e172bd626",
        ),
    ):
        ctx.require_normalized_code_sha256(
            "stage3d-throw-pending",
            ctx.description,
            ctx.stage3b_function("src/engine/api/context/mod.rs", ctx.name, "stage3d-throw-pending"),
            ctx.expected_hash,
        )

    finish_completion_item = ctx.stage3b_function(
        "src/engine/api/context/mod.rs", "finish_completion", "stage3d-throw-pending"
    )

    ctx.require_normalized_code_sha256(
        "stage3d-throw-pending",
        "the public Context completion bridge must retain the original thrown value in the pending-exception slot",
        finish_completion_item,
        "d2e99ad914f05e1e7d81e0d909d480fae797df41f295a0a7abf806f1ea57ebc9",
    )

    ctx.require_ordered_fragments(
        "stage3d-throw-pending",
        "Completion::Throw must become the pending exception before RuntimeError::Exception is returned",
        finish_completion_item,
        (
            "Completion::Return(value) => Ok(value),",
            "Completion::Throw(value) => {",
            "self.runtime.set_pending_exception(value)?;",
            "Err(RuntimeError::Exception)",
        ),
    )
