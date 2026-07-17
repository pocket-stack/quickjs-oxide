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
        Self::evaluate_eval_argument(
            arguments
                .readable
                .first()
                .cloned()
                .unwrap_or(Value::Undefined),
        )
    }

    /// Execute the original-eval branch selected by QuickJS `OP_eval` after
    /// realm-local identity matching. This deliberately bypasses the native
    /// `%eval%` call frame so future String execution can see the bytecode
    /// caller's linked lexical environment.
    pub(in crate::runtime) fn call_direct_eval_original(
        &self,
        input: Value,
    ) -> Result<Completion, RuntimeError> {
        Self::evaluate_eval_argument(input)
    }

    pub(in crate::runtime) fn is_original_eval(
        &self,
        realm: ContextId,
        function: &Value,
    ) -> Result<bool, RuntimeError> {
        if let Value::Object(object) = function {
            if !object.belongs_to(self) {
                return Err(RuntimeError::WrongRuntime("eval function"));
            }
        }
        let original = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .eval_function
            .ok_or(RuntimeError::Invariant(
                "context has no original eval function root",
            ))?;
        Ok(matches!(
            function,
            Value::Object(object) if object.object_id() == original
        ))
    }

    fn evaluate_eval_argument(value: Value) -> Result<Completion, RuntimeError> {
        if matches!(value, Value::String(_)) {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Unsupported,
                "eval source execution is not implemented yet",
            )));
        }
        Ok(Completion::Return(value))
    }
}
