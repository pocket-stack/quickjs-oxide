use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::{ObjectId, RawValue, VarRefId};
use crate::engine::object::property::{CompletePropertyDescriptor, PropertyDescriptor};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::engine::object::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, ObjectRef, OrdinaryPropertyDescriptor,
};
use crate::engine::value::{JsString, Value};

pub(crate) enum RawStringProperty {
    Missing,
    String(JsString),
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct ShapeFingerprint {
    pub(crate) prototype: Option<ObjectId>,
    pub(crate) entries: Box<[ShapeEntry]>,
}

pub(crate) enum PropertySnapshot {
    Data {
        value: RawValue,
        flags: PropertyFlags,
    },
    VarRef {
        var_ref: VarRefId,
        flags: PropertyFlags,
    },
    Accessor {
        get: Option<ObjectId>,
        set: Option<ObjectId>,
        flags: PropertyFlags,
    },
    AutoInit,
}

#[cfg(test)]
pub(crate) enum PropertyGetAction {
    Complete(Value),
    Call {
        getter: CallableRef,
        receiver: Value,
    },
}

pub(crate) enum PropertySetAction {
    Complete,
    Rejected(PropertySetRejection),
    Throw(Value),
    Call {
        setter: CallableRef,
        receiver: Value,
        argument: Value,
    },
}

pub(crate) enum PropertyDefineOutcome {
    Defined(bool),
    Throw(Value),
}

pub(crate) enum ArrayLengthConversion {
    Length(u32),
    Throw(Value),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArrayOwnKey {
    Length,
    Index(u32),
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PropertySetRejection {
    ReadOnly,
    ArrayLengthReadOnly,
    NotConfigurable,
    NoSetter,
    NotExtensible,
    NotObject,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InternalSetResult {
    Accepted,
    Rejected(PropertySetRejection),
    RejectedProxyTrap,
}

#[derive(Clone, Debug)]
pub(crate) enum InternalDefineResult {
    Defined,
    RejectedOrdinary(ObjectRef),
    RejectedProxyTrap,
}

pub(crate) fn descriptor_to_validation_record(
    descriptor: &OrdinaryPropertyDescriptor,
) -> PropertyDescriptor<Value> {
    PropertyDescriptor {
        value: descriptor.value.as_ref().into_option().cloned(),
        writable: descriptor.writable.as_ref().into_option().copied(),
        get: descriptor.get.as_ref().into_option().map(|accessor| {
            accessor
                .as_callable()
                .map(|callable| Value::Object(callable.as_object().clone()))
        }),
        set: descriptor.set.as_ref().into_option().map(|accessor| {
            accessor
                .as_callable()
                .map(|callable| Value::Object(callable.as_object().clone()))
        }),
        enumerable: descriptor.enumerable.as_ref().into_option().copied(),
        configurable: descriptor.configurable.as_ref().into_option().copied(),
    }
}

pub(crate) fn complete_to_validation_record(
    descriptor: &CompleteOrdinaryPropertyDescriptor,
) -> CompletePropertyDescriptor<Value> {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => CompletePropertyDescriptor::Data {
            value: value.clone(),
            writable: *writable,
            enumerable: *enumerable,
            configurable: *configurable,
        },
        CompleteOrdinaryPropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => CompletePropertyDescriptor::Accessor {
            get: get
                .as_ref()
                .map(|callable| Value::Object(callable.as_object().clone())),
            set: set
                .as_ref()
                .map(|callable| Value::Object(callable.as_object().clone())),
            enumerable: *enumerable,
            configurable: *configurable,
        },
    }
}

pub(crate) fn validation_record_to_complete(
    descriptor: CompletePropertyDescriptor<Value>,
) -> Result<CompleteOrdinaryPropertyDescriptor, RuntimeError> {
    match descriptor {
        CompletePropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => Ok(CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        }),
        CompletePropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => {
            let get = get
                .map(|value| match value {
                    Value::Object(object) => Ok(CallableRef::from_validated_object(object)),
                    _ => Err(RuntimeError::Invariant(
                        "validated accessor getter was not callable",
                    )),
                })
                .transpose()?;
            let set = set
                .map(|value| match value {
                    Value::Object(object) => Ok(CallableRef::from_validated_object(object)),
                    _ => Err(RuntimeError::Invariant(
                        "validated accessor setter was not callable",
                    )),
                })
                .transpose()?;
            Ok(CompleteOrdinaryPropertyDescriptor::Accessor {
                get,
                set,
                enumerable,
                configurable,
            })
        }
    }
}
