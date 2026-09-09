use super::*;

/// Per-invocation value stack. This will later grow the remaining
/// `JSStackFrame` fields (arguments, locals, closure variables and realm), but
/// its ownership boundary is already the final one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VmUnwindRegion {
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
        /// Async for-of temporarily disables the region across the cached
        /// `next` call and its Await. Keeping the record family explicit
        /// prevents sync and async continuation opcodes from crossing.
        asynchronous: bool,
    },
}

/// Owned, Rust-stack-independent state of one bytecode invocation.
///
/// This type is deliberately runtime-agnostic. While it is actively executing
/// its rooted values are ordinary [`Value`]s; [`VmActivationParts`] is the
/// conversion boundary used by the runtime to encode/decode its own raw heap
/// representation while a generator is dormant.
#[derive(Debug, PartialEq)]
pub(crate) struct VmActivation {
    pub(in crate::engine::vm) stack: Vec<Value>,
    pub(in crate::engine::vm) regions: Vec<VmUnwindRegion>,
    pub(in crate::engine::vm) pc: usize,
    pub(in crate::engine::vm) caller_realm: Option<ContextId>,
    /// The realm captured by the executing bytecode.
    pub(in crate::engine::vm) callee_realm: Option<ContextId>,
    pub(in crate::engine::vm) current_function: Option<ObjectRef>,
    pub(in crate::engine::vm) this_value: Value,
    pub(in crate::engine::vm) normalized_this: Option<Value>,
    pub(in crate::engine::vm) new_target: Value,
    pub(in crate::engine::vm) strict: bool,
    pub(in crate::engine::vm) callee_global: Option<ObjectRef>,
}

/// Serializable-shaped activation fields exposed to the runtime bridge.
///
/// The fields still contain rooted VM handles, and therefore must only exist
/// transiently. Long-lived generator heap state maps every `Value` and
/// `ObjectRef` here to the heap's raw representation before returning control
/// to JavaScript.
#[derive(Debug, PartialEq)]
pub(crate) struct VmActivationParts {
    pub(crate) stack: Vec<Value>,
    pub(crate) regions: Vec<VmUnwindRegion>,
    pub(crate) pc: usize,
    pub(crate) caller_realm: Option<ContextId>,
    pub(crate) callee_realm: Option<ContextId>,
    pub(crate) current_function: Option<ObjectRef>,
    pub(crate) this_value: Value,
    pub(crate) normalized_this: Option<Value>,
    pub(crate) new_target: Value,
    pub(crate) strict: bool,
    pub(crate) callee_global: Option<ObjectRef>,
}

/// A generator suspension owns the complete interpreter activation. For a
/// visible yield, `take_yielded` must be called exactly once before snapshot or
/// resume; it replaces QuickJS's retained output slot with `undefined`.
#[derive(Debug, PartialEq)]
pub(crate) struct VmSuspension {
    pub(in crate::engine::vm) kind: VmSuspendKind,
    pub(in crate::engine::vm) activation: VmActivation,
    pub(in crate::engine::vm) output_taken: bool,
}

/// Internal interpreter exit. JavaScript completion remains intentionally
/// limited to [`Completion::Return`] and [`Completion::Throw`].
pub(in crate::engine::vm) enum InterpreterExit {
    Complete(Completion),
    Suspend(VmSuspendKind),
}

impl VmSuspension {
    pub(in crate::engine::vm) fn new(
        kind: VmSuspendKind,
        activation: VmActivation,
    ) -> Result<Self, Error> {
        if kind != VmSuspendKind::Initial && activation.stack.is_empty() {
            return Err(Error::internal(
                "VM suspension has no retained output stack slot",
            ));
        }
        Ok(Self {
            kind,
            activation,
            output_taken: kind == VmSuspendKind::Initial,
        })
    }

    #[must_use]
    pub(crate) const fn kind(&self) -> VmSuspendKind {
        self.kind
    }

    /// Extract the visible value of `yield`/`yield*` and leave the retained
    /// operand slot ready for QuickJS-compatible resume injection.
    pub(crate) fn take_yielded(&mut self) -> Result<Value, Error> {
        if !matches!(
            self.kind,
            VmSuspendKind::Yield | VmSuspendKind::YieldStar | VmSuspendKind::AsyncYieldStar
        ) {
            return Err(Error::internal(
                "non-generator suspension has no yielded value",
            ));
        }
        self.take_output("generator suspension output")
    }

    /// Extract the operand retained by `OP_await`. The dormant activation
    /// keeps an `undefined` placeholder in that exact slot until the Promise
    /// reaction explicitly fulfils or rejects the AwaitExpression.
    pub(crate) fn take_awaited(&mut self) -> Result<Value, Error> {
        if self.kind != VmSuspendKind::Await {
            return Err(Error::internal("non-await suspension has no awaited value"));
        }
        self.take_output("async suspension output")
    }

    pub(in crate::engine::vm) fn take_output(
        &mut self,
        operation: &'static str,
    ) -> Result<Value, Error> {
        if self.output_taken {
            return Err(Error::internal(format!(
                "{operation} was already extracted"
            )));
        }
        let slot = self
            .activation
            .stack
            .last_mut()
            .ok_or_else(|| Error::internal(format!("{operation} stack slot is missing")))?;
        let output = std::mem::replace(slot, Value::Undefined);
        self.output_taken = true;
        Ok(output)
    }

    /// Convert a suspension to transient rooted state consumed by the
    /// runtime's raw heap encoder. Every visible suspension operand must have
    /// been extracted first so dormant frames never retain it twice.
    pub(crate) fn into_parts(self) -> Result<(VmSuspendKind, VmActivationParts), Error> {
        if !self.output_taken {
            return Err(Error::internal(
                "VM suspension must be extracted before snapshot",
            ));
        }
        Ok((self.kind, self.activation.into_parts()))
    }

    /// Restore a suspension whose visible yielded slot was already replaced
    /// with `undefined` before it entered long-lived heap state.
    pub(crate) fn from_parts(kind: VmSuspendKind, parts: VmActivationParts) -> Result<Self, Error> {
        let activation = VmActivation::from_parts(parts);
        if kind != VmSuspendKind::Initial && activation.stack.is_empty() {
            return Err(Error::internal(
                "restored VM suspension has no resume operand slot",
            ));
        }
        Ok(Self {
            kind,
            activation,
            output_taken: true,
        })
    }

    /// Continue past the hidden 0-to-0 initial-yield barrier. The first
    /// user-visible `next(argument)` intentionally supplies no operand here;
    /// ECMAScript ignores that argument for a suspended-start generator.
    pub(crate) fn resume_initial(
        self,
        code: &[Instruction],
        host: &mut impl VmHost,
    ) -> Result<VmExit, Error> {
        if self.kind != VmSuspendKind::Initial {
            return Err(Error::internal(
                "non-initial generator suspension used initial resume",
            ));
        }
        self.activation.run(code, host)
    }

    /// Resume a visible generator suspension using QuickJS's private
    /// value-plus-magic ABI.
    pub(crate) fn resume(
        self,
        code: &[Instruction],
        host: &mut impl VmHost,
        resume: VmResume,
    ) -> Result<VmExit, Error> {
        if self.kind == VmSuspendKind::Initial {
            return Err(Error::internal(
                "initial generator suspension requires parameterless resume",
            ));
        }
        if !self.output_taken {
            return Err(Error::internal(
                "generator suspension must be extracted before resume",
            ));
        }
        if self.kind == VmSuspendKind::Await {
            return Err(Error::internal(
                "await suspension requires an async-function resume",
            ));
        }

        let mut activation = self.activation;
        if self.kind == VmSuspendKind::Yield {
            if let VmResume::Throw(value) = resume {
                if let Some(completion) = activation.raise(value, host, code.len())? {
                    return Ok(VmExit::Complete(completion));
                }
                return activation.run(code, host);
            }
        }

        let (value, magic) = match resume {
            VmResume::Next(value) => (value, 0),
            VmResume::Return(value) => (value, 1),
            VmResume::Throw(value) => (value, 2),
        };
        let slot = activation
            .stack
            .last_mut()
            .ok_or_else(|| Error::internal("generator resume stack slot is missing"))?;
        if !matches!(slot, Value::Undefined) {
            return Err(Error::internal(
                "generator resume stack slot was not cleared after yield",
            ));
        }
        *slot = value;
        activation.stack.push(Value::Int(magic));
        activation.run(code, host)
    }

    /// Resume a fulfilled AwaitExpression with one ordinary expression value.
    /// No generator discriminator is injected: `Await` has a verified 1-to-1
    /// stack shape.
    pub(crate) fn resume_await_fulfill(
        self,
        code: &[Instruction],
        host: &mut impl VmHost,
        value: Value,
    ) -> Result<VmExit, Error> {
        self.resume_await(code, host, Ok(value))
    }

    /// Resume a rejected AwaitExpression through the activation's ordinary
    /// exception unwinder. This is intentionally not encoded as generator
    /// magic, so authored catch/finally regions see the rejection directly.
    pub(crate) fn resume_await_reject(
        self,
        code: &[Instruction],
        host: &mut impl VmHost,
        reason: Value,
    ) -> Result<VmExit, Error> {
        self.resume_await(code, host, Err(reason))
    }

    pub(in crate::engine::vm) fn resume_await(
        self,
        code: &[Instruction],
        host: &mut impl VmHost,
        resolution: Result<Value, Value>,
    ) -> Result<VmExit, Error> {
        if self.kind != VmSuspendKind::Await {
            return Err(Error::internal(
                "non-await suspension used async-function resume",
            ));
        }
        if !self.output_taken {
            return Err(Error::internal(
                "async suspension must be extracted before resume",
            ));
        }

        let mut activation = self.activation;
        let slot = activation
            .stack
            .last()
            .ok_or_else(|| Error::internal("await resume stack slot is missing"))?;
        if !matches!(slot, Value::Undefined) {
            return Err(Error::internal(
                "await resume stack slot was not cleared after suspension",
            ));
        }
        match resolution {
            Ok(value) => {
                *activation
                    .stack
                    .last_mut()
                    .ok_or_else(|| Error::internal("await resume stack slot is missing"))? = value;
            }
            Err(reason) => {
                if let Some(completion) = activation.raise(reason, host, code.len())? {
                    return Ok(VmExit::Complete(completion));
                }
            }
        }
        activation.run(code, host)
    }
}

pub(in crate::engine::vm) fn checked_target(target: u32, code_len: usize) -> Result<usize, Error> {
    let target = usize::try_from(target).map_err(|_| Error::internal("jump target overflow"))?;
    if target >= code_len {
        return Err(Error::internal("jump target is out of bounds"));
    }
    Ok(target)
}
