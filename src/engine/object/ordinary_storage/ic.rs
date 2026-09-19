//! Promote a location-cache hit without draining runtime cleanup or invoking JS.
use crate::engine::atom::AtomIdx;
use super::{LinkedNativeSelection, linked_field_atom};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::code::runtime::PublishedFunctionSnapshot;
use crate::engine::heap::{ObjectPayload, RawValue, SlotReleaseReadiness};
use crate::engine::value::JsValue;

impl Runtime {
    /// A miss only records a location and leaves the canonical read untouched.
    /// Native classification, when requested, describes this retained result;
    /// it never caches a value or outlives the result's ordinary slot owner.
    pub(crate) fn try_property_ic_read_owned(
        &self,
        base: &JsValue,
        executable: &PublishedFunctionSnapshot,
        pc: usize,
        key_index: u32,
        keep_receiver: bool,
        native: &mut Option<LinkedNativeSelection>,
    ) -> Result<Option<JsValue>, RuntimeError> {
        let Some(atom) = linked_field_atom(self, executable, key_index) else {
            return Ok(None);
        };
        let Some(cache) = executable.property_read_ic.site(pc) else {
            return Ok(None);
        };
        if !keep_receiver && self.0.deferred_references.has_pending() {
            return Ok(None);
        }
        let Ok(mut state) = self.0.state.try_borrow_mut() else {
            return Ok(None);
        };
        if !keep_receiver && state.heap.has_pending_zero_cleanup() {
            return Ok(None);
        }
        let receiver = match base {
            JsValue::Object(object) => *object,
            _ => {
                cache.miss(
                    &state.heap,
                    &state.atoms,
                    self.domain_id(),
                    executable.realm,
                    None,
                    atom,
                );
                return Ok(None);
            }
        };
        // Prove replacement cannot release the last receiver owner BEFORE
        // promoting a result. The proof and retain share this state borrow.
        if !keep_receiver
            && state.heap.slot_object_release_readiness(receiver)? != SlotReleaseReadiness::Ready
        {
            return Ok(None);
        }
        let Some(raw) = cache.read(&state.heap, self.domain_id(), executable.realm, receiver)
        else {
            cache.miss(
                &state.heap,
                &state.atoms,
                self.domain_id(),
                executable.realm,
                Some(receiver),
                atom,
            );
            return Ok(None);
        };
        if matches!(
            raw,
            RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception
        ) {
            return Ok(None);
        }
        let raw = raw.clone();
        let selected = if keep_receiver {
            if let RawValue::Object(function) = &raw {
                state.heap.object(*function).ok().and_then(|object| {
                    let ObjectPayload::NativeFunction { data, .. } = &object.payload else {
                        return None;
                    };
                    let realm = data.realm?;
                    (data.operation().is_some() && state.heap.context(realm).is_ok())
                        .then_some((*function, *data))
                })
            } else {
                None
            }
        } else {
            None
        };
        // Every heap-backed kind retains one new edge; the internal-value
        // conversion below cannot fail after the sentinel exclusion above.
        state.retain_raw_root(&raw)?;
        drop(state);
        let value = JsValue::from_raw(raw).ok_or(RuntimeError::Invariant(
            "internal value sentinel occupied a cached property slot",
        ))?;
        *native = selected.map(|(function, data)| LinkedNativeSelection {
            runtime: self.clone(),
            function,
            data,
        });
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("property_ic.hit");
        Ok(Some(value))
    }

    /// Trusted shared-borrow data-property read.
    ///
    /// Covers the location-cache hit for a live receiver without a mutable
    /// state borrow or fallible plumbing. Symbols need an atom-table retain
    /// (S1b) and every non-data or non-cached case declines with `None`, so the
    /// caller keeps its canonical `try_property_ic_read_owned` fallback. A
    /// declined read claims no owner.
    #[inline]
    pub(crate) fn property_ic_read_fast(
        &self,
        base: &JsValue,
        executable: &PublishedFunctionSnapshot,
        pc: usize,
        key_index: u32,
        keep_receiver: bool,
        native: &mut Option<LinkedNativeSelection>,
    ) -> Option<JsValue> {
        let atom = linked_field_atom(self, executable, key_index)?;
        let cache = executable.property_read_ic.site(pc)?;
        if !keep_receiver && self.0.deferred_references.has_pending() {
            return None;
        }
        let state = self.0.state.try_borrow().ok()?;
        if !keep_receiver && state.heap.has_pending_zero_cleanup() {
            return None;
        }
        let receiver = match base {
            JsValue::Object(object) => *object,
            _ => {
                cache.miss(
                    &state.heap,
                    &state.atoms,
                    self.domain_id(),
                    executable.realm,
                    None,
                    atom,
                );
                return None;
            }
        };
        if !keep_receiver
            && state.heap.slot_object_release_readiness_fast(receiver)
                != SlotReleaseReadiness::Ready
        {
            return None;
        }
        let Some(raw) = cache.read(&state.heap, self.domain_id(), executable.realm, receiver)
        else {
            cache.miss(
                &state.heap,
                &state.atoms,
                self.domain_id(),
                executable.realm,
                Some(receiver),
                atom,
            );
            return None;
        };
        match raw {
            RawValue::Object(function) => {
                let selected = if keep_receiver {
                    let object = state.heap.object_fast(*function);
                    match &object.payload {
                        ObjectPayload::NativeFunction { data, .. } => {
                            data.realm.and_then(|realm| {
                                (data.operation().is_some() && state.heap.context(realm).is_ok())
                                    .then_some((*function, *data))
                            })
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                state.heap.retain_object_fast(*function);
                *native = selected.map(|(function, data)| LinkedNativeSelection {
                    runtime: self.clone(),
                    function,
                    data,
                });
                Some(JsValue::Object(*function))
            }
            RawValue::String(id) => {
                state.heap.retain_string_shared(*id).ok()?;
                Some(JsValue::String(*id))
            }
            RawValue::BigInt(id) => {
                state.heap.retain_bigint_shared(*id).ok()?;
                Some(JsValue::BigInt(*id))
            }
            RawValue::Symbol(index) => {
                state.atoms.retain_index_shared(*index).ok()?;
                Some(JsValue::Symbol(*index))
            }
            RawValue::Undefined => Some(JsValue::Undefined),
            RawValue::Null => Some(JsValue::Null),
            RawValue::Bool(value) => Some(JsValue::Bool(*value)),
            RawValue::Int(value) => Some(JsValue::Int(*value)),
            RawValue::Float(value) => Some(JsValue::Float(*value)),
            RawValue::Private(_) | RawValue::Uninitialized | RawValue::Exception => None,
        }
    }
}

impl Runtime {
    pub(crate) fn try_property_ic_write_owned(
        &self,
        base: &JsValue,
        executable: &PublishedFunctionSnapshot,
        pc: usize,
        key: u32,
        value: &JsValue,
    ) -> Result<bool, RuntimeError> {
        let Some(atom) = linked_field_atom(self, executable, key) else {
            return Ok(false);
        };
        let JsValue::Object(object) = base else {
            return Ok(false);
        };
        let Some(cache) = executable.property_read_ic.write_site(pc) else {
            return Ok(false);
        };
        // The borrowed value already carries its edges; the stored copy is
        // retained transactionally below, so no producer edge is created.
        let raw = value.as_raw();
        let mut state = self.0.state.borrow_mut();
        let id = *object;
        let slot = match cache.slot(&state.heap, self.domain_id(), executable.realm, id) {
            Some(slot) => slot,
            None => {
                cache.miss(
                    &state.heap,
                    &state.atoms,
                    self.domain_id(),
                    executable.realm,
                    id,
                    atom,
                );
                let Some(slot) = cache.slot(&state.heap, self.domain_id(), executable.realm, id)
                else {
                    drop(state);
                    self.release_converted_value_edge(&conversion_probe);
                    return Ok(false);
                };
                slot
            }
        };
        // Input owners remain rooted; retain the new value before releasing the
        // old edge. The caller has ended RunSlots and published the current PC.
        let replaced =
            state.replace_property_slot(id, slot, crate::engine::heap::PropertySlot::Data(raw));
        drop(state);
        // The slot retained its own copy edge on success; a rejected
        // replacement kept nothing. Balance the producer edge either way.
        self.release_converted_value_edge(&conversion_probe);
        replaced?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("property_write_ic.hit");
        Ok(true)
    }

    pub(crate) fn try_dense_array_write_owned(
        &self,
        base: &JsValue,
        index: u32,
        value: &JsValue,
    ) -> Result<bool, RuntimeError> {
        let JsValue::Object(object) = base else {
            return Ok(false);
        };
        // The borrowed value already carries its edges; the stored copy is
        // retained transactionally below, so no producer edge is created.
        let raw = value.as_raw();
        let mut state = self.0.state.borrow_mut();
        let data = match state.heap.object(*object) {
            Ok(data) => data,
            Err(error) => {
                drop(state);
                self.release_converted_value_edge(&conversion_probe);
                return Err(error.into());
            }
        };
        if !matches!(data.kind, crate::engine::heap::ObjectKind::Array)
            || data.dense_array_value(index).is_none()
        {
            return Ok(false);
        }
        let atoms = match state.retain_raw_value_atoms([&raw]) {
            Ok(atoms) => atoms,
            Err(error) => {
                drop(state);
                self.release_converted_value_edge(&conversion_probe);
                return Err(error);
            }
        };
        let appended = state
            .heap
            .replace_array_dense_value(*object, index, raw);
        match appended {
            Ok(cleanup) => {
                state.apply_cleanup(cleanup)?;
            }
            Err(error) => {
                let released = state.release_atoms(atoms);
                released?;
                return Err(error.into());
            }
        }
        Ok(true)
    }

    pub(crate) fn try_define_field_owned(
        &self,
        base: &JsValue,
        executable: &PublishedFunctionSnapshot,
        key: u32,
        value: &JsValue,
    ) -> Result<bool, RuntimeError> {
        let Some(atom) = linked_field_atom(self, executable, key) else {
            return Ok(false);
        };
        let JsValue::Object(object) = base else {
            return Ok(false);
        };
        // The borrowed value already carries its edges; the stored copy is
        // retained transactionally below, so no producer edge is created.
        let raw = value.as_raw();
        let mut state = self.0.state.borrow_mut();
        let data = match state.heap.object(*object) {
            Ok(data) => data,
            Err(error) => {
                drop(state);
                self.release_converted_value_edge(&conversion_probe);
                return Err(error.into());
            }
        };
        let shape_has_atom = state
            .heap
            .shape(data.shape)
            .map(|shape| shape.find(AtomIdx::from_raw(atom.raw())).is_some())?;
        if !super::is_ordinary(data) || !data.extensible || shape_has_atom {
            return Ok(false);
        }
        state.store_selected_property_slot(
            *object,
            atom,
            crate::engine::object::shape::PropertyFlags::data(true, true, true),
            crate::engine::heap::PropertySlot::Data(raw),
            None,
        )?;
        Ok(true)
    }

    pub(crate) fn try_delete_own_data(
        &self,
        base: &JsValue,
        key: &crate::engine::object::PropertyKey,
    ) -> Result<Option<bool>, RuntimeError> {
        let JsValue::Object(object) = base else {
            return Ok(None);
        };
        {
            let state = self.0.state.borrow();
            let data = state.heap.object(*object)?;
            if !super::is_ordinary(data) {
                return Ok(None);
            }
            let shape = state.heap.shape(data.shape)?;
            let Some(slot) = shape.find(AtomIdx::from_raw(key.atom().raw())) else {
                return Ok(Some(true));
            };
            if !shape.entries()[slot as usize].flags.configurable
                || !matches!(
                    data.slots[slot as usize],
                    crate::engine::heap::PropertySlot::Data(_)
                )
            {
                return Ok(None);
            }
        }
        let object = crate::engine::object::ObjectRef::from_borrowed_handle(self.clone(), *object)?;
        self.delete_property(&object, key).map(Some)
    }
}

impl Runtime {
    pub(crate) fn try_dense_array_kept_read(&self, base: &JsValue, index: u32) -> Option<JsValue> {
        let JsValue::Object(object) = base else {
            return None;
        };
        let state = self.0.state.borrow();
        let data = state.heap.object(*object).ok()?;
        if !matches!(data.kind, crate::engine::heap::ObjectKind::Array) {
            return None;
        }
        super::immediate_value_jsvalue(data.dense_array_value(index)?)
    }
}

impl Runtime {
    pub(crate) fn try_property_ic_write_scalar(
        &self,
        base: &JsValue,
        executable: &PublishedFunctionSnapshot,
        pc: usize,
        key: u32,
        value: &JsValue,
    ) -> Result<bool, RuntimeError> {
        if !matches!(
            value,
            JsValue::Undefined | JsValue::Null | JsValue::Bool(_) | JsValue::Int(_) | JsValue::Float(_)
        ) || self.slot_value_release_readiness_jsvalue(base)? != SlotReleaseReadiness::Ready
        {
            return Ok(false);
        }
        let Some(atom) = linked_field_atom(self, executable, key) else {
            return Ok(false);
        };
        let JsValue::Object(object) = base else {
            return Ok(false);
        };
        let Some(cache) = executable.property_read_ic.write_site(pc) else {
            return Ok(false);
        };
        let mut state = self.0.state.borrow_mut();
        let id = *object;
        let slot = match cache.slot(&state.heap, self.domain_id(), executable.realm, id) {
            Some(slot) => slot,
            None => {
                cache.miss(
                    &state.heap,
                    &state.atoms,
                    self.domain_id(),
                    executable.realm,
                    id,
                    atom,
                );
                let Some(slot) = cache.slot(&state.heap, self.domain_id(), executable.realm, id)
                else {
                    return Ok(false);
                };
                slot
            }
        };
        let crate::engine::heap::PropertySlot::Data(old) = &state.heap.object(id)?.slots[slot]
        else {
            return Ok(false);
        };
        if super::immediate_value(old).is_none() {
            return Ok(false);
        }
        // `value` was matched to a scalar above, so this id copy allocates
        // nothing and never takes the state borrow the caller still holds.
        let raw = value.as_raw();
        state.replace_property_slot(id, slot, crate::engine::heap::PropertySlot::Data(raw))?;
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_owned_execution_event("property_write_ic.hit");
        Ok(true)
    }
}

impl Runtime {
    pub(crate) fn try_dense_array_write_scalar(
        &self,
        base: &JsValue,
        index: u32,
        value: &JsValue,
    ) -> Result<bool, RuntimeError> {
        if !matches!(
            value,
            JsValue::Undefined | JsValue::Null | JsValue::Bool(_) | JsValue::Int(_) | JsValue::Float(_)
        ) || self.slot_value_release_readiness_jsvalue(base)? != SlotReleaseReadiness::Ready
        {
            return Ok(false);
        }
        if self.try_dense_array_kept_read(base, index).is_none() {
            return Ok(false);
        }
        self.try_dense_array_write_owned(base, index, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::code::bytecode::Instruction;

    fn object(value: &Value) -> &crate::engine::object::ObjectRef {
        let Value::Object(object) = value else {
            panic!("object")
        };
        object
    }

    fn site(runtime: &Runtime) -> (PublishedFunctionSnapshot, usize, u32) {
        let mut context = runtime.new_context();
        let callable = runtime
            .callable_from_value(context.eval("(function(o){return o.x})").unwrap())
            .unwrap();
        let crate::engine::vm::call::CallableExecution::Bytecode { bytecode, .. } =
            runtime.bytecode_for_callable(&callable).unwrap()
        else {
            panic!("bytecode")
        };
        let executable = runtime.snapshot_function_bytecode(&bytecode).unwrap();
        let (pc, key) = executable
            .code
            .iter()
            .enumerate()
            .find_map(|(pc, op)| match op {
                Instruction::GetField(key) => Some((pc, *key)),
                _ => None,
            })
            .unwrap();
        (executable, pc, key)
    }

    #[test]
    fn owned_ic_promotes_every_public_value_and_reads_current_slot() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        for expression in [
            "undefined",
            "null",
            "true",
            "123",
            "1.25",
            "'wide λ text'",
            "123456789012345678901234567890n",
            "Symbol('ic')",
            "({nested:7})",
        ] {
            let (code, pc, key) = site(&runtime);
            let base=context.eval(&format!("globalThis.icExpected={expression};globalThis.icHolder={{x:icExpected}};icHolder")).unwrap();
            let expected = context.eval("icExpected").unwrap();
            let mut native = None;
            assert!(
                runtime
                    .try_property_ic_read_owned(&base, &code, pc, key, false, &mut native)
                    .unwrap()
                    .is_none()
            );
            let actual = runtime
                .try_property_ic_read_owned(&base, &code, pc, key, false, &mut native)
                .unwrap()
                .unwrap();
            assert_eq!(actual, expected, "{expression}");
            context.eval("icHolder.x=99").unwrap();
            assert_eq!(
                runtime
                    .try_property_ic_read_owned(&base, &code, pc, key, false, &mut native)
                    .unwrap(),
                Some(Value::Int(99))
            );
            assert!(native.is_none());
        }
    }

    #[test]
    fn owned_ic_guards_borrow_deferred_work_and_final_receiver_before_promotion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (code, pc, key) = site(&runtime);
        let base = context.eval("({x:{marker:1}})").unwrap();
        let mut native = None;
        assert!(
            runtime
                .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
                .unwrap()
                .is_none()
        );
        // A cache hit must still leave the last receiver owner untouched.
        assert!(
            runtime
                .try_property_ic_read_owned(&base, &code, pc, key, false, &mut native)
                .unwrap()
                .is_none()
        );
        let receiver = object(&base);
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object_strong_count(receiver.object_id())
                .unwrap(),
            1
        );
        {
            let _borrow = runtime.0.state.borrow();
            assert!(
                runtime
                    .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
                    .unwrap()
                    .is_none()
            );
        }
        let released = runtime.new_object(None).unwrap();
        {
            let _borrow = runtime.0.state.borrow();
            drop(released);
        }
        assert!(runtime.0.deferred_references.has_pending());
        // A kept receiver hit only retains under the exclusive heap borrow;
        // pending unrelated releases cannot mutate its guarded layout.
        let retained_hit = runtime
            .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
            .unwrap();
        assert!(matches!(retained_hit, Some(Value::Object(_))));
        assert!(runtime.0.deferred_references.has_pending());
        runtime.drain_deferred_references().unwrap();
        assert!(matches!(
            runtime
                .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
                .unwrap(),
            Some(Value::Object(_))
        ));
    }

    #[test]
    fn owned_ic_native_hint_is_bound_to_current_retained_function() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let (code, pc, key) = site(&runtime);
        let base = context
            .eval("globalThis.icNative={x:Math.min};icNative")
            .unwrap();
        let mut native = None;
        assert!(
            runtime
                .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
                .unwrap()
                .is_none()
        );
        let first = runtime
            .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
            .unwrap()
            .unwrap();
        let hint = native.take().unwrap();
        context.eval("icNative.x=Math.max").unwrap();
        let data = hint.into_parts(object(&first)).unwrap();
        assert_eq!(
            data.target,
            crate::engine::builtins::native::NativeFunctionId::MathMinMax(
                crate::engine::builtins::native::MathMinMaxKind::Min
            )
        );
        let second = runtime
            .try_property_ic_read_owned(&base, &code, pc, key, true, &mut native)
            .unwrap()
            .unwrap();
        let hint = native.take().unwrap();
        assert!(hint.into_parts(object(&first)).is_none());
        assert_ne!(first, second);
    }
}
