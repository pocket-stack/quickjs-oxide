use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};

use crate::engine::code::bytecode::{
    ApplyKind, ArgumentsKind, DefineMethodKind, DynamicEnvironmentSource, EvalVariableSource,
    Instruction, IteratorCallKind, PrivateNameSource,
};
#[cfg(test)]
use crate::engine::code::bytecode::{DetachedBytecode, TestConstant};
use crate::engine::heap::ContextId;
use crate::engine::object::ObjectRef;

#[cfg(test)]
use crate::engine::value::bigint::JsBigInt;
use crate::engine::value::{JsString, Value};
#[cfg(test)]
use std::collections::VecDeque;

#[cfg(test)]
mod numeric_coercion_tests;
#[cfg(test)]
mod tests;

pub(crate) mod async_from_sync_iterator;

pub(crate) mod async_function;

pub(crate) mod async_generator;

pub(crate) mod for_in;

pub(crate) mod generator;

pub(crate) mod suspend;

pub(crate) mod native_stack;

#[cfg(feature = "stack-vm")]
mod array_driver;
pub(crate) mod bindings;
mod environment_bindings;
#[cfg(feature = "stack-vm")]
mod environment_driver;
mod eval_bindings;
#[cfg(feature = "stack-vm")]
mod eval_driver;
mod property_keys;
mod pure_operations;
#[cfg(feature = "stack-vm")]
mod with_driver;

#[cfg(feature = "stack-vm")]
mod construct_driver;
#[cfg(feature = "stack-vm")]
mod conversion_driver;
#[cfg(feature = "stack-vm")]
mod driver;
#[cfg(feature = "stack-vm")]
pub(crate) mod entry;
#[cfg(feature = "stack-vm")]
pub(crate) use driver::{RootOperation, execute_root};
#[cfg(feature = "stack-vm")]
mod execution;
#[cfg(feature = "stack-vm")]
pub(crate) use execution::HostBoundaryGuard;
#[cfg(feature = "stack-vm")]
mod frame;
#[cfg(feature = "stack-vm")]
mod run;
#[cfg(feature = "stack-vm")]
mod stack;

pub(crate) mod host_bridge;

pub(crate) mod exception;

pub(crate) mod frames;

pub(crate) mod call;
pub(crate) mod closure;

mod protocol;
pub use protocol::*;

mod completion;
pub(crate) use completion::*;

mod numeric;
#[cfg(test)]
use numeric::to_primitive;

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

#[cfg(test)]
mod published_execution_tests;

#[cfg(feature = "stack-vm")]
mod arguments_driver;

#[cfg(feature = "stack-vm")]
mod closure_driver;

mod private_bindings;

#[cfg(feature = "stack-vm")]
mod private_access;

#[cfg(feature = "stack-vm")]
mod iterator_driver;
mod iterator_support;

#[cfg(feature = "stack-vm")]
mod apply_driver;

#[cfg(feature = "stack-vm")]
mod frame_exit;
#[cfg(feature = "stack-vm")]
mod frame_operations;

#[cfg(feature = "stack-vm")]
mod call_bridge;

#[cfg(feature = "stack-vm")]
mod property_driver;

#[cfg(feature = "stack-vm")]
mod proxy_get_driver;

#[cfg(feature = "stack-vm")]
mod property_write_driver;
#[cfg(feature = "stack-vm")]
mod super_property_driver;

#[cfg(feature = "stack-vm")]
mod predicate_driver;

mod method_arguments;
