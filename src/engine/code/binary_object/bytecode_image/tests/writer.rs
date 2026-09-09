use super::*;

#[test]
fn canonical_writer_round_trips_pinned_quickjs_bytecode_images_byte_exactly() {
    let vectors = [
        bytes("05000c000200a80100010001000000040100000000bb2acb28"),
        bytes(
            "05020a6f757465720a696e6e65720c000200a80100010002000001090100000000be00bb28edb5edcb280c430200e60301010101010001080200010000000000e05e0000cfc7be00280c430200e803010001020001000e010001000000001000640000cf9b116500000e64000028",
        ),
        bytes(
            "05011074656d706c6174650802360c000200a80100010002000002070100000000be00bd01edcb280c0202000001000101000000020100010000cf280b010702780b0107027802e6031301",
        ),
        bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300"),
    ];

    for vector in vectors {
        let image = decode_image(&vector).unwrap();
        assert_eq!(encode_image(&image), Ok(vector));
    }
}

#[test]
fn canonical_writer_preserves_pinned_debug_and_strip_shapes_byte_exactly() {
    let vectors = [
        bytes(
            "05041e7772697465722d666c6167732e6a730a6f7574657208736565640c616e737765720c000600a801000100020000010801aa01000000be00bb28edeccb28e603080000000408040708000c430600e803010001010100010301ea03010040be0028e6030400010d024f66756e6374696f6e206f75746572287365656429207b0a202072657475726e2066756e6374696f6e20616e737765722829207b0a2020202072657475726e2073656564202b20323b0a20207d3b0a7d0c430600ec03000000020001000400ea03000100dbb59b28e60308010903040c0a07172c66756e6374696f6e20616e737765722829207b0a2020202072657475726e2073656564202b20323b0a20207d",
        ),
        bytes(
            "05041e7772697465722d666c6167732e6a730a6f7574657208736565640c616e737765720c000600a801000100020000010801aa01000000be00bb28edeccb28e603080000000408040708000c430600e803010001010100010301ea03010040be0028e6030400010d02000c430600ec03000000020001000400ea03000100dbb59b28e60308010903040c0a071700",
        ),
        bytes(
            "05020a6f757465720c616e737765720c000200a80100010002000001080100000000be00bb28edeccb280c430200e60301000101010001030100010040be00280c430200e80300000002000100040000000100dbb59b28",
        ),
    ];

    for vector in vectors {
        let image = decode_image(&vector).unwrap();
        assert_eq!(encode_image(&image), Ok(vector));
    }
}

#[test]
fn canonical_writer_rebuilds_reference_state_and_rejects_too_small_output() {
    let vector = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    let image = decode_image(&vector).unwrap();
    assert_eq!(encode_image(&image), Ok(vector.clone()));

    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(true, vector.len() - 1, IMAGE_LIMITS),
        ),
        Err(BytecodeImageEncodeError::Wire(WireError::ResourceLimit {
            kind: ResourceKind::OutputBytes,
            requested: vector.len(),
            limit: vector.len() - 1,
        }))
    );
    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, IMAGE_LIMITS),
        ),
        Err(BytecodeImageEncodeError::CircularReference {
            node: NodeId::from_zero_based(0),
        })
    );
}

#[test]
fn canonical_writer_filters_non_string_properties_without_visiting_their_values() {
    // Root ID 0 owns `keep: 42`, a symbol-keyed self-cycling Array (ID 1),
    // and a private-keyed alias to that Array. Pinned JS_WriteObject filters
    // both non-string keys before visiting either value, so refs-off writing
    // succeeds and exactly matches the authenticated public-C-API oracle.
    let input = bytes("0501086b6565700803e6030554ce0309011301ca031301");
    let expected = bytes("0501086b6565700801e6030554");
    let image = decode_image(&input).unwrap();

    for references in [false, true] {
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(references, 65536, IMAGE_LIMITS),
            ),
            Ok(expected.clone()),
        );
    }
}

#[test]
fn canonical_writer_prunes_unused_atoms_and_expands_acyclic_aliases_without_references() {
    let unused_atom_input = header_bytes(&[narrow(b"unused")], &quickjs_42_record());
    let answer = decode_image(&unused_atom_input).unwrap();
    assert_eq!(
        encode_image(&answer),
        Ok(bytes("05000c000200a80100010001000000040100000000bb2acb28")),
    );

    // Root ID 0 contains one Array (ID 1) twice. References-on preserves the
    // alias spelling; references-off performs the same acyclic expansion as
    // pinned JS_WriteObject without JS_WRITE_OBJ_REFERENCE.
    let aliased = bytes("050008020109010554031301");
    let image = decode_image(&aliased).unwrap();
    assert_eq!(encode_image(&image), Ok(aliased));
    assert_eq!(
        encode_bytecode_image(
            &image,
            BytecodeImageEncodeOptions::new(false, 65536, IMAGE_LIMITS),
        ),
        Ok(bytes("0500080201090105540309010554")),
    );
}

#[test]
fn canonical_writer_shares_decoder_aggregate_error_attribution() {
    let cases = [
        (
            constant_record(),
            BytecodeImageResourceKind::TotalConstantPoolEntries,
            1,
            2,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalLocalVariables,
            1,
            2,
        ),
        (
            closure_record(),
            BytecodeImageResourceKind::TotalClosureVariables,
            1,
            2,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalCodeBytes,
            4,
            8,
        ),
        (
            quickjs_42_record(),
            BytecodeImageResourceKind::TotalInstructions,
            3,
            4,
        ),
        (
            atom_relocation_record(),
            BytecodeImageResourceKind::TotalAtomRelocations,
            1,
            2,
        ),
        (
            debug_record(),
            BytecodeImageResourceKind::TotalDebugBytes,
            2,
            4,
        ),
    ];

    for (record, kind, limit, requested) in cases {
        let image = decode_image(&sibling_function_array(&record)).unwrap();
        assert_eq!(
            encode_bytecode_image(
                &image,
                BytecodeImageEncodeOptions::new(
                    true,
                    65536,
                    one_aggregate_limit(ENVELOPE_LIMITS, kind, limit),
                ),
            ),
            Err(BytecodeImageEncodeError::Budget(
                BytecodeImageBudgetError::ResourceLimit {
                    kind,
                    requested,
                    limit,
                }
            )),
            "wrong writer remaining-budget result for {kind:?}",
        );
    }

    // Equal per-function and aggregate caps remain a per-function error, just
    // like the decoder contract above; only a strictly smaller remaining
    // aggregate budget changes the diagnostic owner.
    let envelope_zero_code = FunctionEnvelopeLimits::new(
        256,
        256,
        256,
        4096,
        4096,
        8192,
        CodeLimits::new(0, 4096, 4096),
    );
    let answer =
        decode_image(&bytes("05000c000200a80100010001000000040100000000bb2acb28")).unwrap();
    assert_eq!(
        encode_bytecode_image(
            &answer,
            BytecodeImageEncodeOptions::new(
                true,
                65536,
                one_aggregate_limit(
                    envelope_zero_code,
                    BytecodeImageResourceKind::TotalCodeBytes,
                    0,
                ),
            ),
        ),
        Err(BytecodeImageEncodeError::Envelope(
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested: 4,
                limit: 0,
            })
        )),
    );
}
