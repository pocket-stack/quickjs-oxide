use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::ContextId;
use crate::engine::object::operations::InternalDefineResult;

use crate::engine::object::{
    DescriptorField, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

impl Runtime {
    pub(crate) fn call_iterator_prototype_iterator(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype iterator did not receive a generic invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    pub(crate) fn call_iterator_prototype_to_string_tag_getter(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag getter received the wrong native invocation",
            ));
        };
        Ok(Completion::Return(Value::String(JsString::from_static(
            "Iterator",
        ))))
    }

    pub(crate) fn call_iterator_prototype_to_string_tag_setter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Setter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag setter received the wrong native invocation",
            ));
        };
        let Value::Object(receiver) = this_value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not an object",
            )));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator.prototype toStringTag setter argv was not padded",
            ))?;
        let iterator_prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .iterator_prototype;
        if receiver.object_id() == iterator_prototype {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "Cannot assign to read only property",
            )));
        }

        let key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        let own_property = match self.internal_has_own_property(realm, &receiver, &key)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !own_property {
            let descriptor = OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            };
            return Ok(
                match self.internal_define_own_property(realm, &receiver, &key, &descriptor)? {
                    NativeConversion::Value(InternalDefineResult::Defined) => {
                        Completion::Return(Value::Undefined)
                    }
                    NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => {
                        Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            "proxy: defineProperty exception",
                        )?)
                    }
                    NativeConversion::Value(InternalDefineResult::RejectedOrdinary(target)) => {
                        let message = if !self.has_own_property(&target, &key)?
                            && !self.is_extensible(&target)?
                        {
                            "object is not extensible"
                        } else {
                            "property is not configurable"
                        };
                        Completion::Throw(self.new_native_error(
                            realm,
                            NativeErrorKind::Type,
                            message,
                        )?)
                    }
                    NativeConversion::Throw(value) => Completion::Throw(value),
                },
            );
        }

        Ok(
            if let Some(value) = self.set_property_or_throw(realm, &receiver, &key, value)? {
                Completion::Throw(value)
            } else {
                Completion::Return(Value::Undefined)
            },
        )
    }
}
