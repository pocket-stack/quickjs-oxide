use super::*;

impl Heap {
    /// Read one live object record.
    pub fn object(&self, id: ObjectId) -> Result<&ObjectData, HeapError> {
        match self.live_node(RawId::Object(id))?.data {
            NodeData::Object(ref object) => Ok(object),
            NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed object lookup reached another node payload",
            )),
        }
    }

    /// Set QuickJS's identity-local Annex B `is_HTMLDDA` bit.
    #[cfg(feature = "test262-host")]
    pub(crate) fn set_object_is_html_dda(&mut self, id: ObjectId) -> Result<(), HeapError> {
        self.object_mut(id)?.is_html_dda = true;
        Ok(())
    }

    /// Read the private-method brand owned by an object's HomeObject slot.
    ///
    /// The atom is an internal identity rather than an ECMAScript property.
    /// Its ownership remains with the object until finalization.
    pub fn object_private_brand_home(&self, id: ObjectId) -> Result<Option<Atom>, HeapError> {
        Ok(self.object(id)?.private_brand_home)
    }

    /// Attach the freshly allocated private-method brand for one class side.
    ///
    /// The caller transfers one owned atom reference on success. A HomeObject
    /// has exactly one brand even when the class declares several methods.
    pub fn attach_object_private_brand_home(
        &mut self,
        id: ObjectId,
        brand: Atom,
    ) -> Result<(), HeapError> {
        let object = self.object_mut(id)?;
        if object.private_brand_home.is_some() {
            return Err(HeapError::Invariant(
                "private-method HomeObject already has a brand",
            ));
        }
        object.private_brand_home = Some(brand);
        Ok(())
    }

    /// Read the optional HomeObject edge of one bytecode function.
    ///
    /// Native, bound, and ordinary objects are rejected rather than silently
    /// impersonating bytecode functions at the super-resolution boundary.
    pub fn bytecode_function_home_object(
        &self,
        id: ObjectId,
    ) -> Result<Option<ObjectId>, HeapError> {
        let ObjectPayload::BytecodeFunction { home_object, .. } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "HomeObject lookup reached a non-bytecode function",
            ));
        };
        Ok(*home_object)
    }

    /// Read the hidden public-instance-field initializer attached to one class
    /// constructor bytecode function.
    pub fn bytecode_class_instance_initializer(
        &self,
        id: ObjectId,
    ) -> Result<Option<ObjectId>, HeapError> {
        let ObjectPayload::BytecodeFunction {
            class_instance_initializer,
            ..
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "class initializer lookup reached a non-bytecode function",
            ));
        };
        Ok(*class_instance_initializer)
    }

    /// Read one live shape record.
    pub fn shape(&self, id: ShapeId) -> Result<&Shape, HeapError> {
        match self.live_node(RawId::Shape(id))?.data {
            NodeData::Shape(ref shape) => Ok(shape),
            NodeData::Object(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed shape lookup reached another node payload",
            )),
        }
    }

    pub(in crate::engine::heap) fn shape_mut(
        &mut self,
        id: ShapeId,
    ) -> Result<&mut Shape, HeapError> {
        match self.live_node_mut(RawId::Shape(id))?.data {
            NodeData::Shape(ref mut shape) => Ok(shape),
            NodeData::Object(_)
            | NodeData::VarRef(_)
            | NodeData::Context(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed mutable shape lookup reached another node payload",
            )),
        }
    }

    /// Read one live context record.
    pub fn context(&self, id: ContextId) -> Result<&ContextData, HeapError> {
        match self.live_node(RawId::Context(id))?.data {
            NodeData::Context(ref context) => Ok(context),
            NodeData::Object(_)
            | NodeData::Shape(_)
            | NodeData::VarRef(_)
            | NodeData::FunctionBytecode(_) => Err(HeapError::Invariant(
                "typed context lookup reached another node payload",
            )),
        }
    }

    /// Seed the realm-local xorshift64* stream used by `Math.random`.
    /// QuickJS replaces an all-zero time seed with one because zero is the
    /// generator's absorbing state.
    pub(crate) fn initialize_math_random_state(
        &mut self,
        id: ContextId,
        seed: u64,
    ) -> Result<(), HeapError> {
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(id))?.data else {
            return Err(HeapError::Invariant(
                "typed context lookup reached another node payload",
            ));
        };
        if context.math_random_state != 0 {
            return Err(HeapError::Invariant(
                "Math.random state was initialized more than once",
            ));
        }
        context.math_random_state = if seed == 0 { 1 } else { seed };
        Ok(())
    }

    /// Advance the pinned QuickJS xorshift64* stream for one realm.
    pub(crate) fn next_math_random_u64(&mut self, id: ContextId) -> Result<u64, HeapError> {
        let NodeData::Context(context) = &mut self.live_node_mut(RawId::Context(id))?.data else {
            return Err(HeapError::Invariant(
                "typed context lookup reached another node payload",
            ));
        };
        if context.math_random_state == 0 {
            return Err(HeapError::Invariant(
                "Math.random state was used before initialization",
            ));
        }
        let mut state = context.math_random_state;
        state ^= state >> 12;
        state ^= state << 25;
        state ^= state >> 27;
        context.math_random_state = state;
        Ok(state.wrapping_mul(0x2545_f491_4f6c_dd1d))
    }

    /// Clone one native function's typed hidden capture.  Returned raw values
    /// and identities are borrowed snapshots; callers that keep them across a
    /// heap mutation must first promote or otherwise retain their edges.
    pub(crate) fn native_internal_callable(
        &self,
        id: ObjectId,
    ) -> Result<Option<InternalCallableData>, HeapError> {
        let ObjectPayload::NativeFunction { internal, .. } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "internal callable lookup reached a non-native function",
            ));
        };
        Ok(internal.clone())
    }

    /// Borrow a complete snapshot of one genuine Proxy's hidden state.
    ///
    /// The target and handler identities in the result are borrowed: callers
    /// that keep them across a heap mutation must promote them to owned roots.
    pub(crate) fn proxy_snapshot(&self, id: ObjectId) -> Result<ProxyData, HeapError> {
        let ObjectPayload::Proxy(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Proxy snapshot reached an object with the wrong class",
            ));
        };
        Ok(*data)
    }

    /// Consume a `Proxy.revocable` closure's one owned Proxy capture and revoke
    /// that Proxy as one heap mutation.
    ///
    /// A second call is a no-op and returns `false`. On the first call this
    /// clears only the closure edge, matching QuickJS's one-shot
    /// `func_data[0] = JS_NULL`; the Proxy continues to retain both its target
    /// and handler after `is_revoked` is set. Keeping the mutation atomic also
    /// prevents the captured Proxy from being finalized between clearing the
    /// closure and marking its payload revoked.
    pub(crate) fn revoke_proxy_from_callable(
        &mut self,
        callable: ObjectId,
    ) -> Result<(bool, HeapCleanup), HeapError> {
        let proxy = match &self.object(callable)?.payload {
            ObjectPayload::NativeFunction {
                data:
                    NativeFunctionData {
                        target: NativeFunctionId::ProxyRevoke,
                        ..
                    },
                internal: Some(InternalCallableData::ProxyRevoke { proxy }),
            } => *proxy,
            _ => {
                return Err(HeapError::Invariant(
                    "Proxy revocation reached the wrong native function",
                ));
            }
        };
        let Some(proxy) = proxy else {
            return Ok((false, HeapCleanup::default()));
        };
        if !matches!(&self.object(proxy)?.payload, ObjectPayload::Proxy(_)) {
            return Err(HeapError::Invariant(
                "Proxy revocation closure retained an object with the wrong class",
            ));
        }

        let ObjectPayload::Proxy(data) = &mut self.object_mut(proxy)?.payload else {
            unreachable!("Proxy revocation capture was validated before mutation")
        };
        data.is_revoked = true;

        let ObjectPayload::NativeFunction {
            internal: Some(InternalCallableData::ProxyRevoke { proxy: capture }),
            ..
        } = &mut self.object_mut(callable)?.payload
        else {
            unreachable!("Proxy revocation callable was validated before mutation")
        };
        *capture = None;

        self.release_raw_no_drain(RawId::Object(proxy))?;
        Ok((true, self.drain_zero_queue()?))
    }

    /// Read the internal millisecond time value of one genuine Date object.
    pub fn date_value(&self, id: ObjectId) -> Result<f64, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Date(value) => Ok(*value),
            _ => Err(HeapError::Invariant(
                "Date value requested for an object with the wrong class",
            )),
        }
    }

    /// Replace the internal millisecond time value of one genuine Date.
    /// This payload owns no arena or atom edges, so mutation is infallible
    /// after the branded object identity has been validated.
    pub fn set_date_value(&mut self, id: ObjectId, value: f64) -> Result<(), HeapError> {
        let ObjectPayload::Date(current) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Date value update reached an object with the wrong class",
            ));
        };
        *current = value;
        Ok(())
    }

    /// Read the typed internal state of one genuine RegExp object.
    #[cfg(test)]
    pub fn regexp_data(&self, id: ObjectId) -> Result<&RegExpObjectData, HeapError> {
        let ObjectPayload::RegExp(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "RegExp data requested for an object with the wrong class",
            ));
        };
        Ok(data)
    }

    /// Replace one genuine RegExp object's source/program state.
    ///
    /// Both variants are reference-counted leaves without arena or atom
    /// edges, so mutation needs no retain/release transaction in this heap.
    pub fn replace_regexp_data(
        &mut self,
        id: ObjectId,
        replacement: RegExpObjectData,
    ) -> Result<RegExpObjectData, HeapError> {
        let ObjectPayload::RegExp(current) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "RegExp data update reached an object with the wrong class",
            ));
        };
        Ok(std::mem::replace(current, replacement))
    }

    /// Read QuickJS's representation-sensitive dense count for a genuine
    /// Array. `None` means the Array has converted to slow properties.
    pub fn array_dense_len(&self, id: ObjectId) -> Result<Option<u32>, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Array { dense: Some(dense) } => {
                Ok(Some(u32::try_from(dense.len()).map_err(|_| {
                    HeapError::Invariant("fast Array count exceeded Uint32")
                })?))
            }
            ObjectPayload::Array { dense: None } => Ok(None),
            _ => Err(HeapError::Invariant(
                "Array dense state requested for an object with the wrong class",
            )),
        }
    }

    /// Append one consecutive C/W/E element to a fast Array. Object edges are
    /// retained before publication. A Symbol atom must already be owned by the
    /// caller and transfers to the Array only when this operation succeeds.
    pub fn append_array_dense_value(
        &mut self,
        id: ObjectId,
        value: RawValue,
    ) -> Result<(), HeapError> {
        if !is_map_storable_value(&value) {
            return Err(HeapError::Invariant(
                "fast Array contains an internal value sentinel",
            ));
        }
        {
            let ObjectPayload::Array { dense: Some(dense) } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "dense append reached a slow Array or an object with the wrong class",
                ));
            };
            if dense.len() >= u32::MAX as usize {
                return Err(HeapError::Overflow {
                    operation: "growing fast Array count",
                });
            }
            dense.try_reserve(1).map_err(|_| HeapError::Allocation {
                operation: "growing fast Array storage",
            })?;
        }
        self.retain_edges_transactionally(&raw_value_edges(&value))?;
        let ObjectPayload::Array { dense: Some(dense) } = &mut self.object_mut(id)?.payload else {
            unreachable!("fast Array changed representation while retaining its new value")
        };
        dense.push(value);
        Ok(())
    }

    /// Append to a newly constructed Array whose logical length still equals
    /// its dense count. This is QuickJS's `add_fast_array_element` substrate
    /// for literals, builtin result arrays, and JSON parsing: allocation and
    /// edge retention complete before the infallible length-slot publication.
    pub fn append_fresh_array_dense_value(
        &mut self,
        id: ObjectId,
        value: RawValue,
    ) -> Result<(), HeapError> {
        let next_len = {
            let object = self.object(id)?;
            let ObjectPayload::Array { dense: Some(dense) } = &object.payload else {
                return Err(HeapError::Invariant(
                    "fresh dense append reached a slow Array or wrong object class",
                ));
            };
            let dense_len = u32::try_from(dense.len())
                .map_err(|_| HeapError::Invariant("fast Array count exceeded Uint32"))?;
            let shape = self.shape(object.shape)?;
            let length = shape.entries().first().ok_or(HeapError::Invariant(
                "fresh Array has no physical length entry",
            ))?;
            let stored_len = match object.slots.first() {
                Some(PropertySlot::Data(RawValue::Int(length))) if *length >= 0 => *length as u32,
                Some(PropertySlot::Data(RawValue::Float(length)))
                    if length.is_finite()
                        && *length >= 0.0
                        && *length <= f64::from(u32::MAX)
                        && length.fract() == 0.0 =>
                {
                    *length as u32
                }
                _ => {
                    return Err(HeapError::Invariant(
                        "fresh Array length is not an exact Uint32 data value",
                    ));
                }
            };
            if !length.flags.writable || stored_len != dense_len {
                return Err(HeapError::Invariant(
                    "fresh Array length diverged from its dense count",
                ));
            }
            dense_len.checked_add(1).ok_or(HeapError::Overflow {
                operation: "growing fresh Array length",
            })?
        };

        self.append_array_dense_value(id, value)?;
        let replacement = if let Ok(length) = i32::try_from(next_len) {
            RawValue::Int(length)
        } else {
            RawValue::Float(f64::from(next_len))
        };
        let object = self
            .object_mut(id)
            .expect("fresh Array disappeared after retaining its appended value");
        let Some(PropertySlot::Data(length)) = object.slots.first_mut() else {
            unreachable!("fresh Array length slot changed after preflight")
        };
        *length = replacement;
        Ok(())
    }

    /// Replace one existing fast element transactionally. New edges are
    /// retained before the previous value and its Symbol atom are detached.
    pub fn replace_array_dense_value(
        &mut self,
        id: ObjectId,
        index: u32,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&replacement) {
            return Err(HeapError::Invariant(
                "fast Array contains an internal value sentinel",
            ));
        }
        let index = index as usize;
        match &self.object(id)?.payload {
            ObjectPayload::Array { dense: Some(dense) } if index < dense.len() => {}
            ObjectPayload::Array { dense: Some(_) } => {
                return Err(HeapError::Invariant(
                    "fast Array replacement index is outside its dense prefix",
                ));
            }
            ObjectPayload::Array { dense: None } => {
                return Err(HeapError::Invariant(
                    "fast Array replacement reached a slow Array",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "fast Array replacement reached an object with the wrong class",
                ));
            }
        }
        self.retain_edges_transactionally(&raw_value_edges(&replacement))?;
        let previous = {
            let ObjectPayload::Array { dense: Some(dense) } = &mut self.object_mut(id)?.payload
            else {
                unreachable!("fast Array changed representation during value replacement")
            };
            std::mem::replace(&mut dense[index], replacement)
        };
        self.release_replaced_raw_value(previous)
    }

    /// Reserve every container needed to shorten a fast Array prefix without
    /// changing either its dense storage or logical `length` slot.
    pub(crate) fn prepare_array_dense_truncation(
        &self,
        id: ObjectId,
        new_len: u32,
    ) -> Result<PreparedArrayDenseTruncation, HeapError> {
        let ObjectPayload::Array { dense: Some(dense) } = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "fast Array truncation reached a slow Array or wrong object class",
            ));
        };
        let new_len = new_len as usize;
        let removal_count = dense
            .len()
            .checked_sub(new_len)
            .ok_or(HeapError::Invariant(
                "fast Array truncation attempted to grow its dense prefix",
            ))?;
        let removed_atom_count = dense[new_len..]
            .iter()
            .filter(|value| raw_value_atom(value).is_some())
            .count();
        let mut removed = Vec::new();
        removed
            .try_reserve_exact(removal_count)
            .map_err(|_| HeapError::Allocation {
                operation: "detaching fast Array tail",
            })?;
        let mut cleanup = HeapCleanup::default();
        cleanup
            .atoms
            .try_reserve(removed_atom_count)
            .map_err(|_| HeapError::Allocation {
                operation: "recording detached fast Array atoms",
            })?;
        Ok(PreparedArrayDenseTruncation {
            object: id,
            original_len: dense.len(),
            new_len,
            removed_atom_count,
            removed,
            cleanup,
        })
    }

    /// Publish an allocation-complete fast Array truncation and detach every
    /// removed edge and Symbol atom.
    pub(crate) fn commit_array_dense_truncation(
        &mut self,
        mut prepared: PreparedArrayDenseTruncation,
    ) -> Result<HeapCleanup, HeapError> {
        {
            let ObjectPayload::Array { dense: Some(dense) } =
                &mut self.object_mut(prepared.object)?.payload
            else {
                unreachable!("fast Array changed representation before tail truncation")
            };
            if dense.len() != prepared.original_len
                || dense[prepared.new_len..]
                    .iter()
                    .filter(|value| raw_value_atom(value).is_some())
                    .count()
                    != prepared.removed_atom_count
            {
                return Err(HeapError::Invariant(
                    "fast Array changed after truncation preparation",
                ));
            }
            prepared.removed.extend(dense.drain(prepared.new_len..));
        }
        self.release_raw_values_into(prepared.removed, prepared.cleanup)
    }

    /// Truncate the contiguous fast prefix without changing the Array's
    /// logical `length` slot. Every removed edge and Symbol atom is detached.
    pub fn truncate_array_dense(
        &mut self,
        id: ObjectId,
        new_len: u32,
    ) -> Result<HeapCleanup, HeapError> {
        let prepared = self.prepare_array_dense_truncation(id, new_len)?;
        self.commit_array_dense_truncation(prepared)
    }

    /// Read one Arguments object's representation-sensitive indexed prefix.
    pub fn arguments_state(&self, id: ObjectId) -> Result<(bool, Option<u32>), HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Arguments { mapped, fast_len } => Ok((*mapped, *fast_len)),
            _ => Err(HeapError::Invariant(
                "Arguments state requested for an object with the wrong class",
            )),
        }
    }

    /// Update one Arguments object's fast indexed representation. Conversion
    /// to `None` is irreversible at the runtime semantic boundary.
    pub fn set_arguments_fast_len(
        &mut self,
        id: ObjectId,
        fast_len: Option<u32>,
    ) -> Result<(), HeapError> {
        let ObjectPayload::Arguments {
            fast_len: current, ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Arguments fast state update reached an object with the wrong class",
            ));
        };
        *current = fast_len;
        Ok(())
    }

    /// Update the ordinary object's extensibility bit without changing its
    /// shape or property payloads.
    pub fn set_object_extensible(
        &mut self,
        id: ObjectId,
        extensible: bool,
    ) -> Result<(), HeapError> {
        self.object_mut(id)?.extensible = extensible;
        Ok(())
    }

    /// Permanently lock the object's prototype, matching QuickJS's
    /// immutable-prototype flag used by selected intrinsics.
    pub fn set_immutable_prototype(&mut self, id: ObjectId) -> Result<(), HeapError> {
        self.object_mut(id)?.immutable_prototype = true;
        Ok(())
    }

    /// Transactionally replace one property payload.
    ///
    /// New edges are retained before the old payload is detached.  Releasing
    /// the old payload can reclaim an unrooted receiver, so callers must treat
    /// `id` as potentially stale after this operation unless they hold a root.
    pub fn replace_object_slot(
        &mut self,
        id: ObjectId,
        slot_index: usize,
        replacement: PropertySlot,
    ) -> Result<HeapCleanup, HeapError> {
        self.validate_replacement_slot(id, slot_index, &replacement)?;
        let new_edges = property_slot_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;

        let previous = {
            let object = self.object_mut(id)?;
            let slot = object
                .slots
                .get_mut(slot_index)
                .ok_or(HeapError::Invariant(
                    "validated property slot disappeared before replacement",
                ))?;
            std::mem::replace(slot, replacement)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(property_slot_atoms(&previous));
        for edge in property_slot_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Append one property to an object whose shape has exactly one owner.
    ///
    /// New slot edges are retained before the parallel shape and slot vectors
    /// are mutated. Atom ownership is managed by the enclosing runtime: one
    /// live reference for `atom` and any Symbol slot has to be transferred
    /// before this call succeeds.
    pub fn append_unique_object_property(
        &mut self,
        id: ObjectId,
        atom: Atom,
        flags: PropertyFlags,
        replacement: PropertySlot,
    ) -> Result<(), HeapError> {
        let (shape_id, slot_count) = {
            let object = self.object(id)?;
            (object.shape, object.slots.len())
        };
        if self.shape_strong_count(shape_id)? != 1 {
            return Err(HeapError::Invariant(
                "in-place property append reached a shared shape",
            ));
        }
        let index =
            self.shape(shape_id)?
                .unique_append_index(atom)
                .map_err(|error| match error {
                    ShapeError::NullAtom => {
                        HeapError::Invariant("in-place property append used a null atom")
                    }
                    ShapeError::DuplicateAtom(_) => {
                        HeapError::Invariant("in-place property append duplicated a shape atom")
                    }
                    ShapeError::MissingAtom(_) => HeapError::Invariant(
                        "in-place property append reported an impossible missing atom",
                    ),
                    ShapeError::PropertyIndexOverflow => HeapError::Overflow {
                        operation: "appending an in-place shape property",
                    },
                })?;
        if usize::try_from(index) != Ok(slot_count) {
            return Err(HeapError::Invariant(
                "in-place property append found mismatched shape and slot lengths",
            ));
        }
        if !slot_matches_storage(&replacement, flags.storage) {
            return Err(HeapError::Invariant(
                "appended property storage does not match its shape flags",
            ));
        }
        if matches!(replacement, PropertySlot::Data(RawValue::Private(_))) {
            return Err(HeapError::Invariant(
                "private-name identity escaped into an appended object value slot",
            ));
        }

        self.retain_edges_transactionally(&property_slot_edges(&replacement))?;
        let shape = match self.shape_mut(shape_id) {
            Ok(shape) => shape,
            Err(_) => unreachable!("authenticated unique shape disappeared before append"),
        };
        shape.append_unique_property(atom, flags, index);
        let object = match self.object_mut(id) {
            Ok(object) => object,
            Err(_) => unreachable!("authenticated object disappeared before slot append"),
        };
        object.slots.push(replacement);
        debug_assert!(
            self.object(id)
                .and_then(|object| self.validate_object_layout(object))
                .is_ok()
        );
        Ok(())
    }

    /// Transactionally replace a bytecode function's optional HomeObject.
    ///
    /// The replacement is retained before the previous edge is detached, so
    /// changing from an object which owns the replacement cannot make the new
    /// handle stale mid-operation. Identical `Some` values and `None -> None`
    /// are no-ops and therefore cannot overflow or perturb reference counts.
    /// Releasing the old edge may reclaim an unrooted receiver; callers must
    /// keep `id` rooted if they need to use it after this operation.
    pub fn replace_bytecode_function_home_object(
        &mut self,
        id: ObjectId,
        replacement: Option<ObjectId>,
    ) -> Result<HeapCleanup, HeapError> {
        let previous = self.bytecode_function_home_object(id)?;
        if previous == replacement {
            return Ok(HeapCleanup::default());
        }
        if let Some(home_object) = replacement {
            self.retain_raw(RawId::Object(home_object), 1)?;
        }

        let ObjectPayload::BytecodeFunction { home_object, .. } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("bytecode-function payload was validated before HomeObject replacement")
        };
        *home_object = replacement;

        if let Some(home_object) = previous {
            self.release_raw_no_drain(RawId::Object(home_object))?;
        }
        self.drain_zero_queue()
    }

    /// Atomically attach a fresh instance-field initializer to one class.
    ///
    /// The constructor-to-initializer and initializer-to-prototype edges are a
    /// single publication transaction.  Neither edge can be replaced: these
    /// are compiler-owned capabilities, not mutable JavaScript state.
    pub fn attach_bytecode_class_instance_initializer(
        &mut self,
        constructor: ObjectId,
        prototype: ObjectId,
        initializer: ObjectId,
    ) -> Result<(), HeapError> {
        if constructor == prototype || constructor == initializer || prototype == initializer {
            return Err(HeapError::Invariant(
                "class initializer publication reused an object identity",
            ));
        }
        self.object(prototype)?;
        let constructor_object = self.object(constructor)?;
        let ObjectPayload::BytecodeFunction {
            bytecode: constructor_bytecode,
            class_instance_initializer: existing_initializer,
            ..
        } = &constructor_object.payload
        else {
            return Err(HeapError::Invariant(
                "class initializer owner is not a bytecode function",
            ));
        };
        let constructor_metadata = self.function_bytecode(*constructor_bytecode)?;
        if !constructor_object.is_constructor
            || constructor_metadata.metadata.constructor_kind == ConstructorKind::None
            || constructor_metadata.metadata.has_prototype
            || !constructor_metadata.metadata.strict
            || constructor_metadata
                .metadata
                .class_initializer_kind
                .is_some()
            || existing_initializer.is_some()
        {
            return Err(HeapError::Invariant(
                "class initializer owner is not a fresh class constructor",
            ));
        }
        let constructor_realm = constructor_metadata.realm;

        let initializer_object = self.object(initializer)?;
        let ObjectPayload::BytecodeFunction {
            bytecode: initializer_bytecode,
            home_object,
            class_instance_initializer,
            ..
        } = &initializer_object.payload
        else {
            return Err(HeapError::Invariant(
                "class instance initializer is not a bytecode function",
            ));
        };
        let initializer_bytecode = self.function_bytecode(*initializer_bytecode)?;
        if initializer_object.is_constructor
            || home_object.is_some()
            || class_instance_initializer.is_some()
            || initializer_bytecode.realm != constructor_realm
            || initializer_bytecode.metadata.class_initializer_kind
                != Some(ClassInitializerKind::InstanceFields)
            || !initializer_bytecode.metadata.needs_home_object
        {
            return Err(HeapError::Invariant(
                "class instance initializer is not fresh or has the wrong owner realm",
            ));
        }

        self.retain_edges_transactionally(&[RawId::Object(prototype), RawId::Object(initializer)])?;
        let ObjectPayload::BytecodeFunction { home_object, .. } =
            &mut self.object_mut(initializer)?.payload
        else {
            unreachable!("initializer payload was authenticated before edge publication")
        };
        *home_object = Some(prototype);
        let ObjectPayload::BytecodeFunction {
            class_instance_initializer,
            ..
        } = &mut self.object_mut(constructor)?.payload
        else {
            unreachable!("constructor payload was authenticated before edge publication")
        };
        *class_instance_initializer = Some(initializer);
        Ok(())
    }

    /// Claim the one permitted aggregate static-initializer execution for a
    /// class constructor. The claim is deliberately not rolled back after an
    /// abrupt initializer: a leaked constructor must never replay fields or
    /// static blocks through forged privileged bytecode.
    pub fn begin_bytecode_class_static_initializer(
        &mut self,
        constructor: ObjectId,
    ) -> Result<(), HeapError> {
        {
            let constructor_object = self.object(constructor)?;
            let ObjectPayload::BytecodeFunction {
                bytecode,
                class_static_initializer_started,
                ..
            } = &constructor_object.payload
            else {
                return Err(HeapError::Invariant(
                    "class static initializer owner is not a bytecode function",
                ));
            };
            let metadata = self.function_bytecode(*bytecode)?.metadata;
            if !constructor_object.is_constructor
                || metadata.constructor_kind == ConstructorKind::None
                || metadata.has_prototype
                || !metadata.strict
                || metadata.class_initializer_kind.is_some()
            {
                return Err(HeapError::Invariant(
                    "class static initializer owner is not a class constructor",
                ));
            }
            if *class_static_initializer_started {
                return Err(HeapError::Invariant(
                    "class static initializer was already started",
                ));
            }
        }

        let ObjectPayload::BytecodeFunction {
            class_static_initializer_started,
            ..
        } = &mut self.object_mut(constructor)?.payload
        else {
            unreachable!("static initializer owner was authenticated before its one-shot claim")
        };
        *class_static_initializer_started = true;
        Ok(())
    }

    /// Transactionally replace an object's complete shape/slot layout.
    ///
    /// This is the low-level primitive used by immutable shape transitions.
    /// The caller must already own atom references for symbol values in
    /// `slots`; on success those references transfer to the heap.  The returned
    /// cleanup contains every symbol atom detached from the previous slots.
    pub fn replace_object_layout(
        &mut self,
        id: ObjectId,
        shape: ShapeId,
        slots: Vec<PropertySlot>,
    ) -> Result<HeapCleanup, HeapError> {
        self.validate_property_layout(shape, &slots)?;
        let replacement_prototype = self.shape(shape)?.prototype();
        if matches!(self.object(id)?.payload, ObjectPayload::Proxy(_))
            && replacement_prototype.is_some()
        {
            return Err(HeapError::Invariant(
                "Proxy has invalid null-prototype layout or cached target capabilities",
            ));
        }

        // The class payload, private brand, and capability bits are unchanged.
        // Retaining and releasing only the replacement layout edges keeps that
        // payload in place instead of cloning potentially large non-GC state
        // such as an ArrayBuffer backing store.
        let new_edges = object_layout_edges(shape, &slots);
        self.retain_edges_transactionally(&new_edges)?;

        let (previous_shape, previous_slots) = {
            let object = self
                .object_mut(id)
                .expect("authenticated object disappeared during layout replacement");
            (
                std::mem::replace(&mut object.shape, shape),
                std::mem::replace(&mut object.slots, slots),
            )
        };

        let mut cleanup = HeapCleanup::default();
        cleanup
            .atoms
            .extend(previous_slots.iter().flat_map(property_slot_atoms));
        for edge in object_layout_edges(previous_shape, &previous_slots) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Atomically materialize a fast Array's dense prefix into the indexed
    /// suffix of a prepared shape. Existing slots and dense values move in
    /// place, preserving their edge and Symbol-atom ownership without cloning
    /// or temporarily retaining a second copy of the payload.
    pub fn materialize_array_dense_shape(
        &mut self,
        id: ObjectId,
        shape: ShapeId,
    ) -> Result<HeapCleanup, HeapError> {
        let (previous_shape, previous_slot_len, dense_len) = {
            let object = self.object(id)?;
            let ObjectPayload::Array { dense: Some(dense) } = &object.payload else {
                return Err(HeapError::Invariant(
                    "dense materialization reached a slow Array or wrong object class",
                ));
            };
            (object.shape, object.slots.len(), dense.len())
        };
        let previous = self.shape(previous_shape)?;
        let replacement = self.shape(shape)?;
        let replacement_len =
            previous_slot_len
                .checked_add(dense_len)
                .ok_or(HeapError::Overflow {
                    operation: "materializing fast Array slots",
                })?;
        if replacement.prototype() != previous.prototype()
            || replacement.entries().len() != replacement_len
            || replacement.entries().get(..previous_slot_len) != Some(previous.entries())
            || replacement.entries()[previous_slot_len..]
                .iter()
                .any(|entry| entry.flags != PropertyFlags::data(true, true, true))
        {
            return Err(HeapError::Invariant(
                "materialized Array shape does not extend its dense layout",
            ));
        }
        self.object_mut(id)?
            .slots
            .try_reserve(dense_len)
            .map_err(|_| HeapError::Allocation {
                operation: "materializing fast Array slots",
            })?;
        self.retain_shape(shape)?;

        let detached_shape = {
            let object = self
                .object_mut(id)
                .expect("authenticated Array disappeared during dense materialization");
            let ObjectPayload::Array { dense } = &mut object.payload else {
                unreachable!("Array changed class during dense materialization")
            };
            let previous_dense = dense
                .take()
                .expect("fast Array changed representation during materialization");
            object
                .slots
                .extend(previous_dense.into_iter().map(PropertySlot::Data));
            std::mem::replace(&mut object.shape, shape)
        };
        self.release_and_drain(RawId::Shape(detached_shape))
    }

    /// Change only the object's `[[Construct]]` capability bit.
    /// QuickJS keeps this bit independent from the native cproto used to
    /// initialize it, so changing it must not rewrite callable metadata.
    pub(crate) fn set_object_constructor_bit(
        &mut self,
        id: ObjectId,
        enabled: bool,
    ) -> Result<(), HeapError> {
        self.object_mut(id)?.is_constructor = enabled;
        Ok(())
    }

    pub(in crate::engine::heap) fn validate_property_layout(
        &self,
        shape: ShapeId,
        slots: &[PropertySlot],
    ) -> Result<(), HeapError> {
        let shape = self.shape(shape)?;
        if shape.entries().len() != slots.len() {
            return Err(HeapError::Invariant(
                "object slot count does not match its shape",
            ));
        }
        for (entry, slot) in shape.entries().iter().zip(slots) {
            if !slot_matches_storage(slot, entry.flags.storage) {
                return Err(HeapError::Invariant(
                    "object property storage does not match its shape flags",
                ));
            }
            if matches!(slot, PropertySlot::Data(RawValue::Private(_))) {
                return Err(HeapError::Invariant(
                    "private-name identity escaped into an object value slot",
                ));
            }
        }
        Ok(())
    }

    pub(in crate::engine::heap) fn validate_object_layout(
        &self,
        object: &ObjectData,
    ) -> Result<(), HeapError> {
        if !matches!(
            (object.kind, &object.payload),
            (
                ObjectKind::Ordinary,
                ObjectPayload::Ordinary | ObjectPayload::RawJson
            ) | (ObjectKind::ModuleNamespace, ObjectPayload::Ordinary)
                | (ObjectKind::Iterator, ObjectPayload::Ordinary)
                | (ObjectKind::Array, ObjectPayload::Array { .. })
                | (ObjectKind::Arguments, ObjectPayload::Arguments { .. })
                | (
                    ObjectKind::ArrayIterator,
                    ObjectPayload::ArrayIterator { .. }
                )
                | (ObjectKind::ForInIterator, ObjectPayload::ForInIterator(_))
                | (ObjectKind::Primitive, ObjectPayload::Primitive(_))
                | (ObjectKind::Date, ObjectPayload::Date(_))
                | (ObjectKind::RegExp, ObjectPayload::RegExp(_))
                | (
                    ObjectKind::RegExpStringIterator,
                    ObjectPayload::RegExpStringIterator { .. }
                )
                | (ObjectKind::Map, ObjectPayload::Map { .. })
                | (ObjectKind::MapIterator, ObjectPayload::MapIterator { .. })
                | (ObjectKind::Set, ObjectPayload::Set { .. })
                | (ObjectKind::SetIterator, ObjectPayload::SetIterator { .. })
                | (ObjectKind::WeakMap, ObjectPayload::WeakMap { .. })
                | (ObjectKind::WeakSet, ObjectPayload::WeakSet { .. })
                | (ObjectKind::WeakRef, ObjectPayload::WeakRef { .. })
                | (
                    ObjectKind::FinalizationRegistry,
                    ObjectPayload::FinalizationRegistry(_)
                )
                | (ObjectKind::GlobalObject, ObjectPayload::GlobalObject { .. })
                | (ObjectKind::Error, ObjectPayload::Error)
                | (
                    ObjectKind::StringIterator,
                    ObjectPayload::StringIterator { .. }
                )
                | (ObjectKind::IteratorHelper, ObjectPayload::IteratorHelper(_))
                | (ObjectKind::IteratorWrap, ObjectPayload::IteratorWrap(_))
                | (
                    ObjectKind::AsyncFromSyncIterator,
                    ObjectPayload::AsyncFromSyncIterator(_)
                )
                | (ObjectKind::IteratorConcat, ObjectPayload::IteratorConcat(_))
                | (ObjectKind::Proxy, ObjectPayload::Proxy(_))
                | (ObjectKind::ArrayBuffer, ObjectPayload::ArrayBuffer(_))
                | (
                    ObjectKind::SharedArrayBuffer,
                    ObjectPayload::SharedArrayBuffer(_)
                )
                | (ObjectKind::DataView, ObjectPayload::DataView(_))
                | (ObjectKind::TypedArray, ObjectPayload::TypedArray(_))
                | (
                    ObjectKind::NativeFunction,
                    ObjectPayload::NativeFunction { .. }
                )
                | (
                    ObjectKind::BoundFunction,
                    ObjectPayload::BoundFunction { .. }
                )
                | (
                    ObjectKind::BytecodeFunction,
                    ObjectPayload::BytecodeFunction { .. }
                )
                | (ObjectKind::Generator, ObjectPayload::Generator { .. })
                | (ObjectKind::AsyncGenerator, ObjectPayload::AsyncGenerator(_))
                | (
                    ObjectKind::AsyncFunctionState,
                    ObjectPayload::AsyncFunctionState(_)
                )
                | (ObjectKind::Promise, ObjectPayload::Promise(_))
        ) {
            return Err(HeapError::Invariant(
                "object kind does not match its class payload",
            ));
        }
        self.validate_property_layout(object.shape, &object.slots)?;
        let shape = self.shape(object.shape)?;
        if let ObjectPayload::Array { dense: Some(dense) } = &object.payload
            && (u32::try_from(dense.len()).is_err()
                || dense.iter().any(|value| !is_map_storable_value(value)))
        {
            return Err(HeapError::Invariant(
                "fast Array contains an invalid dense value prefix",
            ));
        }
        if let ObjectPayload::Proxy(data) = &object.payload {
            let target = self.object(data.target)?;
            self.object(data.handler)?;
            let target_is_callable = object_data_is_callable(target);
            // The ordinary prototype is always null and public operations are
            // exotic, but class initialization may attach private elements to
            // the Proxy object itself. Those private slots therefore remain
            // valid physical shape entries.
            if shape.prototype().is_some()
                || !object.extensible
                || object.immutable_prototype
                || data.is_callable != target_is_callable
                || object.is_constructor != target.is_constructor
            {
                return Err(HeapError::Invariant(
                    "Proxy has invalid null-prototype layout or cached target capabilities",
                ));
            }
        }
        if let ObjectPayload::ArrayBuffer(data) = &object.payload {
            let byte_length = u32::try_from(data.bytes.len()).map_err(|_| {
                HeapError::Invariant("ArrayBuffer byte length exceeds the supported range")
            })?;
            if object.is_constructor
                || (data.detached && byte_length != 0)
                || data
                    .max_byte_length
                    .is_some_and(|maximum| maximum < byte_length)
                || byte_length > i32::MAX as u32
                || data
                    .max_byte_length
                    .is_some_and(|maximum| maximum > i32::MAX as u32)
            {
                return Err(HeapError::Invariant(
                    "ArrayBuffer has invalid backing-store state",
                ));
            }
        }
        if let ObjectPayload::SharedArrayBuffer(data) = &object.payload {
            let byte_length = data.handle.byte_length();
            let maximum = data.handle.max_byte_length_option();
            if object.is_constructor
                || byte_length > i32::MAX as u32
                || maximum.is_some_and(|maximum| maximum < byte_length || maximum > i32::MAX as u32)
                || data.handle.backing_capacity() != data.handle.max_byte_length()
            {
                return Err(HeapError::Invariant(
                    "SharedArrayBuffer has invalid wrapper or backing-store state",
                ));
            }
        }
        if let ObjectPayload::DataView(data) = &object.payload {
            let (maximum, growable) = match &self.object(data.buffer)?.payload {
                ObjectPayload::ArrayBuffer(buffer) => {
                    (buffer.max_byte_length, buffer.max_byte_length.is_some())
                }
                ObjectPayload::SharedArrayBuffer(buffer) => (
                    buffer.handle.max_byte_length_option(),
                    buffer.handle.is_growable(),
                ),
                _ => {
                    return Err(HeapError::Invariant(
                        "DataView backing object is not an ArrayBuffer or SharedArrayBuffer",
                    ));
                }
            };
            let structural_end = data
                .fixed_byte_length
                .map(|byte_length| u64::from(data.byte_offset) + u64::from(byte_length));
            if object.is_constructor
                || data.byte_offset > i32::MAX as u32
                || data
                    .fixed_byte_length
                    .is_some_and(|byte_length| byte_length > i32::MAX as u32)
                || structural_end.is_some_and(|byte_end| byte_end > i32::MAX as u64)
                || (data.fixed_byte_length.is_none() && !growable)
                || maximum.is_some_and(|maximum| {
                    data.byte_offset > maximum
                        || structural_end.is_some_and(|byte_end| byte_end > u64::from(maximum))
                })
            {
                return Err(HeapError::Invariant(
                    "DataView has an invalid structural view layout",
                ));
            }
        }
        if let ObjectPayload::TypedArray(data) = &object.payload {
            let view = data.view;
            let (maximum, growable) = match &self.object(view.buffer)?.payload {
                ObjectPayload::ArrayBuffer(buffer) => {
                    (buffer.max_byte_length, buffer.max_byte_length.is_some())
                }
                ObjectPayload::SharedArrayBuffer(buffer) => (
                    buffer.handle.max_byte_length_option(),
                    buffer.handle.is_growable(),
                ),
                _ => {
                    return Err(HeapError::Invariant(
                        "TypedArray backing object is not an ArrayBuffer or SharedArrayBuffer",
                    ));
                }
            };
            let width = u32::from(data.element.byte_length());
            let structural_end = view
                .fixed_byte_length
                .map(|byte_length| u64::from(view.byte_offset) + u64::from(byte_length));
            if object.is_constructor
                || view.byte_offset > i32::MAX as u32
                || view.byte_offset % width != 0
                || view.fixed_byte_length.is_some_and(|byte_length| {
                    byte_length > i32::MAX as u32 || byte_length % width != 0
                })
                || structural_end.is_some_and(|byte_end| byte_end > i32::MAX as u64)
                || (view.fixed_byte_length.is_none() && !growable)
                || maximum.is_some_and(|maximum| {
                    view.byte_offset > maximum
                        || structural_end.is_some_and(|byte_end| byte_end > u64::from(maximum))
                })
            {
                return Err(HeapError::Invariant(
                    "TypedArray has an invalid structural view layout",
                ));
            }
        }
        if let ObjectPayload::NativeFunction { data, internal } = &object.payload {
            match (data.target, internal) {
                (
                    NativeFunctionId::ProxyRevoke,
                    Some(InternalCallableData::ProxyRevoke { proxy }),
                ) => {
                    let proxy_is_valid = proxy
                        .map(|proxy| {
                            self.object(proxy)
                                .map(|proxy| matches!(&proxy.payload, ObjectPayload::Proxy(_)))
                        })
                        .transpose()?
                        .unwrap_or(true);
                    if object.is_constructor || !proxy_is_valid {
                        return Err(HeapError::Invariant(
                            "Proxy revoke callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::AsyncFunctionResume(target_kind),
                    Some(InternalCallableData::AsyncFunctionResume { state, kind }),
                ) if target_kind == *kind => {
                    let ObjectPayload::AsyncFunctionState(state_data) =
                        &self.object(*state)?.payload
                    else {
                        return Err(HeapError::Invariant(
                            "AsyncFunction resume callable has invalid hidden state",
                        ));
                    };
                    if object.is_constructor || data.realm != Some(state_data.driver_realm) {
                        return Err(HeapError::Invariant(
                            "AsyncFunction resume callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::DynamicImportHandler(target_kind),
                    Some(InternalCallableData::DynamicImportHandler {
                        module,
                        resolve,
                        reject,
                        kind,
                    }),
                ) if target_kind == *kind => {
                    let realm = data.realm.ok_or(HeapError::Invariant(
                        "dynamic-import handler has no defining realm",
                    ))?;
                    self.context(realm)?;
                    let cache = self.context(module.cache)?;
                    let resolve_is_callable = object_data_is_callable(self.object(*resolve)?);
                    let reject_is_callable = object_data_is_callable(self.object(*reject)?);
                    let module_is_ready = cache
                        .loaded_modules
                        .records
                        .get(module.module.0)
                        .and_then(Option::as_ref)
                        .is_some_and(|record| {
                            matches!(
                                &record.body,
                                RawModuleRecordBody::SourceText { .. }
                                    | RawModuleRecordBody::Json { .. }
                            )
                        });
                    if object.is_constructor
                        || !resolve_is_callable
                        || !reject_is_callable
                        || !module_is_ready
                    {
                        return Err(HeapError::Invariant(
                            "dynamic-import handler has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::AsyncGeneratorResume(target_kind),
                    Some(InternalCallableData::AsyncGeneratorResume { generator, kind }),
                ) if target_kind == *kind => {
                    if object.is_constructor
                        || !matches!(
                            self.object(*generator)?.payload,
                            ObjectPayload::AsyncGenerator(_)
                        )
                    {
                        return Err(HeapError::Invariant(
                            "AsyncGenerator resume callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseResolving(target_kind),
                    Some(InternalCallableData::PromiseResolving { promise, kind, .. }),
                ) if target_kind == *kind => {
                    if object.is_constructor
                        || !matches!(self.object(*promise)?.payload, ObjectPayload::Promise(_))
                    {
                        return Err(HeapError::Invariant(
                            "Promise resolving callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseCapabilityExecutor,
                    Some(InternalCallableData::PromiseCapabilityExecutor(capture)),
                ) => {
                    if object.is_constructor
                        || capture
                            .resolve
                            .iter()
                            .chain(capture.reject.iter())
                            .any(|value| !is_promise_storable_value(value))
                    {
                        return Err(HeapError::Invariant(
                            "Promise capability executor has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseFinallyHandler(_),
                    Some(InternalCallableData::PromiseFinallyHandler {
                        constructor,
                        on_finally,
                    }),
                ) => {
                    let constructor_is_valid = constructor
                        .map(|constructor| {
                            self.object(constructor)
                                .map(|constructor| constructor.is_constructor)
                        })
                        .transpose()?
                        .unwrap_or(true);
                    let on_finally = self.object(*on_finally)?;
                    let on_finally_is_callable = object_data_is_callable(on_finally);
                    if object.is_constructor || !constructor_is_valid || !on_finally_is_callable {
                        return Err(HeapError::Invariant(
                            "Promise finally handler has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseFinallyThunk(_),
                    Some(InternalCallableData::PromiseFinallyThunk { value }),
                ) => {
                    if object.is_constructor || !is_promise_storable_value(value) {
                        return Err(HeapError::Invariant(
                            "Promise finally thunk has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseAllResolveElement,
                    Some(InternalCallableData::PromiseAllResolveElement {
                        values,
                        resolve,
                        index,
                        ..
                    }),
                ) => {
                    let values = self.object(*values)?;
                    let resolve = self.object(*resolve)?;
                    let resolve_is_callable = object_data_is_callable(resolve);
                    if object.is_constructor
                        || !matches!(values.payload, ObjectPayload::Array { .. })
                        || !resolve_is_callable
                        || *index == u32::MAX
                    {
                        return Err(HeapError::Invariant(
                            "Promise.all resolve-element callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseAllSettledElement(target_outcome),
                    Some(InternalCallableData::PromiseAllSettledElement {
                        values,
                        resolve,
                        index,
                        outcome,
                        ..
                    }),
                ) if target_outcome == *outcome => {
                    let values = self.object(*values)?;
                    let resolve = self.object(*resolve)?;
                    let resolve_is_callable = object_data_is_callable(resolve);
                    if object.is_constructor
                        || !matches!(values.payload, ObjectPayload::Array { .. })
                        || !resolve_is_callable
                        || *index == u32::MAX
                    {
                        return Err(HeapError::Invariant(
                            "Promise.allSettled element callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::PromiseAnyRejectElement,
                    Some(InternalCallableData::PromiseAnyRejectElement {
                        errors,
                        reject,
                        index,
                        ..
                    }),
                ) => {
                    let errors = self.object(*errors)?;
                    let reject = self.object(*reject)?;
                    let reject_is_callable = object_data_is_callable(reject);
                    if object.is_constructor
                        || !matches!(errors.payload, ObjectPayload::Array { .. })
                        || !reject_is_callable
                        || *index == u32::MAX
                    {
                        return Err(HeapError::Invariant(
                            "Promise.any reject-element callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::ModuleEvaluation(target_kind),
                    Some(InternalCallableData::ModuleEvaluation { module, kind }),
                ) if target_kind == *kind => {
                    let realm = data.realm.ok_or(HeapError::Invariant(
                        "module-evaluation callback has no defining realm",
                    ))?;
                    self.context(realm)?;
                    let cache = self.context(module.cache)?;
                    let module_is_ready = cache
                        .loaded_modules
                        .records
                        .get(module.module.0)
                        .and_then(Option::as_ref)
                        .is_some_and(|record| {
                            matches!(
                                &record.body,
                                RawModuleRecordBody::SourceText { .. }
                                    | RawModuleRecordBody::Json { .. }
                            )
                        });
                    if object.is_constructor || !module_is_ready {
                        return Err(HeapError::Invariant(
                            "module-evaluation callback has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::AsyncFromSyncIteratorUnwrap,
                    Some(InternalCallableData::AsyncFromSyncIteratorUnwrap { .. }),
                ) => {
                    if object.is_constructor {
                        return Err(HeapError::Invariant(
                            "Async-from-Sync unwrap callable has invalid hidden state",
                        ));
                    }
                }
                (
                    NativeFunctionId::AsyncFromSyncIteratorClose,
                    Some(InternalCallableData::AsyncFromSyncIteratorClose { sync_iterator }),
                ) => {
                    self.object(*sync_iterator)?;
                    if object.is_constructor {
                        return Err(HeapError::Invariant(
                            "Async-from-Sync close callable has invalid hidden state",
                        ));
                    }
                }
                (NativeFunctionId::ProxyRevoke, _)
                | (NativeFunctionId::AsyncFunctionResume(_), _)
                | (NativeFunctionId::AsyncGeneratorResume(_), _)
                | (NativeFunctionId::PromiseResolving(_), _)
                | (NativeFunctionId::PromiseCapabilityExecutor, _)
                | (NativeFunctionId::PromiseFinallyHandler(_), _)
                | (NativeFunctionId::PromiseFinallyThunk(_), _)
                | (NativeFunctionId::PromiseAllResolveElement, _)
                | (NativeFunctionId::PromiseAllSettledElement(_), _)
                | (NativeFunctionId::PromiseAnyRejectElement, _)
                | (NativeFunctionId::ModuleEvaluation(_), _)
                | (NativeFunctionId::DynamicImportHandler(_), _)
                | (NativeFunctionId::AsyncFromSyncIteratorUnwrap, _)
                | (NativeFunctionId::AsyncFromSyncIteratorClose, _)
                | (_, Some(_)) => {
                    return Err(HeapError::Invariant(
                        "native target does not match its internal callable capture",
                    ));
                }
                (_, None) => {}
            }
        }
        if let ObjectPayload::Promise(data) = &object.payload {
            if !is_promise_storable_value(&data.result)
                || (data.state == PromiseState::Pending && data.result != RawValue::Undefined)
                || (data.state != PromiseState::Pending
                    && (!data.fulfill_reactions.is_empty() || !data.reject_reactions.is_empty()))
                || data
                    .fulfill_reactions
                    .iter()
                    .any(|reaction| reaction.kind != PromiseReactionKind::Fulfill)
                || data
                    .reject_reactions
                    .iter()
                    .any(|reaction| reaction.kind != PromiseReactionKind::Reject)
            {
                return Err(HeapError::Invariant(
                    "Promise payload has invalid hidden state",
                ));
            }
        }
        if let ObjectPayload::BytecodeFunction {
            bytecode,
            class_instance_initializer,
            class_static_initializer_started,
            closure_slots,
            ..
        } = &object.payload
        {
            let owner_bytecode = self.function_bytecode(*bytecode)?;
            let expected = usize::from(owner_bytecode.metadata.closure_count);
            if closure_slots.len() != expected {
                return Err(HeapError::Invariant(
                    "function closure slot count does not match its bytecode metadata",
                ));
            }
            if *class_static_initializer_started
                && (!object.is_constructor
                    || owner_bytecode.metadata.constructor_kind == ConstructorKind::None
                    || owner_bytecode.metadata.has_prototype
                    || !owner_bytecode.metadata.strict
                    || owner_bytecode.metadata.class_initializer_kind.is_some())
            {
                return Err(HeapError::Invariant(
                    "class static initializer guard has malformed ownership metadata",
                ));
            }
            if let Some(initializer) = class_instance_initializer {
                let initializer_object = self.object(*initializer)?;
                let ObjectPayload::BytecodeFunction {
                    bytecode: initializer_bytecode,
                    home_object,
                    class_instance_initializer: nested_initializer,
                    ..
                } = &initializer_object.payload
                else {
                    return Err(HeapError::Invariant(
                        "class instance initializer is not a bytecode function",
                    ));
                };
                let initializer_bytecode = self.function_bytecode(*initializer_bytecode)?;
                if !object.is_constructor
                    || owner_bytecode.metadata.constructor_kind == ConstructorKind::None
                    || owner_bytecode.metadata.has_prototype
                    || !owner_bytecode.metadata.strict
                    || owner_bytecode.metadata.class_initializer_kind.is_some()
                    || initializer_object.is_constructor
                    || home_object.is_none()
                    || nested_initializer.is_some()
                    || initializer_bytecode.realm != owner_bytecode.realm
                    || initializer_bytecode.metadata.class_initializer_kind
                        != Some(ClassInitializerKind::InstanceFields)
                    || !initializer_bytecode.metadata.needs_home_object
                {
                    return Err(HeapError::Invariant(
                        "class instance initializer edge has malformed ownership metadata",
                    ));
                }
            }
        }
        if let ObjectPayload::Generator { state, activation } = &object.payload {
            if object.is_constructor
                || matches!(state, GeneratorState::Executing | GeneratorState::Completed)
                    != activation.is_none()
            {
                return Err(HeapError::Invariant(
                    "generator object has inconsistent state and activation",
                ));
            }
            if let Some(activation) = activation.as_deref() {
                let bytecode = self.function_bytecode(activation.bytecode)?;
                let vm = &activation.vm;
                let function = self.object(vm.current_function)?;
                if bytecode.metadata.function_kind != FunctionKind::Generator
                    || bytecode.metadata.constructor_kind != ConstructorKind::None
                    || !bytecode.metadata.has_prototype
                    || bytecode.realm != vm.callee_realm
                    || vm.strict != bytecode.metadata.strict
                    || self.context(vm.callee_realm)?.global_object != vm.callee_global
                    || !matches!(
                        function.payload,
                        ObjectPayload::BytecodeFunction {
                            bytecode: owner,
                            ..
                        } if owner == activation.bytecode
                    )
                    || function.is_constructor
                    || activation.arguments.len() < usize::from(bytecode.metadata.argument_count)
                    || activation.locals.len() != usize::from(bytecode.metadata.local_count)
                    || activation.reusable_captured_locals.len() != activation.locals.len()
                    || vm.stack.len() > usize::from(bytecode.metadata.max_stack)
                    || vm.pc == 0
                    || vm.pc > bytecode.code.len()
                {
                    return Err(HeapError::Invariant(
                        "generator activation has invalid frame metadata",
                    ));
                }
                let suspension_matches = matches!(
                    (state, bytecode.code.get(vm.pc - 1)),
                    (
                        GeneratorState::SuspendedStart,
                        Some(Instruction::InitialYield)
                    ) | (GeneratorState::SuspendedYield, Some(Instruction::Yield))
                        | (
                            GeneratorState::SuspendedYieldStar,
                            Some(Instruction::YieldStar)
                        )
                );
                if !suspension_matches {
                    return Err(HeapError::Invariant(
                        "generator activation is not parked after its suspension opcode",
                    ));
                }
                for value in vm
                    .stack
                    .iter()
                    .chain(std::iter::once(&vm.this_value))
                    .chain(vm.normalized_this.iter())
                    .chain(std::iter::once(&vm.new_target))
                {
                    if !is_map_storable_value(value) {
                        return Err(HeapError::Invariant(
                            "generator activation contains an internal-only value",
                        ));
                    }
                }
                for binding in activation.arguments.iter().chain(activation.locals.iter()) {
                    match binding {
                        GeneratorFrameBinding::Direct(value) if !is_map_storable_value(value) => {
                            return Err(HeapError::Invariant(
                                "generator frame binding contains an internal-only value",
                            ));
                        }
                        GeneratorFrameBinding::Private(atom) if atom.is_null() => {
                            return Err(HeapError::Invariant(
                                "generator private binding contains the null atom",
                            ));
                        }
                        GeneratorFrameBinding::PrivateCallable(callable) => {
                            let callable = self.object(*callable)?;
                            if !object_data_is_callable(callable) {
                                return Err(HeapError::Invariant(
                                    "generator private callable binding is not callable",
                                ));
                            }
                        }
                        GeneratorFrameBinding::Captured(var_ref) => {
                            self.var_ref(*var_ref)?;
                        }
                        GeneratorFrameBinding::Direct(_)
                        | GeneratorFrameBinding::Private(_)
                        | GeneratorFrameBinding::Uninitialized => {}
                    }
                }
                for region in &vm.regions {
                    match *region {
                        crate::engine::vm::VmUnwindRegion::Catch {
                            target,
                            stack_depth,
                        } if target >= bytecode.code.len() || stack_depth > vm.stack.len() => {
                            return Err(HeapError::Invariant(
                                "generator catch region is outside its saved frame",
                            ));
                        }
                        crate::engine::vm::VmUnwindRegion::Iterator { record_base, .. }
                            if record_base.saturating_add(1) >= vm.stack.len() =>
                        {
                            return Err(HeapError::Invariant(
                                "generator iterator region is outside its saved frame",
                            ));
                        }
                        crate::engine::vm::VmUnwindRegion::Iterator {
                            asynchronous: true, ..
                        } => {
                            return Err(HeapError::Invariant(
                                "generator activation contains an invalid iterator state",
                            ));
                        }
                        crate::engine::vm::VmUnwindRegion::Iterator {
                            record_base,
                            enabled: false,
                            asynchronous: false,
                        } if !matches!(vm.stack.get(record_base), Some(RawValue::Undefined)) => {
                            return Err(HeapError::Invariant(
                                "generator activation contains an invalid completed iterator",
                            ));
                        }
                        crate::engine::vm::VmUnwindRegion::Catch { .. }
                        | crate::engine::vm::VmUnwindRegion::Iterator { .. } => {}
                    }
                }
            }
        }
        if let ObjectPayload::AsyncGenerator(data) = &object.payload {
            validate_async_generator_state(self, object, data)?;
        }
        if let ObjectPayload::AsyncFunctionState(data) = &object.payload {
            validate_async_function_state(self, object, data)?;
        }
        if let ObjectPayload::IteratorHelper(data) = &object.payload {
            if object.is_constructor {
                return Err(HeapError::Invariant(
                    "Iterator Helper object is constructable",
                ));
            }
            validate_iterator_helper_data(self, data)?;
        }
        if let ObjectPayload::IteratorWrap(data) = &object.payload {
            if object.is_constructor {
                return Err(HeapError::Invariant(
                    "Iterator Wrap object is constructable",
                ));
            }
            validate_iterator_wrap_data(self, data)?;
        }
        if let ObjectPayload::AsyncFromSyncIterator(data) = &object.payload {
            if object.is_constructor {
                return Err(HeapError::Invariant(
                    "Async-from-Sync Iterator object is constructable",
                ));
            }
            validate_async_from_sync_iterator_data(self, data)?;
        }
        if let ObjectPayload::IteratorConcat(data) = &object.payload {
            if object.is_constructor {
                return Err(HeapError::Invariant(
                    "Iterator Concat object is constructable",
                ));
            }
            validate_iterator_concat_data(self, data)?;
        }
        if let ObjectPayload::BoundFunction {
            target,
            this_value,
            arguments,
        } = &object.payload
        {
            let target = self.object(*target)?;
            if !object_data_is_callable(target) {
                return Err(HeapError::Invariant(
                    "bound function target is not callable",
                ));
            }
            if std::iter::once(this_value)
                .chain(arguments.iter())
                .any(|value| !is_map_storable_value(value))
            {
                return Err(HeapError::Invariant(
                    "bound function payload contains an internal-only value",
                ));
            }
        }
        if let ObjectPayload::Map {
            records,
            live_indices,
            size,
        } = &object.payload
        {
            let mut live = 0usize;
            let mut expected_indices = BTreeSet::new();
            for (index, record) in records.iter().enumerate() {
                match &record.key {
                    Some(key) => {
                        if !is_map_storable_value(key) || !is_map_storable_value(&record.value) {
                            return Err(HeapError::Invariant(
                                "Map record contains an internal value sentinel",
                            ));
                        }
                        live = live.checked_add(1).ok_or(HeapError::Overflow {
                            operation: "validating Map size",
                        })?;
                        expected_indices.insert(index);
                    }
                    None if !matches!(record.value, RawValue::Undefined) => {
                        return Err(HeapError::Invariant(
                            "Map tombstone retains a value payload",
                        ));
                    }
                    None => {}
                }
            }
            if live != *size {
                return Err(HeapError::Invariant(
                    "Map live record count does not match its payload",
                ));
            }
            if *live_indices != expected_indices {
                return Err(HeapError::Invariant(
                    "Map live index does not match its record layout",
                ));
            }
        }
        if let ObjectPayload::MapIterator {
            object: source,
            next_index,
            current_index,
            ..
        } = &object.payload
        {
            match (source, current_index) {
                (Some(map), current) => {
                    let ObjectPayload::Map { records, .. } = &self.object(*map)?.payload else {
                        return Err(HeapError::Invariant(
                            "Map Iterator source does not have the Map class",
                        ));
                    };
                    if current.is_some_and(|index| index >= *next_index || index >= records.len()) {
                        return Err(HeapError::Invariant(
                            "Map Iterator current record is outside its stable cursor",
                        ));
                    }
                }
                (None, None) => {}
                (None, Some(_)) => {
                    return Err(HeapError::Invariant(
                        "completed Map Iterator retains a current record",
                    ));
                }
            }
        }
        if let ObjectPayload::Set {
            records,
            live_indices,
            size,
        } = &object.payload
        {
            let mut live = 0usize;
            let mut expected_indices = BTreeSet::new();
            for (index, record) in records.iter().enumerate() {
                if !matches!(record.value, RawValue::Undefined) {
                    return Err(HeapError::Invariant(
                        "Set record value slot is not undefined",
                    ));
                }
                if let Some(key) = &record.key {
                    if !is_map_storable_value(key) {
                        return Err(HeapError::Invariant(
                            "Set record contains an internal value sentinel",
                        ));
                    }
                    live = live.checked_add(1).ok_or(HeapError::Overflow {
                        operation: "validating Set size",
                    })?;
                    expected_indices.insert(index);
                }
            }
            if live != *size {
                return Err(HeapError::Invariant(
                    "Set live record count does not match its payload",
                ));
            }
            if *live_indices != expected_indices {
                return Err(HeapError::Invariant(
                    "Set live index does not match its record layout",
                ));
            }
        }
        if let ObjectPayload::SetIterator {
            object: source,
            next_index,
            current_index,
            ..
        } = &object.payload
        {
            match (source, current_index) {
                (Some(set), current) => {
                    let ObjectPayload::Set { records, .. } = &self.object(*set)?.payload else {
                        return Err(HeapError::Invariant(
                            "Set Iterator source does not have the Set class",
                        ));
                    };
                    if current.is_some_and(|index| index >= *next_index || index >= records.len()) {
                        return Err(HeapError::Invariant(
                            "Set Iterator current record is outside its stable cursor",
                        ));
                    }
                }
                (None, None) => {}
                (None, Some(_)) => {
                    return Err(HeapError::Invariant(
                        "completed Set Iterator retains a current record",
                    ));
                }
            }
        }
        if let ObjectPayload::WeakMap { records } = &object.payload {
            records.validate_order()?;
            if records.values().any(|value| !is_map_storable_value(value)) {
                return Err(HeapError::Invariant(
                    "WeakMap record contains an internal value sentinel",
                ));
            }
            for key in records.keys() {
                if let WeakCollectionKey::Object(key) = key {
                    // Weak records deliberately outlive a zero-refcount key
                    // until the next explicit weak-record pruning pass. A
                    // stale generational identity is therefore valid here;
                    // only revalidate keys which are still live.
                    if self.is_live(RawId::Object(*key)) {
                        self.object(*key)?;
                    }
                }
            }
        }
        if let ObjectPayload::WeakSet { records } = &object.payload {
            records.validate_order()?;
            for key in records.keys() {
                if let WeakCollectionKey::Object(key) = key {
                    if self.is_live(RawId::Object(*key)) {
                        self.object(*key)?;
                    }
                }
            }
        }
        if let ObjectPayload::WeakRef {
            target: Some(WeakCollectionKey::Object(target)),
        } = &object.payload
            && self.is_live(RawId::Object(*target))
        {
            self.object(*target)?;
        }
        if let ObjectPayload::FinalizationRegistry(data) = &object.payload {
            if !object_data_is_callable(self.object(data.callback)?) {
                return Err(HeapError::Invariant(
                    "FinalizationRegistry callback is not callable",
                ));
            }
            self.context(data.realm)?;
            for entry in &data.entries {
                if !is_map_storable_value(&entry.held_value)
                    || raw_value_matches_weak_key(&entry.held_value, entry.target)
                {
                    return Err(HeapError::Invariant(
                        "FinalizationRegistry contains an invalid held value",
                    ));
                }
                for weak in [Some(entry.target), entry.unregister_token]
                    .into_iter()
                    .flatten()
                {
                    if let WeakCollectionKey::Object(target) = weak
                        && self.is_live(RawId::Object(target))
                    {
                        self.object(target)?;
                    }
                }
            }
        }
        Ok(())
    }

    pub(in crate::engine::heap) fn validate_replacement_slot(
        &self,
        id: ObjectId,
        slot_index: usize,
        replacement: &PropertySlot,
    ) -> Result<(), HeapError> {
        let object = self.object(id)?;
        let shape = self.shape(object.shape)?;
        let entry = shape.entries().get(slot_index).ok_or(HeapError::Invariant(
            "property slot index is outside the object shape",
        ))?;
        if !slot_matches_storage(replacement, entry.flags.storage) {
            return Err(HeapError::Invariant(
                "replacement property storage does not match its shape flags",
            ));
        }
        if matches!(replacement, PropertySlot::Data(RawValue::Private(_))) {
            return Err(HeapError::Invariant(
                "private-name identity escaped into an object value slot",
            ));
        }
        Ok(())
    }

    pub(in crate::engine::heap) fn validate_slot_identity(
        &self,
        id: RawId,
    ) -> Result<usize, HeapError> {
        let index = id.index() as usize;
        let slot = self.slots.get(index).ok_or(HeapError::Stale {
            index: id.index(),
            generation: id.generation(),
        })?;
        if slot.generation != id.generation() {
            return Err(HeapError::Stale {
                index: id.index(),
                generation: id.generation(),
            });
        }
        let actual = slot.state.kind().ok_or(HeapError::Stale {
            index: id.index(),
            generation: id.generation(),
        })?;
        if actual != id.kind() {
            return Err(HeapError::WrongKind {
                expected: id.kind(),
                actual,
            });
        }
        Ok(index)
    }
}
