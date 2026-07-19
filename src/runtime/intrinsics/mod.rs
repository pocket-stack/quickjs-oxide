use super::*;

mod array;
pub(super) mod date;
mod eval;
mod json;
mod map;
mod math;
mod object;
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
        match self.prepare_set_property_in_realm(realm, object, key, value)? {
            PropertySetAction::Complete => Ok(None),
            PropertySetAction::Throw(value) => Ok(Some(value)),
            PropertySetAction::Call {
                setter,
                receiver,
                argument,
            } => match self.call_internal(realm, &setter, receiver, &[argument])? {
                Completion::Return(_) => Ok(None),
                Completion::Throw(value) => Ok(Some(value)),
            },
            PropertySetAction::Rejected(rejection) => {
                let error = match rejection {
                    PropertySetRejection::ReadOnly => {
                        self.native_atom_error(ErrorKind::Type, "'", key, "' is read-only")?
                    }
                    PropertySetRejection::ArrayLengthReadOnly => {
                        let length = self.intern_property_key("length")?;
                        self.native_atom_error(ErrorKind::Type, "'", &length, "' is read-only")?
                    }
                    PropertySetRejection::NotConfigurable => {
                        Error::new(ErrorKind::Type, "not configurable")
                    }
                    PropertySetRejection::NoSetter => {
                        Error::new(ErrorKind::Type, "no setter for property")
                    }
                    PropertySetRejection::NotExtensible => {
                        Error::new(ErrorKind::Type, "object is not extensible")
                    }
                    PropertySetRejection::NotObject => Error::new(ErrorKind::Type, "not an object"),
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
