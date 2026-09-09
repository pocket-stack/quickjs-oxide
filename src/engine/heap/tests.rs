use super::*;
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::source::LineColumn;

// Heap-only fixtures have no AtomTable; symbol liveness stays with each
// fixture, and detached atom ownership is returned in the cleanup record.
fn collect_heap(heap: &mut Heap) -> Result<GcStats, HeapError> {
    heap.run_gc_with_finalization_sink(
        |event| Ok(matches!(event, WeakSymbolGcEvent::IsLive(_))),
        &mut gc::DiscardFinalizationJobSink,
    )
}

const DATA_FLAGS: PropertyFlags = PropertyFlags::data(true, true, true);

#[test]
fn multiset_difference_preserves_left_order_and_occurrence_counts() {
    assert_eq!(
        multiset_difference(
            &[1u8, 2, 1, 3, 2, 1],
            &[2u8, 1, 4, 2],
            "testing multiset difference",
        )
        .unwrap(),
        vec![1, 3, 1]
    );
}

fn empty_shape(heap: &mut Heap) -> ShapeId {
    heap.allocate_shape(Shape::new(None, []).unwrap()).unwrap()
}

#[derive(Default)]
struct RecordingFinalizationJobSink {
    jobs: VecDeque<PreparedFinalizationJob>,
    fail_next_reservation: bool,
}

impl FinalizationJobSink for RecordingFinalizationJobSink {
    fn try_reserve_one(&mut self) -> bool {
        if std::mem::take(&mut self.fail_next_reservation) {
            return false;
        }
        self.jobs.try_reserve(1).is_ok()
    }

    fn publish_preowned(&mut self, job: PreparedFinalizationJob) {
        self.jobs.push_back(job);
    }
}

fn finalization_test_realm(
    heap: &mut Heap,
    shape: ShapeId,
) -> (ObjectId, ObjectId, ContextId, ObjectId) {
    let root = leaf(heap, shape);
    let function_prototype = heap
        .allocate_bootstrap_native_function(ObjectData::native_function(
            shape,
            Vec::new(),
            NativeFunctionId::FunctionPrototype,
            0,
        ))
        .unwrap();
    let realm = heap
        .allocate_context(ContextData::new(
            root,
            function_prototype,
            root,
            root,
            root,
            root,
            root,
            root,
        ))
        .unwrap();
    heap.attach_native_function_realm(function_prototype, realm)
        .unwrap();
    let callback = heap
        .allocate_object(ObjectData::bound_native_function(
            shape,
            Vec::new(),
            NativeFunctionId::ErrorIsError,
            realm,
            1,
        ))
        .unwrap();
    (root, function_prototype, realm, callback)
}

fn release_finalization_test_realm(
    heap: &mut Heap,
    shape: ShapeId,
    root: ObjectId,
    function_prototype: ObjectId,
    realm: ContextId,
    callback: ObjectId,
) {
    heap.release_object(callback).unwrap();
    heap.release_context(realm).unwrap();
    heap.release_object(function_prototype).unwrap();
    heap.release_object(root).unwrap();
    heap.release_shape(shape).unwrap();
    heap.run_gc_for_runtime_teardown().unwrap();
    assert_eq!(heap.counts().live, 0);
}

fn one_slot_shape(heap: &mut Heap) -> ShapeId {
    let atom = Atom::from_immediate_integer(0).unwrap();
    heap.allocate_shape(
        Shape::new(
            None,
            [ShapeEntry {
                atom,
                flags: DATA_FLAGS,
            }],
        )
        .unwrap(),
    )
    .unwrap()
}

fn leaf(heap: &mut Heap, shape: ShapeId) -> ObjectId {
    heap.allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap()
}

fn bytecode(
    code: &Rc<[Instruction]>,
    realm: ContextId,
    constants: Vec<BytecodeConstant>,
    auxiliary_atoms: Vec<Atom>,
) -> FunctionBytecodeData {
    FunctionBytecodeData {
        code: code.clone(),
        constants: constants.into(),
        realm,
        metadata: FunctionMetadata {
            max_stack: 1,
            ..FunctionMetadata::default()
        },
        parameter_environment: None,
        func_name: None,
        argument_definitions: Rc::from([]),
        local_definitions: Rc::from([]),
        closure_variables: Rc::from([]),
        private_bindings: PublishedPrivateBindings::none(),
        eval_environments: Rc::from([]),
        debug: None,
        auxiliary_atoms: auxiliary_atoms.into_boxed_slice(),
    }
}

fn closure_bytecode(
    code: &Rc<[Instruction]>,
    realm: ContextId,
    closure_count: u16,
) -> FunctionBytecodeData {
    let mut bytecode = bytecode(code, realm, Vec::new(), Vec::new());
    bytecode.metadata.closure_count = closure_count;
    bytecode.closure_variables = (0..closure_count)
        .map(|index| ClosureVariable {
            source: ClosureSource::ParentClosure(index),
            name: ClosureVariableName::None,
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        })
        .collect::<Vec<_>>()
        .into();
    bytecode
}

fn bytecode_test_realm(heap: &mut Heap) -> ContextId {
    let shape = empty_shape(heap);
    let prototype = heap
        .allocate_object(ObjectData::ordinary(shape, Vec::new()))
        .unwrap();
    heap.allocate_context(ContextData::new(
        prototype, prototype, prototype, prototype, prototype, prototype, prototype, prototype,
    ))
    .unwrap()
}

mod buffers;
mod bytecode;
mod collections;
mod eval_bytecode;
mod function_lifetimes;
mod modules;
mod native;
mod objects;
mod private_bytecode;
mod storage;
mod weak_collections;
