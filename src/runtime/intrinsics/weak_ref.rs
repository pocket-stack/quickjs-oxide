//! `%WeakRef%` / `%FinalizationRegistry%` and pinned QuickJS weak-target semantics.
//!
//! The heap owns the mixed construction-order weak-object pass. This layer
//! supplies ECMAScript validation, realm/prototype selection, branded native
//! dispatch, and the atom ownership boundary for held Symbol values.

use crate::heap::{
    FinalizationRegistryNativeKind, WeakCollectionKey, WeakRefNativeKind, WeakRefRealmData,
};

use super::super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WeakIntrinsicKind {
    WeakRef,
    FinalizationRegistry,
}

impl Runtime {
    fn weak_intrinsic_mutation_error(error: HeapError) -> RuntimeError {
        match error {
            HeapError::Allocation { .. } => {
                RuntimeError::Engine(Error::new(ErrorKind::JsInternal, "out of memory"))
            }
            error => RuntimeError::Heap(error),
        }
    }

    pub(in crate::runtime) fn initialize_weak_ref_intrinsics(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let weak_ref_prototype = self.new_object(Some(object_prototype))?;
        self.define_native_builtin_auto_init(
            &weak_ref_prototype,
            realm,
            NativeFunctionId::WeakRef(WeakRefNativeKind::Deref),
            "deref",
            0,
            0,
        )?;
        self.define_weak_intrinsic_to_string_tag(&weak_ref_prototype, "WeakRef")?;
        let weak_ref_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::WeakRef(WeakRefNativeKind::Constructor),
            1,
            "WeakRef",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "WeakRef",
            Value::Object(weak_ref_constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&weak_ref_constructor, &weak_ref_prototype)?;

        let finalization_registry_prototype = self.new_object(Some(object_prototype))?;
        self.define_native_builtin_auto_init(
            &finalization_registry_prototype,
            realm,
            NativeFunctionId::FinalizationRegistry(FinalizationRegistryNativeKind::Register),
            "register",
            2,
            3,
        )?;
        self.define_native_builtin_auto_init(
            &finalization_registry_prototype,
            realm,
            NativeFunctionId::FinalizationRegistry(FinalizationRegistryNativeKind::Unregister),
            "unregister",
            1,
            1,
        )?;
        self.define_weak_intrinsic_to_string_tag(
            &finalization_registry_prototype,
            "FinalizationRegistry",
        )?;
        let finalization_registry_constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::FinalizationRegistry(FinalizationRegistryNativeKind::Constructor),
            1,
            "FinalizationRegistry",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "FinalizationRegistry",
            Value::Object(finalization_registry_constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(
            &finalization_registry_constructor,
            &finalization_registry_prototype,
        )?;

        self.0.state.borrow_mut().heap.attach_weak_ref_intrinsics(
            realm,
            WeakRefRealmData {
                weak_ref_prototype: weak_ref_prototype.object_id(),
                finalization_registry_prototype: finalization_registry_prototype.object_id(),
            },
        )?;
        Ok(())
    }

    fn define_weak_intrinsic_to_string_tag(
        &self,
        object: &ObjectRef,
        value: &'static str,
    ) -> Result<(), RuntimeError> {
        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            object,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static(value))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "weak intrinsic toStringTag definition was rejected",
            ));
        }
        Ok(())
    }

    fn weak_ref_realm_data(&self, realm: ContextId) -> Result<WeakRefRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .weak_ref
            .ok_or(RuntimeError::Invariant("realm has no WeakRef intrinsics"))
    }

    fn weak_intrinsic_prototype(
        &self,
        realm: ContextId,
        kind: WeakIntrinsicKind,
    ) -> Result<ObjectId, RuntimeError> {
        let intrinsics = self.weak_ref_realm_data(realm)?;
        Ok(match kind {
            WeakIntrinsicKind::WeakRef => intrinsics.weak_ref_prototype,
            WeakIntrinsicKind::FinalizationRegistry => intrinsics.finalization_registry_prototype,
        })
    }

    fn weak_intrinsic_prototype_from_new_target(
        &self,
        realm: ContextId,
        new_target: Value,
        kind: WeakIntrinsicKind,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        self.prototype_from_constructor_value(realm, &new_target, |fallback_realm| {
            let prototype = self.weak_intrinsic_prototype(fallback_realm, kind)?;
            Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
        })
    }

    fn weak_target_key(
        &self,
        value: &Value,
        role: &'static str,
    ) -> Result<Option<WeakCollectionKey>, RuntimeError> {
        match value {
            Value::Object(object) => {
                if !object.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime(role));
                }
                Ok(Some(WeakCollectionKey::Object(object.object_id())))
            }
            Value::Symbol(symbol) => {
                if !symbol.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime(role));
                }
                let atom = symbol.atom();
                let can_be_held_weakly =
                    self.0.state.borrow().atoms.kind(atom)? == AtomKind::Symbol;
                Ok(can_be_held_weakly.then_some(WeakCollectionKey::Symbol(atom)))
            }
            _ => Ok(None),
        }
    }

    fn invalid_weak_target(
        &self,
        realm: ContextId,
        message: &'static str,
    ) -> Result<Completion, RuntimeError> {
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            message,
        )?))
    }

    fn new_weak_ref_object(
        &self,
        prototype: &ObjectRef,
        target: WeakCollectionKey,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("WeakRef prototype"));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_weak_ref_object(shape, Vec::new(), target)
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(Self::weak_intrinsic_mutation_error(error));
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn new_finalization_registry_object(
        &self,
        prototype: &ObjectRef,
        callback: &CallableRef,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) || !callback.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime(
                "FinalizationRegistry constructor input",
            ));
        }
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state.heap.allocate_finalization_registry_object(
            shape,
            Vec::new(),
            callback.as_object().object_id(),
            realm,
        ) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(Self::weak_intrinsic_mutation_error(error));
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    pub(in crate::runtime) fn call_weak_ref_native(
        &self,
        realm: ContextId,
        kind: WeakRefNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            WeakRefNativeKind::Constructor => {
                let NativeInvocation::Construct { new_target } = invocation else {
                    return Err(RuntimeError::Invariant(
                        "WeakRef constructor received the wrong native invocation",
                    ));
                };
                if matches!(new_target, Value::Undefined) {
                    return Ok(Completion::Throw(self.new_native_error(
                        realm,
                        NativeErrorKind::Type,
                        "constructor requires 'new'",
                    )?));
                }
                let target_value =
                    arguments
                        .readable
                        .first()
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "WeakRef target argv was not padded",
                        ))?;
                let Some(target) = self.weak_target_key(&target_value, "WeakRef target")? else {
                    return self.invalid_weak_target(realm, "invalid target");
                };
                let prototype = match self.weak_intrinsic_prototype_from_new_target(
                    realm,
                    new_target,
                    WeakIntrinsicKind::WeakRef,
                )? {
                    NativeConversion::Value(prototype) => prototype,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                Ok(Completion::Return(Value::Object(
                    self.new_weak_ref_object(&prototype, target)?,
                )))
            }
            WeakRefNativeKind::Deref => {
                let NativeInvocation::Call { this_value } = invocation else {
                    return Err(RuntimeError::Invariant(
                        "WeakRef.prototype.deref received the wrong native invocation",
                    ));
                };
                let Value::Object(weak_ref) = this_value else {
                    return self.invalid_weak_target(realm, "WeakRef object expected");
                };
                if !weak_ref.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("WeakRef receiver"));
                }
                let target = {
                    let state = self.0.state.borrow();
                    state.heap.weak_ref_target(weak_ref.object_id())
                };
                let target = match target {
                    Ok(target) => target,
                    Err(HeapError::Invariant(_)) => {
                        return self.invalid_weak_target(realm, "WeakRef object expected");
                    }
                    Err(error) => return Err(error.into()),
                };
                let Some(target) = target else {
                    return Ok(Completion::Return(Value::Undefined));
                };
                let live = {
                    let state = self.0.state.borrow();
                    match target {
                        WeakCollectionKey::Object(object) => {
                            match state.heap.object_strong_count(object) {
                                Ok(count) => count != 0,
                                Err(HeapError::Stale { .. }) => false,
                                Err(error) => return Err(error.into()),
                            }
                        }
                        WeakCollectionKey::Symbol(atom) => state.atoms.is_live(atom),
                    }
                };
                if !live {
                    return Ok(Completion::Return(Value::Undefined));
                }
                let raw = match target {
                    WeakCollectionKey::Object(object) => RawValue::Object(object),
                    WeakCollectionKey::Symbol(atom) => RawValue::Symbol(atom),
                };
                Ok(Completion::Return(self.root_raw_value(&raw)?))
            }
        }
    }

    fn finalization_registry_receiver(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "FinalizationRegistry method received the wrong native invocation",
            ));
        };
        let Value::Object(registry) = this_value else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "FinalizationRegistry object expected",
            )?));
        };
        if !registry.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("FinalizationRegistry receiver"));
        }
        let has_brand = matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(registry.object_id())?
                .payload,
            ObjectPayload::FinalizationRegistry(_)
        );
        if !has_brand {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "FinalizationRegistry object expected",
            )?));
        }
        Ok(NativeConversion::Value(registry))
    }

    pub(in crate::runtime) fn call_finalization_registry_native(
        &self,
        realm: ContextId,
        kind: FinalizationRegistryNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        if kind == FinalizationRegistryNativeKind::Constructor {
            let NativeInvocation::Construct { new_target } = invocation else {
                return Err(RuntimeError::Invariant(
                    "FinalizationRegistry constructor received the wrong native invocation",
                ));
            };
            if matches!(new_target, Value::Undefined) {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "constructor requires 'new'",
                )?));
            }
            let callback_value =
                arguments
                    .readable
                    .first()
                    .cloned()
                    .ok_or(RuntimeError::Invariant(
                        "FinalizationRegistry callback argv was not padded",
                    ))?;
            let Value::Object(callback_object) = callback_value else {
                return self.invalid_weak_target(realm, "argument must be a function");
            };
            let Some(callback) = self.as_callable(&callback_object)? else {
                return self.invalid_weak_target(realm, "argument must be a function");
            };
            let prototype = match self.weak_intrinsic_prototype_from_new_target(
                realm,
                new_target,
                WeakIntrinsicKind::FinalizationRegistry,
            )? {
                NativeConversion::Value(prototype) => prototype,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            return Ok(Completion::Return(Value::Object(
                self.new_finalization_registry_object(&prototype, &callback, realm)?,
            )));
        }

        let registry = match self.finalization_registry_receiver(realm, invocation)? {
            NativeConversion::Value(registry) => registry,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let first = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "FinalizationRegistry first argv was not padded",
            ))?;
        match kind {
            FinalizationRegistryNativeKind::Constructor => {
                unreachable!("FinalizationRegistry constructor returned before receiver validation")
            }
            FinalizationRegistryNativeKind::Register => {
                let Some(target) = self.weak_target_key(&first, "FinalizationRegistry target")?
                else {
                    return self.invalid_weak_target(realm, "invalid target");
                };
                let held_value =
                    arguments
                        .readable
                        .get(1)
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "FinalizationRegistry held value argv was not padded",
                        ))?;
                if first.same_value(&held_value) {
                    return self.invalid_weak_target(realm, "held value cannot be the target");
                }
                let token_value =
                    arguments
                        .readable
                        .get(2)
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "FinalizationRegistry unregister token argv was not padded",
                        ))?;
                let unregister_token = if matches!(token_value, Value::Undefined) {
                    None
                } else {
                    let Some(token) = self
                        .weak_target_key(&token_value, "FinalizationRegistry unregister token")?
                    else {
                        return self.invalid_weak_target(realm, "invalid unregister token");
                    };
                    Some(token)
                };

                self.validate_value_domain(&held_value, "FinalizationRegistry held value")?;
                let raw_held_value = self.raw_property_value(&held_value)?;
                let mut state = self.0.state.borrow_mut();
                let retained_atoms = state.retain_raw_value_atoms([&raw_held_value])?;
                if let Err(error) = state.heap.finalization_registry_register(
                    registry.object_id(),
                    target,
                    raw_held_value,
                    unregister_token,
                ) {
                    state.release_atoms(retained_atoms)?;
                    return Err(Self::weak_intrinsic_mutation_error(error));
                }
                drop(state);
                Ok(Completion::Return(Value::Undefined))
            }
            FinalizationRegistryNativeKind::Unregister => {
                let Some(token) =
                    self.weak_target_key(&first, "FinalizationRegistry unregister token")?
                else {
                    return self.invalid_weak_target(realm, "invalid unregister token");
                };
                let mut state = self.0.state.borrow_mut();
                let (removed, cleanup) = state
                    .heap
                    .finalization_registry_unregister(registry.object_id(), token)?;
                state.apply_cleanup(cleanup)?;
                Ok(Completion::Return(Value::Bool(removed)))
            }
        }
    }
}
