//! Checksum-pinned Unicode 17 property sets used by RegExp property escapes.
//!
//! The generated half-open ranges are materialized from the pinned QuickJS
//! `libunicode.c` implementation. Product builds consume only these Rust
//! arrays; the C helper under `tests/fixtures/` is a regeneration oracle.

mod tables {
    include!("unicode_property_tables.rs");
}

fn lookup<'a>(aliases: &[(&str, u16)], ranges: &'a [&'a [u32]], name: &str) -> Option<&'a [u32]> {
    let index = aliases
        .iter()
        .find_map(|(alias, index)| (*alias == name).then_some(usize::from(*index)))?;
    ranges.get(index).copied()
}

pub(crate) fn general_category(name: &str) -> Option<&'static [u32]> {
    lookup(
        tables::GENERAL_CATEGORY_ALIASES,
        tables::GENERAL_CATEGORY_RANGES,
        name,
    )
}

pub(crate) fn script(name: &str, extensions: bool) -> Option<&'static [u32]> {
    lookup(
        tables::SCRIPT_ALIASES,
        if extensions {
            tables::SCRIPT_EXTENSIONS_RANGES
        } else {
            tables::SCRIPT_RANGES
        },
        name,
    )
}

pub(crate) fn binary_property(name: &str) -> Option<&'static [u32]> {
    lookup(
        tables::BINARY_PROPERTY_ALIASES,
        tables::BINARY_PROPERTY_RANGES,
        name,
    )
}

#[cfg(test)]
mod tests {
    use super::{binary_property, general_category, script, tables};

    fn assert_valid_half_open_tables(ranges: &[&[u32]]) {
        for range_set in ranges {
            assert_eq!(range_set.len() % 2, 0);
            let mut previous_end = 0;
            for pair in range_set.chunks_exact(2) {
                assert!(pair[0] < pair[1]);
                assert!(pair[1] <= 0x11_0000);
                assert!(pair[0] >= previous_end);
                previous_end = pair[1];
            }
        }
    }

    #[test]
    fn generated_property_catalog_matches_pinned_quickjs_shape() {
        assert_eq!(tables::GENERAL_CATEGORY_RANGES.len(), 38);
        assert_eq!(tables::SCRIPT_RANGES.len(), 176);
        assert_eq!(tables::SCRIPT_EXTENSIONS_RANGES.len(), 176);
        assert_eq!(tables::BINARY_PROPERTY_RANGES.len(), 55);
        assert_eq!(tables::GENERAL_CATEGORY_ALIASES.len(), 80);
        assert_eq!(tables::SCRIPT_ALIASES.len(), 354);
        assert_eq!(tables::BINARY_PROPERTY_ALIASES.len(), 102);

        for (aliases, range_count) in [
            (
                tables::GENERAL_CATEGORY_ALIASES,
                tables::GENERAL_CATEGORY_RANGES.len(),
            ),
            (tables::SCRIPT_ALIASES, tables::SCRIPT_RANGES.len()),
            (
                tables::BINARY_PROPERTY_ALIASES,
                tables::BINARY_PROPERTY_RANGES.len(),
            ),
        ] {
            assert!(
                aliases
                    .iter()
                    .all(|(_, index)| usize::from(*index) < range_count)
            );
        }

        assert_valid_half_open_tables(tables::GENERAL_CATEGORY_RANGES);
        assert_valid_half_open_tables(tables::SCRIPT_RANGES);
        assert_valid_half_open_tables(tables::SCRIPT_EXTENSIONS_RANGES);
        assert_valid_half_open_tables(tables::BINARY_PROPERTY_RANGES);
    }

    #[test]
    fn aliases_are_exact_and_share_the_expected_range_indices() {
        assert_eq!(general_category("Letter"), general_category("L"));
        assert_eq!(general_category("Uppercase_Letter"), general_category("Lu"));
        assert!(general_category("letter").is_none());

        assert_eq!(script("Latin", false), script("Latn", false));
        assert_eq!(script("Hiragana", true), script("Hira", true));
        assert_ne!(script("Hiragana", false), script("Hiragana", true));

        assert_eq!(binary_property("ASCII_Hex_Digit"), binary_property("AHex"));
        assert_eq!(
            binary_property("Emoji_Presentation"),
            binary_property("EPres")
        );
        assert!(binary_property("RGI_Emoji").is_none());
        assert!(binary_property("ID_Compat_Math_Start").is_none());
    }
}
