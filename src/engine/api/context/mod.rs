//! Realm handle ownership and the shared JavaScript completion boundary.
//!
//! Public operations are implemented in responsibility-specific child modules.
//! Module APIs remain with module execution; host installers remain with their
//! respective qjs and Test262 implementations.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

#[cfg(any(test, feature = "test262-host"))]
use crate::engine::builtins::native::NativeFunctionId;
use crate::engine::builtins::native::PrimitiveKind;
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::code::runtime::Compilation;
use crate::engine::compiler::CompileOptions;
use crate::engine::heap::ContextId;

use crate::engine::object::operations::{InternalDefineResult, InternalSetResult};
use crate::engine::object::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey,
};
use crate::engine::value::Value;
use crate::engine::value::conversion::NativeConversion;
use crate::engine::vm::Completion;

mod bytecode;
mod calls;
mod objects;
mod realm;
mod script;
#[cfg(feature = "test262-host")]
mod test262;

pub use script::EvalOptions;

/// One realm and its execution state.
pub struct Context {
    pub(crate) runtime: Runtime,
    pub(crate) id: u64,
    pub(crate) realm: ContextId,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        self.runtime
            .retain_context_handle(self.realm)
            .expect("a live Context handle must retain its realm");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
            realm: self.realm,
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.runtime.release_context_handle(self.realm);
    }
}

impl Context {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Return the stable arena identity used by runtime jobs and host hooks.
    #[must_use]
    pub const fn realm_id(&self) -> ContextId {
        self.realm
    }

    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    fn finish_completion(&mut self, completion: Completion) -> Result<Value, RuntimeError> {
        match completion {
            Completion::Return(value) => Ok(value),
            Completion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Return whether this runtime currently carries a pending JavaScript
    /// exception completion.
    #[must_use]
    pub fn has_exception(&self) -> bool {
        self.runtime.has_pending_exception()
    }

    /// Move the pending JavaScript exception value out of the runtime slot.
    pub fn take_exception(&mut self) -> Result<Option<Value>, RuntimeError> {
        self.runtime.take_pending_exception()
    }
}
