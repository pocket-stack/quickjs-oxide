//! Ordinary Set semantics. Storage probes finish before observable calls;
//! exceptional receivers retain the internal-method dispatch contract.
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::ContextId;
use crate::engine::object::operations::{
    ArrayOwnKey, InternalDefineResult, InternalSetResult, PropertyDefineOutcome, PropertySetAction,
    PropertySetRejection,
};
use crate::engine::object::ordinary_storage::{SetProbe, SpecialKind};
use crate::engine::object::{
    CompleteOrdinaryPropertyDescriptor, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor,
    PropertyKey,
};
use crate::engine::value::Value;
use crate::engine::value::conversion::NativeConversion;

impl Runtime {
    #[cfg(test)]
    pub(crate) fn prepare_set_property(
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

    #[cfg(test)]
    pub(crate) fn prepare_set_property_with_receiver(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<PropertySetAction, RuntimeError> {
        self.prepare_set_property_with_receiver_in_realm(None, object, key, value, receiver)
    }

    pub(crate) fn prepare_set_property_with_receiver_in_realm(
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
        if realm.is_none()
            && (self.is_proxy_object(object)?
                || matches!(&receiver, Value::Object(object) if self.is_proxy_object(object)?))
        {
            return Err(RuntimeError::Invariant("exotic Set requires a realm"));
        }
        let mut prototype = None;
        loop {
            let current = prototype.as_ref().unwrap_or(object);
            let same_receiver = matches!(&receiver, Value::Object(target) if target == current);
            match self.ordinary_set_probe(current, key, &value, same_receiver)? {
                SetProbe::Stored(accepted) => {
                    return Ok(if accepted {
                        PropertySetAction::Complete
                    } else {
                        PropertySetAction::Rejected(PropertySetRejection::ReadOnly)
                    });
                }
                SetProbe::Writable => break,
                SetProbe::Setter(set) => {
                    return Ok(match set {
                        Some(setter) => PropertySetAction::Call {
                            setter: crate::engine::object::CallableRef::from_validated_object(
                                ObjectRef::from_borrowed_handle(self.clone(), setter)?,
                            ),
                            receiver,
                            argument: value,
                        },
                        None => PropertySetAction::Rejected(PropertySetRejection::NoSetter),
                    });
                }
                SetProbe::Missing(next) => {
                    let Some(next) = next else { break };
                    prototype = Some(next);
                    continue;
                }
                SetProbe::Special(kind) => {
                    if realm.is_none() && matches!(kind, SpecialKind::Proxy) {
                        return Err(RuntimeError::Invariant("exotic Set requires a realm"));
                    }
                    if let Some(realm) = realm
                        && let Some(result) =
                            self.try_special_set(kind, realm, current, key, &value, &receiver)?
                    {
                        return Ok(set_completion(result));
                    }
                }
            }
            if let Some(property) = self.get_own_property(current, key)? {
                match property {
                    CompleteOrdinaryPropertyDescriptor::Data { writable, .. } => {
                        if same_receiver && self.array_own_key(current, key)? == ArrayOwnKey::Length
                        {
                            return self.prepare_set_array_length(realm, current, key, value);
                        }
                        if !writable {
                            return Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly));
                        }
                        break;
                    }
                    CompleteOrdinaryPropertyDescriptor::Accessor { set, .. } => {
                        return Ok(match set {
                            Some(setter) => PropertySetAction::Call {
                                setter,
                                receiver,
                                argument: value,
                            },
                            None => PropertySetAction::Rejected(PropertySetRejection::NoSetter),
                        });
                    }
                }
            }
            let Some(next) = self.get_prototype_of(current)? else {
                break;
            };
            prototype = Some(next);
        }
        let Value::Object(receiver) = receiver else {
            return Ok(PropertySetAction::Rejected(PropertySetRejection::NotObject));
        };
        // A receiver is a distinct lookup. Its accessor is rejected, never
        // called, even when the inherited target descriptor was writable.
        match self.ordinary_set_probe(&receiver, key, &value, true)? {
            SetProbe::Stored(accepted) => {
                return Ok(if accepted {
                    PropertySetAction::Complete
                } else {
                    PropertySetAction::Rejected(PropertySetRejection::ReadOnly)
                });
            }
            SetProbe::Setter(set) => {
                return Ok(PropertySetAction::Rejected(if set.is_some() {
                    PropertySetRejection::ReadOnly
                } else {
                    PropertySetRejection::NoSetter
                }));
            }
            SetProbe::Missing(_) => {
                return self.define_set_receiver(realm, &receiver, key, value, false);
            }
            SetProbe::Special(_) => {}
            SetProbe::Writable => unreachable!("receiver probe commits a writable data slot"),
        }
        let existing = match realm {
            Some(realm) => match self.internal_get_own_property(realm, &receiver, key)? {
                NativeConversion::Value(existing) => existing,
                NativeConversion::Throw(value) => return Ok(PropertySetAction::Throw(value)),
            },
            None => self.get_own_property(&receiver, key)?,
        };
        match existing {
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                writable: false, ..
            }) => Ok(PropertySetAction::Rejected(PropertySetRejection::ReadOnly)),
            Some(CompleteOrdinaryPropertyDescriptor::Accessor { set, .. }) => {
                Ok(PropertySetAction::Rejected(if set.is_some() {
                    PropertySetRejection::ReadOnly
                } else {
                    PropertySetRejection::NoSetter
                }))
            }
            Some(CompleteOrdinaryPropertyDescriptor::Data { .. }) => {
                if self.set_arguments_index_value(&receiver, key, &value)? {
                    return Ok(PropertySetAction::Complete);
                }
                self.define_set_receiver(realm, &receiver, key, value, true)
            }
            None => self.define_set_receiver(realm, &receiver, key, value, false),
        }
    }

    fn define_set_receiver(
        &self,
        realm: Option<ContextId>,
        receiver: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        existing: bool,
    ) -> Result<PropertySetAction, RuntimeError> {
        let descriptor = if existing {
            OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                ..OrdinaryPropertyDescriptor::new()
            }
        } else {
            OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            }
        };
        let mut rejected_object = None;
        let defined = match realm {
            Some(realm) => {
                match self.internal_define_own_property(realm, receiver, key, &descriptor)? {
                    NativeConversion::Value(InternalDefineResult::Defined) => true,
                    NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => {
                        return Ok(PropertySetAction::RejectedProxyTrap);
                    }
                    NativeConversion::Value(InternalDefineResult::RejectedOrdinary(object)) => {
                        rejected_object = Some(object);
                        false
                    }
                    NativeConversion::Throw(value) => return Ok(PropertySetAction::Throw(value)),
                }
            }
            None => match self.define_own_property_in_realm(None, receiver, key, &descriptor)? {
                PropertyDefineOutcome::Defined(defined) => defined,
                PropertyDefineOutcome::Throw(value) => return Ok(PropertySetAction::Throw(value)),
            },
        };
        if defined {
            return Ok(PropertySetAction::Complete);
        }
        let receiver = rejected_object.as_ref().unwrap_or(receiver);
        Ok(PropertySetAction::Rejected(
            if !self.has_own_property(receiver, key)? && !self.is_extensible(receiver)? {
                PropertySetRejection::NotExtensible
            } else if matches!(self.array_own_key(receiver, key)?, ArrayOwnKey::Index(_))
                && !self.array_length_state(receiver)?.1
            {
                PropertySetRejection::ArrayLengthReadOnly
            } else {
                PropertySetRejection::ReadOnly
            },
        ))
    }
}

fn set_completion(result: NativeConversion<InternalSetResult>) -> PropertySetAction {
    match result {
        NativeConversion::Throw(value) => PropertySetAction::Throw(value),
        NativeConversion::Value(InternalSetResult::Accepted) => PropertySetAction::Complete,
        NativeConversion::Value(InternalSetResult::Rejected(reason)) => {
            PropertySetAction::Rejected(reason)
        }
        NativeConversion::Value(InternalSetResult::RejectedProxyTrap) => {
            PropertySetAction::RejectedProxyTrap
        }
    }
}

impl Runtime {
    /// Ordinary nodes are iterative. A special node delegates exactly once and
    /// preserves the distinction between missing and an observed undefined.
    pub(super) fn get_ordinary_chain(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<NativeConversion<Option<Value>>, RuntimeError> {
        use crate::engine::object::ordinary_storage::ReadProbe;
        use crate::engine::vm::Completion;
        let mut prototype = None;
        loop {
            let current = prototype.as_ref().unwrap_or(object);
            match self.ordinary_read_probe(current, key)? {
                ReadProbe::Value(value) => return Ok(NativeConversion::Value(Some(value))),
                ReadProbe::Getter(None) => {
                    return Ok(NativeConversion::Value(Some(Value::Undefined)));
                }
                ReadProbe::Getter(Some(getter)) => {
                    return Ok(match self.call_internal(realm, &getter, receiver, &[])? {
                        Completion::Return(value) => NativeConversion::Value(Some(value)),
                        Completion::Throw(value) => NativeConversion::Throw(value),
                    });
                }
                ReadProbe::Missing(Some(next)) => prototype = Some(next),
                ReadProbe::Missing(None) => return Ok(NativeConversion::Value(None)),
                ReadProbe::Special(kind) => {
                    return self.get_special_or_missing(kind, realm, current, key, receiver);
                }
            }
        }
    }
}
