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

use crate::value::JsString;

/// Compile exact UTF-16 pattern and flag strings without consulting runtime
/// objects or host regular-expression facilities.
pub fn compile(pattern: &JsString, flags: &JsString) -> Result<CompiledRegExp, CompileError> {
    compiler::compile_units(
        &pattern.utf16_units().collect::<Vec<_>>(),
        &flags.utf16_units().collect::<Vec<_>>(),
    )
}
