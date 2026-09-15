//! VM adapter for authenticated class-private element instructions.

use super::*;

impl RuntimeVmHost {
    fn validate_private_definition(
        definition: VariableDefinition,
    ) -> Result<(Atom, ClosureVariableKind), Error> {
        crate::engine::vm::private_bindings::validate_definition(definition)
    }

    fn validate_private_descriptor(
        descriptor: ClosureVariable,
    ) -> Result<ClosureVariableKind, Error> {
        crate::engine::vm::private_bindings::validate_descriptor(descriptor)
    }

    pub(crate) fn initialize_private_name_binding(&mut self, index: u16) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("private-name local index is out of bounds"))?;
        crate::engine::vm::private_bindings::initialize_name(&self.runtime, definition, binding)
    }

    pub(crate) fn initialize_private_method_binding(
        &mut self,
        index: u16,
        home_object: Value,
        method: Value,
    ) -> Result<(), Error> {
        self.initialize_private_callable_binding(index, home_object, method, true, |kind| {
            kind == ClosureVariableKind::PrivateMethod
        })
    }

    pub(crate) fn initialize_private_accessor_binding(
        &mut self,
        index: u16,
        home_object: Value,
        accessor: Value,
    ) -> Result<(), Error> {
        self.initialize_private_callable_binding(index, home_object, accessor, false, |kind| {
            matches!(
                kind,
                ClosureVariableKind::PrivateGetter
                    | ClosureVariableKind::PrivateSetter
                    | ClosureVariableKind::PrivateGetterSetter
            )
        })
    }

    fn initialize_private_callable_binding(
        &mut self,
        index: u16,
        home_object: Value,
        callable_value: Value,
        infer_name: bool,
        accepts_kind: impl FnOnce(ClosureVariableKind) -> bool,
    ) -> Result<(), Error> {
        let definition = self.local_definition(index)?;
        let binding = self
            .locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("private-callable local index is out of bounds"))?;
        crate::engine::vm::private_bindings::initialize_callable(
            &self.runtime,
            definition,
            binding,
            home_object,
            callable_value,
            infer_name,
            accepts_kind,
        )
    }

    fn private_source_kind(&self, source: PrivateNameSource) -> Result<ClosureVariableKind, Error> {
        match source {
            PrivateNameSource::Local(index) => {
                let (_, kind) = Self::validate_private_definition(self.local_definition(index)?)?;
                Ok(kind)
            }
            PrivateNameSource::Closure(index) => {
                let descriptor = self
                    .executable
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
        use crate::engine::vm::private_bindings::PrivateSource;
        let source = match source {
            PrivateNameSource::Local(index) => PrivateSource::Local(
                self.local_definition(index)?,
                self.locals
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("private-name local index is out of bounds"))?,
            ),
            PrivateNameSource::Closure(index) => PrivateSource::Closure(
                *self
                    .executable
                    .closure_variables
                    .get(usize::from(index))
                    .ok_or_else(|| {
                        Error::internal("private-name closure index is out of bounds")
                    })?,
                self.closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("private-name closure slot is out of bounds"))?,
            ),
        };
        crate::engine::vm::private_bindings::optional_field_name(&self.runtime, source)
    }

    fn optional_private_callable(
        &self,
        source: PrivateNameSource,
        expected_kind: ClosureVariableKind,
    ) -> Result<Option<CallableRef>, Error> {
        use crate::engine::vm::private_bindings::PrivateSource;
        let source = match source {
            PrivateNameSource::Local(index) => PrivateSource::Local(
                self.local_definition(index)?,
                self.locals.get(usize::from(index)).ok_or_else(|| {
                    Error::internal("private-callable local index is out of bounds")
                })?,
            ),
            PrivateNameSource::Closure(index) => PrivateSource::Closure(
                *self
                    .executable
                    .closure_variables
                    .get(usize::from(index))
                    .ok_or_else(|| {
                        Error::internal("private-callable closure index is out of bounds")
                    })?,
                self.closure_slots.get(usize::from(index)).ok_or_else(|| {
                    Error::internal("private-callable closure slot is out of bounds")
                })?,
            ),
        };
        crate::engine::vm::private_bindings::optional_callable(&self.runtime, source, expected_kind)
    }

    fn private_name(&self, source: PrivateNameSource) -> Result<PrivateNameRef, Error> {
        self.optional_private_name(source)?
            .ok_or_else(|| Error::new(ErrorKind::Type, "not a symbol"))
    }

    fn private_source_name(&self, source: PrivateNameSource) -> Result<Option<Atom>, Error> {
        Ok(match source {
            PrivateNameSource::Local(index) => self.local_definition(index)?.name,
            PrivateNameSource::Closure(index) => {
                let descriptor = self
                    .executable
                    .closure_variables
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

    fn branded_private_receiver(
        &self,
        callable: &CallableRef,
        kind: ClosureVariableKind,
        base: Value,
    ) -> Result<ObjectRef, Error> {
        crate::engine::vm::private_bindings::branded_receiver(&self.runtime, callable, kind, base)
    }

    pub(crate) fn get_private_field_value(
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
            kind @ ClosureVariableKind::PrivateMethod => {
                let Some(method) = self.optional_private_callable(source, kind)? else {
                    return Err(Error::new(ErrorKind::Type, "not an object"));
                };
                self.branded_private_receiver(&method, kind, base)?;
                Ok(Completion::Return(Value::Object(
                    method.as_object().clone(),
                )))
            }
            kind @ (ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateGetterSetter) => {
                let Some(getter) = self.optional_private_callable(source, kind)? else {
                    return Err(Error::new(ErrorKind::Type, "not an object"));
                };
                let receiver = self.branded_private_receiver(&getter, kind, base)?;
                self.runtime
                    .call_internal(self.current_realm, &getter, Value::Object(receiver), &[])
                    .map_err(runtime_error_to_vm_error)
            }
            ClosureVariableKind::PrivateSetter => {
                Err(self.lexical_read_only_error(self.private_source_name(source)?)?)
            }
            _ => Err(Error::internal(
                "private get referenced an unsupported binding kind",
            )),
        }
    }

    pub(crate) fn put_private_field_value(
        &mut self,
        source: PrivateNameSource,
        base: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        match self.private_source_kind(source)? {
            ClosureVariableKind::PrivateField => {
                let receiver = Self::private_receiver(base, false)?;
                let name = self.private_name(source)?;
                self.runtime
                    .set_private_field_own(&receiver, &name, value)
                    .map(|()| Completion::Return(Value::Undefined))
                    .map_err(runtime_error_to_vm_error)
            }
            kind @ ClosureVariableKind::PrivateSetter => {
                let Some(setter) = self.optional_private_callable(source, kind)? else {
                    return Err(Error::new(ErrorKind::Type, "not an object"));
                };
                let receiver = self.branded_private_receiver(&setter, kind, base)?;
                match self
                    .runtime
                    .call_internal(
                        self.current_realm,
                        &setter,
                        Value::Object(receiver),
                        &[value],
                    )
                    .map_err(runtime_error_to_vm_error)?
                {
                    Completion::Return(_) => Ok(Completion::Return(Value::Undefined)),
                    Completion::Throw(value) => Ok(Completion::Throw(value)),
                }
            }
            ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateGetterSetter => {
                Err(self.lexical_read_only_error(self.private_source_name(source)?)?)
            }
            _ => Err(Error::internal(
                "private put referenced an unsupported binding kind",
            )),
        }
    }

    pub(crate) fn define_private_field_value(
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

    pub(crate) fn private_in_value(
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
            kind @ (ClosureVariableKind::PrivateMethod
            | ClosureVariableKind::PrivateGetter
            | ClosureVariableKind::PrivateGetterSetter) => {
                let Some(method) = self.optional_private_callable(source, kind)? else {
                    return self
                        .uninitialized_private_in(&receiver)
                        .map(|present| Completion::Return(Value::Bool(present)));
                };
                self.runtime
                    .check_private_method_brand(&method, &receiver, kind)
                    .map_err(runtime_error_to_vm_error)?
            }
            kind @ ClosureVariableKind::PrivateSetter => {
                if self.optional_private_callable(source, kind)?.is_some() {
                    return Err(Error::internal(
                        "private-in referenced a synthetic setter cell",
                    ));
                }
                return self
                    .uninitialized_private_in(&receiver)
                    .map(|present| Completion::Return(Value::Bool(present)));
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
