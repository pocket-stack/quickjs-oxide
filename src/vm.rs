use crate::bigint::{BigIntError, JsBigInt};
use crate::bytecode::{BytecodeFunction, Instruction};
use crate::error::{Error, ErrorKind, NativeErrorKind};
use crate::heap::{ContextId, FunctionMetadata};
use crate::object::ObjectRef;
use crate::value::{JsString, Value};
use num_bigint::BigInt;
use num_traits::FromPrimitive;

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
    fn load_constant(&mut self, index: u32) -> Result<Value, Error>;
    /// Build the atom-named diagnostic for `ThrowReadOnly`. Runtime execution
    /// resolves the constant through its atom table; detached execution has no
    /// table and formats the verified String constant directly.
    fn read_only_error(&mut self, index: u32) -> Result<Error, Error>;
    fn type_of(&mut self, value: &Value) -> Result<&'static str, Error>;
    fn box_primitive(&mut self, value: Value) -> Result<Value, Error>;
    fn to_primitive(&mut self, value: Value, hint: ToPrimitiveHint) -> Result<Completion, Error>;
    fn materialize_error(&mut self, error: Error) -> Result<Value, Error>;
    fn instantiate_closure(&mut self, index: u32) -> Result<Value, Error>;
    fn set_function_name(&mut self, value: &Value, name_index: u32) -> Result<(), Error>;
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

struct DetachedHost<'a> {
    function: &'a BytecodeFunction,
    locals: Vec<DetachedLocal>,
}

impl<'a> DetachedHost<'a> {
    fn new(function: &'a BytecodeFunction) -> Self {
        Self {
            function,
            locals: (0..function.local_count)
                .map(|_| DetachedLocal::Initialized(Value::Undefined))
                .collect(),
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

    fn type_of(&mut self, value: &Value) -> Result<&'static str, Error> {
        Ok(value.type_of())
    }

    fn box_primitive(&mut self, _value: Value) -> Result<Value, Error> {
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
        Err(Error::internal(
            "detached VM cannot call runtime-owned function objects",
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
struct CallFrame {
    stack: Vec<Value>,
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
        match self.execute_inner(code, host) {
            Ok(Completion::Throw(value)) => self.raise(value, host),
            Ok(completion) => Ok(completion),
            Err(error) if NativeErrorKind::from_javascript_error(error.kind()).is_some() => {
                let value = host.materialize_error(error)?;
                self.raise(value, host)
            }
            Err(error) => Err(error),
        }
    }

    fn execute_inner(
        &mut self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<Completion, Error> {
        let mut pc = 0_usize;

        loop {
            let instruction = code
                .get(pc)
                .ok_or_else(|| Error::internal("bytecode ended without return"))?;
            host.update_active_bytecode_pc(BytecodePc::new(pc))?;
            pc += 1;

            match instruction {
                Instruction::Nop => {}
                Instruction::PushI32(value) => self.stack.push(Value::Int(*value)),
                Instruction::PushConst(index) => {
                    self.stack.push(host.load_constant(*index)?);
                }
                Instruction::FClosure(index) => {
                    self.stack.push(host.instantiate_closure(*index)?);
                }
                Instruction::SetName(index) => {
                    let value = self
                        .stack
                        .last()
                        .ok_or_else(|| Error::internal("set name on an empty stack"))?;
                    host.set_function_name(value, *index)?;
                }
                Instruction::ThrowReadOnly(index) => {
                    self.pop()?;
                    return Err(host.read_only_error(*index)?);
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
                Instruction::Neg => {
                    if let OperationOutcome::Throw(value) = self.neg(host)? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Plus => {
                    if let OperationOutcome::Throw(value) = self.unary_plus(host)? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Inc | Instruction::Dec => {
                    let increment = matches!(instruction, Instruction::Inc);
                    if let OperationOutcome::Throw(value) =
                        self.update_numeric(host, increment, false)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::PostInc | Instruction::PostDec => {
                    let increment = matches!(instruction, Instruction::PostInc);
                    if let OperationOutcome::Throw(value) =
                        self.update_numeric(host, increment, true)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::BitNot => {
                    if let OperationOutcome::Throw(value) = self.bit_not(host)? {
                        return Ok(Completion::Throw(value));
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
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Sub => {
                    if let OperationOutcome::Throw(value) =
                        self.binary_numeric(host, |left, right| left - right, JsBigInt::sub)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Mul => {
                    if let OperationOutcome::Throw(value) =
                        self.binary_numeric(host, |left, right| left * right, JsBigInt::mul)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Div => {
                    if let OperationOutcome::Throw(value) =
                        self.binary_numeric(host, |left, right| left / right, JsBigInt::div)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Mod => {
                    if let OperationOutcome::Throw(value) =
                        self.binary_numeric(host, |left, right| left % right, JsBigInt::rem)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Pow => {
                    if let OperationOutcome::Throw(value) =
                        self.binary_numeric(host, number_pow, JsBigInt::pow)?
                    {
                        return Ok(Completion::Throw(value));
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
                        return Ok(Completion::Throw(value));
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
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Shr => {
                    if let OperationOutcome::Throw(value) = self.unsigned_shift_right(host)? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::BitAnd => {
                    if let OperationOutcome::Throw(value) = self.binary_numeric(
                        host,
                        |left, right| f64::from(number_to_int32(left) & number_to_int32(right)),
                        JsBigInt::bit_and,
                    )? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::BitXor => {
                    if let OperationOutcome::Throw(value) = self.binary_numeric(
                        host,
                        |left, right| f64::from(number_to_int32(left) ^ number_to_int32(right)),
                        JsBigInt::bit_xor,
                    )? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::BitOr => {
                    if let OperationOutcome::Throw(value) = self.binary_numeric(
                        host,
                        |left, right| f64::from(number_to_int32(left) | number_to_int32(right)),
                        JsBigInt::bit_or,
                    )? {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Eq => {
                    let (left, right) = self.pop_pair()?;
                    match abstract_equal(host, left, right)? {
                        OperationOutcome::Value(equal) => self.stack.push(Value::Bool(equal)),
                        OperationOutcome::Throw(value) => return Ok(Completion::Throw(value)),
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
                        OperationOutcome::Throw(value) => return Ok(Completion::Throw(value)),
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
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Lte => {
                    if let OperationOutcome::Throw(value) =
                        self.compare(host, std::cmp::Ordering::is_le)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Gt => {
                    if let OperationOutcome::Throw(value) =
                        self.compare(host, std::cmp::Ordering::is_gt)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
                Instruction::Gte => {
                    if let OperationOutcome::Throw(value) =
                        self.compare(host, std::cmp::Ordering::is_ge)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
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
                        pc = checked_target(*target, code.len())?;
                    }
                }
                Instruction::IfTrue(target) => {
                    if self.pop()?.to_boolean() {
                        pc = checked_target(*target, code.len())?;
                    }
                }
                Instruction::Goto(target) => {
                    pc = checked_target(*target, code.len())?;
                }
                Instruction::Call(argument_count) => {
                    let arguments = self.take_call_arguments(*argument_count, 1)?;
                    let function = self.pop()?;
                    match host.call(function, Value::Undefined, arguments)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::CallMethod(argument_count) => {
                    let arguments = self.take_call_arguments(*argument_count, 2)?;
                    let function = self.pop()?;
                    let receiver = self.pop()?;
                    match host.call(function, receiver, arguments)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::Construct(argument_count) => {
                    let arguments = self.take_call_arguments(*argument_count, 2)?;
                    let new_target = self.pop()?;
                    let function = self.pop()?;
                    match host.construct(function, new_target, arguments)? {
                        Completion::Return(value) => self.stack.push(value),
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    }
                }
                Instruction::Return => return self.pop().map(Completion::Return),
                Instruction::Throw => return self.pop().map(Completion::Throw),
            }
        }
    }

    fn raise(&mut self, value: Value, host: &mut impl VmHost) -> Result<Completion, Error> {
        host.ensure_backtrace(&value)?;
        // Catch/finally handler lookup will live at this single boundary. Until
        // handler metadata is present, every raised value escapes the frame.
        Ok(Completion::Throw(value))
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

fn number_pow(base: f64, exponent: f64) -> f64 {
    if !exponent.is_finite() && base.abs() == 1.0 {
        f64::NAN
    } else {
        base.powf(exponent)
    }
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
    use crate::bytecode::{BytecodeFunction, Instruction};
    use crate::error::ErrorKind;
    use crate::value::{JsString, Value};

    use super::{Vm, number_pow, number_to_int32, number_to_uint32};

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
        assert!(number_pow(1.0, f64::INFINITY).is_nan());
        assert!(number_pow(-1.0, f64::NEG_INFINITY).is_nan());
        assert!(number_pow(-2.0, 0.5).is_nan());
        assert_eq!(number_pow(f64::NAN, 0.0), 1.0);
        assert_eq!(number_pow(2.0, -2.0), 0.25);
        assert_eq!(number_pow(-0.0, 3.0).to_bits(), (-0.0f64).to_bits());
        assert_eq!(number_pow(-0.0, -3.0), f64::NEG_INFINITY);
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
