use super::*;

/// One realm and its execution state.
/// Execution-only eval options. Compilation metadata remains in
/// [`CompileOptions`]; the barrier mirrors QuickJS
/// `JS_EVAL_FLAG_BACKTRACE_BARRIER` and temporarily marks the caller frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalOptions {
    pub filename: String,
    pub backtrace_barrier: bool,
}

impl EvalOptions {
    #[must_use]
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
            backtrace_barrier: false,
        }
    }
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self::new(crate::compiler::DEFAULT_EVAL_FILENAME)
    }
}

pub struct Context {
    pub(in crate::runtime) runtime: Runtime,
    pub(in crate::runtime) id: u64,
    pub(in crate::runtime) realm: ContextId,
}

impl Clone for Context {
    fn clone(&self) -> Self {
        self.runtime
            .retain_context_handle(self.realm)
            .expect("a live Context handle must retain its realm");
        Self {
            runtime: self.runtime.clone(),
            id: self.id,
            realm: self.realm,
        }
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        self.runtime.release_context_handle(self.realm);
    }
}

impl Context {
    #[must_use]
    pub const fn id(&self) -> u64 {
        self.id
    }

    /// Return the stable arena identity used by runtime jobs and host hooks.
    #[must_use]
    pub const fn realm_id(&self) -> ContextId {
        self.realm
    }

    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Return this realm's `%Object.prototype%` root.
    pub fn object_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .object_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's genuine empty `%Array.prototype%` root.
    pub fn array_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .array_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%Function.prototype%` root.
    pub fn function_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%IteratorPrototype%` root beneath the public
    /// `Iterator`, Iterator Helpers, and `Iterator.concat` intrinsic graph.
    pub fn iterator_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .iterator_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%StringIteratorPrototype%` root.
    pub fn string_iterator_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .string_iterator_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's boxed-+0 `%Number.prototype%` root.
    pub fn number_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Number)
    }

    /// Return this realm's boxed-false `%Boolean.prototype%` root.
    pub fn boolean_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Boolean)
    }

    /// Return this realm's branded-empty partial `%String.prototype%` root.
    pub fn string_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::String)
    }

    /// Return this realm's ordinary `%Symbol.prototype%` root.
    pub fn symbol_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Symbol)
    }

    /// Return this realm's ordinary `%BigInt.prototype%` root.
    pub fn bigint_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::BigInt)
    }

    /// Return this realm's `%Function%` constructor root.
    pub fn function_constructor(&self) -> Result<CallableRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant("realm has no Function constructor"))?;
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_borrowed_handle(self.runtime.clone(), object)?,
        ))
    }

    /// Return this realm's global object root.
    pub fn global_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return the null-prototype object used for global lexical bindings.
    pub fn global_var_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_var_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    #[cfg(test)]
    pub(crate) fn create_global_lexical_for_test(
        &self,
        name: &str,
        is_const: bool,
        initial_value: Option<Value>,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .create_global_lexical_for_test(self.realm, name, is_const, initial_value)
    }

    #[cfg(test)]
    pub(crate) fn initialize_global_lexical_for_test(
        &self,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        self.runtime
            .initialize_global_lexical_for_test(self.realm, name, value)
    }

    /// Allocate an ordinary object with this realm's `%Object.prototype%`.
    pub fn new_object(&mut self) -> Result<ObjectRef, RuntimeError> {
        let prototype = self.object_prototype()?;
        self.runtime.new_object(Some(&prototype))
    }

    /// Create QuickJS's test262-only native `codePointRange` helper in this
    /// context's realm.
    ///
    /// The helper is intentionally not installed as an ECMAScript intrinsic;
    /// embedders such as the Test262 runner decide where to publish it.
    #[cfg(feature = "test262-host")]
    pub fn new_code_point_range_function(&mut self) -> Result<CallableRef, RuntimeError> {
        let function_prototype = self.function_prototype()?;
        self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::StringCodePointRange,
            2,
            "codePointRange",
            2,
        )
    }

    /// Create QuickJS's test262-only `$262.gc` host function.
    ///
    /// The function is not an ECMAScript intrinsic. Embedders choose whether
    /// and where to publish it.
    #[cfg(feature = "test262-host")]
    pub fn new_test262_gc_function(&mut self) -> Result<CallableRef, RuntimeError> {
        let function_prototype = self.function_prototype()?;
        self.runtime.new_native_builtin(
            &function_prototype,
            self.realm,
            NativeFunctionId::Test262Gc,
            0,
            "gc",
            0,
        )
    }

    /// Allocate one genuine empty Array in this realm.
    pub fn new_array(&mut self) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_array(self.realm)
    }

    /// Allocate one genuine Array initialized from consecutive values.
    pub fn new_array_from_values(&mut self, values: Vec<Value>) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_array_from_values(self.realm, values)
    }

    /// Allocate an ordinary object with an explicit object-or-null prototype.
    pub fn new_object_with_prototype(
        &mut self,
        prototype: Option<&ObjectRef>,
    ) -> Result<ObjectRef, RuntimeError> {
        self.runtime.new_object(prototype)
    }

    pub fn get_own_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Option<CompleteOrdinaryPropertyDescriptor>, RuntimeError> {
        match self
            .runtime
            .internal_get_own_property(self.realm, object, key)?
        {
            NativeConversion::Value(value) => Ok(value),
            NativeConversion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    pub fn define_own_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<bool, RuntimeError> {
        match self
            .runtime
            .internal_define_own_property(self.realm, object, key, descriptor)?
        {
            NativeConversion::Value(InternalDefineResult::Defined) => Ok(true),
            NativeConversion::Value(
                InternalDefineResult::RejectedOrdinary(_) | InternalDefineResult::RejectedProxyTrap,
            ) => Ok(false),
            NativeConversion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    pub fn get_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
    ) -> Result<Value, RuntimeError> {
        let completion =
            self.runtime
                .internal_get(self.realm, object, key, Value::Object(object.clone()))?;
        self.finish_completion(completion)
    }

    pub fn get_property_with_receiver(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        receiver: Value,
    ) -> Result<Value, RuntimeError> {
        let completion = self
            .runtime
            .internal_get(self.realm, object, key, receiver)?;
        self.finish_completion(completion)
    }

    pub fn set_property(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<bool, RuntimeError> {
        match self.runtime.internal_set(
            self.realm,
            object,
            key,
            value,
            Value::Object(object.clone()),
        )? {
            NativeConversion::Value(InternalSetResult::Accepted) => Ok(true),
            NativeConversion::Value(
                InternalSetResult::Rejected(_) | InternalSetResult::RejectedProxyTrap,
            ) => Ok(false),
            NativeConversion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    pub fn set_property_with_receiver(
        &mut self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
        receiver: Value,
    ) -> Result<bool, RuntimeError> {
        match self
            .runtime
            .internal_set(self.realm, object, key, value, receiver)?
        {
            NativeConversion::Value(InternalSetResult::Accepted) => Ok(true),
            NativeConversion::Value(
                InternalSetResult::Rejected(_) | InternalSetResult::RejectedProxyTrap,
            ) => Ok(false),
            NativeConversion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Compile one script and publish its immutable bytecode in this realm.
    ///
    /// The returned handle is a runtime root. Its constant pool and captured
    /// realm remain alive even if this particular `Context` handle is dropped.
    pub fn compile(&mut self, source: &str) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::default())
    }

    /// Compile one explicitly sized source buffer. Full debug mode retains
    /// byte-exact authored ranges for nested functions.
    pub fn compile_bytes(&mut self, source: &[u8]) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_bytes_with_options(source, &CompileOptions::default())
    }

    /// Compile one script with an explicit filename attached independently to
    /// every published function's debug metadata.
    pub fn compile_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one explicitly sized source buffer with an explicit debug
    /// filename.
    pub fn compile_bytes_with_filename(
        &mut self,
        source: &[u8],
        filename: &str,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        self.compile_bytes_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one script with named compilation options.
    ///
    /// Implemented JavaScript early errors become pending exceptions. Grammar
    /// which is not implemented remains an engine [`ErrorKind::Unsupported`]
    /// diagnostic so embedders and conformance tooling observe the same
    /// frontier.
    pub fn compile_with_options(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let compilation = self
            .runtime
            .compile_in_realm(self.realm, source, &options.filename)?;
        self.finish_compilation(compilation)
    }

    /// Compile one explicitly sized source buffer with named compilation
    /// options.
    ///
    /// Source bytes are parsed with QuickJS-compatible UTF-8/WTF-8 handling;
    /// malformed bytes remain observable in permitted source regions and
    /// produce syntax errors where the grammar requires source characters.
    pub fn compile_bytes_with_options(
        &mut self,
        source: &[u8],
        options: &CompileOptions,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let compilation =
            self.runtime
                .compile_bytes_in_realm(self.realm, source, &options.filename)?;
        self.finish_compilation(compilation)
    }

    /// Read one trusted QuickJS 2026-06-04 BC5 scalar Script.
    ///
    /// This deliberately narrow compatibility entry point is not a general
    /// untrusted-bytecode sandbox. It accepts only the currently admitted
    /// branch-free scalar Script cohort, translates it to typed engine
    /// instructions, and runs the ordinary verifier before publication.
    /// Well-formed QuickJS bytecode outside that cohort returns
    /// [`ErrorKind::Unsupported`] without creating a pending exception.
    pub fn read_trusted_scalar_script(
        &mut self,
        bytes: &[u8],
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let result = self
            .runtime
            .read_trusted_scalar_script_in_realm(self.realm, bytes);
        self.finish_trusted_bytecode_read(result)
    }

    /// Read one ordinary synchronous leaf from a trusted QuickJS 2026-06-04
    /// BC5 compile-only image.
    ///
    /// `root_constant_index` selects a FunctionBytecode child from the
    /// authenticated root function's constant pool. The selected child must
    /// satisfy the current ordinary-leaf capability profile; its complete
    /// typed CFG is verified before transactional publication. Well-formed
    /// bytecode outside that profile returns [`ErrorKind::Unsupported`]
    /// without creating a pending exception.
    pub fn read_trusted_ordinary_function(
        &mut self,
        bytes: &[u8],
        root_constant_index: u32,
    ) -> Result<CallableRef, RuntimeError> {
        let result = self.runtime.read_trusted_ordinary_function_in_realm(
            self.realm,
            bytes,
            root_constant_index,
        );
        self.finish_trusted_bytecode_read(result)
    }

    fn finish_trusted_bytecode_read<T>(
        &mut self,
        result: Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        match result {
            Ok(function) => Ok(function),
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind())
                    .expect("guard proved this is a JavaScript-visible bytecode read error");
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

    fn finish_compilation(
        &mut self,
        compilation: Compilation,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        match compilation {
            Compilation::Published(function) => Ok(function),
            Compilation::Throw(exception) => {
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

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

    fn finish_completion(&mut self, completion: Completion) -> Result<Value, RuntimeError> {
        match completion {
            Completion::Return(value) => Ok(value),
            Completion::Throw(value) => {
                self.runtime.set_pending_exception(value)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Return whether this runtime currently carries a pending JavaScript
    /// exception completion.
    #[must_use]
    pub fn has_exception(&self) -> bool {
        self.runtime.has_pending_exception()
    }

    /// Move the pending JavaScript exception value out of the runtime slot.
    pub fn take_exception(&mut self) -> Result<Option<Value>, RuntimeError> {
        self.runtime.take_pending_exception()
    }

    /// Compile and evaluate one script through runtime-owned bytecode.
    ///
    /// # Errors
    /// Returns syntax, publication, runtime-domain, or execution errors.
    pub fn eval(&mut self, source: &str) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::default())
    }

    /// Compile and evaluate one explicitly sized source buffer.
    pub fn eval_bytes(&mut self, source: &[u8]) -> Result<Value, RuntimeError> {
        self.eval_bytes_with_options(source, &EvalOptions::default())
    }

    /// Compile and evaluate a script with an explicit debug filename.
    pub fn eval_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<Value, RuntimeError> {
        self.eval_with_options(source, &EvalOptions::new(filename))
    }

    /// Compile and evaluate one explicitly sized source buffer with an
    /// explicit debug filename.
    pub fn eval_bytes_with_filename(
        &mut self,
        source: &[u8],
        filename: &str,
    ) -> Result<Value, RuntimeError> {
        self.eval_bytes_with_options(source, &EvalOptions::new(filename))
    }

    /// Compile and evaluate a script with filename and execution options.
    pub fn eval_with_options(
        &mut self,
        source: &str,
        options: &EvalOptions,
    ) -> Result<Value, RuntimeError> {
        self.eval_compiling_with_options(options, |context, compile_options| {
            context.compile_with_options(source, compile_options)
        })
    }

    /// Compile and evaluate one explicitly sized source buffer with filename
    /// and execution options.
    pub fn eval_bytes_with_options(
        &mut self,
        source: &[u8],
        options: &EvalOptions,
    ) -> Result<Value, RuntimeError> {
        self.eval_compiling_with_options(options, |context, compile_options| {
            context.compile_bytes_with_options(source, compile_options)
        })
    }

    fn eval_compiling_with_options(
        &mut self,
        options: &EvalOptions,
        compile: impl FnOnce(&mut Self, &CompileOptions) -> Result<FunctionBytecodeRef, RuntimeError>,
    ) -> Result<Value, RuntimeError> {
        let barrier = self
            .runtime
            .install_backtrace_barrier(options.backtrace_barrier)?;
        let result = (|| {
            let function = compile(self, &CompileOptions::new(&options.filename))?;
            self.execute(&function)
        })();
        barrier.finish()?;
        result
    }
}
