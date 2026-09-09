//! Insertion-ordered Map/Set storage, weak collection records, and collection iterator state.
//!
//! The runtime owns observable key comparison and iterator protocol behavior.
//! These operations maintain records and transfer or release their owned heap edges.

use super::{
    Atom, Heap, HeapCleanup, HeapError, MapIteratorKind, NodeData, ObjectData, ObjectId,
    ObjectPayload, RawId, RawValue, SetIteratorKind, SlotState, is_map_storable_value,
    raw_value_atom, raw_value_edges,
};
use std::collections::{HashMap, HashSet};

/// One stable insertion-order slot in a genuine Map.
///
/// Deletion clears `key` and resets `value` to `undefined` rather than
/// removing the record. Stable indices are observable through live Map
/// iterators: deleting and re-adding a key appends a fresh record, and records
/// appended after iterator creation remain visible until exhaustion.
#[derive(Clone, Debug, PartialEq)]
pub struct MapRecord {
    pub key: Option<RawValue>,
    pub value: RawValue,
}

/// Printer-only snapshot of Map/Set records retained by live iterators.
///
/// The heap arena is intentionally scanned once for both collection classes.
/// Grouping by source object lets one side-effect-free diagnostic traversal
/// reuse the result for every nested Map and Set it renders.
#[derive(Debug, Default)]
pub(crate) struct CollectionIteratorCurrentIndices {
    maps: HashMap<ObjectId, HashSet<usize>>,
    sets: HashMap<ObjectId, HashSet<usize>>,
}

impl CollectionIteratorCurrentIndices {
    pub(crate) fn map(&self, source: ObjectId) -> Option<&HashSet<usize>> {
        self.maps.get(&source)
    }

    pub(crate) fn set(&self, source: ObjectId) -> Option<&HashSet<usize>> {
        self.sets.get(&source)
    }
}

/// Non-owning key identity stored by WeakMap and WeakSet.
///
/// Copying or storing this value deliberately retains neither an arena object
/// nor an AtomTable entry. Object generations prevent a reclaimed slot from
/// aliasing a later allocation; symbol generations provide the same property
/// at the AtomTable boundary. The runtime admits only ECMAScript-valid weak
/// targets (objects and non-registered symbols) before constructing a key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WeakCollectionKey {
    Object(ObjectId),
    Symbol(Atom),
}

/// One hash-indexed weak-collection record with intrusive insertion-order
/// links. Keys are copied identities and therefore do not own arena or atom
/// references; `value` carries the WeakMap value (or `()` for WeakSet).
#[derive(Clone, Debug, PartialEq)]
struct WeakCollectionRecord<V> {
    value: V,
    prev: Option<WeakCollectionKey>,
    next: Option<WeakCollectionKey>,
}

/// O(1) weak-record storage in QuickJS insertion order.
///
/// Unlike ordinary Map, weak collections expose no live iterator and need no
/// tombstones. Deletion unlinks and removes the hash entry, while re-adding
/// the same identity appends a fresh record at the tail.
#[derive(Clone, Debug, PartialEq)]
pub struct WeakCollectionRecords<V> {
    entries: HashMap<WeakCollectionKey, WeakCollectionRecord<V>>,
    pub(super) head: Option<WeakCollectionKey>,
    pub(super) tail: Option<WeakCollectionKey>,
}

impl<V> Default for WeakCollectionRecords<V> {
    fn default() -> Self {
        Self::new()
    }
}

impl<V> WeakCollectionRecords<V> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            head: None,
            tail: None,
        }
    }

    #[must_use]
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn contains_key(&self, key: &WeakCollectionKey) -> bool {
        self.entries.contains_key(key)
    }

    pub(super) fn get(&self, key: &WeakCollectionKey) -> Option<&V> {
        self.entries.get(key).map(|record| &record.value)
    }

    pub(super) fn values(&self) -> impl Iterator<Item = &V> {
        self.entries.values().map(|record| &record.value)
    }

    pub(super) fn keys(&self) -> impl Iterator<Item = &WeakCollectionKey> {
        self.entries.keys()
    }

    pub(super) fn first_key(&self) -> Option<WeakCollectionKey> {
        self.head
    }

    pub(super) fn next_key(
        &self,
        key: WeakCollectionKey,
    ) -> Result<Option<WeakCollectionKey>, HeapError> {
        self.entries
            .get(&key)
            .map(|record| record.next)
            .ok_or(HeapError::Invariant(
                "weak-record traversal referenced a missing entry",
            ))
    }

    pub(super) fn try_reserve_one(&mut self, operation: &'static str) -> Result<(), HeapError> {
        self.entries
            .try_reserve(1)
            .map_err(|_| HeapError::Allocation { operation })
    }

    fn validate_new_insertion(&self, key: WeakCollectionKey) -> Result<(), HeapError> {
        if self.entries.contains_key(&key) {
            return Err(HeapError::Invariant(
                "weak-record insertion duplicated a live key",
            ));
        }
        if self.head.is_none() != self.tail.is_none() {
            return Err(HeapError::Invariant(
                "weak-record list endpoints disagreed before insertion",
            ));
        }
        if self.entries.is_empty() != self.head.is_none() {
            return Err(HeapError::Invariant(
                "weak-record hash occupancy disagreed with its endpoints",
            ));
        }
        let previous = self.tail;
        if let Some(previous) = previous {
            let record = self.entries.get(&previous).ok_or(HeapError::Invariant(
                "weak-record tail referenced a missing entry",
            ))?;
            if record.next.is_some() {
                return Err(HeapError::Invariant(
                    "weak-record tail had a successor before insertion",
                ));
            }
        }
        Ok(())
    }

    fn insert_new(&mut self, key: WeakCollectionKey, value: V) {
        debug_assert!(self.validate_new_insertion(key).is_ok());
        let previous = self.tail;
        self.entries.insert(
            key,
            WeakCollectionRecord {
                value,
                prev: previous,
                next: None,
            },
        );
        if let Some(previous) = previous {
            self.entries
                .get_mut(&previous)
                .expect("validated weak-record tail disappeared")
                .next = Some(key);
        } else {
            self.head = Some(key);
        }
        self.tail = Some(key);
    }

    fn replace(&mut self, key: WeakCollectionKey, value: V) -> V {
        let record = self
            .entries
            .get_mut(&key)
            .expect("validated weak-record replacement key disappeared");
        std::mem::replace(&mut record.value, value)
    }

    pub(super) fn remove(&mut self, key: &WeakCollectionKey) -> Result<Option<V>, HeapError> {
        let Some(record) = self.entries.get(key) else {
            return Ok(None);
        };
        let (previous, next) = (record.prev, record.next);
        match previous {
            Some(previous) => {
                if self.head == Some(*key)
                    || self
                        .entries
                        .get(&previous)
                        .is_none_or(|record| record.next != Some(*key))
                {
                    return Err(HeapError::Invariant(
                        "weak-record predecessor was inconsistent",
                    ));
                }
            }
            None if self.head != Some(*key) => {
                return Err(HeapError::Invariant(
                    "headless weak record was not the list head",
                ));
            }
            None => {}
        }
        match next {
            Some(next) => {
                if self.tail == Some(*key)
                    || self
                        .entries
                        .get(&next)
                        .is_none_or(|record| record.prev != Some(*key))
                {
                    return Err(HeapError::Invariant(
                        "weak-record successor was inconsistent",
                    ));
                }
            }
            None if self.tail != Some(*key) => {
                return Err(HeapError::Invariant(
                    "tailless weak record was not the list tail",
                ));
            }
            None => {}
        }

        let record = self
            .entries
            .remove(key)
            .expect("validated weak record disappeared before removal");
        if let Some(previous) = previous {
            self.entries
                .get_mut(&previous)
                .expect("validated weak-record predecessor disappeared")
                .next = next;
        } else {
            self.head = next;
        }
        if let Some(next) = next {
            self.entries
                .get_mut(&next)
                .expect("validated weak-record successor disappeared")
                .prev = previous;
        } else {
            self.tail = previous;
        }
        Ok(Some(record.value))
    }

    pub(super) fn validate_order(&self) -> Result<(), HeapError> {
        if self.head.is_none() != self.tail.is_none() {
            return Err(HeapError::Invariant("weak-record list endpoints disagreed"));
        }
        if self.entries.is_empty() != self.head.is_none() {
            return Err(HeapError::Invariant(
                "weak-record hash occupancy disagreed with its endpoints",
            ));
        }
        let mut current = self.head;
        let mut previous = None;
        let mut visited = 0usize;
        while let Some(key) = current {
            if visited == self.entries.len() {
                return Err(HeapError::Invariant(
                    "weak-record insertion-order list contained a cycle",
                ));
            }
            let record = self.entries.get(&key).ok_or(HeapError::Invariant(
                "weak-record insertion-order list referenced a missing entry",
            ))?;
            if record.prev != previous {
                return Err(HeapError::Invariant(
                    "weak-record insertion-order predecessor was inconsistent",
                ));
            }
            previous = Some(key);
            current = record.next;
            visited += 1;
        }
        if previous != self.tail || visited != self.entries.len() {
            return Err(HeapError::Invariant(
                "weak-record insertion-order list did not cover its hash index",
            ));
        }
        Ok(())
    }
}

impl Heap {
    /// Borrow the stable insertion-order record array of one genuine Map.
    /// Tombstones remain present with a `None` key and `undefined` value.
    pub fn map_records(&self, id: ObjectId) -> Result<&[MapRecord], HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Map { records, .. } => Ok(records),
            _ => Err(HeapError::Invariant(
                "Map records requested for an object with the wrong class",
            )),
        }
    }

    /// Read the number of live records in one genuine Map.
    pub fn map_size(&self, id: ObjectId) -> Result<usize, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Map { size, .. } => Ok(*size),
            _ => Err(HeapError::Invariant(
                "Map size requested for an object with the wrong class",
            )),
        }
    }

    /// Append a new live Map record after the caller has established that no
    /// SameValueZero-equal live key exists. Object edges are retained before
    /// publication. Symbol atom references must already be owned by the
    /// caller and transfer to the Map only when this operation succeeds.
    pub fn map_insert_record(
        &mut self,
        id: ObjectId,
        key: RawValue,
        value: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&key) || !is_map_storable_value(&value) {
            return Err(HeapError::Invariant(
                "Map record contains an internal value sentinel",
            ));
        }
        let next_size = match &self.object(id)?.payload {
            ObjectPayload::Map { size, .. } => size.checked_add(1).ok_or(HeapError::Overflow {
                operation: "growing Map size",
            })?,
            _ => {
                return Err(HeapError::Invariant(
                    "Map insertion reached an object with the wrong class",
                ));
            }
        };

        let mut new_edges = raw_value_edges(&key);
        new_edges.extend(raw_value_edges(&value));
        self.retain_edges_transactionally(&new_edges)?;

        let ObjectPayload::Map {
            records,
            live_indices,
            size,
        } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("Map payload was validated before retaining record edges")
        };
        let record_index = records.len();
        records.push(MapRecord {
            key: Some(key),
            value,
        });
        let inserted = live_indices.insert(record_index);
        debug_assert!(inserted, "fresh Map record index was already live");
        *size = next_size;
        Ok(HeapCleanup::default())
    }

    /// Replace the value of a caller-resolved live Map record. The replacement
    /// edge is retained before the previous value is detached. A replacement
    /// Symbol atom transfers on success; the previous Symbol atom is returned
    /// through [`HeapCleanup::atoms`].
    pub fn map_replace_record_value(
        &mut self,
        id: ObjectId,
        index: usize,
        value: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&value) {
            return Err(HeapError::Invariant(
                "Map record contains an internal value sentinel",
            ));
        }
        match &self.object(id)?.payload {
            ObjectPayload::Map { records, .. }
                if records
                    .get(index)
                    .is_some_and(|record| record.key.is_some()) => {}
            ObjectPayload::Map { .. } => {
                return Err(HeapError::Invariant(
                    "Map value replacement requires a live record index",
                ));
            }
            _ => {
                return Err(HeapError::Invariant(
                    "Map value replacement reached an object with the wrong class",
                ));
            }
        }

        let new_edges = raw_value_edges(&value);
        self.retain_edges_transactionally(&new_edges)?;
        let previous = {
            let ObjectPayload::Map { records, .. } = &mut self.object_mut(id)?.payload else {
                unreachable!("Map payload was validated before retaining replacement edges")
            };
            std::mem::replace(&mut records[index].value, value)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&previous));
        for edge in raw_value_edges(&previous) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Turn a caller-resolved live Map record into a tombstone without
    /// changing stable record indices. Owned key/value edges and Symbol atoms
    /// are detached together.
    pub fn map_delete_record(
        &mut self,
        id: ObjectId,
        index: usize,
    ) -> Result<HeapCleanup, HeapError> {
        let (key, value) = {
            let ObjectPayload::Map {
                records,
                live_indices,
                size,
            } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Map deletion reached an object with the wrong class",
                ));
            };
            if *size == 0 {
                return Err(HeapError::Invariant(
                    "Map deletion requires a live record index",
                ));
            }
            let record = records.get_mut(index).ok_or(HeapError::Invariant(
                "Map deletion requires a live record index",
            ))?;
            if record.key.is_none() || !live_indices.remove(&index) {
                return Err(HeapError::Invariant(
                    "Map deletion requires a live record index",
                ));
            }
            let key = record.key.take().ok_or(HeapError::Invariant(
                "Map deletion requires a live record index",
            ))?;
            let value = std::mem::replace(&mut record.value, RawValue::Undefined);
            *size -= 1;
            (key, value)
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&key));
        cleanup.atoms.extend(raw_value_atom(&value));
        for edge in raw_value_edges(&key)
            .into_iter()
            .chain(raw_value_edges(&value))
        {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Tombstone every live Map record while preserving the record array for
    /// existing iterators. All detached edges and Symbol atoms are finalized
    /// after the object mutation has completed.
    pub fn map_clear(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let removed = {
            let ObjectPayload::Map {
                records,
                live_indices,
                size,
            } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Map clear reached an object with the wrong class",
                ));
            };
            let mut removed = Vec::with_capacity(*size);
            for record in records {
                if let Some(key) = record.key.take() {
                    let value = std::mem::replace(&mut record.value, RawValue::Undefined);
                    removed.push((key, value));
                }
            }
            live_indices.clear();
            *size = 0;
            removed
        };

        let mut cleanup = HeapCleanup::default();
        for (key, value) in removed {
            cleanup.atoms.extend(raw_value_atom(&key));
            cleanup.atoms.extend(raw_value_atom(&value));
            for edge in raw_value_edges(&key)
                .into_iter()
                .chain(raw_value_edges(&value))
            {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Snapshot one branded Map Iterator's live source, stable record cursor,
    /// and result projection.
    #[cfg(test)]
    pub fn map_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, usize, MapIteratorKind), HeapError> {
        let ObjectPayload::MapIterator {
            object,
            next_index,
            kind,
            ..
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Map Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*object, *next_index, *kind))
    }

    /// Begin one Map Iterator `next()` call and release the record retained by
    /// its preceding successful step. The stable cursor itself is unchanged.
    pub fn begin_map_iterator_next(
        &mut self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, usize, MapIteratorKind), HeapError> {
        let ObjectPayload::MapIterator {
            object,
            next_index,
            current_index,
            kind,
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Map Iterator state reached an object with the wrong class",
            ));
        };
        *current_index = None;
        Ok((*object, *next_index, *kind))
    }

    /// Advance a live Map Iterator to the next stable record index. The source
    /// edge remains retained so later record appends are visible.
    pub fn set_map_iterator_index(
        &mut self,
        id: ObjectId,
        next_index: usize,
    ) -> Result<(), HeapError> {
        let ObjectPayload::MapIterator {
            object,
            next_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Map Iterator advance reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant("completed Map Iterator was advanced"));
        }
        *stored = next_index;
        Ok(())
    }

    /// Retain the live Map record returned by the current iterator step until
    /// the next call begins or the iterator is exhausted.
    pub fn set_map_iterator_current(
        &mut self,
        id: ObjectId,
        current_index: usize,
    ) -> Result<(), HeapError> {
        let ObjectPayload::MapIterator {
            object,
            next_index,
            current_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Map Iterator current record reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant(
                "completed Map Iterator retained a record",
            ));
        }
        if current_index >= *next_index {
            return Err(HeapError::Invariant(
                "Map Iterator retained a record beyond its cursor",
            ));
        }
        *stored = Some(current_index);
        Ok(())
    }

    /// Permanently detach an exhausted Map Iterator source and release its
    /// owned object edge. Repeated completion is idempotent.
    pub fn finish_map_iterator(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let source = {
            let ObjectPayload::MapIterator {
                object,
                current_index,
                ..
            } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Map Iterator completion reached an object with the wrong class",
                ));
            };
            *current_index = None;
            object.take()
        };
        let Some(source) = source else {
            return Ok(HeapCleanup::default());
        };
        self.release_raw_no_drain(RawId::Object(source))?;
        self.drain_zero_queue()
    }

    /// Borrow the stable insertion-order record array of one genuine Set.
    /// Live elements occupy `key`; both live and tombstoned `value` slots are
    /// always `undefined`.
    pub fn set_records(&self, id: ObjectId) -> Result<&[MapRecord], HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Set { records, .. } => Ok(records),
            _ => Err(HeapError::Invariant(
                "Set records requested for an object with the wrong class",
            )),
        }
    }

    /// Read the number of live records in one genuine Set.
    pub fn set_size(&self, id: ObjectId) -> Result<usize, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::Set { size, .. } => Ok(*size),
            _ => Err(HeapError::Invariant(
                "Set size requested for an object with the wrong class",
            )),
        }
    }

    /// Append a new live Set record after the caller has established that no
    /// SameValueZero-equal element exists. Object edges are retained before
    /// publication. A Symbol atom must already be owned by the caller and
    /// transfers to the Set only when this operation succeeds.
    pub fn set_insert_record(
        &mut self,
        id: ObjectId,
        key: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&key) {
            return Err(HeapError::Invariant(
                "Set record contains an internal value sentinel",
            ));
        }
        let next_size = match &self.object(id)?.payload {
            ObjectPayload::Set { size, .. } => size.checked_add(1).ok_or(HeapError::Overflow {
                operation: "growing Set size",
            })?,
            _ => {
                return Err(HeapError::Invariant(
                    "Set insertion reached an object with the wrong class",
                ));
            }
        };

        let new_edges = raw_value_edges(&key);
        self.retain_edges_transactionally(&new_edges)?;

        let ObjectPayload::Set {
            records,
            live_indices,
            size,
        } = &mut self.object_mut(id)?.payload
        else {
            unreachable!("Set payload was validated before retaining record edges")
        };
        let record_index = records.len();
        records.push(MapRecord {
            key: Some(key),
            value: RawValue::Undefined,
        });
        let inserted = live_indices.insert(record_index);
        debug_assert!(inserted, "fresh Set record index was already live");
        *size = next_size;
        Ok(HeapCleanup::default())
    }

    /// Turn a caller-resolved live Set record into a tombstone without
    /// changing stable record indices. The owned element edge and Symbol atom
    /// are detached together.
    pub fn set_delete_record(
        &mut self,
        id: ObjectId,
        index: usize,
    ) -> Result<HeapCleanup, HeapError> {
        let key = {
            let ObjectPayload::Set {
                records,
                live_indices,
                size,
            } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Set deletion reached an object with the wrong class",
                ));
            };
            if *size == 0 {
                return Err(HeapError::Invariant(
                    "Set deletion requires a live record index",
                ));
            }
            let record = records.get_mut(index).ok_or(HeapError::Invariant(
                "Set deletion requires a live record index",
            ))?;
            if !matches!(record.value, RawValue::Undefined) {
                return Err(HeapError::Invariant(
                    "Set record value slot is not undefined",
                ));
            }
            if record.key.is_none() || !live_indices.remove(&index) {
                return Err(HeapError::Invariant(
                    "Set deletion requires a live record index",
                ));
            }
            let key = record.key.take().ok_or(HeapError::Invariant(
                "Set deletion requires a live record index",
            ))?;
            *size -= 1;
            key
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&key));
        for edge in raw_value_edges(&key) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Tombstone every live Set record while preserving stable indices for
    /// existing iterators. Detached edges and Symbol atoms are finalized only
    /// after the payload mutation completes.
    pub fn set_clear(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let removed = {
            let ObjectPayload::Set {
                records,
                live_indices,
                size,
            } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Set clear reached an object with the wrong class",
                ));
            };
            if records
                .iter()
                .any(|record| !matches!(record.value, RawValue::Undefined))
            {
                return Err(HeapError::Invariant(
                    "Set record value slot is not undefined",
                ));
            }
            let mut removed = Vec::with_capacity(*size);
            for record in records {
                if let Some(key) = record.key.take() {
                    removed.push(key);
                }
            }
            live_indices.clear();
            *size = 0;
            removed
        };

        let mut cleanup = HeapCleanup::default();
        for key in removed {
            cleanup.atoms.extend(raw_value_atom(&key));
            for edge in raw_value_edges(&key) {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Look up one genuine WeakMap value by its non-owning identity key.
    pub fn weak_map_get(
        &self,
        id: ObjectId,
        key: WeakCollectionKey,
    ) -> Result<Option<&RawValue>, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::WeakMap { records } => Ok(records.get(&key)),
            _ => Err(HeapError::Invariant(
                "WeakMap lookup reached an object with the wrong class",
            )),
        }
    }

    /// Insert or replace one WeakMap entry in expected constant time. The key
    /// is deliberately not retained. The replacement value is retained before
    /// the old value is detached, and an owned Symbol atom transfers only on
    /// success.
    pub fn weak_map_set(
        &mut self,
        id: ObjectId,
        key: WeakCollectionKey,
        value: RawValue,
    ) -> Result<HeapCleanup, HeapError> {
        if !is_map_storable_value(&value) {
            return Err(HeapError::Invariant(
                "WeakMap record contains an internal value sentinel",
            ));
        }
        if let WeakCollectionKey::Object(key) = key {
            self.object(key)?;
        }
        let is_new = match &self.object(id)?.payload {
            ObjectPayload::WeakMap { records } => !records.contains_key(&key),
            _ => {
                return Err(HeapError::Invariant(
                    "WeakMap update reached an object with the wrong class",
                ));
            }
        };

        // Match QuickJS's catchable allocation boundary: a genuinely new
        // entry reserves its table slot before retaining or publishing the
        // value. Replacements require no growth and therefore do not reserve.
        if is_new {
            let ObjectPayload::WeakMap { records } = &mut self.object_mut(id)?.payload else {
                unreachable!("WeakMap payload was validated before reserving its entry")
            };
            records.validate_new_insertion(key)?;
            records.try_reserve_one("growing WeakMap storage")?;
        }

        self.retain_edges_transactionally(&raw_value_edges(&value))?;
        let previous = {
            let ObjectPayload::WeakMap { records } = &mut self.object_mut(id)?.payload else {
                unreachable!("WeakMap payload was validated before retaining its value")
            };
            if is_new {
                records.insert_new(key, value);
                None
            } else {
                Some(records.replace(key, value))
            }
        };

        let mut cleanup = HeapCleanup::default();
        if let Some(previous) = previous {
            cleanup.atoms.extend(raw_value_atom(&previous));
            for edge in raw_value_edges(&previous) {
                self.release_raw_no_drain(edge)?;
            }
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok(cleanup)
    }

    /// Delete one WeakMap key in expected constant time and release its value.
    pub fn weak_map_delete(
        &mut self,
        id: ObjectId,
        key: WeakCollectionKey,
    ) -> Result<(bool, HeapCleanup), HeapError> {
        let value = match &mut self.object_mut(id)?.payload {
            ObjectPayload::WeakMap { records } => records.remove(&key)?,
            _ => {
                return Err(HeapError::Invariant(
                    "WeakMap deletion reached an object with the wrong class",
                ));
            }
        };
        let Some(value) = value else {
            return Ok((false, HeapCleanup::default()));
        };

        let mut cleanup = HeapCleanup::default();
        cleanup.atoms.extend(raw_value_atom(&value));
        for edge in raw_value_edges(&value) {
            self.release_raw_no_drain(edge)?;
        }
        cleanup.merge(self.drain_zero_queue()?);
        Ok((true, cleanup))
    }

    /// Test WeakSet membership by identity in expected constant time.
    pub fn weak_set_has(&self, id: ObjectId, key: WeakCollectionKey) -> Result<bool, HeapError> {
        match &self.object(id)?.payload {
            ObjectPayload::WeakSet { records } => Ok(records.contains_key(&key)),
            _ => Err(HeapError::Invariant(
                "WeakSet lookup reached an object with the wrong class",
            )),
        }
    }

    /// Add one non-owning WeakSet key in expected constant time. No arena or
    /// atom ownership transfers. The result reports whether it was new.
    pub fn weak_set_add(
        &mut self,
        id: ObjectId,
        key: WeakCollectionKey,
    ) -> Result<bool, HeapError> {
        if let WeakCollectionKey::Object(key) = key {
            self.object(key)?;
        }
        let is_new = match &self.object(id)?.payload {
            ObjectPayload::WeakSet { records } => !records.contains_key(&key),
            _ => {
                return Err(HeapError::Invariant(
                    "WeakSet insertion reached an object with the wrong class",
                ));
            }
        };
        if !is_new {
            return Ok(false);
        }
        let ObjectPayload::WeakSet { records } = &mut self.object_mut(id)?.payload else {
            unreachable!("WeakSet payload was validated before reserving its entry")
        };
        records.validate_new_insertion(key)?;
        records.try_reserve_one("growing WeakSet storage")?;
        records.insert_new(key, ());
        Ok(true)
    }

    /// Delete one WeakSet key in expected constant time. The result reports
    /// whether an entry was present.
    pub fn weak_set_delete(
        &mut self,
        id: ObjectId,
        key: WeakCollectionKey,
    ) -> Result<bool, HeapError> {
        match &mut self.object_mut(id)?.payload {
            ObjectPayload::WeakSet { records } => Ok(records.remove(&key)?.is_some()),
            _ => Err(HeapError::Invariant(
                "WeakSet deletion reached an object with the wrong class",
            )),
        }
    }

    /// Snapshot one branded Set Iterator's live source, stable record cursor,
    /// and result projection.
    #[cfg(test)]
    pub fn set_iterator_state(
        &self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, usize, SetIteratorKind), HeapError> {
        let ObjectPayload::SetIterator {
            object,
            next_index,
            kind,
            ..
        } = &self.object(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Set Iterator state reached an object with the wrong class",
            ));
        };
        Ok((*object, *next_index, *kind))
    }

    /// Begin one Set Iterator `next()` call and release the record retained by
    /// its preceding successful step. The stable cursor itself is unchanged.
    pub fn begin_set_iterator_next(
        &mut self,
        id: ObjectId,
    ) -> Result<(Option<ObjectId>, usize, SetIteratorKind), HeapError> {
        let ObjectPayload::SetIterator {
            object,
            next_index,
            current_index,
            kind,
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Set Iterator state reached an object with the wrong class",
            ));
        };
        *current_index = None;
        Ok((*object, *next_index, *kind))
    }

    /// Advance a live Set Iterator to the next stable record index. The source
    /// edge remains retained so later record appends are visible.
    pub fn set_set_iterator_index(
        &mut self,
        id: ObjectId,
        next_index: usize,
    ) -> Result<(), HeapError> {
        let ObjectPayload::SetIterator {
            object,
            next_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Set Iterator advance reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant("completed Set Iterator was advanced"));
        }
        *stored = next_index;
        Ok(())
    }

    /// Retain the live Set record returned by the current iterator step until
    /// the next call begins or the iterator is exhausted.
    pub fn set_set_iterator_current(
        &mut self,
        id: ObjectId,
        current_index: usize,
    ) -> Result<(), HeapError> {
        let ObjectPayload::SetIterator {
            object,
            next_index,
            current_index: stored,
            ..
        } = &mut self.object_mut(id)?.payload
        else {
            return Err(HeapError::Invariant(
                "Set Iterator current record reached an object with the wrong class",
            ));
        };
        if object.is_none() {
            return Err(HeapError::Invariant(
                "completed Set Iterator retained a record",
            ));
        }
        if current_index >= *next_index {
            return Err(HeapError::Invariant(
                "Set Iterator retained a record beyond its cursor",
            ));
        }
        *stored = Some(current_index);
        Ok(())
    }

    /// Permanently detach an exhausted Set Iterator source and release its
    /// owned object edge. Repeated completion is idempotent.
    pub fn finish_set_iterator(&mut self, id: ObjectId) -> Result<HeapCleanup, HeapError> {
        let source = {
            let ObjectPayload::SetIterator {
                object,
                current_index,
                ..
            } = &mut self.object_mut(id)?.payload
            else {
                return Err(HeapError::Invariant(
                    "Set Iterator completion reached an object with the wrong class",
                ));
            };
            *current_index = None;
            object.take()
        };
        let Some(source) = source else {
            return Ok(HeapCleanup::default());
        };
        self.release_raw_no_drain(RawId::Object(source))?;
        self.drain_zero_queue()
    }

    /// Snapshot every stable source record currently retained by a live Map or
    /// Set Iterator. A qjs value printer builds this index lazily, then reuses
    /// it for all collections reached during the same bounded traversal.
    pub(crate) fn collection_iterator_current_indices(&self) -> CollectionIteratorCurrentIndices {
        let mut indices = CollectionIteratorCurrentIndices::default();
        for slot in &self.slots {
            let SlotState::Live(node) = &slot.state else {
                continue;
            };
            match &node.data {
                NodeData::Object(ObjectData {
                    payload:
                        ObjectPayload::MapIterator {
                            object: Some(source),
                            current_index: Some(current),
                            ..
                        },
                    ..
                }) => {
                    indices.maps.entry(*source).or_default().insert(*current);
                }
                NodeData::Object(ObjectData {
                    payload:
                        ObjectPayload::SetIterator {
                            object: Some(source),
                            current_index: Some(current),
                            ..
                        },
                    ..
                }) => {
                    indices.sets.entry(*source).or_default().insert(*current);
                }
                _ => {}
            }
        }
        indices
    }
}
