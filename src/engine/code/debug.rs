//! Typed source locations and bytecode-to-source metadata.
//!
//! QuickJS deliberately does not use Unicode-scalar columns for its debug
//! tables.  Only LF advances the line number and every UTF-8 lead byte advances
//! the column.  Keeping that rule in one locator prevents the lexer diagnostic
//! coordinates (which have different ECMAScript line-terminator semantics)
//! from leaking into runtime debug metadata.

use crate::source::LineColumn;

/// Runtime-wide policy for the currently represented, JS-observable portion
/// of QuickJS `JS_SetStripInfo`.
///
/// The selected mode is sampled when a function is compiled. Existing
/// bytecode keeps the debug payload it was published with.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DebugInfoMode {
    /// Keep filename, PC positions and exact authored function source.
    #[default]
    Full,
    /// Keep filename and PC positions but omit authored function source.
    StripSource,
    /// Omit function source and location metadata from the bytecode payload.
    StripDebug,
}

/// One absolute typed entry in a bytecode PC-to-source table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Pc2LineEntry {
    pub pc: u32,
    pub position: LineColumn,
}

/// Debug position of a function definition and marked bytecode instructions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pc2LineTable {
    pub definition: LineColumn,
    pub entries: Box<[Pc2LineEntry]>,
}

impl Pc2LineTable {
    #[must_use]
    pub fn new(definition: LineColumn, entries: impl Into<Box<[Pc2LineEntry]>>) -> Self {
        Self {
            definition,
            entries: entries.into(),
        }
    }

    /// Match QuickJS `find_line_num`: the last entry whose PC is no greater
    /// than the queried PC wins. A missing current PC uses the definition site.
    #[must_use]
    pub fn lookup(&self, pc: Option<u32>) -> LineColumn {
        let Some(pc) = pc else {
            return self.definition;
        };
        self.entries
            .iter()
            .take_while(|entry| entry.pc <= pc)
            .last()
            .map_or(self.definition, |entry| entry.position)
    }
}

#[cfg(test)]
mod tests {
    use super::{LineColumn, Pc2LineEntry, Pc2LineTable};
    use crate::source::DebugMetadataError;
    use crate::source::{QuickJsSourceLocator, SourceOffset};

    #[test]
    fn quickjs_locator_only_treats_lf_as_a_newline() {
        let source = "a\rb\r\nc\u{2028}d\u{2029}e\nf";
        let locator = QuickJsSourceLocator::new(source);

        assert_eq!(locator.locate_byte_offset(0), Ok(LineColumn::new(0, 0)));
        assert_eq!(
            locator.locate_byte_offset(source.find('b').unwrap()),
            Ok(LineColumn::new(0, 2))
        );
        assert_eq!(
            locator.locate_byte_offset(source.find('c').unwrap()),
            Ok(LineColumn::new(1, 0))
        );
        assert_eq!(
            locator.locate_byte_offset(source.find('d').unwrap()),
            Ok(LineColumn::new(1, 2))
        );
        assert_eq!(
            locator.locate_byte_offset(source.find('e').unwrap()),
            Ok(LineColumn::new(1, 4))
        );
        assert_eq!(
            locator.locate_byte_offset(source.find('f').unwrap()),
            Ok(LineColumn::new(2, 0))
        );
    }

    #[test]
    fn quickjs_locator_counts_utf8_lead_bytes_not_raw_bytes() {
        let source = "é中x";
        let locator = QuickJsSourceLocator::new(source);
        assert_eq!(
            locator.locate_byte_offset(source.find('x').unwrap()),
            Ok(LineColumn::new(0, 2))
        );
        assert_eq!(
            locator.locate_byte_offset(1),
            Err(DebugMetadataError::OffsetNotUtf8Boundary)
        );
    }

    #[test]
    fn quickjs_raw_locator_preserves_cesu8_and_malformed_byte_columns() {
        let canonical = QuickJsSourceLocator::from_bytes(b"/*\xf0\x9f\x98\x80*/@");
        let cesu8 = QuickJsSourceLocator::from_bytes(b"/*\xed\xa0\xbd\xed\xb8\x80*/@");
        let continuation = QuickJsSourceLocator::from_bytes(b"/*\x80*/@");
        let invalid_lead = QuickJsSourceLocator::from_bytes(b"/*\xff*/@");

        assert_eq!(canonical.locate_byte_offset(8), Ok(LineColumn::new(0, 5)));
        assert_eq!(cesu8.locate_byte_offset(10), Ok(LineColumn::new(0, 6)));
        assert_eq!(
            continuation.locate_byte_offset(5),
            Ok(LineColumn::new(0, 4))
        );
        assert_eq!(
            invalid_lead.locate_byte_offset(5),
            Ok(LineColumn::new(0, 5))
        );
    }

    #[test]
    fn pc_lookup_uses_the_last_entry_at_or_before_the_pc() {
        let table = Pc2LineTable::new(
            LineColumn::new(1, 2),
            vec![
                Pc2LineEntry {
                    pc: 1,
                    position: LineColumn::new(3, 4),
                },
                Pc2LineEntry {
                    pc: 5,
                    position: LineColumn::new(6, 7),
                },
            ],
        );
        assert_eq!(table.lookup(None), LineColumn::new(1, 2));
        assert_eq!(table.lookup(Some(0)), LineColumn::new(1, 2));
        assert_eq!(table.lookup(Some(4)), LineColumn::new(3, 4));
        assert_eq!(table.lookup(Some(5)), LineColumn::new(6, 7));
        assert_eq!(SourceOffset::try_from_usize(7).unwrap().get(), 7);
    }
}
