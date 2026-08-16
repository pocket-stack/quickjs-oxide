//! Non-executable, heap-independent state shared by a complete BC5 function image.
//!
//! This module owns the bytecode header's semantic atom remap, the bounded
//! whole-image `FunctionBytecode` decoder, and the final immutable image model.
//! It never allocates runtime objects or makes native bytecode executable.

mod atoms;
mod decode;
mod model;

#[allow(unused_imports)]
use atoms::*;
#[allow(unused_imports)]
use decode::*;
#[allow(unused_imports)]
use model::*;
#[allow(unused_imports)]
pub(in crate::runtime::binary_object) use model::{FunctionId, ImageValue};

#[cfg(test)]
mod tests;
