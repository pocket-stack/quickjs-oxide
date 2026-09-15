//! Live Map/Set records indexed by key and by monotonic insertion identity.
//!
//! Records alone own keys, values and GC edges. Record IDs are never reused,
//! even after clear, so paused cursors need no deleted slots or observer leases.
//! The ordered tree contains only live IDs; key and record lookup are average
//! O(1), while insertion, deletion and finding the next live ID are O(log size).
//! Heap collection transactions retain/release edges around these pure mutations.

use std::collections::{BTreeSet, HashMap, btree_set};

use super::collection_index::CollectionIndex;
use super::{HeapError, RawValue};

#[derive(Clone, Debug, PartialEq)]
pub struct MapRecord {
    pub key: RawValue,
    pub value: RawValue,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CollectionRecords {
    entries: HashMap<usize, MapRecord>,
    order: BTreeSet<usize>,
    key_index: CollectionIndex,
    next_id: usize,
}

impl CollectionRecords {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Exclusive upper bound of issued IDs, independent of current live size.
    pub fn next_id(&self) -> usize {
        self.next_id
    }

    pub fn get(&self, id: usize) -> Option<&MapRecord> {
        self.entries.get(&id)
    }

    #[cfg(test)]
    pub(super) fn get_mut(&mut self, id: usize) -> Option<&mut MapRecord> {
        self.entries.get_mut(&id)
    }

    /// Value replacement cannot invalidate either key or insertion indexes.
    pub(super) fn replace_value(&mut self, id: usize, value: RawValue) -> Option<RawValue> {
        self.entries
            .get_mut(&id)
            .map(|record| std::mem::replace(&mut record.value, value))
    }

    pub fn ids(&self) -> impl DoubleEndedIterator<Item = usize> + ExactSizeIterator + '_ {
        self.order.iter().copied()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &MapRecord> + ExactSizeIterator {
        self.order.iter().map(|id| &self.entries[id])
    }

    pub fn next_at_or_after(&self, cursor: usize) -> Option<(usize, &MapRecord)> {
        let &id = self.order.range(cursor..).next()?;
        Some((id, &self.entries[&id]))
    }

    pub(super) fn find(&self, key: &RawValue) -> Option<usize> {
        self.key_index.find(self, key)
    }

    /// Must run before retaining a new record's heap edges.
    pub(super) fn preflight_insert(&self) -> Result<(), HeapError> {
        self.next_id.checked_add(1).ok_or(HeapError::Overflow {
            operation: "advancing collection record identity",
        })?;
        Ok(())
    }

    /// The heap has validated the key, uniqueness and ID capacity before commit.
    pub(super) fn insert(&mut self, record: MapRecord) -> usize {
        let id = self.next_id;
        self.next_id = id
            .checked_add(1)
            .expect("collection insertion was preflighted");
        let key = &record.key;
        self.key_index.insert(key, id);
        assert!(
            self.entries.insert(id, record).is_none(),
            "collection record ID was reused"
        );
        assert!(self.order.insert(id), "collection ordered ID was reused");
        id
    }

    pub(super) fn remove(&mut self, id: usize) -> Option<MapRecord> {
        let record = self.entries.remove(&id)?;
        self.key_index.remove(&record.key, id);
        assert!(
            self.order.remove(&id),
            "live collection record has no ordered ID"
        );
        // Geometric shrinking bounds retained table capacity during churn,
        // without a rebuild for each deletion. The tree releases removed nodes.
        let size = self.entries.len();
        if self.entries.capacity() > size.saturating_mul(4).saturating_add(64) {
            self.entries
                .shrink_to(size.saturating_mul(2).saturating_add(32));
        }
        Some(record)
    }

    /// Transfer all live records in insertion order, preserving the ID clock.
    pub(super) fn take_all(&mut self) -> CollectionRecordsIntoIter {
        self.key_index.clear();
        CollectionRecordsIntoIter {
            entries: std::mem::take(&mut self.entries),
            order: std::mem::take(&mut self.order).into_iter(),
        }
    }

    pub(super) fn validate(&self) -> Result<(), HeapError> {
        if self.entries.len() != self.order.len()
            || self
                .order
                .iter()
                .any(|id| *id >= self.next_id || !self.entries.contains_key(id))
        {
            return Err(HeapError::Invariant(
                "collection record IDs do not match live storage",
            ));
        }
        self.key_index.validate(self)
    }
}

/// Ordered ownership transfer for clear; no record snapshot.
pub struct CollectionRecordsIntoIter {
    entries: HashMap<usize, MapRecord>,
    order: btree_set::IntoIter<usize>,
}

impl Iterator for CollectionRecordsIntoIter {
    type Item = MapRecord;

    fn next(&mut self) -> Option<Self::Item> {
        let id = self.order.next()?;
        Some(
            self.entries
                .remove(&id)
                .expect("ordered collection record exists"),
        )
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.order.size_hint()
    }
}

impl ExactSizeIterator for CollectionRecordsIntoIter {}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(key: i32, value: i32) -> MapRecord {
        MapRecord {
            key: RawValue::Int(key),
            value: RawValue::Int(value),
        }
    }

    #[test]
    fn collection_record_storage_reclaims_capacity_and_keeps_id_clock() {
        for peak in [64, 1024, 16384] {
            let mut records = CollectionRecords::default();
            for key in 0..peak {
                records.insert(record(key as i32, key as i32));
            }
            for id in 0..peak - 1 {
                records.remove(id).unwrap();
            }
            let (buckets, candidates) = records.key_index.retained_capacities();
            assert_eq!(records.len(), 1);
            assert_eq!(records.order.len(), 1);
            assert!(records.entries.capacity() <= 68);
            assert!(buckets <= 68);
            assert!(candidates <= 68);
            println!(
                "peak={peak} live=1 record_capacity={} bucket_capacity={buckets} candidate_capacity={candidates}",
                records.entries.capacity()
            );
            assert_eq!(records.take_all().count(), 1);
            assert_eq!(records.entries.capacity(), 0);
            assert_eq!(records.key_index.retained_capacities(), (0, 0));
            assert_eq!(records.insert(record(1, 2)), peak);
        }
    }

    #[test]
    fn collection_record_storage_matches_a_tombstone_reference_model() {
        let mut records = CollectionRecords::default();
        let mut model: Vec<Option<(i32, i32)>> = Vec::new();
        let mut seed = 0x8cae_7753_u64;
        let mut cursor = 0;
        for step in 0..4096 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let key = ((seed >> 32) % 31) as i32;
            let existing = model
                .iter()
                .position(|entry| entry.is_some_and(|(k, _)| k == key));
            assert_eq!(records.find(&RawValue::Int(key)), existing);
            match (seed >> 16) % 7 {
                0..=2 => {
                    if let Some(id) = existing {
                        records.replace_value(id, RawValue::Int(step)).unwrap();
                        model[id] = Some((key, step));
                    } else {
                        records.preflight_insert().unwrap();
                        assert_eq!(records.insert(record(key, step)), model.len());
                        model.push(Some((key, step)));
                    }
                }
                3 => {
                    if let Some(id) = existing {
                        records.remove(id).unwrap();
                        model[id] = None;
                    }
                }
                4 => {
                    let expected = model
                        .iter()
                        .enumerate()
                        .skip(cursor)
                        .find_map(|(id, entry)| entry.map(|pair| (id, pair)));
                    let actual = records.next_at_or_after(cursor);
                    assert_eq!(actual.map(|(id, _)| id), expected.map(|(id, _)| id));
                    if let Some((id, _)) = actual {
                        cursor = id + 1;
                    }
                }
                5 => cursor = 0,
                _ => {
                    let expected = model.iter().flatten().count();
                    assert_eq!(records.take_all().count(), expected);
                    model.fill(None);
                }
            }
            records.validate().unwrap();
            assert_eq!(records.len(), model.iter().flatten().count());
            assert_eq!(records.next_id(), model.len());
            let actual = records
                .iter()
                .map(|entry| (&entry.key, &entry.value))
                .collect::<Vec<_>>();
            let expected = model
                .iter()
                .flatten()
                .map(|&(key, value)| record(key, value))
                .collect::<Vec<_>>();
            assert_eq!(
                actual,
                expected
                    .iter()
                    .map(|entry| (&entry.key, &entry.value))
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn collection_record_identity_exhaustion_rejects_before_mutation() {
        let mut records = CollectionRecords::default();
        records.next_id = usize::MAX;
        assert!(matches!(
            records.preflight_insert(),
            Err(HeapError::Overflow { .. })
        ));
        assert!(records.is_empty());
    }
}
