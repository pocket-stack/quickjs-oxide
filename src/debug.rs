//! Typed source locations and bytecode-to-source metadata.
//!
//! QuickJS deliberately does not use Unicode-scalar columns for its debug
//! tables.  Only LF advances the line number and every UTF-8 lead byte advances
//! the column.  Keeping that rule in one locator prevents the lexer diagnostic
//! coordinates (which have different ECMAScript line-terminator semantics)
//! from leaking into runtime debug metadata.

use std::error::Error;
use std::fmt;

/// A byte offset into the original UTF-8 source buffer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceOffset(u32);

impl SourceOffset {
    /// Convert a host-sized byte offset to QuickJS's bounded source offset.
    pub fn try_from_usize(value: usize) -> Result<Self, DebugMetadataError> {
        u32::try_from(value)
            .map(Self)
            .map_err(|_| DebugMetadataError::SourceTooLarge)
    }

    /// Return the original zero-based byte offset.
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    /// Return the offset as a host index.
    #[must_use]
    pub const fn as_usize(self) -> usize {
        self.0 as usize
    }
}

/// A zero-based QuickJS debug line and column.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct LineColumn {
    pub line: u32,
    pub column: u32,
}

impl LineColumn {
    #[must_use]
    pub const fn new(line: u32, column: u32) -> Self {
        Self { line, column }
    }

    /// Convert to the one-based spelling used in JavaScript stack traces.
    #[must_use]
    pub const fn one_based(self) -> Option<(u32, u32)> {
        match (self.line.checked_add(1), self.column.checked_add(1)) {
            (Some(line), Some(column)) => Some((line, column)),
            _ => None,
        }
    }
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

/// Failures at the source-offset/debug-location boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DebugMetadataError {
    SourceTooLarge,
    OffsetOutOfBounds,
    OffsetNotUtf8Boundary,
    LineOrColumnOverflow,
}

impl fmt::Display for DebugMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SourceTooLarge => "source is too large for QuickJS debug metadata",
            Self::OffsetOutOfBounds => "source offset is out of bounds",
            Self::OffsetNotUtf8Boundary => "source offset is not a UTF-8 boundary",
            Self::LineOrColumnOverflow => "QuickJS debug line or column overflowed",
        })
    }
}

impl Error for DebugMetadataError {}

/// Convert authoritative UTF-8 byte offsets with pinned QuickJS's exact
/// `get_line_col` rules: LF is the only newline and continuation bytes do not
/// advance the column.
#[derive(Clone, Copy, Debug)]
pub struct QuickJsSourceLocator<'source> {
    source: &'source str,
}

impl<'source> QuickJsSourceLocator<'source> {
    #[must_use]
    pub const fn new(source: &'source str) -> Self {
        Self { source }
    }

    pub fn locate(self, offset: SourceOffset) -> Result<LineColumn, DebugMetadataError> {
        self.locate_byte_offset(offset.as_usize())
    }

    pub fn locate_byte_offset(self, byte_offset: usize) -> Result<LineColumn, DebugMetadataError> {
        if byte_offset > self.source.len() {
            return Err(DebugMetadataError::OffsetOutOfBounds);
        }
        if !self.source.is_char_boundary(byte_offset) {
            return Err(DebugMetadataError::OffsetNotUtf8Boundary);
        }

        let mut line = 0_u32;
        let mut column = 0_u32;
        for byte in &self.source.as_bytes()[..byte_offset] {
            if *byte == b'\n' {
                line = line
                    .checked_add(1)
                    .ok_or(DebugMetadataError::LineOrColumnOverflow)?;
                column = 0;
            } else if byte & 0xc0 != 0x80 {
                column = column
                    .checked_add(1)
                    .ok_or(DebugMetadataError::LineOrColumnOverflow)?;
            }
        }
        Ok(LineColumn { line, column })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DebugMetadataError, LineColumn, Pc2LineEntry, Pc2LineTable, QuickJsSourceLocator,
        SourceOffset,
    };

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
