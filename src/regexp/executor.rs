//! Explicit-stack execution for the typed RegExp instruction program.
//!
//! This is a Rust port of the control model in pinned QuickJS
//! `libregexp.c` 2720-3364.  Backtracking state and capture/register undo
//! records live in fallible `Vec` storage; executing a pattern never recurses
//! through the Rust call stack and never delegates to a host regular-expression
//! engine.

use std::error::Error as StdError;
use std::fmt;
use std::ops::Range;

use super::{CharacterRange, CompiledRegExp, Instruction, RegExpFlags};
use crate::unicode_case::regexp_canonicalize as canonicalize;

const INTERRUPT_POLL_INTERVAL: u32 = 10_000;

/// One successful match. Every range is measured in UTF-16 code units.
/// Capture zero is the complete match; an unmatched group is `None`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegExpMatch {
    captures: Box<[Option<Range<usize>>]>,
}

impl RegExpMatch {
    #[must_use]
    pub fn captures(&self) -> &[Option<Range<usize>>] {
        &self.captures
    }

    #[must_use]
    pub fn capture(&self, index: usize) -> Option<&Range<usize>> {
        self.captures.get(index).and_then(Option::as_ref)
    }
}

/// A structurally invalid typed program. The compiler should make these
/// states unreachable; retaining the classification keeps future opcode work
/// fail-closed instead of turning malformed programs into false matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProgramError {
    EmptyProgram,
    ZeroCaptures,
    ProgramCounter { pc: usize },
    BranchTarget { pc: usize, target: usize },
    CaptureIndex { pc: usize, capture: u8 },
    CaptureRange { pc: usize, from: u8, to: u8 },
    RegisterIndex { pc: usize, register: u8 },
    CounterRegisterExpected { pc: usize, register: u8 },
    PositionRegisterExpected { pc: usize, register: u8 },
    InvalidCharacter { pc: usize, value: u32 },
    InvalidCharacterRange { pc: usize, start: u32, end: u32 },
    IncompleteCapture { capture: u8 },
}

impl fmt::Display for ProgramError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::EmptyProgram => formatter.write_str("empty RegExp program"),
            Self::ZeroCaptures => {
                formatter.write_str("RegExp program has no complete-match capture")
            }
            Self::ProgramCounter { pc } => {
                write!(formatter, "RegExp program counter {pc} is out of bounds")
            }
            Self::BranchTarget { pc, target } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} targets invalid instruction {target}"
                )
            }
            Self::CaptureIndex { pc, capture } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} references capture {capture} out of bounds"
                )
            }
            Self::CaptureRange { pc, from, to } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} has invalid capture range {from}..={to}"
                )
            }
            Self::RegisterIndex { pc, register } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} references register {register} out of bounds"
                )
            }
            Self::CounterRegisterExpected { pc, register } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} requires counter register {register}"
                )
            }
            Self::PositionRegisterExpected { pc, register } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} requires position register {register}"
                )
            }
            Self::InvalidCharacter { pc, value } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} has invalid character U+{value:X}"
                )
            }
            Self::InvalidCharacterRange { pc, start, end } => write!(
                formatter,
                "RegExp instruction {pc} has invalid range U+{start:X}..=U+{end:X}"
            ),
            Self::IncompleteCapture { capture } => {
                write!(formatter, "RegExp capture {capture} has only one boundary")
            }
        }
    }
}

/// Failure to execute one compiled RegExp program.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExecError {
    StartOutOfBounds { start: usize, input_len: usize },
    OutOfMemory,
    Interrupted,
    InvalidProgram(ProgramError),
}

impl fmt::Display for ExecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::StartOutOfBounds { start, input_len } => {
                write!(
                    formatter,
                    "RegExp start {start} exceeds UTF-16 input length {input_len}"
                )
            }
            Self::OutOfMemory => formatter.write_str("out of memory during RegExp execution"),
            Self::Interrupted => formatter.write_str("RegExp execution interrupted"),
            Self::InvalidProgram(error) => error.fmt(formatter),
        }
    }
}

impl StdError for ExecError {}

impl From<ProgramError> for ExecError {
    fn from(error: ProgramError) -> Self {
        Self::InvalidProgram(error)
    }
}

/// Execute without an embedding interrupt hook.
pub fn execute(
    program: &CompiledRegExp,
    input: &[u16],
    start: usize,
) -> Result<Option<RegExpMatch>, ExecError> {
    execute_with_interrupt(program, input, start, || false)
}

/// Execute while polling `interrupted` every 10,000 dispatched control steps.
/// Returning `true` stops execution with [`ExecError::Interrupted`].
pub fn execute_with_interrupt<F>(
    program: &CompiledRegExp,
    input: &[u16],
    start: usize,
    mut interrupted: F,
) -> Result<Option<RegExpMatch>, ExecError>
where
    F: FnMut() -> bool,
{
    validate_program(program)?;
    if start > input.len() {
        return Err(ExecError::StartOutOfBounds {
            start,
            input_len: input.len(),
        });
    }

    let flags = program.flags();
    let unicode = flags.is_unicode();
    let sticky = flags.contains(RegExpFlags::STICKY);
    let mut candidate = normalize_start(input, start, unicode);
    let mut poller = Poller::new(&mut interrupted);

    loop {
        if let Some(result) = run_attempt(program, input, candidate, unicode, &mut poller)? {
            return Ok(Some(result));
        }
        if sticky || candidate == input.len() {
            return Ok(None);
        }
        candidate = advance_string_index(input, candidate, unicode);
    }
}

fn validate_program(program: &CompiledRegExp) -> Result<(), ExecError> {
    let instructions = program.instructions();
    if instructions.is_empty() {
        return Err(ProgramError::EmptyProgram.into());
    }
    if program.capture_count() == 0 {
        return Err(ProgramError::ZeroCaptures.into());
    }
    let capture_count = program.capture_count();
    let register_count = program.register_count();

    for (pc, instruction) in instructions.iter().enumerate() {
        let target = match instruction {
            Instruction::Jump { target } => Some(*target),
            Instruction::Split { first, second } => {
                for target in [*first, *second] {
                    if target >= instructions.len() {
                        return Err(ProgramError::BranchTarget { pc, target }.into());
                    }
                }
                None
            }
            Instruction::Loop { target, .. } => Some(*target),
            _ => None,
        };
        if let Some(target) = target
            && target >= instructions.len()
        {
            return Err(ProgramError::BranchTarget { pc, target }.into());
        }

        match instruction {
            Instruction::Char { value, .. } if *value > 0x10_ffff => {
                return Err(ProgramError::InvalidCharacter { pc, value: *value }.into());
            }
            Instruction::Range { ranges, .. } => {
                for range in ranges.iter() {
                    if range.start > range.end || range.end > 0x10_ffff {
                        return Err(ProgramError::InvalidCharacterRange {
                            pc,
                            start: range.start,
                            end: range.end,
                        }
                        .into());
                    }
                }
            }
            Instruction::SaveStart { capture } | Instruction::SaveEnd { capture }
                if *capture >= capture_count =>
            {
                return Err(ProgramError::CaptureIndex {
                    pc,
                    capture: *capture,
                }
                .into());
            }
            Instruction::ResetCaptures { from, to } if from > to || *to >= capture_count => {
                return Err(ProgramError::CaptureRange {
                    pc,
                    from: *from,
                    to: *to,
                }
                .into());
            }
            Instruction::SetRegister { register, .. }
            | Instruction::Loop { register, .. }
            | Instruction::SavePosition { register }
            | Instruction::CheckAdvance { register }
                if *register >= register_count =>
            {
                return Err(ProgramError::RegisterIndex {
                    pc,
                    register: *register,
                }
                .into());
            }
            _ => {}
        }
    }
    Ok(())
}

struct Poller<'a, F> {
    remaining: u32,
    interrupted: &'a mut F,
}

impl<'a, F> Poller<'a, F>
where
    F: FnMut() -> bool,
{
    fn new(interrupted: &'a mut F) -> Self {
        Self {
            remaining: INTERRUPT_POLL_INTERVAL,
            interrupted,
        }
    }

    fn step(&mut self) -> Result<(), ExecError> {
        self.remaining -= 1;
        if self.remaining == 0 {
            self.remaining = INTERRUPT_POLL_INTERVAL;
            if (self.interrupted)() {
                return Err(ExecError::Interrupted);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RegisterValue {
    Unset,
    Counter(u32),
    Position(usize),
}

#[derive(Clone, Copy, Debug)]
enum Undo {
    Capture {
        slot: usize,
        previous: Option<usize>,
    },
    Register {
        register: usize,
        previous: RegisterValue,
    },
}

#[derive(Clone, Copy, Debug)]
struct Backtrack {
    pc: usize,
    position: usize,
    undo_len: usize,
}

struct AttemptState {
    pc: usize,
    position: usize,
    captures: Vec<Option<usize>>,
    registers: Vec<RegisterValue>,
    undo: Vec<Undo>,
    backtracks: Vec<Backtrack>,
}

impl AttemptState {
    fn new(capture_count: u8, register_count: u8, position: usize) -> Result<Self, ExecError> {
        let mut captures = Vec::new();
        let capture_slots = usize::from(capture_count) * 2;
        captures
            .try_reserve_exact(capture_slots)
            .map_err(|_| ExecError::OutOfMemory)?;
        captures.resize(capture_slots, None);

        let mut registers = Vec::new();
        registers
            .try_reserve_exact(usize::from(register_count))
            .map_err(|_| ExecError::OutOfMemory)?;
        registers.resize(usize::from(register_count), RegisterValue::Unset);

        Ok(Self {
            pc: 0,
            position,
            captures,
            registers,
            undo: Vec::new(),
            backtracks: Vec::new(),
        })
    }

    fn push_backtrack(&mut self, pc: usize) -> Result<(), ExecError> {
        self.backtracks
            .try_reserve(1)
            .map_err(|_| ExecError::OutOfMemory)?;
        self.backtracks.push(Backtrack {
            pc,
            position: self.position,
            undo_len: self.undo.len(),
        });
        Ok(())
    }

    fn restore_backtrack(&mut self) -> bool {
        let Some(backtrack) = self.backtracks.pop() else {
            return false;
        };
        while self.undo.len() > backtrack.undo_len {
            match self.undo.pop().expect("undo length was checked") {
                Undo::Capture { slot, previous } => self.captures[slot] = previous,
                Undo::Register { register, previous } => self.registers[register] = previous,
            }
        }
        self.pc = backtrack.pc;
        self.position = backtrack.position;
        true
    }

    fn record_capture(&mut self, slot: usize) -> Result<(), ExecError> {
        let Some(checkpoint) = self.backtracks.last().map(|state| state.undo_len) else {
            return Ok(());
        };
        if self.undo[checkpoint..]
            .iter()
            .any(|entry| matches!(entry, Undo::Capture { slot: found, .. } if *found == slot))
        {
            return Ok(());
        }
        self.undo
            .try_reserve(1)
            .map_err(|_| ExecError::OutOfMemory)?;
        self.undo.push(Undo::Capture {
            slot,
            previous: self.captures[slot],
        });
        Ok(())
    }

    fn set_capture(&mut self, slot: usize, value: Option<usize>) -> Result<(), ExecError> {
        self.record_capture(slot)?;
        self.captures[slot] = value;
        Ok(())
    }

    fn record_register(&mut self, register: usize) -> Result<(), ExecError> {
        let Some(checkpoint) = self.backtracks.last().map(|state| state.undo_len) else {
            return Ok(());
        };
        if self.undo[checkpoint..].iter().any(
            |entry| matches!(entry, Undo::Register { register: found, .. } if *found == register),
        ) {
            return Ok(());
        }
        self.undo
            .try_reserve(1)
            .map_err(|_| ExecError::OutOfMemory)?;
        self.undo.push(Undo::Register {
            register,
            previous: self.registers[register],
        });
        Ok(())
    }

    fn set_register(&mut self, register: usize, value: RegisterValue) -> Result<(), ExecError> {
        self.record_register(register)?;
        self.registers[register] = value;
        Ok(())
    }

    fn finish_match(&self) -> Result<RegExpMatch, ExecError> {
        let capture_count = self.captures.len() / 2;
        let mut captures = Vec::new();
        captures
            .try_reserve_exact(capture_count)
            .map_err(|_| ExecError::OutOfMemory)?;
        for capture in 0..capture_count {
            let start = self.captures[capture * 2];
            let end = self.captures[capture * 2 + 1];
            let range = match (start, end) {
                (None, None) => None,
                (Some(start), Some(end)) if start <= end => Some(start..end),
                _ => {
                    return Err(ProgramError::IncompleteCapture {
                        capture: capture as u8,
                    }
                    .into());
                }
            };
            captures.push(range);
        }
        Ok(RegExpMatch {
            captures: captures.into_boxed_slice(),
        })
    }
}

fn run_attempt<F>(
    program: &CompiledRegExp,
    input: &[u16],
    start: usize,
    unicode: bool,
    poller: &mut Poller<'_, F>,
) -> Result<Option<RegExpMatch>, ExecError>
where
    F: FnMut() -> bool,
{
    let instructions = program.instructions();
    let mut state = AttemptState::new(program.capture_count(), program.register_count(), start)?;

    macro_rules! fail_branch {
        () => {{
            if state.restore_backtrack() {
                continue;
            }
            return Ok(None);
        }};
    }

    loop {
        poller.step()?;
        let instruction_pc = state.pc;
        let instruction = instructions
            .get(instruction_pc)
            .ok_or(ProgramError::ProgramCounter { pc: instruction_pc })?;
        state.pc += 1;

        match instruction {
            Instruction::Char { value, ignore_case } => {
                let Some((mut character, next)) = read_character(input, state.position, unicode)
                else {
                    fail_branch!();
                };
                if *ignore_case {
                    character = canonicalize(character, unicode);
                }
                if character != *value {
                    fail_branch!();
                }
                state.position = next;
            }
            Instruction::Dot => {
                let Some((character, next)) = read_character(input, state.position, unicode) else {
                    fail_branch!();
                };
                if is_line_terminator(character) {
                    fail_branch!();
                }
                state.position = next;
            }
            Instruction::Any => {
                let Some((_, next)) = read_character(input, state.position, unicode) else {
                    fail_branch!();
                };
                state.position = next;
            }
            Instruction::Space { inverted } => {
                let Some((character, next)) = read_character(input, state.position, unicode) else {
                    fail_branch!();
                };
                if is_space(character) == *inverted {
                    fail_branch!();
                }
                state.position = next;
            }
            Instruction::Range {
                ranges,
                inverted,
                ignore_case,
            } => {
                let Some((mut character, next)) = read_character(input, state.position, unicode)
                else {
                    fail_branch!();
                };
                if *ignore_case {
                    character = canonicalize(character, unicode);
                }
                let included = range_contains(ranges, character);
                if included == *inverted {
                    fail_branch!();
                }
                state.position = next;
            }
            Instruction::LineStart { multiline } => {
                if state.position != 0
                    && (!*multiline
                        || previous_character(input, state.position, unicode)
                            .is_none_or(|character| !is_line_terminator(character)))
                {
                    fail_branch!();
                }
            }
            Instruction::LineEnd { multiline } => {
                if state.position != input.len()
                    && (!*multiline
                        || read_character(input, state.position, unicode)
                            .is_none_or(|(character, _)| !is_line_terminator(character)))
                {
                    fail_branch!();
                }
            }
            Instruction::WordBoundary {
                inverted,
                ignore_case,
            } => {
                let previous = previous_character(input, state.position, unicode)
                    .is_some_and(|character| is_word(character, *ignore_case && unicode));
                let current = read_character(input, state.position, unicode)
                    .is_some_and(|(character, _)| is_word(character, *ignore_case && unicode));
                let boundary = previous != current;
                if boundary == *inverted {
                    fail_branch!();
                }
            }
            Instruction::Jump { target } => state.pc = *target,
            Instruction::Split { first, second } => {
                state.push_backtrack(*second)?;
                state.pc = *first;
            }
            Instruction::Match => return state.finish_match().map(Some),
            Instruction::SaveStart { capture } => {
                state.set_capture(usize::from(*capture) * 2, Some(state.position))?;
            }
            Instruction::SaveEnd { capture } => {
                state.set_capture(usize::from(*capture) * 2 + 1, Some(state.position))?;
            }
            Instruction::ResetCaptures { from, to } => {
                for capture in *from..=*to {
                    state.set_capture(usize::from(capture) * 2, None)?;
                    state.set_capture(usize::from(capture) * 2 + 1, None)?;
                }
            }
            Instruction::SetRegister { register, value } => {
                state.set_register(usize::from(*register), RegisterValue::Counter(*value))?;
            }
            Instruction::Loop { register, target } => {
                let register_index = usize::from(*register);
                let RegisterValue::Counter(value) = state.registers[register_index] else {
                    return Err(ProgramError::CounterRegisterExpected {
                        pc: instruction_pc,
                        register: *register,
                    }
                    .into());
                };
                let value = value.wrapping_sub(1);
                state.set_register(register_index, RegisterValue::Counter(value))?;
                if value != 0 {
                    state.pc = *target;
                }
            }
            Instruction::SavePosition { register } => {
                state.set_register(
                    usize::from(*register),
                    RegisterValue::Position(state.position),
                )?;
            }
            Instruction::CheckAdvance { register } => {
                let register_index = usize::from(*register);
                let RegisterValue::Position(position) = state.registers[register_index] else {
                    return Err(ProgramError::PositionRegisterExpected {
                        pc: instruction_pc,
                        register: *register,
                    }
                    .into());
                };
                if position == state.position {
                    fail_branch!();
                }
            }
        }
    }
}

fn normalize_start(input: &[u16], start: usize, unicode: bool) -> usize {
    if unicode
        && start > 0
        && start < input.len()
        && is_low_surrogate(input[start])
        && is_high_surrogate(input[start - 1])
    {
        start - 1
    } else {
        start
    }
}

fn advance_string_index(input: &[u16], position: usize, unicode: bool) -> usize {
    read_character(input, position, unicode)
        .map(|(_, next)| next)
        .unwrap_or(input.len())
}

fn read_character(input: &[u16], position: usize, unicode: bool) -> Option<(u32, usize)> {
    let first = *input.get(position)?;
    if unicode
        && is_high_surrogate(first)
        && let Some(second) = input.get(position + 1).copied()
        && is_low_surrogate(second)
    {
        let code_point =
            0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00);
        return Some((code_point, position + 2));
    }
    Some((u32::from(first), position + 1))
}

fn previous_character(input: &[u16], position: usize, unicode: bool) -> Option<u32> {
    let last = *input.get(position.checked_sub(1)?)?;
    if unicode && is_low_surrogate(last) && position >= 2 {
        let first = input[position - 2];
        if is_high_surrogate(first) {
            return Some(
                0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(last) - 0xdc00),
            );
        }
    }
    Some(u32::from(last))
}

const fn is_high_surrogate(unit: u16) -> bool {
    unit >= 0xd800 && unit <= 0xdbff
}

const fn is_low_surrogate(unit: u16) -> bool {
    unit >= 0xdc00 && unit <= 0xdfff
}

const fn is_line_terminator(character: u32) -> bool {
    matches!(character, 0x000a | 0x000d | 0x2028 | 0x2029)
}

const fn is_space(character: u32) -> bool {
    matches!(
        character,
        0x0009..=0x000d
            | 0x0020
            | 0x00a0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}

fn is_word(character: u32, ignore_case: bool) -> bool {
    character == u32::from(b'_')
        || character >= u32::from(b'0') && character <= u32::from(b'9')
        || character >= u32::from(b'A') && character <= u32::from(b'Z')
        || character >= u32::from(b'a') && character <= u32::from(b'z')
        || ignore_case && matches!(character, 0x017f | 0x212a)
}

fn range_contains(ranges: &[CharacterRange], character: u32) -> bool {
    ranges
        .iter()
        .any(|range| character >= range.start && character <= range.end)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn program(
        flags: RegExpFlags,
        capture_count: u8,
        register_count: u8,
        instructions: Vec<Instruction>,
    ) -> CompiledRegExp {
        CompiledRegExp::from_parts(flags, capture_count, register_count, instructions)
    }

    fn captured(flags: RegExpFlags, body: Vec<Instruction>) -> CompiledRegExp {
        let mut instructions = Vec::with_capacity(body.len() + 3);
        instructions.push(Instruction::SaveStart { capture: 0 });
        instructions.extend(body);
        instructions.push(Instruction::SaveEnd { capture: 0 });
        instructions.push(Instruction::Match);
        program(flags, 1, 0, instructions)
    }

    fn units(value: &str) -> Vec<u16> {
        value.encode_utf16().collect()
    }

    fn compile(pattern: &str, flags: &str) -> CompiledRegExp {
        super::super::compiler::compile_units(&units(pattern), &units(flags)).unwrap()
    }

    fn complete_range(result: Option<RegExpMatch>) -> Option<Range<usize>> {
        result.and_then(|result| result.capture(0).cloned())
    }

    #[test]
    fn leftmost_search_and_sticky_start_follow_flags() {
        let body = vec![
            Instruction::Char {
                value: u32::from(b'a'),
                ignore_case: false,
            },
            Instruction::Char {
                value: u32::from(b'b'),
                ignore_case: false,
            },
        ];
        let input = units("zzababa");
        assert_eq!(
            complete_range(
                execute(&captured(RegExpFlags::EMPTY, body.clone()), &input, 0).unwrap()
            ),
            Some(2..4)
        );
        assert_eq!(
            execute(&captured(RegExpFlags::STICKY, body.clone()), &input, 0).unwrap(),
            None
        );
        assert_eq!(
            complete_range(execute(&captured(RegExpFlags::STICKY, body), &input, 2).unwrap()),
            Some(2..4)
        );
    }

    #[test]
    fn split_branch_order_encodes_greedy_and_lazy_quantifiers() {
        let greedy = program(
            RegExpFlags::STICKY,
            1,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::Split {
                    first: 3,
                    second: 5,
                },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::Jump { target: 2 },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        let lazy = program(
            RegExpFlags::STICKY,
            1,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::Split {
                    first: 5,
                    second: 3,
                },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::Jump { target: 2 },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        let input = units("aaab");
        assert_eq!(
            complete_range(execute(&greedy, &input, 0).unwrap()),
            Some(0..3)
        );
        assert_eq!(
            complete_range(execute(&lazy, &input, 0).unwrap()),
            Some(0..1)
        );
    }

    #[test]
    fn failed_branches_roll_back_capture_boundaries() {
        let regexp = program(
            RegExpFlags::STICKY,
            2,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::Split {
                    first: 2,
                    second: 7,
                },
                Instruction::SaveStart { capture: 1 },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 1 },
                Instruction::Char {
                    value: u32::from(b'x'),
                    ignore_case: false,
                },
                Instruction::Match,
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::Char {
                    value: u32::from(b'b'),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        let result = execute(&regexp, &units("ab"), 0).unwrap().unwrap();
        assert_eq!(result.capture(0), Some(&(0..2)));
        assert_eq!(result.capture(1), None);
    }

    #[test]
    fn anchors_dotall_and_multiline_use_quickjs_line_terminators() {
        let multiline = captured(
            RegExpFlags::EMPTY,
            vec![
                Instruction::LineStart { multiline: true },
                Instruction::Char {
                    value: u32::from(b'b'),
                    ignore_case: false,
                },
                Instruction::LineEnd { multiline: true },
            ],
        );
        assert_eq!(
            complete_range(execute(&multiline, &units("a\nb\r\nc"), 0).unwrap()),
            Some(2..3)
        );

        let newline = [u16::from(b'\n')];
        assert_eq!(
            execute(
                &captured(RegExpFlags::EMPTY, vec![Instruction::Dot]),
                &newline,
                0,
            )
            .unwrap(),
            None
        );
        assert_eq!(
            complete_range(
                execute(
                    &captured(RegExpFlags::EMPTY, vec![Instruction::Any]),
                    &newline,
                    0,
                )
                .unwrap()
            ),
            Some(0..1)
        );
    }

    #[test]
    fn ranges_space_and_word_boundary_cover_typed_character_ops() {
        let class = captured(
            RegExpFlags::EMPTY,
            vec![Instruction::Range {
                ranges: vec![CharacterRange::new(u32::from(b'0'), u32::from(b'9'))]
                    .into_boxed_slice(),
                inverted: false,
                ignore_case: false,
            }],
        );
        assert_eq!(
            complete_range(execute(&class, &units("xx5"), 0).unwrap()),
            Some(2..3)
        );

        let space = captured(
            RegExpFlags::EMPTY,
            vec![Instruction::Space { inverted: false }],
        );
        assert_eq!(
            complete_range(execute(&space, &[u16::from(b'x'), 0x00a0], 0).unwrap()),
            Some(1..2)
        );

        let word = captured(
            RegExpFlags::UNICODE,
            vec![
                Instruction::WordBoundary {
                    inverted: false,
                    ignore_case: true,
                },
                Instruction::Any,
            ],
        );
        assert_eq!(
            complete_range(execute(&word, &[0x212a], 0).unwrap()),
            Some(0..1)
        );

        let legacy_word = captured(
            RegExpFlags::EMPTY,
            vec![
                Instruction::WordBoundary {
                    inverted: false,
                    ignore_case: true,
                },
                Instruction::Any,
            ],
        );
        assert_eq!(execute(&legacy_word, &[0x017f], 0).unwrap(), None);
    }

    #[test]
    fn unicode_mode_normalizes_and_consumes_utf16_surrogate_pairs() {
        let smile = [0xd83d, 0xde00];
        let literal = captured(
            RegExpFlags::UNICODE,
            vec![Instruction::Char {
                value: 0x1f600,
                ignore_case: false,
            }],
        );
        assert_eq!(
            complete_range(execute(&literal, &smile, 1).unwrap()),
            Some(0..2)
        );

        let unicode_any = captured(RegExpFlags::UNICODE, vec![Instruction::Any]);
        let code_unit_any = captured(RegExpFlags::EMPTY, vec![Instruction::Any]);
        assert_eq!(
            complete_range(execute(&unicode_any, &smile, 0).unwrap()),
            Some(0..2)
        );
        assert_eq!(
            complete_range(execute(&code_unit_any, &smile, 0).unwrap()),
            Some(0..1)
        );

        let kelvin = captured(
            RegExpFlags::UNICODE,
            vec![Instruction::Char {
                value: u32::from(b'k'),
                ignore_case: true,
            }],
        );
        assert_eq!(
            complete_range(execute(&kelvin, &[0x212a], 0).unwrap()),
            Some(0..1)
        );
    }

    #[test]
    fn registers_loop_and_check_advance_follow_typed_state() {
        let exact_three = program(
            RegExpFlags::STICKY,
            1,
            1,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::SetRegister {
                    register: 0,
                    value: 3,
                },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::Loop {
                    register: 0,
                    target: 2,
                },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        assert_eq!(
            complete_range(execute(&exact_three, &units("aaab"), 0).unwrap()),
            Some(0..3)
        );

        let advances = program(
            RegExpFlags::STICKY,
            1,
            1,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::SavePosition { register: 0 },
                Instruction::Any,
                Instruction::CheckAdvance { register: 0 },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        assert_eq!(
            complete_range(execute(&advances, &units("x"), 0).unwrap()),
            Some(0..1)
        );
    }

    #[test]
    fn interrupt_hook_stops_non_recursive_control_flow() {
        let looping = program(
            RegExpFlags::STICKY,
            1,
            0,
            vec![Instruction::Jump { target: 0 }],
        );
        let mut polls = 0;
        let error = execute_with_interrupt(&looping, &[], 0, || {
            polls += 1;
            true
        })
        .unwrap_err();
        assert_eq!(error, ExecError::Interrupted);
        assert_eq!(polls, 1);
    }

    #[test]
    fn malformed_programs_fail_closed() {
        let invalid = program(
            RegExpFlags::EMPTY,
            1,
            0,
            vec![Instruction::Jump { target: 4 }],
        );
        assert_eq!(
            execute(&invalid, &[], 0),
            Err(ExecError::InvalidProgram(ProgramError::BranchTarget {
                pc: 0,
                target: 4,
            }))
        );
    }

    #[test]
    fn compiler_programs_execute_quantifiers_backtracking_and_captures() {
        let input = units("xxabbbc");
        assert_eq!(
            complete_range(execute(&compile("ab+c", ""), &input, 0).unwrap()),
            Some(2..7)
        );

        let input = units("abc");
        let result = execute(&compile("(a|ab)c", "y"), &input, 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.capture(0).cloned(), Some(0..3));
        assert_eq!(result.capture(1).cloned(), Some(0..2));

        let input = units("aaab");
        assert_eq!(
            complete_range(execute(&compile("(?:a?)*b", "y"), &input, 0).unwrap()),
            Some(0..4)
        );
        assert_eq!(
            complete_range(execute(&compile("a{2,4}?", "y"), &input, 0).unwrap()),
            Some(0..2)
        );
    }

    #[test]
    fn compiler_programs_execute_unicode_and_ignore_case() {
        let input = units("x😀😀");
        assert_eq!(
            complete_range(execute(&compile("😀+", "u"), &input, 0).unwrap()),
            Some(1..5)
        );

        assert_eq!(
            complete_range(execute(&compile("a", "i"), &units("A"), 0).unwrap()),
            Some(0..1)
        );
        assert_eq!(
            complete_range(execute(&compile("[A-Z]", "iu"), &units("k"), 0).unwrap()),
            Some(0..1)
        );
        assert_eq!(
            complete_range(execute(&compile("[k]", "iu"), &[0x212a], 0).unwrap()),
            Some(0..1)
        );
    }

    #[test]
    fn nullable_finite_quantifiers_roll_back_zero_advance_captures() {
        let result = execute(&compile("(a?)?", ""), &[], 0).unwrap().unwrap();
        assert_eq!(result.capture(0), Some(&(0..0)));
        assert_eq!(result.capture(1), None);

        let result = execute(&compile("(a|){0,2}", ""), &units("a"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.capture(0), Some(&(0..1)));
        assert_eq!(result.capture(1), Some(&(0..1)));
    }

    #[test]
    fn class_shorthand_complements_fold_before_outer_inversion() {
        for input in [[0x017f], [0x212a]] {
            assert_eq!(execute(&compile(r"[\W]", "iu"), &input, 0).unwrap(), None);
            assert_eq!(
                complete_range(execute(&compile(r"[^\W]", "iu"), &input, 0).unwrap()),
                Some(0..1)
            );
            assert_eq!(
                complete_range(execute(&compile(r"[\W]", "i"), &input, 0).unwrap()),
                Some(0..1)
            );
            assert_eq!(execute(&compile(r"[^\W]", "i"), &input, 0).unwrap(), None);
        }
    }
}
