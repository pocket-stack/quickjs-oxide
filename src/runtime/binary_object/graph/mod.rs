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
    const GRAPH_LIMITS: GraphLimits = GraphLimits::new(64, 64, 32, 128, 256, 1024, 2048, 0, 0);

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
