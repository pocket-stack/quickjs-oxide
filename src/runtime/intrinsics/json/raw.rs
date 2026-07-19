//! Pinned QuickJS Raw JSON construction and unforgeable brand checks.

use super::super::super::*;

impl Runtime {
    pub(super) fn call_json_is_raw_json(
        &self,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let branded = match &arguments.readable[0] {
            Value::Object(object) => self.is_raw_json_object(object)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => false,
        };
        Ok(Completion::Return(Value::Bool(branded)))
    }

    pub(super) fn call_json_raw_json(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.native_to_js_string(realm, &arguments.readable[0])? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let valid_boundary = source
            .code_unit_at(0)
            .zip(source.code_unit_at(source.len().saturating_sub(1)))
            .is_some_and(|(first, last)| {
                is_valid_raw_json_boundary(first) && is_valid_raw_json_boundary(last)
            });
        if !valid_boundary {
            return self.invalid_raw_json(realm);
        }
        match self.parse_json_text(realm, &source, false)? {
            NativeConversion::Value(_) => {}
            NativeConversion::Throw(_) => return self.invalid_raw_json(realm),
        }

        // QuickJS allocates the null-prototype branded object only after the
        // complete strict parse succeeds.
        let object = self.new_raw_json_object()?;
        let raw_json = self.intern_property_key("rawJSON")?;
        if !self.define_own_property(
            &object,
            &raw_json,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(source)),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(false),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh Raw JSON property definition was rejected",
            ));
        }
        self.prevent_extensions(&object)?;
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(super) fn is_raw_json_object(&self, object: &ObjectRef) -> Result<bool, RuntimeError> {
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("Raw JSON object"));
        }
        Ok(matches!(
            self.0
                .state
                .borrow()
                .heap
                .object(object.object_id())?
                .payload,
            ObjectPayload::RawJson
        ))
    }

    fn new_raw_json_object(&self) -> Result<ObjectRef, RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(None, &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::raw_json(shape, Vec::new()))
        {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn invalid_raw_json(&self, realm: ContextId) -> Result<Completion, RuntimeError> {
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Syntax,
            "invalid rawJSON string",
        )?))
    }
}

fn is_valid_raw_json_boundary(unit: u16) -> bool {
    matches!(unit, 0x61..=0x7a | 0x30..=0x39 | 0x2d | 0x22)
}
