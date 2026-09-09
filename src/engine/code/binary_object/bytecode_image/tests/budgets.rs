use super::*;

#[test]
fn whole_image_limits_bound_functions_depth_and_aggregate_payloads() {
    let answer = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    assert_eq!(
        decode_image_with(
            &answer,
            ReaderMode::Strict,
            bounded_image_limits(0, 256, 4096, 16384),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::Functions,
            requested: 1,
            limit: 0,
        })
    );
    assert_eq!(
        decode_image_with(
            &answer,
            ReaderMode::Strict,
            bounded_image_limits(256, 256, 4096, 3),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalCodeBytes,
            requested: 4,
            limit: 3,
        })
    );

    let nested = bytes(
        "05020a6f757465720a696e6e65720c000200a80100010002000001090100000000be00bb28edb5edcb280c430200e60301010101010001080200010000000000e05e0000cfc7be00280c430200e803010001020001000e010001000000001000640000cf9b116500000e64000028",
    );
    assert_eq!(
        decode_image_with(
            &nested,
            ReaderMode::Strict,
            bounded_image_limits(256, 1, 4096, 16384),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::WholeDepth,
            requested: 2,
            limit: 1,
        })
    );

    let ancestor = bytes("050102660801e6030c000200a80100010001000001040100000000bd00cb281300");
    assert_eq!(
        decode_image_with(
            &ancestor,
            ReaderMode::Strict,
            bounded_image_limits(256, 256, 0, 16384),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalConstantPoolEntries,
            requested: 1,
            limit: 0,
        })
    );
}

#[test]
fn aggregate_limits_reject_before_avoidable_prefix_work() {
    let mut invalid_code = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    invalid_code[21] = 0;
    assert_eq!(
        decode_image_with(
            &invalid_code,
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalCodeBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalCodeBytes,
            requested: 4,
            limit: 0,
        })
    );
    assert_eq!(
        decode_image_with(
            &invalid_code[..21],
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalCodeBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalCodeBytes,
            requested: 4,
            limit: 0,
        })
    );

    // The local count is present but its table is deliberately absent. The
    // aggregate count is known from the header and wins before any reserve or
    // child-field read.
    let truncated_local = bytes("05000c000200a801000100010000000401");
    assert_eq!(
        decode_image_with(
            &truncated_local,
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalLocalVariables,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalLocalVariables,
            requested: 1,
            limit: 0,
        })
    );

    // Debug lengths are both known before either slice is copied. Omitting the
    // final source byte therefore cannot mask the whole-image budget failure.
    let mut truncated_debug = vec![5, 0];
    truncated_debug.extend_from_slice(&debug_record());
    assert_eq!(truncated_debug.pop(), Some(0xbb));
    assert_eq!(
        decode_image_with(
            &truncated_debug,
            ReaderMode::Strict,
            one_aggregate_limit(
                ENVELOPE_LIMITS,
                BytecodeImageResourceKind::TotalDebugBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::TotalDebugBytes,
            requested: 2,
            limit: 0,
        })
    );

    // Equal per-function and whole limits retain the established envelope
    // error instead of relabeling it as an aggregate failure.
    let envelope_zero_code = FunctionEnvelopeLimits::new(
        256,
        256,
        256,
        4096,
        4096,
        8192,
        CodeLimits::new(0, 4096, 4096),
    );
    let answer = bytes("05000c000200a80100010001000000040100000000bb2acb28");
    assert_eq!(
        decode_image_with(
            &answer,
            ReaderMode::Strict,
            one_aggregate_limit(
                envelope_zero_code,
                BytecodeImageResourceKind::TotalCodeBytes,
                0,
            ),
            true,
        ),
        Err(BytecodeImageError::Envelope(FunctionEnvelopeError::Code(
            CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested: 4,
                limit: 0,
            }
        )))
    );
}

#[test]
fn aggregate_remaining_budget_bounds_each_later_function_resource() {
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
        let image = sibling_function_array(&record);
        decode_image(&image).unwrap_or_else(|error| {
            panic!("synthetic {kind:?} boundary vector must be valid: {error}")
        });
        assert_eq!(
            decode_image_with(
                &image,
                ReaderMode::Strict,
                one_aggregate_limit(ENVELOPE_LIMITS, kind, limit),
                true,
            ),
            Err(BytecodeImageError::ResourceLimit {
                kind,
                requested,
                limit,
            }),
            "wrong remaining-budget result for {kind:?}",
        );
    }
}

#[test]
fn parent_property_key_errors_precede_recursive_whole_depth_limits() {
    let shallow = bounded_image_limits(256, 1, 4096, 16384);
    assert_eq!(
        decode_image_with(
            &[5, 0, BcTag::Object.to_byte(), 1],
            ReaderMode::Strict,
            shallow,
            true,
        ),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 4,
            needed: 1,
            remaining: 0,
        }))
    );
}

#[test]
fn module_metadata_errors_precede_their_child_whole_depth_check() {
    let tag = BcTag::Module.to_byte();
    let first_atom = AtomIndexSpace::new(BinaryObjectMode::Bytecode, 0)
        .unwrap()
        .first_atom();

    assert_eq!(
        decode_image(&[5, 0, tag]),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 3,
            needed: 1,
            remaining: 0,
        }))
    );
    assert_eq!(
        decode_image(&[5, 0, tag, 0xe6, 0x03]),
        Err(BytecodeImageError::Wire(WireError::InvalidAtomIndex {
            offset: 5,
            index: first_atom,
            first_atom,
            atom_count: 0,
        }))
    );
    assert_eq!(
        decode_image(&[5, 0, tag, 0x80, 0]),
        Err(BytecodeImageError::Wire(WireError::NonCanonicalUleb128 {
            offset: 3,
        }))
    );

    let shallow = bounded_image_limits(256, 1, 4096, 16384);
    // The request name is parent metadata, so its failure wins before the
    // whole-depth check for the attributes child.
    assert_eq!(
        decode_image_with(&[5, 0, tag, 0, 1], ReaderMode::Strict, shallow, true,),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 5,
            needed: 1,
            remaining: 0,
        }))
    );
    assert_eq!(
        decode_image_with(
            &[5, 0, tag, 0, 1, 0xe6, 0x03],
            ReaderMode::Strict,
            shallow,
            true,
        ),
        Err(BytecodeImageError::Wire(WireError::InvalidAtomIndex {
            offset: 7,
            index: first_atom,
            first_atom,
            atom_count: 0,
        }))
    );
    assert_eq!(
        decode_image_with(&[5, 0, tag, 0, 1, 0], ReaderMode::Strict, shallow, true,),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::WholeDepth,
            requested: 2,
            limit: 1,
        })
    );

    // Exports, star exports, imports, and has_tla are likewise consumed before
    // the func_obj child receives its own depth check.
    assert_eq!(
        decode_image_with(&[5, 0, tag, 0, 0], ReaderMode::Strict, shallow, true,),
        Err(BytecodeImageError::Wire(WireError::Truncated {
            offset: 5,
            needed: 1,
            remaining: 0,
        }))
    );
    assert_eq!(
        decode_image_with(
            &[5, 0, tag, 0, 0, 0, 0, 0, 0],
            ReaderMode::Strict,
            shallow,
            true,
        ),
        Err(BytecodeImageError::ResourceLimit {
            kind: BytecodeImageResourceKind::WholeDepth,
            requested: 2,
            limit: 1,
        })
    );
}
