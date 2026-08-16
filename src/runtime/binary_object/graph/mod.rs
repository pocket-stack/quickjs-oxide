//! Validated, heap-independent BC5 data-object graphs.
//!
//! This staging layer proves framing, bounds, identity, and cycle semantics
//! before a later milestone is allowed to allocate or publish runtime objects.

pub(in crate::runtime) mod decode;
pub(in crate::runtime) mod encode;
pub(in crate::runtime) mod model;

#[cfg(test)]
mod tests {
    use super::super::wire::{ReaderMode, WireLimits};
    use super::decode::decode_graph;
    use super::encode::{GraphEncodeOptions, encode_graph};
    use super::model::GraphLimits;

    const WIRE_LIMITS: WireLimits = WireLimits::new(4096, 32, 1024, 2048);
    const GRAPH_LIMITS: GraphLimits =
        GraphLimits::new(64, 64, 32, 128, 256, 1024, 2048, 1024, 2048);

    fn rewrite(bytes: &[u8], references: bool, mode: ReaderMode) -> Vec<u8> {
        let graph = decode_graph(bytes, mode, WIRE_LIMITS, GRAPH_LIMITS, references).unwrap();
        encode_graph(
            &graph,
            GraphEncodeOptions::new(references, 4096, GRAPH_LIMITS),
        )
        .unwrap()
    }

    #[test]
    fn exact_quickjs_object_and_reference_vectors_cross_the_pure_graph() {
        for (bytes, references) in [
            (&[5, 1, 2, b'x', 8, 1, 2, 5, 2][..], false),
            // Data mode keeps `if` in the header even though bytecode mode can
            // refer to its pinned atom directly as raw index four.
            (&[5, 1, 4, b'i', b'f', 8, 1, 2, 5, 84][..], false),
            (&[5, 0, 9, 2, 8, 0, 19, 1][..], true),
            (&[5, 1, 8, b's', b'e', b'l', b'f', 8, 1, 2, 19, 0][..], true),
        ] {
            assert_eq!(rewrite(bytes, references, ReaderMode::Strict), bytes);
        }
    }

    #[test]
    fn quickjs_compatible_bigint_input_rewrites_canonically() {
        assert_eq!(
            rewrite(&[5, 0, 10, 1, 0], false, ReaderMode::QuickJsCompatible),
            [5, 0, 10, 0]
        );
    }

    #[test]
    fn exact_quickjs_array_buffer_vectors_cross_the_pure_graph() {
        for (bytes, references) in [
            (
                &[5, 0, 15, 4, 0xff, 0xff, 0xff, 0xff, 0x0f, 1, 2, 3, 4][..],
                false,
            ),
            (&[5, 0, 15, 4, 4, 1, 2, 3, 4][..], false),
            (&[5, 0, 15, 4, 8, 1, 2, 3, 4][..], false),
            (
                &[
                    5, 0, 9, 2, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x12, 0x34, 19, 1,
                ][..],
                true,
            ),
        ] {
            assert_eq!(rewrite(bytes, references, ReaderMode::Strict), bytes);
        }
    }

    #[test]
    fn exact_quickjs_typed_array_vectors_cross_the_pure_graph() {
        for (bytes, references) in [
            (
                &[
                    5, 0, 14, 4, 2, 2, 15, 8, 0xff, 0xff, 0xff, 0xff, 0x0f, 0, 0, 0, 0, 0, 0, 0, 0,
                ][..],
                true,
            ),
            (
                &[
                    5, 0, 9, 2, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x11, 0x22, 14, 2, 1, 1, 19, 1,
                ][..],
                true,
            ),
            (
                &[
                    5, 0, 9, 2, 14, 2, 1, 1, 15, 2, 0xff, 0xff, 0xff, 0xff, 0x0f, 0x11, 0x22, 19, 1,
                ][..],
                true,
            ),
            (
                &[
                    5, 0, 9, 2, 14, 2, 2, 0, 15, 8, 0xff, 0xff, 0xff, 0xff, 0x0f, 0, 0, 0, 0, 0, 0,
                    0, 0, 14, 3, 2, 2, 19, 2,
                ][..],
                true,
            ),
            (
                &[5, 0, 14, 4, 2, 4, 15, 8, 16, 0, 0, 0, 0, 0, 0, 0, 0][..],
                false,
            ),
        ] {
            assert_eq!(rewrite(bytes, references, ReaderMode::Strict), bytes);
        }
    }

    #[test]
    fn compatible_typed_array_lengths_rewrite_canonically() {
        assert_eq!(
            rewrite(
                &[
                    5, 0, 14, 2, 0x80, 0, 0x80, 0, 15, 0x80, 0, 0xff, 0xff, 0xff, 0xff, 0x0f,
                ],
                false,
                ReaderMode::QuickJsCompatible,
            ),
            [5, 0, 14, 2, 0, 0, 15, 0, 0xff, 0xff, 0xff, 0xff, 0x0f]
        );
    }

    #[test]
    fn exact_quickjs_object_value_vectors_cross_the_pure_graph() {
        for (bytes, references) in [
            (&[5, 0, 18, 3][..], false),
            (&[5, 0, 18, 5, 84][..], true),
            (&[5, 0, 18, 6, 0, 0, 0, 0, 0, 0, 0, 128][..], false),
            (&[5, 0, 18, 6, 66, 0, 0, 0, 0, 0, 248, 127][..], true),
            (&[5, 0, 18, 7, 6, b'a', b'b', b'c'][..], true),
            (&[5, 0, 18, 10, 1, 1][..], false),
            (&[5, 0, 9, 2, 18, 5, 84, 19, 1][..], true),
        ] {
            assert_eq!(rewrite(bytes, references, ReaderMode::Strict), bytes);
        }
    }

    #[test]
    fn object_value_reader_aliases_rewrite_to_canonical_object_identity() {
        for (input, expected) in [
            (&[5, 0, 18, 8, 0][..], &[5, 0, 8, 0][..]),
            (
                &[5, 0, 9, 2, 18, 8, 0, 19, 2][..],
                &[5, 0, 9, 2, 8, 0, 19, 1][..],
            ),
            (
                &[5, 0, 9, 2, 18, 15, 0, 255, 255, 255, 255, 15, 19, 2][..],
                &[5, 0, 9, 2, 15, 0, 255, 255, 255, 255, 15, 19, 1][..],
            ),
            (
                &[5, 0, 9, 2, 18, 19, 0, 19, 1][..],
                &[5, 0, 9, 2, 19, 0, 19, 0][..],
            ),
            (
                &[5, 1, 2, b'x', 8, 1, 2, 18, 19, 0][..],
                &[5, 1, 2, b'x', 8, 1, 2, 19, 0][..],
            ),
            (
                &[5, 0, 9, 2, 18, 18, 5, 2, 19, 2][..],
                &[5, 0, 9, 2, 18, 5, 2, 19, 1][..],
            ),
        ] {
            assert_eq!(rewrite(input, true, ReaderMode::Strict), expected);
        }
        assert_eq!(
            rewrite(&[5, 0, 18, 8, 0], false, ReaderMode::Strict),
            [5, 0, 8, 0]
        );
    }

    #[test]
    fn compatible_object_value_payload_lengths_rewrite_canonically() {
        for (input, expected) in [
            (
                &[5, 0, 18, 7, 0x86, 0, b'a', b'b', b'c'][..],
                &[5, 0, 18, 7, 6, b'a', b'b', b'c'][..],
            ),
            (&[5, 0, 18, 10, 0x81, 0, 1][..], &[5, 0, 18, 10, 1, 1][..]),
        ] {
            assert_eq!(
                rewrite(input, false, ReaderMode::QuickJsCompatible),
                expected
            );
        }
    }

    #[test]
    fn exact_quickjs_date_vectors_cross_the_pure_graph_bit_for_bit() {
        for (bytes, references) in [
            (&[5, 0, 17, 5, 84][..], false),
            (&[5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 69, 64][..], true),
            (&[5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 0, 128][..], false),
            (&[5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 240, 127][..], true),
            (&[5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 240, 255][..], false),
            (&[5, 0, 17, 6, 0, 0, 0, 0, 0, 0, 248, 127][..], true),
            (&[5, 0, 17, 6, 66, 0, 0, 0, 0, 0, 248, 127][..], true),
            (&[5, 0, 17, 6, 1, 0, 0, 0, 0, 0, 240, 127][..], false),
            (&[5, 0, 9, 2, 17, 5, 84, 19, 1][..], true),
            (&[5, 0, 9, 2, 17, 5, 84, 17, 5, 84][..], false),
        ] {
            assert_eq!(rewrite(bytes, references, ReaderMode::Strict), bytes);
        }
    }

    #[test]
    fn exact_quickjs_template_object_vectors_cross_the_pure_graph() {
        for (bytes, references) in [
            (&[5, 0, 11, 0, 2][..], false),
            (&[5, 0, 11, 2, 5, 2, 7, 2, b'x', 2][..], false),
            (
                &[
                    5, 0, 11, 2, 7, 2, b'a', 7, 2, b'b', 11, 2, 7, 2, b'a', 7, 2, b'b', 2,
                ][..],
                false,
            ),
            (&[5, 0, 11, 0, 19, 0][..], true),
            (&[5, 0, 11, 1, 19, 0, 2][..], true),
            // A data-mode header atom inside an indexed template element.
            (
                &[5, 1, 6, b'f', b'o', b'o', 11, 1, 8, 1, 2, 5, 84, 2][..],
                false,
            ),
        ] {
            assert_eq!(rewrite(bytes, references, ReaderMode::Strict), bytes);
        }
    }

    #[test]
    fn compatible_template_length_rewrites_canonically() {
        assert_eq!(
            rewrite(
                &[5, 0, 11, 0x80, 0, 2],
                false,
                ReaderMode::QuickJsCompatible,
            ),
            [5, 0, 11, 0, 2]
        );
    }

    #[test]
    fn date_aliases_and_compatible_ints_rewrite_canonically() {
        assert_eq!(
            rewrite(
                &[5, 0, 9, 2, 18, 17, 5, 84, 19, 2],
                true,
                ReaderMode::Strict,
            ),
            [5, 0, 9, 2, 17, 5, 84, 19, 1]
        );
        assert_eq!(
            rewrite(
                &[5, 0, 17, 5, 0x80, 0],
                false,
                ReaderMode::QuickJsCompatible,
            ),
            [5, 0, 17, 5, 0]
        );
        assert_eq!(
            rewrite(
                &[5, 0, 17, 5, 84, 99, 98],
                false,
                ReaderMode::QuickJsCompatible,
            ),
            [5, 0, 17, 5, 84]
        );
    }

    #[test]
    fn compatible_array_buffer_lengths_rewrite_canonically() {
        assert_eq!(
            rewrite(
                &[5, 0, 15, 0x80, 0x00, 0xff, 0xff, 0xff, 0xff, 0x0f],
                false,
                ReaderMode::QuickJsCompatible,
            ),
            [5, 0, 15, 0, 0xff, 0xff, 0xff, 0xff, 0x0f]
        );
    }

    #[test]
    fn rewrite_matches_quickjs_atom_interning_and_property_materialization() {
        for (input, expected) in [
            // Unused header atom is not written back.
            (&[5, 1, 2, b'x', 1][..], &[5, 0, 1][..]),
            // Duplicate atom slots intern to one semantic atom.
            (
                &[5, 2, 2, b'x', 2, b'x', 8, 1, 4, 1][..],
                &[5, 1, 2, b'x', 8, 1, 2, 1][..],
            ),
            // Duplicate properties keep their first slot and last value.
            (
                &[5, 1, 2, b'x', 8, 2, 2, 1, 2, 4][..],
                &[5, 1, 2, b'x', 8, 1, 2, 4][..],
            ),
            // The output atom table follows property traversal, not input order.
            (
                &[5, 2, 2, b'y', 2, b'x', 8, 2, 4, 1, 2, 4][..],
                &[5, 2, 2, b'x', 2, b'y', 8, 2, 2, 1, 4, 4][..],
            ),
            // JS_NewAtomStr turns the canonical decimal spelling into an index.
            (&[5, 1, 2, b'0', 8, 1, 2, 1][..], &[5, 0, 8, 1, 1, 1][..]),
            // String "0" and a directly tagged zero key are the same property.
            (
                &[5, 1, 2, b'0', 8, 2, 2, 5, 2, 1, 5, 4][..],
                &[5, 0, 8, 1, 1, 5, 4][..],
            ),
            // Atom collection follows the writer's depth-first value walk.
            (
                &[
                    5, 4, 2, b'z', 2, b'c', 2, b'b', 2, b'a', 8, 2, 8, 8, 2, 6, 5, 2, 4, 5, 4, 2,
                    5, 6,
                ][..],
                &[
                    5, 4, 2, b'a', 2, b'b', 2, b'c', 2, b'z', 8, 2, 2, 8, 2, 4, 5, 2, 6, 5, 4, 8,
                    5, 6,
                ][..],
            ),
        ] {
            assert_eq!(rewrite(input, false, ReaderMode::Strict), expected);
        }
    }

    #[test]
    fn compatible_null_atom_key_is_consumed_but_not_materialized() {
        assert_eq!(
            rewrite(&[5, 0, 8, 1, 0, 1], false, ReaderMode::QuickJsCompatible,),
            [5, 0, 8, 0]
        );
    }
}
