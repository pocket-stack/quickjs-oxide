# Both S08's full-state verifier and S09's compact storage must reject a
# publication bypass. Their existing Stage3C-H terminal/fallthrough canaries
# still apply to the same shared instruction corridor.
if [[ $(<"$repository_root/src/engine/code/bytecode.rs") == *"fn verify_parts_with_visits"* ]]; then
    expect_full_rewrite_rejected primitive-verifier-wrapper-bypass \
        published-verifier-kernel src/engine/code/bytecode.rs \
        'verify_parts_with_visits::<CompactVisits>(code, constant_count, declared_max_stack)' \
        'Ok(VerifiedBytecode { max_stack: 0 })'
    expect_full_rewrite_rejected primitive-verifier-join-bypass \
        published-verifier-kernel src/engine/code/bytecode.rs \
        'if !states.visit(pc, &state)? {' 'if false {'
    expect_full_rewrite_rejected primitive-verifier-compact-depth-bypass \
        published-verifier-kernel src/engine/code/bytecode.rs \
        'if *depth != state.depth {' 'if false {'
    expect_full_rewrite_rejected primitive-verifier-compact-history-loss \
        published-verifier-kernel src/engine/code/bytecode.rs \
        'states[pc] = Some(state.clone());' 'states[pc] = None;'
else
    expect_full_rewrite_rejected primitive-verifier-full-join-bypass \
        published-verifier-kernel src/engine/code/bytecode.rs \
        'if previous != &state {' 'if false {'
    expect_full_rewrite_rejected primitive-verifier-full-history-loss \
        published-verifier-kernel src/engine/code/bytecode.rs \
        '*slot = Some(state.clone());' '*slot = None;'
fi
expect_full_rewrite_rejected primitive-verifier-fifo-reordered \
    published-verifier-kernel src/engine/code/bytecode.rs \
    'worklist.pop_front()' 'worklist.pop_back()'
expect_full_rewrite_rejected primitive-set-domain-bypass \
    ordinary-property-contract src/engine/object/ordinary/set.rs \
    'runtime.validate_value_domain(value,' 'runtime.skip_value_domain(value,'
expect_full_rewrite_rejected primitive-set-borrowed-entry-bypass \
    ordinary-property-contract src/engine/object/ordinary/set.rs \
    'initial_set(runtime, Some(realm), object, key, &value, &receiver)' \
    'unchecked_set(runtime, Some(realm), object, key, &value, &receiver)'
expect_full_rewrite_rejected primitive-read-wrong-receiver \
    ordinary-property-contract src/engine/object/ordinary.rs \
    'self.prepare_ordinary_read_borrowed(object, key, &receiver)' \
    'self.prepare_ordinary_read_borrowed(object, key, &Value::Undefined)'
expect_full_rewrite_rejected primitive-for-in-unguarded-sync \
    for-in-local-guard src/engine/vm/for_in/operation.rs \
    'Self::Keys { object, resume } if !runtime.is_proxy_object(&object)? =>' \
    'Self::Keys { object, resume } =>'
expect_full_rewrite_rejected primitive-for-in-wrong-target \
    for-in-local-guard src/engine/vm/for_in/operation.rs \
    'runtime.internal_has_own_property(resume.realm, &object, &key)?' \
    'runtime.internal_has_own_property(resume.realm, &other, &key)?'
expect_full_rewrite_rejected primitive-for-in-helper-callback \
    for-in-local-dependency src/engine/object/internal_methods.rs \
    'return Ok(NativeConversion::Value(flags.is_some()));' \
    'self.call_internal(); return Ok(NativeConversion::Value(flags.is_some()));'
expect_full_rewrite_rejected primitive-for-in-namespace-callback \
    for-in-local-dependency src/engine/modules/namespace.rs \
    'let mut atoms = Vec::new();' \
    'self.call_internal(); let mut atoms = Vec::new();'
expect_full_rewrite_rejected primitive-numeric-parent-disconnected \
    synchronous-domain-route src/engine/vm/frame_operations.rs \
    'complete_numeric(runtime, execution, id, kind)' \
    'legacy_numeric(runtime, execution, id, kind)'
expect_full_rewrite_rejected primitive-numeric-child-disconnected \
    synchronous-domain-route src/engine/vm/frame_operations/numeric.rs \
    'proxy_get_driver::start_numeric(runtime, execution, id, step, depth)' \
    'proxy_get_driver::legacy_numeric(runtime, execution, id, step, depth)'
expect_full_rewrite_rejected primitive-numeric-child-callback \
    synchronous-domain-route src/engine/vm/frame_operations/numeric.rs \
    'let right = execution.slots.pop(&mut frame.window)?;' \
    'runtime.to_primitive(); let right = execution.slots.pop(&mut frame.window)?;'

# S11 moved the selected effect fields into resident owners; the same guard,
# exact receiver and driver routing remain mandatory.
expect_full_rewrite_rejected primitive-mutation-resident-unguarded-delete \
    array-mutation-local-guard src/engine/builtins/array/mutation.rs \
    'MutationAction::Delete(key) if !runtime.is_proxy_object(&self.0.object)? =>' \
    'MutationAction::Delete(key) =>'
expect_full_rewrite_rejected primitive-mutation-resident-wrong-receiver \
    array-mutation-local-guard src/engine/builtins/array/mutation.rs \
    'runtime.internal_delete_property(self.0.realm, &self.0.object, &key)?' \
    'runtime.internal_delete_property(self.0.realm, &other, &key)?'
expect_full_rewrite_rejected primitive-cold-numeric-disconnected \
    synchronous-domain-route src/engine/vm/driver/cold.rs \
    'super::super::frame_operations::numeric(' \
    'super::super::frame_operations::legacy_numeric('
expect_full_rewrite_rejected primitive-environment-resident-wrong-receiver \
    synchronous-domain-route src/engine/vm/proxy_get_driver/request/vm.rs \
    'let receiver = resume.take_read_receiver();' \
    'let receiver = Value::Undefined;'
