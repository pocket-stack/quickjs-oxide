//! Runtime property lookup, definition, and object-layout operations.

use super::*;

impl Runtime {
    fn string_exotic_index_value(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<JsString>, RuntimeError> {
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        let ObjectPayload::Primitive(PrimitiveObjectData::String(value)) = &object.payload else {
            return Ok(None);
        };
        let Some(index) = state.atoms.array_index(key.atom())? else {
            return Ok(None);
        };
        let Ok(index) = usize::try_from(index) else {
            return Ok(None);
        };
        Ok(value.code_unit_at(index).map(JsString::from_code_unit))
    }

    fn string_exotic_length(&self, object: &ObjectRef) -> Result<Option<usize>, RuntimeError> {
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(match &object.payload {
            ObjectPayload::Primitive(PrimitiveObjectData::String(value)) => Some(value.len()),
            ObjectPayload::Ordinary
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => None,
        })
    }

    fn string_exotic_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        Ok(self.string_exotic_index_value(object, key)?.map(|value| {
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(value),
                writable: false,
                enumerable: true,
                configurable: false,
            }
        }))
    }

    /// Snapshot an own property as a complete descriptor, including the
    /// virtual UTF-16 index properties of genuine String wrappers.
    pub fn get_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        if let Some(property) = self.string_exotic_own_property(object, key)? {
            return Ok(Some(property));
        }
        let snapshot = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            let shape = state.heap.shape(object_data.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(None);
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let entry = shape.entries().get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            let slot = object_data
                .slots
                .get(index)
                .ok_or(RuntimeError::Invariant("object property slot was missing"))?;
            match slot {
                PropertySlot::Data(value) => PropertySnapshot::Data {
                    value: value.clone(),
                    flags: entry.flags,
                },
                PropertySlot::VarRef(var_ref) => PropertySnapshot::VarRef {
                    var_ref: *var_ref,
                    flags: entry.flags,
                },
                PropertySlot::Accessor { get, set } => PropertySnapshot::Accessor {
                    get: *get,
                    set: *set,
                    flags: entry.flags,
                },
                PropertySlot::AutoInit(_) => PropertySnapshot::AutoInit,
            }
        };

        match snapshot {
            PropertySnapshot::Data { value, flags } => {
                Ok(Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: self.root_raw_value(&value)?,
                    writable: flags.writable,
                    enumerable: flags.enumerable,
                    configurable: flags.configurable,
                }))
            }
            PropertySnapshot::VarRef { var_ref, flags } => {
                let value = self.0.state.borrow().heap.var_ref(var_ref)?.value.clone();
                if matches!(value, RawValue::Uninitialized) {
                    return Err(RuntimeError::Engine(self.native_atom_error(
                        ErrorKind::Reference,
                        "",
                        key,
                        " is not initialized",
                    )?));
                }
                Ok(Some(CompleteOrdinaryPropertyDescriptor::Data {
                    value: self.root_raw_value(&value)?,
                    writable: flags.writable,
                    enumerable: flags.enumerable,
                    configurable: flags.configurable,
                }))
            }
            PropertySnapshot::Accessor { get, set, flags } => {
                let get = get
                    .map(|id| ObjectRef::from_borrowed_handle(self.clone(), id))
                    .transpose()?
                    .map(CallableRef::from_validated_object);
                let set = set
                    .map(|id| ObjectRef::from_borrowed_handle(self.clone(), id))
                    .transpose()?
                    .map(CallableRef::from_validated_object);
                Ok(Some(CompleteOrdinaryPropertyDescriptor::Accessor {
                    get,
                    set,
                    enumerable: flags.enumerable,
                    configurable: flags.configurable,
                }))
            }
            PropertySnapshot::AutoInit => {
                self.materialize_auto_init_property(object, key)?;
                self.get_own_property(object, key)
            }
        }
    }

    /// Read a string property without materializing autoinit slots or running
    /// accessors, for diagnostics which must remain side-effect free.
    ///
    /// This mirrors QuickJS `get_prop_string`: an own property shadows the
    /// prototype even when it is not an ordinary string data property. Only
    /// when the own property is absent is exactly one prototype level checked.
    pub fn raw_string_property_for_diagnostics(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        raw_string_property_one_level(&self.0.state.borrow(), object.object_id(), key.atom())
    }

    pub(in crate::runtime) fn materialize_auto_init_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<(), RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let object_id = object.object_id();
        let (slot_index, initializer) = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object_id)?;
            let shape = state.heap.shape(object.shape)?;
            let slot_index = usize::try_from(
                shape
                    .find(key.atom())
                    .ok_or(RuntimeError::Invariant("autoinit property disappeared"))?,
            )
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let initializer = match object.slots.get(slot_index) {
                Some(PropertySlot::AutoInit(initializer)) => *initializer,
                Some(
                    PropertySlot::Data(_) | PropertySlot::VarRef(_) | PropertySlot::Accessor { .. },
                ) => return Ok(()),
                None => {
                    return Err(RuntimeError::Invariant(
                        "autoinit property slot was missing",
                    ));
                }
            };
            (slot_index, initializer)
        };

        let initialized = (|| -> Result<Value, RuntimeError> {
            Ok(match initializer {
                AutoInitProperty::FunctionPrototype { realm } => {
                    let object_prototype =
                        self.0.state.borrow().heap.context(realm)?.object_prototype;
                    let object_prototype =
                        ObjectRef::from_borrowed_handle(self.clone(), object_prototype)?;
                    let prototype = self.new_object(Some(&object_prototype))?;
                    self.define_function_data_property(
                        &prototype,
                        "constructor",
                        Value::Object(object.clone()),
                        true,
                        true,
                    )?;
                    Value::Object(prototype)
                }
                AutoInitProperty::NativeBuiltin {
                    realm,
                    target,
                    name,
                    length,
                    min_readable_args,
                } => {
                    let function_prototype = self
                        .0
                        .state
                        .borrow()
                        .heap
                        .context(realm)?
                        .function_prototype;
                    let function_prototype =
                        ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?;
                    let callable = self.new_native_builtin(
                        &function_prototype,
                        realm,
                        target,
                        min_readable_args,
                        name,
                        i32::from(length),
                    )?;
                    Value::Object(callable.as_object().clone())
                }
                AutoInitProperty::String { value, .. } => {
                    Value::String(JsString::from_static(value))
                }
                AutoInitProperty::ArrayUnscopables { realm } => {
                    Value::Object(self.instantiate_array_unscopables(realm)?)
                }
                AutoInitProperty::Math { realm } => {
                    Value::Object(self.instantiate_math_intrinsic(realm)?)
                }
                AutoInitProperty::Reflect { realm } => {
                    Value::Object(self.instantiate_reflect_intrinsic(realm)?)
                }
                AutoInitProperty::Json { realm } => {
                    Value::Object(self.instantiate_json_intrinsic(realm)?)
                }
                #[cfg(test)]
                AutoInitProperty::FailureProbe { .. } => {
                    return Err(RuntimeError::Invariant("autoinit failure probe"));
                }
            })
        })();
        let initialized = match initialized {
            Ok(initialized) => initialized,
            Err(initializer_error) => {
                // Once QuickJS has entered an autoinit callback, failure is
                // terminal for that slot: it becomes an ordinary undefined
                // data property and releases the stored realm edge.
                let mut state = self.0.state.borrow_mut();
                let cleanup = state.heap.replace_object_slot(
                    object_id,
                    slot_index,
                    PropertySlot::Data(RawValue::Undefined),
                )?;
                state.apply_cleanup(cleanup)?;
                return Err(initializer_error);
            }
        };
        let raw = self.raw_property_value(&initialized)?;
        let mut state = self.0.state.borrow_mut();
        let retained_atoms = state.retain_slot_atoms(&[PropertySlot::Data(raw.clone())])?;
        let cleanup =
            match state
                .heap
                .replace_object_slot(object_id, slot_index, PropertySlot::Data(raw))
            {
                Ok(cleanup) => cleanup,
                Err(error) => {
                    state.release_atoms(retained_atoms)?;
                    return Err(error.into());
                }
            };
        state.apply_cleanup(cleanup)?;
        drop(state);
        drop(initialized);
        Ok(())
    }

    pub(in crate::runtime) fn prepare_get_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<PropertyGetAction, RuntimeError> {
        self.prepare_get_property_with_receiver(object, key, Value::Object(object.clone()))
    }

    pub(in crate::runtime) fn prepare_get_property_or_missing(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<PropertyGetAction>, RuntimeError> {
        self.prepare_get_property_with_receiver_or_missing(
            object,
            key,
            Value::Object(object.clone()),
        )
    }

    pub(in crate::runtime) fn prepare_get_property_with_receiver(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<PropertyGetAction, RuntimeError> {
        Ok(self
            .prepare_get_property_with_receiver_or_missing(object, key, receiver)?
            .unwrap_or(PropertyGetAction::Complete(Value::Undefined)))
    }

    fn prepare_get_property_with_receiver_or_missing(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Option<PropertyGetAction>, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_value_domain(&receiver, "property receiver")?;
        let mut cursor = Some(object.clone());
        while let Some(current) = cursor {
            if let Some(property) = self.get_own_property(&current, key)? {
                return match property {
                    CompleteOrdinaryPropertyDescriptor::Data { value, .. } => {
                        Ok(Some(PropertyGetAction::Complete(value)))
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor { get: None, .. } => {
                        Ok(Some(PropertyGetAction::Complete(Value::Undefined)))
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor {
                        get: Some(getter), ..
                    } => Ok(Some(PropertyGetAction::Call { getter, receiver })),
                };
            }
            cursor = self.get_prototype_of(&current)?;
        }
        Ok(None)
    }

    #[cfg(test)]
    pub(in crate::runtime) fn prepare_set_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        let _operation = self.operation();
        self.prepare_set_property_with_receiver_in_realm(
            None,
            object,
            key,
            value,
            Value::Object(object.clone()),
        )
    }

    pub(in crate::runtime) fn prepare_set_property_in_realm(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        self.prepare_set_property_with_receiver_in_realm(
            Some(realm),
            object,
            key,
            value,
            Value::Object(object.clone()),
        )
    }

    #[cfg(test)]
    pub(in crate::runtime) fn prepare_set_property_with_receiver(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        self.prepare_set_property_with_receiver_in_realm(None, object, key, value, receiver)
    }

    pub(in crate::runtime) fn prepare_set_property_with_receiver_in_realm(
        &self,
        realm: Option<ContextId>,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_value_domain(&value, "property value")?;
        self.validate_value_domain(&receiver, "property receiver")?;
        let mut cursor = Some(object.clone());
        let mut inherited_allows_write = true;
        let mut direct_array_length = false;
        while let Some(current) = cursor {
            if let Some(property) = self.get_own_property(&current, key)? {
                match property {
                    CompleteOrdinaryPropertyDescriptor::Data { writable, .. } => {
                        direct_array_length = matches!(&receiver, Value::Object(receiver)
                            if receiver == &current
                                && self.array_own_key(&current, key)? == ArrayOwnKey::Length);
                        inherited_allows_write = writable;
                        break;
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor { set: None, .. } => {
                        return Ok(PropertySetAction::Rejected(PropertySetRejection::NoSetter));
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor {
                        set: Some(setter), ..
                    } => {
                        return Ok(PropertySetAction::Call {
                            setter,
                            receiver,
                            argument: value,
                        });
                    }
                }
            }
            cursor = self.get_prototype_of(&current)?;
        }
        if direct_array_length {
            let Value::Object(receiver) = receiver else {
                return Err(RuntimeError::Invariant(
                    "direct Array length write lost its object receiver",
                ));
            };
            return self.prepare_set_array_length(realm, &receiver, key, value);
        }
        if !inherited_allows_write {
            return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
        }

        let Value::Object(receiver) = receiver else {
            return Ok(PropertySetAction::Rejected(PropertySetRejection::NotObject));
        };
        let descriptor = match self.get_own_property(&receiver, key)? {
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            }) => {
                return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
            }
            Some(CompleteOrdinaryPropertyDescriptor::Accessor { set: None, .. }) => {
                return Ok(PropertySetAction::Rejected(PropertySetRejection::NoSetter));
            }
            Some(CompleteOrdinaryPropertyDescriptor::Accessor { set: Some(_), .. }) => {
                return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
            }
            Some(CompleteOrdinaryPropertyDescriptor::Data { .. }) => {
                if self.set_arguments_index_value(&receiver, key, &value)? {
                    return Ok(PropertySetAction::Complete);
                }
                OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    ..OrdinaryPropertyDescriptor::new()
                }
            }
            None => {
                if !self.is_extensible(&receiver)? {
                    return Ok(PropertySetAction::Rejected(
                        PropertySetRejection::NotExtensible,
                    ));
                }
                OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                }
            }
        };
        Ok(
            match self.define_own_property_in_realm(realm, &receiver, key, &descriptor)? {
                PropertyDefineOutcome::Defined(true) => PropertySetAction::Complete,
                PropertyDefineOutcome::Defined(false) => {
                    let rejection =
                        if matches!(self.array_own_key(&receiver, key)?, ArrayOwnKey::Index(_))
                            && !self.array_length_state(&receiver)?.1
                        {
                            PropertySetRejection::ArrayLengthReadOnly
                        } else {
                            PropertySetRejection::ReadOnly
                        };
                    PropertySetAction::Rejected(rejection)
                }
                PropertyDefineOutcome::Throw(value) => PropertySetAction::Throw(value),
            },
        )
    }

    /// Array `length` assignment has the same conversion and deletion kernel
    /// as DefineOwnProperty, but Set must reject every write once length is
    /// read-only, including a SameValue write that DefineOwnProperty accepts.
    fn prepare_set_array_length(
        &self,
        realm: Option<ContextId>,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        let new_length = match self.to_array_length(realm, &value)? {
            ArrayLengthConversion::Length(length) => length,
            ArrayLengthConversion::Throw(value) => return Ok(PropertySetAction::Throw(value)),
        };
        let (_, writable) = self.array_length_state(object)?;
        if !writable {
            return Ok(PropertySetAction::Rejected(
                PropertySetRejection::ArrayLengthReadOnly,
            ));
        }
        let descriptor = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Self::array_length_value(new_length)),
            ..OrdinaryPropertyDescriptor::new()
        };
        Ok(
            match self.define_array_length(realm, object, key, &descriptor)? {
                PropertyDefineOutcome::Defined(true) => PropertySetAction::Complete,
                PropertyDefineOutcome::Defined(false) => {
                    PropertySetAction::Rejected(PropertySetRejection::NotConfigurable)
                }
                PropertyDefineOutcome::Throw(value) => PropertySetAction::Throw(value),
            },
        )
    }

    /// Validate and apply an own-property descriptor, including genuine Array
    /// index and length exotic semantics. This context-free entry point is
    /// sufficient for primitive descriptor values and host construction; VM
    /// and Context callers use the realm-aware path so object-to-number
    /// conversions can preserve arbitrary JavaScript throws.
    pub fn define_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<bool, RuntimeError> {
        match self.define_own_property_in_realm(None, object, key, descriptor)? {
            PropertyDefineOutcome::Defined(defined) => Ok(defined),
            PropertyDefineOutcome::Throw(_) => Err(RuntimeError::Invariant(
                "context-free property definition produced a JavaScript throw",
            )),
        }
    }

    pub(in crate::runtime) fn define_own_property_in_realm(
        &self,
        realm: Option<ContextId>,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        self.validate_descriptor_domains(descriptor)?;
        if descriptor.is_mixed_descriptor() {
            return Err(PropertyDefinitionError::InvalidDescriptor.into());
        }
        if let Some(defined) = self.define_arguments_index(object, key, descriptor)? {
            return Ok(PropertyDefineOutcome::Defined(defined));
        }
        match self.array_own_key(object, key)? {
            ArrayOwnKey::Length => {
                return self.define_array_length(realm, object, key, descriptor);
            }
            ArrayOwnKey::Index(index) => {
                return self.define_array_index(object, key, index, descriptor);
            }
            ArrayOwnKey::Other => {}
        }
        self.define_ordinary_own_property(object, key, descriptor)
            .map(PropertyDefineOutcome::Defined)
    }

    /// Apply the shared ordinary descriptor algorithm after any class exotic
    /// preconditions and side effects have been handled by the caller.
    pub(super) fn define_ordinary_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<bool, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        self.validate_descriptor_domains(descriptor)?;
        if let Some(current) = self.string_exotic_own_property(object, key)? {
            let descriptor = descriptor_to_validation_record(descriptor);
            let current = complete_to_validation_record(&current);
            return match validate_and_apply_property_descriptor(
                self.is_extensible(object)?,
                &descriptor,
                Some(&current),
                &Value::Undefined,
                Value::same_value,
            ) {
                Ok(_) => Ok(true),
                Err(PropertyDefinitionError::InvalidDescriptor) => {
                    Err(PropertyDefinitionError::InvalidDescriptor.into())
                }
                Err(_) => Ok(false),
            };
        }
        if let Some(flags) = self.auto_init_own_property_flags(object, key)? {
            if descriptor.is_mixed_descriptor() {
                return Err(PropertyDefinitionError::InvalidDescriptor.into());
            }
            // QuickJS check_define_prop_flags checks only the lazy slot's
            // current attributes before JS_AutoInitProperty. Configurable
            // autoinit builtins therefore accept kind and attribute changes,
            // while non-configurable function prototypes and @@hasInstance
            // can reject impossible changes without allocating their value.
            if !flags.configurable
                && (matches!(descriptor.configurable, DescriptorField::Present(true))
                    || matches!(
                        descriptor.enumerable,
                        DescriptorField::Present(enumerable) if enumerable != flags.enumerable
                    )
                    || descriptor.is_accessor_descriptor()
                    || (!flags.writable
                        && matches!(descriptor.writable, DescriptorField::Present(true))))
            {
                return Ok(false);
            }
            // QuickJS performs compatibility checks against the lazy data
            // flags first, then materializes for every compatible define,
            // including an empty descriptor or `writable: false`.
            self.materialize_auto_init_property(object, key)?;
        }
        let current = self.get_own_property(object, key)?;
        let descriptor = descriptor_to_validation_record(descriptor);
        let current_record = current.as_ref().map(complete_to_validation_record);
        let complete = match validate_and_apply_property_descriptor(
            self.is_extensible(object)?,
            &descriptor,
            current_record.as_ref(),
            &Value::Undefined,
            Value::same_value,
        ) {
            Ok(complete) => complete,
            Err(PropertyDefinitionError::InvalidDescriptor) => {
                return Err(PropertyDefinitionError::InvalidDescriptor.into());
            }
            Err(_) => return Ok(false),
        };
        let complete = validation_record_to_complete(complete)?;
        self.store_complete_property(object, key, complete)?;
        Ok(true)
    }

    pub(in crate::runtime) fn array_own_key(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<ArrayOwnKey, RuntimeError> {
        let length = self.intern_property_key("length")?;
        let state = self.0.state.borrow();
        let object_data = state.heap.object(object.object_id())?;
        if !matches!(object_data.payload, ObjectPayload::Array { .. }) {
            return Ok(ArrayOwnKey::Other);
        }
        if key == &length {
            return Ok(ArrayOwnKey::Length);
        }
        Ok(state
            .atoms
            .array_index(key.atom())?
            .map_or(ArrayOwnKey::Other, ArrayOwnKey::Index))
    }

    /// Return QuickJS's representation state for a genuine Array. `Some(n)`
    /// is the dense `u.array.count`; `None` is the irreversible slow form.
    pub(in crate::runtime) fn array_fast_len(
        &self,
        object: &ObjectRef,
    ) -> Result<Option<u32>, RuntimeError> {
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .array_fast_len(object.object_id())?)
    }

    fn set_array_fast_len(
        &self,
        object: &ObjectRef,
        fast_len: Option<u32>,
    ) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .set_array_fast_len(object.object_id(), fast_len)?;
        Ok(())
    }

    fn update_array_fast_state_after_index_define(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        index: u32,
        prior_fast_len: Option<u32>,
    ) -> Result<(), RuntimeError> {
        let Some(fast_len) = prior_fast_len else {
            return Ok(());
        };
        let compatible = matches!(
            self.get_own_property(object, key)?,
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: true,
                enumerable: true,
                configurable: true,
                ..
            })
        );
        let next = if index < fast_len && compatible {
            Some(fast_len)
        } else if index == fast_len && compatible {
            Some(
                fast_len
                    .checked_add(1)
                    .ok_or(RuntimeError::Invariant("fast Array exceeded Uint32"))?,
            )
        } else {
            None
        };
        self.set_array_fast_len(object, next)
    }

    /// Read and structurally validate a genuine Array's mandatory first
    /// `length` slot. The numeric payload is always an exact Uint32, using an
    /// Int for the compact half and a Float above `i32::MAX`.
    pub(in crate::runtime) fn array_length_state(
        &self,
        object: &ObjectRef,
    ) -> Result<(u32, bool), RuntimeError> {
        let length = self.intern_property_key("length")?;
        let state = self.0.state.borrow();
        let object_data = state.heap.object(object.object_id())?;
        if !matches!(object_data.payload, ObjectPayload::Array { .. }) {
            return Err(RuntimeError::Invariant(
                "Array length state requested for a non-Array object",
            ));
        }
        let shape = state.heap.shape(object_data.shape)?;
        let index = shape
            .find(length.atom())
            .ok_or(RuntimeError::Invariant("Array has no length property"))?;
        if index != 0 {
            return Err(RuntimeError::Invariant(
                "Array length property is not physical slot zero",
            ));
        }
        let entry = shape.entries().first().ok_or(RuntimeError::Invariant(
            "Array length shape entry is missing",
        ))?;
        if entry.flags.enumerable
            || entry.flags.configurable
            || entry.flags.storage != crate::shape::PropertyStorageKind::Data
        {
            return Err(RuntimeError::Invariant(
                "Array length property has invalid structural flags",
            ));
        }
        let raw = object_data.slots.first().ok_or(RuntimeError::Invariant(
            "Array length property slot is missing",
        ))?;
        let value = match raw {
            PropertySlot::Data(RawValue::Int(value)) if *value >= 0 => *value as u32,
            PropertySlot::Data(RawValue::Float(value))
                if value.is_finite()
                    && *value >= 0.0
                    && *value <= f64::from(u32::MAX)
                    && value.fract() == 0.0 =>
            {
                *value as u32
            }
            PropertySlot::Data(_)
            | PropertySlot::VarRef(_)
            | PropertySlot::Accessor { .. }
            | PropertySlot::AutoInit(_) => {
                return Err(RuntimeError::Invariant(
                    "Array length property is not an exact Uint32 data value",
                ));
            }
        };
        Ok((value, entry.flags.writable))
    }

    pub(in crate::runtime) fn array_length_value(length: u32) -> Value {
        if let Ok(length) = i32::try_from(length) {
            Value::Int(length)
        } else {
            Value::Float(f64::from(length))
        }
    }

    fn define_array_index(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        index: u32,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        let prior_fast_len = self.array_fast_len(object)?;
        let (old_length, length_writable) = self.array_length_state(object)?;
        if index >= old_length && !length_writable {
            return Ok(PropertyDefineOutcome::Defined(false));
        }
        if !self.define_ordinary_own_property(object, key, descriptor)? {
            return Ok(PropertyDefineOutcome::Defined(false));
        }
        self.update_array_fast_state_after_index_define(object, key, index, prior_fast_len)?;
        if index < old_length {
            return Ok(PropertyDefineOutcome::Defined(true));
        }

        let length = self.intern_property_key("length")?;
        let next_length = index
            .checked_add(1)
            .ok_or(RuntimeError::Invariant("Array index exceeded Uint32 range"))?;
        let updated = self.define_ordinary_own_property(
            object,
            &length,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Self::array_length_value(next_length)),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !updated {
            return Err(RuntimeError::Invariant(
                "writable Array length rejected index growth",
            ));
        }
        Ok(PropertyDefineOutcome::Defined(true))
    }

    fn define_array_length(
        &self,
        realm: Option<ContextId>,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        let DescriptorField::Present(requested) = &descriptor.value else {
            return self
                .define_ordinary_own_property(object, key, descriptor)
                .map(PropertyDefineOutcome::Defined);
        };
        let new_length = match self.to_array_length(realm, requested)? {
            ArrayLengthConversion::Length(length) => length,
            ArrayLengthConversion::Throw(value) => {
                return Ok(PropertyDefineOutcome::Throw(value));
            }
        };

        // Conversion may execute JavaScript and mutate this same Array. Match
        // QuickJS by reloading the length slot only after conversion returns.
        let (old_length, old_writable) = self.array_length_state(object)?;
        let mut canonical = descriptor.clone();
        canonical.value = DescriptorField::Present(Self::array_length_value(new_length));
        if new_length >= old_length || !old_writable {
            return self
                .define_ordinary_own_property(object, key, &canonical)
                .map(PropertyDefineOutcome::Defined);
        }

        let finish_read_only = matches!(canonical.writable, DescriptorField::Present(false));
        if finish_read_only {
            canonical.writable = DescriptorField::Present(true);
        }
        if !self.define_ordinary_own_property(object, key, &canonical)? {
            return Ok(PropertyDefineOutcome::Defined(false));
        }

        for index in self.array_indices_at_or_above(object, new_length)? {
            let index_key = self.intern_property_key(&index.to_string())?;
            if self.delete_property(object, &index_key)? {
                continue;
            }

            // ArraySetLength keeps already deleted higher indices, restores
            // length to the first undeletable index plus one, and still
            // applies a requested writable:false transition.
            let restored_length = index
                .checked_add(1)
                .ok_or(RuntimeError::Invariant("Array index exceeded Uint32 range"))?;
            let restored = self.define_ordinary_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Self::array_length_value(restored_length)),
                    writable: finish_read_only.then_some(false).into(),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !restored {
                return Err(RuntimeError::Invariant(
                    "Array length rollback was rejected",
                ));
            }
            return Ok(PropertyDefineOutcome::Defined(false));
        }

        if finish_read_only {
            let updated = self.define_ordinary_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    writable: DescriptorField::Present(false),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !updated {
                return Err(RuntimeError::Invariant(
                    "Array length writable transition was rejected",
                ));
            }
        }
        Ok(PropertyDefineOutcome::Defined(true))
    }

    fn array_indices_at_or_above(
        &self,
        object: &ObjectRef,
        minimum: u32,
    ) -> Result<Vec<u32>, RuntimeError> {
        let state = self.0.state.borrow();
        let object_data = state.heap.object(object.object_id())?;
        if !matches!(object_data.payload, ObjectPayload::Array { .. }) {
            return Err(RuntimeError::Invariant(
                "Array index scan reached a non-Array object",
            ));
        }
        let shape = state.heap.shape(object_data.shape)?;
        let mut indices = shape
            .entries()
            .iter()
            .filter_map(|entry| state.atoms.array_index(entry.atom).transpose())
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .filter(|index| *index >= minimum)
            .collect::<Vec<_>>();
        indices.sort_unstable_by(|left, right| right.cmp(left));
        Ok(indices)
    }

    pub(in crate::runtime) fn to_array_length(
        &self,
        realm: Option<ContextId>,
        value: &Value,
    ) -> Result<ArrayLengthConversion, RuntimeError> {
        match value {
            Value::Int(value) if *value >= 0 => {
                return Ok(ArrayLengthConversion::Length(*value as u32));
            }
            Value::Bool(value) => {
                return Ok(ArrayLengthConversion::Length(u32::from(*value)));
            }
            Value::Null => return Ok(ArrayLengthConversion::Length(0)),
            Value::Float(value) => return self.validate_array_length_number(realm, *value, None),
            Value::Int(_) => return self.invalid_array_length(realm),
            Value::Undefined
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_)
            | Value::Object(_) => {}
        }

        // QuickJS deliberately preserves the legacy two-conversion behavior
        // for non-number Array length definitions: ToUint32(value), then a
        // second ToNumber(value), followed by equality with an exact Uint32.
        let first = match self.array_length_to_number(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(ArrayLengthConversion::Throw(value)),
        };
        let uint32 = Self::to_uint32_number(first);
        let second = match self.array_length_to_number(realm, value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(ArrayLengthConversion::Throw(value)),
        };
        self.validate_array_length_number(realm, second, Some(uint32))
    }

    fn array_length_to_number(
        &self,
        realm: Option<ContextId>,
        value: &Value,
    ) -> Result<NativeConversion<f64>, RuntimeError> {
        if let Some(realm) = realm {
            self.native_to_number(realm, value)
        } else {
            value
                .to_number()
                .map(NativeConversion::Value)
                .map_err(RuntimeError::Engine)
        }
    }

    pub(in crate::runtime) fn validate_array_length_number(
        &self,
        realm: Option<ContextId>,
        value: f64,
        expected_uint32: Option<u32>,
    ) -> Result<ArrayLengthConversion, RuntimeError> {
        if value >= 0.0 && value <= f64::from(u32::MAX) && value.fract() == 0.0 {
            let length = value as u32;
            if expected_uint32.is_none_or(|expected| expected == length) {
                return Ok(ArrayLengthConversion::Length(length));
            }
        }
        self.invalid_array_length(realm)
    }

    pub(in crate::runtime) fn invalid_array_length(
        &self,
        realm: Option<ContextId>,
    ) -> Result<ArrayLengthConversion, RuntimeError> {
        if let Some(realm) = realm {
            return Ok(ArrayLengthConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Range,
                "invalid array length",
            )?));
        }
        Err(RuntimeError::Engine(Error::new(
            ErrorKind::Range,
            "invalid array length",
        )))
    }

    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub(in crate::runtime) fn to_uint32_number(value: f64) -> u32 {
        if !value.is_finite() || value == 0.0 {
            return 0;
        }
        value.trunc().rem_euclid(4_294_967_296.0) as u32
    }

    fn auto_init_own_property_flags(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<PropertyFlags>, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        let shape = state.heap.shape(object.shape)?;
        let Some(index) = shape.find(key.atom()) else {
            return Ok(None);
        };
        let index = usize::try_from(index)
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
        Ok(
            matches!(object.slots.get(index), Some(PropertySlot::AutoInit(_)))
                .then_some(shape.entries()[index].flags),
        )
    }

    pub(in crate::runtime) fn is_auto_init_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        Ok(self.auto_init_own_property_flags(object, key)?.is_some())
    }

    /// Test own-property presence without materializing autoinit payloads.
    pub fn has_own_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        if self.string_exotic_index_value(object, key)?.is_some() {
            return Ok(true);
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        Ok(state.heap.shape(object.shape)?.find(key.atom()).is_some())
    }

    /// Read an own property's enumerable bit without materializing autoinit
    /// slots. QuickJS's `JS_GetOwnPropertyNamesInternal(...ENUM_ONLY...)`
    /// filters from shape flags before any later property value access.
    pub(in crate::runtime) fn own_property_is_enumerable(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        self.validate_object_and_key(object, key)?;
        if self.string_exotic_index_value(object, key)?.is_some() {
            return Ok(true);
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(object.object_id())?;
        let shape = state.heap.shape(object.shape)?;
        let Some(index) = shape.find(key.atom()) else {
            return Ok(false);
        };
        let index = usize::try_from(index)
            .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
        Ok(shape.entries()[index].flags.enumerable)
    }

    /// Delete an ordinary own property without invoking accessors.
    pub fn delete_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        self.validate_object_and_key(object, key)?;
        if self.string_exotic_index_value(object, key)?.is_some() {
            return Ok(false);
        }
        let array_index = match self.array_own_key(object, key)? {
            ArrayOwnKey::Index(index) => Some(index),
            ArrayOwnKey::Length | ArrayOwnKey::Other => None,
        };
        let arguments_index = self
            .arguments_index_state(object, key)?
            .map(|(index, _, _)| index);
        let global_var_ref = {
            let state = self.0.state.borrow();
            let object_data = state.heap.object(object.object_id())?;
            match &object_data.payload {
                ObjectPayload::GlobalObject { uninitialized_vars } => {
                    let shape = state.heap.shape(object_data.shape)?;
                    let Some(index) = shape.find(key.atom()) else {
                        return Ok(true);
                    };
                    let index = index as usize;
                    let entry = shape.entries().get(index).ok_or(RuntimeError::Invariant(
                        "shape lookup index was out of bounds",
                    ))?;
                    match object_data.slots.get(index).ok_or(RuntimeError::Invariant(
                        "shape property has no parallel object slot",
                    ))? {
                        PropertySlot::VarRef(var_ref)
                            if state.heap.var_ref_strong_count(*var_ref)? > 1 =>
                        {
                            Some((*uninitialized_vars, *var_ref, entry.flags.configurable))
                        }
                        PropertySlot::VarRef(_)
                        | PropertySlot::Data(_)
                        | PropertySlot::Accessor { .. }
                        | PropertySlot::AutoInit(_) => None,
                    }
                }
                ObjectPayload::Ordinary
                | ObjectPayload::Date(_)
                | ObjectPayload::RegExp(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. } => None,
            }
        };
        if let Some((hidden, var_ref, configurable)) = global_var_ref {
            if !configurable {
                return Ok(false);
            }
            let root = VarRefRoot::from_borrowed_handle(self.clone(), var_ref)?;
            let hidden = ObjectRef::from_borrowed_handle(self.clone(), hidden)?;
            match self.own_var_ref_root(&hidden, key)? {
                Some(existing) if existing.id() != root.id() => {
                    return Err(RuntimeError::Invariant(
                        "hidden global table contains a different VarRef",
                    ));
                }
                Some(_) => {}
                None => self.store_property_slot(
                    &hidden,
                    key,
                    PropertyFlags::data(true, true, true),
                    PropertySlot::VarRef(root.id()),
                )?,
            }
            self.reset_var_ref_uninitialized(&root)?;
            self.set_var_ref_metadata(&root, false, false, ClosureVariableKind::Normal)?;
        }
        let mut state = self.0.state.borrow_mut();
        let object_id = object.object_id();
        let (prototype, entries, mut slots, index, configurable) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            let Some(index) = shape.find(key.atom()) else {
                return Ok(true);
            };
            let index = usize::try_from(index)
                .map_err(|_| RuntimeError::Invariant("shape index does not fit usize"))?;
            let entry = *shape.entries().get(index).ok_or(RuntimeError::Invariant(
                "shape lookup index was out of bounds",
            ))?;
            (
                shape.prototype(),
                shape.entries().to_vec(),
                object_data.slots.clone(),
                index,
                entry.flags.configurable,
            )
        };
        if !configurable {
            return Ok(false);
        }

        let fast_update = if let Some(array_index) = array_index {
            match state.heap.array_fast_len(object_id)? {
                Some(fast_len) if array_index < fast_len => Some(if array_index + 1 == fast_len {
                    Some(array_index)
                } else {
                    None
                }),
                Some(_) | None => None,
            }
        } else {
            None
        };
        let arguments_fast_update = if let Some(arguments_index) = arguments_index {
            match state.heap.arguments_state(object_id)?.1 {
                Some(fast_len) if arguments_index < fast_len => {
                    Some(if arguments_index + 1 == fast_len {
                        Some(arguments_index)
                    } else {
                        None
                    })
                }
                Some(_) | None => None,
            }
        } else {
            None
        };

        let mut next_entries = entries;
        next_entries.remove(index);
        slots.remove(index);
        state.replace_layout(object_id, prototype, &next_entries, slots)?;
        if let Some(next_fast_len) = fast_update {
            state.heap.set_array_fast_len(object_id, next_fast_len)?;
        }
        if let Some(next_fast_len) = arguments_fast_update {
            state
                .heap
                .set_arguments_fast_len(object_id, next_fast_len)?;
        }
        Ok(true)
    }

    /// Return a rooted own-key snapshot in ECMAScript order.
    pub fn own_property_keys(&self, object: &ObjectRef) -> Result<Vec<PropertyKey>, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let string_length = self.string_exotic_length(object)?;
        let atoms = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            let atoms = state
                .heap
                .shape(object.shape)?
                .ordered_own_keys(&state.atoms)?;
            if let Some(length) = string_length {
                for atom in &atoms {
                    if state.atoms.array_index(*atom)?.is_some_and(|index| {
                        usize::try_from(index).is_ok_and(|index| index < length)
                    }) {
                        return Err(RuntimeError::Invariant(
                            "String wrapper shape shadowed a virtual index",
                        ));
                    }
                }
            }
            atoms
        };
        let mut keys = Vec::new();
        if let Some(length) = string_length {
            let length = u32::try_from(length).map_err(|_| {
                RuntimeError::Invariant("String wrapper length exceeded QuickJS index space")
            })?;
            for index in 0..length {
                keys.push(self.intern_property_key(&index.to_string())?);
            }
        }
        for atom in atoms {
            keys.push(PropertyKey::from_borrowed_atom(self.clone(), atom)?);
        }
        Ok(keys)
    }

    /// Return the ordinary object's prototype as a new root.
    pub fn get_prototype_of(&self, object: &ObjectRef) -> Result<Option<ObjectRef>, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        let prototype = {
            let state = self.0.state.borrow();
            let object = state.heap.object(object.object_id())?;
            state.heap.shape(object.shape)?.prototype()
        };
        prototype
            .map(|prototype| ObjectRef::from_borrowed_handle(self.clone(), prototype))
            .transpose()
            .map_err(Into::into)
    }

    /// Apply ordinary `[[SetPrototypeOf]]`, including same-value success,
    /// immutable/non-extensible rejection and cycle detection.
    pub fn set_prototype_of(
        &self,
        object: &ObjectRef,
        prototype: Option<&ObjectRef>,
    ) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        if prototype.is_some_and(|prototype| !prototype.belongs_to(self)) {
            return Err(RuntimeError::WrongRuntime("prototype"));
        }
        let object_id = object.object_id();
        let prototype = prototype.map(ObjectRef::object_id);
        let mut state = self.0.state.borrow_mut();
        let (current, extensible, immutable, entries, slots) = {
            let object_data = state.heap.object(object_id)?;
            let shape = state.heap.shape(object_data.shape)?;
            (
                shape.prototype(),
                object_data.extensible,
                object_data.immutable_prototype,
                shape.entries().to_vec(),
                object_data.slots.clone(),
            )
        };
        if current == prototype {
            return Ok(true);
        }
        if immutable || !extensible {
            return Ok(false);
        }

        let mut cursor = prototype;
        while let Some(candidate) = cursor {
            if candidate == object_id {
                return Ok(false);
            }
            let candidate = state.heap.object(candidate)?;
            cursor = state.heap.shape(candidate.shape)?.prototype();
        }
        state.replace_layout(object_id, prototype, &entries, slots)?;
        Ok(true)
    }

    /// Return the ordinary object's extensibility bit.
    pub fn is_extensible(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())?
            .extensible)
    }

    /// Make the ordinary object non-extensible.
    pub fn prevent_extensions(&self, object: &ObjectRef) -> Result<(), RuntimeError> {
        let _operation = self.operation();
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object"));
        }
        self.0
            .state
            .borrow_mut()
            .heap
            .set_object_extensible(object.object_id(), false)?;
        Ok(())
    }
}
