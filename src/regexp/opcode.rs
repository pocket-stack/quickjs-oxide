//! Runtime-independent RegExp instruction IR.
//!
//! The instruction families are a typed Rust analogue of pinned QuickJS
//! `libregexp-opcode.h`. Absolute instruction indices replace the C engine's
//! packed relative byte offsets; greediness is represented by `Split` branch
//! order instead of separate `split_*_first` opcodes.

/// Inclusive Unicode scalar or UTF-16 code-unit interval.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct CharacterRange {
    pub start: u32,
    pub end: u32,
}

impl CharacterRange {
    #[must_use]
    pub const fn new(start: u32, end: u32) -> Self {
        Self { start, end }
    }
}

/// One instruction in a compiled regular-expression program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Instruction {
    Char {
        value: u32,
        ignore_case: bool,
    },
    /// Match any character except a line terminator.
    Dot,
    /// Match any character, including a line terminator (`dotAll`).
    Any,
    /// QuickJS's optimized `\s` / `\S` instruction family.
    Space {
        inverted: bool,
    },
    Range {
        ranges: Box<[CharacterRange]>,
        inverted: bool,
        ignore_case: bool,
    },
    LineStart {
        multiline: bool,
    },
    LineEnd {
        multiline: bool,
    },
    WordBoundary {
        inverted: bool,
        ignore_case: bool,
    },
    /// Move to the preceding UTF-16 code unit or Unicode code point. QuickJS
    /// brackets ordinary consuming instructions with `prev` while compiling a
    /// backwards lookaround body, so the existing forward matcher can be
    /// reused without a second character-instruction family.
    Prev,
    /// QuickJS's variable-length back-reference opcode. Numeric references
    /// currently emit one capture; the slice shape also accommodates duplicate
    /// named captures without changing the executor ABI later.
    BackReference {
        captures: Box<[u8]>,
        ignore_case: bool,
    },
    /// Compare a participating capture from right to left and leave the input
    /// position at the start of the matched text.
    BackwardBackReference {
        captures: Box<[u8]>,
        ignore_case: bool,
    },
    /// Enter a zero-width lookaround assertion. Body instructions encode their
    /// own forward or backward direction; `target` is the instruction after
    /// the paired [`Instruction::LookAheadEnd`].
    LookAhead {
        negative: bool,
        target: usize,
    },
    /// Complete the nearest lookaround assertion of the same polarity.
    LookAheadEnd {
        negative: bool,
    },
    Jump {
        target: usize,
    },
    /// Try `first` immediately and retain `second` as the backtracking branch.
    /// Greedy and lazy quantifiers differ only in these two targets' order.
    Split {
        first: usize,
        second: usize,
    },
    Match,
    SaveStart {
        capture: u8,
    },
    SaveEnd {
        capture: u8,
    },
    /// Reset an inclusive capture range before a quantified subexpression.
    ResetCaptures {
        from: u8,
        to: u8,
    },
    SetRegister {
        register: u8,
        value: u32,
    },
    /// Decrement `register` and jump while the resulting value is non-zero.
    Loop {
        register: u8,
        target: usize,
    },
    SavePosition {
        register: u8,
    },
    /// Fail this branch if the input position still equals the saved value.
    CheckAdvance {
        register: u8,
    },
}
