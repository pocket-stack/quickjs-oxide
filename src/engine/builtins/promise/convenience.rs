//! Promise convenience constructors and the callback-free race combinator.
//!
//! These paths deliberately preserve pinned QuickJS's ordering: capability
//! creation precedes every observable callback, post-capability abrupt
//! completions reject through that capability, and `race` closes an acquired
//! iterator only after failures in `constructor.resolve` or the dynamic
//! `then` invocation. Iterator-step failures themselves do not close it.

use super::*;

impl Runtime {
    pub(crate) fn call_promise_with_resolvers(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::convenience(
            self,
            realm,
            PromiseNativeKind::WithResolvers,
            &invocation,
            &NativeArguments {
                readable: Vec::new(),
                actual_arg_count: 0,
            },
        )?
        .finish(self, realm)
    }

    pub(crate) fn call_promise_try(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::convenience(
            self,
            realm,
            PromiseNativeKind::Try,
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    pub(crate) fn call_promise_race(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::PromiseStep::aggregate(
            self,
            realm,
            PromiseNativeKind::Race,
            &invocation,
            arguments,
        )?
        .finish(self, realm)
    }

    pub(crate) fn promise_callable(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<CallableRef>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        match self.as_callable(&object)? {
            Some(callable) => Ok(NativeConversion::Value(callable)),
            None => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?)),
        }
    }
}
