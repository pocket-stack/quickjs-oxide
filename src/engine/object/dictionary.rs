//! Dictionary transitions stay at the runtime boundary so atom ownership and
//! weak shape-cache entries remain consistent with heap layout transactions.

use super::shape::{Shape, ShapeEntry};
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::runtime::RuntimeState;
use crate::engine::heap::{ObjectId, PropertySlot, ShapeId};
use std::collections::HashMap;

impl RuntimeState {
    pub(crate) fn ensure_dictionary_layout(
        &mut self,
        object: ObjectId,
    ) -> Result<(), RuntimeError> {
        let shape_id = self.heap.object(object)?.shape;
        if self.heap.shape_strong_count(shape_id)? == 1 {
            self.heap.enable_object_dictionary(object)?;
            // Metadata conversion preserves entries; unlink before the first
            // mutation changes the fingerprint of this exclusively owned shape.
            self.unlink_finalized_shapes([shape_id]);
            return Ok(());
        }
        let mut shape = self.heap.shape(shape_id)?.clone();
        shape.enable_dictionary();
        let slots = self.heap.object(object)?.slots.clone();
        let shape_id = self.allocate_uncached_shape(shape)?;
        self.replace_layout_with_owned_shape(object, shape_id, slots)
    }

    fn allocate_uncached_shape(&mut self, shape: Shape) -> Result<ShapeId, RuntimeError> {
        let atoms = self.retain_shape_atoms(shape.entries())?;
        match self.heap.allocate_shape(shape) {
            Ok(id) => Ok(id),
            Err(error) => {
                self.release_atoms(atoms)?;
                Err(error.into())
            }
        }
    }

    /// Rare whole-layout operations supply physical entries and parallel slots.
    /// Restore semantic insertion order before rebuilding, preserving dictionary
    /// mode through prototype/private-element changes and shared-shape detaches.
    pub(crate) fn replace_dictionary_layout(
        &mut self,
        object: ObjectId,
        prototype: Option<ObjectId>,
        entries: &[ShapeEntry],
        slots: Vec<PropertySlot>,
    ) -> Result<(), RuntimeError> {
        if entries.len() != slots.len() {
            return Err(RuntimeError::Invariant(
                "dictionary replacement has mismatched entries and slots",
            ));
        }
        let mut incoming = HashMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            if incoming.insert(entry.atom, index).is_some() {
                return Err(super::shape::ShapeError::DuplicateAtom(entry.atom).into());
            }
        }
        let mut pairs = entries
            .iter()
            .copied()
            .zip(slots)
            .map(Some)
            .collect::<Vec<_>>();
        let mut ordered = Vec::with_capacity(pairs.len());
        let shape = self.heap.shape(self.heap.object(object)?.shape)?;
        for index in shape.ordered_indices() {
            if let Some(&replacement) = incoming.get(&shape.entries()[index].atom) {
                if let Some(pair) = pairs[replacement].take() {
                    ordered.push(pair);
                }
            }
        }
        ordered.extend(pairs.into_iter().flatten());
        let (entries, slots): (Vec<_>, Vec<_>) = ordered.into_iter().unzip();
        let mut shape = Shape::new(prototype, entries)?;
        shape.enable_dictionary();
        let shape_id = self.allocate_uncached_shape(shape)?;
        self.replace_layout_with_owned_shape(object, shape_id, slots)
    }
}

#[cfg(test)]
mod tests {
    use crate::engine::api::runtime::Runtime;
    use crate::engine::object::ObjectRef;
    use crate::engine::value::Value;

    fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
        runtime
            .own_property_keys(object)
            .unwrap()
            .iter()
            .map(|key| {
                runtime
                    .property_key_to_js_string(key)
                    .unwrap()
                    .to_utf8_lossy()
            })
            .collect()
    }
    #[test]
    fn wide_object_deletion_reuses_its_unique_layout_and_preserves_key_order() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(object) = context
        .eval("(() => { const o = {}; for (let i = 0; i < 32; i++) o['p' + i] = i; return o; })()")
        .unwrap()
    else {
        panic!("object fixture");
    };
        let original_shape = runtime
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())
            .unwrap()
            .shape;
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .shape_strong_count(original_shape)
                .unwrap(),
            1
        );
        let key = runtime.intern_property_key("p7").unwrap();
        assert!(runtime.delete_property(&object, &key).unwrap());
        let current_shape = runtime
            .0
            .state
            .borrow()
            .heap
            .object(object.object_id())
            .unwrap()
            .shape;
        assert_eq!(
            original_shape, current_shape,
            "deletion must not publish a complete replacement layout"
        );
        assert_eq!(
            own_key_names(&runtime, &object),
            (0..32)
                .filter(|i| *i != 7)
                .map(|i| format!("p{i}"))
                .collect::<Vec<_>>()
        );
        context.set_property(&object, &key, Value::Int(77)).unwrap();
        let mut expected = (0..32)
            .filter(|i| *i != 7)
            .map(|i| format!("p{i}"))
            .collect::<Vec<_>>();
        expected.push("p7".to_owned());
        assert_eq!(own_key_names(&runtime, &object), expected);
    }

    #[test]
    fn dictionary_deletion_detaches_shared_shapes_and_releases_removed_edges() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(first) = context
            .eval("(() => { const o = {}; for (let i=0;i<32;i++) o['p'+i]=i; return o; })()")
            .unwrap()
        else {
            panic!("object fixture");
        };
        let (shared_shape, second) = {
            let mut state = runtime.0.state.borrow_mut();
            let data = state.heap.object(first.object_id()).unwrap();
            let shape = data.shape;
            let slots = data.slots.clone();
            let second = state
                .heap
                .allocate_object(crate::engine::heap::ObjectData::ordinary(shape, slots))
                .unwrap();
            (shape, second)
        };
        let second = ObjectRef::from_owned_handle(runtime.clone(), second);
        let key = runtime.intern_property_key("p7").unwrap();
        assert!(runtime.delete_property(&first, &key).unwrap());
        assert_eq!(context.get_property(&second, &key).unwrap(), Value::Int(7));
        let detached = runtime
            .0
            .state
            .borrow()
            .heap
            .object(first.object_id())
            .unwrap()
            .shape;
        assert_ne!(detached, shared_shape);
        let child = context.new_object().unwrap();
        let child_id = child.object_id();
        context
            .set_property(&first, &key, Value::Object(child))
            .unwrap();
        assert!(runtime.delete_property(&first, &key).unwrap());
        let state = runtime.0.state.borrow();
        assert!(
            state.heap.object(child_id).is_err(),
            "removed slot must release its only object edge"
        );
        assert_eq!(
            state.heap.object(first.object_id()).unwrap().shape,
            detached
        );
        assert_eq!(
            state.heap.object(second.object_id()).unwrap().shape,
            shared_shape
        );
    }

    #[test]
    fn dictionary_order_survives_descriptors_prototype_changes_and_churn() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(r#"(() => {
        const o = {}, a = Symbol('a'), b = Symbol('b');
        for (let i=0;i<32;i++) o['p'+i]=i;
        o[a]=1; o[b]=2;
        delete o.p7; delete o.p0; delete o[a]; o.p7=77; o[a]=3;
        Object.defineProperty(o, 'p12', {get() { return 120; }, configurable:true, enumerable:true});
        Object.setPrototypeOf(o, {inherited:42});
        const keys = Reflect.ownKeys(o);
        if (keys.slice(-3)[0] !== 'p7' || keys.at(-2) !== b || keys.at(-1) !== a) return false;
        if (o.p12 !== 120 || o.inherited !== 42) return false;
        const expected = keys.filter(k => k !== 'p1');
        for (let i=0;i<1024;i++) { delete o.p1; o.p1=i; }
        expected.push('p1');
        // Strings precede symbols even after a later string insertion.
        const strings = expected.filter(k => typeof k === 'string');
        if (Object.keys(o).join(',') !== strings.join(',')) return false;
        Object.freeze(o);
        return Reflect.ownKeys(o).length === keys.length && Object.isFrozen(o)
            && !Reflect.deleteProperty(o, 'p12');
    })()"#).unwrap(), Value::Bool(true));
    }

    #[test]
    fn holey_dictionary_keeps_prototype_setters_and_partial_length_failure() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(() => {
        const a = [], symbol = Symbol();
        for (let i=0;i<64;i++) a.push(i);
        a.tag = 1; a[symbol] = 2; delete a[7];
        let seen = 0;
        const prototype = Object.create(Array.prototype);
        Object.defineProperty(prototype, '7', {set(value) {seen=value;}, get() {return 99;}});
        Object.setPrototypeOf(a, prototype);
        a[7] = 42;
        if (seen !== 42 || a[7] !== 99 || Object.hasOwn(a, '7')) return false;
        Object.defineProperty(a, '7', {value:7,writable:true,enumerable:true,configurable:true});
        Object.defineProperty(a, '30', {configurable:false});
        let threw = false;
        try { Object.defineProperty(a, 'length', {value:8,writable:false}); }
        catch (error) { threw = error instanceof TypeError; }
        return threw && a.length === 31 && a[30] === 30 && !Object.hasOwn(a, '31')
            && !Object.getOwnPropertyDescriptor(a, 'length').writable
            && a.tag === 1 && a[symbol] === 2;
    })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }

    #[test]
    fn holey_array_churn_reuses_the_layout_after_its_initial_conversion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(array) = context
            .eval(
                "(() => { const a=[]; for(let i=0;i<32;i++) a.push(i); delete a[7]; return a; })()",
            )
            .unwrap()
        else {
            panic!("array fixture");
        };
        let shape = runtime
            .0
            .state
            .borrow()
            .heap
            .object(array.object_id())
            .unwrap()
            .shape;
        let key = runtime.property_key_for_index(9).unwrap();
        assert!(runtime.delete_property(&array, &key).unwrap());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(array.object_id())
                .unwrap()
                .shape,
            shape,
            "a second interior deletion must not rebuild the entire Array layout"
        );
        context.set_property(&array, &key, Value::Int(99)).unwrap();
        assert_eq!(context.get_property(&array, &key).unwrap(), Value::Int(99));
        assert_eq!(
            runtime.array_fast_len(&array).unwrap(),
            None,
            "QuickJS representation-sensitive algorithms still observe slow form"
        );
    }
}
