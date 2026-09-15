//! Accumulator-based `%TypedArray%.prototype` reduction algorithms.
//!
//! Pinned QuickJS validates and snapshots the branded view before checking the
//! callback. Reduction then visits every index in that original range without
//! `HasProperty`, while each element read remains live.

#[cfg(test)]
use super::*;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArrayReduceKind,
    heap::ContextId,
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};

#[cfg(test)]
mod tests;

impl Runtime {
    pub(crate) fn call_typed_array_reduce(
        &self,
        realm: ContextId,
        kind: ArrayReduceKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::traversal::finish(
            self,
            realm,
            super::traversal::TypedTraversalStep::start(
                self,
                realm,
                super::traversal::TypedTraversalKind::Reduce(kind),
                &invocation,
                arguments,
            )?,
        )
    }
}
