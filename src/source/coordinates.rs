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

        let mut line = 0_u32;
        let mut column = 0_u32;
        for byte in &self.source[..byte_offset] {
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
