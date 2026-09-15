//! Authenticated object-environment operations used by `with` and sloppy eval.
//!
//! The bytecode source operand identifies a hidden local or closure slot; no
//! JavaScript value can nominate an arbitrary object as a dynamic environment.
//! Selection and action remain separate because QuickJS deliberately repeats
//! `HasProperty` after observable `Symbol.unscopables` and RHS evaluation.

#[cfg(test)]
use super::FrameBinding;
use super::{RuntimeVmHost, runtime_error_to_vm_error};
use crate::engine::api::{Error, ErrorKind};
use crate::engine::code::bytecode::{DynamicEnvironmentSource, WithObjectSource};
#[cfg(test)]
use crate::engine::code::function::metadata::ClosureVariable;
#[cfg(test)]
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, ClosureVariableName,
};
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::Value;
use crate::engine::vm::Completion;
use crate::engine::vm::environment_bindings::operation::{self, EnvironmentStep};

impl RuntimeVmHost {
    fn with_object(&self, source: WithObjectSource) -> Result<ObjectRef, Error> {
        crate::engine::vm::environment_bindings::with_object(
            &self.runtime,
            &self.executable,
            source,
            |index| self.locals.get(usize::from(index)),
            &self.closure_slots,
        )
    }

    fn dynamic_object(&self, source: DynamicEnvironmentSource) -> Result<ObjectRef, Error> {
        match source {
            DynamicEnvironmentSource::Eval(source) => self.eval_variable_object(source),
            DynamicEnvironmentSource::With(source) => self.with_object(source),
        }
    }

    fn reference_not_defined(&self, key: &PropertyKey) -> Result<Error, Error> {
        self.runtime
            .native_atom_error(ErrorKind::Reference, "'", key, "' is not defined")
            .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn has_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::has_binding(
                self.current_realm,
                object,
                key,
                matches!(source, DynamicEnvironmentSource::With(_)),
            ),
        )
        .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn get_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::get(self.current_realm, object, key, strict),
        )
        .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn put_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::put(self.current_realm, object, key, value, strict, false),
        )
        .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn delete_dynamic_binding_impl(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        let object = self.dynamic_object(source)?;
        let key = self.constant_property_key(name)?;
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::delete(self.current_realm, object, key),
        )
        .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn dynamic_environment_object_impl(
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
    pub(crate) fn global_reference_impl(&mut self, index: u16) -> Result<Completion, Error> {
        let (global_object, key) = match super::super::environment_bindings::global_reference(
            &self.runtime,
            self.current_realm,
            &self.executable,
            &self.closure_slots,
            index,
        )? {
            super::super::environment_bindings::GlobalReference::Lexical(object) => {
                return Ok(Completion::Return(Value::Object(object)));
            }
            super::super::environment_bindings::GlobalReference::Object { object, key } => {
                (object, key)
            }
        };
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::reference(self.current_realm, global_object, key),
        )
        .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn get_ref_value_impl(
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
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::get(self.current_realm, object, key, strict),
        )
        .map_err(runtime_error_to_vm_error)
    }

    pub(crate) fn put_ref_value_impl(
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
        operation::finish(
            &self.runtime,
            self.current_realm,
            EnvironmentStep::put(self.current_realm, object, key, value, strict, true),
        )
        .map_err(runtime_error_to_vm_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::Runtime;
    use crate::engine::atom::Atom;
    use crate::engine::code::bytecode::EvalVariableSource;
    use crate::engine::code::function::metadata::VariableDefinition;

    use crate::engine::heap::{BytecodeConstant, RawValue};
    use crate::engine::value::JsString;
    use std::rc::Rc;

    fn host_with_local(
        runtime: Runtime,
        realm: crate::engine::heap::ContextId,
        object: ObjectRef,
        kind: ClosureVariableKind,
        names: &[&'static str],
    ) -> RuntimeVmHost {
        let mut host = RuntimeVmHost::empty_for_test(runtime, realm);
        host.executable.constants = names
            .iter()
            .map(|name| BytecodeConstant::Value(RawValue::String(JsString::from_static(name))))
            .collect::<Vec<_>>()
            .into();
        host.executable.local_definitions = Rc::from([VariableDefinition {
            name: Some(Atom::from_raw(71)),
            is_lexical: false,
            is_const: false,
            is_parameter_initializer: false,
            kind,
        }]);
        host.locals = vec![FrameBinding::Direct(Value::Object(object))];
        host.reusable_captured_locals = vec![false];
        host
    }

    fn host_with_global_name(
        runtime: &Runtime,
        realm: crate::engine::heap::ContextId,
        name: &'static str,
    ) -> RuntimeVmHost {
        let key = runtime.intern_property_key(name).unwrap();
        let root = runtime.resolve_global_var(realm, key.atom()).unwrap();
        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), realm);
        host.executable.constants = Rc::from([BytecodeConstant::Value(RawValue::String(
            JsString::from_static(name),
        ))]);
        host.executable.closure_variables = Rc::from([ClosureVariable {
            source: ClosureSource::Global,
            name: ClosureVariableName::Atom(key.atom()),
            is_lexical: false,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        }]);
        host.closure_slots = vec![root].into();
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
        eval_host.executable.metadata.eval_variable_object_local = Some(0);
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
