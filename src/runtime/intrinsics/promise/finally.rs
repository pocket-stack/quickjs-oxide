//! `Promise.prototype.finally` and its typed internal callbacks.
//!
//! Pinned QuickJS deliberately carries `undefined` through
//! `SpeciesConstructor` as the default-constructor sentinel.  The outer
//! finally callbacks preserve that value rather than substituting the realm's
//! intrinsic Promise constructor, including QuickJS's observable TypeError
//! when the callback later reaches `PromiseResolve(undefined, result)`.

use super::*;

impl Runtime {
    pub(super) fn call_promise_finally(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise.prototype.finally received a constructor invocation",
            ));
        };
        let Value::Object(receiver) = &this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };

        // QuickJS performs SpeciesConstructor before even testing whether the
        // user-supplied finally callback is callable.
        let constructor = match self.promise_species_constructor(realm, receiver)? {
            NativeConversion::Value(constructor) => constructor,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let on_finally = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise.finally callback argv was not padded",
            ))?;
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
        self.invoke_promise_then(realm, this_value, &handlers)
    }

    pub(in crate::runtime) fn call_promise_finally_handler(
        &self,
        realm: ContextId,
        kind: PromiseReactionKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise finally handler received a constructor invocation",
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
                "Promise finally handler had no internal capture",
            ))?;
        let InternalCallableData::PromiseFinallyHandler {
            constructor,
            on_finally,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "Promise finally handler had the wrong internal capture",
            ));
        };
        let settlement = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Promise finally handler argv was not padded",
            ))?;
        let on_finally = ObjectRef::from_borrowed_handle(self.clone(), on_finally)?;
        let on_finally = self
            .as_callable(&on_finally)?
            .ok_or(RuntimeError::Invariant(
                "Promise finally callback was no longer callable",
            ))?;
        let callback_result = match self.call_internal(realm, &on_finally, Value::Undefined, &[])? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let constructor = match constructor {
            Some(constructor) => {
                Value::Object(ObjectRef::from_borrowed_handle(self.clone(), constructor)?)
            }
            None => Value::Undefined,
        };
        let resolve_arguments = NativeArguments {
            readable: vec![callback_result],
            actual_arg_count: 1,
        };
        let promise = match self.call_promise_static_resolve(
            realm,
            PromiseNativeKind::Resolve,
            NativeInvocation::Call {
                this_value: constructor,
            },
            &resolve_arguments,
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        drop(resolve_arguments);

        let raw_settlement = self.raw_property_value(&settlement)?;
        let thunk = self.new_internal_promise_function(
            realm,
            NativeFunctionId::PromiseFinallyThunk(kind),
            0,
            0,
            InternalCallableData::PromiseFinallyThunk {
                value: raw_settlement,
            },
        )?;
        drop(settlement);
        self.invoke_promise_then(realm, promise, &[Value::Object(thunk.as_object().clone())])
    }

    pub(in crate::runtime) fn call_promise_finally_thunk(
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
