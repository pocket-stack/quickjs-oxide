//! Byte offsets and QuickJS source-coordinate mapping.

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

/// Convert authoritative source-buffer byte offsets with pinned QuickJS's
/// exact `get_line_col` rules: LF is the only newline and bytes shaped like
/// UTF-8 continuations do not advance the column. Raw parser inputs need not
/// themselves be valid UTF-8.
#[derive(Clone, Copy, Debug)]
pub struct QuickJsSourceLocator<'source> {
    source: &'source [u8],
    utf8: Option<&'source str>,
}

impl<'source> QuickJsSourceLocator<'source> {
    #[must_use]
    pub const fn new(source: &'source str) -> Self {
        Self {
            source: source.as_bytes(),
            utf8: Some(source),
        }
    }

    #[must_use]
    pub const fn from_bytes(source: &'source [u8]) -> Self {
        Self { source, utf8: None }
    }

    pub fn locate(self, offset: SourceOffset) -> Result<LineColumn, DebugMetadataError> {
        self.locate_byte_offset(offset.as_usize())
    }

    pub fn locate_byte_offset(self, byte_offset: usize) -> Result<LineColumn, DebugMetadataError> {
        if byte_offset > self.source.len() {
            return Err(DebugMetadataError::OffsetOutOfBounds);
        }
        if self
            .utf8
            .is_some_and(|source| !source.is_char_boundary(byte_offset))
        {
            return Err(DebugMetadataError::OffsetNotUtf8Boundary);
        }

        advance_position(LineColumn::default(), &self.source[..byte_offset])
    }

    /// Prepare bounded-distance lookups for consumers with many source sites.
    /// One-off diagnostics can keep using the allocation-free locator.
    pub fn index(self) -> Result<QuickJsSourceIndex<'source>, DebugMetadataError> {
        let mut checkpoints = Vec::with_capacity(self.source.len() / SOURCE_CHECKPOINT_BYTES + 1);
        let mut position = LineColumn::default();
        checkpoints.push(position);
        for chunk in self.source.chunks_exact(SOURCE_CHECKPOINT_BYTES) {
            position = advance_position(position, chunk)?;
            checkpoints.push(position);
        }
        Ok(QuickJsSourceIndex {
            locator: self,
            checkpoints,
        })
    }
}

const SOURCE_CHECKPOINT_BYTES: usize = 256;

/// Source-owned coordinate index shared across all functions in one lowering.
/// Construction is O(source bytes); each query scans at most 255 bytes after
/// an O(1) checkpoint lookup. Storage is eight bytes per 256 source bytes,
/// excluding Vec metadata/capacity. Long single-line sources remain bounded.
#[derive(Debug)]
pub struct QuickJsSourceIndex<'source> {
    locator: QuickJsSourceLocator<'source>,
    checkpoints: Vec<LineColumn>,
}

impl<'source> QuickJsSourceIndex<'source> {
    pub fn locate(&self, offset: SourceOffset) -> Result<LineColumn, DebugMetadataError> {
        self.locate_byte_offset(offset.as_usize())
    }

    pub fn locate_byte_offset(&self, byte_offset: usize) -> Result<LineColumn, DebugMetadataError> {
        self.validate_byte_offset(byte_offset)?;
        self.locate_validated_byte_offset(byte_offset)
    }

    /// Keep adjacent lookups local without changing the shared immutable index.
    pub(crate) fn cursor(&self) -> QuickJsSourceCursor<'_, 'source> {
        QuickJsSourceCursor {
            index: self,
            previous: None,
        }
    }

    fn validate_byte_offset(&self, byte_offset: usize) -> Result<(), DebugMetadataError> {
        if byte_offset > self.locator.source.len() {
            return Err(DebugMetadataError::OffsetOutOfBounds);
        }
        if self
            .locator
            .utf8
            .is_some_and(|source| !source.is_char_boundary(byte_offset))
        {
            return Err(DebugMetadataError::OffsetNotUtf8Boundary);
        }
        Ok(())
    }

    fn locate_validated_byte_offset(
        &self,
        byte_offset: usize,
    ) -> Result<LineColumn, DebugMetadataError> {
        let checkpoint = byte_offset / SOURCE_CHECKPOINT_BYTES;
        advance_position(
            self.checkpoints[checkpoint],
            &self.locator.source[checkpoint * SOURCE_CHECKPOINT_BYTES..byte_offset],
        )
    }
}

/// A function-local cursor over the source-owned index. Only successful
/// coordinates are cached, and every lookup retains the index's validation.
/// Forward lookups within one checkpoint reuse the same byte-level kernel on
/// a shorter suffix; all other lookups retain the bounded checkpoint scan.
pub(crate) struct QuickJsSourceCursor<'index, 'source> {
    index: &'index QuickJsSourceIndex<'source>,
    previous: Option<(usize, LineColumn)>,
}

impl QuickJsSourceCursor<'_, '_> {
    pub(crate) fn locate(
        &mut self,
        offset: SourceOffset,
    ) -> Result<LineColumn, DebugMetadataError> {
        self.locate_byte_offset(offset.as_usize())
    }

    fn locate_byte_offset(&mut self, byte_offset: usize) -> Result<LineColumn, DebugMetadataError> {
        self.index.validate_byte_offset(byte_offset)?;
        let position = match self.previous {
            Some((previous_offset, position))
                if previous_offset <= byte_offset
                    && previous_offset / SOURCE_CHECKPOINT_BYTES
                        == byte_offset / SOURCE_CHECKPOINT_BYTES =>
            {
                advance_position(
                    position,
                    &self.index.locator.source[previous_offset..byte_offset],
                )?
            }
            _ => self.index.locate_validated_byte_offset(byte_offset)?,
        };
        self.previous = Some((byte_offset, position));
        Ok(position)
    }
}

/// Shared byte-level rule, including malformed UTF-8 and CR-only source.
fn advance_position(
    mut position: LineColumn,
    bytes: &[u8],
) -> Result<LineColumn, DebugMetadataError> {
    for &byte in bytes {
        if byte == b'\n' {
            position.line = position
                .line
                .checked_add(1)
                .ok_or(DebugMetadataError::LineOrColumnOverflow)?;
            position.column = 0;
        } else if byte & 0xc0 != 0x80 {
            position.column = position
                .column
                .checked_add(1)
                .ok_or(DebugMetadataError::LineOrColumnOverflow)?;
        }
    }
    Ok(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn indexed_coordinates_match_scans_at_every_raw_byte_and_utf8_boundary() {
        let raw = [b'a', 0x80, 0xff, b'\r', b'\n', 0xe2, 0x80, 0xa8].repeat(100);
        let locator = QuickJsSourceLocator::from_bytes(&raw);
        let indexed = locator.index().unwrap();
        for offset in (0..=raw.len() + 1).rev() {
            assert_eq!(
                indexed.locate_byte_offset(offset),
                locator.locate_byte_offset(offset)
            );
        }
        let text = ("é\r\n𝄞\u{2028}".to_owned() + &"x".repeat(600)).repeat(3);
        let locator = QuickJsSourceLocator::new(&text);
        let indexed = locator.index().unwrap();
        for offset in 0..=text.len() + 1 {
            assert_eq!(
                indexed.locate_byte_offset(offset),
                locator.locate_byte_offset(offset)
            );
        }
        let empty = QuickJsSourceLocator::from_bytes(b"").index().unwrap();
        assert_eq!(empty.locate_byte_offset(0), Ok(LineColumn::new(0, 0)));
    }
    #[test]
    fn cursor_matches_index_and_scan_for_forward_backward_and_invalid_offsets() {
        let raw = [b'a', 0x80, 0xff, b'\r', b'\n', 0xe2, 0x80, 0xa8].repeat(100);
        let text = ("é\r\n𝄞\u{2028}".to_owned() + &"x".repeat(600)).repeat(3);
        for locator in [
            QuickJsSourceLocator::from_bytes(&raw),
            QuickJsSourceLocator::new(&text),
            QuickJsSourceLocator::from_bytes(b""),
        ] {
            let index = locator.index().unwrap();
            let mut cursor = index.cursor();
            let length = locator.source.len();
            let offsets = (0..=length + 1)
                .chain((0..=length + 1).rev())
                .chain((0..=length).flat_map(|offset| {
                    [offset, usize::MAX, offset, length.saturating_sub(offset)]
                }));
            for offset in offsets {
                let previous = cursor.previous;
                let expected = index.locate_byte_offset(offset);
                assert_eq!(expected, locator.locate_byte_offset(offset));
                assert_eq!(
                    cursor.locate_byte_offset(offset),
                    expected,
                    "offset={offset}"
                );
                if expected.is_err() {
                    assert_eq!(cursor.previous, previous);
                }
            }
        }
    }

    #[test]
    fn cursor_preserves_line_and_column_overflow_and_last_success() {
        // Synthetic checkpoints exercise overflow without allocating sources
        // larger than the public offset limit. Both paths use the same kernel.
        for (bytes, position) in [
            (&b"\nx"[..], LineColumn::new(u32::MAX, 0)),
            (&b"x\n"[..], LineColumn::new(0, u32::MAX)),
        ] {
            let index = QuickJsSourceIndex {
                locator: QuickJsSourceLocator::from_bytes(bytes),
                checkpoints: vec![position],
            };
            let mut cursor = index.cursor();
            assert_eq!(cursor.locate_byte_offset(0), Ok(position));
            for offset in [1, 2, 0, usize::MAX, 1] {
                let previous = cursor.previous;
                let expected = index.locate_byte_offset(offset);
                assert_eq!(cursor.locate_byte_offset(offset), expected);
                if expected.is_err() {
                    assert_eq!(cursor.previous, previous);
                }
            }
        }
    }
}
