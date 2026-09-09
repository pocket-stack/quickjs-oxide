use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};

use crate::engine::code::bytecode::{
    ApplyKind, ArgumentsKind, DefineMethodKind, DynamicEnvironmentSource, EvalVariableSource,
    Instruction, IteratorCallKind, PrivateNameSource,
};
#[cfg(test)]
use crate::engine::code::bytecode::{DetachedBytecode, TestConstant};
use crate::engine::code::function::metadata::FunctionMetadata;
use crate::engine::heap::ContextId;
use crate::engine::object::ObjectRef;

use crate::engine::value::bigint::{BigIntError, JsBigInt};
use crate::engine::value::{JsString, Value};
use num_bigint::BigInt;
use num_traits::FromPrimitive;
#[cfg(test)]
use std::collections::VecDeque;

#[cfg(test)]
mod tests;

pub(crate) mod async_from_sync_iterator;

pub(crate) mod async_function;

pub(crate) mod async_generator;

pub(crate) mod for_in;

pub(crate) mod generator;

pub(crate) mod native_stack;

pub(crate) mod host_bridge;

pub(crate) mod exception;

pub(crate) mod frames;

pub(crate) mod call;

mod protocol;
pub use protocol::*;

mod completion;
pub(crate) use completion::*;

mod numeric;
use numeric::*;

#[cfg(test)]
mod detached;
#[cfg(test)]
use detached::*;

mod activation;
pub use activation::*;

mod dispatch;
mod frame_execution;
mod numeric_execution;
mod unwind;
