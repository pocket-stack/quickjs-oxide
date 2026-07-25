use super::*;

mod array;
pub(super) mod date;
mod error;
mod eval;
mod iterator;
mod json;
mod map;
mod math;
mod object;
pub(super) mod promise;
mod proxy;
mod reflect;
mod regexp;
mod replacement;
mod set;
mod string;

impl Runtime {
    /// Perform ordinary throwing Set for builtin algorithms which publish
    /// values through `[[Set]]` rather than CreateDataProperty.
    pub(in crate::runtime) fn set_property_or_throw(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<Option<Value>, RuntimeError> {
        match self.internal_set(realm, object, key, value, Value::Object(object.clone()))? {
            NativeConversion::Value(InternalSetResult::Accepted) => Ok(None),
            NativeConversion::Throw(value) => Ok(Some(value)),
            NativeConversion::Value(result) => {
                let error = match result {
                    InternalSetResult::RejectedProxyTrap => {
                        Error::new(ErrorKind::Type, "proxy: cannot set property")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::ReadOnly) => {
                        self.native_atom_error(ErrorKind::Type, "'", key, "' is read-only")?
                    }
                    InternalSetResult::Rejected(PropertySetRejection::ArrayLengthReadOnly) => {
                        let length = self.intern_property_key("length")?;
                        self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotConfigurable) => {
                        Error::new(ErrorKind::Type, "not configurable")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NoSetter) => {
                        Error::new(ErrorKind::Type, "no setter for property")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotExtensible) => {
                        Error::new(ErrorKind::Type, "object is not extensible")
                    }
                    InternalSetResult::Rejected(PropertySetRejection::NotObject) => {
                        Error::new(ErrorKind::Type, "not an object")
                    }
                    InternalSetResult::Accepted => unreachable!("accepted Set returned above"),
                };
                Ok(Some(self.new_native_error_from_error(
                    realm,
                    NativeErrorKind::Type,
                    &error,
                )?))
            }
        }
    }
}
