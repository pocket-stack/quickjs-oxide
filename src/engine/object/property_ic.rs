//! Per-executable static-key location facts. Entries own no object, atom, or value.
//! A hit reads today's parallel data slot, never a value retained by the cache.
use std::cell::Cell;

use crate::engine::atom::{Atom, AtomTable};
use crate::engine::code::bytecode::Instruction;
use crate::engine::heap::{
    ContextId, Heap, ObjectId, ObjectKind, ObjectPayload, PropertySlot, RawValue, ShapeId,
};

#[derive(Clone, Copy, Debug)]
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
    Megamorphic,
}

/// One first miss may install a monomorphic fact. The next miss (including
/// invalidation, a different runtime/realm, or an unsupported receiver) disables
/// that site permanently. A cold unsupported miss also counts toward the two.
#[derive(Debug, Default)]
pub(crate) struct PropertyReadCache {
    state: Cell<State>,
    misses: Cell<u8>,
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
        let State::Monomorphic(location) = self.state.get() else {
            return None;
        };
        if location.domain != domain || location.realm != realm {
            return None;
        }
        let object = heap.object(receiver).ok()?;
        if !ordinary_receiver(object, location.numeric_key) {
            return None;
        }
        if object.shape != location.shape {
            return None;
        }
        let shape = heap.shape(object.shape).ok()?;
        if shape.layout_revision() != location.revision {
            return None;
        }
        let mut holder = receiver;
        if location.depth != 0 {
            if heap.property_layout_epoch() != location.prototype_epoch {
                return None;
            }
            for _ in 0..location.depth {
                let data = heap.object(holder).ok()?;
                holder = heap.shape(data.shape).ok()?.prototype()?;
            }
        }
        match heap
            .object(holder)
            .ok()?
            .slots
            .get(location.slot as usize)?
        {
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
        if matches!(self.state.get(), State::Megamorphic) {
            return;
        }
        let misses = self.misses.get() + 1;
        self.misses.set(misses);
        if misses >= 2 {
            self.state.set(State::Megamorphic);
            event("property_ic.megamorphic");
            return;
        }
        let location =
            receiver.and_then(|receiver| locate(heap, atoms, domain, realm, receiver, atom));
        self.state
            .set(location.map_or(State::Cold, State::Monomorphic));
        event("property_ic.miss");
    }
}

fn ordinary_receiver(data: &crate::engine::heap::ObjectData, numeric: bool) -> bool {
    match (data.kind, &data.payload) {
        (ObjectKind::Ordinary, ObjectPayload::Ordinary) => true,
        (ObjectKind::Array, ObjectPayload::Array { .. }) => !numeric,
        _ => false,
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
    let numeric = atoms.array_index(atom).ok()?.is_some();
    let mut holder = receiver;
    let mut depth = 0u32;
    loop {
        let data = heap.object(holder).ok()?;
        if !ordinary_receiver(data, numeric) {
            return None;
        }
        let shape = heap.shape(data.shape).ok()?;
        if let Some(slot) = shape.find(atom) {
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

#[derive(Debug)]
pub(crate) struct PropertyReadCacheTable {
    indices: Box<[u32]>,
    sites: Box<[PropertyReadCache]>,
}
impl PropertyReadCacheTable {
    pub(crate) fn new(code: &[Instruction]) -> Self {
        let mut sites = Vec::new();
        let indices = code
            .iter()
            .map(|instruction| {
                if matches!(
                    instruction,
                    Instruction::GetField(_) | Instruction::GetField2(_)
                ) {
                    let index = u32::try_from(sites.len()).expect("bytecode site count fits u32");
                    sites.push(PropertyReadCache::default());
                    index
                } else {
                    u32::MAX
                }
            })
            .collect();
        Self {
            indices,
            sites: sites.into_boxed_slice(),
        }
    }
    pub(crate) fn site(&self, pc: usize) -> Option<&PropertyReadCache> {
        self.sites.get(*self.indices.get(pc)? as usize)
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
    fn cached_location_reads_replaced_value_and_second_miss_is_permanent() {
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
        assert!(matches!(cache.state.get(), State::Megamorphic));
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
