use crate::bigint::{BigIntError, JsBigInt};
use crate::bytecode::{
    ArgumentsKind, BytecodeFunction, DynamicEnvironmentSource, EvalVariableSource, Instruction,
};
use crate::error::{Error, ErrorKind, NativeErrorKind};
use crate::heap::{ContextId, FunctionMetadata};
use crate::object::ObjectRef;
use crate::value::{JsString, Value};
use num_bigint::BigInt;
use num_traits::FromPrimitive;
#[cfg(test)]
use std::collections::VecDeque;

/// Executes verified stack bytecode inside a future `Context`. The VM is kept
/// independent from parsing so the compiler and decoder can share it.
///
/// Operand roots belong to one invocation, just like a QuickJS stack frame.
/// Keeping them in a local [`CallFrame`] makes every normal and exceptional
/// exit release the frame immediately instead of retaining values until the
/// next call into the VM.
#[derive(Default)]
pub struct Vm;

/// Private JavaScript control completion. A thrown value remains a rooted
/// ordinary [`Value`]; no exception sentinel is exposed through the public
/// value representation.
#[derive(Debug, PartialEq)]
pub(crate) enum Completion {
    Return(Value),
    Throw(Value),
}

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

/// ECMAScript ToPrimitive hint crossing the VM/runtime host boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToPrimitiveHint {
    Default,
    Number,
    String,
}

enum OperationOutcome<T> {
    Value(T),
    Throw(Value),
}

/// Result of the observable `GetIterator` and `Get(iterator, "next")`
/// operations used by `ForOfStart`.
pub(crate) enum ForOfStartOutcome {
    Record { iterator: Value, next_method: Value },
    Throw(Value),
}

/// Result of calling an iterator record's cached `next` method and reading its
/// `done`/`value` properties.
pub(crate) enum ForOfNextOutcome {
    Result { value: Value, done: bool },
    Throw(Value),
}

/// Result of creating the hidden object used by a for-in loop.
pub(crate) enum ForInStartOutcome {
    Iterator(Value),
    Throw(Value),
}

/// Result of advancing a hidden for-in enumeration object.
pub(crate) enum ForInNextOutcome {
    Result { value: Value, done: bool },
    Throw(Value),
}

/// Result of `IteratorClose`. Engine failures remain [`Error`]s; JavaScript
/// throws are explicit so the VM can apply completion precedence itself.
pub(crate) enum IteratorCloseOutcome {
    Closed,
    Throw(Value),
}

enum NumericValue {
    Number(f64),
    BigInt(JsBigInt),
}

/// Stable instruction offset recorded on an active bytecode frame.
///
/// The runtime deliberately records the offset of the instruction currently
/// being executed, rather than the VM's already-advanced dispatch cursor. A
/// nested call therefore leaves its caller parked on the `Call` or
/// `Construct` opcode, matching the frame information QuickJS retains for
/// later exception-stack construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BytecodePc(usize);

impl BytecodePc {
    #[must_use]
    pub(crate) const fn new(index: usize) -> Self {
        Self(index)
    }

    #[must_use]
    pub(crate) const fn index(self) -> usize {
        self.0
    }
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
    fn for_of_next(
        &mut self,
        iterator: Value,
        next_method: Value,
    ) -> Result<ForOfNextOutcome, Error>;
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
    fn type_of(&mut self, value: &Value) -> Result<&'static str, Error>;
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
    /// Create a fresh ordinary Object in the executing bytecode's realm.
    fn object(&mut self) -> Result<Completion, Error>;
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
    fn get_global_var(&mut self, index: u16, throw_if_missing: bool) -> Result<Completion, Error>;
    fn delete_global_var(&mut self, index: u16) -> Result<Completion, Error>;
    fn put_global_var(
        &mut self,
        index: u16,
        value: Value,
        initialize: bool,
        strict: bool,
    ) -> Result<Completion, Error>;
    /// Constant-name property access. Keeping the constant-pool index here
    /// preserves the exact UTF-16 spelling and distinguishes a verified field
    /// operand from an arbitrary computed value.
    fn get_field(&mut self, base: Value, key_index: u32) -> Result<Completion, Error>;
    /// `base[key]` after the VM has preserved QuickJS's operand order. The
    /// runtime host owns `ToObject`/`ToPropertyKey` and accessor execution.
    fn get_property(&mut self, base: Value, key: Value) -> Result<Completion, Error>;
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
    fn delete_property(
        &mut self,
        base: Value,
        key: Value,
        strict: bool,
    ) -> Result<Completion, Error>;
    fn call(
        &mut self,
        function: Value,
        this_value: Value,
        arguments: Vec<Value>,
    ) -> Result<Completion, Error>;
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
    fn closure_count(&self) -> usize;
    fn get_local(&mut self, index: u16) -> Result<Value, Error>;
    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn set_local_uninitialized(&mut self, index: u16) -> Result<(), Error>;
    fn get_local_checked(&mut self, index: u16) -> Result<Value, Error>;
    fn initialize_local(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn put_local_checked(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn close_local(&mut self, index: u16) -> Result<(), Error>;
    fn get_argument(&mut self, index: u16) -> Result<Value, Error>;
    fn put_argument(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn get_var_ref(&mut self, index: u16) -> Result<Value, Error>;
    fn put_var_ref(&mut self, index: u16, value: Value) -> Result<(), Error>;
    fn get_var_ref_checked(&mut self, index: u16) -> Result<Value, Error>;
    fn put_var_ref_checked(&mut self, index: u16, value: Value) -> Result<(), Error>;
}

enum DetachedLocal {
    Initialized(Value),
    Uninitialized,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
enum DetachedEvalVariableOperation {
    Has(EvalVariableSource, u32),
    Get(EvalVariableSource, u32),
    Put(EvalVariableSource, u32, Value),
    Delete(EvalVariableSource, u32),
    Define(EvalVariableSource, u32, Value),
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
enum DetachedDynamicEnvironmentOperation {
    Has(DynamicEnvironmentSource, u32),
    Get(DynamicEnvironmentSource, u32, bool),
    Put(DynamicEnvironmentSource, u32, Value, bool),
    Delete(DynamicEnvironmentSource, u32),
    Object(DynamicEnvironmentSource),
    GetRef(Value, u32, bool),
    PutRef(Value, u32, Value, bool),
}

struct DetachedHost<'a> {
    function: &'a BytecodeFunction,
    locals: Vec<DetachedLocal>,
    #[cfg(test)]
    captured_local_reuse_preparations: usize,
    #[cfg(test)]
    iterator_start_record: Option<(Value, Value)>,
    #[cfg(test)]
    iterator_next_results: VecDeque<Result<(Value, bool), Value>>,
    #[cfg(test)]
    iterator_close_results: VecDeque<Option<Value>>,
    #[cfg(test)]
    iterator_close_pending: Vec<bool>,
    #[cfg(test)]
    array_from_results: VecDeque<Completion>,
    #[cfg(test)]
    array_from_inputs: Vec<Vec<Value>>,
    #[cfg(test)]
    define_field_results: VecDeque<Completion>,
    #[cfg(test)]
    defined_fields: Vec<(Value, u32, Value)>,
    #[cfg(test)]
    define_array_element_results: VecDeque<Completion>,
    #[cfg(test)]
    defined_array_elements: Vec<(Value, Value, Value)>,
    #[cfg(test)]
    object_results: VecDeque<Completion>,
    #[cfg(test)]
    variable_environment_results: VecDeque<Completion>,
    #[cfg(test)]
    eval_variable_results: VecDeque<Completion>,
    #[cfg(test)]
    eval_variable_operations: Vec<DetachedEvalVariableOperation>,
    #[cfg(test)]
    dynamic_environment_results: VecDeque<Completion>,
    #[cfg(test)]
    dynamic_environment_operations: Vec<DetachedDynamicEnvironmentOperation>,
    #[cfg(test)]
    arguments_results: VecDeque<(ArgumentsKind, Completion)>,
    #[cfg(test)]
    set_object_prototype_results: VecDeque<Completion>,
    #[cfg(test)]
    set_object_prototype_inputs: Vec<(Value, Value)>,
    #[cfg(test)]
    copy_data_properties_results: VecDeque<Completion>,
    #[cfg(test)]
    copy_data_properties_inputs: Vec<(Value, Value)>,
    #[cfg(test)]
    eval_identity_results: VecDeque<Result<bool, Error>>,
    #[cfg(test)]
    eval_identity_inputs: Vec<Value>,
    #[cfg(test)]
    direct_eval_results: VecDeque<Result<Completion, Error>>,
    #[cfg(test)]
    direct_eval_inputs: Vec<DirectEvalInvocation>,
    #[cfg(test)]
    box_primitive_results: VecDeque<Result<Value, Error>>,
    #[cfg(test)]
    box_primitive_inputs: Vec<Value>,
    #[cfg(test)]
    call_results: VecDeque<Result<Completion, Error>>,
    #[cfg(test)]
    call_inputs: Vec<(Value, Value, Vec<Value>)>,
}

impl<'a> DetachedHost<'a> {
    fn new(function: &'a BytecodeFunction) -> Self {
        Self {
            function,
            locals: (0..function.local_count)
                .map(|_| DetachedLocal::Initialized(Value::Undefined))
                .collect(),
            #[cfg(test)]
            captured_local_reuse_preparations: 0,
            #[cfg(test)]
            iterator_start_record: None,
            #[cfg(test)]
            iterator_next_results: VecDeque::new(),
            #[cfg(test)]
            iterator_close_results: VecDeque::new(),
            #[cfg(test)]
            iterator_close_pending: Vec::new(),
            #[cfg(test)]
            array_from_results: VecDeque::new(),
            #[cfg(test)]
            array_from_inputs: Vec::new(),
            #[cfg(test)]
            define_field_results: VecDeque::new(),
            #[cfg(test)]
            defined_fields: Vec::new(),
            #[cfg(test)]
            define_array_element_results: VecDeque::new(),
            #[cfg(test)]
            defined_array_elements: Vec::new(),
            #[cfg(test)]
            object_results: VecDeque::new(),
            #[cfg(test)]
            variable_environment_results: VecDeque::new(),
            #[cfg(test)]
            eval_variable_results: VecDeque::new(),
            #[cfg(test)]
            eval_variable_operations: Vec::new(),
            #[cfg(test)]
            dynamic_environment_results: VecDeque::new(),
            #[cfg(test)]
            dynamic_environment_operations: Vec::new(),
            #[cfg(test)]
            arguments_results: VecDeque::new(),
            #[cfg(test)]
            set_object_prototype_results: VecDeque::new(),
            #[cfg(test)]
            set_object_prototype_inputs: Vec::new(),
            #[cfg(test)]
            copy_data_properties_results: VecDeque::new(),
            #[cfg(test)]
            copy_data_properties_inputs: Vec::new(),
            #[cfg(test)]
            eval_identity_results: VecDeque::new(),
            #[cfg(test)]
            eval_identity_inputs: Vec::new(),
            #[cfg(test)]
            direct_eval_results: VecDeque::new(),
            #[cfg(test)]
            direct_eval_inputs: Vec::new(),
            #[cfg(test)]
            box_primitive_results: VecDeque::new(),
            #[cfg(test)]
            box_primitive_inputs: Vec::new(),
            #[cfg(test)]
            call_results: VecDeque::new(),
            #[cfg(test)]
            call_inputs: Vec::new(),
        }
    }

    fn local(&self, index: u16) -> Result<&DetachedLocal, Error> {
        self.locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local-variable index is out of bounds"))
    }

    fn local_mut(&mut self, index: u16) -> Result<&mut DetachedLocal, Error> {
        self.locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local-variable index is out of bounds"))
    }
}

impl VmHost for DetachedHost<'_> {
    fn update_active_bytecode_pc(&mut self, _pc: BytecodePc) -> Result<(), Error> {
        Ok(())
    }

    fn ensure_backtrace(&mut self, _value: &Value) -> Result<(), Error> {
        Ok(())
    }

    fn prepare_captured_local_reuse(&mut self) -> Result<(), Error> {
        #[cfg(test)]
        {
            self.captured_local_reuse_preparations += 1;
        }
        Ok(())
    }

    fn for_of_start(&mut self, _iterable: Value) -> Result<ForOfStartOutcome, Error> {
        #[cfg(test)]
        if let Some((iterator, next_method)) = self.iterator_start_record.take() {
            return Ok(ForOfStartOutcome::Record {
                iterator,
                next_method,
            });
        }
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn for_of_next(
        &mut self,
        _iterator: Value,
        _next_method: Value,
    ) -> Result<ForOfNextOutcome, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.iterator_next_results.pop_front() {
            return Ok(match outcome {
                Ok((value, done)) => ForOfNextOutcome::Result { value, done },
                Err(value) => ForOfNextOutcome::Throw(value),
            });
        }
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn for_in_start(&mut self, _value: Value) -> Result<ForInStartOutcome, Error> {
        Err(Error::internal("detached VM has no for-in intrinsics"))
    }

    fn for_in_next(&mut self, _iterator: Value) -> Result<ForInNextOutcome, Error> {
        Err(Error::internal("detached VM has no for-in intrinsics"))
    }

    fn iterator_close(
        &mut self,
        _iterator: Value,
        exception_pending: bool,
    ) -> Result<IteratorCloseOutcome, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.iterator_close_results.pop_front() {
            self.iterator_close_pending.push(exception_pending);
            return Ok(match outcome {
                Some(value) => IteratorCloseOutcome::Throw(value),
                None => IteratorCloseOutcome::Closed,
            });
        }
        let _ = exception_pending;
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn load_constant(&mut self, index: u32) -> Result<Value, Error> {
        self.function
            .constant(index)
            .cloned()
            .ok_or_else(|| Error::internal("constant index is out of bounds"))
    }

    fn read_only_error(&mut self, index: u32) -> Result<Error, Error> {
        let Value::String(name) = self.load_constant(index)? else {
            return Err(Error::internal(
                "read-only binding opcode referenced a non-string constant",
            ));
        };
        let mut message = crate::error::NativeErrorMessage::new();
        message.push_utf8("'");
        name.push_atom_get_str_to(&mut message);
        message.push_utf8("' is read-only");
        Ok(Error::from_native_message(ErrorKind::Type, message))
    }

    fn redeclaration_error(&mut self, index: u32) -> Result<Error, Error> {
        let Value::String(_) = self.load_constant(index)? else {
            return Err(Error::internal(
                "redeclaration opcode referenced a non-string constant",
            ));
        };
        Ok(Error::new(
            ErrorKind::Syntax,
            "invalid redefinition of lexical identifier",
        ))
    }

    fn type_of(&mut self, value: &Value) -> Result<&'static str, Error> {
        Ok(value.type_of())
    }

    fn box_primitive(&mut self, _value: Value) -> Result<Value, Error> {
        #[cfg(test)]
        {
            self.box_primitive_inputs.push(_value);
            if let Some(result) = self.box_primitive_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM has no primitive wrapper intrinsics",
        ))
    }

    fn to_primitive(&mut self, value: Value, _hint: ToPrimitiveHint) -> Result<Completion, Error> {
        if matches!(value, Value::Object(_)) {
            Err(Error::internal(
                "detached VM cannot execute object ToPrimitive",
            ))
        } else {
            Ok(Completion::Return(value))
        }
    }

    fn materialize_error(&mut self, error: Error) -> Result<Value, Error> {
        Err(error)
    }

    fn instantiate_closure(&mut self, _index: u32) -> Result<Value, Error> {
        Err(Error::internal(
            "detached VM cannot instantiate runtime-owned function bytecode",
        ))
    }

    fn set_function_name(&mut self, _value: &Value, _name_index: u32) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM cannot name a runtime-owned function object",
        ))
    }

    fn set_function_name_computed(&mut self, _value: &Value, _key: &Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM cannot name a runtime-owned function object",
        ))
    }

    fn create_arguments(&mut self, kind: ArgumentsKind) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some((expected, outcome)) = self.arguments_results.pop_front() {
            if expected != kind {
                return Err(Error::internal("unexpected detached arguments kind"));
            }
            return Ok(outcome);
        }
        let _ = kind;
        Err(Error::internal(
            "detached VM cannot create runtime-owned arguments objects",
        ))
    }

    fn object(&mut self) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.object_results.pop_front() {
            return Ok(outcome);
        }
        Err(Error::internal(
            "detached VM cannot create runtime-owned Object values",
        ))
    }

    fn create_variable_environment(&mut self) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.variable_environment_results.pop_front() {
            return Ok(outcome);
        }
        Err(Error::internal(
            "detached VM cannot create runtime-owned eval variable environments",
        ))
    }

    fn has_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Has(source, name));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot inspect eval variable environments",
        ))
    }

    fn get_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Get(source, name));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot inspect eval variable environments",
        ))
    }

    fn put_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Put(
                    source,
                    name,
                    value.clone(),
                ));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, value);
        Err(Error::internal(
            "detached VM cannot mutate eval variable environments",
        ))
    }

    fn delete_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Delete(source, name));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot delete eval variable bindings",
        ))
    }

    fn define_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Define(
                    source,
                    name,
                    value.clone(),
                ));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, value);
        Err(Error::internal(
            "detached VM cannot define eval variable bindings",
        ))
    }

    fn has_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Has(source, name));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot inspect dynamic environments",
        ))
    }

    fn get_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Get(
                    source, name, strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, strict);
        Err(Error::internal(
            "detached VM cannot inspect dynamic environments",
        ))
    }

    fn put_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Put(
                    source,
                    name,
                    value.clone(),
                    strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, value, strict);
        Err(Error::internal(
            "detached VM cannot mutate dynamic environments",
        ))
    }

    fn delete_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Delete(source, name));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot delete dynamic environment bindings",
        ))
    }

    fn dynamic_environment_object(
        &mut self,
        source: DynamicEnvironmentSource,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Object(source));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = source;
        Err(Error::internal(
            "detached VM cannot expose dynamic environment objects",
        ))
    }

    fn get_ref_value(
        &mut self,
        environment: Value,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::GetRef(
                    environment.clone(),
                    name,
                    strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (environment, name, strict);
        Err(Error::internal(
            "detached VM cannot read dynamic reference values",
        ))
    }

    fn put_ref_value(
        &mut self,
        environment: Value,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::PutRef(
                    environment.clone(),
                    name,
                    value.clone(),
                    strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (environment, name, value, strict);
        Err(Error::internal(
            "detached VM cannot write dynamic reference values",
        ))
    }

    fn create_regexp(&mut self, _index: u32) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot create runtime-owned RegExp objects",
        ))
    }

    fn array_from(&mut self, elements: Vec<Value>) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.array_from_results.pop_front() {
            self.array_from_inputs.push(elements);
            return Ok(outcome);
        }
        let _ = elements;
        Err(Error::internal(
            "detached VM cannot create runtime-owned Array objects",
        ))
    }

    fn define_field(
        &mut self,
        base: Value,
        key_index: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_field_results.pop_front() {
            self.defined_fields.push((base, key_index, value));
            return Ok(outcome);
        }
        let _ = (base, key_index, value);
        Err(Error::internal(
            "detached VM cannot define runtime-owned properties",
        ))
    }

    fn define_array_element(
        &mut self,
        base: Value,
        index: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_array_element_results.pop_front() {
            self.defined_array_elements.push((base, index, value));
            return Ok(outcome);
        }
        let _ = (base, index, value);
        Err(Error::internal(
            "detached VM cannot define runtime-owned Array elements",
        ))
    }

    fn set_object_prototype(
        &mut self,
        object: Value,
        prototype: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.set_object_prototype_results.pop_front() {
            self.set_object_prototype_inputs.push((object, prototype));
            return Ok(outcome);
        }
        let _ = (object, prototype);
        Err(Error::internal(
            "detached VM cannot mutate runtime-owned Object prototypes",
        ))
    }

    fn copy_data_properties(&mut self, target: Value, source: Value) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.copy_data_properties_results.pop_front() {
            self.copy_data_properties_inputs.push((target, source));
            return Ok(outcome);
        }
        let _ = (target, source);
        Err(Error::internal(
            "detached VM cannot copy runtime-owned Object properties",
        ))
    }

    fn get_global_var(
        &mut self,
        _index: u16,
        _throw_if_missing: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM has no realm global environment",
        ))
    }

    fn delete_global_var(&mut self, _index: u16) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM has no realm global environment",
        ))
    }

    fn put_global_var(
        &mut self,
        _index: u16,
        _value: Value,
        _initialize: bool,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM has no realm global environment",
        ))
    }

    fn get_field(&mut self, _base: Value, _key_index: u32) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot access runtime-owned properties",
        ))
    }

    fn get_property(&mut self, _base: Value, _key: Value) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot access runtime-owned properties",
        ))
    }

    fn has_property(&mut self, _key: Value, _object: ObjectRef) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot test runtime-owned properties",
        ))
    }

    fn is_instance_of(
        &mut self,
        _candidate: Value,
        _target: ObjectRef,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot perform runtime-owned instance checks",
        ))
    }

    fn convert_property_key(&mut self, _key: Value) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot convert runtime-owned property keys",
        ))
    }

    fn set_field(
        &mut self,
        _base: Value,
        _key_index: u32,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot mutate runtime-owned properties",
        ))
    }

    fn set_property(
        &mut self,
        _base: Value,
        _key: Value,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot mutate runtime-owned properties",
        ))
    }

    fn delete_property(
        &mut self,
        _base: Value,
        _key: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot delete runtime-owned properties",
        ))
    }

    fn call(
        &mut self,
        _function: Value,
        _this_value: Value,
        _arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.call_inputs.push((_function, _this_value, _arguments));
            if let Some(result) = self.call_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM cannot call runtime-owned function objects",
        ))
    }

    fn is_original_eval(&mut self, _function: &Value) -> Result<bool, Error> {
        #[cfg(test)]
        {
            self.eval_identity_inputs.push(_function.clone());
            if let Some(result) = self.eval_identity_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM cannot identify runtime-owned eval function objects",
        ))
    }

    fn direct_eval(&mut self, _invocation: DirectEvalInvocation) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.direct_eval_inputs.push(_invocation);
            if let Some(result) = self.direct_eval_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM cannot directly evaluate runtime-owned function objects",
        ))
    }

    fn construct(
        &mut self,
        _function: Value,
        _new_target: Value,
        _arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot construct runtime-owned function objects",
        ))
    }

    fn closure_count(&self) -> usize {
        0
    }

    fn get_local(&mut self, index: u16) -> Result<Value, Error> {
        match self.local(index)? {
            DetachedLocal::Initialized(value) => Ok(value.clone()),
            DetachedLocal::Uninitialized => Err(Error::internal(
                "unchecked local read reached an uninitialized lexical binding",
            )),
        }
    }

    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let local = self.local_mut(index)?;
        if matches!(local, DetachedLocal::Uninitialized) {
            return Err(Error::internal(
                "unchecked local write reached an uninitialized lexical binding",
            ));
        }
        *local = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn set_local_uninitialized(&mut self, index: u16) -> Result<(), Error> {
        *self.local_mut(index)? = DetachedLocal::Uninitialized;
        Ok(())
    }

    fn get_local_checked(&mut self, index: u16) -> Result<Value, Error> {
        match self.local(index)? {
            DetachedLocal::Initialized(value) => Ok(value.clone()),
            DetachedLocal::Uninitialized => Err(Error::new(
                ErrorKind::Reference,
                "lexical variable is not initialized",
            )),
        }
    }

    fn initialize_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        *self.local_mut(index)? = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn put_local_checked(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let local = self.local_mut(index)?;
        if matches!(local, DetachedLocal::Uninitialized) {
            return Err(Error::new(
                ErrorKind::Reference,
                "lexical variable is not initialized",
            ));
        }
        *local = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn close_local(&mut self, index: u16) -> Result<(), Error> {
        self.local(index)?;
        Ok(())
    }

    fn get_argument(&mut self, _index: u16) -> Result<Value, Error> {
        Err(Error::internal("detached VM has no argument frame"))
    }

    fn put_argument(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal("detached VM has no argument frame"))
    }

    fn get_var_ref(&mut self, _index: u16) -> Result<Value, Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn put_var_ref(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn get_var_ref_checked(&mut self, _index: u16) -> Result<Value, Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn put_var_ref_checked(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }
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
    pub fn execute(&mut self, function: &BytecodeFunction) -> Result<Value, Error> {
        let verified = function.verify()?;
        let mut host = DetachedHost::new(function);
        match CallFrame::new(usize::from(verified.max_stack)).execute(&function.code, &mut host)? {
            Completion::Return(value) => Ok(value),
            Completion::Throw(_) => Err(Error::internal(
                "detached VM execution cannot publish a JavaScript exception",
            )),
        }
    }

    /// Execute an immutable function which was verified before runtime
    /// publication.
    ///
    /// This is intentionally crate-private: safe public callers cannot bypass
    /// [`BytecodeFunction::verify`], while a runtime-owned
    /// `FunctionBytecodeData` does not pay the verifier cost on every call.
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
        CallFrame::new_in_realm(
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
}

/// Per-invocation value stack. This will later grow the remaining
/// `JSStackFrame` fields (arguments, locals, closure variables and realm), but
/// its ownership boundary is already the final one.
#[derive(Clone, Copy, Debug)]
enum UnwindRegion {
    Catch {
        target: usize,
        /// Runtime operand depth before the private catch marker was
        /// installed.
        stack_depth: usize,
    },
    Iterator {
        /// Runtime operand index of `iterator`; `next` immediately follows.
        record_base: usize,
        /// `ForOfNext` disables the record before propagating its throw or
        /// publishing `done = true`, so later unwinding must not call return.
        enabled: bool,
    },
}

struct CallFrame {
    stack: Vec<Value>,
    regions: Vec<UnwindRegion>,
    pc: usize,
    _caller_realm: Option<ContextId>,
    /// The realm captured by the executing bytecode.
    _callee_realm: Option<ContextId>,
    _current_function: Option<ObjectRef>,
    this_value: Value,
    normalized_this: Option<Value>,
    new_target: Value,
    strict: bool,
    callee_global: Option<ObjectRef>,
}

impl CallFrame {
    fn new(max_stack: usize) -> Self {
        Self {
            stack: Vec::with_capacity(max_stack),
            regions: Vec::new(),
            pc: 0,
            _caller_realm: None,
            _callee_realm: None,
            _current_function: None,
            this_value: Value::Undefined,
            normalized_this: None,
            new_target: Value::Undefined,
            strict: true,
            callee_global: None,
        }
    }

    fn new_in_realm(
        metadata: FunctionMetadata,
        caller_realm: ContextId,
        callee_realm: ContextId,
        current_function: ObjectRef,
        this_value: Value,
        new_target: Value,
        callee_global: ObjectRef,
    ) -> Self {
        Self {
            stack: Vec::with_capacity(usize::from(metadata.max_stack)),
            regions: Vec::new(),
            pc: 0,
            _caller_realm: Some(caller_realm),
            _callee_realm: Some(callee_realm),
            _current_function: Some(current_function),
            this_value,
            normalized_this: None,
            new_target,
            strict: metadata.strict,
            callee_global: Some(callee_global),
        }
    }

    fn execute(
        mut self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        loop {
            let raised = match self.execute_inner(code, host) {
                Ok(Completion::Return(value)) => return Ok(Completion::Return(value)),
                Ok(Completion::Throw(value)) => value,
                Err(error) if NativeErrorKind::from_javascript_error(error.kind()).is_some() => {
                    host.materialize_error(error)?
                }
                Err(error) => return Err(error),
            };
            if let Some(completion) = self.raise(raised, host, code.len())? {
                return Ok(completion);
            }
        }
    }

    // Keep cold host bridges behind one call site instead of adding large
    // Result temporaries to `execute_inner`'s interpreter
    // frame. This preserves the proven-safe recursive native/callback depth on
    // the 2 MiB libtest stack.
    #[inline(never)]
    fn execute_cold_instruction(
        &mut self,
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        match instruction {
            Instruction::Arguments(kind) => match host.create_arguments(*kind)? {
                Completion::Return(arguments) => self.stack.push(arguments),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::VariableEnvironment => match host.create_variable_environment()? {
                Completion::Return(environment) => self.stack.push(environment),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::HasEvalVariable { source, name } => {
                match host.has_eval_variable(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetEvalVariable { source, name } => {
                match host.get_eval_variable(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutEvalVariable { source, name } => {
                let value = self.pop()?;
                if let Completion::Throw(value) = host.put_eval_variable(*source, *name, value)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::DeleteEvalVariable { source, name } => {
                match host.delete_eval_variable(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DefineEvalVariable { source, name } => {
                let value = self.pop()?;
                if let Completion::Throw(value) =
                    host.define_eval_variable(*source, *name, value)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::ToObject => {
                let value = self.pop()?;
                match value {
                    value @ Value::Object(_) => self.stack.push(value),
                    Value::Null | Value::Undefined => {
                        return Err(Error::new(ErrorKind::Type, "cannot convert to object"));
                    }
                    primitive => self.stack.push(host.box_primitive(primitive)?),
                }
            }
            Instruction::HasDynamicBinding { source, name } => {
                match host.has_dynamic_binding(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetDynamicBinding { source, name } => {
                match host.get_dynamic_binding(*source, *name, self.strict)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutDynamicBinding { source, name } => {
                let value = self.pop()?;
                if let Completion::Throw(value) =
                    host.put_dynamic_binding(*source, *name, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::DeleteDynamicBinding { source, name } => {
                match host.delete_dynamic_binding(*source, *name)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::DynamicEnvironmentObject(source) => {
                match host.dynamic_environment_object(*source)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::GetRefValue(name) | Instruction::GetRefValueUndef(name) => {
                let environment = self.pop()?;
                let retained_environment = environment.clone();
                let strict = matches!(instruction, Instruction::GetRefValue(_)) && self.strict;
                match host.get_ref_value(environment, *name, strict)? {
                    Completion::Return(value) => {
                        self.stack.push(retained_environment);
                        self.stack.push(value);
                    }
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::PutRefValue(name) => {
                let (environment, value) = self.pop_pair()?;
                if let Completion::Throw(value) =
                    host.put_ref_value(environment, *name, value, self.strict)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Object => match host.object()? {
                Completion::Return(object) => self.stack.push(object),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::RegExp(index) => match host.create_regexp(*index)? {
                Completion::Return(object) => self.stack.push(object),
                Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
            },
            Instruction::SetNameComputed => {
                let key_index = self
                    .stack
                    .len()
                    .checked_sub(2)
                    .ok_or_else(|| Error::internal("set computed name without a key"))?;
                let value = self
                    .stack
                    .last()
                    .ok_or_else(|| Error::internal("set computed name on an empty stack"))?;
                let key = self
                    .stack
                    .get(key_index)
                    .ok_or_else(|| Error::internal("set computed name without a key"))?;
                host.set_function_name_computed(value, key)?;
            }
            Instruction::SetProto => {
                let (object, prototype) = self.pop_pair()?;
                let retained_object = object.clone();
                match host.set_object_prototype(object, prototype)? {
                    Completion::Return(_) => self.stack.push(retained_object),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::CopyDataProperties => {
                let (target, source) = self.pop_pair()?;
                let retained_target = target.clone();
                match host.copy_data_properties(target, source)? {
                    Completion::Return(_) => self.stack.push(retained_target),
                    Completion::Throw(value) => return Ok(Some(Completion::Throw(value))),
                }
            }
            Instruction::ForInStart => {
                let value = self.pop()?;
                match host.for_in_start(value)? {
                    ForInStartOutcome::Iterator(iterator) => self.stack.push(iterator),
                    ForInStartOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::ForInNext => {
                let iterator = self
                    .stack
                    .last()
                    .cloned()
                    .ok_or_else(|| Error::internal("for-in next without an iterator"))?;
                match host.for_in_next(iterator)? {
                    ForInNextOutcome::Result { value, done } => {
                        self.stack.push(value);
                        self.stack.push(Value::Bool(done));
                    }
                    ForInNextOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            _ => {
                return Err(Error::internal("hot instruction reached cold VM dispatch"));
            }
        }
        Ok(None)
    }

    // Calls recursively enter bytecode/native execution. Keep their argument
    // vectors and host-completion temporaries out of the interpreter loop's
    // frame so each nested JavaScript call retains only the hot dispatch
    // state on the native stack.
    #[inline(never)]
    fn execute_call_instruction(
        &mut self,
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        let completion = match instruction {
            Instruction::Call(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 1)?;
                let function = self.pop()?;
                host.call(function, Value::Undefined, arguments)?
            }
            Instruction::Eval {
                argument_count,
                environment,
            } => {
                let arguments = self.take_call_arguments(*argument_count, 1)?;
                let function = self.pop()?;
                if host.is_original_eval(&function)? {
                    let input = arguments.first().cloned().unwrap_or(Value::Undefined);
                    // QuickJS only enters the compiler for primitive String
                    // input. Preserve that lazy boundary for sloppy `this`:
                    // non-String eval must return its input without allocating
                    // a primitive wrapper or touching caller bindings.
                    let this_value = if matches!(input, Value::String(_)) {
                        self.normalized_this(host)?
                    } else {
                        self.this_value.clone()
                    };
                    host.direct_eval(DirectEvalInvocation {
                        input,
                        environment: *environment,
                        this_value,
                        new_target: self.new_target.clone(),
                        caller_strict: self.strict,
                    })?
                } else {
                    host.call(function, Value::Undefined, arguments)?
                }
            }
            Instruction::CallMethod(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 2)?;
                let function = self.pop()?;
                let receiver = self.pop()?;
                host.call(function, receiver, arguments)?
            }
            Instruction::Construct(argument_count) => {
                let arguments = self.take_call_arguments(*argument_count, 2)?;
                let new_target = self.pop()?;
                let function = self.pop()?;
                host.construct(function, new_target, arguments)?
            }
            _ => {
                return Err(Error::internal(
                    "non-call instruction reached VM call dispatch",
                ));
            }
        };
        match completion {
            Completion::Return(value) => {
                self.stack.push(value);
                Ok(None)
            }
            Completion::Throw(value) => Ok(Some(Completion::Throw(value))),
        }
    }

    // Numeric coercion and BigInt paths instantiate several large generic
    // result temporaries in debug builds. Isolate the whole family so ordinary
    // bytecode calls do not reserve those slots in every recursive VM frame.
    #[inline(never)]
    fn execute_numeric_instruction(
        &mut self,
        instruction: &Instruction,
        host: &mut impl VmHost,
    ) -> Result<Option<Completion>, Error> {
        match instruction {
            Instruction::Neg => {
                if let OperationOutcome::Throw(value) = self.neg(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Plus => {
                if let OperationOutcome::Throw(value) = self.unary_plus(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Inc | Instruction::Dec => {
                let increment = matches!(instruction, Instruction::Inc);
                if let OperationOutcome::Throw(value) =
                    self.update_numeric(host, increment, false)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::PostInc | Instruction::PostDec => {
                let increment = matches!(instruction, Instruction::PostInc);
                if let OperationOutcome::Throw(value) =
                    self.update_numeric(host, increment, true)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitNot => {
                if let OperationOutcome::Throw(value) = self.bit_not(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Not => {
                let value = self.pop()?;
                self.stack.push(Value::Bool(!value.to_boolean()));
            }
            Instruction::TypeOf => {
                let value = self.pop()?;
                self.stack
                    .push(Value::String(JsString::from_static(host.type_of(&value)?)));
            }
            Instruction::IsUndefinedOrNull => {
                let value = self.pop()?;
                self.stack
                    .push(Value::Bool(matches!(value, Value::Undefined | Value::Null)));
            }
            Instruction::Add => {
                if let OperationOutcome::Throw(value) = self.add(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Sub => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left - right, JsBigInt::sub)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Mul => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left * right, JsBigInt::mul)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Div => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left / right, JsBigInt::div)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Mod => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, |left, right| left % right, JsBigInt::rem)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Pow => {
                if let OperationOutcome::Throw(value) =
                    self.binary_numeric(host, crate::number::pow, JsBigInt::pow)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Shl => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| {
                        f64::from(
                            number_to_int32(left).wrapping_shl(number_to_uint32(right) & 0x1f),
                        )
                    },
                    JsBigInt::shl,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Sar => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| {
                        f64::from(number_to_int32(left) >> (number_to_uint32(right) & 0x1f))
                    },
                    JsBigInt::shr,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Shr => {
                if let OperationOutcome::Throw(value) = self.unsigned_shift_right(host)? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitAnd => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| f64::from(number_to_int32(left) & number_to_int32(right)),
                    JsBigInt::bit_and,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitXor => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| f64::from(number_to_int32(left) ^ number_to_int32(right)),
                    JsBigInt::bit_xor,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::BitOr => {
                if let OperationOutcome::Throw(value) = self.binary_numeric(
                    host,
                    |left, right| f64::from(number_to_int32(left) | number_to_int32(right)),
                    JsBigInt::bit_or,
                )? {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Eq => {
                let (left, right) = self.pop_pair()?;
                match abstract_equal(host, left, right)? {
                    OperationOutcome::Value(equal) => self.stack.push(Value::Bool(equal)),
                    OperationOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::StrictEq => {
                let (left, right) = self.pop_pair()?;
                self.stack.push(Value::Bool(left.strict_equal(&right)));
            }
            Instruction::Neq => {
                let (left, right) = self.pop_pair()?;
                match abstract_equal(host, left, right)? {
                    OperationOutcome::Value(equal) => self.stack.push(Value::Bool(!equal)),
                    OperationOutcome::Throw(value) => {
                        return Ok(Some(Completion::Throw(value)));
                    }
                }
            }
            Instruction::StrictNeq => {
                let (left, right) = self.pop_pair()?;
                self.stack.push(Value::Bool(!left.strict_equal(&right)));
            }
            Instruction::Lt => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_lt)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Lte => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_le)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Gt => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_gt)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            Instruction::Gte => {
                if let OperationOutcome::Throw(value) =
                    self.compare(host, std::cmp::Ordering::is_ge)?
                {
                    return Ok(Some(Completion::Throw(value)));
                }
            }
            _ => {
                return Err(Error::internal(
                    "non-numeric instruction reached VM numeric dispatch",
                ));
            }
        }
        Ok(None)
    }

    fn execute_inner(
        &mut self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        loop {
            let instruction = code
                .get(self.pc)
                .ok_or_else(|| Error::internal("bytecode ended without return"))?;
            host.update_active_bytecode_pc(BytecodePc::new(self.pc))?;
            self.pc = self
                .pc
                .checked_add(1)
                .ok_or_else(|| Error::internal("program counter overflow"))?;

            if matches!(
                instruction,
                Instruction::Arguments(_)
                    | Instruction::VariableEnvironment
                    | Instruction::HasEvalVariable { .. }
                    | Instruction::GetEvalVariable { .. }
                    | Instruction::PutEvalVariable { .. }
                    | Instruction::DeleteEvalVariable { .. }
                    | Instruction::DefineEvalVariable { .. }
                    | Instruction::ToObject
                    | Instruction::HasDynamicBinding { .. }
                    | Instruction::GetDynamicBinding { .. }
                    | Instruction::PutDynamicBinding { .. }
                    | Instruction::DeleteDynamicBinding { .. }
                    | Instruction::DynamicEnvironmentObject(_)
                    | Instruction::GetRefValue(_)
                    | Instruction::GetRefValueUndef(_)
                    | Instruction::PutRefValue(_)
                    | Instruction::Object
                    | Instruction::RegExp(_)
                    | Instruction::SetNameComputed
                    | Instruction::SetProto
                    | Instruction::CopyDataProperties
                    | Instruction::ForInStart
                    | Instruction::ForInNext
            ) {
                if let Some(completion) = self.execute_cold_instruction(instruction, host)? {
                    return Ok(completion);
                }
                continue;
            }

            if matches!(
                instruction,
                Instruction::Call(_)
                    | Instruction::Eval { .. }
                    | Instruction::CallMethod(_)
                    | Instruction::Construct(_)
            ) {
                if let Some(completion) = self.execute_call_instruction(instruction, host)? {
                    return Ok(completion);
                }
                continue;
            }

            if matches!(
                instruction,
                Instruction::Neg
                    | Instruction::Plus
                    | Instruction::Inc
                    | Instruction::Dec
                    | Instruction::PostInc
                    | Instruction::PostDec
                    | Instruction::BitNot
                    | Instruction::Not
                    | Instruction::TypeOf
                    | Instruction::IsUndefinedOrNull
                    | Instruction::Add
                    | Instruction::Sub
                    | Instruction::Mul
                    | Instruction::Div
                    | Instruction::Mod
                    | Instruction::Pow
                    | Instruction::Shl
                    | Instruction::Sar
                    | Instruction::Shr
                    | Instruction::BitAnd
                    | Instruction::BitXor
                    | Instruction::BitOr
                    | Instruction::Eq
                    | Instruction::StrictEq
                    | Instruction::Neq
                    | Instruction::StrictNeq
                    | Instruction::Lt
                    | Instruction::Lte
                    | Instruction::Gt
                    | Instruction::Gte
            ) {
                if let Some(completion) = self.execute_numeric_instruction(instruction, host)? {
                    return Ok(completion);
                }
                continue;
            }

            match instruction {
                Instruction::Nop => {}
                Instruction::PushI32(value) => self.stack.push(Value::Int(*value)),
                Instruction::PushConst(index) => {
                    self.stack.push(host.load_constant(*index)?);
                }
                Instruction::FClosure(index) => {
                    self.stack.push(host.instantiate_closure(*index)?);
                }
                Instruction::ArrayFrom(element_count) => {
                    let element_count = usize::from(*element_count);
                    let first = self
                        .stack
                        .len()
                        .checked_sub(element_count)
                        .ok_or_else(|| Error::internal("array_from stack underflow"))?;
                    let elements = self.stack.drain(first..).collect();
                    match host.array_from(elements)? {
                        Completion::Return(array) => self.stack.push(array),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::Arguments(_) => {
                    unreachable!("arguments-object dispatch was bypassed")
                }
                Instruction::VariableEnvironment
                | Instruction::HasEvalVariable { .. }
                | Instruction::GetEvalVariable { .. }
                | Instruction::PutEvalVariable { .. }
                | Instruction::DeleteEvalVariable { .. }
                | Instruction::DefineEvalVariable { .. }
                | Instruction::ToObject
                | Instruction::HasDynamicBinding { .. }
                | Instruction::GetDynamicBinding { .. }
                | Instruction::PutDynamicBinding { .. }
                | Instruction::DeleteDynamicBinding { .. }
                | Instruction::DynamicEnvironmentObject(_)
                | Instruction::GetRefValue(_)
                | Instruction::GetRefValueUndef(_)
                | Instruction::PutRefValue(_) => {
                    unreachable!("eval variable-object dispatch was bypassed")
                }
                Instruction::Object => unreachable!("object literal dispatch was bypassed"),
                Instruction::RegExp(_) => {
                    unreachable!("RegExp literal dispatch was bypassed")
                }
                Instruction::SetName(index) => {
                    let value = self
                        .stack
                        .last()
                        .ok_or_else(|| Error::internal("set name on an empty stack"))?;
                    host.set_function_name(value, *index)?;
                }
                Instruction::SetNameComputed => {
                    unreachable!("computed-name literal dispatch was bypassed")
                }
                Instruction::ThrowReadOnly(index) => {
                    self.pop()?;
                    return Err(host.read_only_error(*index)?);
                }
                Instruction::ThrowRedeclaration(index) => {
                    return Err(host.redeclaration_error(*index)?);
                }
                Instruction::Undefined => self.stack.push(Value::Undefined),
                Instruction::Null => self.stack.push(Value::Null),
                Instruction::PushFalse => self.stack.push(Value::Bool(false)),
                Instruction::PushTrue => self.stack.push(Value::Bool(true)),
                Instruction::PushThis => {
                    let value = self.normalized_this(host)?;
                    self.stack.push(value);
                }
                Instruction::PushNewTarget => self.stack.push(self.new_target.clone()),
                Instruction::GetLocal(index) => {
                    self.stack.push(host.get_local(*index)?);
                }
                Instruction::PutLocal(index) => {
                    let value = self.pop()?;
                    host.put_local(*index, value)?;
                }
                Instruction::SetLocal(index) => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("set local on an empty stack"))?;
                    host.put_local(*index, value)?;
                }
                Instruction::SetLocalUninitialized(index) => {
                    host.set_local_uninitialized(*index)?;
                }
                Instruction::GetLocalCheck(index) => {
                    self.stack.push(host.get_local_checked(*index)?);
                }
                Instruction::InitializeLocal(index) => {
                    let value = self.pop()?;
                    host.initialize_local(*index, value)?;
                }
                Instruction::PutLocalCheck(index) => {
                    let value = self.pop()?;
                    host.put_local_checked(*index, value)?;
                }
                Instruction::SetLocalCheck(index) => {
                    let value =
                        self.stack.last().cloned().ok_or_else(|| {
                            Error::internal("set lexical local on an empty stack")
                        })?;
                    host.put_local_checked(*index, value)?;
                }
                Instruction::GetArg(index) => {
                    self.stack.push(host.get_argument(*index)?);
                }
                Instruction::PutArg(index) => {
                    let value = self.pop()?;
                    host.put_argument(*index, value)?;
                }
                Instruction::SetArg(index) => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("set argument on an empty stack"))?;
                    host.put_argument(*index, value)?;
                }
                Instruction::GetVarRef(index) => {
                    self.stack.push(host.get_var_ref(*index)?);
                }
                Instruction::PutVarRef(index) => {
                    let value = self.pop()?;
                    host.put_var_ref(*index, value)?;
                }
                Instruction::SetVarRef(index) => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("set VarRef on an empty stack"))?;
                    host.put_var_ref(*index, value)?;
                }
                Instruction::GetVarRefCheck(index) => {
                    self.stack.push(host.get_var_ref_checked(*index)?);
                }
                Instruction::PutVarRefCheck(index) => {
                    let value = self.pop()?;
                    host.put_var_ref_checked(*index, value)?;
                }
                Instruction::CloseLocal(index) => {
                    host.close_local(*index)?;
                }
                Instruction::GetVar(index) | Instruction::GetVarUndef(index) => {
                    let throw_if_missing = matches!(instruction, Instruction::GetVar(_));
                    match host.get_global_var(*index, throw_if_missing)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::DeleteVar(index) => match host.delete_global_var(*index)? {
                    Completion::Return(value) => self.stack.push(value),
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                },
                Instruction::PutVar(index) | Instruction::PutVarInit(index) => {
                    let value = self.pop()?;
                    let initialize = matches!(instruction, Instruction::PutVarInit(_));
                    if let Completion::Throw(value) =
                        host.put_global_var(*index, value, initialize, self.strict)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::GetField(index) | Instruction::GetField2(index) => {
                    let keep_receiver = matches!(instruction, Instruction::GetField2(_));
                    let base = self.pop()?;
                    let receiver = keep_receiver.then(|| base.clone());
                    match host.get_field(base, *index)? {
                        Completion::Return(value) => {
                            if let Some(receiver) = receiver {
                                self.stack.push(receiver);
                            }
                            self.stack.push(value);
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::GetArrayEl | Instruction::GetArrayEl2 => {
                    let keep_receiver = matches!(instruction, Instruction::GetArrayEl2);
                    let (base, key) = self.pop_pair()?;
                    let receiver = keep_receiver.then(|| base.clone());
                    match host.get_property(base, key)? {
                        Completion::Return(value) => {
                            if let Some(receiver) = receiver {
                                self.stack.push(receiver);
                            }
                            self.stack.push(value);
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::GetArrayEl3 => {
                    let (base, key) = self.pop_pair()?;
                    let key_is_already_canonical =
                        matches!(key, Value::Int(_) | Value::String(_) | Value::Symbol(_));
                    if matches!(base, Value::Null | Value::Undefined) && !key_is_already_canonical {
                        // QuickJS `get_array_el3` performs this special check
                        // before ToPropertyKey for non-fast key tags.
                        return Err(Error::new(ErrorKind::Type, "value has no property"));
                    }
                    if matches!(base, Value::Null | Value::Undefined) {
                        // Reuse the ordinary read path solely for its exact
                        // fast-key nullish diagnostic.
                        match host.get_property(base, key)? {
                            Completion::Return(_) => {
                                return Err(Error::internal(
                                    "nullish property read unexpectedly completed",
                                ));
                            }
                            Completion::Throw(value) => return Ok(Completion::Throw(value)),
                        }
                    }
                    let key = match host.convert_property_key(key)? {
                        Completion::Return(key) => key,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    let value = match host.get_property(base.clone(), key.clone())? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    self.stack.push(base);
                    self.stack.push(key);
                    self.stack.push(value);
                }
                Instruction::ToPropKey => {
                    let key = self.pop()?;
                    match host.convert_property_key(key)? {
                        Completion::Return(key) => self.stack.push(key),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::Insert2 => {
                    let (base, value) = self.pop_pair()?;
                    self.stack.push(value.clone());
                    self.stack.push(base);
                    self.stack.push(value);
                }
                Instruction::Insert3 => {
                    let value = self.pop()?;
                    let key = self.pop()?;
                    let base = self.pop()?;
                    self.stack.push(value.clone());
                    self.stack.push(base);
                    self.stack.push(key);
                    self.stack.push(value);
                }
                Instruction::Perm3 => {
                    let new_value = self.pop()?;
                    let old_value = self.pop()?;
                    let base = self.pop()?;
                    self.stack.push(old_value);
                    self.stack.push(base);
                    self.stack.push(new_value);
                }
                Instruction::Perm4 => {
                    let new_value = self.pop()?;
                    let old_value = self.pop()?;
                    let key = self.pop()?;
                    let base = self.pop()?;
                    self.stack.push(old_value);
                    self.stack.push(base);
                    self.stack.push(key);
                    self.stack.push(new_value);
                }
                Instruction::PutField(index) => {
                    let (base, value) = self.pop_pair()?;
                    if let Completion::Throw(value) =
                        host.set_field(base, *index, value, self.strict)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::PutArrayEl => {
                    let value = self.pop()?;
                    let key = self.pop()?;
                    let base = self.pop()?;
                    if let Completion::Throw(value) =
                        host.set_property(base, key, value, self.strict)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::DefineField(index) => {
                    let (base, value) = self.pop_pair()?;
                    let retained_base = base.clone();
                    match host.define_field(base, *index, value)? {
                        Completion::Return(_) => self.stack.push(retained_base),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::DefineArrayEl => {
                    let value = self.pop()?;
                    let index = self.pop()?;
                    let base = self.pop()?;
                    let retained_base = base.clone();
                    let retained_index = index.clone();
                    match host.define_array_element(base, index, value)? {
                        Completion::Return(_) => {
                            self.stack.push(retained_base);
                            self.stack.push(retained_index);
                        }
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::SetProto => unreachable!("prototype literal dispatch was bypassed"),
                Instruction::CopyDataProperties => {
                    unreachable!("spread literal dispatch was bypassed")
                }
                Instruction::Append => {
                    let iterable = self.pop()?;
                    let index = self.pop()?;
                    let array = self.pop()?;
                    match Self::append_iterable(host, array, index, iterable)? {
                        OperationOutcome::Value((array, index)) => {
                            self.stack.push(array);
                            self.stack.push(index);
                        }
                        OperationOutcome::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
                Instruction::Delete => {
                    let (base, key) = self.pop_pair()?;
                    match host.delete_property(base, key, self.strict)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::Drop => {
                    self.pop()?;
                }
                Instruction::Nip => {
                    let (_, value) = self.pop_pair()?;
                    self.stack.push(value);
                }
                Instruction::Dup => {
                    let value = self
                        .stack
                        .last()
                        .cloned()
                        .ok_or_else(|| Error::internal("dup on an empty stack"))?;
                    self.stack.push(value);
                }
                Instruction::Dup1 => {
                    let index = self
                        .stack
                        .len()
                        .checked_sub(2)
                        .ok_or_else(|| Error::internal("dup1 needs two stack values"))?;
                    let value = self.stack[index].clone();
                    self.stack.insert(index + 1, value);
                }
                Instruction::Neg
                | Instruction::Plus
                | Instruction::Inc
                | Instruction::Dec
                | Instruction::PostInc
                | Instruction::PostDec
                | Instruction::BitNot
                | Instruction::Not
                | Instruction::TypeOf
                | Instruction::IsUndefinedOrNull
                | Instruction::Add
                | Instruction::Sub
                | Instruction::Mul
                | Instruction::Div
                | Instruction::Mod
                | Instruction::Pow
                | Instruction::Shl
                | Instruction::Sar
                | Instruction::Shr
                | Instruction::BitAnd
                | Instruction::BitXor
                | Instruction::BitOr
                | Instruction::Eq
                | Instruction::StrictEq
                | Instruction::Neq
                | Instruction::StrictNeq
                | Instruction::Lt
                | Instruction::Lte
                | Instruction::Gt
                | Instruction::Gte => unreachable!("numeric dispatch was bypassed"),
                Instruction::In => {
                    let (key, object) = self.pop_pair()?;
                    let Value::Object(object) = object else {
                        return Err(Error::new(ErrorKind::Type, "invalid 'in' operand"));
                    };
                    match host.has_property(key, object)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::InstanceOf => {
                    let (candidate, target) = self.pop_pair()?;
                    let Value::Object(target) = target else {
                        return Err(Error::new(
                            ErrorKind::Type,
                            "invalid 'instanceof' right operand",
                        ));
                    };
                    match host.is_instance_of(candidate, target)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::IfFalse(target) => {
                    if !self.pop()?.to_boolean() {
                        self.pc = checked_target(*target, code.len())?;
                    }
                }
                Instruction::IfTrue(target) => {
                    if self.pop()?.to_boolean() {
                        self.pc = checked_target(*target, code.len())?;
                    }
                }
                Instruction::Goto(target) => {
                    self.pc = checked_target(*target, code.len())?;
                }
                Instruction::Catch(target) => {
                    self.regions.push(UnwindRegion::Catch {
                        target: checked_target(*target, code.len())?,
                        stack_depth: self.stack.len(),
                    });
                }
                Instruction::DropCatch => {
                    let region = self
                        .regions
                        .pop()
                        .ok_or_else(|| Error::internal("DropCatch has no active catch handler"))?;
                    let UnwindRegion::Catch { stack_depth, .. } = region else {
                        return Err(Error::internal(
                            "DropCatch did not target the innermost unwind region",
                        ));
                    };
                    if self.stack.len() != stack_depth {
                        return Err(Error::internal(
                            "DropCatch did not reach its catch entry depth",
                        ));
                    }
                }
                Instruction::NipCatch => {
                    let region = *self
                        .regions
                        .last()
                        .ok_or_else(|| Error::internal("NipCatch has no active catch handler"))?;
                    let UnwindRegion::Catch { stack_depth, .. } = region else {
                        return Err(Error::internal(
                            "NipCatch did not target the innermost unwind region",
                        ));
                    };
                    if self.stack.len() <= stack_depth {
                        return Err(Error::internal(
                            "NipCatch has no value above its catch marker",
                        ));
                    }
                    // A return crossing a try/finally uses NipCatch to retain
                    // its pending value while removing the private handler.
                    // QuickJS does not synthesize CloseLocal along that edge;
                    // if the finally body overrides the return, the same frame
                    // may re-enter those captured lexical slots.
                    host.prepare_captured_local_reuse()?;
                    self.regions.pop();
                    let value = self.pop()?;
                    self.stack.truncate(stack_depth);
                    self.stack.push(value);
                }
                Instruction::Gosub(target) => {
                    let return_pc = i32::try_from(self.pc)
                        .map_err(|_| Error::internal("gosub return PC does not fit Int"))?;
                    self.stack.push(Value::Int(return_pc));
                    self.pc = checked_target(*target, code.len())?;
                }
                Instruction::Ret => {
                    let Value::Int(target) = self.pop()? else {
                        return Err(Error::internal("invalid ret value"));
                    };
                    let target = usize::try_from(target)
                        .map_err(|_| Error::internal("invalid ret value"))?;
                    if target >= code.len() {
                        return Err(Error::internal("invalid ret value"));
                    }
                    self.pc = target;
                }
                Instruction::DropGosub => {
                    if !matches!(self.pop()?, Value::Int(_)) {
                        return Err(Error::internal("invalid gosub cleanup value"));
                    }
                }
                Instruction::ForOfStart => {
                    let iterable = self.pop()?;
                    match host.for_of_start(iterable)? {
                        ForOfStartOutcome::Record {
                            iterator,
                            next_method,
                        } => {
                            let record_base = self.stack.len();
                            self.stack.push(iterator);
                            self.stack.push(next_method);
                            self.regions.push(UnwindRegion::Iterator {
                                record_base,
                                enabled: true,
                            });
                        }
                        ForOfStartOutcome::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
                Instruction::ForOfNext(offset) => {
                    let (record_base, enabled) = match self.regions.last() {
                        Some(UnwindRegion::Iterator {
                            record_base,
                            enabled,
                        }) => (*record_base, *enabled),
                        _ => {
                            return Err(Error::internal(
                                "ForOfNext has no innermost iterator region",
                            ));
                        }
                    };
                    let expected_depth = record_base
                        .checked_add(2)
                        .and_then(|depth| depth.checked_add(usize::from(*offset)))
                        .ok_or_else(|| Error::internal("for-of offset overflow"))?;
                    if self.stack.len() != expected_depth {
                        return Err(Error::internal(
                            "ForOfNext offset does not reach its iterator record",
                        ));
                    }
                    if !enabled {
                        self.stack.push(Value::Undefined);
                        self.stack.push(Value::Bool(true));
                        continue;
                    }
                    let iterator = self
                        .stack
                        .get(record_base)
                        .cloned()
                        .ok_or_else(|| Error::internal("iterator record is truncated"))?;
                    let next_method = self
                        .stack
                        .get(record_base + 1)
                        .cloned()
                        .ok_or_else(|| Error::internal("iterator record is truncated"))?;
                    match host.for_of_next(iterator, next_method)? {
                        ForOfNextOutcome::Result { value, done } => {
                            if done {
                                self.disable_iterator_region(record_base)?;
                            }
                            self.stack.push(value);
                            self.stack.push(Value::Bool(done));
                        }
                        ForOfNextOutcome::Throw(value) => {
                            self.disable_iterator_region(record_base)?;
                            return Ok(Completion::Throw(value));
                        }
                    }
                }
                Instruction::ForInStart | Instruction::ForInNext => {
                    unreachable!("for-in dispatch was bypassed")
                }
                Instruction::IteratorClose => {
                    let (iterator, enabled) = self.take_iterator_region(false)?;
                    if enabled {
                        match host.iterator_close(iterator, false)? {
                            IteratorCloseOutcome::Closed => {}
                            IteratorCloseOutcome::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        }
                    }
                }
                Instruction::IteratorClosePreserve => {
                    let (iterator, enabled) = self.take_iterator_region(true)?;
                    if enabled {
                        match host.iterator_close(iterator, false)? {
                            IteratorCloseOutcome::Closed => {}
                            IteratorCloseOutcome::Throw(value) => {
                                return Ok(Completion::Throw(value));
                            }
                        }
                    }
                }
                Instruction::Call(_)
                | Instruction::Eval { .. }
                | Instruction::CallMethod(_)
                | Instruction::Construct(_) => {
                    unreachable!("call dispatch was bypassed")
                }
                Instruction::Return => return self.pop().map(Completion::Return),
                Instruction::Throw => return self.pop().map(Completion::Throw),
            }
        }
    }

    /// Execute QuickJS `OP_append` without publishing a private iterator
    /// region on the frame stack. Unlike ordinary `for-of`, QuickJS closes an
    /// already-created iterator even when its cached `next` path throws while
    /// expanding an Array literal. A close failure cannot replace that
    /// pending throw.
    fn append_iterable(
        host: &mut impl VmHost,
        array: Value,
        mut index: Value,
        iterable: Value,
    ) -> Result<OperationOutcome<(Value, Value)>, Error> {
        let (iterator, next_method) = match host.for_of_start(iterable)? {
            ForOfStartOutcome::Record {
                iterator,
                next_method,
            } => (iterator, next_method),
            ForOfStartOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };

        loop {
            let value = match host.for_of_next(iterator.clone(), next_method.clone())? {
                ForOfNextOutcome::Result { done: true, .. } => {
                    return Ok(OperationOutcome::Value((array, index)));
                }
                ForOfNextOutcome::Result { value, done: false } => value,
                ForOfNextOutcome::Throw(value) => {
                    match host.iterator_close(iterator, true)? {
                        IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                    }
                    return Ok(OperationOutcome::Throw(value));
                }
            };

            if let Err(error) = validate_array_literal_index(&index) {
                let pending = host.materialize_error(error)?;
                match host.iterator_close(iterator, true)? {
                    IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                }
                return Ok(OperationOutcome::Throw(pending));
            }

            match host.define_array_element(array.clone(), index.clone(), value)? {
                Completion::Return(_) => {}
                Completion::Throw(value) => {
                    match host.iterator_close(iterator, true)? {
                        IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                    }
                    return Ok(OperationOutcome::Throw(value));
                }
            }
            index = increment_array_literal_index(index)?;
        }
    }

    fn raise(
        &mut self,
        value: Value,
        host: &mut impl VmHost,
        code_len: usize,
    ) -> Result<Option<Completion>, Error> {
        host.ensure_backtrace(&value)?;
        loop {
            let Some(region) = self.regions.pop() else {
                return Ok(Some(Completion::Throw(value)));
            };
            match region {
                UnwindRegion::Catch {
                    target,
                    stack_depth,
                } => {
                    host.prepare_captured_local_reuse()?;
                    if self.stack.len() < stack_depth {
                        return Err(Error::internal(
                            "exception handler stack depth exceeds the VM stack",
                        ));
                    }
                    self.stack.truncate(stack_depth);
                    self.stack.push(value);
                    self.pc = checked_target(
                        u32::try_from(target)
                            .map_err(|_| Error::internal("catch target overflow"))?,
                        code_len,
                    )?;
                    return Ok(None);
                }
                UnwindRegion::Iterator {
                    record_base,
                    enabled,
                } => {
                    let required_depth = record_base
                        .checked_add(2)
                        .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
                    if self.stack.len() < required_depth {
                        return Err(Error::internal(
                            "iterator unwind region exceeds the VM stack",
                        ));
                    }
                    let iterator = self.stack[record_base].clone();
                    self.stack.truncate(record_base);
                    if enabled {
                        // IteratorClose with a pending throw never replaces
                        // that throw, including when `return` lookup/call
                        // itself throws. Engine invariant failures still
                        // escape through `Err`.
                        match host.iterator_close(iterator, true)? {
                            IteratorCloseOutcome::Closed | IteratorCloseOutcome::Throw(_) => {}
                        }
                    }
                }
            }
        }
    }

    fn disable_iterator_region(&mut self, record_base: usize) -> Result<(), Error> {
        let Some(UnwindRegion::Iterator {
            record_base: active_base,
            enabled,
        }) = self.regions.last_mut()
        else {
            return Err(Error::internal(
                "ForOfNext has no innermost iterator region",
            ));
        };
        if *active_base != record_base {
            return Err(Error::internal(
                "iterator unwind region changed during next",
            ));
        }
        *enabled = false;
        let iterator = self
            .stack
            .get_mut(record_base)
            .ok_or_else(|| Error::internal("iterator record is truncated"))?;
        *iterator = Value::Undefined;
        Ok(())
    }

    /// Remove the innermost iterator region. When `preserve_top` is true, the
    /// top operand replaces the complete iterator record and all intermediate
    /// values, matching an abrupt return crossing a for-of loop.
    fn take_iterator_region(&mut self, preserve_top: bool) -> Result<(Value, bool), Error> {
        let region = self.regions.pop().ok_or_else(|| {
            Error::internal(if preserve_top {
                "IteratorClosePreserve has no iterator region"
            } else {
                "IteratorClose has no iterator region"
            })
        })?;
        let UnwindRegion::Iterator {
            record_base,
            enabled,
        } = region
        else {
            return Err(Error::internal(if preserve_top {
                "IteratorClosePreserve did not target the innermost unwind region"
            } else {
                "IteratorClose did not target the innermost unwind region"
            }));
        };
        let record_end = record_base
            .checked_add(2)
            .ok_or_else(|| Error::internal("iterator record depth overflow"))?;
        if preserve_top {
            if self.stack.len() <= record_end {
                return Err(Error::internal(
                    "IteratorClosePreserve has no value above its iterator marker",
                ));
            }
        } else if self.stack.len() != record_end {
            return Err(Error::internal(
                "IteratorClose did not reach its iterator record",
            ));
        }
        let iterator = self
            .stack
            .get(record_base)
            .cloned()
            .ok_or_else(|| Error::internal("iterator record is truncated"))?;
        let preserved = preserve_top.then(|| self.stack.pop()).flatten();
        self.stack.truncate(record_base);
        if let Some(value) = preserved {
            self.stack.push(value);
        }
        Ok((iterator, enabled))
    }

    fn pop(&mut self) -> Result<Value, Error> {
        self.stack
            .pop()
            .ok_or_else(|| Error::internal("bytecode stack underflow"))
    }

    fn take_call_arguments(
        &mut self,
        argument_count: u16,
        fixed_values: usize,
    ) -> Result<Vec<Value>, Error> {
        let argument_count = usize::from(argument_count);
        let required = argument_count
            .checked_add(fixed_values)
            .ok_or_else(|| Error::internal("call operand count overflow"))?;
        if self.stack.len() < required {
            return Err(Error::internal("call operands underflow the VM stack"));
        }
        let start = self.stack.len() - argument_count;
        Ok(self.stack.split_off(start))
    }

    fn normalized_this(&mut self, host: &mut impl VmHost) -> Result<Value, Error> {
        if let Some(value) = &self.normalized_this {
            return Ok(value.clone());
        }
        if self.strict || matches!(self.this_value, Value::Object(_)) {
            return Ok(self.this_value.clone());
        }
        if matches!(self.this_value, Value::Undefined | Value::Null) {
            return self
                .callee_global
                .as_ref()
                .cloned()
                .map(Value::Object)
                .ok_or_else(|| Error::internal("sloppy frame has no callee global object"));
        }
        let value = host.box_primitive(self.this_value.clone())?;
        self.normalized_this = Some(value.clone());
        Ok(value)
    }

    fn pop_pair(&mut self) -> Result<(Value, Value), Error> {
        let right = self.pop()?;
        let left = self.pop()?;
        Ok((left, right))
    }

    fn neg(&mut self, host: &mut impl VmHost) -> Result<OperationOutcome<()>, Error> {
        let operand = match host.to_primitive(self.pop()?, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match operand {
            Value::BigInt(value) => self
                .stack
                .push(Value::BigInt(value.neg().map_err(bigint_error)?)),
            value => self.stack.push(Value::number(-value.to_number()?)),
        }
        Ok(OperationOutcome::Value(()))
    }

    fn unary_plus(&mut self, host: &mut impl VmHost) -> Result<OperationOutcome<()>, Error> {
        let operand = match host.to_primitive(self.pop()?, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        if matches!(operand, Value::BigInt(_)) {
            return Err(Error::new(ErrorKind::Type, "bigint argument with unary +"));
        }
        self.stack.push(Value::number(operand.to_number()?));
        Ok(OperationOutcome::Value(()))
    }

    /// QuickJS `js_unary_arith_slow` / `js_post_inc_slow`. Postfix updates
    /// retain the converted numeric value, not the original string or object.
    fn update_numeric(
        &mut self,
        host: &mut impl VmHost,
        increment: bool,
        postfix: bool,
    ) -> Result<OperationOutcome<()>, Error> {
        let operand = match to_numeric(host, self.pop()?)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match operand {
            NumericValue::Number(old) => {
                let new = if increment { old + 1.0 } else { old - 1.0 };
                if postfix {
                    self.stack.push(Value::number(old));
                }
                self.stack.push(Value::number(new));
            }
            NumericValue::BigInt(old) => {
                let one = JsBigInt::from(1_i32);
                let new = if increment {
                    old.add(&one)
                } else {
                    old.update_decrement()
                }
                .map_err(bigint_error)?;
                if postfix {
                    self.stack.push(Value::BigInt(old));
                }
                self.stack.push(Value::BigInt(new));
            }
        }
        Ok(OperationOutcome::Value(()))
    }

    fn bit_not(&mut self, host: &mut impl VmHost) -> Result<OperationOutcome<()>, Error> {
        let operand = match to_numeric(host, self.pop()?)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match operand {
            NumericValue::BigInt(value) => self
                .stack
                .push(Value::BigInt(value.bit_not().map_err(bigint_error)?)),
            NumericValue::Number(value) => self.stack.push(Value::Int(!number_to_int32(value))),
        }
        Ok(OperationOutcome::Value(()))
    }

    fn unsigned_shift_right(
        &mut self,
        host: &mut impl VmHost,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match to_numeric(host, left)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match to_numeric(host, right)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let (NumericValue::Number(left), NumericValue::Number(right)) = (left, right) else {
            return Err(Error::new(
                ErrorKind::Type,
                "bigint operands are forbidden for >>>",
            ));
        };
        let result = number_to_uint32(left) >> (number_to_uint32(right) & 0x1f);
        self.stack.push(Value::number(f64::from(result)));
        Ok(OperationOutcome::Value(()))
    }

    fn binary_numeric(
        &mut self,
        host: &mut impl VmHost,
        number_operation: impl FnOnce(f64, f64) -> f64,
        bigint_operation: impl FnOnce(&JsBigInt, &JsBigInt) -> Result<JsBigInt, BigIntError>,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match to_numeric(host, left)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match to_numeric(host, right)? {
            OperationOutcome::Value(value) => value,
            OperationOutcome::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        match (left, right) {
            (NumericValue::BigInt(left), NumericValue::BigInt(right)) => {
                self.stack.push(Value::BigInt(
                    bigint_operation(&left, &right).map_err(bigint_error)?,
                ));
            }
            (NumericValue::BigInt(_), NumericValue::Number(_))
            | (NumericValue::Number(_), NumericValue::BigInt(_)) => {
                return Err(mixed_numeric_type_error());
            }
            (NumericValue::Number(left), NumericValue::Number(right)) => self
                .stack
                .push(Value::number(number_operation(left, right))),
        }
        Ok(OperationOutcome::Value(()))
    }

    fn compare(
        &mut self,
        host: &mut impl VmHost,
        operation: impl FnOnce(std::cmp::Ordering) -> bool,
    ) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match host.to_primitive(left, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match host.to_primitive(right, ToPrimitiveHint::Number)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let ordering = match (&left, &right) {
            (Value::String(left), Value::String(right)) => {
                Some(left.utf16_units().cmp(right.utf16_units()))
            }
            (Value::BigInt(left), Value::BigInt(right)) => Some(left.cmp(right)),
            (Value::BigInt(left), Value::String(right)) => {
                string_to_bigint(right).map(|right| left.cmp(&right))
            }
            (Value::String(left), Value::BigInt(right)) => {
                string_to_bigint(left).map(|left| left.cmp(right))
            }
            _ => {
                let left = to_numeric_primitive(left)?;
                let right = to_numeric_primitive(right)?;
                match (&left, &right) {
                    (NumericValue::BigInt(left), NumericValue::BigInt(right)) => {
                        Some(left.cmp(right))
                    }
                    (NumericValue::BigInt(left), NumericValue::Number(right)) => {
                        compare_bigint_number(left, *right)
                    }
                    (NumericValue::Number(left), NumericValue::BigInt(right)) => {
                        compare_bigint_number(right, *left).map(std::cmp::Ordering::reverse)
                    }
                    (NumericValue::Number(left), NumericValue::Number(right)) => {
                        left.partial_cmp(right)
                    }
                }
            }
        };
        self.stack
            .push(Value::Bool(ordering.is_some_and(operation)));
        Ok(OperationOutcome::Value(()))
    }

    fn add(&mut self, host: &mut impl VmHost) -> Result<OperationOutcome<()>, Error> {
        let (left, right) = self.pop_pair()?;
        let left = match host.to_primitive(left, ToPrimitiveHint::Default)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        let right = match host.to_primitive(right, ToPrimitiveHint::Default)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
        };
        if matches!(left, Value::String(_)) || matches!(right, Value::String(_)) {
            let left = match left {
                Value::String(value) => value,
                value => value.to_js_string()?,
            };
            let right = match right {
                Value::String(value) => value,
                value => value.to_js_string()?,
            };
            self.stack
                .push(Value::String(left.try_concat(&right).map_err(Error::from)?));
        } else {
            let left = to_numeric_primitive(left)?;
            let right = to_numeric_primitive(right)?;
            match (left, right) {
                (NumericValue::BigInt(left), NumericValue::BigInt(right)) => self
                    .stack
                    .push(Value::BigInt(left.add(&right).map_err(bigint_error)?)),
                (NumericValue::BigInt(_), NumericValue::Number(_))
                | (NumericValue::Number(_), NumericValue::BigInt(_)) => {
                    return Err(mixed_numeric_type_error());
                }
                (NumericValue::Number(left), NumericValue::Number(right)) => {
                    self.stack.push(Value::number(left + right));
                }
            }
        }
        Ok(OperationOutcome::Value(()))
    }
}

fn checked_target(target: u32, code_len: usize) -> Result<usize, Error> {
    let target = usize::try_from(target).map_err(|_| Error::internal("jump target overflow"))?;
    if target >= code_len {
        return Err(Error::internal("jump target is out of bounds"));
    }
    Ok(target)
}

fn validate_array_literal_index(index: &Value) -> Result<(), Error> {
    let value = match index {
        Value::Int(value) if *value >= 0 => {
            u32::try_from(*value).expect("non-negative i32 always fits u32")
        }
        Value::Float(value)
            if value.is_finite()
                && *value >= 0.0
                && *value <= f64::from(u32::MAX)
                && value.fract() == 0.0 =>
        {
            number_to_uint32(*value)
        }
        _ => {
            return Err(Error::internal(
                "Array literal dynamic index is not a non-negative uint32",
            ));
        }
    };
    if value == u32::MAX {
        return Err(Error::new(ErrorKind::Range, "invalid array length"));
    }
    Ok(())
}

fn increment_array_literal_index(index: Value) -> Result<Value, Error> {
    validate_array_literal_index(&index)?;
    let value = match index {
        Value::Int(value) => u32::try_from(value).expect("validated non-negative i32 fits u32"),
        Value::Float(value) => number_to_uint32(value),
        _ => unreachable!("validated Array literal index is numeric"),
    };
    Ok(Value::number(f64::from(value + 1)))
}

fn abstract_equal(
    host: &mut impl VmHost,
    mut left: Value,
    mut right: Value,
) -> Result<OperationOutcome<bool>, Error> {
    loop {
        if left.strict_equal(&right) {
            return Ok(OperationOutcome::Value(true));
        }
        match (&left, &right) {
            (Value::Null, Value::Undefined) | (Value::Undefined, Value::Null) => {
                return Ok(OperationOutcome::Value(true));
            }
            (Value::Int(_) | Value::Float(_), Value::String(_)) => {
                right = Value::number(right.to_number()?);
            }
            (Value::String(_), Value::Int(_) | Value::Float(_)) => {
                left = Value::number(left.to_number()?);
            }
            (Value::BigInt(left_bigint), Value::String(right_string)) => {
                return Ok(OperationOutcome::Value(
                    string_to_bigint(right_string).is_some_and(|right| &right == left_bigint),
                ));
            }
            (Value::String(left_string), Value::BigInt(right_bigint)) => {
                return Ok(OperationOutcome::Value(
                    string_to_bigint(left_string).is_some_and(|left| &left == right_bigint),
                ));
            }
            (Value::BigInt(left_bigint), Value::Int(_) | Value::Float(_)) => {
                return Ok(OperationOutcome::Value(
                    compare_bigint_number(left_bigint, right.to_number()?)
                        == Some(std::cmp::Ordering::Equal),
                ));
            }
            (Value::Int(_) | Value::Float(_), Value::BigInt(right_bigint)) => {
                return Ok(OperationOutcome::Value(
                    compare_bigint_number(right_bigint, left.to_number()?)
                        == Some(std::cmp::Ordering::Equal),
                ));
            }
            (Value::Bool(_), _) => left = Value::number(left.to_number()?),
            (_, Value::Bool(_)) => right = Value::number(right.to_number()?),
            (
                Value::Object(_),
                Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_),
            ) => match host.to_primitive(left, ToPrimitiveHint::Default)? {
                Completion::Return(value) => left = value,
                Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
            },
            (
                Value::Int(_)
                | Value::Float(_)
                | Value::BigInt(_)
                | Value::String(_)
                | Value::Symbol(_),
                Value::Object(_),
            ) => match host.to_primitive(right, ToPrimitiveHint::Default)? {
                Completion::Return(value) => right = value,
                Completion::Throw(value) => return Ok(OperationOutcome::Throw(value)),
            },
            _ => return Ok(OperationOutcome::Value(false)),
        }
    }
}

fn to_numeric(
    host: &mut impl VmHost,
    value: Value,
) -> Result<OperationOutcome<NumericValue>, Error> {
    match host.to_primitive(value, ToPrimitiveHint::Number)? {
        Completion::Return(value) => Ok(OperationOutcome::Value(to_numeric_primitive(value)?)),
        Completion::Throw(value) => Ok(OperationOutcome::Throw(value)),
    }
}

fn to_numeric_primitive(value: Value) -> Result<NumericValue, Error> {
    match value {
        Value::BigInt(value) => Ok(NumericValue::BigInt(value)),
        value => Ok(NumericValue::Number(value.to_number()?)),
    }
}

/// ECMAScript `ToInt32`, matching QuickJS's modulo-2^32 conversion for every
/// finite IEEE-754 input and its zero result for NaN and infinities.
fn number_to_int32(value: f64) -> i32 {
    crate::number::to_int32(value)
}

fn number_to_uint32(value: f64) -> u32 {
    u32::from_ne_bytes(number_to_int32(value).to_ne_bytes())
}

fn compare_bigint_number(bigint: &JsBigInt, number: f64) -> Option<std::cmp::Ordering> {
    if number.is_nan() {
        return None;
    }
    if number == f64::INFINITY {
        return Some(std::cmp::Ordering::Less);
    }
    if number == f64::NEG_INFINITY {
        return Some(std::cmp::Ordering::Greater);
    }

    let truncated = BigInt::from_f64(number.trunc())?;
    let ordering = bigint.to_bigint().cmp(&truncated);
    if !ordering.is_eq() {
        return Some(ordering);
    }
    if number.fract().is_sign_positive() && number.fract() != 0.0 {
        Some(std::cmp::Ordering::Less)
    } else if number.fract().is_sign_negative() && number.fract() != 0.0 {
        Some(std::cmp::Ordering::Greater)
    } else {
        Some(std::cmp::Ordering::Equal)
    }
}

fn string_to_bigint(value: &crate::value::JsString) -> Option<JsBigInt> {
    let text = String::from_utf16(&value.utf16_units().collect::<Vec<_>>()).ok()?;
    JsBigInt::parse_js_string(&text).ok()
}

fn mixed_numeric_type_error() -> Error {
    Error::new(ErrorKind::Type, "cannot convert bigint to number")
}

fn bigint_error(error: BigIntError) -> Error {
    let message = match error {
        BigIntError::ShiftTooLarge => "BigInt is too large to allocate".to_owned(),
        error => error.to_string(),
    };
    Error::new(ErrorKind::Range, message)
}

#[cfg(test)]
mod tests {
    use crate::bytecode::{
        ArgumentsKind, BytecodeFunction, DynamicEnvironmentSource, EvalVariableSource, Instruction,
        WithObjectSource,
    };
    use crate::error::ErrorKind;
    use crate::value::{JsString, Value};

    use super::{
        CallFrame, Completion, DetachedDynamicEnvironmentOperation, DetachedEvalVariableOperation,
        DetachedHost, DirectEvalInvocation, Vm, VmHost, number_to_int32, number_to_uint32,
    };

    #[test]
    fn executes_arithmetic_stack_bytecode() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(6),
                Instruction::PushI32(7),
                Instruction::Mul,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };

        assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(42));
    }

    #[test]
    fn eval_opcode_gates_original_identity_and_preserves_fallback_arguments() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(7),
                Instruction::PushI32(11),
                Instruction::PushI32(12),
                Instruction::Eval {
                    argument_count: 2,
                    environment: 17,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        function.verify().unwrap();

        let mut original = DetachedHost::new(&function);
        original.eval_identity_results.push_back(Ok(true));
        original
            .direct_eval_results
            .push_back(Ok(Completion::Return(Value::Int(42))));
        assert_eq!(
            CallFrame::new(3)
                .execute(&function.code, &mut original)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(
            original.direct_eval_inputs,
            [DirectEvalInvocation {
                input: Value::Int(11),
                environment: 17,
                this_value: Value::Undefined,
                new_target: Value::Undefined,
                caller_strict: true,
            }]
        );
        assert_eq!(original.eval_identity_inputs, [Value::Int(7)]);
        assert!(original.call_inputs.is_empty());

        let mut replacement = DetachedHost::new(&function);
        replacement.eval_identity_results.push_back(Ok(false));
        replacement
            .call_results
            .push_back(Ok(Completion::Return(Value::Int(43))));
        assert_eq!(
            CallFrame::new(3)
                .execute(&function.code, &mut replacement)
                .unwrap(),
            Completion::Return(Value::Int(43))
        );
        assert!(replacement.direct_eval_inputs.is_empty());
        assert_eq!(replacement.eval_identity_inputs, [Value::Int(7)]);
        assert_eq!(
            replacement.call_inputs,
            [(
                Value::Int(7),
                Value::Undefined,
                vec![Value::Int(11), Value::Int(12)]
            )]
        );

        let no_arguments = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Undefined,
                Instruction::Eval {
                    argument_count: 0,
                    environment: 29,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        let mut original = DetachedHost::new(&no_arguments);
        original.eval_identity_results.push_back(Ok(true));
        original
            .direct_eval_results
            .push_back(Ok(Completion::Return(Value::Undefined)));
        assert_eq!(
            CallFrame::new(1)
                .execute(&no_arguments.code, &mut original)
                .unwrap(),
            Completion::Return(Value::Undefined)
        );
        assert_eq!(
            original.direct_eval_inputs,
            [DirectEvalInvocation {
                input: Value::Undefined,
                environment: 29,
                this_value: Value::Undefined,
                new_target: Value::Undefined,
                caller_strict: true,
            }]
        );
        assert_eq!(original.eval_identity_inputs, [Value::Undefined]);
    }

    #[test]
    fn string_direct_eval_forwards_environment_and_lazily_normalizes_this() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(7),
                Instruction::PushConst(0),
                Instruction::Eval {
                    argument_count: 1,
                    environment: 23,
                },
                Instruction::Return,
            ],
            constants: vec![Value::String(JsString::from_static("40 + 2"))],
            local_count: 0,
            max_stack: 2,
        };
        function.verify().unwrap();

        let mut host = DetachedHost::new(&function);
        host.eval_identity_results.push_back(Ok(true));
        host.box_primitive_results.push_back(Ok(Value::Int(99)));
        host.direct_eval_results
            .push_back(Ok(Completion::Return(Value::Int(42))));
        let mut frame = CallFrame::new(2);
        frame.this_value = Value::Int(8);
        frame.new_target = Value::Int(9);
        frame.strict = false;
        assert_eq!(
            frame.execute(&function.code, &mut host).unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(host.box_primitive_inputs, [Value::Int(8)]);
        assert_eq!(
            host.direct_eval_inputs,
            [DirectEvalInvocation {
                input: Value::String(JsString::from_static("40 + 2")),
                environment: 23,
                this_value: Value::Int(99),
                new_target: Value::Int(9),
                caller_strict: false,
            }]
        );

        let non_string = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(7),
                Instruction::PushI32(42),
                Instruction::Eval {
                    argument_count: 1,
                    environment: u16::MAX,
                },
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        non_string.verify().unwrap();
        let mut host = DetachedHost::new(&non_string);
        host.eval_identity_results.push_back(Ok(true));
        host.direct_eval_results
            .push_back(Ok(Completion::Return(Value::Int(42))));
        let mut frame = CallFrame::new(2);
        frame.this_value = Value::Int(8);
        frame.strict = false;
        assert_eq!(
            frame.execute(&non_string.code, &mut host).unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert!(host.box_primitive_inputs.is_empty());
        assert_eq!(host.direct_eval_inputs[0].this_value, Value::Int(8));
        assert_eq!(host.direct_eval_inputs[0].environment, u16::MAX);
    }

    #[test]
    fn arguments_opcode_forwards_kind_and_host_completion() {
        for kind in [ArgumentsKind::Mapped, ArgumentsKind::Unmapped] {
            let function = BytecodeFunction {
                name: None,
                code: vec![Instruction::Arguments(kind), Instruction::Return],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            function.verify().unwrap();
            let mut host = DetachedHost::new(&function);
            host.arguments_results
                .push_back((kind, Completion::Return(Value::Int(42))));
            assert_eq!(
                CallFrame::new(1)
                    .execute(&function.code, &mut host)
                    .unwrap(),
                Completion::Return(Value::Int(42))
            );
        }

        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Arguments(ArgumentsKind::Unmapped),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        let thrown = Value::String(JsString::from_static("arguments throw"));
        let mut host = DetachedHost::new(&function);
        host.arguments_results
            .push_back((ArgumentsKind::Unmapped, Completion::Throw(thrown.clone())));
        assert_eq!(
            CallFrame::new(1)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Throw(thrown)
        );
    }

    #[test]
    fn eval_variable_object_opcodes_preserve_stack_and_host_operands() {
        let source = EvalVariableSource::Local(0);
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::VariableEnvironment,
                Instruction::PutLocal(0),
                Instruction::HasEvalVariable { source, name: 0 },
                Instruction::Drop,
                Instruction::GetEvalVariable { source, name: 0 },
                Instruction::Drop,
                Instruction::PushI32(7),
                Instruction::PutEvalVariable { source, name: 0 },
                Instruction::DeleteEvalVariable { source, name: 0 },
                Instruction::Drop,
                Instruction::PushI32(11),
                Instruction::DefineEvalVariable { source, name: 0 },
                Instruction::PushI32(42),
                Instruction::Return,
            ],
            constants: vec![Value::String(JsString::from_static("added"))],
            local_count: 1,
            max_stack: 1,
        };
        function.verify().unwrap();

        let environment = Value::String(JsString::from_static("variable environment"));
        let mut host = DetachedHost::new(&function);
        host.variable_environment_results
            .push_back(Completion::Return(environment.clone()));
        for value in [
            Value::Bool(true),
            Value::Int(3),
            Value::Undefined,
            Value::Bool(true),
            Value::Undefined,
        ] {
            host.eval_variable_results
                .push_back(Completion::Return(value));
        }
        assert_eq!(
            CallFrame::new(1)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(host.get_local(0).unwrap(), environment);
        assert_eq!(
            host.eval_variable_operations,
            [
                DetachedEvalVariableOperation::Has(source, 0),
                DetachedEvalVariableOperation::Get(source, 0),
                DetachedEvalVariableOperation::Put(source, 0, Value::Int(7)),
                DetachedEvalVariableOperation::Delete(source, 0),
                DetachedEvalVariableOperation::Define(source, 0, Value::Int(11)),
            ]
        );
    }

    #[test]
    fn to_object_boxes_primitives_and_rejects_nullish_values() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(7),
                Instruction::ToObject,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        function.verify().unwrap();
        let mut host = DetachedHost::new(&function);
        host.box_primitive_results.push_back(Ok(Value::Int(42)));
        assert_eq!(
            CallFrame::new(1)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(host.box_primitive_inputs, [Value::Int(7)]);

        for nullish in [Instruction::Null, Instruction::Undefined] {
            let function = BytecodeFunction {
                name: None,
                code: vec![nullish, Instruction::ToObject, Instruction::Return],
                constants: vec![],
                local_count: 0,
                max_stack: 1,
            };
            function.verify().unwrap();
            let mut host = DetachedHost::new(&function);
            let error = CallFrame::new(1)
                .execute(&function.code, &mut host)
                .unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Type);
            assert_eq!(error.message(), "cannot convert to object");
            assert!(host.box_primitive_inputs.is_empty());
        }
    }

    #[test]
    fn dynamic_environment_opcodes_forward_sources_strictness_and_stack_values() {
        let source = DynamicEnvironmentSource::With(WithObjectSource::Local(0));
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::HasDynamicBinding { source, name: 0 },
                Instruction::Drop,
                Instruction::GetDynamicBinding { source, name: 0 },
                Instruction::Drop,
                Instruction::PushI32(7),
                Instruction::PutDynamicBinding { source, name: 0 },
                Instruction::DeleteDynamicBinding { source, name: 0 },
                Instruction::Drop,
                Instruction::DynamicEnvironmentObject(source),
                Instruction::GetRefValue(0),
                Instruction::PutRefValue(0),
                Instruction::DynamicEnvironmentObject(source),
                Instruction::GetRefValueUndef(0),
                Instruction::PutRefValue(0),
                Instruction::PushI32(42),
                Instruction::Return,
            ],
            constants: vec![Value::String(JsString::from_static("binding"))],
            local_count: 1,
            max_stack: 2,
        };
        function.verify().unwrap();

        let first_environment = Value::String(JsString::from_static("first environment"));
        let second_environment = Value::String(JsString::from_static("second environment"));
        let mut host = DetachedHost::new(&function);
        for completion in [
            Completion::Return(Value::Bool(true)),
            Completion::Return(Value::Int(3)),
            Completion::Return(Value::Undefined),
            Completion::Return(Value::Bool(true)),
            Completion::Return(first_environment.clone()),
            Completion::Return(Value::Int(11)),
            Completion::Return(Value::Undefined),
            Completion::Return(second_environment.clone()),
            Completion::Return(Value::Undefined),
            Completion::Return(Value::Undefined),
        ] {
            host.dynamic_environment_results.push_back(completion);
        }
        assert_eq!(
            CallFrame::new(2)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(42))
        );
        assert_eq!(
            host.dynamic_environment_operations,
            [
                DetachedDynamicEnvironmentOperation::Has(source, 0),
                DetachedDynamicEnvironmentOperation::Get(source, 0, true),
                DetachedDynamicEnvironmentOperation::Put(source, 0, Value::Int(7), true),
                DetachedDynamicEnvironmentOperation::Delete(source, 0),
                DetachedDynamicEnvironmentOperation::Object(source),
                DetachedDynamicEnvironmentOperation::GetRef(first_environment.clone(), 0, true,),
                DetachedDynamicEnvironmentOperation::PutRef(
                    first_environment,
                    0,
                    Value::Int(11),
                    true,
                ),
                DetachedDynamicEnvironmentOperation::Object(source),
                DetachedDynamicEnvironmentOperation::GetRef(second_environment.clone(), 0, false,),
                DetachedDynamicEnvironmentOperation::PutRef(
                    second_environment,
                    0,
                    Value::Undefined,
                    true,
                ),
            ]
        );
    }

    #[test]
    fn detached_vm_catches_values_and_manages_private_handlers() {
        let thrown = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(3),
                Instruction::PushI32(7),
                Instruction::Throw,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(Vm::new().execute(&thrown).unwrap(), Value::Int(7));

        let normal = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(4),
                Instruction::DropCatch,
                Instruction::PushI32(3),
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(Vm::new().execute(&normal).unwrap(), Value::Int(3));

        let nip = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(10),
                Instruction::Catch(6),
                Instruction::PushI32(20),
                Instruction::PushI32(30),
                Instruction::NipCatch,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(Vm::new().execute(&nip).unwrap(), Value::Int(30));

        let nested = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(7),
                Instruction::Catch(5),
                Instruction::PushI32(11),
                Instruction::Throw,
                Instruction::Nop,
                Instruction::Throw,
                Instruction::Nop,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(Vm::new().execute(&nested).unwrap(), Value::Int(11));
    }

    #[test]
    fn iterator_unwind_preserves_exception_and_completion_precedence() {
        let pending_throw = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(6),
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(41),
                Instruction::Throw,
                Instruction::Nop,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        pending_throw.verify().unwrap();
        let mut host = DetachedHost::new(&pending_throw);
        host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        // A close throw must not replace the already-pending value 41.
        host.iterator_close_results.push_back(Some(Value::Int(99)));
        assert_eq!(
            CallFrame::new(5)
                .execute(&pending_throw.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(41))
        );
        assert_eq!(host.iterator_close_pending, vec![true]);

        let normal_close_throw = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(7),
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::IteratorClose,
                Instruction::DropCatch,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        normal_close_throw.verify().unwrap();
        let mut host = DetachedHost::new(&normal_close_throw);
        host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        host.iterator_close_results.push_back(Some(Value::Int(77)));
        assert_eq!(
            CallFrame::new(4)
                .execute(&normal_close_throw.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(77))
        );
        assert_eq!(host.iterator_close_pending, vec![false]);

        let preserve_close_throw = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::PushI32(42),
                Instruction::IteratorClosePreserve,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        preserve_close_throw.verify().unwrap();
        let mut host = DetachedHost::new(&preserve_close_throw);
        host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        host.iterator_close_results.push_back(Some(Value::Int(88)));
        assert_eq!(
            CallFrame::new(4)
                .execute(&preserve_close_throw.code, &mut host)
                .unwrap(),
            Completion::Throw(Value::Int(88))
        );
        assert_eq!(host.iterator_close_pending, vec![false]);
    }

    #[test]
    fn for_of_next_disables_done_and_throwing_iterators() {
        let done = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::ForOfNext(0),
                Instruction::Drop,
                Instruction::Drop,
                Instruction::IteratorClose,
                Instruction::PushI32(3),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        done.verify().unwrap();
        let mut host = DetachedHost::new(&done);
        host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        host.iterator_next_results
            .push_back(Ok((Value::Undefined, true)));
        assert_eq!(
            CallFrame::new(5).execute(&done.code, &mut host).unwrap(),
            Completion::Return(Value::Int(3))
        );
        assert!(host.iterator_close_pending.is_empty());

        let next_throw = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(10),
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::ForOfNext(0),
                Instruction::Drop,
                Instruction::Drop,
                Instruction::IteratorClose,
                Instruction::DropCatch,
                Instruction::Undefined,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 6,
        };
        next_throw.verify().unwrap();
        let mut host = DetachedHost::new(&next_throw);
        host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        host.iterator_next_results.push_back(Err(Value::Int(55)));
        assert_eq!(
            CallFrame::new(6)
                .execute(&next_throw.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(55))
        );
        assert!(host.iterator_close_pending.is_empty());
    }

    #[test]
    fn array_literal_opcodes_preserve_operands_and_element_order() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::ArrayFrom(2),
                Instruction::PushI32(3),
                Instruction::DefineField(0),
                Instruction::PushI32(4),
                Instruction::PushI32(5),
                Instruction::DefineArrayEl,
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![Value::String(JsString::from_static("2"))],
            local_count: 0,
            max_stack: 3,
        };
        function.verify().unwrap();
        let mut host = DetachedHost::new(&function);
        host.array_from_results
            .push_back(Completion::Return(Value::String(JsString::from_static(
                "array",
            ))));
        host.define_field_results
            .push_back(Completion::Return(Value::Undefined));
        host.define_array_element_results
            .push_back(Completion::Return(Value::Undefined));
        assert_eq!(
            CallFrame::new(3)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::String(JsString::from_static("array")))
        );
        assert_eq!(host.array_from_inputs, [vec![Value::Int(1), Value::Int(2)]]);
        assert_eq!(
            host.defined_fields,
            [(
                Value::String(JsString::from_static("array")),
                0,
                Value::Int(3)
            )]
        );
        assert_eq!(
            host.defined_array_elements,
            [(
                Value::String(JsString::from_static("array")),
                Value::Int(4),
                Value::Int(5)
            )]
        );

        let dup1 = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::Dup1,
                Instruction::Add,
                Instruction::Add,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };
        assert_eq!(Vm::new().execute(&dup1).unwrap(), Value::Int(4));
    }

    #[test]
    fn object_literal_opcodes_preserve_target_and_operand_order() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Object,
                Instruction::Null,
                Instruction::SetProto,
                Instruction::PushI32(7),
                Instruction::CopyDataProperties,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        function.verify().unwrap();
        let object = Value::String(JsString::from_static("object"));
        let mut host = DetachedHost::new(&function);
        host.object_results
            .push_back(Completion::Return(object.clone()));
        host.set_object_prototype_results
            .push_back(Completion::Return(Value::Undefined));
        host.copy_data_properties_results
            .push_back(Completion::Return(Value::Undefined));

        assert_eq!(
            CallFrame::new(2)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(object.clone())
        );
        assert_eq!(
            host.set_object_prototype_inputs,
            [(object.clone(), Value::Null)]
        );
        assert_eq!(host.copy_data_properties_inputs, [(object, Value::Int(7))]);
    }

    #[test]
    fn object_literal_opcodes_forward_host_throws() {
        let thrown = Value::String(JsString::from_static("literal throw"));

        let object = BytecodeFunction {
            name: None,
            code: vec![Instruction::Object, Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        object.verify().unwrap();
        let mut host = DetachedHost::new(&object);
        host.object_results
            .push_back(Completion::Throw(thrown.clone()));
        assert_eq!(
            CallFrame::new(1).execute(&object.code, &mut host).unwrap(),
            Completion::Throw(thrown.clone())
        );

        let proto = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::Null,
                Instruction::SetProto,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        proto.verify().unwrap();
        let mut host = DetachedHost::new(&proto);
        host.set_object_prototype_results
            .push_back(Completion::Throw(thrown.clone()));
        assert_eq!(
            CallFrame::new(2).execute(&proto.code, &mut host).unwrap(),
            Completion::Throw(thrown.clone())
        );
        assert_eq!(
            host.set_object_prototype_inputs,
            [(Value::Int(1), Value::Null)]
        );

        let spread = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(1),
                Instruction::PushI32(2),
                Instruction::CopyDataProperties,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        spread.verify().unwrap();
        let mut host = DetachedHost::new(&spread);
        host.copy_data_properties_results
            .push_back(Completion::Throw(thrown.clone()));
        assert_eq!(
            CallFrame::new(2).execute(&spread.code, &mut host).unwrap(),
            Completion::Throw(thrown)
        );
        assert_eq!(
            host.copy_data_properties_inputs,
            [(Value::Int(1), Value::Int(2))]
        );
    }

    #[test]
    fn append_uses_iterator_protocol_and_preserves_pending_throw_on_close() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushConst(0),
                Instruction::PushI32(0),
                Instruction::PushConst(1),
                Instruction::Append,
                Instruction::Drop,
                Instruction::Return,
            ],
            constants: vec![
                Value::String(JsString::from_static("array")),
                Value::String(JsString::from_static("iterable")),
            ],
            local_count: 0,
            max_stack: 3,
        };
        function.verify().unwrap();

        let mut success = DetachedHost::new(&function);
        success.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        success
            .iterator_next_results
            .push_back(Ok((Value::Int(7), false)));
        success
            .iterator_next_results
            .push_back(Ok((Value::Int(8), false)));
        success
            .iterator_next_results
            .push_back(Ok((Value::Undefined, true)));
        success
            .define_array_element_results
            .push_back(Completion::Return(Value::Undefined));
        success
            .define_array_element_results
            .push_back(Completion::Return(Value::Undefined));
        assert_eq!(
            CallFrame::new(3)
                .execute(&function.code, &mut success)
                .unwrap(),
            Completion::Return(Value::String(JsString::from_static("array")))
        );
        assert_eq!(
            success.defined_array_elements,
            [
                (
                    Value::String(JsString::from_static("array")),
                    Value::Int(0),
                    Value::Int(7)
                ),
                (
                    Value::String(JsString::from_static("array")),
                    Value::Int(1),
                    Value::Int(8)
                )
            ]
        );
        assert!(success.iterator_close_pending.is_empty());

        let mut next_throw = DetachedHost::new(&function);
        next_throw.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        next_throw
            .iterator_next_results
            .push_back(Err(Value::Int(55)));
        next_throw
            .iterator_close_results
            .push_back(Some(Value::Int(99)));
        assert_eq!(
            CallFrame::new(3)
                .execute(&function.code, &mut next_throw)
                .unwrap(),
            Completion::Throw(Value::Int(55))
        );
        assert_eq!(next_throw.iterator_close_pending, [true]);

        let mut define_throw = DetachedHost::new(&function);
        define_throw.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        define_throw
            .iterator_next_results
            .push_back(Ok((Value::Int(7), false)));
        define_throw
            .define_array_element_results
            .push_back(Completion::Throw(Value::Int(66)));
        define_throw.iterator_close_results.push_back(None);
        assert_eq!(
            CallFrame::new(3)
                .execute(&function.code, &mut define_throw)
                .unwrap(),
            Completion::Throw(Value::Int(66))
        );
        assert_eq!(define_throw.iterator_close_pending, [true]);
    }

    #[test]
    fn detached_vm_rejects_array_allocation_without_runtime_intrinsics() {
        let function = BytecodeFunction {
            name: None,
            code: vec![Instruction::ArrayFrom(0), Instruction::Return],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        assert_eq!(
            Vm::new().execute(&function).unwrap_err().message(),
            "detached VM cannot create runtime-owned Array objects"
        );
    }

    #[test]
    fn iterator_region_above_gosub_address_closes_without_consuming_it() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(9),
                Instruction::Gosub(5),
                Instruction::Return,
                Instruction::Nop,
                Instruction::Nop,
                Instruction::PushI32(1),
                Instruction::ForOfStart,
                Instruction::IteratorClose,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 5,
        };
        function.verify().unwrap();
        let mut host = DetachedHost::new(&function);
        host.iterator_start_record = Some((Value::Int(10), Value::Int(11)));
        host.iterator_close_results.push_back(None);
        assert_eq!(
            CallFrame::new(5)
                .execute(&function.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(9))
        );
        assert_eq!(host.iterator_close_pending, vec![false]);
    }

    #[test]
    fn detached_vm_executes_typed_gosub_return_and_cleanup() {
        let returning = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(9),
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::Nop,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(Vm::new().execute(&returning).unwrap(), Value::Int(9));

        let abrupt = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(9),
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::Nop,
                Instruction::DropGosub,
                Instruction::Drop,
                Instruction::PushI32(4),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        assert_eq!(Vm::new().execute(&abrupt).unwrap(), Value::Int(4));

        let caught_inside_gosub = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(9),
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::Nop,
                Instruction::Catch(8),
                Instruction::PushI32(7),
                Instruction::Throw,
                Instruction::Nop,
                Instruction::Drop,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 4,
        };
        assert_eq!(
            Vm::new().execute(&caught_inside_gosub).unwrap(),
            Value::Int(9)
        );
    }

    #[test]
    fn captured_local_reuse_hook_is_limited_to_abrupt_resume_boundaries() {
        let return_unwind = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(4),
                Instruction::PushI32(9),
                Instruction::NipCatch,
                Instruction::Return,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        let mut host = DetachedHost::new(&return_unwind);
        assert_eq!(
            CallFrame::new(2)
                .execute(&return_unwind.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(9))
        );
        assert_eq!(host.captured_local_reuse_preparations, 1);

        let caught_throw = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(4),
                Instruction::PushI32(7),
                Instruction::Throw,
                Instruction::Nop,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        let mut host = DetachedHost::new(&caught_throw);
        assert_eq!(
            CallFrame::new(2)
                .execute(&caught_throw.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(7))
        );
        assert_eq!(host.captured_local_reuse_preparations, 1);

        let ordinary_gosub = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(5),
                Instruction::Gosub(4),
                Instruction::Return,
                Instruction::Nop,
                Instruction::Ret,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        let mut host = DetachedHost::new(&ordinary_gosub);
        assert_eq!(
            CallFrame::new(2)
                .execute(&ordinary_gosub.code, &mut host)
                .unwrap(),
            Completion::Return(Value::Int(5))
        );
        assert_eq!(host.captured_local_reuse_preparations, 0);

        let malformed_nip = BytecodeFunction {
            name: None,
            code: vec![Instruction::PushI32(1), Instruction::NipCatch],
            constants: vec![],
            local_count: 0,
            max_stack: 1,
        };
        let mut host = DetachedHost::new(&malformed_nip);
        let error = CallFrame::new(1)
            .execute(&malformed_nip.code, &mut host)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(host.captured_local_reuse_preparations, 0);
    }

    #[test]
    fn runtime_ret_validation_is_an_uncatchable_engine_invariant() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::Catch(4),
                Instruction::Undefined,
                Instruction::Ret,
                Instruction::Nop,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };
        let mut host = DetachedHost::new(&function);
        let error = CallFrame::new(2)
            .execute(&function.code, &mut host)
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Internal);
        assert_eq!(error.message(), "invalid ret value");
    }

    #[test]
    fn detached_vm_uses_the_declared_undefined_local_frame() {
        let initial = BytecodeFunction {
            name: None,
            code: vec![Instruction::GetLocal(0), Instruction::Return],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        assert_eq!(Vm::new().execute(&initial).unwrap(), Value::Undefined);

        let written = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(42),
                Instruction::PutLocal(0),
                Instruction::GetLocal(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        assert_eq!(Vm::new().execute(&written).unwrap(), Value::Int(42));
    }

    #[test]
    fn detached_vm_enforces_lexical_local_tdz_and_initialization() {
        let tdz = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::GetLocalCheck(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        let error = Vm::new().execute(&tdz).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Reference);
        assert_eq!(error.message(), "lexical variable is not initialized");

        let initialized = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::PushI32(40),
                Instruction::InitializeLocal(0),
                Instruction::GetLocalCheck(0),
                Instruction::PushI32(2),
                Instruction::Add,
                Instruction::SetLocalCheck(0),
                Instruction::CloseLocal(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 2,
        };
        assert_eq!(Vm::new().execute(&initialized).unwrap(), Value::Int(42));

        let consuming_write = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::Undefined,
                Instruction::InitializeLocal(0),
                Instruction::PushI32(42),
                Instruction::PutLocalCheck(0),
                Instruction::GetLocalCheck(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        assert_eq!(Vm::new().execute(&consuming_write).unwrap(), Value::Int(42));
    }

    #[test]
    fn detached_vm_rejects_checked_writes_in_the_tdz_and_allows_plain_reinitialization() {
        for (write, preserves_value) in [
            (Instruction::PutLocalCheck(0), false),
            (Instruction::SetLocalCheck(0), true),
        ] {
            let mut code = vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::PushI32(1),
                write,
            ];
            if !preserves_value {
                code.push(Instruction::Undefined);
            }
            code.push(Instruction::Return);
            let function = BytecodeFunction {
                name: None,
                code,
                constants: vec![],
                local_count: 1,
                max_stack: 1,
            };
            let error = Vm::new().execute(&function).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Reference);
            assert_eq!(error.message(), "lexical variable is not initialized");
        }

        let twice = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::SetLocalUninitialized(0),
                Instruction::PushI32(1),
                Instruction::InitializeLocal(0),
                Instruction::PushI32(2),
                Instruction::InitializeLocal(0),
                Instruction::GetLocalCheck(0),
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 1,
            max_stack: 1,
        };
        assert_eq!(Vm::new().execute(&twice).unwrap(), Value::Int(2));
    }

    #[test]
    fn detached_membership_rejects_primitive_right_operands_before_host_dispatch() {
        for (operator, message) in [
            (Instruction::In, "invalid 'in' operand"),
            (
                Instruction::InstanceOf,
                "invalid 'instanceof' right operand",
            ),
        ] {
            let function = BytecodeFunction {
                name: None,
                code: vec![
                    Instruction::PushI32(1),
                    Instruction::PushI32(2),
                    operator,
                    Instruction::Return,
                ],
                constants: vec![],
                local_count: 0,
                max_stack: 2,
            };
            let error = Vm::new().execute(&function).unwrap_err();
            assert_eq!(error.kind(), ErrorKind::Type);
            assert_eq!(error.message(), message);
        }
    }

    #[test]
    fn executes_power_stack_bytecode_and_quickjs_number_edges() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(2),
                Instruction::PushI32(3),
                Instruction::PushI32(2),
                Instruction::Pow,
                Instruction::Pow,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 3,
        };

        assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(512));
        assert!(crate::number::pow(1.0, f64::INFINITY).is_nan());
        assert!(crate::number::pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert!(crate::number::pow(-2.0, 0.5).is_nan());
        assert_eq!(crate::number::pow(f64::NAN, 0.0), 1.0);
        assert_eq!(crate::number::pow(2.0, -2.0), 0.25);
        assert_eq!(crate::number::pow(-0.0, 3.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(crate::number::pow(-0.0, -3.0), f64::NEG_INFINITY);
    }

    #[test]
    fn executes_string_addition() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushConst(0),
                Instruction::PushConst(1),
                Instruction::Add,
                Instruction::Return,
            ],
            constants: vec![
                Value::String(JsString::from_static("quick")),
                Value::String(JsString::from_static("js")),
            ],
            local_count: 0,
            max_stack: 2,
        };

        assert_eq!(
            Vm::new().execute(&function).unwrap(),
            Value::String(JsString::from_static("quickjs"))
        );
    }

    #[test]
    fn string_addition_builds_ropes_and_reports_the_quickjs_length_error() {
        let chunk = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushConst(0),
                Instruction::PushConst(1),
                Instruction::Add,
                Instruction::Return,
            ],
            constants: vec![Value::String(chunk.clone()), Value::String(chunk.clone())],
            local_count: 0,
            max_stack: 2,
        };
        let Value::String(rope) = Vm::new().execute(&function).unwrap() else {
            panic!("large String addition did not return a String");
        };
        assert_eq!(rope.len(), 16_386);
        assert!(!rope.is_flat());

        let mut near_limit = chunk;
        for _ in 0..16 {
            near_limit = near_limit.try_concat(&near_limit).unwrap();
        }
        let overflow = BytecodeFunction {
            name: None,
            code: function.code,
            constants: vec![Value::String(near_limit.clone()), Value::String(near_limit)],
            local_count: 0,
            max_stack: 2,
        };
        let error = Vm::new().execute(&overflow).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::JsInternal);
        assert_eq!(error.message(), "string too long");
    }

    #[test]
    fn executes_bitwise_stack_bytecode() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(0b1010),
                Instruction::PushI32(0b1100),
                Instruction::BitAnd,
                Instruction::BitNot,
                Instruction::PushI32(0b0011),
                Instruction::BitXor,
                Instruction::PushI32(0b0100),
                Instruction::BitOr,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };

        assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(-12));
    }

    #[test]
    fn to_int32_uses_ecmascript_modulo_semantics() {
        for (value, expected) in [
            (0.0, 0),
            (-0.0, 0),
            (f64::NAN, 0),
            (f64::INFINITY, 0),
            (f64::NEG_INFINITY, 0),
            (1.9, 1),
            (-1.9, -1),
            (2_147_483_647.0, i32::MAX),
            (2_147_483_648.0, i32::MIN),
            (4_294_967_295.0, -1),
            (4_294_967_296.0, 0),
            (4_294_967_297.0, 1),
            (-2_147_483_649.0, i32::MAX),
            (1.0e300, 0),
        ] {
            assert_eq!(number_to_int32(value), expected, "input {value:?}");
        }

        for (value, expected) in [
            (-1.0, u32::MAX),
            (2_147_483_648.0, 2_147_483_648),
            (4_294_967_295.0, u32::MAX),
            (4_294_967_296.0, 0),
            (-4_294_967_295.0, 1),
        ] {
            assert_eq!(number_to_uint32(value), expected, "input {value:?}");
        }
    }

    #[test]
    fn executes_shift_stack_bytecode() {
        let function = BytecodeFunction {
            name: None,
            code: vec![
                Instruction::PushI32(-8),
                Instruction::PushI32(1),
                Instruction::Sar,
                Instruction::PushI32(1),
                Instruction::Shr,
                Instruction::PushI32(2),
                Instruction::Shl,
                Instruction::Return,
            ],
            constants: vec![],
            local_count: 0,
            max_stack: 2,
        };

        assert_eq!(Vm::new().execute(&function).unwrap(), Value::Int(-8));
    }
}
