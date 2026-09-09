use super::*;

#[test]
fn sab_transport_whole_image_oracle_preserves_topology_and_normalizes_tokens() {
    const FIRST_TOKEN: u64 = 0x0123_4567_89ab_cdef;
    const RENAMED_TOKEN: u64 = 0xfedc_ba98_7654_3210;

    let first_wire = function_bytecode_sab_reference_wire(FIRST_TOKEN);
    assert_eq!(first_wire.len(), 50);
    let first = decode_sab_image(&first_wire, &[FIRST_TOKEN], ReaderMode::Strict, true)
        .expect("pinned FunctionBytecode/SAB oracle must decode");
    let first_snapshot = snapshot_sab_image(&first);

    assert_eq!(first_snapshot.atoms, []);
    assert_eq!(
        first_snapshot.root,
        SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(0)))
    );
    assert_eq!(first_snapshot.references, [0, 1, 2]);
    assert_eq!(first_snapshot.shared_backing_count, 1);
    assert_eq!(first_snapshot.module_count, 0);
    assert_eq!(first_snapshot.functions.len(), 1);
    assert_eq!(
        first_snapshot.functions[0].envelope.code().as_bytes(),
        [0xbb, 0x2a, 0xcb, 0x28]
    );
    assert!(first_snapshot.functions[0].envelope.debug().is_none());
    assert!(first_snapshot.functions[0].constants.is_empty());
    assert_eq!(
        first_snapshot.nodes.as_slice(),
        [
            SabImageNodeSnapshot::Array(vec![
                SabImageValueSnapshot::Function(0),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(1))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
            ]),
            SabImageNodeSnapshot::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 4,
                byte_offset: 0,
                buffer: 2,
            },
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
        ]
    );
    assert_eq!(
        encode_image(first.test_image()),
        Err(BytecodeImageEncodeError::ArchivedBackingContextRequired {
            node: NodeId::from_zero_based(2),
        })
    );

    let renamed_wire = function_bytecode_sab_reference_wire(RENAMED_TOKEN);
    let renamed = decode_sab_image(&renamed_wire, &[RENAMED_TOKEN], ReaderMode::Strict, true)
        .expect("alpha-renamed native token must decode to the same semantic image");
    assert_eq!(snapshot_sab_image(&renamed), first_snapshot);

    for (archive, raw_token) in [(&first, FIRST_TOKEN), (&renamed, RENAMED_TOKEN)] {
        let debug = format!("{archive:#?}");
        for spelling in [
            raw_token.to_string(),
            format!("{raw_token:x}"),
            format!("{raw_token:X}"),
            format!("0x{raw_token:016x}"),
            format!("0x{raw_token:016X}"),
        ] {
            assert!(
                !debug.contains(&spelling),
                "ArchivedBytecodeImage Debug leaked native token spelling {spelling}"
            );
        }
    }
}

#[test]
fn sab_transport_whole_image_canonicalizes_repeated_and_distinct_backings() {
    const FIRST_TOKEN: u64 = 0x0123_4567_89ab_cdef;
    const SECOND_TOKEN: u64 = 0xfedc_ba98_7654_3210;
    let limits = sab_image_limits_for(2, 2, 8);

    let repeated_wire = two_sab_records_wire(FIRST_TOKEN, FIRST_TOKEN);
    let repeated = decode_sab_image_with_limits(
        &repeated_wire,
        &[FIRST_TOKEN, FIRST_TOKEN],
        ReaderMode::Strict,
        true,
        limits,
    )
    .expect("two complete SAB records with one token must share one archived backing");
    let repeated = snapshot_sab_image(&repeated);
    assert_eq!(repeated.references, [0, 1, 2]);
    assert_eq!(repeated.shared_backing_count, 1);
    assert_eq!(
        repeated.root,
        SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(0)))
    );
    assert_eq!(
        repeated.nodes,
        [
            SabImageNodeSnapshot::Array(vec![
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(1))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
            ]),
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
        ]
    );

    let distinct_wire = two_sab_records_wire(FIRST_TOKEN, SECOND_TOKEN);
    let distinct = decode_sab_image_with_limits(
        &distinct_wire,
        &[FIRST_TOKEN, SECOND_TOKEN],
        ReaderMode::Strict,
        true,
        limits,
    )
    .expect("two complete SAB records with distinct tokens must retain distinct backings");
    let distinct = snapshot_sab_image(&distinct);
    assert_eq!(distinct.references, [0, 1, 2]);
    assert_eq!(distinct.shared_backing_count, 2);
    assert_eq!(
        distinct.nodes,
        [
            SabImageNodeSnapshot::Array(vec![
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(1))),
                SabImageValueSnapshot::Data(WireValue::Node(NodeId::from_zero_based(2))),
            ]),
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 0,
                capacity: 4,
                growable: false,
            },
            SabImageNodeSnapshot::SharedArrayBuffer {
                byte_length: 4,
                max_byte_length: None,
                backing: 1,
                capacity: 4,
                growable: false,
            },
        ]
    );
}

#[test]
fn sab_transport_whole_image_rejects_split_and_truncated_inputs() {
    const TOKEN: u64 = 0x0123_4567_89ab_cdef;
    let wire = function_bytecode_sab_reference_wire(TOKEN);

    assert_eq!(
        decode_sab_image(&wire, &[], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableTooShort {
                offset: 38,
                ordinal: 0,
                entry_count: 0,
            })
        ))
    );
    assert_eq!(
        decode_sab_image(&wire, &[TOKEN ^ 1], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableTokenMismatch {
                offset: 38,
                ordinal: 0,
            })
        ))
    );
    assert_eq!(
        decode_sab_image(&wire, &[TOKEN, TOKEN ^ 1], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableHasExtra {
                consumed: 1,
                entry_count: 2,
            })
        ))
    );
    assert_eq!(
        decode_sab_image(&wire[..45], &[TOKEN], ReaderMode::Strict, true),
        Err(BytecodeImageError::Data(DecodeError::Wire(
            WireError::Truncated {
                offset: 38,
                needed: 8,
                remaining: 7,
            }
        )))
    );
}

#[test]
fn sab_transport_whole_image_preserves_reference_and_finalization_policy() {
    const TOKEN: u64 = 0x0123_4567_89ab_cdef;
    let wire = function_bytecode_sab_reference_wire(TOKEN);
    let writer_occurrences = [NativeSabToken::from_test_bits(TOKEN)];
    assert_eq!(
        decode_bytecode_image_with_sab_transport(
            SabTransportInput::new(&wire, &writer_occurrences),
            ReaderMode::Strict,
            TEST_LIMITS,
            IMAGE_LIMITS,
            true,
        ),
        Err(BytecodeImageError::Data(DecodeError::Graph(
            GraphError::ResourceLimit {
                kind: GraphResourceKind::SharedArrayBufferOccurrences,
                requested: 1,
                limit: 0,
            }
        )))
    );
    assert_eq!(
        decode_sab_image(&wire, &[TOKEN], ReaderMode::Strict, false),
        Err(BytecodeImageError::Data(
            DecodeError::ObjectReferencesNotAllowed { offset: 46 }
        ))
    );

    let mut trailing = wire;
    trailing.push(0xff);
    assert_eq!(
        decode_sab_image(&trailing, &[TOKEN, TOKEN ^ 1], ReaderMode::Strict, true),
        Err(BytecodeImageError::Wire(WireError::TrailingBytes {
            offset: 50,
            remaining: 1,
        }))
    );
    assert_eq!(
        decode_sab_image(
            &trailing,
            &[TOKEN, TOKEN ^ 1],
            ReaderMode::QuickJsCompatible,
            true,
        ),
        Err(BytecodeImageError::Data(
            DecodeError::SharedArrayBufferArchive(SabArchiveError::SideTableHasExtra {
                consumed: 1,
                entry_count: 2,
            })
        ))
    );
}

#[test]
fn image_writer_requires_archived_context_for_reachable_shared_backings_only() {
    const TOKEN: u64 = 0xfeed_face_dead_beef;
    let mut wire = vec![
        5,
        0,
        BcTag::SharedArrayBuffer.to_byte(),
        4,
        0xff,
        0xff,
        0xff,
        0xff,
        0x0f,
    ];
    wire.extend_from_slice(&TOKEN.to_le_bytes());
    let writer_occurrences = [NativeSabToken::from_test_bits(TOKEN)];
    let archive = decode_graph_with_sab_transport(
        SabTransportInput::new(&wire, &writer_occurrences),
        ReaderMode::Strict,
        TEST_LIMITS,
        GRAPH_LIMITS.with_shared_array_buffers(1, 1, 4, 4),
        true,
    )
    .unwrap();
    let WireNodeCarrier::SharedArrayBuffer {
        byte_length,
        max_byte_length,
        backing,
    } = archive.test_graph().nodes[0]
    else {
        panic!("transport must archive one SharedArrayBuffer node");
    };

    let image_with_graph = |nodes: Vec<WireNodeCarrier<ImageValue, ImageKey>>, root: WireValue| {
        let machine = DataMachine::<ImageValue, ImageKey>::new(GRAPH_LIMITS, true).unwrap();
        super::BytecodeImage::new(
            machine.source(),
            super::ImageAtomSummary::new(0, Box::default()),
            nodes.into_boxed_slice(),
            Box::default(),
            Box::default(),
            Box::default(),
            ImageValue::from_wire(root),
        )
    };
    let shared_node = || WireNodeCarrier::SharedArrayBuffer {
        byte_length,
        max_byte_length,
        backing,
    };

    let direct = image_with_graph(
        vec![shared_node()],
        WireValue::Node(NodeId::from_zero_based(0)),
    );
    assert_eq!(
        encode_image(&direct),
        Err(BytecodeImageEncodeError::ArchivedBackingContextRequired {
            node: NodeId::from_zero_based(0),
        })
    );

    let viewed = image_with_graph(
        vec![
            WireNodeCarrier::TypedArray {
                kind: TypedArrayKind::Uint8,
                length: 4,
                byte_offset: 0,
                buffer: NodeId::from_zero_based(1),
            },
            shared_node(),
        ],
        WireValue::Node(NodeId::from_zero_based(0)),
    );
    assert_eq!(
        encode_image(&viewed),
        Err(BytecodeImageEncodeError::ArchivedBackingContextRequired {
            node: NodeId::from_zero_based(1),
        })
    );

    let unreachable = image_with_graph(vec![shared_node()], WireValue::Int32(42));
    assert_eq!(encode_image(&unreachable), Ok(vec![5, 0, 5, 84]));
}
