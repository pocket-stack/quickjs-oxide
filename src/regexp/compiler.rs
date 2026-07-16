//! Typed RegExp parser/compiler foundation.
//!
//! This is a safe Rust port of the front-end structure in pinned QuickJS
//! `libregexp.c` (`re_parse_disjunction` through `lre_compile`, lines
//! 1848-2612). It deliberately emits typed instructions instead of the C
//! engine's packed byte buffer. Unsupported advanced syntax is rejected with
//! a distinct error kind so later milestones cannot accidentally accept it
//! with different semantics.

use super::RegExpFlags;
use super::flags::{FlagParseErrorKind, parse_flags};
use super::opcode::{CharacterRange, Instruction};

const INFINITE_REPETITION: u32 = i32::MAX as u32;
const MAX_CODE_POINT: u32 = 0x10_ffff;
const MAX_GROUP_NESTING: usize = 256;

/// One runtime-independent compiled regular-expression program.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledRegExp {
    flags: RegExpFlags,
    capture_count: u8,
    register_count: u8,
    instructions: Box<[Instruction]>,
}

impl CompiledRegExp {
    pub(super) fn from_parts(
        flags: RegExpFlags,
        capture_count: u8,
        register_count: u8,
        instructions: Vec<Instruction>,
    ) -> Self {
        Self {
            flags,
            capture_count,
            register_count,
            instructions: instructions.into_boxed_slice(),
        }
    }

    #[must_use]
    pub const fn flags(&self) -> RegExpFlags {
        self.flags
    }

    /// Capture zero is the complete match, matching QuickJS's bytecode header.
    #[must_use]
    pub const fn capture_count(&self) -> u8 {
        self.capture_count
    }

    #[must_use]
    pub const fn register_count(&self) -> u8 {
        self.register_count
    }

    #[must_use]
    pub fn instructions(&self) -> &[Instruction] {
        &self.instructions
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompileErrorSource {
    Pattern,
    Flags,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnsupportedFeature {
    UnicodePropertyEscape,
    UnicodeSetOperation,
    NamedCapture,
    Backreference,
    Lookaround,
    InlineModifier,
    LegacyOctalEscape,
    LegacyControlEscape,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompileErrorKind {
    Syntax,
    Unsupported(UnsupportedFeature),
    TooManyCaptures,
    TooManyRegisters,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileError {
    kind: CompileErrorKind,
    source: CompileErrorSource,
    position: usize,
    message: String,
}

impl CompileError {
    #[must_use]
    pub const fn kind(&self) -> &CompileErrorKind {
        &self.kind
    }

    #[must_use]
    pub const fn source(&self) -> CompileErrorSource {
        self.source
    }

    /// UTF-16 code-unit offset in the pattern or flags source.
    #[must_use]
    pub const fn position(&self) -> usize {
        self.position
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }

    fn syntax(position: usize, message: impl Into<String>) -> Self {
        Self {
            kind: CompileErrorKind::Syntax,
            source: CompileErrorSource::Pattern,
            position,
            message: message.into(),
        }
    }

    fn unsupported(position: usize, feature: UnsupportedFeature) -> Self {
        Self {
            kind: CompileErrorKind::Unsupported(feature),
            source: CompileErrorSource::Pattern,
            position,
            message: format!("unsupported regular-expression syntax: {feature:?}"),
        }
    }

    fn too_many_captures(position: usize) -> Self {
        Self {
            kind: CompileErrorKind::TooManyCaptures,
            source: CompileErrorSource::Pattern,
            position,
            message: "too many captures".to_owned(),
        }
    }

    fn too_many_registers(position: usize) -> Self {
        Self {
            kind: CompileErrorKind::TooManyRegisters,
            source: CompileErrorSource::Pattern,
            position,
            message: "too many imbricated quantifiers".to_owned(),
        }
    }

    fn invalid_flags(position: usize, kind: FlagParseErrorKind) -> Self {
        let detail = match kind {
            FlagParseErrorKind::Invalid => "unknown flag",
            FlagParseErrorKind::Duplicate => "duplicate flag",
            FlagParseErrorKind::UnicodeConflict => "the 'u' and 'v' flags are mutually exclusive",
        };
        Self {
            kind: CompileErrorKind::Syntax,
            source: CompileErrorSource::Flags,
            position,
            message: format!("invalid regular expression flags: {detail}"),
        }
    }
}

impl std::fmt::Display for CompileError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} at {:?} UTF-16 offset {}",
            self.message, self.source, self.position
        )
    }
}

impl std::error::Error for CompileError {}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Expression {
    alternatives: Vec<Sequence>,
}

type Sequence = Vec<Term>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Term {
    atom: Atom,
    quantifier: Option<Quantifier>,
    position: usize,
}

const MODIFIER_IGNORE_CASE: u8 = 1 << 0;
const MODIFIER_MULTILINE: u8 = 1 << 1;
const MODIFIER_DOT_ALL: u8 = 1 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct ModifierState {
    ignore_case: bool,
    multiline: bool,
    dot_all: bool,
}

impl ModifierState {
    fn from_flags(flags: RegExpFlags) -> Self {
        Self {
            ignore_case: flags.contains(RegExpFlags::IGNORE_CASE),
            multiline: flags.contains(RegExpFlags::MULTILINE),
            dot_all: flags.contains(RegExpFlags::DOT_ALL),
        }
    }

    fn updated(self, add_mask: u8, remove_mask: u8) -> Self {
        Self {
            ignore_case: update_modifier(
                self.ignore_case,
                add_mask,
                remove_mask,
                MODIFIER_IGNORE_CASE,
            ),
            multiline: update_modifier(self.multiline, add_mask, remove_mask, MODIFIER_MULTILINE),
            dot_all: update_modifier(self.dot_all, add_mask, remove_mask, MODIFIER_DOT_ALL),
        }
    }
}

fn update_modifier(mut value: bool, add_mask: u8, remove_mask: u8, modifier: u8) -> bool {
    if add_mask & modifier != 0 {
        value = true;
    }
    if remove_mask & modifier != 0 {
        value = false;
    }
    value
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Atom {
    Literal(u32),
    Dot,
    LineStart,
    LineEnd,
    WordBoundary {
        inverted: bool,
    },
    Space {
        inverted: bool,
    },
    Class(CharacterClass),
    BackReference {
        capture: u8,
    },
    Group {
        capture: Option<u8>,
        modifiers: Option<ModifierState>,
        expression: Expression,
    },
}

impl Atom {
    fn is_quantifiable(&self) -> bool {
        matches!(
            self,
            Self::Literal(_)
                | Self::Dot
                | Self::Space { .. }
                | Self::Class(_)
                | Self::BackReference { .. }
                | Self::Group { .. }
        )
    }

    fn can_match_empty(&self) -> bool {
        match self {
            Self::LineStart | Self::LineEnd | Self::WordBoundary { .. } => true,
            Self::BackReference { .. } => true,
            Self::Group { expression, .. } => expression.can_match_empty(),
            Self::Literal(_) | Self::Dot | Self::Space { .. } | Self::Class(_) => false,
        }
    }

    fn capture_range(&self) -> Option<(u8, u8)> {
        match self {
            Self::Group {
                capture,
                expression,
                ..
            } => match (*capture, expression.capture_range()) {
                (Some(capture), Some((_, end))) => Some((capture, end)),
                (Some(capture), None) => Some((capture, capture)),
                (None, range) => range,
            },
            Self::Literal(_)
            | Self::Dot
            | Self::LineStart
            | Self::LineEnd
            | Self::WordBoundary { .. }
            | Self::Space { .. }
            | Self::Class(_)
            | Self::BackReference { .. } => None,
        }
    }
}

impl Expression {
    fn can_match_empty(&self) -> bool {
        self.alternatives.iter().any(|sequence| {
            sequence.iter().all(|term| {
                term.quantifier
                    .is_some_and(|quantifier| quantifier.minimum == 0)
                    || term.atom.can_match_empty()
            })
        })
    }

    fn capture_range(&self) -> Option<(u8, u8)> {
        self.alternatives
            .iter()
            .flatten()
            .filter_map(|term| term.atom.capture_range())
            .fold(None, |range, (start, end)| {
                Some(match range {
                    Some((old_start, old_end)) => (old_start.min(start), old_end.max(end)),
                    None => (start, end),
                })
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Quantifier {
    minimum: u32,
    maximum: Option<u32>,
    greedy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CharacterClass {
    ranges: Vec<CharacterRange>,
    inverted: bool,
}

enum ClassAtom {
    Single(u32),
    Set(Vec<CharacterRange>),
    ComplementSet(Vec<CharacterRange>),
}

struct Parser<'a> {
    units: &'a [u16],
    position: usize,
    flags: RegExpFlags,
    modifiers: ModifierState,
    next_capture: u16,
    total_capture_count: u32,
    group_depth: usize,
}

impl<'a> Parser<'a> {
    fn new(units: &'a [u16], flags: RegExpFlags) -> Self {
        Self {
            units,
            position: 0,
            flags,
            modifiers: ModifierState::from_flags(flags),
            next_capture: 1,
            total_capture_count: count_captures(units),
            group_depth: 0,
        }
    }

    fn parse(mut self) -> Result<(Expression, u8), CompileError> {
        let expression = self.parse_disjunction(false)?;
        if self.position != self.units.len() {
            return Err(CompileError::syntax(
                self.position,
                "unexpected closing parenthesis",
            ));
        }
        let capture_count = u8::try_from(self.next_capture)
            .map_err(|_| CompileError::too_many_captures(self.position))?;
        Ok((expression, capture_count))
    }

    fn parse_disjunction(&mut self, in_group: bool) -> Result<Expression, CompileError> {
        let mut alternatives = Vec::new();
        loop {
            alternatives.push(self.parse_sequence(in_group)?);
            if self.peek() == Some(u16::from(b'|')) {
                self.position += 1;
                continue;
            }
            break;
        }
        Ok(Expression { alternatives })
    }

    fn parse_sequence(&mut self, in_group: bool) -> Result<Sequence, CompileError> {
        let mut sequence = Vec::new();
        while let Some(unit) = self.peek() {
            if unit == u16::from(b'|') || (in_group && unit == u16::from(b')')) {
                break;
            }
            if matches!(unit, 0x2a | 0x2b | 0x3f) {
                return Err(CompileError::syntax(self.position, "nothing to repeat"));
            }
            if unit == u16::from(b'{')
                && !self.flags.is_unicode()
                && self.brace_quantifier_follows()
            {
                return Err(CompileError::syntax(self.position, "nothing to repeat"));
            }
            let position = self.position;
            let atom = self.parse_atom()?;
            // QuickJS clears last_atom_start for assertions. A following
            // brace is therefore parsed as the next term (and becomes a
            // unicode syntax error), while *, + and ? still reach the common
            // "nothing to repeat" path.
            let quantifier = if !atom.is_quantifiable() && self.peek() == Some(u16::from(b'{')) {
                None
            } else {
                self.parse_quantifier()?
            };
            if quantifier.is_some() && !atom.is_quantifiable() {
                return Err(CompileError::syntax(position, "invalid quantifier target"));
            }
            sequence.push(Term {
                atom,
                quantifier,
                position,
            });
        }
        Ok(sequence)
    }

    fn parse_atom(&mut self) -> Result<Atom, CompileError> {
        let position = self.position;
        let unit = self
            .take()
            .ok_or_else(|| CompileError::syntax(position, "unexpected end of pattern"))?;
        match unit {
            0x2e => Ok(Atom::Dot),
            0x5e => Ok(Atom::LineStart),
            0x24 => Ok(Atom::LineEnd),
            0x28 => self.parse_group(position),
            0x5b => self.parse_character_class(position).map(Atom::Class),
            0x5c => self.parse_escape(false),
            0x29 => Err(CompileError::syntax(
                position,
                "unexpected closing parenthesis",
            )),
            0x5d | 0x7d if self.flags.is_unicode() => Err(CompileError::syntax(
                position,
                "regular expression syntax error",
            )),
            0x7b if self.flags.is_unicode() => Err(CompileError::syntax(
                position,
                "regular expression syntax error",
            )),
            first => Ok(Atom::Literal(self.finish_code_point(first))),
        }
    }

    fn parse_group(&mut self, position: usize) -> Result<Atom, CompileError> {
        if self.group_depth >= MAX_GROUP_NESTING {
            return Err(CompileError::syntax(position, "stack overflow"));
        }
        self.group_depth += 1;
        let result = self.parse_group_inner(position);
        self.group_depth -= 1;
        result
    }

    fn parse_group_inner(&mut self, position: usize) -> Result<Atom, CompileError> {
        let (capture, modifiers) = if self.peek() == Some(u16::from(b'?')) {
            self.position += 1;
            match self.peek() {
                Some(0x3a) => {
                    self.position += 1;
                    (None, None)
                }
                Some(0x3d | 0x21) => {
                    return Err(CompileError::unsupported(
                        position,
                        UnsupportedFeature::Lookaround,
                    ));
                }
                Some(0x3c) => {
                    if matches!(self.peek_n(1), Some(0x3d | 0x21)) {
                        return Err(CompileError::unsupported(
                            position,
                            UnsupportedFeature::Lookaround,
                        ));
                    }
                    return Err(CompileError::unsupported(
                        position,
                        UnsupportedFeature::NamedCapture,
                    ));
                }
                Some(0x69 | 0x6d | 0x73 | 0x2d) => {
                    let add_mask = self.parse_modifiers()?;
                    let remove_mask = if self.peek() == Some(u16::from(b'-')) {
                        self.position += 1;
                        self.parse_modifiers()?
                    } else {
                        0
                    };
                    if (add_mask == 0 && remove_mask == 0) || add_mask & remove_mask != 0 {
                        return Err(CompileError::syntax(position, "invalid modifiers"));
                    }
                    if self.peek() != Some(u16::from(b':')) {
                        return Err(CompileError::syntax(self.position, "expecting ':'"));
                    }
                    self.position += 1;
                    (None, Some(self.modifiers.updated(add_mask, remove_mask)))
                }
                Some(_) | None => {
                    return Err(CompileError::syntax(position, "invalid group specifier"));
                }
            }
        } else {
            if self.next_capture >= u16::from(u8::MAX) {
                return Err(CompileError::too_many_captures(position));
            }
            let capture = u8::try_from(self.next_capture)
                .map_err(|_| CompileError::too_many_captures(position))?;
            self.next_capture += 1;
            (Some(capture), None)
        };

        let saved_modifiers = self.modifiers;
        if let Some(modifiers) = modifiers {
            self.modifiers = modifiers;
        }
        let result = (|| {
            let expression = self.parse_disjunction(true)?;
            if self.take() != Some(u16::from(b')')) {
                return Err(CompileError::syntax(position, "unterminated group"));
            }
            Ok(Atom::Group {
                capture,
                modifiers,
                expression,
            })
        })();
        self.modifiers = saved_modifiers;
        result
    }

    fn parse_modifiers(&mut self) -> Result<u8, CompileError> {
        let mut mask = 0;
        loop {
            let modifier = match self.peek() {
                Some(0x69) => MODIFIER_IGNORE_CASE,
                Some(0x6d) => MODIFIER_MULTILINE,
                Some(0x73) => MODIFIER_DOT_ALL,
                _ => break,
            };
            if mask & modifier != 0 {
                let duplicate = char::from_u32(u32::from(
                    self.peek().expect("modifier disappeared after matching"),
                ))
                .expect("RegExp modifiers are ASCII");
                return Err(CompileError::syntax(
                    self.position,
                    format!("duplicate modifier: '{duplicate}'"),
                ));
            }
            mask |= modifier;
            self.position += 1;
        }
        Ok(mask)
    }

    fn parse_escape(&mut self, in_class: bool) -> Result<Atom, CompileError> {
        let escape_position = self.position.saturating_sub(1);
        let unit = self
            .take()
            .ok_or_else(|| CompileError::syntax(escape_position, "trailing backslash"))?;
        match unit {
            0x62 if !in_class => Ok(Atom::WordBoundary { inverted: false }),
            0x42 if !in_class => Ok(Atom::WordBoundary { inverted: true }),
            0x64 => Ok(Atom::Class(
                self.make_character_class(digit_ranges(), false),
            )),
            0x44 => Ok(Atom::Class(self.make_character_class(digit_ranges(), true))),
            0x73 => Ok(Atom::Space { inverted: false }),
            0x53 => Ok(Atom::Space { inverted: true }),
            0x77 => Ok(Atom::Class(self.make_character_class(word_ranges(), false))),
            0x57 => Ok(Atom::Class(self.make_character_class(word_ranges(), true))),
            0x70 | 0x50 => Err(CompileError::unsupported(
                escape_position,
                UnsupportedFeature::UnicodePropertyEscape,
            )),
            0x6b => Err(CompileError::unsupported(
                escape_position,
                UnsupportedFeature::Backreference,
            )),
            0x31..=0x39 if in_class => Err(CompileError::syntax(
                escape_position,
                "invalid identity escape",
            )),
            0x31..=0x39 => self.parse_decimal_escape(escape_position, unit),
            0x30 => {
                if self.flags.is_unicode() && self.peek().is_some_and(is_ascii_digit) {
                    Err(CompileError::syntax(
                        escape_position,
                        "invalid decimal escape in regular expression",
                    ))
                } else if !self.flags.is_unicode() {
                    Ok(Atom::Literal(self.parse_legacy_octal(unit)))
                } else {
                    Ok(Atom::Literal(0))
                }
            }
            0x66 => Ok(Atom::Literal(0x0c)),
            0x6e => Ok(Atom::Literal(0x0a)),
            0x72 => Ok(Atom::Literal(0x0d)),
            0x74 => Ok(Atom::Literal(0x09)),
            0x76 => Ok(Atom::Literal(0x0b)),
            0x62 if in_class => Ok(Atom::Literal(0x08)),
            0x63 => self
                .parse_control_escape(escape_position)
                .map(Atom::Literal),
            0x78 => self
                .parse_fixed_hex_escape(escape_position, 2)
                .map(Atom::Literal),
            0x75 => self
                .parse_unicode_escape(escape_position)
                .map(Atom::Literal),
            escaped if is_syntax_character(escaped) || escaped == u16::from(b'/') => {
                Ok(Atom::Literal(u32::from(escaped)))
            }
            _ if self.flags.is_unicode() => Err(CompileError::syntax(
                escape_position,
                "invalid identity escape",
            )),
            escaped => Ok(Atom::Literal(self.finish_code_point(escaped))),
        }
    }

    fn parse_decimal_escape(&mut self, position: usize, first: u16) -> Result<Atom, CompileError> {
        let (reference, digit_end) = self.scan_decimal_escape(first);
        if reference
            .filter(|reference| *reference < self.total_capture_count)
            .is_some()
        {
            self.position = digit_end;
            let capture = u8::try_from(reference.expect("checked decimal reference range"))
                .expect("capture prepass caps valid decimal references below u8::MAX");
            return Ok(Atom::BackReference { capture });
        }
        if self.flags.is_unicode() {
            return Err(CompileError::syntax(
                position,
                "back reference out of range in regular expression",
            ));
        }
        if is_ascii_octal_digit(first) {
            Ok(Atom::Literal(self.parse_legacy_octal(first)))
        } else {
            Ok(Atom::Literal(u32::from(first)))
        }
    }

    /// Scan all decimal digits without committing the parser cursor. QuickJS
    /// must first decide whether the complete number names a capture before it
    /// can reinterpret the same source as an Annex B legacy escape.
    fn scan_decimal_escape(&self, first: u16) -> (Option<u32>, usize) {
        let mut value = Some(u32::from(first - u16::from(b'0')));
        let mut position = self.position;
        while let Some(unit) = self
            .units
            .get(position)
            .copied()
            .filter(|unit| is_ascii_digit(*unit))
        {
            value = value
                .and_then(|value| value.checked_mul(10))
                .and_then(|value| value.checked_add(u32::from(unit - u16::from(b'0'))))
                .filter(|value| *value < i32::MAX as u32);
            position += 1;
        }
        (value, position)
    }

    /// Annex B legacy octal parsing from pinned QuickJS `lre_parse_escape`.
    /// The first digit was already consumed; at most two additional octal
    /// digits belong to this escape.
    fn parse_legacy_octal(&mut self, first: u16) -> u32 {
        let mut value = u32::from(first - u16::from(b'0'));
        let Some(second) = self.peek().filter(|unit| is_ascii_octal_digit(*unit)) else {
            return value;
        };
        value = (value << 3) | u32::from(second - u16::from(b'0'));
        self.position += 1;
        if value >= 32 {
            return value;
        }
        let Some(third) = self.peek().filter(|unit| is_ascii_octal_digit(*unit)) else {
            return value;
        };
        self.position += 1;
        (value << 3) | u32::from(third - u16::from(b'0'))
    }

    fn parse_control_escape(&mut self, position: usize) -> Result<u32, CompileError> {
        match self.peek() {
            Some(unit) if is_ascii_letter(unit) => {
                self.position += 1;
                Ok(u32::from(unit & 0x1f))
            }
            _ if self.flags.is_unicode() => {
                Err(CompileError::syntax(position, "invalid control escape"))
            }
            _ => Err(CompileError::unsupported(
                position,
                UnsupportedFeature::LegacyControlEscape,
            )),
        }
    }

    fn parse_fixed_hex_escape(
        &mut self,
        position: usize,
        digits: usize,
    ) -> Result<u32, CompileError> {
        let start = self.position;
        let mut value = 0_u32;
        for _ in 0..digits {
            let Some(unit) = self.take() else {
                self.position = start;
                return self.invalid_hex_escape(position, u32::from(b'x'));
            };
            let Some(digit) = hex_value(unit) else {
                self.position = start;
                return self.invalid_hex_escape(position, u32::from(b'x'));
            };
            value = value * 16 + digit;
        }
        Ok(value)
    }

    fn invalid_hex_escape(&self, position: usize, identity: u32) -> Result<u32, CompileError> {
        if self.flags.is_unicode() {
            Err(CompileError::syntax(position, "invalid hexadecimal escape"))
        } else {
            Ok(identity)
        }
    }

    fn parse_unicode_escape(&mut self, position: usize) -> Result<u32, CompileError> {
        if self.flags.is_unicode() && self.peek() == Some(u16::from(b'{')) {
            self.position += 1;
            let digit_start = self.position;
            let mut value = 0_u32;
            while let Some(unit) = self.peek() {
                if unit == u16::from(b'}') {
                    break;
                }
                let Some(digit) = hex_value(unit) else {
                    return Err(CompileError::syntax(position, "invalid Unicode escape"));
                };
                value = value
                    .checked_mul(16)
                    .and_then(|value| value.checked_add(digit))
                    .filter(|value| *value <= MAX_CODE_POINT)
                    .ok_or_else(|| CompileError::syntax(position, "invalid Unicode escape"))?;
                self.position += 1;
            }
            if self.position == digit_start || self.take() != Some(u16::from(b'}')) {
                return Err(CompileError::syntax(position, "invalid Unicode escape"));
            }
            return Ok(value);
        }

        let start = self.position;
        let mut value = 0_u32;
        for _ in 0..4 {
            let Some(unit) = self.take() else {
                self.position = start;
                return self.invalid_unicode_escape(position);
            };
            let Some(digit) = hex_value(unit) else {
                self.position = start;
                return self.invalid_unicode_escape(position);
            };
            value = value * 16 + digit;
        }
        if self.flags.is_unicode() && is_high_surrogate(value) {
            let pair_start = self.position;
            if self.take() == Some(u16::from(b'\\')) && self.take() == Some(u16::from(b'u')) {
                let low_start = self.position;
                let mut low = 0_u32;
                let mut valid = true;
                for _ in 0..4 {
                    let Some(unit) = self.take() else {
                        valid = false;
                        break;
                    };
                    let Some(digit) = hex_value(unit) else {
                        valid = false;
                        break;
                    };
                    low = low * 16 + digit;
                }
                if valid && is_low_surrogate(low) {
                    return Ok(combine_surrogates(value, low));
                }
                self.position = low_start;
            }
            self.position = pair_start;
        }
        Ok(value)
    }

    fn invalid_unicode_escape(&self, position: usize) -> Result<u32, CompileError> {
        if self.flags.is_unicode() {
            Err(CompileError::syntax(position, "invalid Unicode escape"))
        } else {
            Ok(u32::from(b'u'))
        }
    }

    fn parse_character_class(&mut self, position: usize) -> Result<CharacterClass, CompileError> {
        let inverted = if self.peek() == Some(u16::from(b'^')) {
            self.position += 1;
            true
        } else {
            false
        };
        let mut ranges = Vec::new();
        loop {
            let Some(unit) = self.peek() else {
                return Err(CompileError::syntax(
                    position,
                    "unterminated character class",
                ));
            };
            if unit == u16::from(b']') {
                self.position += 1;
                break;
            }
            if self.flags.contains(RegExpFlags::UNICODE_SETS)
                && (unit == u16::from(b'[')
                    || (unit == u16::from(b'&') && self.peek_n(1) == Some(u16::from(b'&')))
                    || (unit == u16::from(b'-') && self.peek_n(1) == Some(u16::from(b'-'))))
            {
                return Err(CompileError::unsupported(
                    self.position,
                    UnsupportedFeature::UnicodeSetOperation,
                ));
            }
            let first_position = self.position;
            let first = self.parse_class_atom()?;
            if self.peek() == Some(u16::from(b'-'))
                && self.peek_n(1).is_some()
                && self.peek_n(1) != Some(u16::from(b']'))
            {
                self.position += 1;
                let second = self.parse_class_atom()?;
                match (first, second) {
                    (ClassAtom::Single(start), ClassAtom::Single(end)) => {
                        if start > end {
                            return Err(CompileError::syntax(
                                first_position,
                                "invalid class range",
                            ));
                        }
                        add_class_atom(
                            &mut ranges,
                            ClassAtom::Set(vec![CharacterRange::new(start, end)]),
                            self.class_max_code_point(),
                            self.modifiers.ignore_case,
                            self.flags.is_unicode(),
                        );
                    }
                    _ if self.flags.is_unicode() => {
                        return Err(CompileError::syntax(first_position, "invalid class range"));
                    }
                    (first, second) => {
                        // Annex B permits a legacy CharacterClassEscape at
                        // either range endpoint. QuickJS reinterprets the
                        // would-be range as the first atom, a literal '-', and
                        // the second atom instead of rejecting it.
                        for atom in [first, ClassAtom::Single(u32::from(b'-')), second] {
                            add_class_atom(
                                &mut ranges,
                                atom,
                                self.class_max_code_point(),
                                self.modifiers.ignore_case,
                                false,
                            );
                        }
                    }
                }
            } else {
                add_class_atom(
                    &mut ranges,
                    first,
                    self.class_max_code_point(),
                    self.modifiers.ignore_case,
                    self.flags.is_unicode(),
                );
            }
        }
        Ok(CharacterClass {
            ranges: normalize_ranges(ranges),
            inverted,
        })
    }

    fn parse_class_atom(&mut self) -> Result<ClassAtom, CompileError> {
        let position = self.position;
        let unit = self
            .take()
            .ok_or_else(|| CompileError::syntax(position, "unterminated character class"))?;
        if unit != u16::from(b'\\') {
            if self.flags.contains(RegExpFlags::UNICODE_SETS) && unit == u16::from(b'[') {
                return Err(CompileError::unsupported(
                    position,
                    UnsupportedFeature::UnicodeSetOperation,
                ));
            }
            return Ok(ClassAtom::Single(self.finish_code_point(unit)));
        }
        let escaped_position = self.position.saturating_sub(1);
        let escaped = self
            .take()
            .ok_or_else(|| CompileError::syntax(escaped_position, "trailing backslash"))?;
        match escaped {
            0x64 => Ok(ClassAtom::Set(digit_ranges())),
            0x44 => Ok(ClassAtom::ComplementSet(digit_ranges())),
            0x73 => Ok(ClassAtom::Set(space_ranges())),
            0x53 => Ok(ClassAtom::ComplementSet(space_ranges())),
            0x77 => Ok(ClassAtom::Set(word_ranges())),
            0x57 => Ok(ClassAtom::ComplementSet(word_ranges())),
            0x70 | 0x50 => Err(CompileError::unsupported(
                escaped_position,
                UnsupportedFeature::UnicodePropertyEscape,
            )),
            0x71 if self.flags.contains(RegExpFlags::UNICODE_SETS) => {
                Err(CompileError::unsupported(
                    escaped_position,
                    UnsupportedFeature::UnicodeSetOperation,
                ))
            }
            0x62 => Ok(ClassAtom::Single(0x08)),
            0x66 => Ok(ClassAtom::Single(0x0c)),
            0x6e => Ok(ClassAtom::Single(0x0a)),
            0x72 => Ok(ClassAtom::Single(0x0d)),
            0x74 => Ok(ClassAtom::Single(0x09)),
            0x76 => Ok(ClassAtom::Single(0x0b)),
            0x63 => self
                .parse_control_escape(escaped_position)
                .map(ClassAtom::Single),
            0x78 => self
                .parse_fixed_hex_escape(escaped_position, 2)
                .map(ClassAtom::Single),
            0x75 => self
                .parse_unicode_escape(escaped_position)
                .map(ClassAtom::Single),
            0x30 => {
                if self.flags.is_unicode() && self.peek().is_some_and(is_ascii_digit) {
                    Err(CompileError::syntax(
                        escaped_position,
                        "invalid identity escape",
                    ))
                } else if !self.flags.is_unicode() {
                    Ok(ClassAtom::Single(self.parse_legacy_octal(escaped)))
                } else {
                    Ok(ClassAtom::Single(0))
                }
            }
            0x31..=0x37 if self.flags.is_unicode() => Err(CompileError::syntax(
                escaped_position,
                "invalid identity escape",
            )),
            0x31..=0x37 => Ok(ClassAtom::Single(self.parse_legacy_octal(escaped))),
            0x38..=0x39 if self.flags.is_unicode() => Err(CompileError::syntax(
                escaped_position,
                "invalid identity escape",
            )),
            0x38..=0x39 => Ok(ClassAtom::Single(u32::from(escaped))),
            0x6b if self.flags.is_unicode() => Err(CompileError::syntax(
                escaped_position,
                "invalid identity escape",
            )),
            0x6b => Ok(ClassAtom::Single(u32::from(b'k'))),
            unit if is_syntax_character(unit)
                || unit == u16::from(b'/')
                || unit == u16::from(b'-') =>
            {
                Ok(ClassAtom::Single(u32::from(unit)))
            }
            _ if self.flags.is_unicode() => Err(CompileError::syntax(
                escaped_position,
                "invalid identity escape",
            )),
            unit => Ok(ClassAtom::Single(self.finish_code_point(unit))),
        }
    }

    fn parse_quantifier(&mut self) -> Result<Option<Quantifier>, CompileError> {
        let start = self.position;
        let (minimum, maximum) = match self.peek() {
            Some(0x2a) => {
                self.position += 1;
                (0, None)
            }
            Some(0x2b) => {
                self.position += 1;
                (1, None)
            }
            Some(0x3f) => {
                self.position += 1;
                (0, Some(1))
            }
            Some(0x7b) => {
                if !self.peek_n(1).is_some_and(is_ascii_digit) {
                    if self.flags.is_unicode() {
                        return Err(CompileError::syntax(start, "invalid repetition count"));
                    }
                    return Ok(None);
                }
                self.position += 1;
                let minimum = self.parse_decimal_clamped();
                let maximum = if self.peek() == Some(u16::from(b',')) {
                    self.position += 1;
                    if self.peek().is_some_and(is_ascii_digit) {
                        Some(self.parse_decimal_clamped())
                    } else {
                        None
                    }
                } else {
                    Some(minimum)
                };
                if self.peek() != Some(u16::from(b'}')) {
                    if self.flags.is_unicode() {
                        return Err(CompileError::syntax(start, "expecting '}'"));
                    }
                    self.position = start;
                    return Ok(None);
                }
                self.position += 1;
                if maximum.is_some_and(|maximum| maximum < minimum) {
                    return Err(CompileError::syntax(start, "invalid repetition count"));
                }
                (minimum, maximum)
            }
            Some(_) | None => return Ok(None),
        };
        let greedy = if self.peek() == Some(u16::from(b'?')) {
            self.position += 1;
            false
        } else {
            true
        };
        Ok(Some(Quantifier {
            minimum,
            maximum,
            greedy,
        }))
    }

    fn parse_decimal_clamped(&mut self) -> u32 {
        let mut value = 0_u32;
        while let Some(unit) = self.peek().filter(|unit| is_ascii_digit(*unit)) {
            value = value
                .saturating_mul(10)
                .saturating_add(u32::from(unit - u16::from(b'0')))
                .min(INFINITE_REPETITION);
            self.position += 1;
        }
        value
    }

    fn finish_code_point(&mut self, first: u16) -> u32 {
        if self.flags.is_unicode()
            && is_high_surrogate(u32::from(first))
            && self
                .peek()
                .is_some_and(|unit| is_low_surrogate(u32::from(unit)))
        {
            let low = self.take().expect("peeked low surrogate disappeared");
            combine_surrogates(u32::from(first), u32::from(low))
        } else {
            u32::from(first)
        }
    }

    fn class_max_code_point(&self) -> u32 {
        if self.flags.is_unicode() {
            MAX_CODE_POINT
        } else {
            u32::from(u16::MAX)
        }
    }

    fn make_character_class(&self, ranges: Vec<CharacterRange>, inverted: bool) -> CharacterClass {
        let ranges = if self.modifiers.ignore_case {
            canonicalize_ranges(&ranges, self.flags.is_unicode())
        } else {
            normalize_ranges(ranges)
        };
        CharacterClass { ranges, inverted }
    }

    fn brace_quantifier_follows(&self) -> bool {
        let mut position = self.position + 1;
        if !self
            .units
            .get(position)
            .copied()
            .is_some_and(is_ascii_digit)
        {
            return false;
        }
        while self
            .units
            .get(position)
            .copied()
            .is_some_and(is_ascii_digit)
        {
            position += 1;
        }
        if self.units.get(position) == Some(&u16::from(b',')) {
            position += 1;
            while self
                .units
                .get(position)
                .copied()
                .is_some_and(is_ascii_digit)
            {
                position += 1;
            }
        }
        self.units.get(position) == Some(&u16::from(b'}'))
    }

    fn peek(&self) -> Option<u16> {
        self.peek_n(0)
    }

    fn peek_n(&self, offset: usize) -> Option<u16> {
        self.units.get(self.position + offset).copied()
    }

    fn take(&mut self) -> Option<u16> {
        let unit = self.peek()?;
        self.position += 1;
        Some(unit)
    }
}

/// QuickJS `re_count_captures`-style lexical prepass. Used here to distinguish
/// a potentially in-range Unicode decimal backreference from an immediately
/// out-of-range decimal escape. It does not validate the pattern remainder.
fn count_captures(units: &[u16]) -> u32 {
    let mut count = 1_u32;
    let mut position = 0;
    while position < units.len() {
        match units[position] {
            0x28 => {
                let is_plain = units.get(position + 1) != Some(&u16::from(b'?'));
                let is_named = units.get(position + 1) == Some(&u16::from(b'?'))
                    && units.get(position + 2) == Some(&u16::from(b'<'))
                    && !matches!(units.get(position + 3).copied(), Some(0x3d | 0x21));
                if is_plain || is_named {
                    count = count.saturating_add(1);
                    if count >= u32::from(u8::MAX) {
                        break;
                    }
                }
            }
            0x5c => {
                position = position.saturating_add(1);
            }
            0x5b => {
                position += 1;
                while position < units.len() && units[position] != u16::from(b']') {
                    if units[position] == u16::from(b'\\') {
                        position = position.saturating_add(1);
                    }
                    position = position.saturating_add(1);
                }
            }
            _ => {}
        }
        position = position.saturating_add(1);
    }
    count
}

struct CodeBuilder {
    instructions: Vec<Instruction>,
    register_depth: u16,
    max_registers: u16,
    ignore_case: bool,
    unicode: bool,
    multiline: bool,
    dot_all: bool,
}

impl CodeBuilder {
    fn new(flags: RegExpFlags) -> Self {
        Self {
            instructions: Vec::new(),
            register_depth: 0,
            max_registers: 0,
            ignore_case: flags.contains(RegExpFlags::IGNORE_CASE),
            unicode: flags.is_unicode(),
            multiline: flags.contains(RegExpFlags::MULTILINE),
            dot_all: flags.contains(RegExpFlags::DOT_ALL),
        }
    }

    fn compile(mut self, expression: &Expression) -> Result<(Vec<Instruction>, u8), CompileError> {
        self.emit(Instruction::SaveStart { capture: 0 });
        self.compile_expression(expression)?;
        self.emit(Instruction::SaveEnd { capture: 0 });
        self.emit(Instruction::Match);
        let register_count =
            u8::try_from(self.max_registers).map_err(|_| CompileError::too_many_registers(0))?;
        Ok((self.instructions, register_count))
    }

    fn compile_expression(&mut self, expression: &Expression) -> Result<(), CompileError> {
        let mut end_jumps = Vec::new();
        for (index, sequence) in expression.alternatives.iter().enumerate() {
            if index + 1 == expression.alternatives.len() {
                self.compile_sequence(sequence)?;
                break;
            }
            let split = self.emit(Instruction::Split {
                first: usize::MAX,
                second: usize::MAX,
            });
            let first = self.instructions.len();
            self.compile_sequence(sequence)?;
            let jump = self.emit(Instruction::Jump { target: usize::MAX });
            let second = self.instructions.len();
            self.patch_split(split, first, second);
            end_jumps.push(jump);
        }
        let end = self.instructions.len();
        for jump in end_jumps {
            self.patch_jump(jump, end);
        }
        Ok(())
    }

    fn compile_sequence(&mut self, sequence: &Sequence) -> Result<(), CompileError> {
        for term in sequence {
            match term.quantifier {
                Some(quantifier) => {
                    self.compile_quantified(&term.atom, quantifier, term.position)?
                }
                None => self.compile_atom(&term.atom)?,
            }
        }
        Ok(())
    }

    fn compile_atom(&mut self, atom: &Atom) -> Result<(), CompileError> {
        match atom {
            Atom::Literal(value) => {
                let value = if self.ignore_case {
                    crate::unicode_case::regexp_canonicalize(*value, self.unicode)
                } else {
                    *value
                };
                self.emit(Instruction::Char {
                    value,
                    ignore_case: self.ignore_case,
                });
            }
            Atom::Dot => {
                let instruction = if self.dot_all {
                    Instruction::Any
                } else {
                    Instruction::Dot
                };
                self.emit(instruction);
            }
            Atom::LineStart => {
                self.emit(Instruction::LineStart {
                    multiline: self.multiline,
                });
            }
            Atom::LineEnd => {
                self.emit(Instruction::LineEnd {
                    multiline: self.multiline,
                });
            }
            Atom::WordBoundary { inverted } => {
                self.emit(Instruction::WordBoundary {
                    inverted: *inverted,
                    ignore_case: self.ignore_case,
                });
            }
            Atom::Space { inverted } => {
                self.emit(Instruction::Space {
                    inverted: *inverted,
                });
            }
            Atom::Class(class) => {
                self.emit(Instruction::Range {
                    ranges: class.ranges.clone().into_boxed_slice(),
                    inverted: class.inverted,
                    ignore_case: self.ignore_case,
                });
            }
            Atom::BackReference { capture } => {
                self.emit(Instruction::BackReference {
                    captures: vec![*capture].into_boxed_slice(),
                    ignore_case: self.ignore_case,
                });
            }
            Atom::Group {
                capture,
                modifiers,
                expression,
            } => {
                if let Some(capture) = capture {
                    self.emit(Instruction::SaveStart { capture: *capture });
                }
                let saved_modifiers = ModifierState {
                    ignore_case: self.ignore_case,
                    multiline: self.multiline,
                    dot_all: self.dot_all,
                };
                if let Some(modifiers) = modifiers {
                    self.ignore_case = modifiers.ignore_case;
                    self.multiline = modifiers.multiline;
                    self.dot_all = modifiers.dot_all;
                }
                let result = self.compile_expression(expression);
                self.ignore_case = saved_modifiers.ignore_case;
                self.multiline = saved_modifiers.multiline;
                self.dot_all = saved_modifiers.dot_all;
                result?;
                if let Some(capture) = capture {
                    self.emit(Instruction::SaveEnd { capture: *capture });
                }
            }
        }
        Ok(())
    }

    fn compile_quantified(
        &mut self,
        atom: &Atom,
        quantifier: Quantifier,
        position: usize,
    ) -> Result<(), CompileError> {
        let capture_range = atom.capture_range();
        if quantifier.maximum == Some(0) {
            self.emit_capture_reset(capture_range);
            return Ok(());
        }

        self.compile_required_repetitions(atom, quantifier.minimum, capture_range, position)?;
        match quantifier.maximum {
            Some(maximum) if maximum == quantifier.minimum => {}
            Some(maximum) => {
                self.compile_optional_repetitions(
                    atom,
                    maximum - quantifier.minimum,
                    quantifier.greedy,
                    capture_range,
                    position,
                )?;
            }
            None => {
                self.compile_unbounded_repetition(
                    atom,
                    quantifier.greedy,
                    capture_range,
                    position,
                )?;
            }
        }
        Ok(())
    }

    fn compile_required_repetitions(
        &mut self,
        atom: &Atom,
        count: u32,
        capture_range: Option<(u8, u8)>,
        position: usize,
    ) -> Result<(), CompileError> {
        match count {
            0 => {
                self.emit_capture_reset(capture_range);
            }
            1 => self.compile_iteration(atom, capture_range)?,
            count => {
                let register = self.allocate_register(position)?;
                self.emit(Instruction::SetRegister {
                    register,
                    value: count,
                });
                let start = self.instructions.len();
                self.compile_iteration(atom, capture_range)?;
                self.emit(Instruction::Loop {
                    register,
                    target: start,
                });
                self.release_register(register);
            }
        }
        Ok(())
    }

    fn compile_optional_repetitions(
        &mut self,
        atom: &Atom,
        count: u32,
        greedy: bool,
        capture_range: Option<(u8, u8)>,
        position: usize,
    ) -> Result<(), CompileError> {
        if count == 0 {
            return Ok(());
        }
        let register = if count > 1 {
            let register = self.allocate_register(position)?;
            self.emit(Instruction::SetRegister {
                register,
                value: count,
            });
            Some(register)
        } else {
            None
        };
        let decision = self.emit(Instruction::Split {
            first: usize::MAX,
            second: usize::MAX,
        });
        let body = self.instructions.len();
        let advance_register = if atom.can_match_empty() {
            let register = self.allocate_register(position)?;
            self.emit(Instruction::SavePosition { register });
            Some(register)
        } else {
            None
        };
        self.compile_iteration(atom, capture_range)?;
        if let Some(register) = advance_register {
            self.emit(Instruction::CheckAdvance { register });
        }
        if let Some(register) = register {
            self.emit(Instruction::Loop {
                register,
                target: decision,
            });
        }
        let after = self.instructions.len();
        self.patch_preferred_split(decision, body, after, greedy);
        if let Some(register) = advance_register {
            self.release_register(register);
        }
        if let Some(register) = register {
            self.release_register(register);
        }
        Ok(())
    }

    fn compile_unbounded_repetition(
        &mut self,
        atom: &Atom,
        greedy: bool,
        capture_range: Option<(u8, u8)>,
        position: usize,
    ) -> Result<(), CompileError> {
        let decision = self.emit(Instruction::Split {
            first: usize::MAX,
            second: usize::MAX,
        });
        let body = self.instructions.len();
        let advance_register = if atom.can_match_empty() {
            let register = self.allocate_register(position)?;
            self.emit(Instruction::SavePosition { register });
            Some(register)
        } else {
            None
        };
        self.compile_iteration(atom, capture_range)?;
        if let Some(register) = advance_register {
            self.emit(Instruction::CheckAdvance { register });
        }
        self.emit(Instruction::Jump { target: decision });
        let after = self.instructions.len();
        self.patch_preferred_split(decision, body, after, greedy);
        if let Some(register) = advance_register {
            self.release_register(register);
        }
        Ok(())
    }

    fn compile_iteration(
        &mut self,
        atom: &Atom,
        capture_range: Option<(u8, u8)>,
    ) -> Result<(), CompileError> {
        self.emit_capture_reset(capture_range);
        self.compile_atom(atom)
    }

    fn emit_capture_reset(&mut self, capture_range: Option<(u8, u8)>) {
        if let Some((from, to)) = capture_range {
            self.emit(Instruction::ResetCaptures { from, to });
        }
    }

    fn allocate_register(&mut self, position: usize) -> Result<u8, CompileError> {
        if self.register_depth >= u16::from(u8::MAX) {
            return Err(CompileError::too_many_registers(position));
        }
        let register = self.register_depth as u8;
        self.register_depth += 1;
        self.max_registers = self.max_registers.max(self.register_depth);
        Ok(register)
    }

    fn release_register(&mut self, register: u8) {
        debug_assert_eq!(self.register_depth, u16::from(register) + 1);
        self.register_depth -= 1;
    }

    fn emit(&mut self, instruction: Instruction) -> usize {
        let index = self.instructions.len();
        self.instructions.push(instruction);
        index
    }

    fn patch_jump(&mut self, index: usize, target: usize) {
        self.instructions[index] = Instruction::Jump { target };
    }

    fn patch_split(&mut self, index: usize, first: usize, second: usize) {
        self.instructions[index] = Instruction::Split { first, second };
    }

    fn patch_preferred_split(&mut self, index: usize, body: usize, after: usize, greedy: bool) {
        if greedy {
            self.patch_split(index, body, after);
        } else {
            self.patch_split(index, after, body);
        }
    }
}

pub(super) fn compile_units(
    pattern: &[u16],
    flag_units: &[u16],
) -> Result<CompiledRegExp, CompileError> {
    let flags = parse_flags(flag_units)
        .map_err(|error| CompileError::invalid_flags(error.position, error.kind))?;
    if flags.contains(RegExpFlags::UNICODE_SETS) {
        return Err(CompileError::unsupported(
            0,
            UnsupportedFeature::UnicodeSetOperation,
        ));
    }
    let (expression, capture_count) = Parser::new(pattern, flags).parse()?;
    let (instructions, register_count) = CodeBuilder::new(flags).compile(&expression)?;
    Ok(CompiledRegExp::from_parts(
        flags,
        capture_count,
        register_count,
        instructions,
    ))
}

fn digit_ranges() -> Vec<CharacterRange> {
    vec![CharacterRange::new(u32::from(b'0'), u32::from(b'9'))]
}

fn word_ranges() -> Vec<CharacterRange> {
    vec![
        CharacterRange::new(u32::from(b'0'), u32::from(b'9')),
        CharacterRange::new(u32::from(b'A'), u32::from(b'Z')),
        CharacterRange::new(u32::from(b'_'), u32::from(b'_')),
        CharacterRange::new(u32::from(b'a'), u32::from(b'z')),
    ]
}

fn space_ranges() -> Vec<CharacterRange> {
    vec![
        CharacterRange::new(0x0009, 0x000d),
        CharacterRange::new(0x0020, 0x0020),
        CharacterRange::new(0x00a0, 0x00a0),
        CharacterRange::new(0x1680, 0x1680),
        CharacterRange::new(0x2000, 0x200a),
        CharacterRange::new(0x2028, 0x2029),
        CharacterRange::new(0x202f, 0x202f),
        CharacterRange::new(0x205f, 0x205f),
        CharacterRange::new(0x3000, 0x3000),
        CharacterRange::new(0xfeff, 0xfeff),
    ]
}

fn add_class_atom(
    ranges: &mut Vec<CharacterRange>,
    atom: ClassAtom,
    max: u32,
    ignore_case: bool,
    unicode: bool,
) {
    let (mut atom_ranges, complement) = match atom {
        ClassAtom::Single(value) => (vec![CharacterRange::new(value, value)], false),
        ClassAtom::Set(set) => (set, false),
        ClassAtom::ComplementSet(set) => (set, true),
    };
    atom_ranges = atom_ranges
        .into_iter()
        .filter(|range| range.start <= max)
        .map(|range| CharacterRange::new(range.start, range.end.min(max)))
        .collect();
    if ignore_case {
        atom_ranges = canonicalize_ranges(&atom_ranges, unicode);
    } else {
        atom_ranges = normalize_ranges(atom_ranges);
    }
    if complement {
        atom_ranges = complement_ranges(&atom_ranges, max);
    }
    ranges.extend(atom_ranges);
}

fn canonicalize_ranges(ranges: &[CharacterRange], unicode: bool) -> Vec<CharacterRange> {
    let mut canonicalized = Vec::new();
    for range in ranges {
        canonicalized.extend((range.start..=range.end).map(|value| {
            let value = crate::unicode_case::regexp_canonicalize(value, unicode);
            CharacterRange::new(value, value)
        }));
    }
    normalize_ranges(canonicalized)
}

fn normalize_ranges(mut ranges: Vec<CharacterRange>) -> Vec<CharacterRange> {
    ranges.sort_unstable_by_key(|range| (range.start, range.end));
    let mut normalized: Vec<CharacterRange> = Vec::with_capacity(ranges.len());
    for range in ranges {
        if let Some(previous) = normalized.last_mut()
            && range.start <= previous.end.saturating_add(1)
        {
            previous.end = previous.end.max(range.end);
        } else {
            normalized.push(range);
        }
    }
    normalized
}

fn complement_ranges(ranges: &[CharacterRange], max: u32) -> Vec<CharacterRange> {
    let ranges = normalize_ranges(ranges.to_vec());
    let mut complement = Vec::new();
    let mut start = 0_u32;
    for range in ranges {
        if range.start > start {
            complement.push(CharacterRange::new(start, range.start - 1));
        }
        if range.end >= max {
            return complement;
        }
        start = range.end + 1;
    }
    if start <= max {
        complement.push(CharacterRange::new(start, max));
    }
    complement
}

fn is_ascii_digit(unit: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(&unit)
}

fn is_ascii_octal_digit(unit: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'7')).contains(&unit)
}

fn is_ascii_letter(unit: u16) -> bool {
    (u16::from(b'a')..=u16::from(b'z')).contains(&unit)
        || (u16::from(b'A')..=u16::from(b'Z')).contains(&unit)
}

fn is_syntax_character(unit: u16) -> bool {
    matches!(
        unit,
        0x5e | 0x24
            | 0x5c
            | 0x2e
            | 0x2a
            | 0x2b
            | 0x3f
            | 0x28
            | 0x29
            | 0x5b
            | 0x5d
            | 0x7b
            | 0x7d
            | 0x7c
    )
}

fn hex_value(unit: u16) -> Option<u32> {
    match unit {
        0x30..=0x39 => Some(u32::from(unit - 0x30)),
        0x41..=0x46 => Some(u32::from(unit - 0x41 + 10)),
        0x61..=0x66 => Some(u32::from(unit - 0x61 + 10)),
        _ => None,
    }
}

fn is_high_surrogate(value: u32) -> bool {
    (0xd800..=0xdbff).contains(&value)
}

fn is_low_surrogate(value: u32) -> bool {
    (0xdc00..=0xdfff).contains(&value)
}

fn combine_surrogates(high: u32, low: u32) -> u32 {
    0x10000 + ((high - 0xd800) << 10) + (low - 0xdc00)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile_ascii(pattern: &str, flags: &str) -> Result<CompiledRegExp, CompileError> {
        compile_units(
            &pattern.encode_utf16().collect::<Vec<_>>(),
            &flags.encode_utf16().collect::<Vec<_>>(),
        )
    }

    #[test]
    fn flags_match_pinned_bits_and_reject_duplicate_unknown_and_u_v() {
        let compiled = compile_ascii("", "dgimsuy").unwrap();
        assert_eq!(compiled.flags().bits(), 0x7f);
        assert_eq!(compiled.flags().canonical_string(), "dgimsuy");
        assert_eq!(parse_flags(&[u16::from(b'v')]).unwrap().bits(), 1 << 8);
        assert!(matches!(
            compile_ascii("", "v").unwrap_err().kind(),
            CompileErrorKind::Unsupported(UnsupportedFeature::UnicodeSetOperation)
        ));
        for flags in ["gg", "z", "uv", "vu"] {
            let error = compile_ascii("", flags).unwrap_err();
            assert_eq!(error.kind(), &CompileErrorKind::Syntax, "{flags}");
            assert_eq!(error.source(), CompileErrorSource::Flags, "{flags}");
        }
    }

    #[test]
    fn empty_literal_and_unicode_utf16_compile_to_typed_characters() {
        let empty = compile_ascii("", "").unwrap();
        assert_eq!(
            empty.instructions(),
            &[
                Instruction::SaveStart { capture: 0 },
                Instruction::SaveEnd { capture: 0 },
                Instruction::Match,
            ],
        );

        let source = [0x61, 0xd83d, 0xde00, 0xd800];
        let ordinary = compile_units(&source, &[]).unwrap();
        assert!(ordinary.instructions().contains(&Instruction::Char {
            value: 0xd83d,
            ignore_case: false,
        }));
        let unicode = compile_units(&source, &[u16::from(b'u')]).unwrap();
        assert!(unicode.instructions().contains(&Instruction::Char {
            value: 0x1f600,
            ignore_case: false,
        }));
        assert!(unicode.instructions().contains(&Instruction::Char {
            value: 0xd800,
            ignore_case: false,
        }));
    }

    #[test]
    fn dot_anchors_alternation_and_groups_preserve_metadata_and_priority() {
        let compiled = compile_ascii("^(a|(?:b.))$", "ms").unwrap();
        assert_eq!(compiled.capture_count(), 2);
        assert!(
            compiled
                .instructions()
                .contains(&Instruction::LineStart { multiline: true })
        );
        assert!(
            compiled
                .instructions()
                .contains(&Instruction::LineEnd { multiline: true })
        );
        assert!(compiled.instructions().contains(&Instruction::Any));
        assert!(compiled
            .instructions()
            .iter()
            .any(|instruction| matches!(instruction, Instruction::Split { first, second } if first < second)));
    }

    #[test]
    fn greedy_lazy_and_bounded_quantifiers_use_splits_loops_and_guards() {
        let compiled = compile_ascii("a*b+?c{2}d{1,4}e{3,}", "").unwrap();
        assert_eq!(compiled.register_count(), 1);
        assert!(
            compiled
                .instructions()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Split { .. }))
        );
        assert!(
            compiled
                .instructions()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::Loop { .. }))
        );

        let nullable = compile_ascii("(?:a?)*", "").unwrap();
        assert!(
            nullable
                .instructions()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::SavePosition { .. }))
        );
        assert!(
            nullable
                .instructions()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CheckAdvance { .. }))
        );
    }

    #[test]
    fn classes_ranges_inversion_shorthands_and_simple_escapes_compile() {
        let compiled =
            compile_ascii(r"[a-cx\d][^\s]\D\S\w\W\b\B\f\n\r\t\v\x41\u0042", "i").unwrap();
        assert!(compiled.instructions().iter().any(|instruction| {
            matches!(instruction, Instruction::Range { ranges, inverted: false, ignore_case: true }
                if ranges.contains(&CharacterRange::new(u32::from(b'A'), u32::from(b'C'))))
        }));
        assert!(
            compiled
                .instructions()
                .contains(&Instruction::Space { inverted: true })
        );
        assert!(compiled.instructions().contains(&Instruction::Char {
            value: u32::from(b'A'),
            ignore_case: true,
        }));
        assert!(compiled.instructions().contains(&Instruction::Char {
            value: u32::from(b'B'),
            ignore_case: true,
        }));

        let nul_class = compile_ascii(r"[\0]", "").unwrap();
        assert!(nul_class.instructions().iter().any(|instruction| {
            matches!(instruction, Instruction::Range { ranges, inverted: false, .. }
                if ranges.as_ref() == [CharacterRange::new(0, 0)])
        }));

        for pattern in [r"[\d-a]", r"[a-\d]", r"[\d-\w]"] {
            let legacy = compile_ascii(pattern, "").unwrap();
            assert!(legacy.instructions().iter().any(|instruction| {
                matches!(instruction, Instruction::Range { ranges, inverted: false, .. }
                    if ranges.contains(&CharacterRange::new(u32::from(b'-'), u32::from(b'-'))))
            }));
            assert!(compile_ascii(pattern, "u").is_err());
        }
    }

    #[test]
    fn ignore_case_literals_and_classes_are_canonicalized_in_the_ir() {
        let legacy_literal = compile_ascii("a", "i").unwrap();
        assert!(legacy_literal.instructions().contains(&Instruction::Char {
            value: u32::from(b'A'),
            ignore_case: true,
        }));

        let unicode_literal = compile_ascii("A", "iu").unwrap();
        assert!(unicode_literal.instructions().contains(&Instruction::Char {
            value: u32::from(b'a'),
            ignore_case: true,
        }));

        let legacy_class = compile_ascii("[^a]", "i").unwrap();
        assert!(legacy_class.instructions().iter().any(|instruction| {
            matches!(instruction, Instruction::Range { ranges, inverted: true, ignore_case: true }
                if ranges.as_ref() == [CharacterRange::new(u32::from(b'A'), u32::from(b'A'))])
        }));

        let unicode_class = compile_ascii("[A-Z]", "iu").unwrap();
        assert!(unicode_class.instructions().iter().any(|instruction| {
            matches!(instruction, Instruction::Range { ranges, inverted: false, ignore_case: true }
                if ranges.as_ref() == [CharacterRange::new(u32::from(b'a'), u32::from(b'z'))])
        }));
    }

    #[test]
    fn scoped_modifier_grammar_matches_quickjs_error_priority() {
        for pattern in [
            "(?i:a)",
            "(?-i:a)",
            "(?i-:a)",
            "(?ims-:a)",
            "(?-ims:a)",
            "(?im-s:a)",
        ] {
            assert!(compile_ascii(pattern, "").is_ok(), "{pattern}");
        }

        for (pattern, message) in [
            ("(?ii:a)", "duplicate modifier: 'i'"),
            ("(?i-mm:a)", "duplicate modifier: 'm'"),
            ("(?i-i:a)", "invalid modifiers"),
            ("(?ims-m:a)", "invalid modifiers"),
            ("(?-:a)", "invalid modifiers"),
            // QuickJS validates duplicate/overlapping/empty modifier sets
            // before requiring the colon.
            ("(?ii)", "duplicate modifier: 'i'"),
            ("(?i-i)", "invalid modifiers"),
            ("(?-)", "invalid modifiers"),
            ("(?i)", "expecting ':'"),
            ("(?i-x:a)", "expecting ':'"),
            ("(?d:a)", "invalid group specifier"),
        ] {
            let error = compile_ascii(pattern, "").unwrap_err();
            assert_eq!(error.kind(), &CompileErrorKind::Syntax, "{pattern}");
            assert_eq!(error.message(), message, "{pattern}");
        }
    }

    #[test]
    fn scoped_ignore_case_applies_to_literals_classes_and_word_boundaries() {
        let compiled = compile_ascii(r"(?i:a[a]\b)(?-i:b[b]\B)c", "").unwrap();
        assert_eq!(compiled.flags(), RegExpFlags::EMPTY);
        assert_eq!(compiled.flags().canonical_string(), "");

        let characters = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Char { value, ignore_case } => Some((*value, *ignore_case)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            characters,
            vec![
                (u32::from(b'A'), true),
                (u32::from(b'b'), false),
                (u32::from(b'c'), false),
            ],
        );

        let classes = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Range {
                    ranges,
                    inverted: false,
                    ignore_case,
                } => Some((ranges.to_vec(), *ignore_case)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            classes,
            vec![
                (
                    vec![CharacterRange::new(u32::from(b'A'), u32::from(b'A'))],
                    true
                ),
                (
                    vec![CharacterRange::new(u32::from(b'b'), u32::from(b'b'))],
                    false
                ),
            ],
        );

        let boundaries = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::WordBoundary {
                    inverted,
                    ignore_case,
                } => Some((*inverted, *ignore_case)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(boundaries, vec![(false, true), (true, false)]);
    }

    #[test]
    fn nested_scoped_modifiers_restore_the_enclosing_and_global_state() {
        let nested = compile_ascii("(?i:a(?-i:b(?i:c)d)e)f", "").unwrap();
        let characters = nested
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Char { value, ignore_case } => Some((*value, *ignore_case)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            characters,
            vec![
                (u32::from(b'A'), true),
                (u32::from(b'b'), false),
                (u32::from(b'C'), true),
                (u32::from(b'd'), false),
                (u32::from(b'E'), true),
                (u32::from(b'f'), false),
            ],
        );

        let global = compile_ascii("(?-i:a)b", "i").unwrap();
        assert_eq!(global.flags().canonical_string(), "i");
        let characters = global
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Char { value, ignore_case } => Some((*value, *ignore_case)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            characters,
            vec![(u32::from(b'a'), false), (u32::from(b'B'), true)],
        );
    }

    #[test]
    fn scoped_modifier_parser_restores_state_after_nested_parse_errors() {
        for pattern in ["(?i:a", "(?i:(?x:a))", "(?i:[a"] {
            let units = pattern.encode_utf16().collect::<Vec<_>>();
            let mut parser = Parser::new(&units, RegExpFlags::EMPTY);
            assert!(parser.parse_atom().is_err(), "{pattern}");
            assert_eq!(
                parser.modifiers,
                ModifierState::from_flags(RegExpFlags::EMPTY),
                "{pattern}",
            );
            assert_eq!(parser.group_depth, 0, "{pattern}");
        }

        let units = "(?-ims:(?x:a))".encode_utf16().collect::<Vec<_>>();
        let flags = parse_flags(&[u16::from(b'i'), u16::from(b'm'), u16::from(b's')]).unwrap();
        let mut parser = Parser::new(&units, flags);
        assert!(parser.parse_atom().is_err());
        assert_eq!(parser.modifiers, ModifierState::from_flags(flags));
        assert_eq!(parser.group_depth, 0);
    }

    #[test]
    fn scoped_multiline_and_dot_all_apply_only_inside_their_group() {
        let compiled = compile_ascii("^(?ms:^.$)(?-ms:^.$)^.$", "").unwrap();
        assert_eq!(compiled.flags(), RegExpFlags::EMPTY);

        let starts = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::LineStart { multiline } => Some(*multiline),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(starts, vec![false, true, false, false]);

        let ends = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::LineEnd { multiline } => Some(*multiline),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ends, vec![true, false, false]);

        let dots = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Any => Some(true),
                Instruction::Dot => Some(false),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(dots, vec![true, false, false]);

        let global = compile_ascii("(?-ms:^.$)^.$", "ms").unwrap();
        assert_eq!(global.flags().canonical_string(), "ms");
        let dots = global
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Any => Some(true),
                Instruction::Dot => Some(false),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(dots, vec![false, true]);
    }

    #[test]
    fn malformed_core_syntax_is_rejected_at_compile_time() {
        for pattern in ["(", "[a", "*a", "{1}", "a{1}{2}", "a{3,2}", "a**", "(?x:a)"] {
            let error = compile_ascii(pattern, "").unwrap_err();
            assert_eq!(error.source(), CompileErrorSource::Pattern, "{pattern}");
            assert_eq!(error.kind(), &CompileErrorKind::Syntax);
        }
        assert!(compile_ascii("a{not-a-quantifier}", "").is_ok());
        assert!(compile_ascii("a{not-a-quantifier}", "u").is_err());
        for pattern in [r"\!", r"[\!]", r"\-"] {
            assert!(matches!(
                compile_ascii(pattern, "u").unwrap_err().kind(),
                CompileErrorKind::Syntax
            ));
        }
        for pattern in ["^{", "^{1}", r"\b{1}"] {
            assert_eq!(
                compile_ascii(pattern, "u").unwrap_err().message(),
                "regular expression syntax error",
                "{pattern}",
            );
        }
        for pattern in ["a{1", "a{1,", "a{1,x}", "a{1,2"] {
            assert_eq!(
                compile_ascii(pattern, "u").unwrap_err().message(),
                "expecting '}'",
                "{pattern}",
            );
        }
    }

    #[test]
    fn advanced_syntax_is_typed_unsupported_instead_of_miscompiled() {
        for (pattern, feature) in [
            (r"\p{Letter}", UnsupportedFeature::UnicodePropertyEscape),
            (r"(?<name>a)", UnsupportedFeature::NamedCapture),
            (r"\k<name>", UnsupportedFeature::Backreference),
            (r"(?=a)", UnsupportedFeature::Lookaround),
        ] {
            let error = compile_ascii(pattern, "u").unwrap_err();
            assert_eq!(
                error.kind(),
                &CompileErrorKind::Unsupported(feature),
                "{pattern}",
            );
        }
        let error = compile_ascii("[a&&b]", "v").unwrap_err();
        assert_eq!(
            error.kind(),
            &CompileErrorKind::Unsupported(UnsupportedFeature::UnicodeSetOperation),
        );

        for pattern in [r"\c0", r"[\c0]"] {
            let error = compile_ascii(pattern, "").unwrap_err();
            assert_eq!(
                error.kind(),
                &CompileErrorKind::Unsupported(UnsupportedFeature::LegacyControlEscape),
                "{pattern}",
            );
        }
    }

    #[test]
    fn decimal_backreferences_use_the_complete_number_and_total_capture_count() {
        for pattern in [r"\1", r"\2", r"(a)\2", r"(?:a)\1", r"\10"] {
            let error = compile_ascii(pattern, "u").unwrap_err();
            assert_eq!(error.kind(), &CompileErrorKind::Syntax, "{pattern}");
            assert_eq!(
                error.message(),
                "back reference out of range in regular expression",
                "{pattern}",
            );
        }
        for pattern in [r"[\1]", r"[\2]"] {
            let error = compile_ascii(pattern, "u").unwrap_err();
            assert_eq!(error.kind(), &CompileErrorKind::Syntax, "{pattern}");
            assert_eq!(error.message(), "invalid identity escape", "{pattern}");
        }

        for (pattern, captures) in [
            (r"(a)\1", &[1_u8][..]),
            (r"\1(a)", &[1]),
            (r"\2(a)(b)", &[2]),
        ] {
            let compiled = compile_ascii(pattern, "u").unwrap();
            assert!(compiled.instructions().iter().any(|instruction| {
                matches!(instruction, Instruction::BackReference { captures: found, .. }
                    if found.as_ref() == captures)
            }));
        }

        let named = compile_ascii(r"\1(?<x>a)", "u").unwrap_err();
        assert_eq!(
            named.kind(),
            &CompileErrorKind::Unsupported(UnsupportedFeature::NamedCapture),
        );
        assert!(compile_ascii(r"[](a)\1", "u").is_ok());

        let ten_captures = format!("{}\\10", "(a)".repeat(10));
        let compiled = compile_ascii(&ten_captures, "u").unwrap();
        assert!(compiled.instructions().iter().any(|instruction| {
            matches!(instruction, Instruction::BackReference { captures, .. }
                if captures.as_ref() == [10])
        }));
        assert!(!compiled.instructions().iter().any(|instruction| {
            matches!(instruction, Instruction::Char { value, .. } if *value == u32::from(b'0'))
        }));

        let scoped = compile_ascii(r"(a)(?i:\1)(?-i:\1)", "").unwrap();
        let ignore_case = scoped
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::BackReference { ignore_case, .. } => Some(*ignore_case),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(ignore_case, vec![true, false]);

        let nullable = compile_ascii(r"(a)?\1*", "").unwrap();
        assert!(
            nullable
                .instructions()
                .iter()
                .any(|instruction| matches!(instruction, Instruction::CheckAdvance { .. }))
        );

        let capture_limit = format!(r"\255{}", "(a)".repeat(255));
        let error = compile_ascii(&capture_limit, "u").unwrap_err();
        assert_eq!(error.kind(), &CompileErrorKind::Syntax);
        assert_eq!(
            error.message(),
            "back reference out of range in regular expression",
        );

        let too_many_capture_priority = format!("{}((?=a))", "(a)".repeat(254));
        let error = compile_ascii(&too_many_capture_priority, "u").unwrap_err();
        assert_eq!(error.kind(), &CompileErrorKind::TooManyCaptures);
        assert_eq!(error.message(), "too many captures");
    }

    #[test]
    fn annex_b_decimal_fallback_matches_quickjs_legacy_escape_width() {
        let compiled = compile_ascii(r"\1\7\8\9\10\18\377\400\1234\08", "").unwrap();
        let characters = compiled
            .instructions()
            .iter()
            .filter_map(|instruction| match instruction {
                Instruction::Char { value, .. } => Some(*value),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            characters,
            vec![
                0x01,
                0x07,
                u32::from(b'8'),
                u32::from(b'9'),
                0x08,
                0x01,
                u32::from(b'8'),
                0xff,
                0x20,
                u32::from(b'0'),
                u32::from(b'S'),
                u32::from(b'4'),
                0x00,
                u32::from(b'8'),
            ],
        );

        for (pattern, value) in [(r"[\1]", 0x01), (r"[\07]", 0x07), (r"[\377]", 0xff)] {
            let compiled = compile_ascii(pattern, "").unwrap();
            assert!(compiled.instructions().iter().any(|instruction| {
                matches!(instruction, Instruction::Range { ranges, .. }
                    if ranges.as_ref() == [CharacterRange::new(value, value)])
            }));
        }
    }

    #[test]
    fn nesting_is_bounded_and_sequential_quantifiers_reuse_registers() {
        let depth = MAX_GROUP_NESTING + 1;
        let pattern = format!("{}a{}", "(?:".repeat(depth), ")".repeat(depth));
        let error = compile_ascii(&pattern, "").unwrap_err();
        assert_eq!(error.kind(), &CompileErrorKind::Syntax);
        assert_eq!(error.message(), "stack overflow");

        let sequential = "a{2}".repeat(300);
        let compiled = compile_ascii(&sequential, "").unwrap();
        assert_eq!(compiled.register_count(), 1);

        let nested = compile_ascii("(?:(?:a{2}){2}){2}", "").unwrap();
        assert_eq!(nested.register_count(), 3);
    }
}
