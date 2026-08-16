//! Non-executable, heap-independent state shared by a complete BC5 function image.
//!
//! This module currently owns only the bytecode header's semantic atom remap.
//! It does not decode `FunctionBytecode` records, allocate runtime objects, or
//! make bytecode executable.

mod atoms;

#[allow(unused_imports)]
use atoms::*;

#[cfg(test)]
mod tests;
