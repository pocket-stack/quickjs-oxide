//! Exact source bytes and source coordinates, independent of parsing policy.
pub mod coordinates;
pub use coordinates::{DebugMetadataError, LineColumn, QuickJsSourceLocator, SourceOffset};
pub mod text;
pub use text::*;

/// A byte and human-readable location in a UTF-8 source file.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceLocation {
    pub byte_offset: usize,
    pub line: u32,
    pub column: u32,
}

impl SourceLocation {
    #[must_use]
    pub const fn new(byte_offset: usize, line: u32, column: u32) -> Self {
        Self {
            byte_offset,
            line,
            column,
        }
    }
}

/// Half-open source range `[start, end)`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SourceSpan {
    pub start: SourceLocation,
    pub end: SourceLocation,
}

impl SourceSpan {
    #[must_use]
    pub const fn new(start: SourceLocation, end: SourceLocation) -> Self {
        Self { start, end }
    }
}

pub mod unicode;
