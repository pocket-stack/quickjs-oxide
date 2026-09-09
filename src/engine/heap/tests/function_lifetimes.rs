use super::*;
use crate::engine::heap::native::NativeCProto;

#[test]
fn two_closures_share_one_mutable_var_ref_cell() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let bytecode = heap
        .allocate_function_bytecode(closure_bytecode(&code, context, 1))
        .unwrap();
    let cell = heap
        .allocate_var_ref(VarRefData::local(RawValue::Int(1)))
        .unwrap();
    let first = heap
        .allocate_object(ObjectData::bytecode_function_with_closures(
            shape,
            Vec::new(),
            bytecode,
            None,
            vec![cell],
            false,
        ))
        .unwrap();
    let second = heap
        .allocate_object(ObjectData::bytecode_function_with_closures(
            shape,
            Vec::new(),
            bytecode,
            None,
            vec![cell],
            false,
        ))
        .unwrap();

    assert_eq!(heap.var_ref_strong_count(cell), Ok(3));
    for function in [first, second] {
        let ObjectPayload::BytecodeFunction { closure_slots, .. } =
            &heap.object(function).unwrap().payload
        else {
            panic!("expected a bytecode function payload");
        };
        assert_eq!(closure_slots, &[cell]);
    }

    assert_eq!(
        heap.replace_var_ref_value(cell, RawValue::Int(9)).unwrap(),
        HeapCleanup::default()
    );
    assert_eq!(heap.var_ref(cell).unwrap().value, RawValue::Int(9));

    assert_eq!(heap.release_var_ref(cell).unwrap(), HeapCleanup::default());
    assert_eq!(heap.release_object(first).unwrap().finalized_objects, 1);
    let cleanup = heap.release_object(second).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_var_refs, 1);

    heap.release_shape(shape).unwrap();
    heap.release_function_bytecode(bytecode).unwrap();
    let cleanup = heap.release_context(context).unwrap();
    assert_eq!(cleanup.finalized_contexts, 1);
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn bytecode_home_object_replacement_is_retain_first_and_idempotent() {
    let mut heap = Heap::new();
    assert!(!FunctionMetadata::default().needs_home_object);

    let empty = empty_shape(&mut heap);
    let realm_root = leaf(&mut heap, empty);
    let context = heap
        .allocate_context(ContextData::new(
            realm_root, realm_root, realm_root, realm_root, realm_root, realm_root, realm_root,
            realm_root,
        ))
        .unwrap();
    heap.release_object(realm_root).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let bytecode = heap
        .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
        .unwrap();
    let function = heap
        .allocate_object(ObjectData::bytecode_function(
            empty,
            Vec::new(),
            bytecode,
            None,
            false,
        ))
        .unwrap();
    assert_eq!(heap.bytecode_function_home_object(function), Ok(None));
    assert_eq!(
        heap.bytecode_function_home_object(realm_root),
        Err(HeapError::Invariant(
            "HomeObject lookup reached a non-bytecode function"
        ))
    );

    let replacement = leaf(&mut heap, empty);
    let owner_shape = one_slot_shape(&mut heap);
    let previous = heap
        .allocate_object(ObjectData::ordinary(
            owner_shape,
            vec![PropertySlot::Data(RawValue::Object(replacement))],
        ))
        .unwrap();
    assert_eq!(heap.object_strong_count(replacement), Ok(2));

    assert_eq!(
        heap.replace_bytecode_function_home_object(function, Some(previous))
            .unwrap(),
        HeapCleanup::default()
    );
    heap.release_object(previous).unwrap();
    heap.release_object(replacement).unwrap();
    assert_eq!(heap.object_strong_count(previous), Ok(1));
    assert_eq!(heap.object_strong_count(replacement), Ok(1));

    // `previous` owns `replacement`. Retaining the new edge first keeps it
    // live while detaching and finalizing the old HomeObject.
    let cleanup = heap
        .replace_bytecode_function_home_object(function, Some(replacement))
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(
        heap.bytecode_function_home_object(function),
        Ok(Some(replacement))
    );
    assert_eq!(heap.object_strong_count(replacement), Ok(1));

    assert_eq!(
        heap.replace_bytecode_function_home_object(function, Some(replacement))
            .unwrap(),
        HeapCleanup::default()
    );
    assert_eq!(heap.object_strong_count(replacement), Ok(1));

    let cleanup = heap
        .replace_bytecode_function_home_object(function, None)
        .unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(heap.bytecode_function_home_object(function), Ok(None));
    assert_eq!(
        heap.replace_bytecode_function_home_object(function, None)
            .unwrap(),
        HeapCleanup::default()
    );

    assert_eq!(heap.release_object(function).unwrap().finalized_objects, 1);
    heap.release_function_bytecode(bytecode).unwrap();
    heap.release_context(context).unwrap();
    heap.release_shape(owner_shape).unwrap();
    heap.release_shape(empty).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn bytecode_home_object_edge_participates_in_cycle_collection() {
    let mut heap = Heap::new();
    let empty = empty_shape(&mut heap);
    let realm_root = leaf(&mut heap, empty);
    let context = heap
        .allocate_context(ContextData::new(
            realm_root, realm_root, realm_root, realm_root, realm_root, realm_root, realm_root,
            realm_root,
        ))
        .unwrap();
    heap.release_object(realm_root).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let bytecode = heap
        .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
        .unwrap();
    let function = heap
        .allocate_object(ObjectData::bytecode_function(
            empty,
            Vec::new(),
            bytecode,
            None,
            false,
        ))
        .unwrap();
    let home_shape = one_slot_shape(&mut heap);
    let home_object = heap
        .allocate_object(ObjectData::ordinary(
            home_shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();

    heap.replace_bytecode_function_home_object(function, Some(home_object))
        .unwrap();
    heap.replace_object_slot(
        home_object,
        0,
        PropertySlot::Data(RawValue::Object(function)),
    )
    .unwrap();
    heap.release_object(function).unwrap();
    heap.release_object(home_object).unwrap();

    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.candidate_nodes, 2);
    assert_eq!(stats.cleanup.finalized_objects, 2);

    heap.release_function_bytecode(bytecode).unwrap();
    heap.release_context(context).unwrap();
    heap.release_shape(home_shape).unwrap();
    heap.release_shape(empty).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn autoinit_property_slot_retains_and_releases_its_creation_realm() {
    let mut heap = Heap::new();
    let empty = empty_shape(&mut heap);
    let property_shape = one_slot_shape(&mut heap);
    let prototype = leaf(&mut heap, empty);
    let realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    assert_eq!(heap.context_strong_count(realm), Ok(1));

    let function = heap
        .allocate_object(ObjectData::ordinary(
            property_shape,
            vec![PropertySlot::AutoInit(
                AutoInitProperty::FunctionPrototype { realm },
            )],
        ))
        .unwrap();
    assert_eq!(heap.context_strong_count(realm), Ok(2));

    heap.replace_object_slot(function, 0, PropertySlot::Data(RawValue::Undefined))
        .unwrap();
    assert_eq!(heap.context_strong_count(realm), Ok(1));

    heap.release_object(function).unwrap();
    heap.release_context(realm).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(property_shape).unwrap();
    heap.release_shape(empty).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn function_var_ref_object_cycle_is_collected() {
    let mut heap = Heap::new();
    let empty = empty_shape(&mut heap);
    let data = one_slot_shape(&mut heap);
    let prototype = leaf(&mut heap, empty);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let bytecode = heap
        .allocate_function_bytecode(closure_bytecode(&code, context, 1))
        .unwrap();
    let captured = heap
        .allocate_object(ObjectData::ordinary(
            data,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    let cell = heap
        .allocate_var_ref(VarRefData::local(RawValue::Object(captured)))
        .unwrap();
    let function = heap
        .allocate_object(ObjectData::bytecode_function_with_closures(
            empty,
            Vec::new(),
            bytecode,
            None,
            vec![cell],
            false,
        ))
        .unwrap();
    heap.replace_object_slot(captured, 0, PropertySlot::Data(RawValue::Object(function)))
        .unwrap();

    heap.release_shape(empty).unwrap();
    heap.release_shape(data).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_context(context).unwrap();
    heap.release_function_bytecode(bytecode).unwrap();
    heap.release_object(captured).unwrap();
    heap.release_var_ref(cell).unwrap();
    heap.release_object(function).unwrap();

    assert_eq!(heap.counts().live, 8);
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.candidate_nodes, 8);
    assert_eq!(stats.cleanup.finalized_objects, 3);
    assert_eq!(stats.cleanup.finalized_shapes, 2);
    assert_eq!(stats.cleanup.finalized_var_refs, 1);
    assert_eq!(stats.cleanup.finalized_contexts, 1);
    assert_eq!(stats.cleanup.finalized_function_bytecodes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn fifty_thousand_closure_cells_finalize_iteratively() {
    const LENGTH: usize = 50_000;

    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();
    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let bytecode = heap
        .allocate_function_bytecode(closure_bytecode(&code, context, 1))
        .unwrap();

    let mut head = None;
    for _ in 0..LENGTH {
        let value = head.map_or(RawValue::Undefined, RawValue::Object);
        let cell = heap.allocate_var_ref(VarRefData::local(value)).unwrap();
        let function = heap
            .allocate_object(ObjectData::bytecode_function_with_closures(
                shape,
                Vec::new(),
                bytecode,
                None,
                vec![cell],
                false,
            ))
            .unwrap();
        heap.release_var_ref(cell).unwrap();
        if let Some(previous) = head {
            assert_eq!(
                heap.release_object(previous).unwrap(),
                HeapCleanup::default()
            );
        }
        head = Some(function);
    }

    heap.release_shape(shape).unwrap();
    let cleanup = heap.release_object(head.unwrap()).unwrap();
    assert_eq!(cleanup.finalized_objects, LENGTH);
    assert_eq!(cleanup.finalized_var_refs, LENGTH);
    assert_eq!(heap.counts().object_nodes, 1);
    assert_eq!(heap.counts().var_ref_nodes, 0);

    heap.release_function_bytecode(bytecode).unwrap();
    let cleanup = heap.release_context(context).unwrap();
    assert_eq!(cleanup.finalized_contexts, 1);
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn context_object_bytecode_realm_cycle_is_collected() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let prototype = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
        ))
        .unwrap();
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let function_bytecode = heap
        .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
        .unwrap();
    let function = heap
        .allocate_object(ObjectData::bytecode_function(
            shape,
            vec![PropertySlot::Data(RawValue::Undefined)],
            function_bytecode,
            None,
            false,
        ))
        .unwrap();
    heap.replace_object_slot(prototype, 0, PropertySlot::Data(RawValue::Object(function)))
        .unwrap();

    heap.release_shape(shape).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_context(context).unwrap();
    heap.release_function_bytecode(function_bytecode).unwrap();
    heap.release_object(function).unwrap();

    assert_eq!(heap.counts().live, 5);
    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.candidate_nodes, 5);
    assert_eq!(stats.cleanup.finalized_objects, 2);
    assert_eq!(stats.cleanup.finalized_shapes, 1);
    assert_eq!(stats.cleanup.finalized_contexts, 1);
    assert_eq!(stats.cleanup.finalized_function_bytecodes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn bytecode_constant_pool_owns_child_and_returns_all_atoms() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(shape).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let child_atom = Atom::from_raw(41);
    let parent_atom = Atom::from_raw(42);
    let symbol_atom = Atom::from_raw(43);
    let child = heap
        .allocate_function_bytecode(bytecode(&code, context, Vec::new(), vec![child_atom]))
        .unwrap();
    let parent = heap
        .allocate_function_bytecode(bytecode(
            &code,
            context,
            vec![
                BytecodeConstant::Function(child),
                BytecodeConstant::Value(RawValue::Symbol(symbol_atom)),
            ],
            vec![parent_atom],
        ))
        .unwrap();
    assert_eq!(heap.function_bytecode_strong_count(child), Ok(2));

    assert_eq!(
        heap.release_function_bytecode(child).unwrap(),
        HeapCleanup::default()
    );
    let mut cleanup = heap.release_function_bytecode(parent).unwrap();
    assert_eq!(cleanup.finalized_function_bytecodes, 2);
    cleanup.atoms.sort_unstable();
    let mut expected = vec![child_atom, parent_atom, symbol_atom];
    expected.sort_unstable();
    assert_eq!(cleanup.atoms, expected);

    let context_cleanup = heap.release_context(context).unwrap();
    assert_eq!(context_cleanup.finalized_contexts, 1);
    assert_eq!(context_cleanup.finalized_objects, 1);
    assert_eq!(context_cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn bytecode_regexp_constant_releases_its_rc_leaf_on_finalization() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(shape).unwrap();

    let pattern = JsString::from_static("a");
    let flags = JsString::from_static("g");
    let program = Rc::new(crate::regexp::compile(&pattern, &flags).unwrap());
    let weak = Rc::downgrade(&program);
    let code: Rc<[Instruction]> = Rc::from([Instruction::RegExp(0), Instruction::Return]);
    let function = heap
        .allocate_function_bytecode(bytecode(
            &code,
            context,
            vec![BytecodeConstant::RegExp { pattern, program }],
            Vec::new(),
        ))
        .unwrap();
    assert_eq!(weak.strong_count(), 1);

    let cleanup = heap.release_function_bytecode(function).unwrap();
    assert_eq!(cleanup.finalized_function_bytecodes, 1);
    assert_eq!(weak.strong_count(), 0);

    let context_cleanup = heap.release_context(context).unwrap();
    assert_eq!(context_cleanup.finalized_contexts, 1);
    assert_eq!(context_cleanup.finalized_objects, 1);
    assert_eq!(context_cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn async_function_intrinsic_root_is_a_function_prototype_child() {
    let mut heap = Heap::new();
    let base_shape = empty_shape(&mut heap);
    let function_prototype = leaf(&mut heap, base_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            function_prototype,
            function_prototype,
            function_prototype,
            function_prototype,
            function_prototype,
            function_prototype,
            function_prototype,
            function_prototype,
        ))
        .unwrap();
    let child_shape = heap
        .allocate_shape(Shape::new(Some(function_prototype), []).unwrap())
        .unwrap();
    let async_function_prototype = leaf(&mut heap, child_shape);
    let wrong_prototype = leaf(&mut heap, base_shape);

    assert_eq!(
        heap.attach_async_function_intrinsics(
            realm,
            AsyncFunctionRealmData {
                function_prototype: wrong_prototype,
            },
        ),
        Err(HeapError::Invariant(
            "AsyncFunction prototype does not inherit from Function.prototype"
        ))
    );
    assert_eq!(heap.context(realm).unwrap().async_function, None);

    let roots = AsyncFunctionRealmData {
        function_prototype: async_function_prototype,
    };
    let before = heap.object_strong_count(async_function_prototype).unwrap();
    heap.attach_async_function_intrinsics(realm, roots).unwrap();
    assert_eq!(heap.context(realm).unwrap().async_function, Some(roots));
    assert_eq!(
        heap.object_strong_count(async_function_prototype),
        Ok(before + 1)
    );
    assert_eq!(
        heap.attach_async_function_intrinsics(realm, roots),
        Err(HeapError::Invariant(
            "context already has AsyncFunction intrinsic roots"
        ))
    );
    assert_eq!(
        heap.object_strong_count(async_function_prototype),
        Ok(before + 1)
    );
    assert!(
        context_edges(heap.context(realm).unwrap())
            .contains(&RawId::Object(async_function_prototype))
    );

    heap.release_context(realm).unwrap();
    heap.release_object(async_function_prototype).unwrap();
    heap.release_object(wrong_prototype).unwrap();
    heap.release_shape(child_shape).unwrap();
    heap.release_object(function_prototype).unwrap();
    heap.release_shape(base_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn async_function_state_traces_callbacks_and_transfers_await_activation() {
    let mut heap = Heap::new();
    let base_shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, base_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let callee_realm = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    let function_shape = heap
        .allocate_shape(Shape::new(Some(prototype), []).unwrap())
        .unwrap();
    let outer_resolve = heap
        .allocate_object(ObjectData::bound_native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    let outer_reject = heap
        .allocate_object(ObjectData::bound_native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();

    let ordinary = leaf(&mut heap, base_shape);
    assert_eq!(
        heap.allocate_object(ObjectData::async_function_state(
            base_shape,
            Vec::new(),
            realm,
            ordinary,
            outer_reject,
        )),
        Err(HeapError::Invariant(
            "AsyncFunction state retains a non-callable resolving function"
        ))
    );
    let mut inconsistent_state = ObjectData::async_function_state(
        base_shape,
        Vec::new(),
        realm,
        outer_resolve,
        outer_reject,
    );
    let ObjectPayload::AsyncFunctionState(inconsistent_data) = &mut inconsistent_state.payload
    else {
        unreachable!("constructor publishes AsyncFunction state payload")
    };
    inconsistent_data.phase = AsyncFunctionPhase::Awaiting;
    assert_eq!(
        heap.allocate_object(inconsistent_state),
        Err(HeapError::Invariant(
            "AsyncFunction state has inconsistent phase and activation"
        ))
    );

    let state = heap
        .allocate_object(ObjectData::async_function_state(
            base_shape,
            Vec::new(),
            realm,
            outer_resolve,
            outer_reject,
        ))
        .unwrap();
    assert_eq!(
        object_edges(heap.object(state).unwrap()),
        [
            RawId::Shape(base_shape),
            RawId::Context(realm),
            RawId::Object(outer_resolve),
            RawId::Object(outer_reject),
        ]
    );
    assert_eq!(
        heap.async_function_state_snapshot(state).unwrap().phase,
        AsyncFunctionPhase::Executing
    );
    assert_eq!(
        heap.begin_async_function_resume(state),
        Err(HeapError::Invariant(
            "AsyncFunction resume began outside an awaiting phase"
        ))
    );

    let callback = heap
        .allocate_object(ObjectData::bound_internal_native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::AsyncFunctionResume(AsyncFunctionResumeKind::Fulfill),
            realm,
            1,
            InternalCallableData::AsyncFunctionResume {
                state,
                kind: AsyncFunctionResumeKind::Fulfill,
            },
        ))
        .unwrap();
    assert!(
        NativeFunctionId::AsyncFunctionResume(AsyncFunctionResumeKind::Reject).uses_calling_realm()
    );
    assert_eq!(
        NativeFunctionId::AsyncFunctionResume(AsyncFunctionResumeKind::Fulfill)
            .descriptor()
            .cproto,
        NativeCProto::Generic
    );
    assert!(!heap.object(callback).unwrap().is_constructor);
    assert_eq!(
        heap.allocate_object(ObjectData::bound_internal_native_function(
            function_shape,
            Vec::new(),
            NativeFunctionId::AsyncFunctionResume(AsyncFunctionResumeKind::Reject),
            realm,
            1,
            InternalCallableData::AsyncFunctionResume {
                state,
                kind: AsyncFunctionResumeKind::Fulfill,
            },
        )),
        Err(HeapError::Invariant(
            "native target does not match its internal callable capture"
        ))
    );

    let code: Rc<[Instruction]> = Rc::from([
        Instruction::Undefined,
        Instruction::Await,
        Instruction::Return,
    ]);
    assert_eq!(
        heap.allocate_function_bytecode(bytecode(&code, callee_realm, Vec::new(), Vec::new(),)),
        Err(HeapError::Invariant(
            "non-async bytecode contains a suspension opcode"
        ))
    );
    let mut malformed_async = bytecode(&code, callee_realm, Vec::new(), Vec::new());
    malformed_async.metadata.function_kind = FunctionKind::Async;
    malformed_async.metadata.has_prototype = true;
    assert_eq!(
        heap.allocate_function_bytecode(malformed_async),
        Err(HeapError::Invariant(
            "async bytecode has invalid execution metadata"
        ))
    );
    let mut async_bytecode = bytecode(&code, callee_realm, Vec::new(), Vec::new());
    async_bytecode.metadata.function_kind = FunctionKind::Async;
    let bytecode = heap.allocate_function_bytecode(async_bytecode).unwrap();
    let function = heap
        .allocate_object(ObjectData::bytecode_function(
            function_shape,
            Vec::new(),
            bytecode,
            None,
            false,
        ))
        .unwrap();
    let awaited_atom = Atom::from_raw(8_071);
    let activation = GeneratorActivationData {
        bytecode,
        vm: GeneratorVmActivation {
            stack: vec![RawValue::Symbol(awaited_atom)],
            regions: Vec::new(),
            pc: 2,
            callee_realm,
            current_function: function,
            this_value: RawValue::Undefined,
            normalized_this: None,
            new_target: RawValue::Undefined,
            strict: false,
            callee_global: prototype,
        },
        actual_argument_count: 0,
        arguments: Vec::new(),
        locals: Vec::new(),
        reusable_captured_locals: Vec::new(),
    };

    let mut invalid_activation = activation.clone();
    invalid_activation.vm.pc = 1;
    assert_eq!(
        heap.suspend_async_function(state, invalid_activation),
        Err(HeapError::Invariant(
            "AsyncFunction activation is not parked after its await opcode"
        ))
    );
    assert_eq!(
        heap.async_function_state_snapshot(state).unwrap().phase,
        AsyncFunctionPhase::Executing
    );

    heap.suspend_async_function(state, activation.clone())
        .unwrap();
    assert_eq!(
        heap.async_function_state_snapshot(state).unwrap().phase,
        AsyncFunctionPhase::Awaiting
    );
    assert_eq!(
        object_atoms(heap.object(state).unwrap()).collect::<Vec<_>>(),
        [awaited_atom]
    );
    assert_eq!(
        heap.suspend_async_function(state, activation.clone()),
        Err(HeapError::Invariant(
            "AsyncFunction suspension did not follow an executing phase"
        ))
    );
    let (resumed, cleanup) = heap.begin_async_function_resume(state).unwrap();
    assert_eq!(resumed, activation);
    assert_eq!(cleanup.atoms, [awaited_atom]);
    assert_eq!(
        heap.async_function_state_snapshot(state).unwrap().phase,
        AsyncFunctionPhase::Executing
    );
    assert_eq!(
        heap.complete_async_function(state),
        Ok(HeapCleanup::default())
    );
    assert_eq!(
        heap.complete_async_function(state),
        Err(HeapError::Invariant(
            "AsyncFunction completion reached an inconsistent phase"
        ))
    );

    let awaiting_state = heap
        .allocate_object(ObjectData::async_function_state(
            base_shape,
            Vec::new(),
            realm,
            outer_resolve,
            outer_reject,
        ))
        .unwrap();
    heap.suspend_async_function(awaiting_state, activation.clone())
        .unwrap();
    assert_eq!(
        heap.complete_async_function(awaiting_state).unwrap().atoms,
        [awaited_atom]
    );
    assert_eq!(
        heap.async_function_state_snapshot(awaiting_state)
            .unwrap()
            .phase,
        AsyncFunctionPhase::Completed
    );

    let promise = heap
        .allocate_object(ObjectData::promise(base_shape, Vec::new()))
        .unwrap();
    let callback_count = heap.object_strong_count(callback).unwrap();
    heap.promise_add_reactions(
        promise,
        PromiseReaction {
            kind: PromiseReactionKind::Fulfill,
            handler: Some(callback),
            capability: None,
        },
        PromiseReaction {
            kind: PromiseReactionKind::Reject,
            handler: Some(callback),
            capability: None,
        },
    )
    .unwrap();
    assert_eq!(heap.object_strong_count(callback), Ok(callback_count + 2));
    heap.promise_settle(promise, PromiseState::Fulfilled, RawValue::Undefined)
        .unwrap();
    assert_eq!(heap.object_strong_count(callback), Ok(callback_count));

    assert_eq!(
        heap.complete_async_function(ordinary),
        Err(HeapError::Invariant(
            "AsyncFunction completion reached an object with the wrong class"
        ))
    );

    heap.release_object(promise).unwrap();
    heap.release_object(callback).unwrap();
    heap.release_object(awaiting_state).unwrap();
    heap.release_object(state).unwrap();
    heap.release_object(function).unwrap();
    heap.release_object(outer_reject).unwrap();
    heap.release_object(outer_resolve).unwrap();
    heap.release_object(ordinary).unwrap();
    heap.release_function_bytecode(bytecode).unwrap();
    heap.release_context(callee_realm).unwrap();
    heap.release_context(realm).unwrap();
    heap.release_shape(function_shape).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(base_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn long_bytecode_constant_chain_finalizes_iteratively() {
    const LENGTH: usize = 50_000;

    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let prototype = leaf(&mut heap, shape);
    let context = heap
        .allocate_context(ContextData::new(
            prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
        ))
        .unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_shape(shape).unwrap();

    let code: Rc<[Instruction]> = Rc::from([Instruction::Undefined, Instruction::Return]);
    let mut head = heap
        .allocate_function_bytecode(bytecode(&code, context, Vec::new(), Vec::new()))
        .unwrap();
    for _ in 1..LENGTH {
        let next = heap
            .allocate_function_bytecode(bytecode(
                &code,
                context,
                vec![BytecodeConstant::Function(head)],
                Vec::new(),
            ))
            .unwrap();
        assert_eq!(
            heap.release_function_bytecode(head).unwrap(),
            HeapCleanup::default()
        );
        head = next;
    }

    let cleanup = heap.release_function_bytecode(head).unwrap();
    assert_eq!(cleanup.finalized_function_bytecodes, LENGTH);
    assert_eq!(heap.counts().function_bytecode_nodes, 0);
    let cleanup = heap.release_context(context).unwrap();
    assert_eq!(cleanup.finalized_contexts, 1);
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
}
