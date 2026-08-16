//! Non-executable, heap-independent state shared by a complete BC5 bytecode image.
//!
//! This module owns the bytecode header's semantic atom remap, the bounded
//! whole-image `FunctionBytecode`/`Module` decoder, and the final immutable
//! image model.
//! It never allocates runtime objects or makes native bytecode executable.

mod atoms;
mod budget;
mod decode;
mod encode;
mod model;

#[allow(unused_imports)]
use atoms::*;
#[allow(unused_imports)]
use budget::*;
#[allow(unused_imports)]
use decode::*;
#[allow(unused_imports)]
use encode::*;
#[allow(unused_imports)]
use model::*;
#[allow(unused_imports)]
pub(in crate::runtime::binary_object) use model::{FunctionId, ImageOpaque, ImageValue, ModuleId};

#[cfg(test)]
mod tests;
