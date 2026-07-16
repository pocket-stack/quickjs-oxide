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
    EmptyBackReference { pc: usize },
    RegisterIndex { pc: usize, register: u8 },
    CounterRegisterExpected { pc: usize, register: u8 },
    PositionRegisterExpected { pc: usize, register: u8 },
    InvalidCharacter { pc: usize, value: u32 },
    InvalidCharacterRange { pc: usize, start: u32, end: u32 },
    AssertionStructure { pc: usize },
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
            Self::EmptyBackReference { pc } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} has no back-reference captures"
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
            Self::AssertionStructure { pc } => {
                write!(
                    formatter,
                    "RegExp instruction {pc} has invalid lookaround structure"
                )
            }
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
    const NO_ASSERTION_SCOPE: usize = usize::MAX;

    let instructions = program.instructions();
    if instructions.is_empty() {
        return Err(ProgramError::EmptyProgram.into());
    }
    if program.capture_count() == 0 {
        return Err(ProgramError::ZeroCaptures.into());
    }
    let capture_count = program.capture_count();
    let register_count = program.register_count();
    let mut assertions = Vec::new();
    let mut assertion_scopes = Vec::new();
    assertion_scopes
        .try_reserve_exact(instructions.len())
        .map_err(|_| ExecError::OutOfMemory)?;

    for (pc, instruction) in instructions.iter().enumerate() {
        // A begin PC is unique, so the innermost assertion identifies the
        // complete active assertion chain and distinguishes sibling regions.
        assertion_scopes.push(
            assertions
                .last()
                .map_or(NO_ASSERTION_SCOPE, |(begin, _, _)| *begin),
        );
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
            Instruction::LookAhead {
                negative, target, ..
            } => {
                assertions
                    .try_reserve(1)
                    .map_err(|_| ExecError::OutOfMemory)?;
                assertions.push((pc, *negative, *target));
                Some(*target)
            }
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
            Instruction::BackReference { captures, .. }
            | Instruction::BackwardBackReference { captures, .. } => {
                if captures.is_empty() {
                    return Err(ProgramError::EmptyBackReference { pc }.into());
                }
                for capture in captures {
                    if *capture >= capture_count {
                        return Err(ProgramError::CaptureIndex {
                            pc,
                            capture: *capture,
                        }
                        .into());
                    }
                }
            }
            Instruction::LookAheadEnd { negative } => {
                let Some((begin, expected_negative, target)) = assertions.pop() else {
                    return Err(ProgramError::AssertionStructure { pc }.into());
                };
                if expected_negative != *negative || target != pc + 1 || begin >= pc {
                    return Err(ProgramError::AssertionStructure { pc }.into());
                }
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
    if let Some((pc, _, _)) = assertions.pop() {
        return Err(ProgramError::AssertionStructure { pc }.into());
    }

    for (pc, instruction) in instructions.iter().enumerate() {
        let scope = assertion_scopes[pc];
        let validate_target = |target: usize| -> Result<(), ExecError> {
            if assertion_scopes[target] != scope {
                return Err(ProgramError::AssertionStructure { pc }.into());
            }
            Ok(())
        };
        match instruction {
            Instruction::Jump { target } | Instruction::Loop { target, .. } => {
                validate_target(*target)?;
            }
            Instruction::Split { first, second } => {
                validate_target(*first)?;
                validate_target(*second)?;
            }
            Instruction::LookAhead { target, .. } => {
                validate_target(*target)?;
            }
            Instruction::Match if scope != NO_ASSERTION_SCOPE => {
                return Err(ProgramError::AssertionStructure { pc }.into());
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

impl Undo {
    fn same_location(self, other: Self) -> bool {
        match (self, other) {
            (Self::Capture { slot: left, .. }, Self::Capture { slot: right, .. }) => left == right,
            (
                Self::Register { register: left, .. },
                Self::Register {
                    register: right, ..
                },
            ) => left == right,
            _ => false,
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum ControlKind {
    Split { pc: usize },
    LookAhead { negative: bool, target: usize },
}

#[derive(Clone, Copy, Debug)]
struct ControlFrame {
    kind: ControlKind,
    position: usize,
    undo_len: usize,
}

struct AttemptState {
    pc: usize,
    position: usize,
    captures: Vec<Option<usize>>,
    registers: Vec<RegisterValue>,
    undo: Vec<Undo>,
    controls: Vec<ControlFrame>,
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
            controls: Vec::new(),
        })
    }

    fn push_backtrack(&mut self, pc: usize) -> Result<(), ExecError> {
        self.controls
            .try_reserve(1)
            .map_err(|_| ExecError::OutOfMemory)?;
        self.controls.push(ControlFrame {
            kind: ControlKind::Split { pc },
            position: self.position,
            undo_len: self.undo.len(),
        });
        Ok(())
    }

    fn push_lookahead(&mut self, negative: bool, target: usize) -> Result<(), ExecError> {
        self.controls
            .try_reserve(1)
            .map_err(|_| ExecError::OutOfMemory)?;
        self.controls.push(ControlFrame {
            kind: ControlKind::LookAhead { negative, target },
            position: self.position,
            undo_len: self.undo.len(),
        });
        Ok(())
    }

    fn rollback_to(&mut self, undo_len: usize) {
        while self.undo.len() > undo_len {
            match self.undo.pop().expect("undo length was checked") {
                Undo::Capture { slot, previous } => self.captures[slot] = previous,
                Undo::Register { register, previous } => self.registers[register] = previous,
            }
        }
    }

    /// Restore the next viable control state after one matching branch fails.
    ///
    /// A failed positive lookahead propagates through its boundary. A failed
    /// negative lookahead succeeds at its continuation. Split frames resume
    /// their retained branch.
    fn restore_backtrack(&mut self) -> bool {
        loop {
            let Some(frame) = self.controls.pop() else {
                return false;
            };
            self.rollback_to(frame.undo_len);
            self.position = frame.position;
            match frame.kind {
                ControlKind::Split { pc } => {
                    self.pc = pc;
                    return true;
                }
                ControlKind::LookAhead {
                    negative: false, ..
                } => {}
                ControlKind::LookAhead {
                    negative: true,
                    target,
                } => {
                    self.pc = target;
                    return true;
                }
            }
        }
    }

    fn finish_positive_lookahead(&mut self, pc: usize) -> Result<(), ExecError> {
        let Some(index) = self
            .controls
            .iter()
            .rposition(|frame| matches!(frame.kind, ControlKind::LookAhead { .. }))
        else {
            return Err(ProgramError::AssertionStructure { pc }.into());
        };
        let frame = self.controls[index];
        if !matches!(
            frame.kind,
            ControlKind::LookAhead {
                negative: false,
                target,
            } if target == pc + 1
        ) {
            return Err(ProgramError::AssertionStructure { pc }.into());
        }

        self.controls.truncate(index);
        self.position = frame.position;
        if let Some(outer) = self.controls.last() {
            // Internal alternatives can record the same location at several
            // nested checkpoints. QuickJS retains only the first old value
            // needed by the surviving outer transaction.
            let outer_checkpoint = outer.undo_len;
            let old_len = self.undo.len();
            let mut write = frame.undo_len;
            for read in frame.undo_len..old_len {
                let entry = self.undo[read];
                if self.undo[outer_checkpoint..write]
                    .iter()
                    .copied()
                    .any(|existing| existing.same_location(entry))
                {
                    continue;
                }
                self.undo[write] = entry;
                write += 1;
            }
            self.undo.truncate(write);
        } else {
            // Captures retain their successful values. With no outer control
            // state, however, their undo records can no longer be observed.
            self.undo.truncate(frame.undo_len);
        }
        Ok(())
    }

    fn reject_negative_lookahead(&mut self, pc: usize) -> Result<(), ExecError> {
        let Some(index) = self
            .controls
            .iter()
            .rposition(|frame| matches!(frame.kind, ControlKind::LookAhead { .. }))
        else {
            return Err(ProgramError::AssertionStructure { pc }.into());
        };
        let frame = self.controls[index];
        if !matches!(
            frame.kind,
            ControlKind::LookAhead {
                negative: true,
                target,
            } if target == pc + 1
        ) {
            return Err(ProgramError::AssertionStructure { pc }.into());
        }

        self.rollback_to(frame.undo_len);
        self.controls.truncate(index);
        self.position = frame.position;
        Ok(())
    }

    fn record_capture(&mut self, slot: usize) -> Result<(), ExecError> {
        let Some(checkpoint) = self.controls.last().map(|state| state.undo_len) else {
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
        let Some(checkpoint) = self.controls.last().map(|state| state.undo_len) else {
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
            Instruction::Prev => {
                let Some((_, previous)) = read_previous_character(input, state.position, unicode)
                else {
                    fail_branch!();
                };
                state.position = previous;
            }
            Instruction::BackReference {
                captures,
                ignore_case,
            } => {
                let participating = captures.iter().find_map(|capture| {
                    let capture = usize::from(*capture);
                    match (state.captures[capture * 2], state.captures[capture * 2 + 1]) {
                        (Some(start), Some(end)) => Some((start, end)),
                        _ => None,
                    }
                });
                if let Some((start, end)) = participating {
                    let Some(position) = match_back_reference(
                        input,
                        start,
                        end,
                        state.position,
                        unicode,
                        *ignore_case,
                    ) else {
                        fail_branch!();
                    };
                    state.position = position;
                }
            }
            Instruction::BackwardBackReference {
                captures,
                ignore_case,
            } => {
                let participating = captures.iter().find_map(|capture| {
                    let capture = usize::from(*capture);
                    match (state.captures[capture * 2], state.captures[capture * 2 + 1]) {
                        (Some(start), Some(end)) => Some((start, end)),
                        _ => None,
                    }
                });
                if let Some((start, end)) = participating {
                    let Some(position) = match_backward_back_reference(
                        input,
                        start,
                        end,
                        state.position,
                        unicode,
                        *ignore_case,
                    ) else {
                        fail_branch!();
                    };
                    state.position = position;
                }
            }
            Instruction::LookAhead { negative, target } => {
                state.push_lookahead(*negative, *target)?;
            }
            Instruction::LookAheadEnd { negative: false } => {
                state.finish_positive_lookahead(instruction_pc)?;
            }
            Instruction::LookAheadEnd { negative: true } => {
                state.reject_negative_lookahead(instruction_pc)?;
                fail_branch!();
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

fn match_back_reference(
    input: &[u16],
    capture_start: usize,
    capture_end: usize,
    input_start: usize,
    unicode: bool,
    ignore_case: bool,
) -> Option<usize> {
    let capture_input = input.get(..capture_end)?;
    let mut capture_position = capture_start;
    let mut input_position = input_start;
    while capture_position < capture_end {
        let (mut captured, next_capture) =
            read_character(capture_input, capture_position, unicode)?;
        let (mut current, next_input) = read_character(input, input_position, unicode)?;
        if ignore_case {
            captured = canonicalize(captured, unicode);
            current = canonicalize(current, unicode);
        }
        if captured != current {
            return None;
        }
        capture_position = next_capture;
        input_position = next_input;
    }
    Some(input_position)
}

fn match_backward_back_reference(
    input: &[u16],
    capture_start: usize,
    capture_end: usize,
    input_end: usize,
    unicode: bool,
    ignore_case: bool,
) -> Option<usize> {
    let mut capture_position = capture_end;
    let mut input_position = input_end;
    while capture_position > capture_start {
        let (mut captured, previous_capture) =
            read_previous_character_bounded(input, capture_position, capture_start, unicode)?;
        let (mut current, previous_input) =
            read_previous_character_bounded(input, input_position, 0, unicode)?;
        if ignore_case {
            captured = canonicalize(captured, unicode);
            current = canonicalize(current, unicode);
        }
        if captured != current {
            return None;
        }
        capture_position = previous_capture;
        input_position = previous_input;
    }
    (capture_position == capture_start).then_some(input_position)
}

fn previous_character(input: &[u16], position: usize, unicode: bool) -> Option<u32> {
    read_previous_character(input, position, unicode).map(|(character, _)| character)
}

fn read_previous_character(input: &[u16], position: usize, unicode: bool) -> Option<(u32, usize)> {
    read_previous_character_bounded(input, position, 0, unicode)
}

fn read_previous_character_bounded(
    input: &[u16],
    position: usize,
    lower_bound: usize,
    unicode: bool,
) -> Option<(u32, usize)> {
    if position <= lower_bound {
        return None;
    }
    let last = *input.get(position.checked_sub(1)?)?;
    if unicode && is_low_surrogate(last) && position >= lower_bound.saturating_add(2) {
        let first = input[position - 2];
        if is_high_surrogate(first) {
            return Some((
                0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(last) - 0xdc00),
                position - 2,
            ));
        }
    }
    Some((u32::from(last), position - 1))
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
    let candidate = ranges.partition_point(|range| range.start <= character);
    candidate
        .checked_sub(1)
        .and_then(|index| ranges.get(index))
        .is_some_and(|range| character <= range.end)
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

    #[test]
    fn normalized_range_lookup_uses_the_preceding_interval() {
        let ranges = [
            CharacterRange::new(0x0010, 0x001f),
            CharacterRange::new(0x0100, 0x01ff),
            CharacterRange::new(0x1_0000, 0x10_ffff),
        ];
        for (character, expected) in [
            (0x000f, false),
            (0x0010, true),
            (0x001f, true),
            (0x0020, false),
            (0x00ff, false),
            (0x0100, true),
            (0x01ff, true),
            (0x0200, false),
            (0xffff, false),
            (0x1_0000, true),
            (0x10_ffff, true),
        ] {
            assert_eq!(range_contains(&ranges, character), expected);
        }
        assert!(!range_contains(&[], 0));
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

        let empty_back_reference = program(
            RegExpFlags::EMPTY,
            1,
            0,
            vec![Instruction::BackReference {
                captures: Box::new([]),
                ignore_case: false,
            }],
        );
        assert_eq!(
            execute(&empty_back_reference, &[], 0),
            Err(ExecError::InvalidProgram(
                ProgramError::EmptyBackReference { pc: 0 }
            ))
        );
        let empty_backward_reference = program(
            RegExpFlags::EMPTY,
            1,
            0,
            vec![Instruction::BackwardBackReference {
                captures: Box::new([]),
                ignore_case: false,
            }],
        );
        assert_eq!(
            execute(&empty_backward_reference, &[], 0),
            Err(ExecError::InvalidProgram(
                ProgramError::EmptyBackReference { pc: 0 }
            ))
        );

        let invalid_back_reference = program(
            RegExpFlags::EMPTY,
            1,
            0,
            vec![Instruction::BackReference {
                captures: Box::new([1]),
                ignore_case: false,
            }],
        );
        assert_eq!(
            execute(&invalid_back_reference, &[], 0),
            Err(ExecError::InvalidProgram(ProgramError::CaptureIndex {
                pc: 0,
                capture: 1,
            }))
        );
        let invalid_backward_reference = program(
            RegExpFlags::EMPTY,
            1,
            0,
            vec![Instruction::BackwardBackReference {
                captures: Box::new([1]),
                ignore_case: false,
            }],
        );
        assert_eq!(
            execute(&invalid_backward_reference, &[], 0),
            Err(ExecError::InvalidProgram(ProgramError::CaptureIndex {
                pc: 0,
                capture: 1,
            }))
        );

        for (instructions, pc) in [
            (
                vec![
                    Instruction::LookAheadEnd { negative: false },
                    Instruction::Match,
                ],
                0,
            ),
            (
                vec![
                    Instruction::LookAhead {
                        negative: false,
                        target: 2,
                    },
                    Instruction::LookAheadEnd { negative: true },
                    Instruction::Match,
                ],
                1,
            ),
            (
                vec![
                    Instruction::LookAhead {
                        negative: false,
                        target: 1,
                    },
                    Instruction::Match,
                ],
                0,
            ),
            (
                vec![
                    Instruction::LookAhead {
                        negative: false,
                        target: 3,
                    },
                    Instruction::Jump { target: 3 },
                    Instruction::LookAheadEnd { negative: false },
                    Instruction::Match,
                ],
                1,
            ),
            (
                vec![
                    Instruction::LookAhead {
                        negative: false,
                        target: 3,
                    },
                    Instruction::Match,
                    Instruction::LookAheadEnd { negative: false },
                    Instruction::Match,
                ],
                1,
            ),
            (
                vec![
                    Instruction::LookAhead {
                        negative: false,
                        target: 3,
                    },
                    Instruction::Jump { target: 5 },
                    Instruction::LookAheadEnd { negative: false },
                    Instruction::LookAhead {
                        negative: false,
                        target: 6,
                    },
                    Instruction::Char {
                        value: u32::from(b'a'),
                        ignore_case: false,
                    },
                    Instruction::LookAheadEnd { negative: false },
                    Instruction::Match,
                ],
                1,
            ),
        ] {
            let invalid = program(RegExpFlags::EMPTY, 1, 0, instructions);
            assert_eq!(
                execute(&invalid, &[], 0),
                Err(ExecError::InvalidProgram(
                    ProgramError::AssertionStructure { pc }
                ))
            );
        }
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
    fn compiler_programs_execute_backreferences_with_quickjs_empty_semantics() {
        for (pattern, input, complete, captures) in [
            (r"(ab)\1", "abab", 0..4, vec![Some(0..4), Some(0..2)]),
            (r"\1(a)", "a", 0..1, vec![Some(0..1), Some(0..1)]),
            (r"(a\1)", "a", 0..1, vec![Some(0..1), Some(0..1)]),
            (
                r"(a|(b))\2c",
                "ac",
                0..2,
                vec![Some(0..2), Some(0..1), None],
            ),
            (r"(a)?\1b", "b", 0..1, vec![Some(0..1), None]),
            (r"()\1x", "x", 0..1, vec![Some(0..1), Some(0..0)]),
        ] {
            let result = execute(&compile(pattern, "u"), &units(input), 0)
                .unwrap()
                .unwrap();
            assert_eq!(result.capture(0), Some(&complete), "{pattern}");
            assert_eq!(result.captures(), captures.as_slice(), "{pattern}");
        }

        let empty_loop = execute(&compile(r"^(a)?\1*$", ""), &[], 0)
            .unwrap()
            .unwrap();
        assert_eq!(empty_loop.capture(0), Some(&(0..0)));
        assert_eq!(empty_loop.capture(1), None);
    }

    #[test]
    fn forward_lookahead_is_zero_width_and_preserves_only_positive_captures() {
        for (pattern, input, expected) in [
            (r"^(?=a)a$", "a", true),
            (r"^(?!b)a$", "a", true),
            (r"^(?=b)a$", "a", false),
            (r"^(?!a)a$", "a", false),
            (r"^(?=(?=a)a)a$", "a", true),
            (r"^(?!(?!a))a$", "a", true),
        ] {
            assert_eq!(
                execute(&compile(pattern, "u"), &units(input), 0)
                    .unwrap()
                    .is_some(),
                expected,
                "{pattern}"
            );
        }

        let positive = execute(&compile(r"^(?=(a+))\1$", "u"), &units("aaa"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(positive.captures(), &[Some(0..3), Some(0..3)]);

        let negative = execute(&compile(r"^(?!b(a))a\1$", "u"), &units("a"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(negative.captures(), &[Some(0..1), None]);
    }

    #[test]
    fn lookbehind_runs_quickjs_reverse_programs_without_consuming_twice() {
        for (pattern, input, complete, captures) in [
            (r"(?<=ab)c", "abc", 2..3, vec![Some(2..3)]),
            (r"(?<=(\w){3})d", "abcd", 3..4, vec![Some(3..4), Some(0..1)]),
            (r"(?<=([ab]+))c", "abc", 2..3, vec![Some(2..3), Some(0..2)]),
            (
                r"(?<=(bc)|(cd)).",
                "abcdef",
                3..4,
                vec![Some(3..4), Some(1..3), None],
            ),
            (r"(?<=\1(a))b", "aab", 2..3, vec![Some(2..3), Some(1..2)]),
        ] {
            let result = execute(&compile(pattern, "u"), &units(input), 0)
                .unwrap()
                .unwrap();
            assert_eq!(result.capture(0), Some(&complete), "{pattern}");
            assert_eq!(result.captures(), captures.as_slice(), "{pattern}");
        }

        let negative = execute(&compile(r"(?<!(^|[ab]))\w{2}", "u"), &units("abcdef"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(negative.captures(), &[Some(3..5), None]);
    }

    #[test]
    fn nested_and_unicode_lookbehind_preserve_assertion_boundaries() {
        for (pattern, flags, input, complete) in [
            (r"(?<=(?=b)b)c", "u", "abc", 2..3),
            (r"(?<=(?<=a)b)c", "u", "abc", 2..3),
            (r"(?<=😀)x", "u", "😀x", 2..3),
            (r"(?<=😀)x", "", "😀x", 2..3),
            (r"(?<=^a)b", "u", "ab", 1..2),
            (r"(?<=\ba)b", "u", "ab", 1..2),
        ] {
            assert_eq!(
                complete_range(execute(&compile(pattern, flags), &units(input), 0).unwrap()),
                Some(complete),
                "{pattern}/{flags}",
            );
        }

        let result = execute(&compile(r"(?<=\1(\w))d", "iu"), &units("abcCd"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.captures(), &[Some(4..5), Some(3..4)]);
    }

    #[test]
    fn lookbehind_is_atomic_and_rolls_back_abandoned_captures() {
        assert!(
            execute(&compile(r"(?<=(a|ba))c", "u"), &units("bac"), 0)
                .unwrap()
                .is_some()
        );

        let result = execute(&compile(r"(?:(?<=(a))b|a)", "u"), &units("a"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.captures(), &[Some(0..1), None]);

        let result = execute(&compile(r"(?<!(a))b", "u"), &units("b"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.captures(), &[Some(0..1), None]);
    }

    #[test]
    fn reverse_character_reads_respect_unicode_capture_bounds() {
        let input = [0xd83d, 0xde00, 0xde00];
        assert_eq!(
            match_backward_back_reference(&input, 1, 2, 3, true, false),
            Some(2)
        );

        let at_start = captured(compile("", "uy").flags(), vec![Instruction::Prev]);
        assert_eq!(execute(&at_start, &input, 0).unwrap(), None);
    }

    #[test]
    fn positive_lookahead_is_atomic_and_outer_backtracking_undoes_its_captures() {
        assert!(
            execute(&compile(r"^(?=(a|ab))\1c$", "u"), &units("abc"), 0)
                .unwrap()
                .is_none()
        );

        let result = execute(&compile(r"^(?:(?=(a))b|a)$", "u"), &units("a"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.captures(), &[Some(0..1), None]);
    }

    #[test]
    fn annex_b_quantified_lookahead_terminates_and_resets_captures() {
        for pattern in [
            r"^(?=a)*a$",
            r"^(?=a)+a$",
            r"^(?=a)??a$",
            r"^(?=a){2}a$",
            r"^(?!b)*a$",
        ] {
            assert!(
                execute(&compile(pattern, ""), &units("a"), 0)
                    .unwrap()
                    .is_some(),
                "{pattern}"
            );
        }

        for pattern in [r"^(?=(a))*a$", r"^(?=(a)){0}a$"] {
            let result = execute(&compile(pattern, ""), &units("a"), 0)
                .unwrap()
                .unwrap();
            assert_eq!(result.captures(), &[Some(0..1), None], "{pattern}");
        }
    }

    #[test]
    fn backreferences_restore_backtracking_captures_and_apply_scoped_case_folding() {
        let result = execute(&compile(r"^(a+)\1$", ""), &units("aaaaaa"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.capture(0), Some(&(0..6)));
        assert_eq!(result.capture(1), Some(&(0..3)));

        let result = execute(&compile(r"^(a|ab)+\1$", ""), &units("abab"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.capture(0), Some(&(0..4)));
        assert_eq!(result.capture(1), Some(&(0..2)));

        let result = execute(&compile(r"(a(b)?)+\2", ""), &units("aba"), 0)
            .unwrap()
            .unwrap();
        assert_eq!(result.capture(0), Some(&(0..3)));
        assert_eq!(result.capture(1), Some(&(2..3)));
        assert_eq!(result.capture(2), None);

        assert!(
            execute(&compile(r"(a)(?i:\1)", ""), &units("aA"), 0)
                .unwrap()
                .is_some()
        );
        assert!(
            execute(&compile(r"(a)(?-i:\1)", "i"), &units("aA"), 0)
                .unwrap()
                .is_none()
        );
        assert!(
            execute(&compile(r"(k)\1", "iu"), &units("kK"), 0)
                .unwrap()
                .is_some()
        );
        assert!(
            execute(&compile(r"(k)\1", "i"), &units("kK"), 0)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn multi_capture_backreferences_use_the_first_participating_capture_only() {
        let first_unmatched = program(
            RegExpFlags::STICKY,
            3,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::SaveStart { capture: 2 },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 2 },
                Instruction::BackReference {
                    captures: Box::new([1, 2]),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        assert_eq!(
            complete_range(execute(&first_unmatched, &units("aa"), 0).unwrap()),
            Some(0..2)
        );

        let first_empty = program(
            RegExpFlags::STICKY,
            3,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::SaveStart { capture: 1 },
                Instruction::SaveEnd { capture: 1 },
                Instruction::SaveStart { capture: 2 },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 2 },
                Instruction::BackReference {
                    captures: Box::new([1, 2]),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        assert_eq!(
            complete_range(execute(&first_empty, &units("aa"), 0).unwrap()),
            Some(0..1)
        );

        let first_mismatch = program(
            RegExpFlags::STICKY,
            3,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::SaveStart { capture: 1 },
                Instruction::Char {
                    value: u32::from(b'a'),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 1 },
                Instruction::SaveStart { capture: 2 },
                Instruction::Char {
                    value: u32::from(b'b'),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 2 },
                Instruction::BackReference {
                    captures: Box::new([1, 2]),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        assert_eq!(execute(&first_mismatch, &units("abb"), 0).unwrap(), None);

        let all_unmatched = program(
            RegExpFlags::STICKY,
            3,
            0,
            vec![
                Instruction::SaveStart { capture: 0 },
                Instruction::BackReference {
                    captures: Box::new([1, 2]),
                    ignore_case: false,
                },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );
        assert_eq!(
            complete_range(execute(&all_unmatched, &[], 0).unwrap()),
            Some(0..0)
        );
    }

    #[test]
    fn unicode_backreferences_respect_capture_end_and_code_point_boundaries() {
        let deseret = [
            0xd801, 0xdc00, // U+10400
            0xd801, 0xdc28, // U+10428
        ];
        assert!(
            execute(&compile(r"(\u{10400})\1", "iu"), &deseret, 0)
                .unwrap()
                .is_some()
        );

        let split_pair = [
            u16::from(b'f'),
            u16::from(b'o'),
            u16::from(b'o'),
            0xd834,
            u16::from(b'b'),
            u16::from(b'a'),
            u16::from(b'r'),
            0xd834,
            0xdc00,
        ];
        assert!(
            execute(&compile(r"foo(.+)bar\1", "u"), &split_pair, 0)
                .unwrap()
                .is_none()
        );

        let lone_surrogates = [
            u16::from(b'f'),
            u16::from(b'o'),
            u16::from(b'o'),
            0xd834,
            u16::from(b'b'),
            u16::from(b'a'),
            u16::from(b'r'),
            0xd834,
            0xd834,
        ];
        assert_eq!(
            complete_range(execute(&compile(r"foo(.+)bar\1", "u"), &lone_surrogates, 0).unwrap()),
            Some(0..8)
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
