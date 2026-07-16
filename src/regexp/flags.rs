//! Typed flags from pinned QuickJS `libregexp.h` lines 30-38.

/// Flags recorded by one compiled regular-expression program.
///
/// Bit values intentionally match pinned QuickJS so a later packed-bytecode
/// encoder can preserve the release's header format without translation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RegExpFlags(u16);

impl RegExpFlags {
    pub const GLOBAL: Self = Self(1 << 0);
    pub const IGNORE_CASE: Self = Self(1 << 1);
    pub const MULTILINE: Self = Self(1 << 2);
    pub const DOT_ALL: Self = Self(1 << 3);
    pub const UNICODE: Self = Self(1 << 4);
    pub const STICKY: Self = Self(1 << 5);
    pub const HAS_INDICES: Self = Self(1 << 6);
    /// Internal compiled-program marker. It matches QuickJS's bytecode-header
    /// bit and is intentionally absent from the observable flags string.
    pub const NAMED_GROUPS: Self = Self(1 << 7);
    pub const UNICODE_SETS: Self = Self(1 << 8);

    pub const EMPTY: Self = Self(0);

    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn contains(self, flag: Self) -> bool {
        (self.0 & flag.0) != 0
    }

    #[must_use]
    pub const fn is_unicode(self) -> bool {
        self.contains(Self::UNICODE) || self.contains(Self::UNICODE_SETS)
    }

    /// Canonical observable order used by QuickJS's `flags` getter: dgimsuvy.
    #[must_use]
    pub fn canonical_string(self) -> String {
        let mut output = String::with_capacity(8);
        for (flag, character) in [
            (Self::HAS_INDICES, 'd'),
            (Self::GLOBAL, 'g'),
            (Self::IGNORE_CASE, 'i'),
            (Self::MULTILINE, 'm'),
            (Self::DOT_ALL, 's'),
            (Self::UNICODE, 'u'),
            (Self::UNICODE_SETS, 'v'),
            (Self::STICKY, 'y'),
        ] {
            if self.contains(flag) {
                output.push(character);
            }
        }
        output
    }

    pub(super) fn insert(&mut self, flag: Self) -> bool {
        let duplicate = self.contains(flag);
        self.0 |= flag.0;
        !duplicate
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FlagParseErrorKind {
    Invalid,
    Duplicate,
    UnicodeConflict,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct FlagParseError {
    pub position: usize,
    pub kind: FlagParseErrorKind,
}

pub(super) fn parse_flags(units: &[u16]) -> Result<RegExpFlags, FlagParseError> {
    let mut flags = RegExpFlags::EMPTY;
    for (position, unit) in units.iter().copied().enumerate() {
        let flag = match unit {
            0x64 => RegExpFlags::HAS_INDICES,
            0x67 => RegExpFlags::GLOBAL,
            0x69 => RegExpFlags::IGNORE_CASE,
            0x6d => RegExpFlags::MULTILINE,
            0x73 => RegExpFlags::DOT_ALL,
            0x75 => RegExpFlags::UNICODE,
            0x76 => RegExpFlags::UNICODE_SETS,
            0x79 => RegExpFlags::STICKY,
            _ => {
                return Err(FlagParseError {
                    position,
                    kind: FlagParseErrorKind::Invalid,
                });
            }
        };
        if !flags.insert(flag) {
            return Err(FlagParseError {
                position,
                kind: FlagParseErrorKind::Duplicate,
            });
        }
    }
    if flags.contains(RegExpFlags::UNICODE) && flags.contains(RegExpFlags::UNICODE_SETS) {
        let position = units
            .iter()
            .position(|unit| *unit == u16::from(b'v'))
            .unwrap_or(units.len());
        return Err(FlagParseError {
            position,
            kind: FlagParseErrorKind::UnicodeConflict,
        });
    }
    Ok(flags)
}
