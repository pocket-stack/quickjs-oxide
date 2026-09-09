//! Opt-in observation surface for differential tests, absent from production builds.
pub use crate::engine::compiler::lexer::{Lexer, TokenKind};
pub use crate::engine::value::number::{to_exponential, to_fixed, to_precision, to_string_radix};
pub use crate::engine::value::number_parse::{parse_float, parse_int};
