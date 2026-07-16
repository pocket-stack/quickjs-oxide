//! Pure Rust RegExp engine foundation for pinned QuickJS 2026-06-04.
//!
//! This module is runtime independent: it owns typed flags, compiler errors,
//! and instruction IR, while heap objects and ECMAScript intrinsics are wired
//! by later layers.

mod compiler;
mod executor;
mod flags;
mod opcode;

pub use compiler::{
    CompileError, CompileErrorKind, CompileErrorSource, CompiledRegExp, UnsupportedFeature,
};
pub use executor::{ExecError, ProgramError, RegExpMatch, execute, execute_with_interrupt};
pub use flags::RegExpFlags;
pub use opcode::{CharacterRange, Instruction};

use crate::error::ErrorKind;
use crate::value::JsString;

/// Compile exact UTF-16 pattern and flag strings without consulting runtime
/// objects or host regular-expression facilities.
pub fn compile(pattern: &JsString, flags: &JsString) -> Result<CompiledRegExp, CompileError> {
    compiler::compile_units(
        &pattern.utf16_units().collect::<Vec<_>>(),
        &flags.utf16_units().collect::<Vec<_>>(),
    )
}

/// Classify a pure-Rust compiler failure at the JavaScript compilation
/// boundary. Unsupported engine features stay distinct from conforming
/// `SyntaxError`s so Test262 bookkeeping cannot count an implementation gap as
/// a language rejection.
#[must_use]
pub(crate) const fn javascript_compile_error_kind(error: &CompileError) -> ErrorKind {
    match error.kind() {
        CompileErrorKind::Unsupported(_) => ErrorKind::Unsupported,
        CompileErrorKind::Syntax
        | CompileErrorKind::TooManyCaptures
        | CompileErrorKind::TooManyRegisters => ErrorKind::Syntax,
    }
}

/// Return the short diagnostic exposed by pinned QuickJS `lre_compile`.
///
/// The typed compiler retains source and UTF-16 position metadata in its
/// `Display` form; JavaScript compilation deliberately exposes only this
/// QuickJS-compatible message.
#[must_use]
pub(crate) fn javascript_compile_error_message(error: &CompileError) -> &str {
    if error.source() == CompileErrorSource::Flags {
        return "invalid regular expression flags";
    }
    match error.message() {
        "unexpected end of pattern" | "unterminated character class" | "trailing backslash" => {
            "unexpected end"
        }
        "unterminated group" => "expecting ')'",
        "unexpected closing parenthesis" => "extraneous characters at the end",
        "invalid group specifier" => "invalid group",
        "regular expression syntax error" => "syntax error",
        "invalid quantifier target" => "nothing to repeat",
        "invalid control escape"
        | "invalid hexadecimal escape"
        | "invalid Unicode escape"
        | "invalid identity escape" => "invalid escape sequence in regular expression",
        message => message,
    }
}
