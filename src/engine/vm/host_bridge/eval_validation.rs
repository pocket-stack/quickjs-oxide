//! Eval publication owns topology, names and access modes. Execution validates
//! the caller and actual frame slots before compilation can create captures.
use super::*;

impl RuntimeVmHost {
    pub(super) fn validate_eval_frame_bindings(
        &self,
        environment: &EvalEnvironment<Atom>,
        caller_strict: bool,
    ) -> Result<(), Error> {
        super::super::eval_bindings::validate(
            &self.runtime,
            &self.executable,
            environment,
            caller_strict,
            self.locals.len(),
            self.arguments.len(),
            &self.closure_slots,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::code::function::metadata::{
        EvalBinding, EvalScope, EvalScopeKind, EvalVariableEnvironment,
    };

    #[test]
    fn eval_frame_validation_rejects_missing_slots_and_wrong_cell_metadata() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let key = runtime.intern_property_key("binding").unwrap();
        let mut host = RuntimeVmHost::empty_for_test(runtime.clone(), context.realm);
        let descriptor = ClosureVariable {
            source: ClosureSource::ParentLocal(0),
            name: ClosureVariableName::None,
            is_lexical: true,
            is_const: false,
            kind: ClosureVariableKind::Normal,
        };
        host.executable.closure_variables = Rc::from([descriptor]);
        let mut environment = EvalEnvironment {
            scopes: vec![EvalScope {
                kind: EvalScopeKind::FunctionRoot,
                bindings: vec![EvalBinding {
                    name: key.atom(),
                    source: EvalBindingSource::Closure(0),
                    is_lexical: true,
                    is_const: false,
                    kind: ClosureVariableKind::Normal,
                    is_catch_parameter: false,
                }]
                .into_boxed_slice(),
            }]
            .into_boxed_slice(),
            variable_environment: EvalVariableEnvironment::StrictLocal(0),
            caller_strict: true,
            super_call_allowed: false,
            super_allowed: false,
        };
        assert!(
            host.validate_eval_frame_bindings(&environment, false)
                .is_err()
        );
        assert!(
            host.validate_eval_frame_bindings(&environment, true)
                .is_err()
        );
        host.closure_slots.test_roots_mut().push(
            runtime
                .new_var_ref(Value::Int(1), false, false, ClosureVariableKind::Normal)
                .unwrap(),
        );
        assert!(
            host.validate_eval_frame_bindings(&environment, true)
                .is_err()
        );
        host.closure_slots.test_roots_mut()[0] = runtime
            .new_var_ref(Value::Int(1), true, false, ClosureVariableKind::Normal)
            .unwrap();
        host.validate_eval_frame_bindings(&environment, true)
            .unwrap();
        for source in [EvalBindingSource::Local(0), EvalBindingSource::Argument(0)] {
            environment.scopes[0].bindings[0].source = source;
            assert!(
                host.validate_eval_frame_bindings(&environment, true)
                    .is_err()
            );
        }
    }
}
