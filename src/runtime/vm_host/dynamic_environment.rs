//! Authenticated object-environment operations used by `with` and sloppy eval.
//!
//! The bytecode source operand identifies a hidden local or closure slot; no
//! JavaScript value can nominate an arbitrary object as a dynamic environment.
//! Selection and action remain separate because QuickJS deliberately repeats
//! `HasProperty` after observable `Symbol.unscopables` and RHS evaluation.

use super::{FrameBinding, RuntimeVmHost, read_frame_binding, runtime_error_to_vm_error};
use crate::bytecode::{DynamicEnvironmentSource, WithObjectSource};
use crate::heap::{ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName};
use crate::object::{ObjectRef, PropertyKey, WellKnownSymbol};
use crate::value::Value;
use crate::vm::Completion;
use crate::{Error, ErrorKind};

enum PropertyPresence {
    Present(bool),
    Throw(Value),
}

impl RuntimeVmHost {
    fn with_object(&self, source: WithObjectSource) -> Result<ObjectRef, Error> {
        let value = match source {
            WithObjectSource::Local(index) => {
                let definition = self.local_definition(index)?;
                if definition.kind != ClosureVariableKind::WithObject
                    || definition.is_lexical
                    || definition.is_const
                {
                    return Err(Error::internal(
                        "dynamic with opcode referenced a non-with local",
                    ));
                }
                let binding = self
                    .locals
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("with-object local index is out of bounds"))?;
                if let FrameBinding::Captured(root) = binding {
                    self.runtime
                        .validate_var_ref_metadata(
                            root,
                            ClosureVariable {
                                source: ClosureSource::ParentLocal(index),
                                name: definition
                                    .name
                                    .map_or(ClosureVariableName::None, ClosureVariableName::Atom),
                                is_lexical: definition.is_lexical,
                                is_const: definition.is_const,
                                kind: definition.kind,
                            },
                        )
                        .map_err(runtime_error_to_vm_error)?;
                }
                read_frame_binding(&self.runtime, binding)?
            }
            WithObjectSource::Closure(index) => {
                let descriptor = self
                    .closure_variables
                    .get(usize::from(index))
                    .copied()
                    .ok_or_else(|| Error::internal("with-object closure index is out of bounds"))?;
                if descriptor.kind != ClosureVariableKind::WithObject
                    || descriptor.is_lexical
                    || descriptor.is_const
                {
                    return Err(Error::internal(
                        "dynamic with opcode referenced a non-with closure",
                    ));
                }
                let root = self
                    .closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("with-object closure slot is out of bounds"))?;
                self.runtime
                    .validate_var_ref_metadata(root, descriptor)
                    .map_err(runtime_error_to_vm_error)?;
                self.runtime
                    .read_var_ref(root)
                    .map_err(runtime_error_to_vm_error)?
            }
        };
        let Value::Object(object) = value else {
            return Err(Error::internal(
                "with-object binding did not contain an Object",
            ));
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal("with object belongs to another runtime"));
        }
        Ok(object)
    }

    fn dynamic_object(&self, source: DynamicEnvironmentSource) -> Result<ObjectRef, Error> {
        match source {
            DynamicEnvironmentSource::Eval(source) => self.eval_variable_object(source),
            DynamicEnvironmentSource::With(source) => self.with_object(source),
        }
    }

    fn property_presence(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<PropertyPresence, Error> {
        match self
            .runtime
            .has_property_in_realm(self.current_realm, object, key)
            .map_err(runtime_error_to_vm_error)?
        {
            Completion::Return(Value::Bool(present)) => Ok(PropertyPresence::Present(present)),
            Completion::Return(_) => Err(Error::internal(
                "HasProperty returned a non-Boolean completion",
            )),
            Completion::Throw(value) => Ok(PropertyPresence::Throw(value)),
        }
    }

    fn reference_not_defined(&self, key: &PropertyKey) -> Result<Error, Error> {
        self.runtime
            .native_atom_error(ErrorKind::Reference, "'", key, "' is not defined")
            .map_err(runtime_error_to_vm_error)
    }

    pub(super) fn has_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        match self.property_presence(&object, &key)? {
            PropertyPresence::Present(false) => {
                return Ok(Completion::Return(Value::Bool(false)));
            }
            PropertyPresence::Throw(value) => return Ok(Completion::Throw(value)),
            PropertyPresence::Present(true) => {}
        }

        if matches!(source, DynamicEnvironmentSource::With(_)) {
            let unscopables_key =
                PropertyKey::from(self.runtime.well_known_symbol(WellKnownSymbol::Unscopables));
            let unscopables =
                match self.get_property_with_key(Value::Object(object), &unscopables_key, true)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
            if let Value::Object(unscopables) = unscopables {
                let excluded =
                    match self.get_property_with_key(Value::Object(unscopables), &key, true)? {
                        Completion::Return(value) => value.to_boolean(),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                if excluded {
                    return Ok(Completion::Return(Value::Bool(false)));
                }
            }
        }
        Ok(Completion::Return(Value::Bool(true)))
    }

    pub(super) fn get_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        match self.property_presence(&object, &key)? {
            PropertyPresence::Present(true) => {
                self.get_property_with_key(Value::Object(object), &key, true)
            }
            PropertyPresence::Present(false) if strict => Err(self.reference_not_defined(&key)?),
            PropertyPresence::Present(false) => Ok(Completion::Return(Value::Undefined)),
            PropertyPresence::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    pub(super) fn put_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        match self.property_presence(&object, &key)? {
            PropertyPresence::Present(false) if strict => {
                return Err(self.reference_not_defined(&key)?);
            }
            PropertyPresence::Throw(value) => return Ok(Completion::Throw(value)),
            PropertyPresence::Present(_) => {}
        }
        // QuickJS uses JS_PROP_THROW_STRICT for both with_put_var and
        // put_ref_value, independently from the frame's missing-binding mode.
        self.set_property_with_key(Value::Object(object), &key, value, true)
    }

    pub(super) fn delete_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        self.delete_property_with_key(Value::Object(object), &key, false)
    }

    pub(super) fn dynamic_environment_object_impl(
        &mut self,
        source: DynamicEnvironmentSource,
    ) -> Result<Completion, Error> {
        self.dynamic_object(source)
            .map(|object| Completion::Return(Value::Object(object)))
    }

    pub(super) fn get_ref_value_impl(
        &mut self,
        environment: Value,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = environment else {
            return Err(Error::internal(
                "dynamic reference base did not contain an Object",
            ));
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "dynamic reference base belongs to another runtime",
            ));
        }
        let key = self.constant_property_key(name)?;
        match self.property_presence(&object, &key)? {
            PropertyPresence::Present(true) => {
                self.get_property_with_key(Value::Object(object), &key, true)
            }
            PropertyPresence::Present(false) if strict => Err(self.reference_not_defined(&key)?),
            PropertyPresence::Present(false) => Ok(Completion::Return(Value::Undefined)),
            PropertyPresence::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    pub(super) fn put_ref_value_impl(
        &mut self,
        environment: Value,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        let Value::Object(object) = environment else {
            return Err(Error::internal(
                "dynamic reference base did not contain an Object",
            ));
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "dynamic reference base belongs to another runtime",
            ));
        }
        let key = self.constant_property_key(name)?;
        match self.property_presence(&object, &key)? {
            PropertyPresence::Present(false) if strict => {
                return Err(self.reference_not_defined(&key)?);
            }
            PropertyPresence::Throw(value) => return Ok(Completion::Throw(value)),
            PropertyPresence::Present(_) => {}
        }
        self.set_property_with_key(Value::Object(object), &key, value, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Runtime;
    use crate::atom::Atom;
    use crate::bytecode::EvalVariableSource;
    use crate::heap::{BytecodeConstant, RawValue, VariableDefinition};
    use crate::value::JsString;
    use std::rc::Rc;

    fn host_with_local(
        runtime: Runtime,
        realm: crate::heap::ContextId,
        object: ObjectRef,
        kind: ClosureVariableKind,
        names: &[&'static str],
    ) -> RuntimeVmHost {
        let mut host = RuntimeVmHost::empty_for_test(runtime, realm);
        host.constants = names
            .iter()
            .map(|name| BytecodeConstant::Value(RawValue::String(JsString::from_static(name))))
            .collect::<Vec<_>>()
            .into();
        host.local_definitions = Rc::from([VariableDefinition {
            name: Some(Atom::from_raw(71)),
            is_lexical: false,
            is_const: false,
            kind,
        }]);
        host.locals = vec![FrameBinding::Direct(Value::Object(object))];
        host.reusable_captured_locals = vec![false];
        host
    }

    #[test]
    fn dynamic_environment_selection_actions_refs_and_inherited_eval_match_quickjs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(with_object) = context
            .eval("({visible:42,hidden:7,[Symbol.unscopables]:{hidden:true}})")
            .unwrap()
        else {
            panic!("with fixture was not an Object");
        };
        let names = ["visible", "hidden", "missing", "inherited"];
        let mut with_host = host_with_local(
            runtime.clone(),
            context.realm,
            with_object.clone(),
            ClosureVariableKind::WithObject,
            &names,
        );
        let source = DynamicEnvironmentSource::With(WithObjectSource::Local(0));

        assert_eq!(
            with_host.has_dynamic_binding_impl(source, 0).unwrap(),
            Completion::Return(Value::Bool(true))
        );
        assert_eq!(
            with_host.has_dynamic_binding_impl(source, 1).unwrap(),
            Completion::Return(Value::Bool(false))
        );
        assert_eq!(
            with_host
                .get_dynamic_binding_impl(source, 0, false)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(
            with_host
                .get_dynamic_binding_impl(source, 2, false)
                .unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            with_host
                .get_dynamic_binding_impl(source, 2, true)
                .unwrap_err()
                .kind(),
            ErrorKind::Reference
        );
        assert_eq!(
            with_host
                .get_ref_value_impl(Value::Object(with_object.clone()), 0, false)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(
            with_host
                .put_ref_value_impl(Value::Object(with_object.clone()), 0, Value::Int(43), false,)
                .unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            with_host
                .get_dynamic_binding_impl(source, 0, false)
                .unwrap(),
            Completion::Return(Value::Int(43))
        );

        let eval_object = runtime.new_object(None).unwrap();
        let Value::Object(prototype) = context.eval("({inherited:42})").unwrap() else {
            panic!("prototype fixture was not an Object");
        };
        assert!(
            runtime
                .set_prototype_of(&eval_object, Some(&prototype))
                .unwrap()
        );
        let mut eval_host = host_with_local(
            runtime,
            context.realm,
            eval_object,
            ClosureVariableKind::EvalVariableObject,
            &names,
        );
        eval_host.eval_variable_object_local = Some(0);
        let source = DynamicEnvironmentSource::Eval(EvalVariableSource::Local(0));
        assert_eq!(
            eval_host.has_dynamic_binding_impl(source, 3).unwrap(),
            Completion::Return(Value::Bool(true))
        );
        assert_eq!(
            eval_host
                .get_dynamic_binding_impl(source, 3, false)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
    }
}
