//! Completion-aware ECMAScript internal-method dispatch.
//!
//! Physical ordinary/Array/Arguments/String storage remains in
//! [`super::properties`].  This module is the observable semantic boundary:
//! every operation first selects an exotic implementation and otherwise
//! delegates to the existing ordinary kernel.  Keeping JavaScript calls here,
//! above the heap, ensures no `RefCell` borrow survives a re-entrant Proxy
//! trap.

use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind, NativeErrorMessage};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::object::ordinary_storage::SpecialKind;

use crate::engine::atom::PropertyKeyKind;
use crate::engine::builtins::CanonicalNumericIndex;
use crate::engine::heap::{ContextId, ObjectPayload, ProxyData};
use crate::engine::object::operations::{
    InternalDefineResult, InternalSetResult, PropertyDefineOutcome, PropertySetAction,
    PropertySetRejection, complete_to_validation_record, descriptor_to_validation_record,
};
use crate::engine::object::property::validate_and_apply_property_descriptor;
use crate::engine::object::{
    AccessorValue, CallableRef, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, SymbolRef,
};
use crate::engine::value::Value;
use crate::engine::value::conversion::NativeConversion;
use crate::engine::vm::Completion;

use crate::engine::vm::call::{ConstructNewTarget, ConstructorRef, DirectCallTarget};

mod boolean;
mod prototype;
mod reuse;

pub(crate) use prototype::ProxyPrototypeResume;
pub(crate) use prototype::{ProxyPrototypeKind, ProxyPrototypeStep};
mod define;
mod set;

pub(crate) use define::ProxyDefineResume;
pub(crate) use define::ProxyDefineStep;

pub(crate) use set::ProxySetResume;
pub(crate) use set::ProxySetStep;
mod call;
mod construct;

pub(crate) use call::ProxyCallResume;
pub(crate) use call::ProxyCallStep;

pub(crate) use construct::{ProxyConstructResume, ProxyConstructStep};
mod get;
mod method;
mod own_keys;
mod own_property;

pub(crate) use boolean::ProxyBooleanResume;
pub(crate) use boolean::{ProxyBooleanKind, ProxyBooleanStep};

pub(crate) use get::ProxyGetResume;
use get::ProxyGetStep;

pub(crate) use get::ProxyGetStep as OwnedProxyGetStep;

pub(crate) use own_keys::{KeysResume, KeysStep};

pub(crate) use own_property::ProxyOwnResume;
pub(crate) use own_property::ProxyOwnStep;

#[derive(Clone)]
struct RootedProxy {
    proxy: ObjectRef,
    data: ProxyData,
    target: ObjectRef,
    handler: ObjectRef,
}

pub(crate) enum PreparedHas {
    Complete(bool),
    Proxy(ObjectRef),
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

// Selection preserves IntegerIndexedElementSet's conversion/fallthrough split.
// A different valid receiver needs its original descriptor path; ignored
// canonical indices do not convert the supplied value.
enum TypedSetSelection {
    Decline,
    Ignore,
    Element(Option<u64>),
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

    pub(crate) fn direct_call_target_from_value(
        &self,
        value: Value,
    ) -> Result<DirectCallTarget, RuntimeError> {
        let Value::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        self.direct_call_target_from_object(object)
    }

    /// Handle form of [`Runtime::direct_call_target_from_value`]; borrows the
    /// object's edge and roots the classified capability only.
    pub(crate) fn direct_call_target_from_jsvalue(
        &self,
        value: crate::engine::value::JsValue,
    ) -> Result<DirectCallTarget, RuntimeError> {
        let crate::engine::value::JsValue::Object(object) = value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not a function",
            )));
        };
        self.direct_call_target_from_object(object)
    }

    fn direct_call_target_from_object(
        &self,
        object: crate::engine::object::ObjectRef,
    ) -> Result<DirectCallTarget, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("call target"));
        }
        let object = match self.try_into_callable(object)? {
            Ok(callable) => return Ok(DirectCallTarget::Callable(callable)),
            Err(object) => object,
        };
        if self.is_proxy_object(&object)? {
            return Ok(DirectCallTarget::NonCallableProxy(object));
        }
        Err(RuntimeError::Engine(Error::new(
            ErrorKind::Type,
            "not a function",
        )))
    }

    /// Raw realm lookup retained for internal callers which have already
    /// excluded revoked Proxy wrappers.
    #[cfg(test)]
    pub(crate) fn callable_realm(&self, callable: &CallableRef) -> Result<ContextId, RuntimeError> {
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
    pub(crate) fn function_realm(
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
    pub(crate) fn function_realm_from_value(
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

    pub(crate) fn is_proxy_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        self.proxy_snapshot_if_any(object)
            .map(|value| value.is_some())
    }

    /// Pinned QuickJS `JS_IsArray`.
    ///
    /// Array branding crosses every Proxy layer without invoking a handler
    /// trap. A revoked Proxy still fails observably in the caller's realm.
    pub(crate) fn internal_is_array(
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

    fn raw_extensible_bit(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .extensible)
    }

    pub(crate) fn internal_get_own_property(
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
    pub(crate) fn internal_has_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if let Some(flags) = self.ordinary_property_flags(object, key)? {
            return Ok(NativeConversion::Value(flags.is_some()));
        }
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
    pub(crate) fn internal_own_property_is_enumerable(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if let Some(own) = self.ordinary_property_flags(object, key)? {
            match own {
                None => return Ok(NativeConversion::Value(false)),
                Some(own) if !own.needs_materialization => {
                    return Ok(NativeConversion::Value(own.flags.enumerable));
                }
                Some(_) => {}
            }
        }
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
    pub(crate) fn internal_snapshot_own_property_is_enumerable(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if let Some(flags) = self.ordinary_property_flags(object, key)? {
            return Ok(NativeConversion::Value(
                flags.is_some_and(|own| own.flags.enumerable),
            ));
        }
        if self.proxy_snapshot_if_any(object)?.is_none()
            && !self.is_module_namespace_object(object)?
        {
            return self
                .own_property_is_enumerable(object, key)
                .map(NativeConversion::Value);
        }
        self.internal_own_property_is_enumerable(realm, object, key)
    }

    pub(crate) fn internal_get_prototype_of(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<Option<ObjectRef>>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self.get_prototype_of(object).map(NativeConversion::Value);
        };
        match prototype::finish(
            self,
            realm,
            ProxyPrototypeStep::start(self, realm, object.clone(), ProxyPrototypeKind::Get)?,
        )? {
            Completion::Return(Value::Object(object)) => Ok(NativeConversion::Value(Some(object))),
            Completion::Return(Value::Null) => Ok(NativeConversion::Value(None)),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
            _ => Err(RuntimeError::Invariant(
                "GetPrototypeOf completed with an invalid value",
            )),
        }
    }

    pub(crate) fn internal_set_prototype_of(
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
        match prototype::finish(
            self,
            realm,
            ProxyPrototypeStep::start(
                self,
                realm,
                object.clone(),
                ProxyPrototypeKind::Set(prototype.cloned()),
            )?,
        )? {
            Completion::Return(Value::Bool(value)) => Ok(NativeConversion::Value(value)),
            Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
            _ => Err(RuntimeError::Invariant(
                "SetPrototypeOf completed with an invalid value",
            )),
        }
    }

    pub(crate) fn internal_is_extensible(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self.is_extensible(object).map(NativeConversion::Value);
        };
        boolean::finish(
            self,
            realm,
            ProxyBooleanStep::start(self, realm, object.clone(), ProxyBooleanKind::Extensible)?,
        )
    }

    pub(crate) fn internal_prevent_extensions(
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
        boolean::finish(
            self,
            realm,
            ProxyBooleanStep::start(
                self,
                realm,
                object.clone(),
                ProxyBooleanKind::PreventExtensions,
            )?,
        )
    }

    pub(crate) fn internal_has_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        self.finish_prepared_has(realm, key, self.prepare_has_property(object, key)?)
    }

    /// Finish the selected Proxy boundary without replaying a traversed prefix.
    pub(crate) fn finish_prepared_has(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        probe: PreparedHas,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        match probe {
            PreparedHas::Complete(value) => Ok(NativeConversion::Value(value)),
            PreparedHas::Proxy(object) => self.proxy_has_property(realm, &object, key),
        }
    }

    /// Probe existing storage without executing a Proxy trap. A Proxy on the
    /// prototype chain is returned with the already traversed prefix consumed.
    pub(crate) fn prepare_has_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<PreparedHas, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let mut prototype = None;
        loop {
            let current = prototype.as_ref().unwrap_or(object);
            match self.ordinary_property_flags(current, key)? {
                Some(Some(_)) => return Ok(PreparedHas::Complete(true)),
                Some(None) => {}
                None => {
                    if self.proxy_snapshot_if_any(current)?.is_some() {
                        return Ok(PreparedHas::Proxy(current.clone()));
                    }
                    if self.typed_array_is_object(current)?
                        && let Some(numeric) = self.typed_array_canonical_numeric_index(key)?
                    {
                        return Ok(PreparedHas::Complete(match numeric {
                            CanonicalNumericIndex::Valid(index) => self
                                .typed_array_get_index_descriptor(current, index)?
                                .is_some(),
                            CanonicalNumericIndex::Invalid => false,
                        }));
                    }
                    if self.has_own_property(current, key)? {
                        return Ok(PreparedHas::Complete(true));
                    }
                }
            }
            prototype = self.get_prototype_of(current)?;
            if prototype.is_none() {
                return Ok(PreparedHas::Complete(false));
            }
        }
    }

    fn proxy_has_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        boolean::finish(
            self,
            realm,
            ProxyBooleanStep::start(
                self,
                realm,
                object.clone(),
                ProxyBooleanKind::Has(key.clone()),
            )?,
        )
    }

    pub(crate) fn internal_get(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Completion, RuntimeError> {
        Ok(
            match self.internal_get_or_missing(realm, object, key, receiver)? {
                NativeConversion::Value(value) => {
                    Completion::Return(value.unwrap_or(Value::Undefined))
                }
                NativeConversion::Throw(value) => Completion::Throw(value),
            },
        )
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
    pub(crate) fn internal_get_or_missing(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_value_domain(&receiver, "property receiver")?;
        self.get_ordinary_chain(realm, object, key, receiver)
    }

    pub(super) fn get_special_or_missing(
        &self,
        kind: SpecialKind,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        if !matches!(kind, SpecialKind::Proxy) {
            return Err(RuntimeError::Invariant(
                "prepared property read left a non-Proxy storage boundary",
            ));
        }
        // Proxy Get observes undefined even when its target lookup is missing.
        // Non-Proxy descriptor/prototype work is shared by prepared reads.
        Ok(match self.proxy_get(realm, object, key, receiver)? {
            Completion::Return(value) => NativeConversion::Value(Some(value)),
            Completion::Throw(value) => NativeConversion::Throw(value),
        })
    }

    fn proxy_get(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Completion, RuntimeError> {
        let mut step = ProxyGetStep::start(self, realm, object.clone(), key.clone(), receiver)?;
        loop {
            step = match step {
                ProxyGetStep::Complete(completion) => return Ok(completion),
                ProxyGetStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    let receiver = resume.take_read_receiver();
                    resume.resume(self, self.internal_get(realm, &object, &key, receiver)?)?
                }
                ProxyGetStep::Call { mut resume } => {
                    let target = resume.take_call_target();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    {
                        let completion = match target {
                            DirectCallTarget::Callable(callable) => {
                                self.call_internal(realm, &callable, receiver, &arguments)?
                            }
                            DirectCallTarget::NonCallableProxy(proxy) => {
                                self.call_proxy(realm, &proxy, receiver, &arguments)?
                            }
                        };
                        resume.resume(self, completion)?
                    }
                }
                ProxyGetStep::Descriptor { mut resume } => {
                    let object = resume.take_descriptor_object();
                    let key = resume.take_descriptor_key();
                    resume
                        .descriptor(self, self.internal_get_own_property(realm, &object, &key)?)?
                }
            };
        }
    }

    pub(crate) fn internal_set(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<NativeConversion<InternalSetResult>, RuntimeError> {
        match self.prepare_set_property_with_receiver_in_realm(
            Some(realm),
            object,
            key,
            value,
            receiver,
        )? {
            PropertySetAction::Complete => Ok(NativeConversion::Value(InternalSetResult::Accepted)),
            PropertySetAction::RejectedProxyTrap => Ok(NativeConversion::Value(
                InternalSetResult::RejectedProxyTrap,
            )),
            PropertySetAction::Throw(value) => Ok(NativeConversion::Throw(value)),
            PropertySetAction::Rejected(reason) => {
                Ok(NativeConversion::Value(InternalSetResult::Rejected(reason)))
            }
            PropertySetAction::Call { payload } => {
                let crate::engine::object::operations::PropertySetterCall {
                    setter,
                    receiver,
                    argument,
                } = *payload;

                let _operation = self.operation();
                match self.call_internal(realm, &setter, receiver, &[argument])? {
                    Completion::Return(_) => {
                        Ok(NativeConversion::Value(InternalSetResult::Accepted))
                    }
                    Completion::Throw(value) => Ok(NativeConversion::Throw(value)),
                }
            }
        }
    }

    /// Only encountered special targets reach this dispatch. Ordinary own
    /// writes do not pre-classify or walk any prototype here.
    pub(super) fn try_special_set(
        &self,
        kind: SpecialKind,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
        receiver: &Value,
    ) -> Result<Option<NativeConversion<InternalSetResult>>, RuntimeError> {
        if matches!(kind, SpecialKind::Proxy) {
            return self
                .proxy_set(realm, object, key, value.clone(), receiver.clone())
                .map(Some);
        }
        if matches!(kind, SpecialKind::ModuleNamespace) {
            return Ok(Some(NativeConversion::Value(InternalSetResult::Rejected(
                PropertySetRejection::ReadOnly,
            ))));
        }
        if !matches!(kind, SpecialKind::TypedArray) {
            return Ok(None);
        }
        let Some(step) = self.prepare_typed_array_set(object, key, value, receiver)? else {
            return Ok(None);
        };
        Ok(Some(match step.finish_sync(self, realm)? {
            NativeConversion::Value(_) => NativeConversion::Value(InternalSetResult::Accepted),
            NativeConversion::Throw(value) => NativeConversion::Throw(value),
        }))
    }

    pub(crate) fn prepare_typed_array_set(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
        receiver: &Value,
    ) -> Result<Option<crate::engine::builtins::TypedWriteStep>, RuntimeError> {
        self.prepare_typed_array_set_in_realm(None, object, key, value, receiver)
    }

    pub(crate) fn prepare_typed_array_set_in_realm(
        &self,
        _realm: Option<ContextId>,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
        receiver: &Value,
    ) -> Result<Option<crate::engine::builtins::TypedWriteStep>, RuntimeError> {
        use crate::engine::builtins::TypedWriteStep;
        match self.select_typed_array_set(object, key, receiver)? {
            TypedSetSelection::Decline => Ok(None),
            TypedSetSelection::Ignore => Ok(Some(TypedWriteStep::Complete(
                NativeConversion::Value(true),
            ))),
            TypedSetSelection::Element(index) => {
                if let Some(realm) = _realm
                    && !matches!(value, Value::Object(_))
                {
                    return TypedWriteStep::set_primitive(self, realm, object, index, value)
                        .map(Some);
                }
                TypedWriteStep::set(self, object.clone(), index, value.clone()).map(Some)
            }
        }
    }

    pub(crate) fn try_typed_array_set_primitive(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: &Value,
        receiver: &Value,
    ) -> Result<Option<NativeConversion<bool>>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "primitive typed Set received an object",
            ));
        }
        match self.select_typed_array_set(object, key, receiver)? {
            TypedSetSelection::Decline => Ok(None),
            TypedSetSelection::Ignore => Ok(Some(NativeConversion::Value(true))),
            TypedSetSelection::Element(index) => {
                crate::engine::builtins::TypedWriteStep::set_primitive_result(
                    self, realm, object, index, value,
                )
                .map(Some)
            }
        }
    }

    fn select_typed_array_set(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: &Value,
    ) -> Result<TypedSetSelection, RuntimeError> {
        let Some(numeric) = self.typed_array_canonical_numeric_index(key)? else {
            return Ok(TypedSetSelection::Decline);
        };
        let same_receiver = matches!(receiver, Value::Object(receiver) if receiver == object);
        if same_receiver {
            return Ok(TypedSetSelection::Element(match numeric {
                CanonicalNumericIndex::Valid(index) => Some(index),
                CanonicalNumericIndex::Invalid => None,
            }));
        }
        if let CanonicalNumericIndex::Valid(index) = numeric
            && self
                .typed_array_get_index_descriptor(object, index)?
                .is_some()
        {
            return Ok(TypedSetSelection::Decline);
        }
        Ok(TypedSetSelection::Ignore)
    }

    pub(super) fn proxy_set(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<NativeConversion<InternalSetResult>, RuntimeError> {
        let mut step =
            ProxySetStep::start(self, realm, object.clone(), key.clone(), value, receiver)?;
        loop {
            step = match step {
                ProxySetStep::Complete(result) => return Ok(result),
                ProxySetStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    let receiver = resume.take_read_receiver();
                    resume.resume(self, self.internal_get(realm, &object, &key, receiver)?)?
                }
                ProxySetStep::Call { mut resume } => {
                    let target = resume.take_call_target();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    {
                        let completion = match target {
                            DirectCallTarget::Callable(callable) => {
                                self.call_internal(realm, &callable, receiver, &arguments)?
                            }
                            DirectCallTarget::NonCallableProxy(proxy) => {
                                self.call_proxy(realm, &proxy, receiver, &arguments)?
                            }
                        };
                        resume.resume(self, completion)?
                    }
                }
                ProxySetStep::Set { mut resume } => {
                    let object = resume.take_set_object();
                    let key = resume.take_set_key();
                    let value = resume.take_set_value();
                    let receiver = resume.take_set_receiver();
                    resume.set(self.internal_set(realm, &object, &key, value, receiver)?)?
                }
                ProxySetStep::Descriptor { mut resume } => {
                    let object = resume.take_descriptor_object();
                    let key = resume.take_descriptor_key();
                    resume
                        .descriptor(self, self.internal_get_own_property(realm, &object, &key)?)?
                }
            };
        }
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

    fn proxy_get_own_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<NativeConversion<Option<CompleteOrdinaryPropertyDescriptor>>, RuntimeError> {
        let mut step = ProxyOwnStep::start(self, realm, object.clone(), key.clone())?;
        loop {
            step = match step {
                ProxyOwnStep::Complete(result) => return Ok(result),
                ProxyOwnStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    let receiver = resume.take_read_receiver();
                    resume.resume(self, self.internal_get(realm, &object, &key, receiver)?)?
                }
                ProxyOwnStep::Call { mut resume } => {
                    let target = resume.take_call_target();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    {
                        let completion = match target {
                            DirectCallTarget::Callable(callable) => {
                                self.call_internal(realm, &callable, receiver, &arguments)?
                            }
                            DirectCallTarget::NonCallableProxy(proxy) => {
                                self.call_proxy(realm, &proxy, receiver, &arguments)?
                            }
                        };
                        resume.resume(self, completion)?
                    }
                }
                ProxyOwnStep::Descriptor { mut resume } => {
                    let object = resume.take_descriptor_object();
                    let key = resume.take_descriptor_key();
                    resume
                        .descriptor(self, self.internal_get_own_property(realm, &object, &key)?)?
                }
                ProxyOwnStep::Extensible { mut resume } => {
                    let object = resume.take_extensible_object();
                    resume.extensible(self.internal_is_extensible(realm, &object)?)?
                }
                ProxyOwnStep::Convert { mut resume } => {
                    let value = resume.take_convert_value();
                    resume.converted(self, self.native_to_property_descriptor(realm, value)?)?
                }
            };
        }
    }

    pub(crate) fn internal_define_own_property(
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
        let mut step =
            ProxyDefineStep::start(self, realm, object.clone(), key.clone(), descriptor.clone())?;
        loop {
            step = match step {
                ProxyDefineStep::Complete(result) => return Ok(result),
                ProxyDefineStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    let receiver = resume.take_read_receiver();
                    resume.resume(self, self.internal_get(realm, &object, &key, receiver)?)?
                }
                ProxyDefineStep::Call { mut resume } => {
                    let target = resume.take_call_target();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    {
                        let completion = match target {
                            DirectCallTarget::Callable(callable) => {
                                self.call_internal(realm, &callable, receiver, &arguments)?
                            }
                            DirectCallTarget::NonCallableProxy(proxy) => {
                                self.call_proxy(realm, &proxy, receiver, &arguments)?
                            }
                        };
                        resume.resume(self, completion)?
                    }
                }
                ProxyDefineStep::Define { mut resume } => {
                    let object = resume.take_define_object();
                    let key = resume.take_define_key();
                    let descriptor = resume.take_define_descriptor();
                    resume.defined(self.internal_define_own_property(
                        realm,
                        &object,
                        &key,
                        &descriptor,
                    )?)?
                }
                ProxyDefineStep::Descriptor { mut resume } => {
                    let object = resume.take_descriptor_object();
                    let key = resume.take_descriptor_key();
                    resume
                        .descriptor(self, self.internal_get_own_property(realm, &object, &key)?)?
                }
            };
        }
    }

    pub(crate) fn internal_delete_property(
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
        boolean::finish(
            self,
            realm,
            ProxyBooleanStep::start(
                self,
                realm,
                object.clone(),
                ProxyBooleanKind::Delete(key.clone()),
            )?,
        )
    }

    pub(crate) fn internal_own_property_keys(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<Vec<PropertyKey>>, RuntimeError> {
        let Some(_) = self.proxy_snapshot_if_any(object)? else {
            return self.own_property_keys(object).map(NativeConversion::Value);
        };
        own_keys::finish(
            self,
            realm,
            own_keys::KeysStep::start(self, realm, object.clone())?,
        )
    }

    pub(crate) fn call_proxy(
        &self,
        realm: ContextId,
        proxy: &ObjectRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        let mut owned_arguments = Vec::new();
        owned_arguments
            .try_reserve_exact(arguments.len())
            .map_err(|_| RuntimeError::Invariant("Proxy call arguments allocation failed"))?;
        owned_arguments.extend_from_slice(arguments);
        let mut step =
            ProxyCallStep::start(self, realm, proxy.clone(), this_value, owned_arguments)?;
        loop {
            step = match step {
                ProxyCallStep::Complete(completion) => return Ok(completion),
                ProxyCallStep::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    let receiver = resume.take_read_receiver();
                    {
                        let completion = self.internal_get(realm, &object, &key, receiver)?;
                        resume.resume(self, completion)?
                    }
                }
                ProxyCallStep::Call { mut resume } => {
                    let target = resume.take_call_target();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    {
                        let completion = match target {
                            DirectCallTarget::Callable(callable) => {
                                self.call_internal(realm, &callable, receiver, &arguments)?
                            }
                            DirectCallTarget::NonCallableProxy(proxy) => {
                                self.call_proxy(realm, &proxy, receiver, &arguments)?
                            }
                        };
                        resume.resume(self, completion)?
                    }
                }
            };
        }
    }

    pub(crate) fn construct_proxy(
        &self,
        realm: ContextId,
        proxy: &ConstructorRef,
        new_target: ConstructNewTarget,
        arguments: &[Value],
    ) -> Result<Completion, RuntimeError> {
        construct::finish(
            self,
            realm,
            construct::ProxyConstructStep::start(
                self,
                realm,
                proxy.clone(),
                new_target,
                arguments.to_vec(),
            )?,
        )
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
