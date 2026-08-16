//! Non-executable QuickJS 2026-06-04 FunctionBytecode envelope foundation.
//!
//! The prefix owns flags, frame metadata, locals, closures, scanned function
//! bytecode, and optional debug bytes. Constant-pool values remain pending for
//! a future whole-image decoder with one shared object-reference arena.
//! Parsing this prefix never admits tag 12 to the data graph or to execution.

mod model;
mod prefix;

#[allow(unused_imports)]
pub(in crate::runtime) use model::*;
#[allow(unused_imports)]
pub(in crate::runtime) use prefix::*;

#[cfg(test)]
mod tests;
