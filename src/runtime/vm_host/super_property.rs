//! QuickJS HomeObject and `super` property bridges for one VM frame.
//!
//! The compiler freezes the HomeObject prototype before evaluating a
//! computed key. This module keeps the remaining receiver-aware property
//! behavior at the runtime boundary, where accessors and strict writes can
//! enter JavaScript again without weakening the bytecode stack contract.

use super::*;

impl RuntimeVmHost {
    pub(super) fn active_home_object(&self) -> Result<Value, Error> {
        let function = self
            .current_function
            .as_ref()
            .ok_or_else(|| Error::internal("HomeObject read has no current function"))?;
        let home_object = self
            .runtime
            .bytecode_function_home_object(function)
            .map_err(runtime_error_to_vm_error)?
            .ok_or_else(|| Error::internal("bytecode requested an uninstalled HomeObject"))?;
        Ok(Value::Object(home_object))
    }

    pub(super) fn resolve_super_base(&self, home_object: Value) -> Result<Value, Error> {
        let Value::Object(home_object) = home_object else {
            return Err(Error::new(ErrorKind::Type, "not an object"));
        };
        self.runtime
            .get_prototype_of(&home_object)
            .map(|prototype| prototype.map_or(Value::Null, Value::Object))
            .map_err(runtime_error_to_vm_error)
    }

    pub(super) fn read_super_property(
        &mut self,
        receiver: Value,
        base: Value,
        key: Value,
    ) -> Result<Completion, Error> {
        // QuickJS's get_super_value converts the key before asking its
        // receiver-aware property machinery to inspect a null super base.
        let key = match self.property_key_from_value(key)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Value::Object(base) = base else {
            let suffix = if matches!(base, Value::Null) {
                "' of null"
            } else if matches!(base, Value::Undefined) {
                "' of undefined"
            } else {
                return Err(Error::new(ErrorKind::Type, "not an object"));
            };
            return Err(self
                .runtime
                .native_atom_error(ErrorKind::Type, "cannot read property '", &key, suffix)
                .map_err(runtime_error_to_vm_error)?);
        };
        let action = self
            .runtime
            .prepare_get_property_with_receiver(&base, &key, receiver)
            .map_err(runtime_error_to_vm_error)?;
        self.finish_property_get_action(action)
    }

    pub(super) fn write_super_property(
        &mut self,
        receiver: Value,
        base: Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        // OP_put_super_value first rejects a null/non-object super base, then
        // performs ToPropertyKey after the RHS has run. Its property write
        // uses JS_PROP_THROW_STRICT, so rejection throws only in a strict
        // concise method.
        let Value::Object(base) = base else {
            return Err(Error::new(ErrorKind::Type, "not an object"));
        };
        let key = match self.property_key_from_value(key)? {
            VmPropertyKeyConversion::Key(key) => key,
            VmPropertyKeyConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let action = self
            .runtime
            .prepare_set_property_with_receiver_in_realm(
                Some(self.current_realm),
                &base,
                &key,
                value,
                receiver,
            )
            .map_err(runtime_error_to_vm_error)?;
        self.finish_property_set_action(action, &key, strict)
    }
}
