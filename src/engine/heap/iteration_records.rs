use super::*;

/// One live registration owned by a genuine `FinalizationRegistry`.
///
/// `target` and `unregister_token` are non-owning generational identities.
/// `held_value` owns its ordinary object or Symbol edge until the registration
/// is unregistered, finalized with its registry, or moved into a prepared
/// finalization job by the ordered weak-object pass.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine::heap) struct FinalizationRegistryEntry {
    pub(in crate::engine::heap) target: WeakCollectionKey,
    pub(in crate::engine::heap) held_value: RawValue,
    pub(in crate::engine::heap) unregister_token: Option<WeakCollectionKey>,
}

/// QuickJS-shaped opaque data for one genuine `FinalizationRegistry`.
///
/// The cleanup callback and creation realm are strong arena edges. Entries
/// remain in registration order so token clearing, target clearing, and job
/// preparation follow pinned QuickJS's single forward traversal.
#[doc(hidden)]
#[derive(Clone, Debug, PartialEq)]
pub struct FinalizationRegistryData {
    pub(in crate::engine::heap) callback: ObjectId,
    pub(in crate::engine::heap) realm: ContextId,
    pub(in crate::engine::heap) entries: Vec<FinalizationRegistryEntry>,
}

/// One string-key entry captured by QuickJS's `JS_GPN_SET_ENUM` enumeration.
/// `JsString` avoids storing runtime-owning `PropertyKey` roots in the heap.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForInProperty {
    pub name: JsString,
    pub enumerable: bool,
}

/// Mutable state of one hidden for-in enumeration object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForInIteratorData {
    pub object: Option<ObjectId>,
    pub index: usize,
    pub properties: Vec<ForInProperty>,
    /// The iterator, not the source object, remembers whether QuickJS selected
    /// its count-only fast-Array path at loop entry.
    pub fast_array: bool,
    pub array_count: u32,
    pub in_prototype_chain: bool,
    pub visited: HashSet<JsString>,
}

/// One non-observable step selected from a hidden for-in iterator. The
/// runtime performs live property/prototype operations only after the heap
/// borrow used to advance the cursor has ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ForInCandidate {
    Done,
    BaseComplete { object: ObjectId, fast_array: bool },
    LevelComplete(ObjectId),
    ArrayIndex { object: ObjectId, index: u32 },
    Property { object: ObjectId, name: JsString },
}
