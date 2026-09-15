//! `Promise.prototype.finally` and its typed internal callbacks.
//!
//! Pinned QuickJS deliberately carries `undefined` through
//! `SpeciesConstructor` as the default-constructor sentinel.  The outer
//! finally callbacks preserve that value rather than substituting the realm's
//! intrinsic Promise constructor, including QuickJS's observable TypeError
//! when the callback later reaches `PromiseResolve(undefined, result)`.

use super::*;

impl Runtime {
    pub(crate) fn call_promise_finally(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::start(
            self,
            realm,
            NativeFunctionId::Promise(PromiseNativeKind::Finally),
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    pub(super) fn prepare_promise_finally_handlers(
        &self,
        realm: ContextId,
        constructor: Option<ConstructorRef>,
        on_finally: Value,
    ) -> Result<[Value; 2], RuntimeError> {
        let callable = match &on_finally {
            Value::Object(object) => self.as_callable(object)?,
            _ => None,
        };

        let handlers = if let Some(callable) = callable {
            let constructor = constructor
                .as_ref()
                .map(|constructor| constructor.as_object().object_id());
            let capture = || InternalCallableData::PromiseFinallyHandler {
                constructor,
                on_finally: callable.as_object().object_id(),
            };
            let fulfill = self.new_internal_promise_function(
                realm,
                NativeFunctionId::PromiseFinallyHandler(PromiseReactionKind::Fulfill),
                1,
                1,
                capture(),
            )?;
            let reject = self.new_internal_promise_function(
                realm,
                NativeFunctionId::PromiseFinallyHandler(PromiseReactionKind::Reject),
                1,
                1,
                capture(),
            )?;
            [
                Value::Object(fulfill.as_object().clone()),
                Value::Object(reject.as_object().clone()),
            ]
        } else {
            [on_finally.clone(), on_finally.clone()]
        };

        // The internal handlers (or argument copies) now own every edge needed
        // by the dynamic then call, matching QuickJS's pre-Invoke releases.
        drop(constructor);
        drop(on_finally);
        Ok(handlers)
    }

    pub(crate) fn call_promise_finally_handler(
        &self,
        realm: ContextId,
        kind: PromiseReactionKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::start(
            self,
            realm,
            NativeFunctionId::PromiseFinallyHandler(kind),
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    pub(crate) fn call_promise_finally_thunk(
        &self,
        kind: PromiseReactionKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise finally thunk received a constructor invocation",
            ));
        };
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "Promise finally thunk had no internal capture",
            ))?;
        let InternalCallableData::PromiseFinallyThunk { value } = internal else {
            return Err(RuntimeError::Invariant(
                "Promise finally thunk had the wrong internal capture",
            ));
        };
        let value = self.root_raw_value(&value)?;
        Ok(match kind {
            PromiseReactionKind::Fulfill => Completion::Return(value),
            PromiseReactionKind::Reject => Completion::Throw(value),
        })
    }
}
