//! Callback-based `%TypedArray%.prototype` find algorithms.
//!
//! Pinned QuickJS validates and snapshots the branded view once, then visits
//! every index in that original range without a `HasProperty` check. Each
//! element read remains live: shrink, detach, regrow, and callback writes are
//! observed at the next index while growth never extends the traversal.

#[cfg(test)]
use super::*;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArrayFindKind,
    heap::ContextId,
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};

#[cfg(test)]
mod tests;

impl Runtime {
    pub(crate) fn call_typed_array_find(
        &self,
        realm: ContextId,
        kind: ArrayFindKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::traversal::finish(
            self,
            realm,
            super::traversal::TypedTraversalStep::start(
                self,
                realm,
                super::traversal::TypedTraversalKind::Find(kind),
                &invocation,
                arguments,
            )?,
        )
    }
}
