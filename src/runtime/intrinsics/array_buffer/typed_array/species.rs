//! TypedArray species construction shared by copying prototype methods.
//!
//! QuickJS uses `JS_SpeciesConstructor` with an undefined default sentinel,
//! then either constructs the authored species or allocates the source element
//! class directly in the builtin's defining realm.

use super::*;

impl Runtime {
    pub(super) fn typed_array_species_create(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        length: u64,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let constructor_key = self.intern_property_key("constructor")?;
        let constructor = match self.get_property_in_realm(realm, source, &constructor_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        let species = if matches!(&constructor, Value::Undefined) {
            Value::Undefined
        } else {
            let Value::Object(constructor) = constructor else {
                return Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "not an object",
                )?));
            };
            let species_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
            match self.get_property_in_realm(realm, &constructor, &species_key)? {
                Completion::Return(Value::Null | Value::Undefined) => Value::Undefined,
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            }
        };

        if matches!(&species, Value::Undefined) {
            let prototype = self.typed_array_default_prototype(realm, source_element)?;
            return self.new_typed_array_for_length(realm, &prototype, source_element, length);
        }
        self.typed_array_create_from_constructor(realm, species, length)
    }

    pub(super) fn typed_array_filter_result(
        &self,
        realm: ContextId,
        source: &ObjectRef,
        source_element: TypedArrayElementKind,
        selected: ObjectRef,
        selected_length: u64,
    ) -> Result<Completion, RuntimeError> {
        let target = match self.typed_array_species_create(
            realm,
            source,
            source_element,
            selected_length,
        )? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let set_key = self.intern_property_key("set")?;
        let set = match self.get_property_in_realm(realm, &target, &set_key)? {
            Completion::Return(value) => self.callable_from_value(value)?,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        match self.call_internal(
            realm,
            &set,
            Value::Object(target.clone()),
            &[Value::Object(selected)],
        )? {
            Completion::Return(_) => Ok(Completion::Return(Value::Object(target))),
            Completion::Throw(value) => Ok(Completion::Throw(value)),
        }
    }
}
