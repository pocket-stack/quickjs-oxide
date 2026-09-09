use super::*;

/// Executes verified stack bytecode inside a `Context`. The VM is kept
/// independent from parsing so the compiler and decoder can share it.
///
/// Operand roots belong to one invocation, just like a QuickJS stack frame.
/// Keeping them in a local `VmActivation` makes every normal and exceptional
/// exit release the frame immediately. Generator execution transfers the same
/// activation into a `VmSuspension` instead, so no Rust interpreter frame is
/// retained between calls.
#[derive(Default)]
pub struct Vm;

/// Caller state attached to one original direct-eval invocation.
///
/// The VM constructs this only after the realm-local original-eval identity
/// gate succeeds. `this_value` is the caller-visible binding: primitive String
/// input therefore triggers the caller frame's lazy sloppy-`this`
/// normalization before crossing the runtime boundary, while non-String input
/// retains the raw call value and cannot allocate a wrapper merely to be
/// returned unchanged.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DirectEvalInvocation {
    pub input: Value,
    pub environment: u16,
    pub this_value: Value,
    pub new_target: Value,
    pub caller_strict: bool,
}

pub(crate) struct CallInput<'a> {
    pub code: &'a [Instruction],
    pub metadata: FunctionMetadata,
    pub caller_realm: ContextId,
    pub callee_realm: ContextId,
    pub current_function: ObjectRef,
    pub this_value: Value,
    pub new_target: Value,
    pub callee_global: ObjectRef,
}

pub(crate) trait VmHost {
    fn update_active_bytecode_pc(&mut self, pc: BytecodePc) -> Result<(), Error>;
    /// Attach a QuickJS-style backtrace before the active frame can unwind.
    /// Detached execution has no realm heap and therefore implements this as
    /// a no-op.
    fn ensure_backtrace(&mut self, value: &Value) -> Result<(), Error>;
    /// Mark captured lexical cells which may be reset by the next same-frame
    /// scope entry. QuickJS can skip the ordinary `CloseLocal` path both when
    /// dispatching a caught throw and when unwinding a return through finally;
    /// detached execution has no captured cells.
    fn prepare_captured_local_reuse(&mut self) -> Result<(), Error>;
    fn for_of_start(&mut self, iterable: Value) -> Result<ForOfStartOutcome, Error>;
    fn for_await_of_start(&mut self, iterable: Value) -> Result<ForOfStartOutcome, Error> {
        self.for_of_start(iterable)
    }
    fn append_start(&mut self, iterable: Value) -> Result<AppendStartOutcome, Error> {
        Ok(match self.for_of_start(iterable)? {
            ForOfStartOutcome::Record {
                iterator,
                next_method,
            } => AppendStartOutcome::Record {
                iterator,
                next_method,
                fast_values: None,
            },
            ForOfStartOutcome::Throw(value) => AppendStartOutcome::Throw(value),
        })
    }
    fn for_of_next(
        &mut self,
        iterator: Value,
        next_method: Value,
    ) -> Result<ForOfNextOutcome, Error>;
    /// Complete the awaited result of an async iterator's cached `next`
    /// method. Pinned QuickJS reads `done` and then `value` even when `done`
    /// is true; JavaScript throws stay explicit so the temporarily disabled
    /// iterator region cannot close them.
    fn iterator_get_value_done(&mut self, result: Value) -> Result<ForOfNextOutcome, Error>;
    fn for_in_start(&mut self, value: Value) -> Result<ForInStartOutcome, Error>;
    fn for_in_next(&mut self, iterator: Value) -> Result<ForInNextOutcome, Error>;
    /// Close an iterator. With `exception_pending`, the VM retains the
    /// original thrown value even if this hook reports a JavaScript throw.
    fn iterator_close(
        &mut self,
        iterator: Value,
        exception_pending: bool,
    ) -> Result<IteratorCloseOutcome, Error>;
    fn load_constant(&mut self, index: u32) -> Result<Value, Error>;
    /// Build the atom-named diagnostic for `ThrowReadOnly`. Runtime execution
    /// resolves the constant through its atom table; detached execution has no
    /// table and formats the verified String constant directly.
    fn read_only_error(&mut self, index: u32) -> Result<Error, Error>;
    /// Build QuickJS's eval-time lexical-redeclaration SyntaxError. Keeping
    /// this as bytecode execution (rather than a compile failure) lets direct
    /// global eval instantiate its declaration records first, as QuickJS does.
    fn redeclaration_error(&mut self, index: u32) -> Result<Error, Error>;
    fn to_boolean(&mut self, value: &Value) -> Result<bool, Error>;
    fn is_html_dda(&mut self, value: &Value) -> Result<bool, Error>;
    fn is_callable(&mut self, value: &Value) -> Result<bool, Error>;
    /// Return QuickJS's atom-backed `typeof` String. Runtime-backed hosts must
    /// preserve the canonical atom identity instead of allocating a fresh
    /// String for every execution.
    fn type_of(&mut self, value: &Value) -> Result<JsString, Error>;
    fn box_primitive(&mut self, value: Value) -> Result<Value, Error>;
    fn to_primitive(&mut self, value: Value, hint: ToPrimitiveHint) -> Result<Completion, Error>;
    fn materialize_error(&mut self, error: Error) -> Result<Value, Error>;
    fn instantiate_closure(&mut self, index: u32) -> Result<Value, Error>;
    fn set_function_name(&mut self, value: &Value, name_index: u32) -> Result<(), Error>;
    /// QuickJS `OP_set_name_computed`. `key` has already passed through
    /// `ToPropertyKey`, so this hook must not repeat observable conversion.
    fn set_function_name_computed(&mut self, value: &Value, key: &Value) -> Result<(), Error>;
    /// Create the current ordinary function's mapped or unmapped arguments
    /// object in its defining realm.
    fn create_arguments(&mut self, kind: ArgumentsKind) -> Result<Completion, Error>;
    /// Collect the active frame's actual arguments from `start` onward into a
    /// fresh Array in the callee realm, matching QuickJS `OP_rest`.
    fn create_rest(&mut self, start: u16) -> Result<Completion, Error>;
    /// Create a fresh ordinary Object in the executing bytecode's realm.
    fn object(&mut self) -> Result<Completion, Error>;
    /// Read the active bytecode function's authenticated HomeObject. Detached
    /// execution has no function object and therefore keeps the default
    /// rejection.
    fn home_object(&mut self) -> Result<Value, Error> {
        Err(Error::internal("VM host has no active HomeObject"))
    }
    /// QuickJS `OP_get_super`: return the current prototype of HomeObject,
    /// represented by JavaScript `null` when no prototype exists.
    fn get_super(&mut self, _home_object: Value) -> Result<Value, Error> {
        Err(Error::internal("VM host cannot resolve a super base"))
    }
    /// Create the null-prototype object which backs one sloppy direct-eval
    /// variable environment.
    fn create_variable_environment(&mut self) -> Result<Completion, Error>;
    fn has_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error>;
    fn get_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error>;
    fn put_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error>;
    fn delete_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error>;
    fn define_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error>;
    fn has_dynamic_binding(
        &mut self,
        _source: DynamicEnvironmentSource,
        _name: u32,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host does not support dynamic environment lookup",
        ))
    }
    fn get_dynamic_binding(
        &mut self,
        _source: DynamicEnvironmentSource,
        _name: u32,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host does not support dynamic environment lookup",
        ))
    }
    fn put_dynamic_binding(
        &mut self,
        _source: DynamicEnvironmentSource,
        _name: u32,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host does not support dynamic environment mutation",
        ))
    }
    fn delete_dynamic_binding(
        &mut self,
        _source: DynamicEnvironmentSource,
        _name: u32,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host does not support dynamic environment deletion",
        ))
    }
    fn dynamic_environment_object(
        &mut self,
        _source: DynamicEnvironmentSource,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host cannot expose dynamic environment objects",
        ))
    }
    fn global_reference(&mut self, _index: u16) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host cannot resolve global reference objects",
        ))
    }
    fn get_ref_value(
        &mut self,
        _environment: Value,
        _name: u32,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host does not support dynamic reference reads",
        ))
    }
    fn put_ref_value(
        &mut self,
        _environment: Value,
        _name: u32,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host does not support dynamic reference writes",
        ))
    }
    /// Instantiate a compile-time RegExp constant in the executing bytecode's
    /// realm, bypassing observable constructor and prototype reads.
    fn create_regexp(&mut self, index: u32) -> Result<Completion, Error>;
    /// Create a fresh Array in the executing bytecode's realm from one dense
    /// literal prefix. `Return` carries the Array; `Throw` carries allocation
    /// or Array-exotic failure.
    fn array_from(&mut self, elements: Vec<Value>) -> Result<Completion, Error>;
    /// QuickJS `OP_define_field`: define one own C_W_E data property named by
    /// a verified string constant. The VM preserves `base` on success.
    fn define_field(
        &mut self,
        base: Value,
        key_index: u32,
        value: Value,
    ) -> Result<Completion, Error>;
    /// Computed-key form of [`VmHost::define_field`]. `key` is already the
    /// canonical output of `ToPropKey`; hosts must reject malformed values
    /// rather than invoking observable conversion again. The VM preserves
    /// `base` on success.
    fn define_field_computed(
        &mut self,
        base: Value,
        key: Value,
        value: Value,
    ) -> Result<Completion, Error>;
    /// QuickJS `OP_define_method`: set the closure's inferred name and define
    /// the corresponding data or accessor property. The VM preserves `base`.
    fn define_method(
        &mut self,
        base: Value,
        key_index: u32,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<Completion, Error>;
    /// Computed-key form of [`VmHost::define_method`]. `key` has already been
    /// canonicalized by `ToPropKey`, and the VM preserves only `base`.
    fn define_method_computed(
        &mut self,
        base: Value,
        key: Value,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<Completion, Error>;
    /// QuickJS `OP_define_class`: publish a constructor/prototype pair from a
    /// compiler-created constructor closure and its evaluated parent value.
    fn define_class(
        &mut self,
        parent: Value,
        constructor: Value,
        name: u32,
        has_heritage: bool,
    ) -> Result<DefineClassOutcome, Error>;
    /// Install the hidden instance-fields closure on a freshly created class.
    /// The host authenticates all three bytecode-function roles and owns the
    /// retain-first HomeObject/internal-slot mutations.
    fn install_class_instance_initializer(
        &mut self,
        _constructor: Value,
        _prototype: Value,
        _initializer: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host cannot install a class instance initializer",
        ))
    }
    /// Invoke the active constructor's hidden instance-fields closure with the
    /// supplied initialized receiver. A class without fields returns normally.
    fn call_class_instance_initializer(
        &mut self,
        _active_constructor: Value,
        _receiver: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host cannot call a class instance initializer",
        ))
    }
    /// Install HomeObject and run the aggregate static-elements closure.
    fn run_class_static_initializer(
        &mut self,
        _constructor: Value,
        _initializer: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "VM host cannot run a class static initializer",
        ))
    }
    /// Invoke one non-escaping static-block child inside its aggregate static
    /// initializer frame.
    fn call_class_static_block(
        &mut self,
        _static_initializer: ObjectRef,
        _this_value: Value,
        _block: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot call a class static block"))
    }
    /// QuickJS `OP_define_array_el`: define one own C_W_E data property using
    /// the dynamic internal Array-literal index. The VM preserves both base
    /// and index on success.
    fn define_array_element(
        &mut self,
        base: Value,
        index: Value,
        value: Value,
    ) -> Result<Completion, Error>;
    /// Apply object-literal `__proto__` semantics to a fresh ordinary Object.
    fn set_object_prototype(
        &mut self,
        object: Value,
        prototype: Value,
    ) -> Result<Completion, Error>;
    /// Copy QuickJS object-literal spread data properties into `target`.
    fn copy_data_properties(&mut self, target: Value, source: Value) -> Result<Completion, Error>;
    /// Copy Object-rest data properties after the compiler has performed the
    /// pattern's leading `ToObject`. `excluded` is the private fresh Object
    /// whose own String/Symbol keys identify properties already bound by the
    /// pattern.
    fn copy_data_properties_excluded(
        &mut self,
        target: Value,
        source: Value,
        excluded: Value,
    ) -> Result<Completion, Error>;
    fn get_global_var(&mut self, index: u16, throw_if_missing: bool) -> Result<Completion, Error>;
    fn delete_global_var(&mut self, index: u16) -> Result<Completion, Error>;
    fn put_global_var(
        &mut self,
        index: u16,
        value: Value,
        initialize: bool,
        strict: bool,
    ) -> Result<Completion, Error>;
    /// Allocate and initialize one fresh class-private data-field identity in
    /// an authenticated lexical local. The identity never enters the operand
    /// stack as a public JavaScript value.
    fn initialize_private_name(&mut self, _index: u16) -> Result<(), Error> {
        Err(Error::internal("VM host cannot initialize a private name"))
    }
    /// Initialize one authenticated private-method binding from a fresh
    /// callable while retaining its class-side HomeObject in the VM.
    fn initialize_private_method(
        &mut self,
        _index: u16,
        _home_object: Value,
        _method: Value,
    ) -> Result<(), Error> {
        Err(Error::internal(
            "VM host cannot initialize a private method",
        ))
    }
    /// Initialize one authenticated private getter/setter cell from a fresh
    /// callable while retaining its class-side HomeObject in the VM. The host
    /// must not infer a public function name: pinned QuickJS leaves private
    /// accessor names empty.
    fn initialize_private_accessor(
        &mut self,
        _index: u16,
        _home_object: Value,
        _accessor: Value,
    ) -> Result<(), Error> {
        Err(Error::internal(
            "VM host cannot initialize a private accessor",
        ))
    }
    fn get_private_field(
        &mut self,
        _source: PrivateNameSource,
        _base: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot read a private field"))
    }
    fn put_private_field(
        &mut self,
        _source: PrivateNameSource,
        _base: Value,
        _value: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot write a private field"))
    }
    fn define_private_field(
        &mut self,
        _source: PrivateNameSource,
        _base: Value,
        _value: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot define a private field"))
    }
    fn private_in(
        &mut self,
        _source: PrivateNameSource,
        _base: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot test a private field"))
    }
    /// Constant-name property access. Keeping the constant-pool index here
    /// preserves the exact UTF-16 spelling and distinguishes a verified field
    /// operand from an arbitrary computed value.
    fn get_field(&mut self, base: Value, key_index: u32) -> Result<Completion, Error>;
    /// `base[key]` after the VM has preserved QuickJS's operand order. The
    /// runtime host owns `ToObject`/`ToPropertyKey` and accessor execution.
    fn get_property(&mut self, base: Value, key: Value) -> Result<Completion, Error>;
    /// Receiver-aware `super[key]` read. `base` is the HomeObject prototype
    /// frozen before computed-key evaluation; `receiver` is the method's
    /// actual `this` value.
    fn get_super_property(
        &mut self,
        _receiver: Value,
        _base: Value,
        _key: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot read a super property"))
    }
    /// QuickJS `OP_in`: the VM has already validated the RHS Object, so the
    /// host can perform observable left-operand ToPropertyKey conversion.
    fn has_property(&mut self, key: Value, object: ObjectRef) -> Result<Completion, Error>;
    /// QuickJS `OP_instanceof`: full GetMethod/Call/fallback semantics live at
    /// the runtime boundary so arbitrary throws and defining realms survive.
    fn is_instance_of(&mut self, candidate: Value, target: ObjectRef) -> Result<Completion, Error>;
    /// Convert an arbitrary value to the canonical Int/String/Symbol value
    /// which represents its property key. This can execute user code and throw.
    fn convert_property_key(&mut self, key: Value) -> Result<Completion, Error>;
    fn set_field(
        &mut self,
        base: Value,
        key_index: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error>;
    fn set_property(
        &mut self,
        base: Value,
        key: Value,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error>;
    /// Receiver-aware super write using QuickJS `JS_PROP_THROW_STRICT`:
    /// rejected writes throw only when the containing frame is strict.
    fn set_super_property(
        &mut self,
        _receiver: Value,
        _base: Value,
        _key: Value,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal("VM host cannot write a super property"))
    }
    fn delete_property(
        &mut self,
        base: Value,
        key: Value,
        strict: bool,
    ) -> Result<Completion, Error>;
    /// QuickJS `OP_import`. Both expressions have already been evaluated and
    /// remain in source order; the runtime host will eventually own Promise
    /// creation, option processing, and FIFO module-job scheduling.
    fn dynamic_import(&mut self, specifier: Value, options: Value) -> Result<Completion, Error>;
    fn call(
        &mut self,
        function: Value,
        this_value: Value,
        arguments: Vec<Value>,
    ) -> Result<Completion, Error>;
    /// QuickJS `OP_apply`. The host owns callable/constructor validation
    /// ordering around argument-list construction.
    fn apply(
        &mut self,
        function: Value,
        this_or_new_target: Value,
        argument_array: Value,
        kind: ApplyKind,
    ) -> Result<Completion, Error>;
    /// QuickJS `build_arg_list`, used directly by `OP_apply_eval` before its
    /// callee identity decision.
    fn build_argument_list(&mut self, argument_array: Value) -> Result<ArgumentListOutcome, Error>;
    /// Test the callee identity for QuickJS `OP_eval` against the executing
    /// realm's cached original eval, never its mutable global property.
    fn is_original_eval(&mut self, function: &Value) -> Result<bool, Error>;
    /// Enter original direct eval without creating a native `%eval%` frame.
    /// Arguments have already been evaluated; only the first input survives.
    fn direct_eval(&mut self, invocation: DirectEvalInvocation) -> Result<Completion, Error>;
    fn construct(
        &mut self,
        function: Value,
        new_target: Value,
        arguments: Vec<Value>,
    ) -> Result<Completion, Error>;
    /// Execute the implicit body of a default derived constructor. The host
    /// owns the frame's raw actual arguments; the VM supplies the exact active
    /// function and original `new.target` so neither can be reconstructed from
    /// JavaScript-visible state.
    fn init_derived_constructor(
        &mut self,
        active_function: ObjectRef,
        new_target: Value,
    ) -> Result<Completion, Error>;
    fn closure_count(&self) -> usize;
    fn get_local(&mut self, index: u16) -> Result<Value, Error>;
    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn set_local_uninitialized(&mut self, index: u16) -> Result<(), Error>;
    fn get_local_checked(&mut self, index: u16) -> Result<Value, Error>;
    fn initialize_local(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn initialize_derived_local(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn put_local_checked(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn close_local(&mut self, index: u16) -> Result<(), Error>;
    fn get_argument(&mut self, index: u16) -> Result<Value, Error>;
    fn put_argument(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn get_var_ref(&mut self, index: u16) -> Result<Value, Error>;
    fn put_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn get_var_ref_checked(&mut self, index: u16) -> Result<Value, Error>;
    fn put_var_ref_checked(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn initialize_var_ref(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::new(
            crate::engine::api::error::ErrorKind::Unsupported,
            "module lexical initialization requires a module runtime publisher",
        ))
    }
    fn initialize_module_import_collision(
        &mut self,
        _index: u16,
        _value: Value,
    ) -> Result<(), Error> {
        Err(Error::new(
            crate::engine::api::error::ErrorKind::Unsupported,
            "module import collision initialization requires a module runtime publisher",
        ))
    }
    fn initialize_derived_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error>;
    /// Apply the derived-constructor return protocol. Unlike ordinary VM
    /// errors, protocol TypeError/ReferenceError objects are allocated in the
    /// outer caller realm by the runtime host.
    fn return_derived(&mut self, index: u16, value: Value) -> Result<Completion, Error>;
}

impl Vm {
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Execute a bytecode function to completion.
    ///
    /// # Errors
    /// Returns an internal error for malformed bytecode and a JavaScript-style
    /// type error for an operation not valid for the operand types.
    #[cfg(test)]
    pub fn execute<T: TestConstant + Clone + Into<Value>>(
        &mut self,
        function: &DetachedBytecode<T>,
    ) -> Result<Value, Error> {
        let verified = function.verify()?;
        let mut host = DetachedHost::new(function);
        match VmActivation::new(usize::from(verified.max_stack))
            .execute(&function.code, &mut host)?
        {
            Completion::Return(value) => Ok(value),
            Completion::Throw(_) => Err(Error::internal(
                "detached VM execution cannot publish a JavaScript exception",
            )),
        }
    }

    /// Execute an immutable non-generator function which was verified before
    /// runtime publication.
    ///
    /// Keep this driver separate from [`Self::start_published`]. Returning the
    /// generator-capable [`VmExit`] from every ordinary recursive call would
    /// reserve space for a complete suspended activation in each native stack
    /// frame, regressing the proven two-MiB recursion boundary.
    pub(crate) fn execute_published(
        &mut self,
        input: CallInput<'_>,
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        let CallInput {
            code,
            metadata,
            caller_realm,
            callee_realm,
            current_function,
            this_value,
            new_target,
            callee_global,
        } = input;
        if host.closure_count() != usize::from(metadata.closure_count) {
            return Err(Error::internal(
                "function object closure slot count does not match bytecode metadata",
            ));
        }
        VmActivation::new_in_realm(
            metadata,
            caller_realm,
            callee_realm,
            current_function,
            this_value,
            new_target,
            callee_global,
        )
        .execute(code, host)
    }

    /// Start an immutable published bytecode activation and allow it to
    /// transfer ownership at a generator or async-function suspension point.
    pub(crate) fn start_published(
        &mut self,
        input: CallInput<'_>,
        host: &mut impl VmHost,
    ) -> Result<VmExit, Error> {
        let CallInput {
            code,
            metadata,
            caller_realm,
            callee_realm,
            current_function,
            this_value,
            new_target,
            callee_global,
        } = input;
        if host.closure_count() != usize::from(metadata.closure_count) {
            return Err(Error::internal(
                "function object closure slot count does not match bytecode metadata",
            ));
        }
        VmActivation::new_in_realm(
            metadata,
            caller_realm,
            callee_realm,
            current_function,
            this_value,
            new_target,
            callee_global,
        )
        .run(code, host)
    }

    /// Continue past a generator's hidden initial-yield barrier. No resume
    /// value is accepted because the first `next` argument is ignored.
    pub(crate) fn resume_published_initial(
        &mut self,
        suspension: VmSuspension,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<VmExit, Error> {
        suspension.resume_initial(code, host)
    }

    /// Resume a visible `yield` or `yield*` suspension.
    pub(crate) fn resume_published(
        &mut self,
        suspension: VmSuspension,
        code: &[Instruction],
        host: &mut impl VmHost,
        resume: VmResume,
    ) -> Result<VmExit, Error> {
        suspension.resume(code, host, resume)
    }
}
