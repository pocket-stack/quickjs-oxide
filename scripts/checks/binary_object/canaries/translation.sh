expect_rejected translate-extra-module function-translate-module-set \
    src/engine/code/binary_object/function_translate/escape.rs 'fn escape() {}'
expect_full_rewrite_table < "$boundary_dir/canaries/translate_canaries.txt"
expect_full_rewrite_rejected translate-ready-remap \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    '    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));' \
    '    let ready = |_operation| Ok(PendingExpansion::one(PendingOperation::Ready(FunctionOp::PushNull)));'
expect_full_rewrite_rejected translate-push-i32-payload \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (Recipe::PushI32, NativeOperands::I32(value) | NativeOperands::NoneInt(value)) => {\n            ready(FunctionOp::PushI32(*value))\n        }' \
    $'        (Recipe::PushI32, NativeOperands::I32(value) | NativeOperands::NoneInt(value)) => {\n            ready(FunctionOp::PushI32(-*value))\n        }'
expect_full_rewrite_rejected translate-get-local-payload \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (Recipe::GetLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {\n            ready(FunctionOp::GetLocal(*index))\n        }' \
    $'        (Recipe::GetLocal, NativeOperands::Loc(index) | NativeOperands::NoneLoc(index)) => {\n            ready(FunctionOp::GetLocal(index.saturating_add(1)))\n        }'
expect_full_rewrite_rejected translate-call-format-union-collapse \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (\n            Recipe::Call,\n            NativeOperands::NPop(argument_count) | NativeOperands::NPopX(argument_count),\n        ) => ready(FunctionOp::Call(*argument_count)),' \
    $'        (Recipe::Call, NativeOperands::NPop(argument_count)) =>\n            ready(FunctionOp::Call(*argument_count)),'
expect_full_rewrite_rejected translate-call-argc-plus-one \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    '        ) => ready(FunctionOp::Call(*argument_count)),' \
    '        ) => ready(FunctionOp::Call(argument_count.saturating_add(1))),'
expect_full_rewrite_rejected translate-call-argc-minus-one \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    '        ) => ready(FunctionOp::Call(*argument_count)),' \
    '        ) => ready(FunctionOp::Call(argument_count.saturating_sub(1))),'
expect_full_rewrite_rejected stage3a-translate-construct-argc-plus-one \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(*argument_count))\n        }' \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(argument_count.saturating_add(1)))\n        }'
expect_full_rewrite_rejected stage3a-translate-construct-call-method-swap \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(*argument_count))\n        }\n        (Recipe::CallMethod, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::CallMethod(*argument_count))\n        }' \
    $'        (Recipe::Construct, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::CallMethod(*argument_count))\n        }\n        (Recipe::CallMethod, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Construct(*argument_count))\n        }'
expect_full_rewrite_rejected stage3c-translate-tail-call-to-call \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (Recipe::TailCall, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::TailCall(*argument_count))\n        }' \
    $'        (Recipe::TailCall, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::Call(*argument_count))\n        }'
expect_full_rewrite_rejected stage3c-translate-tail-method-to-call-return \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    $'        (Recipe::TailCallMethod, NativeOperands::NPop(argument_count)) => {\n            ready(FunctionOp::TailCallMethod(*argument_count))\n        }' \
    $'        (Recipe::TailCallMethod, NativeOperands::NPop(argument_count)) => {\n            Ok(PendingExpansion::two(\n                PendingOperation::Ready(FunctionOp::CallMethod(*argument_count)),\n                PendingOperation::Ready(FunctionOp::Return),\n            ))\n        }'
expect_full_rewrite_rejected stage3b-translate-apply-zero-kind \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    '            ready(FunctionOp::Apply(FunctionApplyKind::Call))' \
    '            ready(FunctionOp::Apply(FunctionApplyKind::Construct))'
expect_full_rewrite_rejected stage3b-translate-apply-noncanonical-admission \
    function-translate-semantic-dispatch src/engine/code/binary_object/function_translate/mod.rs \
    '            Err(FunctionTranslateError::non_canonical_apply_magic(*magic))' \
    '            ready(FunctionOp::Apply(FunctionApplyKind::Call))'
expect_full_rewrite_rejected stage3b-apply-error-classification \
    ordinary-leaf-apply-admission src/engine/code/binary_object/ordinary_leaf.rs \
    '    if error.is_unadmitted_operand_error() {' \
    '    if false && error.is_unadmitted_operand_error() {'
expect_full_rewrite_rejected stage3b-apply-stack-effect \
    stage3b-apply-stack src/engine/code/bytecode.rs \
    '            Self::Apply(_) | Self::ApplySuper => (3, 1),' \
    '            Self::Apply(_) | Self::ApplySuper => (2, 1),'
expect_full_rewrite_rejected stage3b-nullish-apply-bypass \
    stage3b-apply-order src/engine/vm/host_bridge.rs \
    '        if matches!(argument_array, Value::Undefined | Value::Null) {' \
    '        if false && matches!(argument_array, Value::Undefined | Value::Null) {'
expect_full_rewrite_rejected stage3b-raw-new-target-collapse \
    stage3b-raw-construction src/engine/heap/runtime/mod.rs \
    '            ConstructNewTarget::Raw(new_target) => {' \
    '            ConstructNewTarget::Validated(new_target) => {'
expect_full_rewrite_rejected stage3b-constructor-callable-narrowing \
    stage3b-constructor-capability src/engine/heap/runtime/mod.rs \
    '            object_data.is_constructor' \
    '            object_data.is_constructor && object_data.is_callable'
expect_full_rewrite_rejected stage3b-bound-new-target-retarget \
    stage3b-raw-construction src/engine/heap/runtime/mod.rs \
    '                    new_target.retarget_bound_identity(&constructor, &target);' \
    '                    let _ = (&new_target, &constructor, &target);'
expect_full_rewrite_rejected stage3b-proxy-before-callable \
    stage3b-raw-construction src/engine/heap/runtime/mod.rs \
    '            if self.is_proxy_object(constructor.as_object())? {' \
    '            if false && self.is_proxy_object(constructor.as_object())? {'
expect_full_rewrite_rejected stage3b-function-realm-fallback \
    stage3b-constructor-prototype src/engine/heap/runtime/mod.rs \
    '        self.function_realm_from_value(caller_realm, new_target)' \
    '        Ok(NativeConversion::Value(caller_realm))'
expect_full_rewrite_rejected stage3b-native-prototype-helper-bypass \
    stage3b-native-prototype-family src/engine/builtins/array_buffer.rs \
    '        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {' \
    '        self.constructor_prototype_source(realm, &new_target).map(|_| |fallback_realm| {'
expect_full_rewrite_rejected stage3b-proxy-call-layer-capability \
    stage3b-proxy-call-order src/engine/object/internal_methods.rs \
    '            if !rooted.data.is_callable {' \
    '            if false && !rooted.data.is_callable {'
expect_full_rewrite_rejected stage3b-proxy-construct-callable-narrowing \
    stage3b-proxy-construct-order src/engine/object/internal_methods.rs \
    '                match self.constructor_from_value(realm, Value::Object(rooted.target.clone()))? {' \
    '                match self.callable_from_value(Value::Object(rooted.target.clone())) {'
expect_full_rewrite_rejected stage3b-public-raw-construction-leak \
    stage3b-public-construction src/engine/api/context/calls.rs \
    '            .construct_internal(self.realm, constructor, new_target, arguments)' \
    '            .construct_value_with_raw_new_target_internal(self.realm, Value::Object(constructor.as_object().clone()), Value::Object(new_target.as_object().clone()), arguments)'
expect_full_rewrite_rejected stage3b-species-callable-narrowing \
    stage3b-species-constructor src/engine/builtins/promise.rs \
    '    ) -> Result<NativeConversion<Option<ConstructorRef>>, RuntimeError> {' \
    '    ) -> Result<NativeConversion<Option<CallableRef>>, RuntimeError> {'
expect_full_rewrite_table < "$boundary_dir/canaries/stage3b_payload_canaries.txt"
expect_full_rewrite_rejected stage3b-apply-nullish-prework \
    stage3b-apply-order src/engine/vm/host_bridge.rs \
    $'        if matches!(argument_array, Value::Undefined | Value::Null) {\n            return self' \
    $'        if matches!(argument_array, Value::Undefined | Value::Null) {\n            let _ = self.build_argument_list(Value::Undefined)?;\n            return self'
expect_full_rewrite_rejected stage3b-native-prototype-payload \
    stage3b-native-prototype-family src/engine/builtins/array_buffer.rs \
    '        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {' \
    '        self.prototype_from_constructor_value(realm, &Value::Undefined, |fallback_realm| {'
expect_full_rewrite_table < "$boundary_dir/canaries/stage3c_canaries.txt"
expect_full_rewrite_rejected stage3c-tail-terminal-fallthrough \
    stage3c-tail-verifier src/engine/code/bytecode.rs \
    $'            Instruction::TailCall(_)\n            | Instruction::TailCallMethod(_)\n            | Instruction::Return\n            | Instruction::ReturnUndefined\n            | Instruction::ReturnDerived(_)\n            | Instruction::Throw\n            | Instruction::Ret => {}' \
    $'            Instruction::Return\n            | Instruction::ReturnUndefined\n            | Instruction::ReturnDerived(_)\n            | Instruction::Throw\n            | Instruction::Ret => {}'
expect_full_rewrite_rejected stage3c-blocker-mapping-swap \
    function-translate-registry-blockers \
    src/engine/code/binary_object/function_translate/capability.rs \
    $'    row!(3, Const, Blocked, FunctionGraph),\n    row!(4, Atom, ScalarOnly, Recipe::PushAtom),\n    row!(5, Atom, Blocked, ValueConstruction),' \
    $'    row!(3, Const, Blocked, ValueConstruction),\n    row!(4, Atom, ScalarOnly, Recipe::PushAtom),\n    row!(5, Atom, Blocked, FunctionGraph),'
expect_full_rewrite_rejected stage3c-translate-ready-shadow \
    function-translate-semantic-dispatch \
    src/engine/code/binary_object/function_translate/mod.rs \
    $'    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));\n    match (recipe, operands) {' \
    $'    let ready = |operation| Ok(PendingExpansion::one(PendingOperation::Ready(operation)));\n    let ready = |operation| {\n        let operation = match operation {\n            FunctionOp::TailCall(argument_count) => FunctionOp::Call(argument_count),\n            FunctionOp::TailCallMethod(argument_count) => FunctionOp::CallMethod(argument_count),\n            operation => operation,\n        };\n        Ok(PendingExpansion::one(PendingOperation::Ready(operation)))\n    };\n    match (recipe, operands) {'
expect_full_rewrite_rejected stage3c-ordinary-guarded-tail-bypass \
    ordinary-leaf-translated-code \
    src/engine/code/binary_object/ordinary_leaf.rs \
    $'    match operation {\n        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),\n        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),\n        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),\n        FunctionOp::ToPropKey => Ok(OrdinaryLeafOp::ToPropKey),\n        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),\n        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),' \
    $'    if matches!(operation, FunctionOp::TailCall(0)) {\n        return Ok(OrdinaryLeafOp::Call(0));\n    }\n    match operation {\n        FunctionOp::Nop => Ok(OrdinaryLeafOp::Nop),\n        FunctionOp::Object => Ok(OrdinaryLeafOp::Object),\n        FunctionOp::ToObject => Ok(OrdinaryLeafOp::ToObject),\n        FunctionOp::ToPropKey => Ok(OrdinaryLeafOp::ToPropKey),\n        FunctionOp::PushThis => Ok(OrdinaryLeafOp::PushThis),\n        FunctionOp::PushI32(value) => Ok(OrdinaryLeafOp::PushI32(*value)),'
expect_full_rewrite_rejected stage3c-publisher-alias-tail-bypass \
    ordinary-leaf-consumer-lowering \
    src/engine/code/binary_object_publish.rs \
    $'    let instruction = match operation {\n        OrdinaryLeafOp::Nop => Instruction::Nop,\n        OrdinaryLeafOp::Object => Instruction::Object,\n        OrdinaryLeafOp::ToObject => Instruction::ToObject,\n        OrdinaryLeafOp::ToPropKey => Instruction::ToPropKey,\n        OrdinaryLeafOp::PushThis => Instruction::PushThis,\n        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),' \
    $'    use OrdinaryLeafOp as O;\n    if let O::TailCall(argument_count) = &operation {\n        return Ok(Instruction::Call(*argument_count));\n    }\n    let instruction = match operation {\n        OrdinaryLeafOp::Nop => Instruction::Nop,\n        OrdinaryLeafOp::Object => Instruction::Object,\n        OrdinaryLeafOp::ToObject => Instruction::ToObject,\n        OrdinaryLeafOp::ToPropKey => Instruction::ToPropKey,\n        OrdinaryLeafOp::PushThis => Instruction::PushThis,\n        OrdinaryLeafOp::PushI32(value) => Instruction::PushI32(value),'
expect_full_rewrite_rejected stage3c-stack-effect-guarded-bypass \
    stage3c-tail-verifier src/engine/code/bytecode.rs \
    $'    pub const fn stack_effect(&self) -> (usize, usize) {\n        match self {' \
    $'    pub const fn stack_effect(&self) -> (usize, usize) {\n        if let Self::TailCall(0) | Self::TailCallMethod(0) = self {\n            return (1, 1);\n        }\n        match self {'
expect_full_rewrite_rejected stage3c-verifier-alias-fallthrough \
    stage3c-tail-verifier src/engine/code/bytecode.rs \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses' \
    $'        record_maximum_depth(&mut maximum, next_depth, declared_max_stack)?;\n        use Instruction as I;\n        if let I::TailCall(_) | I::TailCallMethod(_) = instruction {\n            enqueue_fallthrough(\n                &mut worklist,\n                pc,\n                VerificationState {\n                    depth: next_depth,\n                    regions: next_regions.clone(),\n                    return_addresses: next_return_addresses.clone(),\n                    super_call_bases: next_super_call_bases.clone(),\n                },\n                code.len(),\n            )?;\n        }\n        // QuickJS `compute_stack_size` stops as soon as a reachable PC crosses'
expect_full_rewrite_rejected stage3c-call-arguments-shadow \
    stage3c-tail-vm src/engine/vm/mod.rs \
    $'    ) -> Result<Vec<Value>, Error> {\n        let argument_count = usize::from(argument_count);' \
    $'    ) -> Result<Vec<Value>, Error> {\n        let argument_count = 0;\n        let argument_count = usize::from(argument_count);'
expect_full_rewrite_rejected stage3c-call-arguments-drop \
    stage3c-tail-vm src/engine/vm/mod.rs \
    $'        let start = self.stack.len() - argument_count;\n        Ok(self.stack.split_off(start))' \
    $'        let start = self.stack.len() - argument_count;\n        let mut arguments = self.stack.split_off(start);\n        arguments.pop();\n        Ok(arguments)'
expect_full_rewrite_rejected stage3c-call-dispatch-alias-bypass \
    stage3c-tail-vm src/engine/vm/mod.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        let completion = match instruction {\n            Instruction::Import => {' \
    $'    ) -> Result<Option<Completion>, Error> {\n        use Instruction as I;\n        let completion = match instruction {\n            I::TailCall(argument_count) if *argument_count == 0 => {\n                let _ = self.pop()?;\n                return host.call(Value::Undefined, Value::Null, Vec::new()).map(Some);\n            }\n            Instruction::Import => {'
expect_full_rewrite_rejected stage3c-execute-inner-tail-intercept \
    stage3c-tail-vm src/engine/vm/mod.rs \
    $'            if matches!(\n                instruction,\n                Instruction::Import\n                    | Instruction::Call(_)' \
    $'            use Instruction as I;\n            if matches!(instruction, I::TailCall(_) | I::TailCallMethod(_)) {\n                return Ok(InterpreterExit::Complete(Completion::Return(Value::Undefined)));\n            }\n\n            if matches!(\n                instruction,\n                Instruction::Import\n                    | Instruction::Call(_)'
expect_full_rewrite_rejected stage3c-execute-alias-return-bypass \
    stage3c-tail-completion src/engine/vm/mod.rs \
    $'    ) -> Result<Completion, Error> {\n        loop {\n            let raised = match self.execute_inner(code, host) {' \
    $'    ) -> Result<Completion, Error> {\n        use Completion as C;\n        loop {\n            let raised = match self.execute_inner(code, host) {\n                Ok(InterpreterExit::Complete(C::Return(value))) if matches!(&value, Value::Undefined) => {\n                    self.pc = self.pc.saturating_add(1);\n                    continue;\n                }'
expect_full_rewrite_rejected stage3c-run-throw-bypass \
    stage3c-tail-completion src/engine/vm/mod.rs \
    $'                Ok(InterpreterExit::Complete(Completion::Return(value))) => {\n                    return Ok(VmExit::Complete(Completion::Return(value)));\n                }\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,' \
    $'                Ok(InterpreterExit::Complete(Completion::Return(value))) => {\n                    return Ok(VmExit::Complete(Completion::Return(value)));\n                }\n                Ok(InterpreterExit::Complete(Completion::Throw(value)))\n                    if matches!(&value, Value::Undefined) => {\n                        return Ok(VmExit::Complete(Completion::Throw(value)));\n                    }\n                Ok(InterpreterExit::Complete(Completion::Throw(value))) => value,'
expect_full_rewrite_rejected stage3c-raise-guarded-bypass \
    stage3c-tail-completion src/engine/vm/mod.rs \
    $'    ) -> Result<Option<Completion>, Error> {\n        host.ensure_backtrace(&value)?;\n        loop {' \
    $'    ) -> Result<Option<Completion>, Error> {\n        if matches!(value, Value::Undefined) {\n            return Ok(Some(Completion::Throw(value)));\n        }\n        host.ensure_backtrace(&value)?;\n        loop {'
expect_full_rewrite_rejected stage3c-required-module-cfg-excluded \
    stage3c-runtime-evidence src/engine/vm/mod.rs \
    $'#[cfg(test)]\nmod tests;' \
    $'#[cfg(any())]\n#[cfg(test)]\nmod tests;'
expect_full_rewrite_rejected stage3c-required-module-inner-cfg-excluded \
    stage3c-runtime-evidence src/engine/heap/runtime/tests.rs \
    'use super::{' \
    $'#![cfg(any())]\n\nuse super::{'
expect_full_rewrite_rejected stage3c-required-test-macro-shadow \
    stage3c-runtime-evidence src/engine/heap/runtime/tests.rs \
    $'fn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {\n    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);' \
    $'fn trusted_quickjs_ordinary_tail_invocations_use_exact_bc5_wires_and_semantics() {\n    macro_rules! assert_eq { ($($tokens:tt)*) => {}; }\n    assert_eq!(QUICKJS_ORDINARY_TAIL_CALL_BC5.len(), 57);'
