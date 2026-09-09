use super::*;

#[test]
fn self_contained_quickjs_module_decodes_and_encodes_byte_exactly() {
    let vector = self_contained_module_vector();
    assert_eq!(vector.len(), 109);
    let (atoms, _) = read_table(&vector);

    for references in [false, true] {
        let image =
            decode_image_with(&vector, ReaderMode::Strict, IMAGE_LIMITS, references).unwrap();
        assert!(image.nodes().is_empty());
        assert!(image.reference_table().is_empty());
        assert_eq!(image.modules().len(), 1);
        assert_eq!(image.functions().len(), 1);

        let root = module_id(image.root());
        let module = image.module(root).unwrap();
        assert_eq!(module, &image.modules()[0]);
        assert_eq!(module.name(), atoms.slot_atoms()[0]);
        assert!(module.requests().is_empty());
        assert_eq!(module.exports().len(), 1);
        assert_eq!(module.exports()[0].export_type(), 0);
        assert_eq!(module.exports()[0].local_variable_index(), Some(0));
        assert_eq!(module.exports()[0].request_index(), None);
        assert_eq!(module.exports()[0].local_name(), None);
        assert_eq!(module.exports()[0].export_name(), atoms.slot_atoms()[1]);
        assert!(module.star_export_request_indices().is_empty());
        assert!(module.imports().is_empty());
        assert!(!module.has_tla());

        let function = function_id(module.func_obj());
        assert_eq!(function.zero_based(), 0);
        assert_eq!(root.source(), function.source());
        assert_eq!(image.function(function), image.functions().first());
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(vector.clone()),
        );
    }
}

#[test]
fn metadata_rich_quickjs_module_preserves_the_complete_topology() {
    let vector = metadata_rich_module_vector();
    assert_eq!(vector.len(), 283);
    let (atoms, _) = read_table(&vector);
    let slots = atoms.slot_atoms();

    for references in [false, true] {
        let image =
            decode_image_with(&vector, ReaderMode::Strict, IMAGE_LIMITS, references).unwrap();
        assert_eq!(image.modules().len(), 1);
        assert_eq!(image.functions().len(), 1);
        assert_eq!(image.nodes().len(), 1);
        if references {
            assert_eq!(image.reference_table(), [NodeId::from_zero_based(0)]);
        } else {
            assert!(image.reference_table().is_empty());
        }

        let module = image.module(module_id(image.root())).unwrap();
        assert_eq!(module.name(), slots[0]);
        assert_eq!(module.requests().len(), 5);
        assert_eq!(
            module
                .requests()
                .iter()
                .map(|request| request.name())
                .collect::<Vec<_>>(),
            [slots[1], slots[4], slots[1], slots[5], slots[4]],
        );
        assert_eq!(node_id(module.requests()[0].attributes()).zero_based(), 0);
        for request in &module.requests()[1..] {
            assert!(matches!(
                request.attributes().as_wire(),
                Ok(WireValue::Undefined)
            ));
        }

        let WireNodeCarrier::Ordinary { properties } = &image.nodes()[0] else {
            panic!("module attributes must retain their ordinary object");
        };
        assert_eq!(properties.len(), 2);
        assert_eq!(properties[0].key, image_key(slots[2]));
        assert_eq!(
            properties[0].value.as_wire(),
            Ok(&WireValue::String(narrow(b"oracle")))
        );
        assert_eq!(properties[1].key, image_key(slots[3]));
        assert_eq!(
            properties[1].value.as_wire(),
            Ok(&WireValue::String(narrow(b"rich")))
        );

        let exports = module.exports();
        assert_eq!(exports.len(), 3);
        assert_eq!(exports[0].export_type(), 0);
        assert_eq!(exports[0].local_variable_index(), Some(3));
        assert_eq!(exports[0].request_index(), None);
        assert_eq!(exports[0].local_name(), None);
        assert_eq!(exports[0].export_name(), slots[6]);
        assert_eq!(exports[1].export_type(), 1);
        assert_eq!(exports[1].local_variable_index(), None);
        assert_eq!(exports[1].request_index(), Some(2));
        assert_eq!(exports[1].local_name(), Some(slots[7]));
        assert_eq!(exports[1].export_name(), slots[8]);
        assert_eq!(exports[2].export_type(), 1);
        assert_eq!(exports[2].request_index(), Some(4));
        assert_eq!(
            exports[2].local_name(),
            Some(ImageAtom::Predefined(pinned(130)))
        );
        assert_eq!(exports[2].export_name(), slots[9]);

        assert_eq!(module.star_export_request_indices(), [3]);
        let imports = module.imports();
        assert_eq!(imports.len(), 3);
        assert_eq!(
            (
                imports[0].variable_index(),
                imports[0].is_star(),
                imports[0].import_name(),
                imports[0].request_index(),
            ),
            (0, false, ImageAtom::Predefined(pinned(22)), 0),
        );
        assert_eq!(
            (
                imports[1].variable_index(),
                imports[1].is_star(),
                imports[1].import_name(),
                imports[1].request_index(),
            ),
            (1, false, slots[7], 0),
        );
        assert_eq!(
            (
                imports[2].variable_index(),
                imports[2].is_star(),
                imports[2].import_name(),
                imports[2].request_index(),
            ),
            (2, true, ImageAtom::Predefined(pinned(130)), 1),
        );
        assert!(module.has_tla());
        assert_eq!(function_id(module.func_obj()).zero_based(), 0);

        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(vector.clone()),
        );
    }
}

#[test]
fn module_raw_export_type_is_preserved_but_boolean_bytes_are_canonicalized() {
    const EXPORT_TYPE_OFFSET: usize = 195;
    const IS_STAR_OFFSET: usize = 220;
    const HAS_TLA_OFFSET: usize = 224;

    let mut mutated = metadata_rich_module_vector();
    mutated[EXPORT_TYPE_OFFSET] = 0x7f;
    mutated[IS_STAR_OFFSET] = 0x7f;
    mutated[HAS_TLA_OFFSET] = 0x7f;

    let image = decode_image(&mutated).unwrap();
    let module = image.module(module_id(image.root())).unwrap();
    assert_eq!(module.exports()[1].export_type(), 0x7f);
    assert!(module.imports()[2].is_star());
    assert!(module.has_tla());

    let mut canonical = mutated;
    canonical[IS_STAR_OFFSET] = 1;
    canonical[HAS_TLA_OFFSET] = 1;
    for references in [false, true] {
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(canonical.clone()),
        );
    }
}

#[test]
fn compatible_module_ulebs_accept_non_minimal_counts_and_fields_then_write_canonically() {
    let mut canonical = vec![5, 0];
    canonical.extend_from_slice(&counted_module_record(0, 1, 0, 0));

    // Request count and the local export variable index are both zero in the
    // canonical vector. QuickJS accepts their two-byte spellings; Strict does
    // not, and the canonical writer must not retain the redundant byte.
    for (description, offset) in [("request count", 4), ("export variable index", 7)] {
        let mut non_minimal = canonical.clone();
        non_minimal.splice(offset..offset + 1, [0x80, 0x00]);

        assert_eq!(
            decode_image_with(&non_minimal, ReaderMode::Strict, IMAGE_LIMITS, true,),
            Err(BytecodeImageError::Wire(WireError::NonCanonicalUleb128 {
                offset
            })),
            "Strict accepted a non-minimal Module {description}",
        );

        let image = decode_image_with(
            &non_minimal,
            ReaderMode::QuickJsCompatible,
            IMAGE_LIMITS,
            true,
        )
        .unwrap_or_else(|error| panic!("compatible Module {description} must decode: {error}"));
        assert_eq!(
            encode_image(&image),
            Ok(canonical.clone()),
            "writer retained a non-minimal Module {description}",
        );
    }
}

#[test]
fn module_positive_int_boundary_has_typed_count_and_field_errors() {
    const FIRST_NON_POSITIVE_INT: u32 = 0x8000_0000;
    const ENCODED: [u8; 5] = [0x80, 0x80, 0x80, 0x80, 0x08];

    let mut request_count = vec![5, 0, BcTag::Module.to_byte(), 0];
    request_count.extend_from_slice(&ENCODED);

    let mut export_variable = vec![
        5,
        0,
        BcTag::Module.to_byte(),
        0, // name
        0, // requests
        1, // exports
        0, // local export type
    ];
    export_variable.extend_from_slice(&ENCODED);

    let cases = [
        (
            "request count",
            request_count,
            BytecodeImageError::ModuleCountOutOfRange {
                kind: ModuleResourceKind::Requests,
                offset: 4,
                count: FIRST_NON_POSITIVE_INT,
                maximum: i32::MAX as u32,
            },
        ),
        (
            "local export variable index",
            export_variable,
            BytecodeImageError::ModuleFieldOutOfRange {
                field: ModuleField::LocalExportVariable,
                offset: 7,
                value: FIRST_NON_POSITIVE_INT,
                maximum: i32::MAX as u32,
            },
        ),
    ];

    for (description, vector, expected) in cases {
        assert_eq!(
            decode_image(&vector),
            Err(expected),
            "wrong positive-int error for Module {description}",
        );
    }
}

#[test]
fn module_codec_preserves_relationally_invalid_metadata_without_linking_it() {
    let vector = vec![
        5,
        0,
        BcTag::Module.to_byte(),
        0,    // module name
        0,    // requests
        2,    // exports
        0,    // local export
        123,  // variable index, despite no function locals
        0,    // export name
        0x7f, // unknown non-local export type
        125,  // request index, despite no requests
        0,    // local name
        0,    // export name
        1,    // star exports
        126,  // request index, despite no requests
        1,    // imports
        127,  // variable index, despite no function locals
        1,    // is_star
        0,    // import name
        124,  // request index, despite no requests
        1,    // has_tla
        BcTag::Null.to_byte(),
    ];
    let image = decode_image(&vector).unwrap();
    let module = image.module(module_id(image.root())).unwrap();

    assert!(module.requests().is_empty());
    assert_eq!(module.exports()[0].local_variable_index(), Some(123));
    assert_eq!(module.exports()[1].export_type(), 0x7f);
    assert_eq!(module.exports()[1].request_index(), Some(125));
    assert_eq!(module.star_export_request_indices(), [126]);
    assert_eq!(module.imports()[0].variable_index(), 127);
    assert_eq!(module.imports()[0].request_index(), 124);
    assert_eq!(module.func_obj().as_wire(), Ok(&WireValue::Null));

    for references in [false, true] {
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(vector.clone()),
        );
    }
}

#[test]
fn module_writer_visits_request_children_before_tail_budget_checks() {
    let vector = vec![
        5,
        0,
        BcTag::Module.to_byte(),
        0, // module name
        1, // request count
        0, // request name
        BcTag::Array.to_byte(),
        1,
        BcTag::ObjectReference.to_byte(),
        0, // attributes Array refers to itself
        1, // export count
        0, // local export
        0, // variable index
        0, // export name
        0, // star exports
        0, // imports
        0, // has_tla
        BcTag::Null.to_byte(),
    ];
    let image = decode_image(&vector).unwrap();
    let limits = module_image_limits(
        ModuleLimits::new(256, 0, 256, 256),
        256,
        4096,
        4096,
        4096,
        4096,
    );

    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, limits),
        ),
        Err(BytecodeImageEncodeError::CircularReference {
            node: NodeId::from_zero_based(0),
        }),
    );
    assert_eq!(
        encode_bytecode_image(&image, BytecodeImageEncodeOptions::new(true, 65536, limits),),
        Err(BytecodeImageEncodeError::Module(
            ModuleBudgetError::ResourceLimit {
                kind: ModuleResourceKind::Exports,
                requested: 1,
                limit: 0,
            },
        )),
    );
}

#[test]
fn modules_functions_and_objects_share_one_authenticated_preorder_traversal() {
    let with_references = mixed_module_function_vector(true);
    let without_references = mixed_module_function_vector(false);
    assert!(matches!(
        decode_image_with(&with_references, ReaderMode::Strict, IMAGE_LIMITS, false,),
        Err(BytecodeImageError::Data(
            DecodeError::ObjectReferencesNotAllowed { .. }
        ))
    ));
    let image = decode_image(&with_references).unwrap();

    assert_eq!(image.modules().len(), 2);
    assert_eq!(image.functions().len(), 1);
    assert_eq!(image.nodes().len(), 2);
    assert_eq!(
        image.reference_table(),
        [NodeId::from_zero_based(0), NodeId::from_zero_based(1)]
    );

    let WireNodeCarrier::Array { elements } = &image.nodes()[0] else {
        panic!("mixed root must be an array");
    };
    let outer_id = module_id(&elements[0]);
    assert_eq!(outer_id.zero_based(), 0);
    assert_eq!(node_id(&elements[1]).zero_based(), 1);
    let outer = image.module(outer_id).unwrap();
    assert_eq!(outer.requests().len(), 1);
    assert_eq!(node_id(outer.requests()[0].attributes()).zero_based(), 1);

    let function = function_id(outer.func_obj());
    assert_eq!(function.zero_based(), 0);
    assert_eq!(function.source(), outer_id.source());
    let nested_id = module_id(&image.function(function).unwrap().constants()[0]);
    assert_eq!(nested_id.zero_based(), 1);
    assert_eq!(nested_id.source(), outer_id.source());
    assert_eq!(
        node_id(image.module(nested_id).unwrap().func_obj()).zero_based(),
        1
    );

    assert_eq!(encode_image(&image), Ok(with_references));
    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, IMAGE_LIMITS),
        ),
        Ok(without_references.clone()),
    );
    decode_image_with(&without_references, ReaderMode::Strict, IMAGE_LIMITS, false).unwrap();
}

#[test]
fn module_count_budget_is_exact_for_reader_writer_and_later_modules() {
    let single = counted_module_image();
    let single_image = decode_image(&single).unwrap();
    let exact = module_image_limits(MODULE_LIMITS, 1, 4096, 4096, 4096, 4096);
    assert!(decode_image_with(&single, ReaderMode::Strict, exact, true).is_ok());
    assert_eq!(
        encode_bytecode_image(
            &single_image,
            BytecodeImageEncodeOptions::new(true, 65536, exact),
        ),
        Ok(single),
    );

    let zero = module_image_limits(MODULE_LIMITS, 0, 4096, 4096, 4096, 4096);
    let expected_first = BytecodeImageBudgetError::ResourceLimit {
        kind: BytecodeImageResourceKind::Modules,
        requested: 1,
        limit: 0,
    };
    assert_eq!(
        decode_image_with(&counted_module_image(), ReaderMode::Strict, zero, true,),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::Modules,
            requested: 1,
            limit: 0,
        }),
    );
    assert_eq!(
        encode_bytecode_image(
            &single_image,
            BytecodeImageEncodeOptions::new(true, 65536, zero),
        ),
        Err(BytecodeImageEncodeError::Budget(expected_first)),
    );

    let siblings = sibling_counted_modules();
    let siblings_image = decode_image(&siblings).unwrap();
    let expected_later = BytecodeImageBudgetError::ResourceLimit {
        kind: BytecodeImageResourceKind::Modules,
        requested: 2,
        limit: 1,
    };
    assert_eq!(
        decode_image_with(&siblings, ReaderMode::Strict, exact, true),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::Modules,
            requested: 2,
            limit: 1,
        }),
    );
    assert_eq!(
        encode_bytecode_image(
            &siblings_image,
            BytecodeImageEncodeOptions::new(true, 65536, exact),
        ),
        Err(BytecodeImageEncodeError::Budget(expected_later)),
    );
}

#[test]
fn every_module_table_budget_has_exact_off_by_one_and_remaining_attribution() {
    let single = counted_module_image();
    let single_image = decode_image(&single).unwrap();
    let siblings = sibling_counted_modules();
    let siblings_image = decode_image(&siblings).unwrap();
    let cases = [
        (
            ModuleResourceKind::Requests,
            BytecodeImageResourceKind::TotalModuleRequests,
        ),
        (
            ModuleResourceKind::Exports,
            BytecodeImageResourceKind::TotalModuleExports,
        ),
        (
            ModuleResourceKind::StarExports,
            BytecodeImageResourceKind::TotalModuleStarExports,
        ),
        (
            ModuleResourceKind::Imports,
            BytecodeImageResourceKind::TotalModuleImports,
        ),
    ];

    let exact = module_image_limits(ModuleLimits::new(1, 1, 1, 1), 1, 1, 1, 1, 1);
    assert!(decode_image_with(&single, ReaderMode::Strict, exact, true).is_ok());
    assert_eq!(
        encode_bytecode_image(
            &single_image,
            BytecodeImageEncodeOptions::new(true, 65536, exact),
        ),
        Ok(single.clone()),
    );

    for (module_kind, aggregate_kind) in cases {
        let per_record = module_image_limits(
            one_per_module_limit(module_kind, 0),
            256,
            4096,
            4096,
            4096,
            4096,
        );
        let per_record_error = ModuleBudgetError::ResourceLimit {
            kind: module_kind,
            requested: 1,
            limit: 0,
        };
        assert_eq!(
            decode_image_with(&single, ReaderMode::Strict, per_record, true),
            Err(BytecodeImageError::Module(per_record_error.clone())),
            "wrong reader per-record result for {module_kind:?}",
        );
        assert_eq!(
            encode_bytecode_image(
                &single_image,
                BytecodeImageEncodeOptions::new(true, 65536, per_record),
            ),
            Err(BytecodeImageEncodeError::Module(per_record_error)),
            "wrong writer per-record result for {module_kind:?}",
        );

        let aggregate_zero = one_module_aggregate_limit(aggregate_kind, 0);
        assert_eq!(
            decode_image_with(&single, ReaderMode::Strict, aggregate_zero, true),
            Err(BytecodeImageError::ResourceLimit {
                kind: aggregate_kind,
                requested: 1,
                limit: 0,
            }),
            "wrong reader aggregate result for {aggregate_kind:?}",
        );
        assert_eq!(
            encode_bytecode_image(
                &single_image,
                BytecodeImageEncodeOptions::new(true, 65536, aggregate_zero),
            ),
            Err(BytecodeImageEncodeError::Budget(
                BytecodeImageBudgetError::ResourceLimit {
                    kind: aggregate_kind,
                    requested: 1,
                    limit: 0,
                }
            )),
            "wrong writer aggregate result for {aggregate_kind:?}",
        );

        let aggregate_one = one_module_aggregate_limit(aggregate_kind, 1);
        assert_eq!(
            decode_image_with(&siblings, ReaderMode::Strict, aggregate_one, true),
            Err(BytecodeImageError::ResourceLimit {
                kind: aggregate_kind,
                requested: 2,
                limit: 1,
            }),
            "wrong later-module reader result for {aggregate_kind:?}",
        );
        assert_eq!(
            encode_bytecode_image(
                &siblings_image,
                BytecodeImageEncodeOptions::new(true, 65536, aggregate_one),
            ),
            Err(BytecodeImageEncodeError::Budget(
                BytecodeImageBudgetError::ResourceLimit {
                    kind: aggregate_kind,
                    requested: 2,
                    limit: 1,
                }
            )),
            "wrong later-module writer result for {aggregate_kind:?}",
        );
    }
}
