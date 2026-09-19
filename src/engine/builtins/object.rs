//! Object constructor and prototype intrinsics.

use crate::engine::api::error::{ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::PropertyKeyKind;

use crate::engine::builtins::native::{
    NativeFunctionId, ObjectAccessorKind, ObjectExtensibilityKind, ObjectIntegrityKind,
    ObjectKeysKind, ObjectOwnPropertyKeysKind, PrimitiveKind,
};
use crate::engine::heap::{ContextId, ObjectPayload, PrimitiveObjectData};
use crate::engine::object::operations::{ArrayOwnKey, InternalDefineResult};
use crate::engine::object::{
    AccessorValue, CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, SymbolRef,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

pub(super) mod constructor;
pub(crate) mod copy;
pub(super) mod definitions;
pub(super) mod iteration;
pub(super) mod predicate;
pub(super) mod property;
pub(super) mod prototype;
pub(super) mod string;

#[cfg(test)]
mod tests;

pub(crate) enum ObjectIteratorStep {
    Yield(Value),
    Done,
    Throw(Value),
}

impl Runtime {
    /// QuickJS `JS_ToPrimitive(..., HINT_FORCE_ORDINARY)`: probe the ordinary
    /// conversion methods without consulting `Symbol.toPrimitive`. Date's
    /// standard exotic method delegates here after translating its hint.
    /// QuickJS `js_object_groupBy(..., is_map = 0)`.
    ///
    /// The upstream routine deliberately closes the iterator only after an
    /// abrupt callback, property-key conversion, or element-count check. An
    /// abrupt iterator step or group-Array append takes the ordinary exception
    /// exit instead, so those branches must remain separate here.
    pub(crate) fn call_object_group_by(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        const MAX_SAFE_INTEGER: u64 = (1_u64 << 53) - 1;

        self.call_object_group_by_with_element_limit(realm, invocation, arguments, MAX_SAFE_INTEGER)
    }

    fn call_object_group_by_with_element_limit(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        element_limit: u64,
    ) -> Result<Completion, RuntimeError> {
        iteration::finish(
            self,
            realm,
            iteration::IterationStep::start_with_limit(
                self,
                realm,
                iteration::IterationKind::Group,
                &invocation,
                arguments,
                element_limit,
            )?,
        )
    }

    /// QuickJS `js_object_fromEntries`.
    ///
    /// Unlike `Object.groupBy`, the pinned implementation allocates its
    /// defining-realm result before touching the iterable and closes an
    /// acquired iterator after every subsequent abrupt completion, including
    /// `next` lookup and iterator-step failures. `JS_IteratorClose(..., TRUE)`
    /// always restores the original pending exception, so close failures are
    /// deliberately ignored by `close_iterator_preserving_throw`.
    pub(crate) fn call_object_from_entries(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        iteration::finish(
            self,
            realm,
            iteration::IterationStep::start(
                self,
                realm,
                iteration::IterationKind::Entries,
                &invocation,
                arguments,
            )?,
        )
    }

    /// QuickJS `js_object_hasOwn`.
    ///
    /// The static method converts its target before its key (the reverse of
    /// `Object.prototype.hasOwnProperty`) and then performs the descriptor-free
    /// `JS_GetOwnPropertyInternal` presence check. The local property kernel
    /// preserves that check without materializing AutoInit payloads.
    pub(crate) fn call_object_has_own(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        predicate::finish(
            self,
            realm,
            predicate::PredicateStep::start(
                self,
                realm,
                predicate::PredicateKind::HasOwn,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn initialize_object_prototype_intrinsics(
        &self,
        realm: ContextId,
        object_prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        for (target, name, length, min_readable_args) in [
            (NativeFunctionId::ObjectPrototypeToString, "toString", 0, 0),
            (
                NativeFunctionId::ObjectPrototypeToLocaleString,
                "toLocaleString",
                0,
                0,
            ),
            (NativeFunctionId::ObjectPrototypeValueOf, "valueOf", 0, 0),
            (
                NativeFunctionId::ObjectPrototypeHasOwnProperty,
                "hasOwnProperty",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectPrototypeIsPrototypeOf,
                "isPrototypeOf",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectPrototypePropertyIsEnumerable,
                "propertyIsEnumerable",
                1,
                1,
            ),
        ] {
            self.define_native_builtin_auto_init(
                object_prototype,
                realm,
                target,
                name,
                length,
                min_readable_args,
            )?;
        }

        let function_prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .function_prototype;
        let function_prototype = ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
        let getter = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::ObjectPrototypeProtoGetter,
            0,
            "get __proto__",
            0,
        )?;
        let setter = self.new_native_builtin(
            &function_prototype,
            realm,
            NativeFunctionId::ObjectPrototypeProtoSetter,
            1,
            "set __proto__",
            1,
        )?;
        let proto = self.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Proto)?;
        if !self.define_own_property(
            object_prototype,
            &proto,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(getter)),
                set: DescriptorField::Present(AccessorValue::Callable(setter)),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "Object.prototype __proto__ definition was rejected",
            ));
        }

        for (target, name, length, min_readable_args) in [
            (
                NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Getter),
                "__defineGetter__",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectPrototypeDefineAccessor(ObjectAccessorKind::Setter),
                "__defineSetter__",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Getter),
                "__lookupGetter__",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectPrototypeLookupAccessor(ObjectAccessorKind::Setter),
                "__lookupSetter__",
                1,
                1,
            ),
        ] {
            self.define_native_builtin_auto_init(
                object_prototype,
                realm,
                target,
                name,
                length,
                min_readable_args,
            )?;
        }
        Ok(())
    }

    pub(crate) fn initialize_object_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::ObjectConstructor,
            1,
            "Object",
            1,
        )?;
        for (target, name, length, min_readable_args) in [
            (NativeFunctionId::ObjectCreate, "create", 2, 2),
            (
                NativeFunctionId::ObjectGetPrototypeOf,
                "getPrototypeOf",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectSetPrototypeOf,
                "setPrototypeOf",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectDefineProperty,
                "defineProperty",
                3,
                3,
            ),
            (
                NativeFunctionId::ObjectDefineProperties,
                "defineProperties",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Names),
                "getOwnPropertyNames",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyKeys(ObjectOwnPropertyKeysKind::Symbols),
                "getOwnPropertySymbols",
                1,
                1,
            ),
            (NativeFunctionId::ObjectGroupBy, "groupBy", 2, 2),
            (
                NativeFunctionId::ObjectKeys(ObjectKeysKind::Keys),
                "keys",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectKeys(ObjectKeysKind::Values),
                "values",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectKeys(ObjectKeysKind::Entries),
                "entries",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::IsExtensible),
                "isExtensible",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectExtensibility(ObjectExtensibilityKind::PreventExtensions),
                "preventExtensions",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyDescriptor,
                "getOwnPropertyDescriptor",
                2,
                2,
            ),
            (
                NativeFunctionId::ObjectGetOwnPropertyDescriptors,
                "getOwnPropertyDescriptors",
                1,
                1,
            ),
            (NativeFunctionId::ObjectIs, "is", 2, 2),
            (NativeFunctionId::ObjectAssign, "assign", 2, 2),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Seal),
                "seal",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::Freeze),
                "freeze",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsSealed),
                "isSealed",
                1,
                1,
            ),
            (
                NativeFunctionId::ObjectIntegrity(ObjectIntegrityKind::IsFrozen),
                "isFrozen",
                1,
                1,
            ),
            (NativeFunctionId::ObjectFromEntries, "fromEntries", 1, 1),
            (NativeFunctionId::ObjectHasOwn, "hasOwn", 2, 2),
        ] {
            self.define_native_builtin_auto_init(
                constructor.as_object(),
                realm,
                target,
                name,
                length,
                min_readable_args,
            )?;
        }
        self.define_function_data_property(
            global_object,
            "Object",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, object_prototype)
    }

    fn object_default_to_string_tag(
        &self,
        realm: ContextId,
        object: &ObjectRef,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        // Pinned QuickJS performs JS_IsArray before its class/callability
        // fallback and before reading @@toStringTag. That unwraps every Proxy
        // layer and makes revocation observable even when the handler would
        // otherwise provide a custom tag.
        let is_array = match self.internal_is_array(realm, &Value::Object(object.clone()))? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let default_tag = if is_array {
            JsString::from_static("Array")
        } else {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            match &object_data.payload {
                ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => JsString::from_static("Function"),
                ObjectPayload::Proxy(proxy) if proxy.is_callable => {
                    JsString::from_static("Function")
                }
                ObjectPayload::Proxy(_) => JsString::from_static("Object"),
                ObjectPayload::Error => JsString::from_static("Error"),
                ObjectPayload::Primitive(PrimitiveObjectData::Number(_)) => {
                    JsString::from_static("Number")
                }
                ObjectPayload::Primitive(PrimitiveObjectData::String(_)) => {
                    JsString::from_static("String")
                }
                ObjectPayload::Primitive(PrimitiveObjectData::Boolean(_)) => {
                    JsString::from_static("Boolean")
                }
                // QuickJS's built-in class fallback has no Symbol- or
                // BigInt-wrapper case. Their standard tags come exclusively
                // from inherited configurable @@toStringTag properties.
                ObjectPayload::Primitive(
                    PrimitiveObjectData::Symbol(_) | PrimitiveObjectData::BigInt(_),
                ) => JsString::from_static("Object"),
                ObjectPayload::Array { .. } => JsString::from_static("Array"),
                ObjectPayload::Arguments { .. } => JsString::from_static("Arguments"),
                ObjectPayload::Date(_) => JsString::from_static("Date"),
                ObjectPayload::RegExp(_) => JsString::from_static("RegExp"),
                ObjectPayload::Ordinary
                | ObjectPayload::ArrayBuffer(_)
                | ObjectPayload::SharedArrayBuffer(_)
                | ObjectPayload::DataView(_)
                | ObjectPayload::TypedArray(_)
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
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
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::AsyncFunctionState(_)
                // Pinned QuickJS's Object.prototype.toString class switch
                // deliberately excludes JS_CLASS_GENERATOR. The standard
                // "Generator" tag comes from the inherited @@toStringTag;
                // deleting it therefore falls back to "Object".
                | ObjectPayload::Generator { .. }
            | ObjectPayload::AsyncGenerator(_) => JsString::from_static("Object"),
                ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. } => JsString::from_static("Object"),
            }
        };
        Ok(NativeConversion::Value(default_tag))
    }

    pub(crate) fn call_object_prototype_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        string::finish(
            self,
            realm,
            string::ObjectStringStep::start(
                self,
                realm,
                string::ObjectStringKind::Tag,
                &invocation,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_to_locale_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        string::finish(
            self,
            realm,
            string::ObjectStringStep::start(
                self,
                realm,
                string::ObjectStringKind::Locale,
                &invocation,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_value_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.prototype.valueOf did not receive a generic invocation",
            ));
        };
        match this_value {
            value @ Value::Object(_) => Ok(Completion::Return(value)),
            Value::Undefined | Value::Null => Ok(Completion::Throw(self.new_native_error_jsvalue(
                realm,
                NativeErrorKind::Type,
                "cannot convert to object",
            )?)),
            value @ Value::Bool(_) => {
                let prototype =
                    self.primitive_prototype_for_realm(realm, PrimitiveKind::Boolean)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Boolean, value)?,
                )))
            }
            value @ (Value::Int(_) | Value::Float(_)) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::Number)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Number, value)?,
                )))
            }
            value @ Value::String(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::String)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::String, value)?,
                )))
            }
            value @ Value::BigInt(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::BigInt)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::BigInt, value)?,
                )))
            }
            value @ Value::Symbol(_) => {
                let prototype = self.primitive_prototype_for_realm(realm, PrimitiveKind::Symbol)?;
                Ok(Completion::Return(Value::Object(
                    self.new_primitive_object(&prototype, PrimitiveKind::Symbol, value)?,
                )))
            }
        }
    }

    pub(crate) fn call_object_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        constructor::finish(
            self,
            realm,
            constructor::ObjectConstructorStep::start(self, realm, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_object_create(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object definitions did not receive a generic invocation",
            ));
        };
        definitions::finish(
            self,
            realm,
            definitions::DefinitionsStep::start(
                self,
                realm,
                definitions::DefinitionsKind::Create,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_get_prototype_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.getPrototypeOf did not receive a generic invocation",
            ));
        };
        prototype::finish(
            self,
            realm,
            prototype::BuiltinPrototypeStep::start(
                self,
                realm,
                prototype::BuiltinPrototypeKind::ObjectGet,
                arguments,
            )?,
        )
    }

    fn finish_set_prototype_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        result: NativeConversion<bool>,
    ) -> Result<Option<Value>, RuntimeError> {
        match result {
            NativeConversion::Value(true) => return Ok(None),
            NativeConversion::Throw(value) => return Ok(Some(value)),
            NativeConversion::Value(false) => {}
        }
        if self.is_proxy_object(object)? {
            return Ok(Some(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "proxy: bad prototype",
            )?));
        }
        let (immutable, extensible) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            (object.immutable_prototype, object.extensible)
        };
        let message = if immutable {
            "prototype is immutable"
        } else if !extensible {
            "object is not extensible"
        } else {
            "circular prototype chain"
        };
        Ok(Some(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }

    pub(crate) fn call_object_set_prototype_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.setPrototypeOf did not receive a generic invocation",
            ));
        };
        prototype::finish(
            self,
            realm,
            prototype::BuiltinPrototypeStep::start(
                self,
                realm,
                prototype::BuiltinPrototypeKind::ObjectSet,
                arguments,
            )?,
        )
    }

    fn property_define_rejection(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        if let ArrayOwnKey::Index(index) = self.array_own_key(object, key)? {
            let (length, writable) = self.array_length_state(object)?;
            if index >= length && !writable {
                let length =
                    self.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?;
                let error =
                    self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?;
                return self.new_native_error_from_error(realm, NativeErrorKind::Type, &error);
            }
        }
        let message = if !self.has_own_property(object, key)? && !self.is_extensible(object)? {
            "object is not extensible"
        } else {
            "property is not configurable"
        };
        self.new_native_error(realm, NativeErrorKind::Type, message)
    }

    fn finish_define_property_or_throw(
        &self,
        realm: ContextId,
        key: &PropertyKey,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<Option<Value>, RuntimeError> {
        match result {
            NativeConversion::Value(InternalDefineResult::Defined) => Ok(None),
            NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => self
                .new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "proxy: defineProperty exception",
                )
                .map(Some),
            NativeConversion::Value(InternalDefineResult::RejectedOrdinary(target)) => self
                .property_define_rejection(realm, &target, key)
                .map(Some),
            NativeConversion::Throw(value) => Ok(Some(value)),
        }
    }

    pub(crate) fn call_object_define_property(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object property method did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(
                self,
                realm,
                property::PropertyKind::ObjectDefine,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_define_properties(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object definitions did not receive a generic invocation",
            ));
        };
        definitions::finish(
            self,
            realm,
            definitions::DefinitionsStep::start(
                self,
                realm,
                definitions::DefinitionsKind::Define,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_get_own_property_keys(
        &self,
        realm: ContextId,
        kind: ObjectOwnPropertyKeysKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object enumeration did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(
                self,
                realm,
                property::PropertyKind::ObjectOwnKeys(kind),
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_keys(
        &self,
        realm: ContextId,
        kind: ObjectKeysKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object enumeration did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(
                self,
                realm,
                property::PropertyKind::ObjectKeys(kind),
                arguments,
            )?,
        )
    }

    fn define_fresh_object_keys_array_element(
        &self,
        array: &ObjectRef,
        index: u32,
        value: Value,
        rejection: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(&index.to_string())?;
        if !self.define_own_property(
            array,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(rejection));
        }
        Ok(())
    }

    pub(crate) fn call_object_extensibility(
        &self,
        realm: ContextId,
        kind: ObjectExtensibilityKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object extensibility method did not receive a generic invocation",
            ));
        };
        let kind = match kind {
            ObjectExtensibilityKind::IsExtensible => property::PropertyKind::ObjectExtensible,
            ObjectExtensibilityKind::PreventExtensions => property::PropertyKind::ObjectPrevent,
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(self, realm, kind, arguments)?,
        )
    }

    /// Allocate the ordinary Object produced by QuickJS `OP_object` in the
    /// executing realm, without routing VM allocation through Context's public
    /// convenience layer.
    pub(crate) fn new_ordinary_object_in_realm(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype)?;
        self.new_object(Some(&prototype))
    }

    fn define_fresh_object_descriptor_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        rejection: &'static str,
    ) -> Result<(), RuntimeError> {
        if !self.define_own_property(
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(rejection));
        }
        Ok(())
    }

    fn complete_descriptor_to_object(
        &self,
        realm: ContextId,
        descriptor: CompleteOrdinaryPropertyDescriptor,
    ) -> Result<ObjectRef, RuntimeError> {
        let object = self.new_ordinary_object_in_realm(realm)?;
        let mut fields = Vec::with_capacity(4);
        match descriptor {
            CompleteOrdinaryPropertyDescriptor::Data {
                value,
                writable,
                enumerable,
                configurable,
            } => {
                fields.push(("value", value));
                fields.push(("writable", Value::Bool(writable)));
                fields.push(("enumerable", Value::Bool(enumerable)));
                fields.push(("configurable", Value::Bool(configurable)));
            }
            CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            } => {
                fields.push((
                    "get",
                    get.map_or(Value::Undefined, |value| Value::Object(value.into_object())),
                ));
                fields.push((
                    "set",
                    set.map_or(Value::Undefined, |value| Value::Object(value.into_object())),
                ));
                fields.push(("enumerable", Value::Bool(enumerable)));
                fields.push(("configurable", Value::Bool(configurable)));
            }
        }
        for (name, value) in fields {
            let key = self.intern_property_key(name)?;
            self.define_fresh_object_descriptor_property(
                &object,
                &key,
                value,
                "fresh property descriptor object rejected a field",
            )?;
        }
        Ok(object)
    }

    pub(crate) fn call_object_get_own_property_descriptor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object property method did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(
                self,
                realm,
                property::PropertyKind::ObjectDescriptor,
                arguments,
            )?,
        )
    }

    pub(crate) fn object_property_key_value(
        &self,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        let kind = self.0.state.borrow().atoms.property_key_kind(key.atom())?;
        match kind {
            PropertyKeyKind::String => Ok(Value::String(
                self.0.state.borrow().atoms.to_js_string(key.atom())?,
            )),
            PropertyKeyKind::Symbol => Ok(Value::Symbol(SymbolRef::from_borrowed_atom(
                self.clone(),
                key.atom(),
            )?)),
            PropertyKeyKind::Private => Err(RuntimeError::Invariant(
                "private key escaped into Object.getOwnPropertyDescriptors",
            )),
        }
    }

    pub(crate) fn call_object_get_own_property_descriptors(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object enumeration did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(
                self,
                realm,
                property::PropertyKind::ObjectDescriptors,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_is(
        &self,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.is did not receive a generic invocation",
            ));
        };
        let left = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant("Object.is lhs argv was not padded"))?;
        let right = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant("Object.is rhs argv was not padded"))?;
        Ok(Completion::Return(Value::Bool(left.same_value(right))))
    }

    pub(crate) fn call_object_assign(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object.assign did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(self, realm, property::PropertyKind::Assign, arguments)?,
        )
    }

    pub(crate) fn call_object_integrity(
        &self,
        realm: ContextId,
        kind: ObjectIntegrityKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object integrity method did not receive a generic invocation",
            ));
        };
        property::finish(
            self,
            realm,
            property::PropertyStep::start(
                self,
                realm,
                property::PropertyKind::Integrity(kind),
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_has_own_property(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        predicate::finish(
            self,
            realm,
            predicate::PredicateStep::start(
                self,
                realm,
                predicate::PredicateKind::PrototypeHasOwn,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_property_is_enumerable(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        predicate::finish(
            self,
            realm,
            predicate::PredicateStep::start(
                self,
                realm,
                predicate::PredicateKind::Enumerable,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_is_prototype_of(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        prototype::finish(
            self,
            realm,
            prototype::BuiltinPrototypeStep::start_invocation(
                self,
                realm,
                prototype::BuiltinPrototypeKind::IsPrototype,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_proto_getter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        prototype::finish(
            self,
            realm,
            prototype::BuiltinPrototypeStep::start_invocation(
                self,
                realm,
                prototype::BuiltinPrototypeKind::Getter,
                &invocation,
                &NativeArguments {
                    actual_arg_count: 0,
                    readable: Vec::new(),
                },
            )?,
        )
    }

    pub(crate) fn call_object_prototype_proto_setter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        prototype::finish(
            self,
            realm,
            prototype::BuiltinPrototypeStep::start_invocation(
                self,
                realm,
                prototype::BuiltinPrototypeKind::Setter,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_define_accessor(
        &self,
        realm: ContextId,
        kind: ObjectAccessorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        predicate::finish(
            self,
            realm,
            predicate::PredicateStep::start(
                self,
                realm,
                predicate::PredicateKind::Define(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    pub(crate) fn call_object_prototype_lookup_accessor(
        &self,
        realm: ContextId,
        kind: ObjectAccessorKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        predicate::finish(
            self,
            realm,
            predicate::PredicateStep::start(
                self,
                realm,
                predicate::PredicateKind::Lookup(kind),
                &invocation,
                arguments,
            )?,
        )
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ObjectIteratorStep>() <= 64);
