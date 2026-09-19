use crate::engine::value::JsValue;

/// Private JavaScript control completion. A thrown value remains an owned
/// internal [`JsValue`]; no exception sentinel is exposed through the public
/// value representation.
#[derive(Debug)]
pub(crate) enum Completion {
    Return(JsValue),
    Throw(JsValue),
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
#[derive(Debug)]
pub(crate) enum VmResume {
    Next(JsValue),
    Return(JsValue),
    Throw(JsValue),
}

/// Result of QuickJS `OP_define_class` at the VM/runtime boundary.
///
/// A successful definition replaces the two input operands with two freshly
/// published outputs. JavaScript-visible failures stay typed as thrown values
/// so an enclosing bytecode catch region can handle them normally.
#[derive(Debug)]
pub(crate) enum DefineClassOutcome {
    Defined {
        constructor: JsValue,
        prototype: JsValue,
    },
    Throw(JsValue),
}

/// ECMAScript ToPrimitive hint crossing the VM/runtime host boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ToPrimitiveHint {
    Default,
    Number,
    String,
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
