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

mod array_driver;
pub(crate) mod bindings;
mod environment_bindings;

mod environment_driver;
pub(crate) mod eval_bindings;

mod eval_driver;
mod property_keys;
mod pure_operations;

mod with_driver;

mod construct_driver;

mod conversion_driver;

mod driver;

pub(crate) mod entry;

pub(crate) use driver::{RootOperation, execute_root};

mod execution;

pub(crate) use execution::HostBoundaryGuard;

mod frame;

mod run;

mod stack;

mod root_call;

pub(crate) mod exception;

pub(crate) mod frames;

pub(crate) mod call;
pub(crate) mod closure;

mod protocol;
pub(crate) use protocol::{CallInput, DirectEvalInvocation, TYPEOF_STATIC_ATOMS};

mod completion;
pub(crate) use completion::{
    BytecodePc, Completion, DefineClassOutcome, ToPrimitiveHint, VmResume, VmSuspendKind,
};

mod numeric;
pub(crate) use numeric::to_js_string_jsvalue;

mod activation;
pub use activation::VmUnwindRegion;

#[cfg(test)]
mod published_execution_tests;

mod arguments_driver;

mod closure_driver;

mod private_bindings;

mod private_access;

mod iterator_driver;
mod iterator_support;

mod apply_driver;

mod frame_exit;

mod frame_operations;

mod property_driver;

mod proxy_get_driver;

mod property_write_driver;

mod super_property_driver;

mod predicate_driver;

mod method_arguments;
