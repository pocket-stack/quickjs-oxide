use crate::engine::heap::native::{DataViewElementKind, NativeCProto};

use super::*;

#[test]
fn layout_replacement_preserves_array_buffer_payload_and_rolls_back_failures() {
    let mut heap = Heap::new();
    let original_shape = empty_shape(&mut heap);
    let bytes = (0..4096)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let pointer = bytes.as_ptr();
    let object = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            original_shape,
            Vec::new(),
            bytes,
            Some(8192),
        ))
        .unwrap();
    let property_shape = one_slot_shape(&mut heap);

    assert_eq!(
        heap.replace_object_layout(object, property_shape, Vec::new()),
        Err(HeapError::Invariant(
            "object slot count does not match its shape",
        )),
    );
    let current = heap.object(object).unwrap();
    assert_eq!(current.shape, original_shape);
    let ObjectPayload::ArrayBuffer(data) = &current.payload else {
        panic!("failed layout replacement removed the ArrayBuffer brand");
    };
    assert_eq!(data.bytes.as_ptr(), pointer);
    assert_eq!(data.max_byte_length, Some(8192));
    assert!(!data.detached);

    assert_eq!(
        heap.replace_object_layout(
            object,
            property_shape,
            vec![PropertySlot::Data(RawValue::Int(42))],
        )
        .unwrap(),
        HeapCleanup::default(),
    );
    let current = heap.object(object).unwrap();
    assert_eq!(current.shape, property_shape);
    let ObjectPayload::ArrayBuffer(data) = &current.payload else {
        panic!("successful layout replacement removed the ArrayBuffer brand");
    };
    assert_eq!(data.bytes.as_ptr(), pointer);
    assert_eq!(data.bytes.len(), 4096);
    assert_eq!(data.max_byte_length, Some(8192));
    assert!(!data.detached);

    assert_eq!(
        heap.release_shape(original_shape).unwrap().finalized_shapes,
        1,
    );
    assert_eq!(
        heap.release_shape(property_shape).unwrap(),
        HeapCleanup::default(),
    );
    let cleanup = heap.release_object(object).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn shared_array_buffer_heap_leaf_shares_bytes_but_not_wrapper_growth() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let external = SharedBufferHandle::new(2, Some(8)).unwrap();
    let object = heap
        .allocate_object(ObjectData::shared_array_buffer(
            shape,
            Vec::new(),
            external.clone(),
        ))
        .unwrap();

    assert_eq!(
        heap.object(object).unwrap().kind,
        ObjectKind::SharedArrayBuffer
    );
    assert_eq!(
        heap.buffer_state(object),
        Ok(ArrayBufferState {
            byte_length: 2,
            max_byte_length: Some(8),
            detached: false,
        })
    );
    heap.clone_shared_array_buffer_handle(object)
        .unwrap()
        .write_word(0, &[7, 9])
        .unwrap();
    assert_eq!(external.read_range(0, 2).unwrap(), [7, 9]);

    let before_grow = heap.clone_shared_array_buffer_handle(object).unwrap();
    heap.grow_shared_array_buffer(object, 6).unwrap();
    assert_eq!(heap.buffer_state(object).unwrap().byte_length, 6);
    assert_eq!(before_grow.byte_length(), 2);
    assert_eq!(external.byte_length(), 2);
    let after_grow = heap.clone_shared_array_buffer_handle(object).unwrap();
    assert_eq!(after_grow.read_word(2, 4).unwrap(), [0; 8]);
    assert!(after_grow.read_range(6, 1).is_err());

    let slice = after_grow.slice(0, 2).unwrap();
    assert_eq!(slice.read_range(0, 2).unwrap(), [7, 9]);
    assert!(!slice.shares_backing_with(&external));
    slice.write_range(0, &[1]).unwrap();
    assert_eq!(external.read_range(0, 1).unwrap(), [7]);

    assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());
    let cleanup = heap.release_object(object).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(external.read_range(0, 2).unwrap(), [7, 9]);
}

#[test]
fn shared_array_buffer_view_edge_remains_wrapper_local_to_gc() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let handle = SharedBufferHandle::new(4, Some(8)).unwrap();
    let buffer = heap
        .allocate_object(ObjectData::shared_array_buffer(
            shape,
            Vec::new(),
            handle.clone(),
        ))
        .unwrap();
    let view = heap
        .allocate_object(ObjectData::data_view(
            shape,
            Vec::new(),
            ArrayBufferViewData {
                buffer,
                byte_offset: 1,
                fixed_byte_length: None,
            },
        ))
        .unwrap();

    assert_eq!(heap.object_strong_count(buffer), Ok(2));
    assert_eq!(heap.release_shape(shape).unwrap(), HeapCleanup::default());
    assert_eq!(heap.release_object(buffer).unwrap(), HeapCleanup::default());
    let cleanup = heap.release_object(view).unwrap();
    assert_eq!(cleanup.finalized_objects, 2);
    assert_eq!(cleanup.finalized_shapes, 1);

    handle.write_word(0, &[3, 4, 5, 6]).unwrap();
    assert_eq!(handle.read_range(0, 4).unwrap(), [3, 4, 5, 6]);
}

#[test]
fn array_buffer_resize_failure_is_atomic_and_shrink_releases_capacity() {
    const INITIAL_LENGTH: usize = 64 * 1024;
    const SHRUNK_LENGTH: usize = 43;

    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let bytes = (0..INITIAL_LENGTH)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let object = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            shape,
            Vec::new(),
            bytes.clone(),
            Some(128 * 1024),
        ))
        .unwrap();
    heap.release_shape(shape).unwrap();

    let before_failure = heap.object(object).unwrap();
    let ObjectPayload::ArrayBuffer(before_failure) = &before_failure.payload else {
        panic!("test object lost its ArrayBuffer payload");
    };
    let allocated_pointer = before_failure.bytes.as_ptr();
    let allocated_capacity = before_failure.bytes.capacity();
    assert!(allocated_capacity >= INITIAL_LENGTH);

    assert!(!heap.resize_array_buffer_bytes(object, usize::MAX).unwrap());
    let after_failure = heap.object(object).unwrap();
    let ObjectPayload::ArrayBuffer(after_failure) = &after_failure.payload else {
        panic!("failed resize removed the ArrayBuffer payload");
    };
    assert_eq!(after_failure.bytes.as_ptr(), allocated_pointer);
    assert_eq!(after_failure.bytes.capacity(), allocated_capacity);
    assert_eq!(after_failure.bytes, bytes);
    assert!(!after_failure.detached);

    assert!(
        heap.resize_array_buffer_bytes(object, SHRUNK_LENGTH)
            .unwrap()
    );
    let shrunk = heap.object(object).unwrap();
    let ObjectPayload::ArrayBuffer(shrunk) = &shrunk.payload else {
        panic!("successful resize removed the ArrayBuffer payload");
    };
    assert_ne!(shrunk.bytes.as_ptr(), allocated_pointer);
    assert!(shrunk.bytes.capacity() < allocated_capacity);
    assert_eq!(shrunk.bytes, bytes[..SHRUNK_LENGTH]);
    assert!(!shrunk.detached);

    let cleanup = heap.release_object(object).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn array_buffer_transfer_failure_is_atomic_and_shrink_releases_capacity() {
    const INITIAL_LENGTH: usize = 64 * 1024;
    const TRANSFERRED_LENGTH: usize = 47;

    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let bytes = (0..INITIAL_LENGTH)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    let source = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            shape,
            Vec::new(),
            bytes.clone(),
            None,
        ))
        .unwrap();
    let target = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            shape,
            Vec::new(),
            Vec::new(),
            None,
        ))
        .unwrap();
    heap.release_shape(shape).unwrap();
    let source_pointer = match &heap.object(source).unwrap().payload {
        ObjectPayload::ArrayBuffer(data) => data.bytes.as_ptr(),
        _ => panic!("source object lost its ArrayBuffer payload"),
    };
    let source_capacity = match &heap.object(source).unwrap().payload {
        ObjectPayload::ArrayBuffer(data) => data.bytes.capacity(),
        _ => panic!("source object lost its ArrayBuffer payload"),
    };

    assert!(
        !heap
            .transfer_array_buffer_bytes(source, target, usize::MAX)
            .unwrap()
    );
    let ObjectPayload::ArrayBuffer(source_after_failure) = &heap.object(source).unwrap().payload
    else {
        panic!("failed transfer removed the source ArrayBuffer payload");
    };
    assert_eq!(source_after_failure.bytes.as_ptr(), source_pointer);
    assert_eq!(source_after_failure.bytes.capacity(), source_capacity);
    assert_eq!(source_after_failure.bytes, bytes);
    assert!(!source_after_failure.detached);
    let ObjectPayload::ArrayBuffer(target_after_failure) = &heap.object(target).unwrap().payload
    else {
        panic!("failed transfer removed the target ArrayBuffer payload");
    };
    assert!(target_after_failure.bytes.is_empty());
    assert!(!target_after_failure.detached);

    assert!(
        heap.transfer_array_buffer_bytes(source, target, TRANSFERRED_LENGTH)
            .unwrap()
    );
    let ObjectPayload::ArrayBuffer(source_after_transfer) = &heap.object(source).unwrap().payload
    else {
        panic!("successful transfer removed the source ArrayBuffer payload");
    };
    assert!(source_after_transfer.bytes.is_empty());
    assert_eq!(source_after_transfer.bytes.capacity(), 0);
    assert!(source_after_transfer.detached);
    let ObjectPayload::ArrayBuffer(target_after_transfer) = &heap.object(target).unwrap().payload
    else {
        panic!("successful transfer removed the target ArrayBuffer payload");
    };
    assert_ne!(target_after_transfer.bytes.as_ptr(), source_pointer);
    assert!(target_after_transfer.bytes.capacity() < source_capacity);
    assert_eq!(target_after_transfer.bytes, bytes[..TRANSFERRED_LENGTH],);
    assert!(!target_after_transfer.detached);

    assert_eq!(heap.release_object(source).unwrap().finalized_objects, 1,);
    let cleanup = heap.release_object(target).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn data_view_retains_its_buffer_and_survives_oob_detach_and_cycle_collection() {
    let mut heap = Heap::new();
    let base_shape = empty_shape(&mut heap);
    let buffer = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            base_shape,
            Vec::new(),
            vec![0; 8],
            Some(16),
        ))
        .unwrap();
    let ordinary = leaf(&mut heap, base_shape);
    let ordinary_strong = heap.object_strong_count(ordinary).unwrap();
    assert_eq!(
        heap.allocate_object(ObjectData::data_view(
            base_shape,
            Vec::new(),
            ArrayBufferViewData {
                buffer: ordinary,
                byte_offset: 0,
                fixed_byte_length: Some(0),
            },
        )),
        Err(HeapError::Invariant(
            "DataView backing object is not an ArrayBuffer or SharedArrayBuffer",
        )),
    );
    assert_eq!(heap.object_strong_count(ordinary), Ok(ordinary_strong));
    assert_eq!(heap.release_object(ordinary).unwrap().finalized_objects, 1);
    let fixed_buffer = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            base_shape,
            Vec::new(),
            vec![0; 4],
            None,
        ))
        .unwrap();
    assert_eq!(
        heap.allocate_object(ObjectData::data_view(
            base_shape,
            Vec::new(),
            ArrayBufferViewData {
                buffer: fixed_buffer,
                byte_offset: 0,
                fixed_byte_length: None,
            },
        )),
        Err(HeapError::Invariant(
            "DataView has an invalid structural view layout",
        )),
    );
    assert_eq!(
        heap.release_object(fixed_buffer).unwrap().finalized_objects,
        1,
    );
    assert_eq!(
        heap.allocate_object(ObjectData::data_view(
            base_shape,
            Vec::new(),
            ArrayBufferViewData {
                buffer,
                byte_offset: i32::MAX as u32,
                fixed_byte_length: Some(1),
            },
        )),
        Err(HeapError::Invariant(
            "DataView has an invalid structural view layout",
        )),
    );
    assert_eq!(heap.object_strong_count(buffer), Ok(1));
    let view = heap
        .allocate_object(ObjectData::data_view(
            base_shape,
            Vec::new(),
            ArrayBufferViewData {
                buffer,
                byte_offset: 4,
                fixed_byte_length: Some(4),
            },
        ))
        .unwrap();

    assert_eq!(
        object_edges(heap.object(view).unwrap()),
        vec![RawId::Shape(base_shape), RawId::Object(buffer)],
    );
    assert_eq!(heap.object_strong_count(buffer), Ok(2));
    assert_eq!(heap.release_object(buffer).unwrap(), HeapCleanup::default(),);
    assert_eq!(heap.object_strong_count(buffer), Ok(1));

    assert!(heap.resize_array_buffer_bytes(buffer, 2).unwrap());
    assert_eq!(
        heap.validate_object_layout(heap.object(view).unwrap()),
        Ok(()),
    );
    assert_eq!(
        heap.replace_object_layout(view, base_shape, Vec::new())
            .unwrap(),
        HeapCleanup::default(),
    );
    let ObjectPayload::DataView(out_of_bounds) = &heap.object(view).unwrap().payload else {
        panic!("out-of-bounds transition removed the DataView payload");
    };
    assert_eq!(
        *out_of_bounds,
        ArrayBufferViewData {
            buffer,
            byte_offset: 4,
            fixed_byte_length: Some(4),
        },
    );

    heap.detach_array_buffer(buffer).unwrap();
    assert_eq!(
        heap.validate_object_layout(heap.object(view).unwrap()),
        Ok(()),
    );
    assert_eq!(
        heap.replace_object_layout(view, base_shape, Vec::new())
            .unwrap(),
        HeapCleanup::default(),
    );
    assert_eq!(
        heap.buffer_state(buffer),
        Ok(ArrayBufferState {
            byte_length: 0,
            max_byte_length: Some(16),
            detached: true,
        }),
    );

    let buffer_cycle_shape = heap
        .allocate_shape(Shape::new(Some(view), []).unwrap())
        .unwrap();
    heap.replace_object_layout(buffer, buffer_cycle_shape, Vec::new())
        .unwrap();
    assert_eq!(
        heap.release_shape(base_shape).unwrap(),
        HeapCleanup::default(),
    );
    assert_eq!(
        heap.release_shape(buffer_cycle_shape).unwrap(),
        HeapCleanup::default(),
    );
    assert_eq!(heap.release_object(view).unwrap(), HeapCleanup::default(),);

    let stats = collect_heap(&mut heap).unwrap();
    assert_eq!(stats.cleanup.finalized_objects, 2);
    assert_eq!(stats.cleanup.finalized_shapes, 2);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn array_buffer_word_access_is_borrow_contained_and_bounds_checked() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let buffer = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            shape,
            Vec::new(),
            vec![0; 12],
            Some(20),
        ))
        .unwrap();
    heap.release_shape(shape).unwrap();

    assert_eq!(
        heap.buffer_state(buffer),
        Ok(ArrayBufferState {
            byte_length: 12,
            max_byte_length: Some(20),
            detached: false,
        }),
    );
    heap.write_array_buffer_word(buffer, 3, &[0xaa, 0xbb, 0xcc, 0xdd])
        .unwrap();
    assert_eq!(
        heap.read_array_buffer_word(buffer, 3, 4).unwrap(),
        [0xaa, 0xbb, 0xcc, 0xdd, 0, 0, 0, 0],
    );
    assert_eq!(
        heap.read_array_buffer_word(buffer, 0, 3),
        Err(HeapError::Invariant(
            "ArrayBuffer word read has an unsupported byte length",
        )),
    );
    assert_eq!(
        heap.write_array_buffer_word(buffer, 0, &[1, 2, 3]),
        Err(HeapError::Invariant(
            "ArrayBuffer word write has an unsupported byte length",
        )),
    );
    assert_eq!(
        heap.read_array_buffer_word(buffer, 9, 4),
        Err(HeapError::Invariant(
            "ArrayBuffer word read exceeded the live backing store",
        )),
    );
    assert_eq!(
        heap.write_array_buffer_word(buffer, 11, &[1, 2]),
        Err(HeapError::Invariant(
            "ArrayBuffer word write exceeded the live backing store",
        )),
    );
    assert_eq!(
        heap.read_array_buffer_word(buffer, usize::MAX, 1),
        Err(HeapError::Invariant(
            "ArrayBuffer word read range overflowed usize",
        )),
    );

    heap.detach_array_buffer(buffer).unwrap();
    assert_eq!(
        heap.read_array_buffer_word(buffer, 0, 1),
        Err(HeapError::Invariant(
            "ArrayBuffer word read exceeded the live backing store",
        )),
    );
    assert_eq!(
        heap.write_array_buffer_word(buffer, 0, &[1]),
        Err(HeapError::Invariant(
            "ArrayBuffer word write exceeded the live backing store",
        )),
    );

    let cleanup = heap.release_object(buffer).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn array_buffer_word_mutation_preserves_width_and_rejects_stale_ranges() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let buffer = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            shape,
            Vec::new(),
            vec![0, 1, 2, 3, 4, 5, 6, 7],
            Some(12),
        ))
        .unwrap();
    heap.release_shape(shape).unwrap();

    heap.fill_array_buffer_words(buffer, 2, &[0xaa, 0xbb], 2)
        .unwrap();
    heap.reverse_array_buffer_words(buffer, 0, 2, 4).unwrap();
    assert_eq!(
        heap.read_array_buffer_word(buffer, 0, 8).unwrap(),
        [6, 7, 0xaa, 0xbb, 0xaa, 0xbb, 0, 1],
    );
    assert_eq!(
        heap.fill_array_buffer_words(buffer, 0, &[1, 2, 3], 1),
        Err(HeapError::Invariant(
            "ArrayBuffer fill word has an invalid width",
        )),
    );
    assert_eq!(
        heap.reverse_array_buffer_words(buffer, 0, 3, 1),
        Err(HeapError::Invariant(
            "ArrayBuffer reverse word has an invalid width",
        )),
    );
    assert_eq!(
        heap.fill_array_buffer_words(buffer, 7, &[1, 2], 1),
        Err(HeapError::Invariant(
            "ArrayBuffer fill exceeded a live backing store",
        )),
    );
    assert_eq!(
        heap.reverse_array_buffer_words(buffer, 2, 2, 4),
        Err(HeapError::Invariant(
            "ArrayBuffer reverse exceeded a live backing store",
        )),
    );

    heap.detach_array_buffer(buffer).unwrap();
    assert_eq!(
        heap.fill_array_buffer_words(buffer, 0, &[1], 1),
        Err(HeapError::Invariant(
            "ArrayBuffer fill exceeded a live backing store",
        )),
    );
    assert_eq!(
        heap.reverse_array_buffer_words(buffer, 0, 1, 1),
        Err(HeapError::Invariant(
            "ArrayBuffer reverse exceeded a live backing store",
        )),
    );

    let cleanup = heap.release_object(buffer).unwrap();
    assert_eq!(cleanup.finalized_objects, 1);
    assert_eq!(cleanup.finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn data_view_native_descriptors_cover_all_element_formats() {
    assert_eq!(
        NativeFunctionId::DataView(DataViewNativeKind::Constructor)
            .descriptor()
            .cproto,
        NativeCProto::Constructor,
    );
    for getter in [
        DataViewNativeKind::Buffer,
        DataViewNativeKind::ByteLength,
        DataViewNativeKind::ByteOffset,
    ] {
        assert_eq!(
            NativeFunctionId::DataView(getter).descriptor().cproto,
            NativeCProto::Getter,
        );
    }
    for (element, byte_length) in [
        (DataViewElementKind::Int8, 1),
        (DataViewElementKind::Uint8, 1),
        (DataViewElementKind::Int16, 2),
        (DataViewElementKind::Uint16, 2),
        (DataViewElementKind::Int32, 4),
        (DataViewElementKind::Uint32, 4),
        (DataViewElementKind::BigInt64, 8),
        (DataViewElementKind::BigUint64, 8),
        (DataViewElementKind::Float16, 2),
        (DataViewElementKind::Float32, 4),
        (DataViewElementKind::Float64, 8),
    ] {
        assert_eq!(element.byte_length(), byte_length);
        assert_eq!(
            NativeFunctionId::DataView(DataViewNativeKind::Get(element))
                .descriptor()
                .cproto,
            NativeCProto::GenericMagic,
        );
        assert_eq!(
            NativeFunctionId::DataView(DataViewNativeKind::Set(element))
                .descriptor()
                .cproto,
            NativeCProto::GenericMagic,
        );
    }
}

#[test]
fn typed_array_payloads_share_one_structural_view_kernel() {
    let mut heap = Heap::new();
    let shape = empty_shape(&mut heap);
    let resizable = heap
        .allocate_object(ObjectData::array_buffer_from_bytes(
            shape,
            Vec::new(),
            vec![0; 16],
            Some(32),
        ))
        .unwrap();
    let ordinary = leaf(&mut heap, shape);

    assert_eq!(
        heap.allocate_object(ObjectData::typed_array(
            shape,
            Vec::new(),
            TypedArrayData {
                view: ArrayBufferViewData {
                    buffer: ordinary,
                    byte_offset: 0,
                    fixed_byte_length: Some(0),
                },
                element: TypedArrayElementKind::Uint8,
            },
        )),
        Err(HeapError::Invariant(
            "TypedArray backing object is not an ArrayBuffer or SharedArrayBuffer",
        )),
    );
    assert_eq!(
        heap.allocate_object(ObjectData::typed_array(
            shape,
            Vec::new(),
            TypedArrayData {
                view: ArrayBufferViewData {
                    buffer: resizable,
                    byte_offset: 1,
                    fixed_byte_length: Some(4),
                },
                element: TypedArrayElementKind::Uint16,
            },
        )),
        Err(HeapError::Invariant(
            "TypedArray has an invalid structural view layout",
        )),
    );

    let tracking = heap
        .allocate_object(ObjectData::typed_array(
            shape,
            Vec::new(),
            TypedArrayData {
                view: ArrayBufferViewData {
                    buffer: resizable,
                    byte_offset: 4,
                    fixed_byte_length: None,
                },
                element: TypedArrayElementKind::Uint32,
            },
        ))
        .unwrap();
    assert_eq!(
        object_edges(heap.object(tracking).unwrap()),
        vec![RawId::Shape(shape), RawId::Object(resizable)],
    );
    assert_eq!(heap.object_strong_count(resizable), Ok(2));

    assert!(heap.resize_array_buffer_bytes(resizable, 2).unwrap());
    assert_eq!(
        heap.validate_object_layout(heap.object(tracking).unwrap()),
        Ok(()),
    );
    heap.detach_array_buffer(resizable).unwrap();
    assert_eq!(
        heap.validate_object_layout(heap.object(tracking).unwrap()),
        Ok(()),
    );

    assert_eq!(
        heap.release_object(resizable).unwrap(),
        HeapCleanup::default()
    );
    let cleanup = heap.release_object(tracking).unwrap();
    assert_eq!(cleanup.finalized_objects, 2);
    assert_eq!(heap.release_object(ordinary).unwrap().finalized_objects, 1);
    assert_eq!(heap.release_shape(shape).unwrap().finalized_shapes, 1);
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn typed_array_native_descriptors_cover_all_concrete_classes() {
    assert_eq!(
        NativeFunctionId::TypedArray(TypedArrayNativeKind::BaseConstructor)
            .descriptor()
            .cproto,
        NativeCProto::ConstructorOrFunction,
    );
    let expected = [
        (TypedArrayElementKind::Uint8Clamped, 1, false),
        (TypedArrayElementKind::Int8, 1, false),
        (TypedArrayElementKind::Uint8, 1, false),
        (TypedArrayElementKind::Int16, 2, false),
        (TypedArrayElementKind::Uint16, 2, false),
        (TypedArrayElementKind::Int32, 4, false),
        (TypedArrayElementKind::Uint32, 4, false),
        (TypedArrayElementKind::BigInt64, 8, true),
        (TypedArrayElementKind::BigUint64, 8, true),
        (TypedArrayElementKind::Float16, 2, false),
        (TypedArrayElementKind::Float32, 4, false),
        (TypedArrayElementKind::Float64, 8, false),
    ];
    assert_eq!(TypedArrayElementKind::ALL.len(), expected.len());
    for (index, (element, byte_length, is_bigint)) in expected.into_iter().enumerate() {
        assert_eq!(TypedArrayElementKind::ALL[index], element);
        assert_eq!(element.byte_length(), byte_length);
        assert_eq!(element.is_bigint(), is_bigint);
        assert_eq!(
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Constructor(element))
                .descriptor()
                .cproto,
            NativeCProto::ConstructorMagic,
        );
    }
}

#[test]
fn data_view_intrinsics_attach_transactionally_once() {
    let mut heap = Heap::new();
    let null_shape = empty_shape(&mut heap);
    let object_prototype = leaf(&mut heap, null_shape);
    let child_shape = heap
        .allocate_shape(Shape::new(Some(object_prototype), []).unwrap())
        .unwrap();
    let prototype = leaf(&mut heap, child_shape);
    let realm = heap
        .allocate_context(ContextData::new(
            object_prototype,
            object_prototype,
            object_prototype,
            object_prototype,
            object_prototype,
            object_prototype,
            object_prototype,
            object_prototype,
        ))
        .unwrap();
    let constructor = heap
        .allocate_object(ObjectData::bound_native_function(
            child_shape,
            Vec::new(),
            NativeFunctionId::DataView(DataViewNativeKind::Constructor),
            realm,
            1,
        ))
        .unwrap();

    let root_strong = heap.object_strong_count(object_prototype).unwrap();
    assert_eq!(
        heap.attach_data_view_intrinsics(
            realm,
            constructor,
            DataViewRealmData {
                prototype: object_prototype,
            },
        ),
        Err(HeapError::Invariant(
            "DataView prototype is not an ordinary child of Object.prototype",
        )),
    );
    assert_eq!(heap.object_strong_count(object_prototype), Ok(root_strong),);
    assert_eq!(heap.context(realm).unwrap().data_view, None);

    let prototype_strong = heap.object_strong_count(prototype).unwrap();
    heap.attach_data_view_intrinsics(realm, constructor, DataViewRealmData { prototype })
        .unwrap();
    assert_eq!(
        heap.context(realm).unwrap().data_view,
        Some(DataViewRealmData { prototype }),
    );
    assert_eq!(
        heap.object_strong_count(prototype),
        Ok(prototype_strong + 1),
    );
    assert_eq!(
        heap.attach_data_view_intrinsics(realm, constructor, DataViewRealmData { prototype },),
        Err(HeapError::Invariant(
            "context already has DataView intrinsic roots",
        )),
    );
    assert_eq!(
        heap.object_strong_count(prototype),
        Ok(prototype_strong + 1),
    );

    heap.release_object(constructor).unwrap();
    heap.release_context(realm).unwrap();
    heap.release_object(prototype).unwrap();
    heap.release_object(object_prototype).unwrap();
    heap.release_shape(child_shape).unwrap();
    heap.release_shape(null_shape).unwrap();
    assert_eq!(heap.counts().live, 0);
}

#[test]
fn symbol_atom_ownership_is_returned_on_replace_and_finalize() {
    let mut heap = Heap::new();
    let shape = one_slot_shape(&mut heap);
    let first_symbol = Atom::from_raw(17);
    let second_symbol = Atom::from_raw(23);
    let object = heap
        .allocate_object(ObjectData::ordinary(
            shape,
            vec![PropertySlot::Data(RawValue::Symbol(first_symbol))],
        ))
        .unwrap();

    let replacement = heap
        .replace_object_slot(
            object,
            0,
            PropertySlot::Data(RawValue::Symbol(second_symbol)),
        )
        .unwrap();
    assert_eq!(replacement.atoms, vec![first_symbol]);

    let object_cleanup = heap.release_object(object).unwrap();
    assert_eq!(object_cleanup.atoms, vec![second_symbol]);
    let shape_cleanup = heap.release_shape(shape).unwrap();
    assert_eq!(
        shape_cleanup.atoms,
        vec![Atom::from_immediate_integer(0).unwrap()]
    );
}
