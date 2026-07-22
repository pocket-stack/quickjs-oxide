//! VM adapter for authenticated class-private element instructions.

use super::*;

impl RuntimeVmHost {
    fn validate_private_definition(
        definition: VariableDefinition,
    ) -> Result<(Atom, ClosureVariableKind), Error> {
        if !matches!(
            definition.kind,
            ClosureVariableKind::PrivateField | ClosureVariableKind::PrivateMethod
        ) || !definition.is_lexical
            || !definition.is_const
            || definition.is_parameter_initializer
        {
            return Err(Error::internal(
                "private-name opcode referenced a non-private local definition",
            ));
        }
        definition
            .name
            .map(|name| (name, definition.kind))
            .ok_or_else(|| Error::internal("private-element local has no source name"))
    }

    fn validate_private_descriptor(
        descriptor: ClosureVariable,
    ) -> Result<ClosureVariableKind, Error> {
        if !matches!(
            descriptor.kind,
            ClosureVariableKind::PrivateField | ClosureVariableKind::PrivateMethod
        ) || !descriptor.is_lexical
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
        Ok(descriptor.kind)
    }

    pub(super) fn initialize_private_name_binding(&mut self, index: u16) -> Result<(), Error> {
        let (source_name, kind) = Self::validate_private_definition(self.local_definition(index)?)?;
        if kind != ClosureVariableKind::PrivateField {
            return Err(Error::internal(
                "private-name initializer referenced a non-field binding",
            ));
        }
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
            FrameBinding::PrivateCallable(_) => Err(Error::internal(
                "private-name initializer reached a private-method frame cell",
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

    pub(super) fn initialize_private_method_binding(
        &mut self,
        index: u16,
        home_object: Value,
        method: Value,
    ) -> Result<(), Error> {
        let (source_name, kind) = Self::validate_private_definition(self.local_definition(index)?)?;
        if kind != ClosureVariableKind::PrivateMethod {
            return Err(Error::internal(
                "private-method initializer referenced a non-method binding",
            ));
        }
        let Value::Object(home_object) = home_object else {
            return Err(Error::internal(
                "private-method initializer did not receive a HomeObject",
            ));
        };
        let callable = self
            .runtime
            .callable_from_value(method)
            .map_err(|error| Error::internal(error.to_string()))?;
        let name = self
            .runtime
            .0
            .state
            .borrow()
            .atoms
            .to_js_string(source_name)
            .map_err(|error| Error::internal(error.to_string()))?;
        self.runtime
            .define_object_name(&Value::Object(callable.as_object().clone()), &name)
            .map_err(runtime_error_to_vm_error)?;
        self.runtime
            .install_object_literal_home_object(&callable, &home_object)
            .map_err(runtime_error_to_vm_error)?;

        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("private-method local index is out of bounds"))?;
        match binding {
            FrameBinding::Uninitialized => {
                *binding = FrameBinding::PrivateCallable(callable);
                Ok(())
            }
            FrameBinding::Captured(root) => self
                .runtime
                .initialize_private_method_var_ref(root, &callable)
                .map_err(runtime_error_to_vm_error),
            FrameBinding::PrivateCallable(_) => Err(Error::internal(
                "private-method local was initialized more than once",
            )),
            FrameBinding::Private(_) => Err(Error::internal(
                "private-method initializer reached a private-field frame cell",
            )),
            FrameBinding::Direct(_) => Err(Error::internal(
                "private-method initializer reached an ordinary frame value",
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

    fn captured_private_method(&self, root: &VarRefRoot) -> Result<Option<CallableRef>, Error> {
        match self
            .runtime
            .raw_var_ref_value(root)
            .map_err(runtime_error_to_vm_error)?
        {
            RawValue::Uninitialized => Ok(None),
            RawValue::Object(_) => self
                .runtime
                .private_method_from_raw_var_ref(root)
                .map(Some)
                .map_err(runtime_error_to_vm_error),
            _ => Err(Error::internal(
                "private-method VarRef contains an incompatible value",
            )),
        }
    }

    fn private_source_kind(&self, source: PrivateNameSource) -> Result<ClosureVariableKind, Error> {
        match source {
            PrivateNameSource::Local(index) => {
                let (_, kind) = Self::validate_private_definition(self.local_definition(index)?)?;
                Ok(kind)
            }
            PrivateNameSource::Closure(index) => {
                let descriptor = self
                    .closure_variables
                    .get(usize::from(index))
                    .copied()
                    .ok_or_else(|| {
                        Error::internal("private-element closure index is out of bounds")
                    })?;
                Self::validate_private_descriptor(descriptor)
            }
        }
    }

    fn optional_private_name(
        &self,
        source: PrivateNameSource,
    ) -> Result<Option<PrivateNameRef>, Error> {
        match source {
            PrivateNameSource::Local(index) => {
                let (_, kind) = Self::validate_private_definition(self.local_definition(index)?)?;
                if kind != ClosureVariableKind::PrivateField {
                    return Err(Error::internal(
                        "private-field operation referenced a non-field local",
                    ));
                }
                let binding = self
                    .locals
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("private-name local index is out of bounds"))?;
                match binding {
                    FrameBinding::Private(name) => Ok(Some(name.clone())),
                    FrameBinding::Captured(root) => self.captured_private_name(root),
                    FrameBinding::Uninitialized => Ok(None),
                    FrameBinding::PrivateCallable(_) => Err(Error::internal(
                        "private-field local contains a private method",
                    )),
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
                let kind = Self::validate_private_descriptor(descriptor)?;
                if kind != ClosureVariableKind::PrivateField {
                    return Err(Error::internal(
                        "private-field operation referenced a non-field closure",
                    ));
                }
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

    fn optional_private_method(
        &self,
        source: PrivateNameSource,
    ) -> Result<Option<CallableRef>, Error> {
        match source {
            PrivateNameSource::Local(index) => {
                let (_, kind) = Self::validate_private_definition(self.local_definition(index)?)?;
                if kind != ClosureVariableKind::PrivateMethod {
                    return Err(Error::internal(
                        "private-method operation referenced a non-method local",
                    ));
                }
                let binding = self.locals.get(usize::from(index)).ok_or_else(|| {
                    Error::internal("private-method local index is out of bounds")
                })?;
                match binding {
                    FrameBinding::PrivateCallable(callable) => Ok(Some(callable.clone())),
                    FrameBinding::Captured(root) => self.captured_private_method(root),
                    FrameBinding::Uninitialized => Ok(None),
                    FrameBinding::Private(_) => Err(Error::internal(
                        "private-method local contains a private field identity",
                    )),
                    FrameBinding::Direct(_) => Err(Error::internal(
                        "private-method local contains an ordinary frame value",
                    )),
                }
            }
            PrivateNameSource::Closure(index) => {
                let descriptor = self
                    .closure_variables
                    .get(usize::from(index))
                    .copied()
                    .ok_or_else(|| {
                        Error::internal("private-method closure index is out of bounds")
                    })?;
                let kind = Self::validate_private_descriptor(descriptor)?;
                if kind != ClosureVariableKind::PrivateMethod {
                    return Err(Error::internal(
                        "private-method operation referenced a non-method closure",
                    ));
                }
                let root = self.closure_slots.get(usize::from(index)).ok_or_else(|| {
                    Error::internal("private-method closure slot is out of bounds")
                })?;
                self.runtime
                    .validate_var_ref_metadata(root, descriptor)
                    .map_err(runtime_error_to_vm_error)?;
                self.captured_private_method(root)
            }
        }
    }

    fn private_name(&self, source: PrivateNameSource) -> Result<PrivateNameRef, Error> {
        self.optional_private_name(source)?
            .ok_or_else(|| Error::new(ErrorKind::Type, "not a symbol"))
    }

    fn private_source_name(&self, source: PrivateNameSource) -> Result<Option<Atom>, Error> {
        Ok(match source {
            PrivateNameSource::Local(index) => self.local_definition(index)?.name,
            PrivateNameSource::Closure(index) => {
                let descriptor =
                    self.closure_variables
                        .get(usize::from(index))
                        .ok_or_else(|| {
                            Error::internal("private-element closure index is out of bounds")
                        })?;
                match descriptor.name {
                    ClosureVariableName::Atom(name) => Some(name),
                    ClosureVariableName::None | ClosureVariableName::Constant(_) => None,
                }
            }
        })
    }

    fn uninitialized_private_in(&self, receiver: &ObjectRef) -> Result<bool, Error> {
        // QuickJS feeds its internal JS_UNINITIALIZED value through
        // JS_ValueToAtom here. JS_ToStringInternal deliberately spells that
        // non-language tag as "[unsupported type]", then the private-in
        // opcode performs an own-property probe with the resulting atom.
        let key = self
            .runtime
            .intern_property_key("[unsupported type]")
            .map_err(|error| Error::internal(error.to_string()))?;
        self.runtime
            .has_own_property(receiver, &key)
            .map_err(runtime_error_to_vm_error)
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
        match self.private_source_kind(source)? {
            ClosureVariableKind::PrivateField => {
                let receiver = Self::private_receiver(base, false)?;
                let name = self.private_name(source)?;
                self.runtime
                    .get_private_field_own(&receiver, &name)
                    .map(Completion::Return)
                    .map_err(runtime_error_to_vm_error)
            }
            ClosureVariableKind::PrivateMethod => {
                let Some(method) = self.optional_private_method(source)? else {
                    return Err(Error::new(ErrorKind::Type, "not an object"));
                };
                self.runtime
                    .require_private_method_brand(&method)
                    .map_err(runtime_error_to_vm_error)?;
                let receiver = Self::private_receiver(base, false)?;
                if !self
                    .runtime
                    .check_private_method_brand(&method, &receiver)
                    .map_err(runtime_error_to_vm_error)?
                {
                    return Err(Error::new(ErrorKind::Type, "invalid brand on object"));
                }
                Ok(Completion::Return(Value::Object(
                    method.as_object().clone(),
                )))
            }
            _ => Err(Error::internal(
                "private get referenced an unsupported binding kind",
            )),
        }
    }

    pub(super) fn put_private_field_value(
        &mut self,
        source: PrivateNameSource,
        base: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        if self.private_source_kind(source)? == ClosureVariableKind::PrivateMethod {
            return Err(self.lexical_read_only_error(self.private_source_name(source)?)?);
        }
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
        if self.private_source_kind(source)? != ClosureVariableKind::PrivateField {
            return Err(Error::internal(
                "private-field definition referenced a non-field binding",
            ));
        }
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
        let present = match self.private_source_kind(source)? {
            ClosureVariableKind::PrivateField => {
                let Some(name) = self.optional_private_name(source)? else {
                    return self
                        .uninitialized_private_in(&receiver)
                        .map(|present| Completion::Return(Value::Bool(present)));
                };
                self.runtime
                    .has_private_field_own(&receiver, &name)
                    .map_err(runtime_error_to_vm_error)?
            }
            ClosureVariableKind::PrivateMethod => {
                let Some(method) = self.optional_private_method(source)? else {
                    return self
                        .uninitialized_private_in(&receiver)
                        .map(|present| Completion::Return(Value::Bool(present)));
                };
                self.runtime
                    .check_private_method_brand(&method, &receiver)
                    .map_err(runtime_error_to_vm_error)?
            }
            _ => {
                return Err(Error::internal(
                    "private-in referenced an unsupported binding kind",
                ));
            }
        };
        Ok(Completion::Return(Value::Bool(present)))
    }
}
