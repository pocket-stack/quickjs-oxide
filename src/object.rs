//! Runtime-rooted object, symbol, property-key, and callable identities.
//!
//! QuickJS values exposed outside the heap own a reference to their underlying
//! GC object or atom.  The wrappers in this module preserve that contract in
//! safe Rust: cloning a public root duplicates the runtime reference and
//! dropping it releases that reference.  The runtime itself remains alive for
//! as long as any root exists.
//!
//! Heap payloads must not store these public wrappers.  They store raw
//! [`ObjectId`] and [`Atom`] handles and let the runtime's tracing/refcount
//! machinery account for internal edges.  This separation avoids a strong
//! `Runtime -> heap -> Runtime` ownership cycle and is the foundation for
//! QuickJS-compatible zero-reference and cycle collection.

use std::fmt;
use std::hash::{Hash, Hasher};

use crate::atom::{Atom, AtomError};
use crate::heap::{HeapError, ObjectId};
use crate::runtime::Runtime;
use crate::value::Value;

/// A public owning root for one runtime-local object identity.
///
/// The constructor is crate-private: only the runtime may turn a validated raw
/// heap handle into a public root.  Equality includes the runtime domain, so
/// numerically equal arena handles from different runtimes never alias.
pub struct ObjectRef {
    runtime: Runtime,
    id: ObjectId,
}

impl ObjectRef {
    /// Consume one object reference already owned by the caller.
    ///
    /// Allocation and raw-to-root promotion use this path after establishing
    /// that `id` is live in `runtime`; it deliberately does not retain again.
    #[must_use]
    pub(crate) const fn from_owned_handle(runtime: Runtime, id: ObjectId) -> Self {
        Self { runtime, id }
    }

    /// Promote a borrowed raw heap edge to a public owning root.
    pub(crate) fn from_borrowed_handle(runtime: Runtime, id: ObjectId) -> Result<Self, HeapError> {
        runtime.retain_object_handle(id)?;
        Ok(Self { runtime, id })
    }

    /// Duplicate this root without turning a runtime invariant failure into a
    /// panic.  The public [`Clone`] implementation delegates to this method.
    pub(crate) fn try_clone(&self) -> Result<Self, HeapError> {
        self.runtime.retain_object_handle(self.id)?;
        Ok(Self {
            runtime: self.runtime.clone(),
            id: self.id,
        })
    }

    /// Return the runtime which owns this root.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    /// Return whether this object belongs to `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }

    /// Return whether two roots belong to the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.runtime.is_same_runtime(&other.runtime)
    }

    /// Stable identity of the owning runtime domain.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.runtime.domain_id()
    }

    /// Raw identity for runtime and heap internals.
    #[must_use]
    pub(crate) const fn object_id(&self) -> ObjectId {
        self.id
    }
}

impl Clone for ObjectRef {
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|_| {
            panic!("attempted to clone a stale object root or overflow its reference count")
        })
    }
}

impl Drop for ObjectRef {
    fn drop(&mut self) {
        self.runtime.release_object_handle(self.id);
    }
}

impl PartialEq for ObjectRef {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id && self.runtime.is_same_runtime(&other.runtime)
    }
}

impl Eq for ObjectRef {}

impl Hash for ObjectRef {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.runtime.domain_id().hash(state);
        self.id.hash(state);
    }
}

impl fmt::Debug for ObjectRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ObjectRef")
            .field("domain_id", &self.runtime.domain_id())
            .field("id", &self.id)
            .finish()
    }
}

/// Shared owning implementation for atom-backed public roots.
///
/// `AtomOwner` is not public because its atom kind has not been refined.  The
/// runtime validates the kind first, then constructs [`PropertyKey`] or
/// [`SymbolRef`].  This prevents safe callers from manufacturing a symbol out
/// of a string atom while keeping retain/release behavior in one place.
struct AtomOwner {
    runtime: Runtime,
    atom: Atom,
}

impl AtomOwner {
    /// Consume one atom reference already owned by the caller.
    #[must_use]
    const fn from_owned_handle(runtime: Runtime, atom: Atom) -> Self {
        Self { runtime, atom }
    }

    /// Promote a borrowed raw atom edge to an owning root.
    fn from_borrowed_handle(runtime: Runtime, atom: Atom) -> Result<Self, AtomError> {
        runtime.retain_atom_handle(atom)?;
        Ok(Self { runtime, atom })
    }

    fn try_clone(&self) -> Result<Self, AtomError> {
        self.runtime.retain_atom_handle(self.atom)?;
        Ok(Self {
            runtime: self.runtime.clone(),
            atom: self.atom,
        })
    }

    #[must_use]
    const fn runtime(&self) -> &Runtime {
        &self.runtime
    }

    #[must_use]
    const fn atom(&self) -> Atom {
        self.atom
    }

    #[must_use]
    fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }

    #[must_use]
    fn is_same_runtime(&self, other: &Self) -> bool {
        self.runtime.is_same_runtime(&other.runtime)
    }

    #[must_use]
    fn domain_id(&self) -> u64 {
        self.runtime.domain_id()
    }
}

impl Clone for AtomOwner {
    fn clone(&self) -> Self {
        self.try_clone().unwrap_or_else(|_| {
            panic!("attempted to clone a stale atom root or overflow its reference count")
        })
    }
}

impl Drop for AtomOwner {
    fn drop(&mut self) {
        self.runtime.release_atom_handle(self.atom);
    }
}

impl PartialEq for AtomOwner {
    fn eq(&self, other: &Self) -> bool {
        self.atom == other.atom && self.runtime.is_same_runtime(&other.runtime)
    }
}

impl Eq for AtomOwner {}

impl Hash for AtomOwner {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.runtime.domain_id().hash(state);
        self.atom.hash(state);
    }
}

impl fmt::Debug for AtomOwner {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AtomOwner")
            .field("domain_id", &self.runtime.domain_id())
            .field("atom", &self.atom)
            .finish()
    }
}

/// A runtime-owned ECMAScript property key.
///
/// This wrapper carries the runtime domain missing from a raw `Atom`, so two
/// tables which happen to allocate the same numeric atom ID remain distinct.
/// Construction is restricted to runtime code which has validated that the
/// atom is a string or symbol property key.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PropertyKey(AtomOwner);

impl PropertyKey {
    /// Consume one already-owned, kind-validated atom reference.
    #[must_use]
    pub(crate) const fn from_owned_atom(runtime: Runtime, atom: Atom) -> Self {
        Self(AtomOwner::from_owned_handle(runtime, atom))
    }

    /// Promote one borrowed, kind-validated atom reference.
    pub(crate) fn from_borrowed_atom(runtime: Runtime, atom: Atom) -> Result<Self, AtomError> {
        AtomOwner::from_borrowed_handle(runtime, atom).map(Self)
    }

    /// Return the runtime which owns this key.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        self.0.runtime()
    }

    /// Return whether this key belongs to `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.0.belongs_to(runtime)
    }

    /// Return whether two keys belong to the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.0.is_same_runtime(&other.0)
    }

    /// Stable identity of the owning runtime domain.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.domain_id()
    }

    /// Raw atom identity for runtime and shape internals.
    #[must_use]
    pub(crate) const fn atom(&self) -> Atom {
        self.0.atom()
    }
}

/// The fixed well-known-symbol set in QuickJS 2026-06-04
/// (`quickjs-atom.h`, in allocation order).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WellKnownSymbol {
    ToPrimitive,
    Iterator,
    Match,
    MatchAll,
    Replace,
    Search,
    Split,
    ToStringTag,
    IsConcatSpreadable,
    HasInstance,
    Species,
    Unscopables,
    AsyncIterator,
}

impl WellKnownSymbol {
    pub const ALL: [Self; 13] = [
        Self::ToPrimitive,
        Self::Iterator,
        Self::Match,
        Self::MatchAll,
        Self::Replace,
        Self::Search,
        Self::Split,
        Self::ToStringTag,
        Self::IsConcatSpreadable,
        Self::HasInstance,
        Self::Species,
        Self::Unscopables,
        Self::AsyncIterator,
    ];

    #[must_use]
    pub const fn description(self) -> &'static str {
        match self {
            Self::ToPrimitive => "Symbol.toPrimitive",
            Self::Iterator => "Symbol.iterator",
            Self::Match => "Symbol.match",
            Self::MatchAll => "Symbol.matchAll",
            Self::Replace => "Symbol.replace",
            Self::Search => "Symbol.search",
            Self::Split => "Symbol.split",
            Self::ToStringTag => "Symbol.toStringTag",
            Self::IsConcatSpreadable => "Symbol.isConcatSpreadable",
            Self::HasInstance => "Symbol.hasInstance",
            Self::Species => "Symbol.species",
            Self::Unscopables => "Symbol.unscopables",
            Self::AsyncIterator => "Symbol.asyncIterator",
        }
    }
}

/// A runtime-owned ECMAScript Symbol primitive identity.
///
/// Unlike a raw atom, this type can only be constructed after the runtime has
/// verified a symbol atom kind.  Clone and drop retain/release through the
/// embedded [`AtomOwner`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SymbolRef(AtomOwner);

impl SymbolRef {
    /// Consume one already-owned, symbol-kind-validated atom reference.
    #[must_use]
    pub(crate) const fn from_owned_atom(runtime: Runtime, atom: Atom) -> Self {
        Self(AtomOwner::from_owned_handle(runtime, atom))
    }

    /// Promote one borrowed, symbol-kind-validated atom reference.
    pub(crate) fn from_borrowed_atom(runtime: Runtime, atom: Atom) -> Result<Self, AtomError> {
        AtomOwner::from_borrowed_handle(runtime, atom).map(Self)
    }

    /// Return the runtime which owns this symbol.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        self.0.runtime()
    }

    /// Return whether this symbol belongs to `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.0.belongs_to(runtime)
    }

    /// Return whether two symbols belong to the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.0.is_same_runtime(&other.0)
    }

    /// Stable identity of the owning runtime domain.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.domain_id()
    }

    /// Raw atom identity for runtime internals.
    #[must_use]
    pub(crate) const fn atom(&self) -> Atom {
        self.0.atom()
    }
}

impl From<SymbolRef> for PropertyKey {
    fn from(symbol: SymbolRef) -> Self {
        Self(symbol.0)
    }
}

impl From<&SymbolRef> for PropertyKey {
    fn from(symbol: &SymbolRef) -> Self {
        Self(symbol.0.clone())
    }
}

/// A callable object root whose `[[Call]]` capability has been validated.
///
/// There is intentionally no public constructor or `From<ObjectRef>` impl.
/// Runtime code must check the object's internal call method before using the
/// crate-private constructor.  Consequently an accessor can contain only
/// `undefined` or a genuinely callable root.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct CallableRef(ObjectRef);

impl CallableRef {
    /// Wrap an object after runtime code has validated its `[[Call]]` method.
    #[must_use]
    pub(crate) const fn from_validated_object(object: ObjectRef) -> Self {
        Self(object)
    }

    /// View the underlying rooted object identity.
    #[must_use]
    pub const fn as_object(&self) -> &ObjectRef {
        &self.0
    }

    /// Consume the callable capability and return its ordinary object root.
    #[must_use]
    pub fn into_object(self) -> ObjectRef {
        self.0
    }

    /// Return the owning runtime.
    #[must_use]
    pub const fn runtime(&self) -> &Runtime {
        self.0.runtime()
    }

    /// Return whether this callable belongs to `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.0.belongs_to(runtime)
    }

    /// Stable identity of the owning runtime domain.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.domain_id()
    }
}

/// Presence of one field in an ECMAScript Property Descriptor record.
///
/// This must not be collapsed to `Option<T>` when `T` itself can represent
/// JavaScript `undefined`: `Absent` and `Present(undefined)` have different
/// descriptor semantics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DescriptorField<T> {
    #[default]
    Absent,
    Present(T),
}

impl<T> DescriptorField<T> {
    #[must_use]
    pub const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    #[must_use]
    pub const fn is_present(&self) -> bool {
        matches!(self, Self::Present(_))
    }

    #[must_use]
    pub const fn as_ref(&self) -> DescriptorField<&T> {
        match self {
            Self::Absent => DescriptorField::Absent,
            Self::Present(value) => DescriptorField::Present(value),
        }
    }

    #[must_use]
    pub fn map<U>(self, map: impl FnOnce(T) -> U) -> DescriptorField<U> {
        match self {
            Self::Absent => DescriptorField::Absent,
            Self::Present(value) => DescriptorField::Present(map(value)),
        }
    }

    #[must_use]
    pub fn into_option(self) -> Option<T> {
        match self {
            Self::Absent => None,
            Self::Present(value) => Some(value),
        }
    }
}

impl<T> From<Option<T>> for DescriptorField<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            Some(value) => Self::Present(value),
            None => Self::Absent,
        }
    }
}

/// The only legal values of a present descriptor `get` or `set` field.
///
/// A non-callable JavaScript value cannot inhabit this type.  Conversion from
/// a general [`Value`] therefore belongs at the Context/runtime boundary where
/// callability can be checked and a TypeError can be raised.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum AccessorValue {
    Undefined,
    Callable(CallableRef),
}

impl AccessorValue {
    #[must_use]
    pub const fn as_callable(&self) -> Option<&CallableRef> {
        match self {
            Self::Undefined => None,
            Self::Callable(callable) => Some(callable),
        }
    }
}

/// A possibly incomplete ordinary ECMAScript Property Descriptor record.
///
/// This is an input/validation record, not an object's physical property
/// storage.  Shape flags hold C/W/E and the storage kind; the heap owns a
/// parallel data or accessor payload slot.  A descriptor may temporarily carry
/// both data and accessor fields so `ToPropertyDescriptor` can reject it, but a
/// present accessor field itself is always either `undefined` or callable.
#[derive(Clone, Debug, PartialEq)]
pub struct OrdinaryPropertyDescriptor {
    pub value: DescriptorField<Value>,
    pub writable: DescriptorField<bool>,
    pub get: DescriptorField<AccessorValue>,
    pub set: DescriptorField<AccessorValue>,
    pub enumerable: DescriptorField<bool>,
    pub configurable: DescriptorField<bool>,
}

impl OrdinaryPropertyDescriptor {
    /// Construct an empty generic descriptor.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            value: DescriptorField::Absent,
            writable: DescriptorField::Absent,
            get: DescriptorField::Absent,
            set: DescriptorField::Absent,
            enumerable: DescriptorField::Absent,
            configurable: DescriptorField::Absent,
        }
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.value.is_absent()
            && self.writable.is_absent()
            && self.get.is_absent()
            && self.set.is_absent()
            && self.enumerable.is_absent()
            && self.configurable.is_absent()
    }

    #[must_use]
    pub const fn is_data_descriptor(&self) -> bool {
        self.value.is_present() || self.writable.is_present()
    }

    #[must_use]
    pub const fn is_accessor_descriptor(&self) -> bool {
        self.get.is_present() || self.set.is_present()
    }

    #[must_use]
    pub const fn is_generic_descriptor(&self) -> bool {
        !self.is_data_descriptor() && !self.is_accessor_descriptor()
    }

    /// Whether this record must be rejected for mixing data and accessor fields.
    #[must_use]
    pub const fn is_mixed_descriptor(&self) -> bool {
        self.is_data_descriptor() && self.is_accessor_descriptor()
    }
}

impl Default for OrdinaryPropertyDescriptor {
    fn default() -> Self {
        Self::new()
    }
}

/// A fully materialized ordinary property descriptor returned by
/// `[[GetOwnProperty]]` and used to bridge validation to physical shape/slot
/// storage.
#[derive(Clone, Debug, PartialEq)]
pub enum CompleteOrdinaryPropertyDescriptor {
    Data {
        value: Value,
        writable: bool,
        enumerable: bool,
        configurable: bool,
    },
    Accessor {
        get: Option<CallableRef>,
        set: Option<CallableRef>,
        enumerable: bool,
        configurable: bool,
    },
}

impl CompleteOrdinaryPropertyDescriptor {
    #[must_use]
    pub const fn enumerable(&self) -> bool {
        match self {
            Self::Data { enumerable, .. } | Self::Accessor { enumerable, .. } => *enumerable,
        }
    }

    #[must_use]
    pub const fn configurable(&self) -> bool {
        match self {
            Self::Data { configurable, .. } | Self::Accessor { configurable, .. } => *configurable,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AccessorValue, DescriptorField, OrdinaryPropertyDescriptor};
    use crate::value::Value;

    #[test]
    fn descriptor_field_preserves_absent_and_present_undefined() {
        let absent = DescriptorField::<AccessorValue>::Absent;
        let undefined = DescriptorField::Present(AccessorValue::Undefined);

        assert!(absent.is_absent());
        assert!(undefined.is_present());
        assert_ne!(absent, undefined);
    }

    #[test]
    fn descriptor_classification_matches_ecmascript_records() {
        let generic = OrdinaryPropertyDescriptor {
            enumerable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(generic.is_generic_descriptor());

        let data = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Undefined),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(data.is_data_descriptor());
        assert!(!data.is_mixed_descriptor());

        let accessor = OrdinaryPropertyDescriptor {
            get: DescriptorField::Present(AccessorValue::Undefined),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(accessor.is_accessor_descriptor());

        let mixed = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Int(1)),
            set: DescriptorField::Present(AccessorValue::Undefined),
            ..OrdinaryPropertyDescriptor::new()
        };
        assert!(mixed.is_mixed_descriptor());
    }

    #[test]
    fn descriptor_field_mapping_does_not_collapse_presence() {
        assert_eq!(
            DescriptorField::Present(21).map(|value| value * 2),
            DescriptorField::Present(42)
        );
        assert_eq!(
            DescriptorField::<i32>::Absent.map(|value| value * 2),
            DescriptorField::Absent
        );
    }
}
