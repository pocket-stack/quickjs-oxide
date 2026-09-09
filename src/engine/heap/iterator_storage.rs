use super::*;

impl Heap {
    /// Advance one branded String Iterator by one Unicode code point.
    ///
    /// The stored cursor is a UTF-16 code-unit index. A valid lead/trail pair
    /// advances by two and is returned unchanged as a two-unit string; every
    /// lone surrogate advances by one and is preserved verbatim. At end the
    /// backing string is released eagerly, matching QuickJS's transition to
    /// an undefined iterator target.
    pub fn string_iterator_next(&mut self, id: ObjectId) -> Result<Option<JsString>, HeapError> {
        let object = self.object_mut(id)?;
        let ObjectPayload::StringIterator { string, next_index } = &mut object.payload else {
            return Err(HeapError::Invariant(
                "String Iterator next reached an object with the wrong class",
            ));
        };
        let Some(value) = string.as_ref() else {
            return Ok(None);
        };
        if *next_index >= value.len() {
            *string = None;
            return Ok(None);
        }

        let first = value
            .code_unit_at(*next_index)
            .expect("validated String Iterator index must name a code unit");
        let pair = (0xd800..=0xdbff).contains(&first)
            && next_index
                .checked_add(1)
                .and_then(|index| value.code_unit_at(index))
                .is_some_and(|unit| (0xdc00..=0xdfff).contains(&unit));
        let width = if pair { 2 } else { 1 };
        let result = JsString::try_from_utf16((0..width).map(|offset| {
            value
                .code_unit_at(*next_index + offset)
                .expect("validated String Iterator code-point width must remain in bounds")
        }))
        .map_err(|_| HeapError::Invariant("String Iterator produced an oversized code point"))?;
        *next_index += width;
        Ok(Some(result))
    }

    /// Snapshot one branded RegExp String Iterator's retained matcher, input
    /// string, cached flag modes, and completion state.
    pub fn regexp_string_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(ObjectId, JsString, bool, bool, bool), HeapError> {
        let ObjectPayload::RegExpStringIterator {
            regexp,
            string,
            global,
            full_unicode,
            done,
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "RegExp String Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*regexp, string.clone(), *global, *full_unicode, *done))
    }

    /// Mark one branded RegExp String Iterator complete without releasing its
    /// matcher or input string. Pinned QuickJS retains both payload values until
    /// the iterator object itself is finalized.
    pub fn finish_regexp_string_iterator(&mut self, id: ObjectId) -> Result<(), HeapError> {
        let ObjectPayload::RegExpStringIterator { done, .. } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "RegExp String Iterator completion reached an object with the wrong class",
            ));
        };
        *done = true;
        Ok(())
    }

    /// Snapshot one branded Array Iterator's live target, cursor, and mode.
    pub fn array_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, u32, ArrayIteratorKind), HeapError> {
        let ObjectPayload::ArrayIterator {
            object,
            next_index,
            kind,
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Array Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*object, *next_index, *kind))
    }

    /// Advance a branded Array Iterator after its current element has been
    /// selected. Property lookup may still throw after this update, matching
    /// QuickJS's cursor order.
    pub fn set_array_iterator_index(
        &mut self,
        id: ObjectId,
        next_index: u32,
    ) -> Result<(), HeapError> {
        let ObjectPayload::ArrayIterator {
            object,
            next_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Array Iterator advance reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant(
                "completed Array Iterator was advanced",
            ));
        }
        *stored = next_index;
        Ok(())
    }

    /// Permanently detach a completed Array Iterator target and release its
    /// owned object edge.
    pub fn finish_array_iterator(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let source = {
            let ObjectPayload::ArrayIterator { object, .. } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Array Iterator completion reached an object with the wrong class",
                ));
            };
            object.take()
        };
        let Some(source) = source else {
            return Ok(HeapCleanup::default());
        };
        self.release_raw_no_drain(RawId::Object(source))?;
        self.drain_zero_queue()
    }

    /// Snapshot one genuine Iterator Helper payload. Raw values in the clone
    /// do not own additional arena or atom references.
    pub(crate) fn iterator_helper_state(
        &self,
        id: ObjectId,
    ) -> Result<IteratorHelperData, HeapError> {
        let ObjectPayload::IteratorHelper(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Iterator Helper snapshot reached an object with the wrong class",
            ));
        };
        Ok(data.clone())
    }

    /// Snapshot one `Iterator.from` forwarding wrapper. Raw values in the
    /// clone do not own additional arena or atom references.
    pub(crate) fn iterator_wrap_state(
        &self,
        id: ObjectId,
    ) -> Result<(RawValue, RawValue), HeapError> {
        let ObjectPayload::IteratorWrap(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Iterator Wrap snapshot reached an object with the wrong class",
            ));
        };
        Ok((data.source.clone(), data.next.clone()))
    }

    /// Snapshot one branded Async-from-Sync iterator. The cached raw method
    /// does not gain an additional arena or atom occurrence in the clone.
    pub(crate) fn async_from_sync_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(ObjectId, RawValue), HeapError> {
        let ObjectPayload::AsyncFromSyncIterator(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Async-from-Sync Iterator snapshot reached an object with the wrong class",
            ));
        };
        Ok((data.sync_iterator, data.next.clone()))
    }

    /// Snapshot one `Iterator.concat` state machine. Raw values in the clone
    /// do not own additional arena or atom references.
    pub(crate) fn iterator_concat_state(
        &self,
        id: ObjectId,
    ) -> Result<IteratorConcatData, HeapError> {
        let ObjectPayload::IteratorConcat(data) = &self.object(id)?.payload else {
            return Err(HeapError::Invariant(
                "Iterator Concat snapshot reached an object with the wrong class",
            ));
        };
        Ok(data.clone())
    }

    /// Set or clear the `Iterator.concat` reentrancy guard.
    pub(crate) fn set_iterator_concat_running(
        &mut self,
        id: ObjectId,
        running: bool,
    ) -> Result<(), HeapError> {
        let ObjectPayload::IteratorConcat(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Iterator Concat execution update reached an object with the wrong class",
            ));
        };
        data.running = running;
        Ok(())
    }

    /// Replace the lazily created current iterator.
    pub(crate) fn set_iterator_concat_iterator(
        &mut self,
        id: ObjectId,
        replacement: Option<ObjectId>,
    ) -> Result<HeapCleanup, HeapError> {
        self.iterator_concat_state(id)?;
        if let Some(replacement) = replacement {
            self.object(replacement)?;
        }
        let new_edges: Vec<_> = replacement.into_iter().map(RawId::Object).collect();
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::IteratorConcat(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Iterator Concat iterator update reached an object with the wrong class",
                ));
            };
            std::mem::replace(&mut data.iterator, replacement)
        };
        if let Some(previous) = previous {
            self.release_raw_no_drain(RawId::Object(previous))?;
        }
        self.drain_zero_queue()
    }

    /// Cache the current iterator's `next` property. Symbol atom ownership is
    /// transferred separately by the runtime before this mutation.
    pub(crate) fn set_iterator_concat_next(
        &mut self,
        id: ObjectId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        self.iterator_concat_state(id)?;
        if !is_map_storable_value(&replacement) {
            return Err(HeapError::Invariant(
                "Iterator Concat next cache contains an internal value sentinel",
            ));
        }
        let new_edges = raw_value_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::IteratorConcat(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Iterator Concat next update reached an object with the wrong class",
                ));
            };
            std::mem::replace(&mut data.next, replacement)
        };
        self.release_replaced_raw_value(previous)
    }

    /// Finish the current iterable and advance to the next retained pair.
    pub(crate) fn advance_iterator_concat(
        &mut self,
        id: ObjectId,
    ) -> Result<HeapCleanup, HeapError> {
        let (item, iterator, next) = {
            let ObjectPayload::IteratorConcat(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Iterator Concat advance reached an object with the wrong class",
                ));
            };
            if data.index >= data.items.len() {
                return Err(HeapError::Invariant(
                    "Iterator Concat advanced past its retained inputs",
                ));
            }
            let item = data.items[data.index].take().ok_or(HeapError::Invariant(
                "Iterator Concat current input was already released",
            ))?;
            let iterator = data.iterator.take().ok_or(HeapError::Invariant(
                "Iterator Concat advanced without a current iterator",
            ))?;
            let next = std::mem::replace(&mut data.next, RawValue::Undefined);
            data.index += 1;
            (item, iterator, next)
        };
        // Pinned QuickJS releases a normally exhausted input in this order:
        // active iterator, cached next, captured open method, iterable.
        let mut cleanup = HeapCleanup::default();
        self.release_raw_no_drain(RawId::Object(iterator))?;
        cleanup.atoms.extend(raw_value_atom(&next));
        for edge in raw_value_edges(&next) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.atoms.extend(raw_value_atom(&item.method));
        for edge in raw_value_edges(&item.method) {
            self.release_raw_no_drain(edge)?;
        }
        self.release_raw_no_drain(RawId::Object(item.iterable))?;
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Release the current iterator and every unvisited input after
    /// `Iterator Concat.prototype.return` completes.
    pub(crate) fn clear_iterator_concat(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let (items, iterator, next) = {
            let ObjectPayload::IteratorConcat(data) = &mut self.object_mut(id)?.payload else {
                return Err(HeapError::Invariant(
                    "Iterator Concat clear reached an object with the wrong class",
                ));
            };
            let items = data.items[data.index..]
                .iter_mut()
                .filter_map(Option::take)
                .collect::<Vec<_>>();
            data.index = data.items.len();
            let iterator = data.iterator.take();
            let next = std::mem::replace(&mut data.next, RawValue::Undefined);
            (items, iterator, next)
        };

        let mut cleanup = HeapCleanup::default();
        for item in items {
            self.release_raw_no_drain(RawId::Object(item.iterable))?;
            cleanup.atoms.extend(raw_value_atom(&item.method));
            for edge in raw_value_edges(&item.method) {
                self.release_raw_no_drain(edge)?;
            }
            cleanup.merge(self.drain_zero_queue()?);
        }
        if let Some(iterator) = iterator {
            self.release_raw_no_drain(RawId::Object(iterator))?;
        }
        cleanup.atoms.extend(raw_value_atom(&next));
        for edge in raw_value_edges(&next) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Replace the source iterator edge retained by a helper. The new edge is
    /// retained before the previous edge is detached.
    #[cfg(test)]
    pub(crate) fn set_iterator_helper_source(
        &mut self,
        id: ObjectId,
        replacement: ObjectId,
    ) -> Result<HeapCleanup, HeapError> {
        self.object(replacement)?;
        let previous = self.iterator_helper_state(id)?.source;
        self.retain_raw(RawId::Object(replacement), 1)?;
        let ObjectPayload::IteratorHelper(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("Iterator Helper was validated before retaining its source")
        };
        data.source = replacement;
        self.release_raw_no_drain(RawId::Object(previous))?;
        self.drain_zero_queue()
    }

    /// Replace the cached `next` value transactionally. A replacement Symbol
    /// atom transfers on success; the previous Symbol atom is returned in the
    /// cleanup.
    #[cfg(test)]
    pub(crate) fn set_iterator_helper_next(
        &mut self,
        id: ObjectId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        self.replace_iterator_helper_raw_value(id, IteratorHelperRawValueField::Next, replacement)
    }

    /// Replace the helper callback transactionally, preserving its callable
    /// invariant and retaining the replacement edge before detaching the old
    /// callback.
    #[cfg(test)]
    pub(crate) fn set_iterator_helper_callback(
        &mut self,
        id: ObjectId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        self.replace_iterator_helper_raw_value(
            id,
            IteratorHelperRawValueField::Callback,
            replacement,
        )
    }

    /// Replace the optional inner iterator used by `flatMap`.
    pub(crate) fn set_iterator_helper_inner(
        &mut self,
        id: ObjectId,
        replacement: Option<ObjectId>,
    ) -> Result<HeapCleanup, HeapError> {
        if let Some(replacement) = replacement {
            self.object(replacement)?;
        }
        let current = self.iterator_helper_state(id)?;
        if current.kind != IteratorHelperKind::FlatMap && replacement.is_some() {
            return Err(HeapError::Invariant(
                "only a flatMap Iterator Helper may retain an inner iterator",
            ));
        }
        let new_edges: Vec<_> = replacement.into_iter().map(RawId::Object).collect();
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::IteratorHelper(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("Iterator Helper was validated before retaining its inner iterator")
            };
            std::mem::replace(&mut data.inner, replacement)
        };
        if let Some(previous) = previous {
            self.release_raw_no_drain(RawId::Object(previous))?;
        }
        self.drain_zero_queue()
    }

    #[cfg(test)]
    pub(in crate::engine::heap) fn replace_iterator_helper_raw_value(
        &mut self,
        id: ObjectId,
        field: IteratorHelperRawValueField,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        let mut candidate = self.iterator_helper_state(id)?;
        *field.get_mut(&mut candidate) = replacement.clone();
        validate_iterator_helper_data(self, &candidate)?;

        let new_edges = raw_value_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::IteratorHelper(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("Iterator Helper was validated before retaining replacement edges")
            };
            std::mem::replace(field.get_mut(data), replacement)
        };
        self.release_replaced_raw_value(previous)
    }

    /// Replace the source iterator retained by an Iterator Wrap.
    #[cfg(test)]
    pub(crate) fn set_iterator_wrap_source(
        &mut self,
        id: ObjectId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        let (_, next) = self.iterator_wrap_state(id)?;
        let candidate = IteratorWrapData {
            source: replacement.clone(),
            next,
        };
        validate_iterator_wrap_data(self, &candidate)?;

        let new_edges = raw_value_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::IteratorWrap(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("Iterator Wrap was validated before retaining replacement edges")
            };
            std::mem::replace(&mut data.source, replacement)
        };
        self.release_replaced_raw_value(previous)
    }

    /// Replace the cached `next` value retained by an Iterator Wrap.
    #[cfg(test)]
    pub(crate) fn set_iterator_wrap_next(
        &mut self,
        id: ObjectId,
        replacement: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        let (source, _) = self.iterator_wrap_state(id)?;
        let candidate = IteratorWrapData {
            source,
            next: replacement.clone(),
        };
        validate_iterator_wrap_data(self, &candidate)?;

        let new_edges = raw_value_edges(&replacement);
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::IteratorWrap(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("Iterator Wrap was validated before retaining replacement edges")
            };
            std::mem::replace(&mut data.next, replacement)
        };
        self.release_replaced_raw_value(previous)
    }

    /// Update the helper's signed 64-bit limit/callback index.
    pub(crate) fn set_iterator_helper_count(
        &mut self,
        id: ObjectId,
        count: i64,
    ) -> Result<(), HeapError> {
        let mut candidate = self.iterator_helper_state(id)?;
        candidate.count = count;
        validate_iterator_helper_data(self, &candidate)?;
        let ObjectPayload::IteratorHelper(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("Iterator Helper was validated before updating its count")
        };
        data.count = count;
        Ok(())
    }

    /// Set or clear QuickJS's reentrancy guard.
    pub(crate) fn set_iterator_helper_running(
        &mut self,
        id: ObjectId,
        running: bool,
    ) -> Result<(), HeapError> {
        let ObjectPayload::IteratorHelper(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "Iterator Helper execution update reached an object with the wrong class",
            ));
        };
        data.executing = running;
        Ok(())
    }

    /// Update completion and reentrancy flags together. Marking a helper done
    /// preserves every payload-owned value until object finalization.
    pub(crate) fn set_iterator_helper_done_and_running(
        &mut self,
        id: ObjectId,
        done: bool,
        running: bool,
    ) -> Result<(), HeapError> {
        let mut candidate = self.iterator_helper_state(id)?;
        candidate.done = done;
        candidate.executing = running;
        validate_iterator_helper_data(self, &candidate)?;
        let ObjectPayload::IteratorHelper(data) = &mut self.object_mut(id)?.payload else {
            unreachable!("Iterator Helper was validated before completing its resume")
        };
        data.done = done;
        data.executing = running;
        Ok(())
    }

    /// Advance within one snapshotted level without cloning the complete key
    /// vector or visited set. Non-enumerable and duplicate prototype keys are
    /// consumed internally because neither can be yielded.
    pub fn next_for_in_candidate(&mut self, id: ObjectId) -> Result<ForInCandidate, HeapError> {
        let ObjectPayload::ForInIterator(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "for-in advance reached an object with the wrong class",
            ));
        };
        loop {
            let Some(object) = data.object else {
                return Ok(ForInCandidate::Done);
            };
            if data.fast_array {
                let index = u32::try_from(data.index)
                    .map_err(|_| HeapError::Invariant("for-in fast Array index exceeded Uint32"))?;
                if index < data.array_count {
                    data.index += 1;
                    return Ok(ForInCandidate::ArrayIndex { object, index });
                }
                return Ok(ForInCandidate::BaseComplete {
                    object,
                    fast_array: true,
                });
            }
            if data.index >= data.properties.len() {
                if !data.in_prototype_chain {
                    return Ok(ForInCandidate::BaseComplete {
                        object,
                        fast_array: false,
                    });
                }
                return Ok(ForInCandidate::LevelComplete(object));
            }

            let entry = data.properties[data.index].clone();
            data.index += 1;
            if data.in_prototype_chain && !data.visited.insert(entry.name.clone()) {
                continue;
            }
            if !entry.enumerable {
                continue;
            }
            let object = data.object.ok_or(HeapError::Invariant(
                "for-in property snapshot lost its current object",
            ))?;
            return Ok(ForInCandidate::Property {
                object,
                name: entry.name,
            });
        }
    }

    /// Complete QuickJS's one-time prototype-chain preparation. A generic
    /// iterator records its original base snapshot; a fast Array records a
    /// fresh own-key snapshot supplied after the prototype pre-scan.
    pub fn enter_for_in_prototype_chain(
        &mut self,
        id: ObjectId,
        refreshed_fast_properties: Option<Vec<ForInProperty>>,
    ) -> Result<(), HeapError> {
        let ObjectPayload::ForInIterator(data) = &mut self.object_mut(id)?.payload else {
            return Err(HeapError::Invariant(
                "for-in prototype preparation reached an object with the wrong class",
            ));
        };
        if data.in_prototype_chain {
            return Err(HeapError::Invariant(
                "for-in prototype chain was prepared more than once",
            ));
        }
        let properties = refreshed_fast_properties
            .as_ref()
            .unwrap_or(&data.properties);
        data.visited
            .extend(properties.iter().map(|entry| entry.name.clone()));
        data.in_prototype_chain = true;
        Ok(())
    }

    /// Install the next prototype level transactionally. The new current edge
    /// is retained before the old level is detached; `None` marks exhaustion.
    pub fn replace_for_in_level(
        &mut self,
        id: ObjectId,
        next_object: Option<ObjectId>,
        properties: Vec<ForInProperty>,
    ) -> Result<HeapCleanup, HeapError> {
        if !matches!(self.object(id)?.payload, ObjectPayload::ForInIterator(_)) {
            return Err(HeapError::Invariant(
                "for-in level update reached an object with the wrong class",
            ));
        }
        if let Some(object) = next_object {
            self.retain_raw(RawId::Object(object), 1)?;
        }
        let previous = {
            let ObjectPayload::ForInIterator(data) = &mut self.object_mut(id)?.payload else {
                unreachable!("for-in payload was validated before level replacement")
            };
            data.index = 0;
            data.properties = properties;
            data.fast_array = false;
            data.array_count = 0;
            std::mem::replace(&mut data.object, next_object)
        };
        if let Some(object) = previous {
            self.release_raw_no_drain(RawId::Object(object))?;
        }
        self.drain_zero_queue()
    }
}
