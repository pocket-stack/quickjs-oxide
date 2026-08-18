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
mod native_plan;
mod scalar_atom;

#[allow(unused_imports)]
pub(in crate::runtime) use super::graph::sab_transport::{
    ArchivedBytecodeImage, decode_bytecode_image_with_sab_transport,
};
#[allow(unused_imports)]
pub(in crate::runtime::binary_object) use atoms::ImageAtomError;
#[allow(unused_imports)]
use atoms::*;
#[allow(unused_imports)]
use budget::*;
pub(in crate::runtime::binary_object) use budget::{BytecodeImageLimits, ModuleLimits};
#[allow(unused_imports)]
use decode::*;
pub(in crate::runtime::binary_object) use decode::{
    BytecodeImageError, decode_bytecode_image_body,
};
#[allow(unused_imports)]
use encode::*;
#[allow(unused_imports)]
use model::*;
#[allow(unused_imports)]
pub(in crate::runtime::binary_object) use model::{
    BytecodeImage, FunctionId, ImageOpaque, ImageValue, ModuleId,
};
#[allow(unused_imports)]
pub(in crate::runtime::binary_object) use scalar_atom::{
    ImageStringAtomProjection, ImageStringAtomProjectionError,
};

#[cfg(test)]
mod tests;
