//! Execute published scripts, call functions, and construct objects.

use super::*;

impl Context {
    /// Instantiate and evaluate runtime-owned script bytecode.
    ///
    /// As in QuickJS's `JS_EvalFunctionInternal`, the raw bytecode is first
    /// wrapped in a callable object in the initiating context. The call then
    /// executes in the realm captured by the bytecode.
    pub fn execute(&mut self, function: &FunctionBytecodeRef) -> Result<Value, RuntimeError> {
        let callable = match self.runtime.new_bytecode_closure(self.realm, function) {
            Ok(callable) => callable,
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible declaration error");
                let exception = self
                    .runtime
                    .new_native_error_from_error(self.realm, kind, &error)?;
                self.runtime
                    .ensure_error_backtrace(&exception, false, None)?;
                self.runtime.set_pending_exception(exception)?;
                return Err(RuntimeError::Exception);
            }
            Err(error) => return Err(error),
        };
        let this_value = Value::Object(self.global_object()?);
        self.call(&callable, this_value, &[])
    }

    /// Invoke a validated callable with an explicit `this` value and arguments.
    pub fn call(
        &mut self,
        callable: &CallableRef,
        this_value: Value,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        let completion = self
            .runtime
            .call_internal(self.realm, callable, this_value, arguments)?;
        self.finish_completion(completion)
    }

    /// Invoke a validated constructor with itself as `new.target`, matching
    /// `JS_CallConstructor` and source-level `new`.
    pub fn construct(
        &mut self,
        constructor: &CallableRef,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        self.construct_with_new_target(constructor, constructor, arguments)
    }

    /// Invoke a constructor with an explicit `new.target`, matching
    /// `JS_CallConstructor2`/`Reflect.construct` semantics.
    pub fn construct_with_new_target(
        &mut self,
        constructor: &CallableRef,
        new_target: &CallableRef,
        arguments: &[Value],
    ) -> Result<Value, RuntimeError> {
        match self
            .runtime
            .construct_internal(self.realm, constructor, new_target, arguments)
        {
            Ok(completion) => self.finish_completion(completion),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible native error");
                let exception = self
                    .runtime
                    .new_native_error_from_error(self.realm, kind, &error)?;
                self.runtime
                    .ensure_error_backtrace(&exception, false, None)?;
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            Err(error) => Err(error),
        }
    }
}
