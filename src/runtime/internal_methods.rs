//! Completion-aware ECMAScript internal-method dispatch.
//!
//! Physical ordinary/Array/Arguments/String storage remains in
//! [`super::properties`].  This module is the observable semantic boundary:
//! every operation first selects an exotic implementation and otherwise
//! delegates to the existing ordinary kernel.  Keeping JavaScript calls here,
//! above the heap, ensures no `RefCell` borrow survives a re-entrant Proxy
//! trap.

use crate::heap::ProxyData;
use std::collections::HashSet;

use super::*;

#[derive(Clone)]
struct RootedProxy {
    proxy: ObjectRef,
    data: ProxyData,
    target: ObjectRef,
    handler: ObjectRef,
}

struct ProxyOwnKeys {
    keys: Vec<PropertyKey>,
    key_atoms: HashSet<Atom>,
}

struct ProxyMethodStackGuard {
    runtime: Runtime,
}

impl ProxyMethodStackGuard {
    fn enter(runtime: &Runtime) -> Self {
        runtime
            .0
            .proxy_method_depth
            .set(runtime.0.proxy_method_depth.get().saturating_add(1));
        Self {
            runtime: runtime.clone(),
        }
    }
}

impl Drop for ProxyMethodStackGuard {
    fn drop(&mut self) {
        self.runtime
            .0
            .proxy_method_depth
            .set(self.runtime.0.proxy_method_depth.get().saturating_sub(1));
    }
}

impl Runtime {
    fn proxy_method_chain_limit(&self, name: &'static str) -> Option<usize> {
        // Empty-handler forwarding is recursive C code in the pinned build.
        // Four fallback shapes compile as tail calls and remain effectively
        // unbounded; the others retain differently sized native frames. Rust
        // forwards iteratively for host safety, so charge broad frame classes
        // against the same one-MiB logical budget. These powers of two encode
        // the source-level call shape, not machine-specific measured depths.
        let logical_frame_bytes = match name {
            "getPrototypeOf" | "has" => 128,
            "setPrototypeOf" => 64,
            "isExtensible" | "preventExtensions" => return None,
            "get" => 512,
            "set" | "deleteProperty" | "ownKeys" | "construct" => 256,
            "getOwnPropertyDescriptor" | "defineProperty" => return None,
            "apply" => 1024,
            _ => return Some(0),
        };
        Some(self.proxy_method_logical_stack_budget() / logical_frame_bytes)
    }

    pub(in crate::runtime) fn direct_call_target_from_value(
        &self,
        value: Value,
    ) -> Result<DirectCallTarget, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("call target"));
        }
        if let Some(callable) = self.as_callable(&object)? {
            return Ok(DirectCallTarget::Callable(callable));
        }
        if self.is_proxy_object(&object)? {
            return Ok(DirectCallTarget::NonCallableProxy(object));
        }
        Err(RuntimeError::Engine(Error::new(
            ErrorKind::Type,
            "not a function",
        )))
    }

    pub(in crate::runtime) fn call_value_internal(
        &self,
        caller_realm: ContextId,
        function: Value,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        match self.direct_call_target_from_value(function)? {
            DirectCallTarget::Callable(callable) => {
                self.call_internal(caller_realm, &callable, this_value, arguments)
            }
            DirectCallTarget::NonCallableProxy(proxy) => {
                self.call_proxy(caller_realm, &proxy, this_value, arguments)
            }
        }
    }

    /// Raw realm lookup retained for internal callers which have already
    /// excluded revoked Proxy wrappers.
    #[cfg(test)]
    pub(in crate::runtime) fn callable_realm(
        &self,
        callable: &CallableRef,
    ) -> Result<ContextId, RuntimeError> {
        match self.function_realm_object_impl(None, callable.as_object().clone(), false)? {
            NativeConversion::Value(realm) => Ok(realm),
            NativeConversion::Throw(_) => Err(RuntimeError::Invariant(
                "raw callable realm lookup produced a JavaScript throw",
            )),
        }
    }

    /// Completion-aware QuickJS `JS_GetFunctionRealm`.
    ///
    /// Bound functions and Proxy wrappers are unwrapped recursively. A revoked
    /// Proxy throws in the caller's realm instead of leaking an engine error.
    pub(in crate::runtime) fn function_realm(
        &self,
        caller_realm: ContextId,
        callable: &CallableRef,
    ) -> Result<NativeConversion<ContextId>, RuntimeError> {
        self.function_realm_object_impl(Some(caller_realm), callable.as_object().clone(), false)
    }

    /// Raw-value form of QuickJS `JS_GetFunctionRealm`. Non-functions and
    /// primitives fall back to the current realm; Proxy and bound wrappers are
    /// still traversed so revocation and nested function realms remain
    /// observable after a `newTarget.prototype` lookup.
    pub(in crate::runtime) fn function_realm_from_value(
        &self,
        caller_realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<ContextId>, RuntimeError> {
        self.0.state.borrow().heap.context(caller_realm)?;
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Value(caller_realm));
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function realm value"));
        }
        self.function_realm_object_impl(Some(caller_realm), object.clone(), true)
    }

    fn function_realm_object_impl(
        &self,
        caller_realm: Option<ContextId>,
        object: ObjectRef,
        allow_non_function: bool,
    ) -> Result<NativeConversion<ContextId>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function realm object"));
        }
        let mut object = object;
        loop {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            match &object_data.payload {
                ObjectPayload::NativeFunction { data, .. } if data.realm.is_some() => {
                    if allow_non_function && data.target.uses_calling_realm() {
                        let realm = caller_realm.ok_or(RuntimeError::Invariant(
                            "raw function realm lookup had no fallback realm",
                        ))?;
                        state.heap.context(realm)?;
                        return Ok(NativeConversion::Value(realm));
                    }
                    let realm = data
                        .realm
                        .expect("guard proved native function has a defining realm");
                    state.heap.context(realm)?;
                    return Ok(NativeConversion::Value(realm));
                }
                ObjectPayload::BytecodeFunction { bytecode, .. } => {
                    let realm = state.heap.function_bytecode(*bytecode)?.realm;
                    state.heap.context(realm)?;
                    return Ok(NativeConversion::Value(realm));
                }
                ObjectPayload::BoundFunction { target, .. } => {
                    let target = *target;
                    drop(state);
                    object = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                }
                ObjectPayload::Proxy(data) => {
                    let is_revoked = data.is_revoked;
                    let target = data.target;
                    drop(state);
                    if is_revoked {
                        return match caller_realm {
                            Some(realm) => self.proxy_revoked_throw(realm),
                            None => Err(RuntimeError::Engine(Error::new(
                                ErrorKind::Type,
                                "revoked proxy",
                            ))),
                        };
                    }
                    object = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                }
                ObjectPayload::NativeFunction { .. } => {
                    return Err(RuntimeError::Invariant(
                        "native function has no defining realm",
                    ));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::AsyncFunctionState(_)
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::ArrayBuffer(_)
                | ObjectPayload::SharedArrayBuffer(_)
                | ObjectPayload::DataView(_)
                | ObjectPayload::TypedArray(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::AsyncFromSyncIterator(_)
                | ObjectPayload::IteratorConcat(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::WeakMap { .. }
                | ObjectPayload::WeakSet { .. }
                | ObjectPayload::WeakRef { .. }
                | ObjectPayload::FinalizationRegistry(_)
                | ObjectPayload::SetIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::Generator { .. }
                | ObjectPayload::AsyncGenerator(_) => {
                    if allow_non_function {
                        let realm = caller_realm.ok_or(RuntimeError::Invariant(
                            "raw function realm lookup had no fallback realm",
                        ))?;
                        return Ok(NativeConversion::Value(realm));
                    }
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "not a function",
                    )));
                }
            }
        }
    }

    fn proxy_snapshot_if_any(&self, object: &ObjectRef) -> Result<Option<ProxyData>, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let state = self.0.state.borrow();
        let object_data = state.heap.object(object.object_id())?;
        match object_data.payload {
            ObjectPayload::Proxy(data) => Ok(Some(data)),
            _ => Ok(None),
        }
    }

    pub(in crate::runtime) fn is_proxy_object(
        &self,
        object: &ObjectRef,
    ) -> Result<bool, RuntimeError> {
        self.proxy_snapshot_if_any(object)
            .map(|value| value.is_some())
    }

    /// Pinned QuickJS `JS_IsArray`.
    ///
    /// Array branding crosses every Proxy layer without invoking a handler
    /// trap. A revoked Proxy still fails observably in the caller's realm.
    pub(in crate::runtime) fn internal_is_array(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(NativeConversion::Value(false));
        };
        let mut current = object.clone();
        let mut depth = 0_u32;
        loop {
            let Some(data) = self.proxy_snapshot_if_any(&current)? else {
                return self.is_array_object(&current).map(NativeConversion::Value);
            };
            // `js_resolve_proxy` checks the prior depth, then increments it.
            // This admits 1001 Proxy layers and fails on the 1002nd.
            if depth > 1000 {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "stack overflow",
                )?));
            }
            depth += 1;
            if data.is_revoked {
                return self.proxy_revoked_throw(realm);
            }
            current = ObjectRef::from_borrowed_handle(self.clone(), data.target)?;
        }
    }

    fn root_proxy_snapshot(
        &self,
        object: &ObjectRef,
        data: ProxyData,
    ) -> Result<RootedProxy, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Proxy"));
        }
        // Promote both borrowed heap identities before any observable work.
        // A trap may revoke the Proxy, trigger collection, or recursively
        // mutate either object.
        let target = ObjectRef::from_borrowed_handle(self.clone(), data.target)?;
        let handler = ObjectRef::from_borrowed_handle(self.clone(), data.handler)?;
        Ok(RootedProxy {
            proxy: object.clone(),
            data,
            target,
            handler,
        })
    }

    fn proxy_is_revoked(&self, proxy: &ObjectRef) -> Result<bool, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .proxy_snapshot(proxy.object_id())?
            .is_revoked)
    }

    fn proxy_revoked_throw<T>(
        &self,
        realm: ContextId,
    ) -> Result<NativeConversion<T>, RuntimeError> {
        Ok(NativeConversion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "revoked proxy",
        )?))
    }

    fn proxy_invariant_throw<T>(
        &self,
        realm: ContextId,
        operation: &'static str,
    ) -> Result<NativeConversion<T>, RuntimeError> {
        Ok(NativeConversion::Throw(
            self.new_native_error_from_message(
                realm,
                NativeErrorKind::Type,
                NativeErrorMessage::from_utf8(&format!("proxy: inconsistent {operation}")),
            )?,
        ))
    }

    fn property_key_value(&self, key: &PropertyKey) -> Result<Value, RuntimeError> {
        let kind = {
            let state = self.0.state.borrow();
            state.atoms.property_key_kind(key.atom())?
        };
        match kind {
            PropertyKeyKind::String => Ok(Value::String(
                self.0.state.borrow().atoms.to_js_string(key.atom())?,
            )),
            PropertyKeyKind::Symbol => Ok(Value::Symbol(SymbolRef::from_borrowed_atom(
                self.clone(),
                key.atom(),
            )?)),
            PropertyKeyKind::Private => Err(RuntimeError::Invariant(
                "private key escaped into an ECMAScript internal method",
            )),
        }
    }

    /// Pinned QuickJS `get_proxy_method`.
    ///
    /// The snapshot and roots are established before handler property access.
    /// `null`, like `undefined`, selects the target fallback in this release.
    fn proxy_method(
        &self,
        realm: ContextId,
        proxy: &ObjectRef,
        name: &'static str,
    ) -> Result<NativeConversion<(RootedProxy, Option<DirectCallTarget>)>, RuntimeError> {
        if self.proxy_method_stack_would_overflow() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let _stack_guard = ProxyMethodStackGuard::enter(self);
        let key = self.intern_property_key(name)?;
        let mut current = proxy.clone();
        let chain_limit = self.proxy_method_chain_limit(name);
        let mut depth = 0_usize;
        loop {
            if chain_limit.is_some_and(|limit| depth == limit) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "stack overflow",
                )?));
            }
            let data = self
                .proxy_snapshot_if_any(&current)?
                .ok_or(RuntimeError::Invariant(
                    "Proxy method dispatch reached an ordinary object",
                ))?;
            if data.is_revoked {
                return self.proxy_revoked_throw(realm);
            }
            let rooted = self.root_proxy_snapshot(&current, data)?;
            let method = match self.internal_get(
                realm,
                &rooted.handler,
                &key,
                Value::Object(rooted.handler.clone()),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if matches!(method, Value::Undefined | Value::Null) {
                if self.proxy_snapshot_if_any(&rooted.target)?.is_some() {
                    current = rooted.target.clone();
                    depth = depth.saturating_add(1);
                    continue;
                }
                return Ok(NativeConversion::Value((rooted, None)));
            }
            let method = match self.direct_call_target_from_value(method) {
                Ok(method) => method,
                Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Type => {
                    return Ok(NativeConversion::Throw(self.new_native_error_from_error(
                        realm,
                        NativeErrorKind::Type,
                        &error,
                    )?));
                }
                Err(error) => return Err(error),
            };
            return Ok(NativeConversion::Value((rooted, Some(method))));
        }
    }

    fn call_proxy_trap(
        &self,
        realm: ContextId,
        rooted: &RootedProxy,
        trap: &DirectCallTarget,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let this_value = Value::Object(rooted.handler.clone());
        match trap {
            DirectCallTarget::Callable(trap) => {
                self.call_internal(realm, trap, this_value, arguments)
            }
            DirectCallTarget::NonCallableProxy(trap) => {
                self.call_proxy(realm, trap, this_value, arguments)
            }
        }
    }

    fn raw_extensible_bit(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .extensible)
    }

    pub(in crate::runtime) fn internal_get_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self
                .get_own_property(object, key)
                .map(NativeConversion::Value);
        };
        self.proxy_get_own_property(realm, object, key)
    }

    /// Completion-aware `HasOwnProperty`.
    ///
    /// QuickJS can answer presence for an ordinary lazy property from its
    /// shape without materializing the auto-init value. Proxy presence remains
    /// observable through `[[GetOwnProperty]]`.
    pub(in crate::runtime) fn internal_has_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if self.proxy_snapshot_if_any(object)?.is_none()
            && !self.is_module_namespace_object(object)?
        {
            return self
                .has_own_property(object, key)
                .map(NativeConversion::Value);
        }
        Ok(match self.internal_get_own_property(realm, object, key)? {
            NativeConversion::Value(value) => NativeConversion::Value(value.is_some()),
            NativeConversion::Throw(value) => NativeConversion::Throw(value),
        })
    }

    /// Recheck enumerability through `[[GetOwnProperty]]`.
    ///
    /// Unlike an own-key `ENUM_ONLY` snapshot, this materializes an ordinary
    /// auto-init descriptor. A Proxy observes the same operation through its
    /// `getOwnPropertyDescriptor` trap.
    pub(in crate::runtime) fn internal_own_property_is_enumerable(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        Ok(match self.internal_get_own_property(realm, object, key)? {
            NativeConversion::Value(Some(descriptor)) => {
                NativeConversion::Value(descriptor.enumerable())
            }
            NativeConversion::Value(None) => NativeConversion::Value(false),
            NativeConversion::Throw(value) => NativeConversion::Throw(value),
        })
    }

    /// Read the enumerable bit while building a QuickJS
    /// `JS_GPN_ENUM_ONLY`/`JS_GPN_SET_ENUM` own-key snapshot.
    ///
    /// Ordinary objects are filtered directly from their shape, so an
    /// auto-init slot must not be materialized. Proxy keys remain observable
    /// and therefore use the completion-aware `[[GetOwnProperty]]` path.
    pub(in crate::runtime) fn internal_snapshot_own_property_is_enumerable(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if self.proxy_snapshot_if_any(object)?.is_none()
            && !self.is_module_namespace_object(object)?
        {
            return self
                .own_property_is_enumerable(object, key)
                .map(NativeConversion::Value);
        }
        self.internal_own_property_is_enumerable(realm, object, key)
    }

    pub(in crate::runtime) fn internal_get_prototype_of(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<Option<ObjectRef>>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self.get_prototype_of(object).map(NativeConversion::Value);
        };
        let (rooted, method) = match self.proxy_method(realm, object, "getPrototypeOf")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_get_prototype_of(realm, &rooted.target);
        };
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone())],
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let result = match result {
            Value::Object(prototype) => Some(prototype),
            Value::Null => None,
            _ => {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "proxy: inconsistent prototype",
                )?));
            }
        };
        let extensible = match self.internal_is_extensible(realm, &rooted.target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !extensible {
            let target = match self.internal_get_prototype_of(realm, &rooted.target)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if target != result {
                return self.proxy_invariant_throw(realm, "prototype");
            }
        }
        Ok(NativeConversion::Value(result))
    }

    pub(in crate::runtime) fn internal_set_prototype_of(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        prototype: Option<&ObjectRef>,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self
                .set_prototype_of(object, prototype)
                .map(NativeConversion::Value);
        };
        let (rooted, method) = match self.proxy_method(realm, object, "setPrototypeOf")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_set_prototype_of(realm, &rooted.target, prototype);
        };
        let prototype_value = prototype.cloned().map_or(Value::Null, Value::Object);
        let accepted = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone()), prototype_value],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !accepted {
            return Ok(NativeConversion::Value(false));
        }
        let extensible = match self.internal_is_extensible(realm, &rooted.target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !extensible {
            let target = match self.internal_get_prototype_of(realm, &rooted.target)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if target.as_ref() != prototype {
                return self.proxy_invariant_throw(realm, "prototype");
            }
        }
        Ok(NativeConversion::Value(true))
    }

    pub(in crate::runtime) fn internal_is_extensible(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self.is_extensible(object).map(NativeConversion::Value);
        };
        let (rooted, method) = match self.proxy_method(realm, object, "isExtensible")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_is_extensible(realm, &rooted.target);
        };
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone())],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let target = match self.internal_is_extensible(realm, &rooted.target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if result != target {
            return self.proxy_invariant_throw(realm, "isExtensible");
        }
        Ok(NativeConversion::Value(result))
    }

    pub(in crate::runtime) fn internal_prevent_extensions(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            if self.typed_array_is_object(object)?
                && self.typed_array_prevent_extensions_is_rejected(object)?
            {
                return Ok(NativeConversion::Value(false));
            }
            self.prevent_extensions(object)?;
            return Ok(NativeConversion::Value(true));
        };
        let (rooted, method) = match self.proxy_method(realm, object, "preventExtensions")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_prevent_extensions(realm, &rooted.target);
        };
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone())],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if result {
            let target = match self.internal_is_extensible(realm, &rooted.target)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if target {
                return self.proxy_invariant_throw(realm, "preventExtensions");
            }
        }
        Ok(NativeConversion::Value(result))
    }

    pub(in crate::runtime) fn internal_has_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if self.proxy_snapshot_if_any(object)?.is_some() {
            return self.proxy_has_property(realm, object, key);
        }
        if self.typed_array_is_object(object)?
            && let Some(numeric) = self.typed_array_canonical_numeric_index(key)?
        {
            return Ok(NativeConversion::Value(match numeric {
                CanonicalNumericIndex::Valid(index) => self
                    .typed_array_get_index_descriptor(object, index)?
                    .is_some(),
                CanonicalNumericIndex::Invalid => false,
            }));
        }
        if self.has_own_property(object, key)? {
            return Ok(NativeConversion::Value(true));
        }
        let prototype = match self.internal_get_prototype_of(realm, object)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(prototype) = prototype else {
            return Ok(NativeConversion::Value(false));
        };
        self.internal_has_property(realm, &prototype, key)
    }

    fn proxy_has_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let (rooted, method) = match self.proxy_method(realm, object, "has")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_has_property(realm, &rooted.target, key);
        };
        let key_value = self.property_key_value(key)?;
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone()), key_value],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if result {
            return Ok(NativeConversion::Value(true));
        }
        let target = match self.internal_get_own_property(realm, &rooted.target, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if let Some(target) = target
            && (!target.configurable() || !self.raw_extensible_bit(&rooted.target)?)
        {
            return self.proxy_invariant_throw(realm, "has");
        }
        Ok(NativeConversion::Value(false))
    }

    pub(in crate::runtime) fn internal_get(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Completion, RuntimeError> {
        if self.proxy_snapshot_if_any(object)?.is_some() {
            return self.proxy_get(realm, object, key, receiver);
        }
        if self.typed_array_is_object(object)?
            && let Some(numeric) = self.typed_array_canonical_numeric_index(key)?
        {
            let value = match numeric {
                CanonicalNumericIndex::Valid(index) => self
                    .typed_array_read_index(object, index)?
                    .unwrap_or(Value::Undefined),
                CanonicalNumericIndex::Invalid => Value::Undefined,
            };
            return Ok(Completion::Return(value));
        }
        let own = match self.internal_get_own_property(realm, object, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Some(own) = own {
            return match own {
                CompleteOrdinaryPropertyDescriptor::Data { value, .. } => {
                    Ok(Completion::Return(value))
                }
                CompleteOrdinaryPropertyDescriptor::Accessor { get: None, .. } => {
                    Ok(Completion::Return(Value::Undefined))
                }
                CompleteOrdinaryPropertyDescriptor::Accessor {
                    get: Some(getter), ..
                } => self.call_internal(realm, &getter, receiver, &[]),
            };
        }
        let prototype = match self.internal_get_prototype_of(realm, object)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(prototype) = prototype else {
            return Ok(Completion::Return(Value::Undefined));
        };
        self.internal_get(realm, &prototype, key, receiver)
    }

    /// Completion-aware property read which preserves QuickJS's internal
    /// "missing" sentinel for ordinary prototype chains.
    ///
    /// A Proxy is deliberately a terminal observable boundary here. Even
    /// when its `get` trap is absent and the target lookup ultimately returns
    /// `undefined`, QuickJS treats the Proxy lookup as a completed Get rather
    /// than recovering the ordinary-chain missing sentinel. Global binding
    /// reads depend on that distinction to choose between `undefined` and a
    /// ReferenceError.
    pub(in crate::runtime) fn internal_get_or_missing(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        if self.proxy_snapshot_if_any(object)?.is_some() {
            return Ok(match self.internal_get(realm, object, key, receiver)? {
                Completion::Return(value) => NativeConversion::Value(Some(value)),
                Completion::Throw(value) => NativeConversion::Throw(value),
            });
        }
        if self.typed_array_is_object(object)?
            && let Some(numeric) = self.typed_array_canonical_numeric_index(key)?
        {
            let value = match numeric {
                CanonicalNumericIndex::Valid(index) => self
                    .typed_array_read_index(object, index)?
                    .unwrap_or(Value::Undefined),
                CanonicalNumericIndex::Invalid => Value::Undefined,
            };
            return Ok(NativeConversion::Value(Some(value)));
        }
        let own = match self.internal_get_own_property(realm, object, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if let Some(own) = own {
            return match own {
                CompleteOrdinaryPropertyDescriptor::Data { value, .. } => {
                    Ok(NativeConversion::Value(Some(value)))
                }
                CompleteOrdinaryPropertyDescriptor::Accessor { get: None, .. } => {
                    Ok(NativeConversion::Value(Some(Value::Undefined)))
                }
                CompleteOrdinaryPropertyDescriptor::Accessor {
                    get: Some(getter), ..
                } => Ok(match self.call_internal(realm, &getter, receiver, &[])? {
                    Completion::Return(value) => NativeConversion::Value(Some(value)),
                    Completion::Throw(value) => NativeConversion::Throw(value),
                }),
            };
        }
        let prototype = match self.internal_get_prototype_of(realm, object)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(prototype) = prototype else {
            return Ok(NativeConversion::Value(None));
        };
        self.internal_get_or_missing(realm, &prototype, key, receiver)
    }

    fn proxy_get(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Completion, RuntimeError> {
        let (rooted, method) = match self.proxy_method(realm, object, "get")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_get(realm, &rooted.target, key, receiver);
        };
        let key_value = self.property_key_value(key)?;
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone()), key_value, receiver],
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let target = match self.internal_get_own_property(realm, &rooted.target, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if let Some(target) = target {
            match target {
                CompleteOrdinaryPropertyDescriptor::Data {
                    value,
                    writable: false,
                    configurable: false,
                    ..
                } if !result.same_value(&value) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "proxy: inconsistent get",
                    )?));
                }
                CompleteOrdinaryPropertyDescriptor::Accessor {
                    get: None,
                    configurable: false,
                    ..
                } if !matches!(result, Value::Undefined) => {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "proxy: inconsistent get",
                    )?));
                }
                _ => {}
            }
        }
        Ok(Completion::Return(result))
    }

    fn ordinary_set_fast_path_available(
        &self,
        object: &ObjectRef,
        receiver: &Value,
    ) -> Result<bool, RuntimeError> {
        if let Value::Object(receiver) = receiver
            && (self.proxy_snapshot_if_any(receiver)?.is_some()
                || self.typed_array_is_object(receiver)?)
        {
            return Ok(false);
        }
        let mut current = Some(object.clone());
        while let Some(object) = current {
            if self.proxy_snapshot_if_any(&object)?.is_some()
                || self.typed_array_is_object(&object)?
            {
                return Ok(false);
            }
            current = self.get_prototype_of(&object)?;
        }
        Ok(true)
    }

    pub(in crate::runtime) fn internal_set(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<NativeConversion<InternalSetResult>, RuntimeError> {
        if self.proxy_snapshot_if_any(object)?.is_some() {
            return self.proxy_set(realm, object, key, value, receiver);
        }
        if self.is_module_namespace_object(object)? {
            return Ok(NativeConversion::Value(InternalSetResult::Rejected(
                PropertySetRejection::ReadOnly,
            )));
        }
        if self.typed_array_is_object(object)?
            && let Some(numeric) = self.typed_array_canonical_numeric_index(key)?
        {
            let receiver_is_target =
                matches!(&receiver, Value::Object(receiver) if receiver == object);
            match numeric {
                CanonicalNumericIndex::Valid(index) if receiver_is_target => {
                    return Ok(
                        match self.typed_array_set_index(realm, object, index, &value)? {
                            NativeConversion::Value(()) => {
                                NativeConversion::Value(InternalSetResult::Accepted)
                            }
                            NativeConversion::Throw(value) => NativeConversion::Throw(value),
                        },
                    );
                }
                CanonicalNumericIndex::Valid(index) => {
                    if self
                        .typed_array_get_index_descriptor(object, index)?
                        .is_none()
                    {
                        return Ok(NativeConversion::Value(InternalSetResult::Accepted));
                    }
                }
                CanonicalNumericIndex::Invalid if receiver_is_target => {
                    let element = self.typed_array_snapshot(object)?.element;
                    return Ok(
                        match self.typed_array_convert_element(realm, element, &value)? {
                            NativeConversion::Value(_) => {
                                NativeConversion::Value(InternalSetResult::Accepted)
                            }
                            NativeConversion::Throw(value) => NativeConversion::Throw(value),
                        },
                    );
                }
                CanonicalNumericIndex::Invalid => {
                    return Ok(NativeConversion::Value(InternalSetResult::Accepted));
                }
            }
        }
        if self.ordinary_set_fast_path_available(object, &receiver)? {
            return match self.prepare_set_property_with_receiver_in_realm(
                Some(realm),
                object,
                key,
                value,
                receiver,
            )? {
                PropertySetAction::Complete => {
                    Ok(NativeConversion::Value(InternalSetResult::Accepted))
                }
                PropertySetAction::Throw(value) => Ok(NativeConversion::Throw(value)),
                PropertySetAction::Rejected(rejection) => Ok(NativeConversion::Value(
                    InternalSetResult::Rejected(rejection),
                )),
                PropertySetAction::Call {
                    setter,
                    receiver,
                    argument,
                } => match self.call_internal(realm, &setter, receiver, &[argument])? {
                    Completion::Return(_) => {
                        Ok(NativeConversion::Value(InternalSetResult::Accepted))
                    }
                    Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
                },
            };
        }

        let own = match self.internal_get_own_property(realm, object, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let own = if let Some(own) = own {
            own
        } else {
            let prototype = match self.internal_get_prototype_of(realm, object)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if let Some(prototype) = prototype {
                return self.internal_set(realm, &prototype, key, value, receiver);
            }
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Undefined,
                writable: true,
                enumerable: true,
                configurable: true,
            }
        };

        match own {
            CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            } => Ok(NativeConversion::Value(InternalSetResult::Rejected(
                PropertySetRejection::ReadOnly,
            ))),
            CompleteOrdinaryPropertyDescriptor::Data { .. } => {
                let Value::Object(receiver) = receiver else {
                    return Ok(NativeConversion::Value(InternalSetResult::Rejected(
                        PropertySetRejection::NotObject,
                    )));
                };
                let existing = match self.internal_get_own_property(realm, &receiver, key)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
                let descriptor = match existing {
                    Some(CompleteOrdinaryPropertyDescriptor::Accessor { set: None, .. }) => {
                        return Ok(NativeConversion::Value(InternalSetResult::Rejected(
                            PropertySetRejection::NoSetter,
                        )));
                    }
                    Some(CompleteOrdinaryPropertyDescriptor::Accessor { set: Some(_), .. }) => {
                        return Ok(NativeConversion::Value(InternalSetResult::Rejected(
                            PropertySetRejection::ReadOnly,
                        )));
                    }
                    Some(CompleteOrdinaryPropertyDescriptor::Data {
                        writable: false, ..
                    }) => {
                        return Ok(NativeConversion::Value(InternalSetResult::Rejected(
                            PropertySetRejection::ReadOnly,
                        )));
                    }
                    Some(CompleteOrdinaryPropertyDescriptor::Data { .. }) => {
                        OrdinaryPropertyDescriptor {
                            value: DescriptorField::Present(value),
                            ..OrdinaryPropertyDescriptor::new()
                        }
                    }
                    None => OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(value),
                        writable: DescriptorField::Present(true),
                        enumerable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                };
                match self.internal_define_own_property(realm, &receiver, key, &descriptor)? {
                    NativeConversion::Value(InternalDefineResult::Defined) => {
                        Ok(NativeConversion::Value(InternalSetResult::Accepted))
                    }
                    NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => Ok(
                        NativeConversion::Value(InternalSetResult::RejectedProxyTrap),
                    ),
                    NativeConversion::Value(InternalDefineResult::RejectedOrdinary(object)) => {
                        let rejection = if !self.has_own_property(&object, key)?
                            && !self.is_extensible(&object)?
                        {
                            PropertySetRejection::NotExtensible
                        } else if matches!(self.array_own_key(&object, key)?, ArrayOwnKey::Index(_))
                            && !self.array_length_state(&object)?.1
                        {
                            PropertySetRejection::ArrayLengthReadOnly
                        } else {
                            PropertySetRejection::ReadOnly
                        };
                        Ok(NativeConversion::Value(InternalSetResult::Rejected(
                            rejection,
                        )))
                    }
                    NativeConversion::Throw(value) => Ok(NativeConversion::Throw(value)),
                }
            }
            CompleteOrdinaryPropertyDescriptor::Accessor { set: None, .. } => {
                Ok(NativeConversion::Value(InternalSetResult::Rejected(
                    PropertySetRejection::NoSetter,
                )))
            }
            CompleteOrdinaryPropertyDescriptor::Accessor {
                set: Some(setter), ..
            } => match self.call_internal(realm, &setter, receiver, &[value])? {
                Completion::Return(_) => Ok(NativeConversion::Value(InternalSetResult::Accepted)),
                Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
            },
        }
    }

    fn proxy_set(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<NativeConversion<InternalSetResult>, RuntimeError> {
        let (rooted, method) = match self.proxy_method(realm, object, "set")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_set(realm, &rooted.target, key, value, receiver);
        };
        let key_value = self.property_key_value(key)?;
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[
                Value::Object(rooted.target.clone()),
                key_value,
                value.clone(),
                receiver,
            ],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !result {
            return Ok(NativeConversion::Value(
                InternalSetResult::RejectedProxyTrap,
            ));
        }
        let target = match self.internal_get_own_property(realm, &rooted.target, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if let Some(target) = target {
            match target {
                CompleteOrdinaryPropertyDescriptor::Data {
                    value: target_value,
                    writable: false,
                    configurable: false,
                    ..
                } if !value.same_value(&target_value) => {
                    return self.proxy_invariant_throw(realm, "set");
                }
                CompleteOrdinaryPropertyDescriptor::Accessor {
                    set: None,
                    configurable: false,
                    ..
                } => return self.proxy_invariant_throw(realm, "set"),
                _ => {}
            }
        }
        Ok(NativeConversion::Value(InternalSetResult::Accepted))
    }

    fn proxy_descriptor_object(
        &self,
        realm: ContextId,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<ObjectRef, RuntimeError> {
        let object = self.new_ordinary_object_in_realm(realm)?;
        let mut fields = Vec::with_capacity(6);
        if let DescriptorField::Present(value) = &descriptor.get {
            fields.push((
                "get",
                match value {
                    AccessorValue::Undefined => Value::Undefined,
                    AccessorValue::Callable(callable) => {
                        Value::Object(callable.as_object().clone())
                    }
                },
            ));
        }
        if let DescriptorField::Present(value) = &descriptor.set {
            fields.push((
                "set",
                match value {
                    AccessorValue::Undefined => Value::Undefined,
                    AccessorValue::Callable(callable) => {
                        Value::Object(callable.as_object().clone())
                    }
                },
            ));
        }
        if let DescriptorField::Present(value) = &descriptor.value {
            fields.push(("value", value.clone()));
        }
        if let DescriptorField::Present(value) = descriptor.writable {
            fields.push(("writable", Value::Bool(value)));
        }
        if let DescriptorField::Present(value) = descriptor.enumerable {
            fields.push(("enumerable", Value::Bool(value)));
        }
        if let DescriptorField::Present(value) = descriptor.configurable {
            fields.push(("configurable", Value::Bool(value)));
        }
        for (name, value) in fields {
            let key = self.intern_property_key(name)?;
            let accepted = self.define_own_property(
                &object,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !accepted {
                return Err(RuntimeError::Invariant(
                    "fresh Proxy descriptor object rejected a field",
                ));
            }
        }
        Ok(object)
    }

    fn complete_proxy_descriptor(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        let descriptor = match self.native_to_property_descriptor(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let descriptor = descriptor_to_validation_record(&descriptor);
        let complete = validate_and_apply_property_descriptor(
            true,
            &descriptor,
            None,
            &Value::Undefined,
            Value::same_value,
        )
        .map_err(|_| {
            RuntimeError::Invariant("validated Proxy descriptor could not be completed")
        })?;
        Ok(NativeConversion::Value(validation_record_to_complete(
            complete,
        )?))
    }

    fn proxy_get_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>, RuntimeError> {
        let (rooted, method) = match self.proxy_method(realm, object, "getOwnPropertyDescriptor")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_get_own_property(realm, &rooted.target, key);
        };
        let key_value = self.property_key_value(key)?;
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone()), key_value],
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !matches!(result, Value::Undefined | Value::Object(_)) {
            return self.proxy_invariant_throw(realm, "getOwnPropertyDescriptor");
        }

        let target_descriptor = match self.internal_get_own_property(realm, &rooted.target, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(result, Value::Undefined) {
            if let Some(target) = target_descriptor
                && (!target.configurable() || !self.raw_extensible_bit(&rooted.target)?)
            {
                return self.proxy_invariant_throw(realm, "getOwnPropertyDescriptor");
            }
            return Ok(NativeConversion::Value(None));
        }

        // Pinned QuickJS checks target extensibility before converting the
        // result object to a descriptor.
        let extensible = match self.internal_is_extensible(realm, &rooted.target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let result = match self.complete_proxy_descriptor(realm, result)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !proxy_gopd_descriptor_is_compatible(target_descriptor.as_ref(), &result, extensible) {
            return self.proxy_invariant_throw(realm, "getOwnPropertyDescriptor");
        }
        Ok(NativeConversion::Value(Some(result)))
    }

    pub(in crate::runtime) fn internal_define_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<NativeConversion<InternalDefineResult>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return Ok(
                match self.define_own_property_in_realm(Some(realm), object, key, descriptor)? {
                    PropertyDefineOutcome::Defined(true) => {
                        NativeConversion::Value(InternalDefineResult::Defined)
                    }
                    PropertyDefineOutcome::Defined(false) => NativeConversion::Value(
                        InternalDefineResult::RejectedOrdinary(object.clone()),
                    ),
                    PropertyDefineOutcome::Throw(value) => NativeConversion::Throw(value),
                },
            );
        };
        self.proxy_define_own_property(realm, object, key, descriptor)
    }

    fn proxy_define_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<NativeConversion<InternalDefineResult>, RuntimeError> {
        let (rooted, method) = match self.proxy_method(realm, object, "defineProperty")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_define_own_property(realm, &rooted.target, key, descriptor);
        };
        let key_value = self.property_key_value(key)?;
        let descriptor_object = self.proxy_descriptor_object(realm, descriptor)?;
        let accepted = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[
                Value::Object(rooted.target.clone()),
                key_value,
                Value::Object(descriptor_object),
            ],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !accepted {
            return Ok(NativeConversion::Value(
                InternalDefineResult::RejectedProxyTrap,
            ));
        }

        let target_descriptor = match self.internal_get_own_property(realm, &rooted.target, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let setting_not_configurable =
            matches!(descriptor.configurable, DescriptorField::Present(false));
        let compatible = if let Some(target) = target_descriptor.as_ref() {
            proxy_define_descriptor_is_compatible(target, descriptor)
        } else {
            self.raw_extensible_bit(&rooted.target)? && !setting_not_configurable
        };
        if !compatible {
            return self.proxy_invariant_throw(realm, "defineProperty");
        }
        Ok(NativeConversion::Value(InternalDefineResult::Defined))
    }

    pub(in crate::runtime) fn internal_delete_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self
                .delete_property(object, key)
                .map(NativeConversion::Value);
        };
        let (rooted, method) = match self.proxy_method(realm, object, "deleteProperty")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_delete_property(realm, &rooted.target, key);
        };
        let key_value = self.property_key_value(key)?;
        let accepted = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone()), key_value],
        )? {
            Completion::Return(value) => self.value_to_boolean(&value)?,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if !accepted {
            return Ok(NativeConversion::Value(false));
        }
        let target = match self.internal_get_own_property(realm, &rooted.target, key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if let Some(target) = target {
            if !target.configurable() {
                return self.proxy_invariant_throw(realm, "deleteProperty");
            }
            let extensible = match self.internal_is_extensible(realm, &rooted.target)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            if !extensible {
                return self.proxy_invariant_throw(realm, "deleteProperty");
            }
        }
        Ok(NativeConversion::Value(true))
    }

    pub(in crate::runtime) fn internal_own_property_keys(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<Vec<PropertyKey>>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self.own_property_keys(object).map(NativeConversion::Value);
        };
        let (rooted, method) = match self.proxy_method(realm, object, "ownKeys")? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let Some(method) = method else {
            return self.internal_own_property_keys(realm, &rooted.target);
        };
        let result = match self.call_proxy_trap(
            realm,
            &rooted,
            &method,
            &[Value::Object(rooted.target.clone())],
        )? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let ProxyOwnKeys {
            keys,
            mut key_atoms,
        } = match self.proxy_own_keys_list(realm, result)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let extensible = match self.internal_is_extensible(realm, &rooted.target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if self.proxy_is_revoked(&rooted.proxy)? {
            return self.proxy_revoked_throw(realm);
        }
        let target_keys = match self.internal_own_property_keys(realm, &rooted.target)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        for target_key in target_keys {
            if self.proxy_is_revoked(&rooted.proxy)? {
                return self.proxy_revoked_throw(realm);
            }
            let descriptor =
                match self.internal_get_own_property(realm, &rooted.target, &target_key)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
                };
            let Some(descriptor) = descriptor else {
                continue;
            };
            if !extensible {
                if !key_atoms.remove(&target_key.atom()) {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "proxy: target property must be present in proxy ownKeys",
                    )?));
                }
            } else if !descriptor.configurable() && !key_atoms.contains(&target_key.atom()) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "proxy: target property must be present in proxy ownKeys",
                )?));
            }
        }
        if !extensible && !key_atoms.is_empty() {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "proxy: property not present in target were returned by non extensible proxy",
            )?));
        }
        Ok(NativeConversion::Value(keys))
    }

    fn proxy_own_keys_list(
        &self,
        realm: ContextId,
        value: Value,
    ) -> Result<NativeConversion<ProxyOwnKeys>, RuntimeError> {
        let length_key = self.intern_property_key("length")?;
        let length = match self.get_value_property_in_realm(realm, value.clone(), &length_key)? {
            Completion::Return(value) => {
                let number = match self.native_to_number(realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(NativeConversion::Throw(value));
                    }
                };
                Self::to_uint32_number(number)
            }
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let capacity = usize::try_from(length)
            .map_err(|_| RuntimeError::Invariant("Proxy ownKeys length does not fit usize"))?;
        let mut keys = Vec::with_capacity(capacity);
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let value = match self.get_value_property_in_realm(realm, value.clone(), &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
            let key = match value {
                Value::String(value) => self.intern_property_key_js_string(&value)?,
                Value::Symbol(value) => PropertyKey::from(value),
                _ => {
                    return Ok(NativeConversion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "proxy: properties must be strings or symbols",
                    )?));
                }
            };
            keys.push(key);
        }
        let mut key_atoms = HashSet::with_capacity(keys.len());
        for key in &keys {
            if !key_atoms.insert(key.atom()) {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "proxy: duplicate property",
                )?));
            }
        }
        Ok(NativeConversion::Value(ProxyOwnKeys { keys, key_atoms }))
    }

    pub(in crate::runtime) fn call_proxy(
        &self,
        realm: ContextId,
        proxy: &ObjectRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        if self.proxy_method_stack_would_overflow() {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let _stack_guard = ProxyMethodStackGuard::enter(self);
        let key = self.intern_property_key("apply")?;
        let chain_limit = self.proxy_method_chain_limit("apply");
        let mut current = proxy.clone();
        let mut depth = 0_usize;

        loop {
            if chain_limit.is_some_and(|limit| depth == limit) {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "stack overflow",
                )?));
            }
            let data = self
                .proxy_snapshot_if_any(&current)?
                .ok_or(RuntimeError::Invariant(
                    "Proxy call dispatch reached an ordinary object",
                ))?;
            if data.is_revoked {
                return match self.proxy_revoked_throw(realm)? {
                    NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
                    NativeConversion::Value(()) => Err(RuntimeError::Invariant(
                        "revoked Proxy call returned a value",
                    )),
                };
            }
            let rooted = self.root_proxy_snapshot(&current, data)?;
            let method = match self.internal_get(
                realm,
                &rooted.handler,
                &key,
                Value::Object(rooted.handler.clone()),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            // Pinned js_proxy_call checks this layer's cached [[Call]] bit
            // after the observable trap Get and before a missing-trap
            // fallback reaches another Proxy layer.
            if !rooted.data.is_callable {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not a function",
                )?));
            }
            if matches!(method, Value::Undefined | Value::Null) {
                if self.is_proxy_object(&rooted.target)? {
                    current = rooted.target.clone();
                    depth = depth.saturating_add(1);
                    continue;
                }
                return self.call_value_internal(
                    realm,
                    Value::Object(rooted.target.clone()),
                    this_value,
                    arguments,
                );
            }

            // Upstream builds the trap argv Array before JS_Call validates
            // the method's callability.
            let argument_array = self.new_array_from_values(realm, arguments.to_vec())?;
            let method = match self.direct_call_target_from_value(method) {
                Ok(method) => method,
                Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Type => {
                    return Ok(Completion::Throw(self.new_native_error_from_error(
                        realm,
                        NativeErrorKind::Type,
                        &error,
                    )?));
                }
                Err(error) => return Err(error),
            };
            return self.call_proxy_trap(
                realm,
                &rooted,
                &method,
                &[
                    Value::Object(rooted.target.clone()),
                    this_value,
                    Value::Object(argument_array),
                ],
            );
        }
    }

    pub(in crate::runtime) fn construct_proxy(
        &self,
        realm: ContextId,
        proxy: &ConstructorRef,
        new_target: ConstructNewTarget,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        if self.proxy_method_stack_would_overflow() {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }
        let _stack_guard = ProxyMethodStackGuard::enter(self);
        let key = self.intern_property_key("construct")?;
        let chain_limit = self.proxy_method_chain_limit("construct");
        let mut current = proxy.clone();
        let mut depth = 0_usize;

        loop {
            if chain_limit.is_some_and(|limit| depth == limit) {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    "stack overflow",
                )?));
            }
            let data =
                self.proxy_snapshot_if_any(current.as_object())?
                    .ok_or(RuntimeError::Invariant(
                        "Proxy construct dispatch reached an ordinary object",
                    ))?;
            if data.is_revoked {
                return match self.proxy_revoked_throw(realm)? {
                    NativeConversion::Throw(value) => Ok(Completion::Throw(value)),
                    NativeConversion::Value(()) => Err(RuntimeError::Invariant(
                        "revoked Proxy construct returned a value",
                    )),
                };
            }
            let rooted = self.root_proxy_snapshot(current.as_object(), data)?;
            let method = match self.internal_get(
                realm,
                &rooted.handler,
                &key,
                Value::Object(rooted.handler.clone()),
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };

            // Pinned js_proxy_call_constructor checks this layer's immediate
            // target after the observable trap Get and before a missing-trap
            // fallback reaches another Proxy layer.
            let target =
                match self.constructor_from_value(realm, Value::Object(rooted.target.clone()))? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            if matches!(method, Value::Undefined | Value::Null) {
                if self.is_proxy_object(target.as_object())? {
                    current = target;
                    depth = depth.saturating_add(1);
                    continue;
                }
                return self
                    .construct_internal_with_new_target(realm, &target, new_target, arguments);
            }

            // Upstream allocates the trap argv Array before JS_Call validates
            // trap callability, so keep the raw method until this point.
            let argument_array = self.new_array_from_values(realm, arguments.to_vec())?;
            let method = match self.direct_call_target_from_value(method) {
                Ok(method) => method,
                Err(RuntimeError::Engine(error)) if error.kind() == ErrorKind::Type => {
                    return Ok(Completion::Throw(self.new_native_error_from_error(
                        realm,
                        NativeErrorKind::Type,
                        &error,
                    )?));
                }
                Err(error) => return Err(error),
            };
            let result = self.call_proxy_trap(
                realm,
                &rooted,
                &method,
                &[
                    Value::Object(rooted.target.clone()),
                    Value::Object(argument_array),
                    new_target.value(),
                ],
            )?;
            return match result {
                Completion::Return(value @ Value::Object(_)) => Ok(Completion::Return(value)),
                Completion::Return(_) => Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?)),
                Completion::Throw(value) => Ok(Completion::Throw(value)),
            };
        }
    }
}

fn proxy_gopd_descriptor_is_compatible(
    target: Option<&CompleteOrdinaryPropertyDescriptor>,
    result: &CompleteOrdinaryPropertyDescriptor,
    extensible: bool,
) -> bool {
    let Some(target) = target else {
        return extensible && result.configurable();
    };

    if !target.configurable() {
        if result.configurable()
            || target.enumerable() != result.enumerable()
            || complete_descriptor_is_data(target) != complete_descriptor_is_data(result)
        {
            return false;
        }
        if let (
            CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            },
            CompleteOrdinaryPropertyDescriptor::Data { writable: true, .. },
        ) = (target, result)
        {
            return false;
        }
    }

    // QuickJS's explicit proxy-missing-checks additions forbid reporting a
    // newly non-configurable property or freezing a target's writable data
    // property.  Deliberately do not compare value/get/set here: the pinned
    // release documents that gOPD compatibility omission.
    if !result.configurable() {
        if target.configurable() {
            return false;
        }
        if let (
            CompleteOrdinaryPropertyDescriptor::Data { writable: true, .. },
            CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            },
        ) = (target, result)
        {
            return false;
        }
    }
    true
}

fn proxy_define_descriptor_is_compatible(
    target: &CompleteOrdinaryPropertyDescriptor,
    descriptor: &OrdinaryPropertyDescriptor,
) -> bool {
    let descriptor_record = descriptor_to_validation_record(descriptor);
    let target_record = complete_to_validation_record(target);
    if validate_and_apply_property_descriptor(
        true,
        &descriptor_record,
        Some(&target_record),
        &Value::Undefined,
        Value::same_value,
    )
    .is_err()
    {
        return false;
    }
    let setting_not_configurable =
        matches!(descriptor.configurable, DescriptorField::Present(false));
    if target.configurable() && setting_not_configurable {
        return false;
    }
    if let CompleteOrdinaryPropertyDescriptor::Data { writable: true, .. } = target
        && matches!(descriptor.writable, DescriptorField::Present(false))
        && !target.configurable()
    {
        return false;
    }
    true
}

const fn complete_descriptor_is_data(descriptor: &CompleteOrdinaryPropertyDescriptor) -> bool {
    matches!(descriptor, CompleteOrdinaryPropertyDescriptor::Data { .. })
}
