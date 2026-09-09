use super::*;

/// Private JavaScript control completion. A thrown value remains a rooted
/// ordinary [`Value`]; no exception sentinel is exposed through the public
/// value representation.
#[derive(Debug, PartialEq)]
pub(crate) enum Completion {
    Return(Value),
    Throw(Value),
}

/// The suspension sites used by QuickJS generator and async-function bytecode.
///
/// `Initial` is the hidden prologue barrier reached while constructing a
/// generator and has no yielded operand. `Yield` and `YieldStar` both preserve
/// their output in the activation's top stack slot until the generator driver
/// extracts it. `AsyncYieldStar` carries the already-extracted delegated value
/// rather than the synchronous iterator-result object. `Await` retains the
/// awaited operand in the same owned activation, but its resume protocol is
/// deliberately separate from the generator value-plus-magic ABI.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum VmSuspendKind {
    Initial,
    Yield,
    YieldStar,
    AsyncYieldStar,
    Await,
}

/// A caller-supplied completion used to resume a suspended generator.
///
/// The VM translates this typed boundary back to QuickJS's private stack ABI:
/// next/return/throw are magic integers 0/1/2. A plain `yield` throw is the one
/// exception: it is raised through the activation's normal unwind machinery,
/// while `yield*` receives value + magic 2 so the compiled delegation loop can
/// invoke the delegate's `throw` method.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum VmResume {
    Next(Value),
    Return(Value),
    Throw(Value),
}

/// A resumable VM run either completes normally/abruptly or transfers its
/// owned activation to the generator driver.
#[derive(Debug, PartialEq)]
pub(crate) enum VmExit {
    Complete(Completion),
    Suspend(VmSuspension),
}

/// Result of QuickJS `OP_define_class` at the VM/runtime boundary.
///
/// A successful definition replaces the two input operands with two freshly
/// published outputs. JavaScript-visible failures stay typed as thrown values
/// so an enclosing bytecode catch region can handle them normally.
#[derive(Debug, PartialEq)]
pub(crate) enum DefineClassOutcome {
    Defined {
        constructor: Value,
        prototype: Value,
    },
    Throw(Value),
}

/// ECMAScript ToPrimitive hint crossing the VM/runtime host boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToPrimitiveHint {
    Default,
    Number,
    String,
}

pub(in crate::engine::vm) enum OperationOutcome<T> {
    Value(T),
    Throw(Value),
}

/// Result of the observable `GetIterator` and `Get(iterator, "next")`
/// operations used by `ForOfStart`.
pub(crate) enum ForOfStartOutcome {
    Record { iterator: Value, next_method: Value },
    Throw(Value),
}

/// Append-specific iterator start. Pinned QuickJS performs one additional
/// `@@iterator` Get and may snapshot a genuine fast Array after creating the
/// second iterator record; ordinary for-of must not inherit those quirks.
pub(crate) enum AppendStartOutcome {
    Record {
        iterator: Value,
        next_method: Value,
        fast_values: Option<Vec<Value>>,
    },
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

/// Result of QuickJS `build_arg_list` at the VM/runtime boundary.
///
/// The compiler-created spread Array is still a JavaScript value, so length
/// and indexed reads may throw. Keep those throws explicit so `ApplyEval` can
/// preserve upstream's ordering before the original-eval identity check.
pub(crate) enum ArgumentListOutcome {
    Values(Vec<Value>),
    Throw(Value),
}

/// Stable instruction offset recorded on an active bytecode frame.
///
/// The runtime deliberately records the offset of the instruction currently
/// being executed, rather than the VM's already-advanced dispatch cursor. A
/// nested call therefore leaves its caller parked on the `Call` or
/// `Construct` opcode, matching the frame information QuickJS retains for
/// later exception-stack construction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct BytecodePc(pub(in crate::engine::vm) usize);

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
