//! VM adapter for authenticated class-private data-field instructions.

use super::*;

impl RuntimeVmHost {
    fn validate_private_definition(definition: VariableDefinition) -> Result<Atom, Error> {
        if definition.kind != ClosureVariableKind::PrivateField
            || !definition.is_lexical
            || !definition.is_const
            || definition.is_parameter_initializer
        {
            return Err(Error::internal(
                "private-name opcode referenced a non-private local definition",
            ));
        }
        definition
            .name
            .ok_or_else(|| Error::internal("private-name local has no source name"))
    }

    fn validate_private_descriptor(descriptor: ClosureVariable) -> Result<(), Error> {
        if descriptor.kind != ClosureVariableKind::PrivateField
            || !descriptor.is_lexical
            || !descriptor.is_const
            || !matches!(descriptor.name, ClosureVariableName::Atom(_))
            || !matches!(
                descriptor.source,
                ClosureSource::ParentLocal(_)
                    | ClosureSource::ParentClosure(_)
                    | ClosureSource::EvalEnvironment(_)
            )
        {
            return Err(Error::internal(
                "private-name opcode referenced a non-private closure descriptor",
            ));
        }
        Ok(())
    }

    pub(super) fn initialize_private_name_binding(&mut self, index: u16) -> Result<(), Error> {
        let source_name = Self::validate_private_definition(self.local_definition(index)?)?;
        let description = self
            .runtime
            .0
            .state
            .borrow()
            .atoms
            .to_js_string(source_name)
            .map_err(|error| Error::internal(error.to_string()))?;
        let name = self
            .runtime
            .new_private_name(description)
            .map_err(runtime_error_to_vm_error)?;
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("private-name local index is out of bounds"))?;
        match binding {
            FrameBinding::Uninitialized => {
                *binding = FrameBinding::Private(name);
                Ok(())
            }
            FrameBinding::Private(_) => Err(Error::internal(
                "private-name local was initialized more than once",
            )),
            FrameBinding::Captured(root) => self
                .runtime
                .initialize_private_var_ref(root, &name)
                .map_err(runtime_error_to_vm_error),
            FrameBinding::Direct(_) => Err(Error::internal(
                "private-name initializer reached an ordinary frame value",
            )),
        }
    }

    fn captured_private_name(&self, root: &VarRefRoot) -> Result<Option<PrivateNameRef>, Error> {
        match self
            .runtime
            .raw_var_ref_value(root)
            .map_err(runtime_error_to_vm_error)?
        {
            RawValue::Uninitialized => Ok(None),
            RawValue::Private(_) => self
                .runtime
                .private_name_from_raw_var_ref(root)
                .map(Some)
                .map_err(runtime_error_to_vm_error),
            _ => Err(Error::internal(
                "private-name VarRef contains an ordinary value",
            )),
        }
    }

    fn optional_private_name(
        &self,
        source: PrivateNameSource,
    ) -> Result<Option<PrivateNameRef>, Error> {
        match source {
            PrivateNameSource::Local(index) => {
                Self::validate_private_definition(self.local_definition(index)?)?;
                let binding = self
                    .locals
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("private-name local index is out of bounds"))?;
                match binding {
                    FrameBinding::Private(name) => Ok(Some(name.clone())),
                    FrameBinding::Captured(root) => self.captured_private_name(root),
                    FrameBinding::Uninitialized => Ok(None),
                    FrameBinding::Direct(_) => Err(Error::internal(
                        "private-name local contains an ordinary frame value",
                    )),
                }
            }
            PrivateNameSource::Closure(index) => {
                let descriptor = self
                    .closure_variables
                    .get(usize::from(index))
                    .copied()
                    .ok_or_else(|| {
                        Error::internal("private-name closure index is out of bounds")
                    })?;
                Self::validate_private_descriptor(descriptor)?;
                let root = self
                    .closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("private-name closure slot is out of bounds"))?;
                self.runtime
                    .validate_var_ref_metadata(root, descriptor)
                    .map_err(runtime_error_to_vm_error)?;
                self.captured_private_name(root)
            }
        }
    }

    fn private_name(&self, source: PrivateNameSource) -> Result<PrivateNameRef, Error> {
        self.optional_private_name(source)?
            .ok_or_else(|| Error::new(ErrorKind::Type, "not a symbol"))
    }

    fn private_receiver(base: Value, private_in: bool) -> Result<ObjectRef, Error> {
        let Value::Object(receiver) = base else {
            return Err(Error::new(
                ErrorKind::Type,
                if private_in {
                    "invalid 'in' operand"
                } else {
                    "not an object"
                },
            ));
        };
        Ok(receiver)
    }

    pub(super) fn get_private_field_value(
        &mut self,
        source: PrivateNameSource,
        base: Value,
    ) -> Result<Completion, Error> {
        let receiver = Self::private_receiver(base, false)?;
        let name = self.private_name(source)?;
        self.runtime
            .get_private_field_own(&receiver, &name)
            .map(Completion::Return)
            .map_err(runtime_error_to_vm_error)
    }

    pub(super) fn put_private_field_value(
        &mut self,
        source: PrivateNameSource,
        base: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        let receiver = Self::private_receiver(base, false)?;
        let name = self.private_name(source)?;
        self.runtime
            .set_private_field_own(&receiver, &name, value)
            .map(|()| Completion::Return(Value::Undefined))
            .map_err(runtime_error_to_vm_error)
    }

    pub(super) fn define_private_field_value(
        &mut self,
        source: PrivateNameSource,
        base: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        let receiver = Self::private_receiver(base, false)?;
        let name = self.private_name(source)?;
        self.runtime
            .define_private_field_own(&receiver, &name, value)
            .map(|()| Completion::Return(Value::Undefined))
            .map_err(runtime_error_to_vm_error)
    }

    pub(super) fn private_in_value(
        &mut self,
        source: PrivateNameSource,
        base: Value,
    ) -> Result<Completion, Error> {
        let receiver = Self::private_receiver(base, true)?;
        let Some(name) = self.optional_private_name(source)? else {
            // Pinned QuickJS converts its uninitialized private-name slot to
            // an invalid atom and treats `#x in object` as false. This is
            // observable from a computed key before the later declaration.
            return Ok(Completion::Return(Value::Bool(false)));
        };
        self.runtime
            .has_private_field_own(&receiver, &name)
            .map(|present| Completion::Return(Value::Bool(present)))
            .map_err(runtime_error_to_vm_error)
    }
}
