use super::*;

/// ECMAScript-visible state of one genuine Promise object.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseState {
    Pending,
    Fulfilled,
    Rejected,
}

/// Which settlement path owns a Promise reaction record.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PromiseReactionKind {
    Fulfill,
    Reject,
}

/// Raw, heap-owned resolving functions retained by a Promise reaction.
///
/// These are internal arena edges, not public runtime-owning wrappers.  A
/// reaction keeps both callables alive until it is detached or its owning
/// Promise is finalized. The result Promise itself is returned synchronously
/// from `then`; QuickJS does not retain it as a separate reaction edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromiseCapabilityData {
    pub resolve: ObjectId,
    pub reject: ObjectId,
}

/// One `PerformPromiseThen` reaction retained by a pending Promise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromiseReaction {
    pub kind: PromiseReactionKind,
    pub handler: Option<ObjectId>,
    /// `None` is QuickJS's thrown-away capability optimization used by the
    /// internal reactions which resume a suspended async function.
    pub capability: Option<PromiseCapabilityData>,
}

/// Complete hidden state of one genuine Promise object.
///
/// `result` is `undefined` while pending.  Reaction vectors are kept separate
/// to preserve QuickJS's fulfill/reject list order, while each record also
/// carries its kind so queued jobs remain self-describing after detachment.
#[derive(Clone, Debug, PartialEq)]
pub struct PromiseData {
    pub state: PromiseState,
    pub result: RawValue,
    pub fulfill_reactions: Vec<PromiseReaction>,
    pub reject_reactions: Vec<PromiseReaction>,
    pub is_handled: bool,
}

/// Mutable edge capture owned by an internal NewPromiseCapability executor.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromiseCapabilityExecutorData {
    pub resolve: Option<RawValue>,
    pub reject: Option<RawValue>,
}

pub(in crate::engine::heap) const fn is_promise_storable_value(value: &RawValue) -> bool {
    !matches!(
        value,
        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
    )
}
