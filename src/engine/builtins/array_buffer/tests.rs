use super::*;

#[test]
fn transfer_copies_prefix_zero_fills_growth_and_detaches_source() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(4);__source",
    );
    write_bytes(&runtime, &source, &[1, 2, 3, 4]);

    let target = eval_object(&mut context, "__source.transfer(6)");
    assert_eq!(snapshot(&runtime, &source), (Vec::new(), None, true));
    assert_eq!(
        snapshot(&runtime, &target),
        (vec![1, 2, 3, 4, 0, 0], None, false),
    );
}

#[test]
fn same_length_transfer_moves_the_owned_backing_allocation() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(4096);__source",
    );
    let bytes = (0..4096)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    write_bytes(&runtime, &source, &bytes);
    let pointer = backing_pointer(&runtime, &source);

    let target = eval_object(&mut context, "__source.transfer()");
    assert_eq!(snapshot(&runtime, &source), (Vec::new(), None, true));
    assert_eq!(backing_pointer(&runtime, &target), pointer);
    assert_eq!(snapshot(&runtime, &target), (bytes, None, false));
}

#[test]
fn resizable_transfer_truncates_and_preserves_maximum() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(4,{maxByteLength:8});__source",
    );
    write_bytes(&runtime, &source, &[11, 22, 33, 44]);

    let target = eval_object(&mut context, "__source.transfer(2)");
    assert_eq!(snapshot(&runtime, &source), (Vec::new(), Some(8), true));
    assert_eq!(snapshot(&runtime, &target), (vec![11, 22], Some(8), false),);
}

#[test]
fn shrinking_resize_releases_the_oversized_backing_allocation() {
    const INITIAL_LENGTH: usize = 64 * 1024;
    const SHRUNK_LENGTH: usize = 37;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(65536,{maxByteLength:131072});__source",
    );
    let bytes = (0..INITIAL_LENGTH)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    write_bytes(&runtime, &source, &bytes);
    let (original_pointer, original_capacity) = backing_layout(&runtime, &source);

    context.eval("__source.resize(37)").unwrap();

    let (shrunk_pointer, shrunk_capacity) = backing_layout(&runtime, &source);
    assert_ne!(shrunk_pointer, original_pointer);
    assert!(shrunk_capacity < original_capacity);
    assert_eq!(
        snapshot(&runtime, &source),
        (bytes[..SHRUNK_LENGTH].to_vec(), Some(131_072), false),
    );
}

#[test]
fn shrinking_transfer_releases_capacity_and_preserves_the_prefix() {
    const INITIAL_LENGTH: usize = 64 * 1024;
    const TRANSFERRED_LENGTH: usize = 41;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(65536,{maxByteLength:131072});__source",
    );
    let bytes = (0..INITIAL_LENGTH)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    write_bytes(&runtime, &source, &bytes);
    let (original_pointer, original_capacity) = backing_layout(&runtime, &source);

    let target = eval_object(&mut context, "__source.transfer(41)");

    let (target_pointer, target_capacity) = backing_layout(&runtime, &target);
    assert_ne!(target_pointer, original_pointer);
    assert!(target_capacity < original_capacity);
    assert_eq!(
        snapshot(&runtime, &source),
        (Vec::new(), Some(131_072), true)
    );
    assert_eq!(
        snapshot(&runtime, &target),
        (bytes[..TRANSFERRED_LENGTH].to_vec(), Some(131_072), false,),
    );
}

#[test]
fn failed_transfer_keeps_the_source_backing_store_attached() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(4,{maxByteLength:4});__source",
    );
    write_bytes(&runtime, &source, &[5, 6, 7, 8]);

    assert_eq!(
        context.eval("__source.transfer(5)"),
        Err(RuntimeError::Exception),
    );
    context.take_exception().unwrap().unwrap();
    assert_eq!(
        snapshot(&runtime, &source),
        (vec![5, 6, 7, 8], Some(4), false),
    );
}

#[test]
fn slice_copies_only_the_selected_backing_store_range() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(12288);__source",
    );
    let bytes = (0..12_288)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    write_bytes(&runtime, &source, &bytes);

    let target = eval_object(&mut context, "__source.slice(2047,10242)");
    assert_eq!(snapshot(&runtime, &source), (bytes.clone(), None, false),);
    assert_eq!(
        snapshot(&runtime, &target),
        (bytes[2047..10_242].to_vec(), None, false),
    );
}

#[test]
fn ordinary_layout_changes_preserve_large_backing_store_in_place() {
    const BYTE_LENGTH: usize = 64 * 1024;

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let source = eval_object(
        &mut context,
        "globalThis.__source=new ArrayBuffer(65536,{maxByteLength:131072});__source",
    );
    let bytes = (0..BYTE_LENGTH)
        .map(|index| u8::try_from(index % 251).unwrap())
        .collect::<Vec<_>>();
    write_bytes(&runtime, &source, &bytes);
    let pointer = backing_pointer(&runtime, &source);

    for operation in [
        "Object.defineProperty(__source,'marker',{value:42,writable:true,enumerable:true,configurable:true})",
        "Object.defineProperty(__source,'marker',{enumerable:false})",
        "delete __source.marker",
    ] {
        context.eval(operation).unwrap();
        let state = runtime.0.state.borrow();
        let ObjectPayload::ArrayBuffer(data) =
            &state.heap.object(source.object_id()).unwrap().payload
        else {
            panic!("ordinary layout change removed the ArrayBuffer brand");
        };
        assert_eq!(
            data.bytes.as_ptr(),
            pointer,
            "ordinary layout change cloned or replaced the backing store: {operation}",
        );
        assert_eq!(data.bytes, bytes);
        assert_eq!(data.max_byte_length, Some(131_072));
        assert!(!data.detached);
    }

    assert_eq!(
        context
            .eval(
                "[__source.byteLength,__source.maxByteLength,__source.resizable,\
                 __source.detached,Object.prototype.toString.call(__source),\
                 Object.getPrototypeOf(__source)===ArrayBuffer.prototype].join('|')",
            )
            .unwrap(),
        Value::String(JsString::from_static(
            "65536|131072|true|false|[object ArrayBuffer]|true",
        )),
    );
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("ArrayBuffer test source did not return an object");
    };
    object
}

fn backing_pointer(runtime: &Runtime, object: &ObjectRef) -> *const u8 {
    backing_layout(runtime, object).0
}

fn backing_layout(runtime: &Runtime, object: &ObjectRef) -> (*const u8, usize) {
    let state = runtime.0.state.borrow();
    let ObjectPayload::ArrayBuffer(data) = &state.heap.object(object.object_id()).unwrap().payload
    else {
        panic!("test object lost its ArrayBuffer payload");
    };
    (data.bytes.as_ptr(), data.bytes.capacity())
}

fn write_bytes(runtime: &Runtime, object: &ObjectRef, bytes: &[u8]) {
    runtime
        .0
        .state
        .borrow_mut()
        .heap
        .write_array_buffer_prefix(object.object_id(), bytes)
        .unwrap();
}

fn snapshot(runtime: &Runtime, object: &ObjectRef) -> (Vec<u8>, Option<u32>, bool) {
    let state = runtime.0.state.borrow();
    let ObjectPayload::ArrayBuffer(data) = &state.heap.object(object.object_id()).unwrap().payload
    else {
        panic!("test object lost its ArrayBuffer payload");
    };
    (data.bytes.clone(), data.max_byte_length, data.detached)
}
