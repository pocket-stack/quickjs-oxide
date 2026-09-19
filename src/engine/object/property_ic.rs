//! Per-executable static-key location facts. Entries own no object, atom, or value.
//! A hit reads today's parallel data slot, never a value retained by the cache.
use std::cell::Cell;

use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::atom::{Atom, AtomIdx, AtomTable};
use crate::engine::code::bytecode::Instruction;
use crate::engine::heap::{ContextId, Heap, ObjectId, ObjectKind, PropertySlot, RawValue, ShapeId};
use crate::engine::value::Value;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Location {
    domain: u64,
    realm: ContextId,
    shape: ShapeId,
    revision: u64,
    // Only prototype hits depend on other objects' layouts.
    prototype_epoch: u64,
    depth: u32,
    slot: u32,
    numeric_key: bool,
}

#[derive(Clone, Copy, Debug, Default)]
enum State {
    #[default]
    Cold,
    Monomorphic(Location),
    Polymorphic([Location; 2]),
    Megamorphic(u16),
}

/// Two guarded locations cover alternating shapes. Unsupported/overflow sites
/// periodically retry specialization, without retaining object or value owners.
#[derive(Debug, Default)]
pub(crate) struct PropertyReadCache {
    state: Cell<State>,
}

impl PropertyReadCache {
    /// The caller holds the heap borrow until it has retained/copied the value.
    /// No raw borrowed handle escapes that boundary.
    pub(crate) fn read<'a>(
        &self,
        heap: &'a Heap,
        domain: u64,
        realm: ContextId,
        receiver: ObjectId,
    ) -> Option<&'a RawValue> {
        match self.state.get() {
            State::Cold => None,
            State::Monomorphic(location) => {
                Self::read_location(location, heap, domain, realm, receiver)
            }
            State::Polymorphic([first, second]) => {
                if let Some(value) = Self::read_location(first, heap, domain, realm, receiver) {
                    Some(value)
                } else {
                    let value = Self::read_location(second, heap, domain, realm, receiver)?;
                    self.state.set(State::Polymorphic([second, first]));
                    Some(value)
                }
            }
            State::Megamorphic(left) => {
                if left <= 1 {
                    self.state.set(State::Cold);
                    event("property_ic.revive");
                } else {
                    self.state.set(State::Megamorphic(left - 1));
                }
                None
            }
        }
    }

    fn read_location(
        location: Location,
        heap: &Heap,
        domain: u64,
        realm: ContextId,
        receiver: ObjectId,
    ) -> Option<&RawValue> {
        if location.domain != domain || location.realm != realm {
            return None;
        }
        let object = heap.object_fast(receiver);
        if !ordinary_receiver(object, location.numeric_key) {
            return None;
        }
        if object.shape != location.shape {
            return None;
        }
        let shape = heap.shape_fast(object.shape);
        if shape.layout_revision() != location.revision {
            return None;
        }
        let mut holder = receiver;
        if location.depth != 0 {
            if heap.property_layout_epoch() != location.prototype_epoch {
                return None;
            }
            for _ in 0..location.depth {
                let data = heap.object_fast(holder);
                holder = heap.shape_fast(data.shape).prototype()?;
            }
        }
        match heap.object_fast(holder).slots.get(location.slot as usize)? {
            PropertySlot::Data(value) => Some(value),
            // VarRef/AutoInit can share data-shaped storage; never treat them
            // as immutable data, even if an internal slot writer changed kind.
            _ => None,
        }
    }

    /// Called once on a miss, before the canonical read. This is observational:
    /// it neither roots a value nor invokes an accessor/exotic operation.
    pub(crate) fn miss(
        &self,
        heap: &Heap,
        atoms: &AtomTable,
        domain: u64,
        realm: ContextId,
        receiver: Option<ObjectId>,
        atom: Atom,
    ) {
        let state = self.state.get();
        if matches!(state, State::Megamorphic(_)) {
            return;
        }
        let Some(location) = receiver.and_then(|r| locate(heap, atoms, domain, realm, r, atom))
        else {
            self.state.set(State::Megamorphic(1024));
            event("property_ic.megamorphic");
            return;
        };
        // A revision change of the same shape replaces stale knowledge instead
        // of spending another polymorphic slot on an unreachable old revision.
        let same_key = |old: Location| {
            old.domain == location.domain
                && old.realm == location.realm
                && old.shape == location.shape
        };
        let next = match state {
            State::Cold => State::Monomorphic(location),
            State::Monomorphic(old) if same_key(old) => State::Monomorphic(location),
            State::Monomorphic(old) => State::Polymorphic([location, old]),
            State::Polymorphic([first, second]) if same_key(first) => {
                State::Polymorphic([location, second])
            }
            State::Polymorphic([first, second]) if same_key(second) => {
                State::Polymorphic([location, first])
            }
            _ => {
                event("property_ic.megamorphic");
                State::Megamorphic(1024)
            }
        };
        self.state.set(next);
        event("property_ic.miss");
    }
}

impl Runtime {
    /// Resolve one Proxy trap method through the per-trap location cache.
    ///
    /// A hit reads today's data slot and returns an owned value retained under
    /// the exclusive heap borrow. A miss records the location (data slots only)
    /// and returns `None`, so the caller keeps its canonical dynamic read.
    pub(crate) fn proxy_trap_read(
        &self,
        trap: usize,
        realm: ContextId,
        handler: ObjectId,
        atom: Atom,
    ) -> Result<Option<Value>, RuntimeError> {
        let raw = {
            let state = self.0.state.borrow();
            let cache = &state.proxy_trap_reads[trap];
            match cache.read(&state.heap, self.domain_id(), realm, handler) {
                Some(raw) => {
                    if matches!(
                        raw,
                        RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
                    ) {
                        return Ok(None);
                    }
                    raw.clone()
                }
                None => {
                    cache.miss(
                        &state.heap,
                        &state.atoms,
                        self.domain_id(),
                        realm,
                        Some(handler),
                        atom,
                    );
                    return Ok(None);
                }
            }
        };
        // String/BigInt clone their backing owner; Object/Symbol retain their
        // heap count. The handler slot owner keeps the source alive meanwhile.
        let mut state = self.0.state.borrow_mut();
        state.retain_raw_root(&raw)?;
        drop(state);
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("proxy_trap_read.hit");
        Ok(Some(self.take_owned_raw_value(raw)?))
    }
}

fn ordinary_receiver(data: &crate::engine::heap::ObjectData, numeric: bool) -> bool {
    match data.kind {
        ObjectKind::Proxy | ObjectKind::ModuleNamespace => false,
        // Indexed exotics may intercept keys before ordinary shape lookup.
        ObjectKind::Array
        | ObjectKind::Arguments
        | ObjectKind::Primitive
        | ObjectKind::TypedArray => !numeric,
        ObjectKind::Ordinary
        | ObjectKind::Iterator
        | ObjectKind::ArrayIterator
        | ObjectKind::ForInIterator
        | ObjectKind::Date
        | ObjectKind::RegExp
        | ObjectKind::RegExpStringIterator
        | ObjectKind::Map
        | ObjectKind::MapIterator
        | ObjectKind::Set
        | ObjectKind::SetIterator
        | ObjectKind::WeakMap
        | ObjectKind::WeakSet
        | ObjectKind::WeakRef
        | ObjectKind::FinalizationRegistry
        | ObjectKind::GlobalObject
        | ObjectKind::Error
        | ObjectKind::StringIterator
        | ObjectKind::IteratorHelper
        | ObjectKind::IteratorWrap
        | ObjectKind::AsyncFromSyncIterator
        | ObjectKind::IteratorConcat
        | ObjectKind::ArrayBuffer
        | ObjectKind::SharedArrayBuffer
        | ObjectKind::DataView
        | ObjectKind::NativeFunction
        | ObjectKind::BoundFunction
        | ObjectKind::BytecodeFunction
        | ObjectKind::Generator
        | ObjectKind::AsyncGenerator
        | ObjectKind::AsyncFunctionState
        | ObjectKind::Promise => true,
    }
}

fn locate(
    heap: &Heap,
    atoms: &AtomTable,
    domain: u64,
    realm: ContextId,
    receiver: ObjectId,
    atom: Atom,
) -> Option<Location> {
    let initial = heap.object(receiver).ok()?;
    let initial_shape = heap.shape(initial.shape).ok()?;
    let revision = initial_shape.layout_revision();
    let epoch = heap.property_layout_epoch();
    if revision == u64::MAX || epoch == u64::MAX {
        return None;
    }
    let numeric = atoms.array_index(atom).ok()?.is_some()
        || (atoms.property_key_kind(atom).ok()? == crate::engine::atom::PropertyKeyKind::String
            && {
                // Conservative, allocation-free superset of CanonicalNumericIndexString.
                // TypedArray intercepts -0/NaN/Infinity and non-array-index numbers.
                let spelling = atoms.to_js_string(atom).ok()?;
                let first = spelling.utf16_units().next();
                matches!(first, Some(43 | 45 | 46 | 48..=57))
                    || spelling.utf16_units().eq("NaN".encode_utf16())
                    || spelling.utf16_units().eq("Infinity".encode_utf16())
            });
    let mut holder = receiver;
    let mut depth = 0u32;
    loop {
        let data = heap.object(holder).ok()?;
        if !ordinary_receiver(data, numeric) {
            return None;
        }
        let shape = heap.shape(data.shape).ok()?;
        if let Some(slot) = shape.find(AtomIdx::from_raw(atom.raw())) {
            return matches!(data.slots.get(slot as usize), Some(PropertySlot::Data(_))).then_some(
                Location {
                    domain,
                    realm,
                    shape: initial.shape,
                    revision,
                    prototype_epoch: epoch,
                    depth,
                    slot,
                    numeric_key: numeric,
                },
            );
        }
        holder = shape.prototype()?;
        depth = depth.checked_add(1)?;
    }
}

/// Own writable data locations use the same domain/revision/revival guards.
#[derive(Debug, Default)]
pub(crate) struct PropertyWriteCache(PropertyReadCache);
impl PropertyWriteCache {
    pub(crate) fn slot(
        &self,
        heap: &Heap,
        domain: u64,
        realm: ContextId,
        receiver: ObjectId,
    ) -> Option<usize> {
        self.0.read(heap, domain, realm, receiver)?;
        let location = match self.0.state.get() {
            State::Monomorphic(location) => location,
            State::Polymorphic([first, _]) => first,
            _ => return None,
        };
        if location.depth != 0 {
            return None;
        }
        if matches!(heap.object(receiver).ok()?.kind, ObjectKind::Array) && location.slot == 0 {
            return None;
        }
        let shape = heap.shape(location.shape).ok()?;
        shape
            .entries()
            .get(location.slot as usize)?
            .flags
            .writable
            .then_some(location.slot as usize)
    }
    pub(crate) fn miss(
        &self,
        heap: &Heap,
        atoms: &AtomTable,
        domain: u64,
        realm: ContextId,
        receiver: ObjectId,
        atom: Atom,
    ) {
        self.0
            .miss(heap, atoms, domain, realm, Some(receiver), atom);
    }
}
#[derive(Debug)]
enum PropertyCache {
    Read(PropertyReadCache),
    Write(PropertyWriteCache),
}
#[derive(Debug)]
pub(crate) struct PropertyReadCacheTable {
    site_bits: Box<[u64]>,
    block_ranks: Box<[u32]>,
    sites: Box<[PropertyCache]>,
}
impl PropertyReadCacheTable {
    pub(crate) fn new(code: &[Instruction]) -> Self {
        let count = code
            .iter()
            .filter(|instruction| {
                matches!(
                    instruction,
                    Instruction::GetField(_) | Instruction::GetField2(_) | Instruction::PutField(_)
                )
            })
            .count();
        let mut sites = Vec::with_capacity(count);
        let mut bits = vec![0u64; code.len().div_ceil(64)];
        let mut ranks = vec![0u32; bits.len()];
        for (pc, instruction) in code.iter().enumerate() {
            if pc % 64 == 0 {
                ranks[pc / 64] = u32::try_from(sites.len()).expect("bytecode site count fits u32");
            }
            let cache = match instruction {
                Instruction::GetField(_) | Instruction::GetField2(_) => {
                    PropertyCache::Read(PropertyReadCache::default())
                }
                Instruction::PutField(_) => PropertyCache::Write(PropertyWriteCache::default()),
                _ => continue,
            };
            bits[pc / 64] |= 1u64 << (pc % 64);
            sites.push(cache);
        }
        Self {
            site_bits: bits.into_boxed_slice(),
            block_ranks: ranks.into_boxed_slice(),
            sites: sites.into_boxed_slice(),
        }
    }
    fn site_index(&self, pc: usize) -> Option<usize> {
        let bits = *self.site_bits.get(pc / 64)?;
        let mask = 1u64 << (pc % 64);
        if bits & mask == 0 {
            return None;
        }
        Some(self.block_ranks[pc / 64] as usize + (bits & (mask - 1)).count_ones() as usize)
    }
    pub(crate) fn site(&self, pc: usize) -> Option<&PropertyReadCache> {
        match self.sites.get(self.site_index(pc)?)? {
            PropertyCache::Read(cache) => Some(cache),
            _ => None,
        }
    }
    pub(crate) fn write_site(&self, pc: usize) -> Option<&PropertyWriteCache> {
        match self.sites.get(self.site_index(pc)?)? {
            PropertyCache::Write(cache) => Some(cache),
            _ => None,
        }
    }
}

#[inline]
fn event(name: &'static str) {
    #[cfg(feature = "profiling")]
    crate::engine::api::profiling::record_owned_execution_event(name);
    #[cfg(not(feature = "profiling"))]
    let _ = name;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::api::runtime::Runtime;
    use crate::engine::value::Value;

    fn object(value: Value) -> crate::engine::object::ObjectRef {
        let Value::Object(value) = value else {
            panic!("expected object")
        };
        value
    }
    fn install(
        cache: &PropertyReadCache,
        runtime: &Runtime,
        realm: ContextId,
        object: &crate::engine::object::ObjectRef,
        atom: Atom,
    ) {
        let state = runtime.0.state.borrow();
        cache.miss(
            &state.heap,
            &state.atoms,
            runtime.domain_id(),
            realm,
            Some(object.object_id()),
            atom,
        );
    }
    fn number(
        cache: &PropertyReadCache,
        runtime: &Runtime,
        realm: ContextId,
        object: &crate::engine::object::ObjectRef,
    ) -> Option<f64> {
        let state = runtime.0.state.borrow();
        cache
            .read(&state.heap, runtime.domain_id(), realm, object.object_id())
            .and_then(|value| match value {
                RawValue::Int(n) => Some(f64::from(*n)),
                RawValue::Float(n) => Some(*n),
                _ => None,
            })
    }
    #[test]
    fn sparse_site_rank_crosses_words_and_distinguishes_writes() {
        let mut code = vec![Instruction::Nop; 130];
        code[0] = Instruction::GetField(0);
        code[63] = Instruction::PutField(1);
        code[64] = Instruction::GetField2(2);
        code[65] = Instruction::PutField(3);
        code[129] = Instruction::GetField(4);
        let table = PropertyReadCacheTable::new(&code);
        for (rank, pc) in [0, 63, 64, 65, 129].into_iter().enumerate() {
            assert_eq!(table.site_index(pc), Some(rank));
        }
        for pc in [1, 62, 66, 128, 130, usize::MAX] {
            assert_eq!(table.site_index(pc), None);
        }
        assert!(table.site(0).is_some());
        assert!(table.site(63).is_none());
        assert!(table.write_site(63).is_some());
        assert!(table.write_site(64).is_none());
        assert_eq!(table.site_bits.len(), 3);
        assert_eq!(table.block_ranks.len(), 3);
        assert!(PropertyReadCacheTable::new(&[]).site(0).is_none());
    }

    #[test]
    fn two_shapes_alternate_and_third_shape_eventually_revives() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let first = object(context.eval("({x:1})").unwrap());
        let second = object(context.eval("({y:0,x:2})").unwrap());
        let third = object(context.eval("({z:0,y:0,x:3})").unwrap());
        let key = runtime.intern_property_key("x").unwrap();
        let realm = context.realm_id();
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, realm, &first, key.atom());
        install(&cache, &runtime, realm, &second, key.atom());
        assert!(matches!(cache.state.get(), State::Polymorphic(_)));
        for _ in 0..8 {
            assert_eq!(number(&cache, &runtime, realm, &first), Some(1.0));
            assert_eq!(number(&cache, &runtime, realm, &second), Some(2.0));
        }
        install(&cache, &runtime, realm, &third, key.atom());
        assert!(matches!(cache.state.get(), State::Megamorphic(_)));
        for _ in 0..1024 {
            assert_eq!(number(&cache, &runtime, realm, &first), None);
        }
        assert!(matches!(cache.state.get(), State::Cold));
        install(&cache, &runtime, realm, &third, key.atom());
        assert_eq!(number(&cache, &runtime, realm, &third), Some(3.0));
    }

    #[test]
    fn exotic_named_storage_is_cached_but_typed_numeric_keys_are_not() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let key = runtime.intern_property_key("x").unwrap();
        for expression in ["new Map()", "new Date()", "new Uint8Array(2)"] {
            let receiver = object(
                context
                    .eval(&format!("var exotic={expression}; exotic.x=7; exotic"))
                    .unwrap(),
            );
            let cache = PropertyReadCache::default();
            install(&cache, &runtime, context.realm_id(), &receiver, key.atom());
            assert_eq!(
                number(&cache, &runtime, context.realm_id(), &receiver),
                Some(7.0)
            );
        }
        let typed = object(context.eval("new Uint8Array(2)").unwrap());
        for spelling in ["0", "-0", "NaN", "Infinity", "1.5"] {
            let key = runtime.intern_property_key(spelling).unwrap();
            let cache = PropertyReadCache::default();
            install(&cache, &runtime, context.realm_id(), &typed, key.atom());
            assert!(matches!(cache.state.get(), State::Megamorphic(_)));
        }
    }

    #[test]
    fn cached_location_reads_replaced_value_and_unsupported_miss_cools_down() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let obj = object(context.eval("var o = {x:1}; o").unwrap());
        let key = runtime.intern_property_key("x").unwrap();
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        assert_eq!(
            number(&cache, &runtime, context.realm_id(), &obj),
            Some(1.0)
        );
        context.eval("o.x=9").unwrap();
        assert_eq!(
            number(&cache, &runtime, context.realm_id(), &obj),
            Some(9.0)
        );
        context.eval("delete o.x").unwrap();
        assert_eq!(number(&cache, &runtime, context.realm_id(), &obj), None);
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        context.eval("o.x=11").unwrap();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        assert!(matches!(cache.state.get(), State::Megamorphic(_)));
        assert_eq!(number(&cache, &runtime, context.realm_id(), &obj), None);
    }
    #[test]
    fn attributes_and_prototype_replacement_invalidate_before_accessor_execution() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let key = runtime.intern_property_key("x").unwrap();
        for mutation in [
            "Object.defineProperty(o,'x',{get(){throw 7}})",
            "Object.defineProperty(o,'x',{writable:false})",
            "Object.setPrototypeOf(o,{x:6})",
        ] {
            let obj = object(context.eval("var p={x:3}; var o={x:1}; o").unwrap());
            let cache = PropertyReadCache::default();
            install(&cache, &runtime, context.realm_id(), &obj, key.atom());
            context.eval(mutation).unwrap();
            assert_eq!(
                number(&cache, &runtime, context.realm_id(), &obj),
                None,
                "{mutation}"
            );
        }
    }
    #[test]
    fn prototype_holder_mutation_and_shadowing_invalidate_without_caching_values() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let key = runtime.intern_property_key("x").unwrap();
        for mutation in [
            "delete p.x",
            "m.x=4",
            "Object.setPrototypeOf(m,{x:5})",
            "Object.defineProperty(p,'x',{get(){throw 7}})",
        ] {
            let obj = object(
                context
                    .eval("var p={x:3}; var m=Object.create(p); var o=Object.create(m); o")
                    .unwrap(),
            );
            let cache = PropertyReadCache::default();
            install(&cache, &runtime, context.realm_id(), &obj, key.atom());
            assert_eq!(
                number(&cache, &runtime, context.realm_id(), &obj),
                Some(3.0)
            );
            context.eval("p.x=8").unwrap();
            assert_eq!(
                number(&cache, &runtime, context.realm_id(), &obj),
                Some(8.0)
            );
            context.eval(mutation).unwrap();
            assert_eq!(
                number(&cache, &runtime, context.realm_id(), &obj),
                None,
                "{mutation}"
            );
        }
    }
    #[test]
    fn dictionary_slot_swap_and_new_key_invalidate() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let obj = object(
            context
                .eval("var o={x:1}; for(var i=0;i<100;i++)o['p'+i]=i; delete o.p0; o")
                .unwrap(),
        );
        assert!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .shape(
                    runtime
                        .0
                        .state
                        .borrow()
                        .heap
                        .object(obj.object_id())
                        .unwrap()
                        .shape
                )
                .unwrap()
                .is_dictionary()
        );
        let key = runtime.intern_property_key("x").unwrap();
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        assert_eq!(
            number(&cache, &runtime, context.realm_id(), &obj),
            Some(1.0)
        );
        context.eval("delete o.p1").unwrap();
        assert_eq!(number(&cache, &runtime, context.realm_id(), &obj), None);
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        context.eval("o.more=6").unwrap();
        assert_eq!(number(&cache, &runtime, context.realm_id(), &obj), None);
    }
    #[test]
    fn realm_and_runtime_identity_never_alias_and_proxy_is_not_admitted() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let other = context.runtime().new_context();
        let obj = object(context.eval("({x:1})").unwrap());
        let key = runtime.intern_property_key("x").unwrap();
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        assert_eq!(number(&cache, &runtime, other.realm_id(), &obj), None);
        let state = runtime.0.state.borrow();
        assert!(
            cache
                .read(
                    &state.heap,
                    runtime.domain_id() + 1,
                    context.realm_id(),
                    obj.object_id()
                )
                .is_none()
        );
        drop(state);
        let proxy = object(context.eval("new Proxy({x:2},{get(){throw 9}})").unwrap());
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &proxy, key.atom());
        assert_eq!(number(&cache, &runtime, context.realm_id(), &proxy), None);
    }
    #[test]
    fn named_array_cache_survives_value_write_and_invalidates_holey_materialization() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let obj = object(context.eval("var o=[1,2,3]; o.x=4; o").unwrap());
        let key = runtime.intern_property_key("x").unwrap();
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        assert_eq!(
            number(&cache, &runtime, context.realm_id(), &obj),
            Some(4.0)
        );
        context.eval("o.x=5; o[0]=8").unwrap();
        assert_eq!(
            number(&cache, &runtime, context.realm_id(), &obj),
            Some(5.0)
        );
        context.eval("delete o[1]").unwrap();
        assert_eq!(number(&cache, &runtime, context.realm_id(), &obj), None);
    }
    #[test]
    fn entering_dictionary_storage_invalidates_an_existing_own_fact() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let obj = object(context.eval("var o={x:1}; o").unwrap());
        let key = runtime.intern_property_key("x").unwrap();
        let cache = PropertyReadCache::default();
        install(&cache, &runtime, context.realm_id(), &obj, key.atom());
        assert_eq!(
            number(&cache, &runtime, context.realm_id(), &obj),
            Some(1.0)
        );
        context
            .eval("for(var i=0;i<100;i++) o['p'+i]=i; delete o.p0")
            .unwrap();
        assert_eq!(number(&cache, &runtime, context.realm_id(), &obj), None);
    }
    #[test]
    fn prototype_attribute_changes_invalidate_epoch_with_unchanged_receiver_layout() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let key = runtime.intern_property_key("x").unwrap();
        for dictionary in [false, true] {
            for mutation in [
                "Object.defineProperty(p,'x',{writable:false})",
                "Object.defineProperty(p,'x',{enumerable:false})",
                "Object.defineProperty(p,'x',{get(){calls++; return 17}})",
                "Object.defineProperty(m,'x',{get(){calls++; return 19},configurable:true})",
            ] {
                let receiver = object(context.eval("var calls=0; var p={x:3}; var m=Object.create(p); var o=Object.create(m); o").unwrap());
                if dictionary {
                    let holder = object(context.eval("p").unwrap());
                    runtime
                        .0
                        .state
                        .borrow_mut()
                        .ensure_dictionary_layout(holder.object_id())
                        .unwrap();
                }
                let cache = PropertyReadCache::default();
                install(&cache, &runtime, context.realm_id(), &receiver, key.atom());
                assert_eq!(
                    number(&cache, &runtime, context.realm_id(), &receiver),
                    Some(3.0)
                );
                let (shape, revision, epoch) = {
                    let state = runtime.0.state.borrow();
                    let shape = state.heap.object(receiver.object_id()).unwrap().shape;
                    (
                        shape,
                        state.heap.shape(shape).unwrap().layout_revision(),
                        state.heap.property_layout_epoch(),
                    )
                };
                context.eval(mutation).unwrap();
                let state = runtime.0.state.borrow();
                assert_eq!(
                    state.heap.object(receiver.object_id()).unwrap().shape,
                    shape
                );
                assert_eq!(state.heap.shape(shape).unwrap().layout_revision(), revision);
                assert!(
                    state.heap.property_layout_epoch() > epoch,
                    "dictionary={dictionary}: {mutation}"
                );
                drop(state);
                assert_eq!(
                    number(&cache, &runtime, context.realm_id(), &receiver),
                    None
                );
                assert_eq!(context.eval("calls").unwrap(), Value::Int(0));
            }
        }
    }
}
