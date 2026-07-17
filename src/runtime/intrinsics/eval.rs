use super::*;

impl Runtime {
    pub(in crate::runtime) fn initialize_eval_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let function = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::GlobalEval,
            1,
            "eval",
            1,
        )?;
        self.define_function_data_property(
            global_object,
            "eval",
            Value::Object(function.as_object().clone()),
            true,
            true,
        )?;
        self.0
            .state
            .borrow_mut()
            .heap
            .attach_eval_function(realm, function.as_object().object_id())?;
        Ok(())
    }

    pub(in crate::runtime) fn call_global_eval(
        &self,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "global eval used an unexpected native invocation protocol",
            ));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .unwrap_or(Value::Undefined);
        if matches!(value, Value::String(_)) {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Unsupported,
                "eval source execution is not implemented yet",
            )));
        }
        Ok(Completion::Return(value))
    }
}
