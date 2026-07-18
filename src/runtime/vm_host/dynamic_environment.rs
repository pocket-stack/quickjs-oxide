//! Authenticated object-environment operations used by `with` and sloppy eval.
//!
//! The bytecode source operand identifies a hidden local or closure slot; no
//! JavaScript value can nominate an arbitrary object as a dynamic environment.
//! Selection and action remain separate because QuickJS deliberately repeats
//! `HasProperty` after observable `Symbol.unscopables` and RHS evaluation.

use super::{FrameBinding, RuntimeVmHost, read_frame_binding, runtime_error_to_vm_error};
use crate::bytecode::{DynamicEnvironmentSource, WithObjectSource};
use crate::heap::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, RawValue,
};
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
        // QuickJS forwards JS_PROP_THROW_STRICT for both with_put_var and
        // put_ref_value only from strict code; sloppy Set rejection is silent.
        self.set_property_with_key(Value::Object(object), &key, value, strict)
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

    /// Resolve QuickJS `OP_make_var_ref` against the current realm rather
    /// than trusting the closure slot's historical global resolution. A later
    /// script can install a same-name global lexical binding after this
    /// bytecode was published, and that live lexical VarRef must win.
    pub(super) fn global_reference_impl(&mut self, index: u16) -> Result<Completion, Error> {
        let descriptor = self
            .closure_variables
            .get(usize::from(index))
            .copied()
            .ok_or_else(|| Error::internal("global reference closure index is out of bounds"))?;
        if !matches!(
            descriptor.source,
            ClosureSource::GlobalDeclaration
                | ClosureSource::Global
                | ClosureSource::ParentGlobal(_)
        ) || !matches!(
            descriptor.kind,
            ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
        ) {
            return Err(Error::internal(
                "global reference opcode referenced a non-global closure",
            ));
        }
        let ClosureVariableName::Atom(atom) = descriptor.name else {
            return Err(Error::internal(
                "published global reference descriptor has no name atom",
            ));
        };
        let root = self
            .closure_slots
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("global reference closure slot is out of bounds"))?;
        if !root.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "global reference closure belongs to another runtime",
            ));
        }

        let key = PropertyKey::from_borrowed_atom(self.runtime.clone(), atom)
            .map_err(|error| Error::internal(error.to_string()))?;
        let global_var_object = {
            let state = self.runtime.0.state.borrow();
            state
                .heap
                .context(self.current_realm)
                .map_err(|error| Error::internal(error.to_string()))?
                .global_var_object
        };
        let global_var_object =
            ObjectRef::from_borrowed_handle(self.runtime.clone(), global_var_object)
                .map_err(|error| Error::internal(error.to_string()))?;
        if let Some(root) = self
            .runtime
            .own_var_ref_root(&global_var_object, &key)
            .map_err(runtime_error_to_vm_error)?
        {
            let cell = self
                .runtime
                .0
                .state
                .borrow()
                .heap
                .var_ref(root.id())
                .map_err(|error| Error::internal(error.to_string()))?
                .clone();
            if !cell.is_lexical || cell.kind != ClosureVariableKind::Normal {
                return Err(Error::internal(
                    "global lexical object contained a non-lexical VarRef",
                ));
            }
            if matches!(cell.value, RawValue::Uninitialized) {
                return Err(self.lexical_uninitialized_error(Some(atom))?);
            }
            if cell.is_const {
                return Err(self.lexical_read_only_error(Some(atom))?);
            }
            return Ok(Completion::Return(Value::Object(global_var_object)));
        }

        let global_object = self
            .runtime
            .global_object_for_realm(self.current_realm)
            .map_err(runtime_error_to_vm_error)?;
        match self.property_presence(&global_object, &key)? {
            PropertyPresence::Present(true) => Ok(Completion::Return(Value::Object(global_object))),
            PropertyPresence::Present(false) => Ok(Completion::Return(Value::Undefined)),
            PropertyPresence::Throw(value) => Ok(Completion::Throw(value)),
        }
    }

    pub(super) fn get_ref_value_impl(
        &mut self,
        environment: Value,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        let key = self.constant_property_key(name)?;
        let object = match environment {
            Value::Object(object) => object,
            Value::Undefined => return Err(self.reference_not_defined(&key)?),
            _ => {
                return Err(Error::internal(
                    "dynamic reference base was neither an Object nor undefined",
                ));
            }
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "dynamic reference base belongs to another runtime",
            ));
        }
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
        let key = self.constant_property_key(name)?;
        let object = match environment {
            Value::Object(object) => object,
            Value::Undefined if strict => return Err(self.reference_not_defined(&key)?),
            Value::Undefined => self
                .runtime
                .global_object_for_realm(self.current_realm)
                .map_err(runtime_error_to_vm_error)?,
            _ => {
                return Err(Error::internal(
                    "dynamic reference base was neither an Object nor undefined",
                ));
            }
        };
        if !object.belongs_to(&self.runtime) {
            return Err(Error::internal(
                "dynamic reference base belongs to another runtime",
            ));
        }
        match self.property_presence(&object, &key)? {
            PropertyPresence::Present(false) if strict => {
                return Err(self.reference_not_defined(&key)?);
            }
            PropertyPresence::Throw(value) => return Ok(Completion::Throw(value)),
            PropertyPresence::Present(_) => {}
        }
        // The realm's lexical storage object is structurally ordinary in
        // this rewrite, but its slots still carry VarRefs. A generic ordinary
        // DefineOwnProperty write would replace that slot with plain data and
        // sever every compiled closure. QuickJS writes through the VarRef
        // property, so preserve the shared cell explicitly after the required
        // repeated HasProperty step.
        if let Some(root) = self
            .runtime
            .own_var_ref_root(&object, &key)
            .map_err(runtime_error_to_vm_error)?
        {
            let cell = self
                .runtime
                .0
                .state
                .borrow()
                .heap
                .var_ref(root.id())
                .map_err(|error| Error::internal(error.to_string()))?
                .clone();
            if matches!(cell.value, RawValue::Uninitialized) {
                return Err(self.lexical_uninitialized_error(Some(key.atom()))?);
            }
            if cell.is_const {
                return if strict {
                    Err(self.lexical_read_only_error(Some(key.atom()))?)
                } else {
                    Ok(Completion::Return(Value::Undefined))
                };
            }
            self.runtime
                .write_var_ref(&root, value)
                .map_err(runtime_error_to_vm_error)?;
            return Ok(Completion::Return(Value::Undefined));
        }
        self.set_property_with_key(Value::Object(object), &key, value, strict)
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

    fn host_with_global_name(
        runtime: &Runtime,
        realm: crate::heap::ContextId,
        name: &'static str,
    ) -> RuntimeVmHost {
        let key = runtime.intern_property_key(name).unwrap();
        let root = runtime.resolve_global_var(realm, key.atom()).unwrap();
        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), realm);
        host.constants = Rc::from([BytecodeConstant::Value(RawValue::String(
            JsString::from_static(name),
        ))]);
        host.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Atom(key.atom()),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }]);
        host.closure_slots = vec![root];
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

    #[test]
    fn global_reference_uses_live_lexical_storage_and_checks_it_before_access() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        context
            .create_global_lexical_for_test("mutableLexical", false, Some(Value::Int(41)))
            .unwrap();
        context
            .create_global_lexical_for_test("readonlyLexical", true, Some(Value::Int(7)))
            .unwrap();
        context
            .create_global_lexical_for_test("tdzLexical", false, None)
            .unwrap();

        let mut mutable = host_with_global_name(&runtime, context.realm, "mutableLexical");
        let Completion::Return(Value::Object(base)) = mutable.global_reference_impl(0).unwrap()
        else {
            panic!("mutable global lexical did not resolve to an Object");
        };
        assert_eq!(base, context.global_var_object().unwrap());
        assert_eq!(
            mutable
                .get_ref_value_impl(Value::Object(base.clone()), 0, false)
                .unwrap(),
            Completion::Return(Value::Int(41))
        );
        assert_eq!(
            mutable
                .put_ref_value_impl(Value::Object(base), 0, Value::Int(42), false)
                .unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            mutable
                .global_reference_impl(0)
                .and_then(|completion| match completion {
                    Completion::Return(base) => mutable.get_ref_value_impl(base, 0, false),
                    Completion::Throw(value) => Ok(Completion::Throw(value)),
                })
                .unwrap(),
            Completion::Return(Value::Int(42))
        );

        let mut readonly = host_with_global_name(&runtime, context.realm, "readonlyLexical");
        assert_eq!(
            readonly.global_reference_impl(0).unwrap_err().kind(),
            ErrorKind::Type
        );

        let mut tdz = host_with_global_name(&runtime, context.realm, "tdzLexical");
        assert_eq!(
            tdz.global_reference_impl(0).unwrap_err().kind(),
            ErrorKind::Reference
        );
    }

    #[test]
    fn unresolved_global_reference_reads_throw_and_sloppy_writes_create_a_global() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let mut host = host_with_global_name(&runtime, context.realm, "createdByReference");

        assert_eq!(
            host.global_reference_impl(0).unwrap(),
            Completion::Return(Value::Undefined)
        );
        for strict in [false, true] {
            assert_eq!(
                host.get_ref_value_impl(Value::Undefined, 0, strict)
                    .unwrap_err()
                    .kind(),
                ErrorKind::Reference
            );
        }
        assert_eq!(
            host.put_ref_value_impl(Value::Undefined, 0, Value::Int(7), true)
                .unwrap_err()
                .kind(),
            ErrorKind::Reference
        );

        assert_eq!(
            host.put_ref_value_impl(Value::Undefined, 0, Value::Int(42), false)
                .unwrap(),
            Completion::Return(Value::Undefined)
        );
        let key = runtime.intern_property_key("createdByReference").unwrap();
        let global = context.global_object().unwrap();
        assert_eq!(context.get_property(&global, &key).unwrap(), Value::Int(42));
        assert_eq!(
            host.global_reference_impl(0).unwrap(),
            Completion::Return(Value::Object(global))
        );
    }
}
