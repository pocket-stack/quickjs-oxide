//! `%Proxy%` allocation and revocation lifecycle.
//!
//! Observable internal-method dispatch lives in the runtime's exotic-method
//! layer. This module mirrors the pinned QuickJS constructor boundary:
//! genuine Proxy objects have a null physical prototype, cache the target's
//! call/construct capabilities at creation time, and retain target and handler
//! even after a one-shot revocation closure is consumed.

use crate::heap::InternalCallableData;

use super::super::*;

impl Runtime {
    /// Publish `%Proxy%` with QuickJS's exact initial own-property surface.
    ///
    /// The constructor owns `length`, `name`, then `revocable` in that order.
    /// It deliberately has no own `prototype` property.
    pub(in crate::runtime) fn initialize_proxy_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ProxyConstructor,
            2,
            "Proxy",
            2,
        )?;
        self.set_constructor_bit(constructor.as_object(), true)?;
        self.define_native_builtin_auto_init(
            constructor.as_object(),
            realm,
            NativeFunctionId::ProxyRevocable,
            "revocable",
            2,
            2,
        )?;
        self.define_function_data_property(
            global_object,
            "Proxy",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )
    }

    /// Allocate one genuine Proxy after validating the two object operands.
    ///
    /// QuickJS uses a null physical prototype for `JS_CLASS_PROXY`; every
    /// observable prototype operation is therefore handled by the exotic
    /// internal-method dispatcher rather than this shape.
    pub(in crate::runtime) fn new_proxy(
        &self,
        realm: ContextId,
        target: Value,
        handler: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let (Value::Object(target), Value::Object(handler)) = (target, handler) else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        if !target.belongs_to(self) || !handler.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Proxy target or handler"));
        }

        let (is_callable, is_constructor) = {
            let state = self.0.state.borrow();
            let target_data = state.heap.object(target.object_id())?;
            let is_callable = matches!(
                &target_data.payload,
                ObjectPayload::NativeFunction { .. }
                    | ObjectPayload::BoundFunction { .. }
                    | ObjectPayload::BytecodeFunction { .. }
                    | ObjectPayload::Proxy(crate::heap::ProxyData {
                        is_callable: true,
                        ..
                    })
            );
            (is_callable, target_data.is_constructor)
        };

        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(None, &[])?;
        let object = match state.heap.allocate_object(ObjectData::proxy(
            shape,
            Vec::new(),
            target.object_id(),
            handler.object_id(),
            is_callable,
            is_constructor,
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);

        Ok(NativeConversion::Value(ObjectRef::from_owned_handle(
            self.clone(),
            object,
        )))
    }

    /// Native `%Proxy%` constructor entrypoint.
    pub(in crate::runtime) fn call_proxy_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Proxy constructor did not receive a constructor invocation",
            ));
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant("Proxy target argv was not padded"))?;
        let handler = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant("Proxy handler argv was not padded"))?;
        match self.new_proxy(realm, target, handler)? {
            NativeConversion::Value(proxy) => Ok(Completion::Return(Value::Object(proxy))),
            NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    /// Native `Proxy.revocable` entrypoint.
    pub(in crate::runtime) fn call_proxy_revocable(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Proxy.revocable did not receive a call invocation",
            ));
        };
        let target = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Proxy.revocable target argv was not padded",
            ))?;
        let handler = arguments
            .readable
            .get(1)
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Proxy.revocable handler argv was not padded",
            ))?;
        let proxy = match self.new_proxy(realm, target, handler)? {
            NativeConversion::Value(proxy) => proxy,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let revoke = self.new_internal_promise_function(
            realm,
            NativeFunctionId::ProxyRevoke,
            0,
            0,
            InternalCallableData::ProxyRevoke {
                proxy: Some(proxy.object_id()),
            },
        )?;

        let object_prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
        let object_prototype = ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?;
        let result = self.new_object(Some(&object_prototype))?;
        for (name, value) in [
            ("proxy", Value::Object(proxy)),
            ("revoke", Value::Object(revoke.as_object().clone())),
        ] {
            let key = self.intern_property_key(name)?;
            if !self.define_own_property(
                &result,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                return Err(RuntimeError::Invariant(
                    "Proxy.revocable result property definition was rejected",
                ));
            }
        }
        Ok(Completion::Return(Value::Object(result)))
    }

    /// Consume a revocation closure's capture exactly once.
    pub(in crate::runtime) fn call_proxy_revoke(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Proxy revoke function did not receive a call invocation",
            ));
        };
        let active = self.active_function()?;
        let mut state = self.0.state.borrow_mut();
        let (_, cleanup) = state.heap.revoke_proxy_from_callable(active.object_id())?;
        state.apply_cleanup(cleanup)?;
        Ok(Completion::Return(Value::Undefined))
    }
}
