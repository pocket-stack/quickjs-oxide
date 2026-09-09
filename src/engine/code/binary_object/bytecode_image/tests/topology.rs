use super::*;

#[test]
fn nested_functions_receive_preorder_ids_without_object_reference_ids() {
    let vector = bytes(
        "05020a6f757465720a696e6e65720c000200a80100010002000001090100000000be00bb28edb5edcb280c430200e60301010101010001080200010000000000e05e0000cfc7be00280c430200e803010001020001000e010001000000001000640000cf9b116500000e64000028",
    );
    let image = decode_image(&vector).unwrap();

    assert_eq!(image.functions().len(), 3);
    assert!(image.nodes().is_empty());
    assert!(image.reference_table().is_empty());
    let root = function_id(image.root());
    let outer = function_id(&image.functions()[0].constants()[0]);
    let inner = function_id(&image.functions()[1].constants()[0]);
    assert_eq!(
        [root.zero_based(), outer.zero_based(), inner.zero_based()],
        [0, 1, 2]
    );
    assert!(image.functions()[2].constants().is_empty());
    assert_eq!(image.functions()[2].envelope().closures().len(), 1);
    assert!(image.function(root).is_some());
    assert!(image.function(outer).is_some());
    assert!(image.function(inner).is_some());
}

#[test]
fn one_arena_spans_functions_templates_and_trailing_references() {
    let vector = bytes(
        "05011074656d706c6174650802360c000200a80100010002000002070100000000be00bd01edcb280c0202000001000101000000020100010000cf280b010702780b0107027802e6031301",
    );
    let image = decode_image(&vector).unwrap();

    let root = node_id(image.root());
    assert_eq!(root.zero_based(), 0);
    assert_eq!(image.functions().len(), 2);
    assert_eq!(image.nodes().len(), 3);
    assert_eq!(
        image
            .reference_table()
            .iter()
            .map(|node| node.zero_based())
            .collect::<Vec<_>>(),
        [0, 1, 2]
    );

    let WireNodeCarrier::Ordinary { properties } = &image.nodes()[0] else {
        panic!("root must be ordinary");
    };
    assert_eq!(properties.len(), 2);
    assert_eq!(function_id(&properties[0].value).zero_based(), 0);
    assert_eq!(node_id(&properties[1].value).zero_based(), 1);
    assert_eq!(
        function_id(&image.functions()[0].constants()[0]).zero_based(),
        1
    );
    assert_eq!(
        node_id(&image.functions()[0].constants()[1]).zero_based(),
        1
    );
    assert!(image.functions()[1].constants().is_empty());

    let WireNodeCarrier::TemplateObject { elements, raw } = &image.nodes()[1] else {
        panic!("node one must be a template object");
    };
    assert_eq!(elements.len(), 1);
    assert_eq!(node_id(raw).zero_based(), 2);
}

#[test]
fn function_constant_pool_can_reference_its_enclosing_object_ancestor() {
    let vector = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    let image = decode_image(&vector).unwrap();

    let root = node_id(image.root());
    assert_eq!(root.zero_based(), 0);
    assert_eq!(image.reference_table(), [root]);
    assert_eq!(image.functions().len(), 1);
    assert_eq!(node_id(&image.functions()[0].constants()[0]), root);
    let WireNodeCarrier::Ordinary { properties } = &image.nodes()[0] else {
        panic!("root must be ordinary");
    };
    assert_eq!(properties.len(), 1);
    assert_eq!(function_id(&properties[0].value).zero_based(), 0);
}

#[test]
fn functions_are_valid_children_of_each_data_container_shape() {
    let record = quickjs_42_record();
    for (body, expected_kind) in [
        (vec![BcTag::Array.to_byte(), 1], "array"),
        (vec![BcTag::TemplateObject.to_byte(), 0], "template"),
    ] {
        let mut vector = vec![5, 0];
        vector.extend_from_slice(&body);
        vector.extend_from_slice(&record);
        let image = decode_image(&vector).unwrap();
        assert_eq!(image.functions().len(), 1);
        match (&image.nodes()[0], expected_kind) {
            (WireNodeCarrier::Array { elements }, "array") => {
                assert_eq!(function_id(&elements[0]).zero_based(), 0);
            }
            (WireNodeCarrier::TemplateObject { elements, raw }, "template") => {
                assert!(elements.is_empty());
                assert_eq!(function_id(raw).zero_based(), 0);
            }
            (node, kind) => panic!("expected {kind}, got {node:?}"),
        }
    }
}

#[test]
fn data_only_coercion_tags_reject_function_children_with_typed_errors() {
    let record = quickjs_42_record();
    let mut object_value = vec![5, 0, BcTag::ObjectValue.to_byte()];
    object_value.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&object_value),
        Err(BytecodeImageError::Data(DecodeError::OpaqueObjectValue {
            offset: 2,
            value,
        })) if matches!(value, ImageOpaque::Function(function) if function.zero_based() == 0)
    ));

    let mut date = vec![5, 0, BcTag::Date.to_byte()];
    date.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&date),
        Err(BytecodeImageError::Data(DecodeError::OpaqueDateValue {
            offset: 2,
            value,
        })) if matches!(value, ImageOpaque::Function(function) if function.zero_based() == 0)
    ));

    let mut typed_array = vec![5, 0, BcTag::TypedArray.to_byte(), 2, 1, 0];
    typed_array.extend_from_slice(&record);
    assert!(matches!(
        decode_image(&typed_array),
        Err(BytecodeImageError::Data(
            DecodeError::OpaqueTypedArrayBacking {
                offset: 2,
                value,
            }
        )) if matches!(value, ImageOpaque::Function(function) if function.zero_based() == 0)
    ));
}

#[test]
fn data_only_coercion_tags_reject_module_children_with_typed_errors() {
    let record = counted_module_record(0, 0, 0, 0);
    for (tag, mut vector) in [
        (BcTag::ObjectValue, vec![5, 0, BcTag::ObjectValue.to_byte()]),
        (BcTag::Date, vec![5, 0, BcTag::Date.to_byte()]),
        (
            BcTag::TypedArray,
            vec![5, 0, BcTag::TypedArray.to_byte(), 2, 1, 0],
        ),
    ] {
        vector.extend_from_slice(&record);
        let (offset, value) = match (tag, decode_image(&vector)) {
            (
                BcTag::ObjectValue,
                Err(BytecodeImageError::Data(DecodeError::OpaqueObjectValue { offset, value })),
            )
            | (
                BcTag::Date,
                Err(BytecodeImageError::Data(DecodeError::OpaqueDateValue { offset, value })),
            )
            | (
                BcTag::TypedArray,
                Err(BytecodeImageError::Data(DecodeError::OpaqueTypedArrayBacking {
                    offset,
                    value,
                })),
            ) => (offset, value),
            (_, result) => panic!("{tag:?} returned the wrong Module coercion result: {result:?}"),
        };
        assert_eq!(offset, 2);
        assert!(
            matches!(value, ImageOpaque::Module(module) if module.zero_based() == 0),
            "{tag:?} did not retain the typed Module identity",
        );
    }
}

#[test]
fn opaque_function_source_tokens_reject_cross_image_rebranding() {
    let first = decode_image(&bytes("05000c000200a80100010001000000040100000000bb2acb28")).unwrap();
    let second =
        decode_image(&bytes("05000c000200a80100010001000000040100000000bb2acb28")).unwrap();
    let first_id = function_id(first.root());
    let second_id = function_id(second.root());
    assert_eq!(first_id.zero_based(), second_id.zero_based());
    assert_ne!(first_id, second_id);
    assert!(first.function(first_id).is_some());
    assert!(first.function(second_id).is_none());

    let machine_b = DataMachine::<ImageValue, ImageKey>::new(GRAPH_LIMITS, true).unwrap();
    assert!(matches!(
        machine_b.wrap_opaque_value(first.root().clone()),
        Err(DecodeError::InvalidCompletionTarget)
    ));
}

#[test]
fn opaque_module_source_tokens_reject_cross_image_rebranding() {
    let mut vector = vec![5, 0];
    vector.extend_from_slice(&counted_module_record(0, 0, 0, 0));
    let first = decode_image(&vector).unwrap();
    let second = decode_image(&vector).unwrap();
    let first_id = module_id(first.root());
    let second_id = module_id(second.root());

    assert_eq!(first_id.zero_based(), second_id.zero_based());
    assert_ne!(first_id, second_id);
    assert!(first.module(first_id).is_some());
    assert!(first.module(second_id).is_none());

    let machine_b = DataMachine::<ImageValue, ImageKey>::new(GRAPH_LIMITS, true).unwrap();
    assert!(matches!(
        machine_b.wrap_opaque_value(first.root().clone()),
        Err(DecodeError::InvalidCompletionTarget)
    ));
}
