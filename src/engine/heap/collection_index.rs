//! Non-owning key lookup for insertion-ordered strong collections.
//!
//! Records own keys and GC edges; this index owns only hashes and record IDs.
//! The collection storage boundary maintains both together. Lookups borrow
//! records without rooting every candidate or invoking JavaScript. Public
//! iterator cursors remain the responsibility of ordered record storage.
//!
//! The string-hash memo is keyed by generational `StringId`: a live record
//! owns its key node, so the identity cannot be reclaimed or aliased while
//! cached, and slot reuse always changes the generation.

use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use super::{CollectionRecords, Heap, HeapError, RawValue, StringId};
use crate::engine::value::collection_key;

#[derive(Clone, Default)]
pub struct CollectionIndex {
    buckets: HashMap<u64, Vec<usize>, crate::engine::hash::IdentityBuildHasher>,
    key_hasher: std::collections::hash_map::RandomState,
    // Bounded hash memo keyed by generational string identity, scoped to the
    // key hasher seed. Non-owning: record keys keep their nodes alive.
    string_hashes: std::cell::RefCell<Option<Box<StringHashCache>>>,
    #[cfg(test)]
    hash_computations: std::cell::Cell<usize>,
}

const STRING_HASH_CACHE_SIZE: usize = 8;

#[derive(Clone)]
struct CachedStringHash {
    id: StringId,
    hash: u64,
}

#[derive(Clone, Default)]
struct StringHashCache {
    entries: VecDeque<CachedStringHash>,
}

impl StringHashCache {
    fn find(&self, id: StringId) -> Option<u64> {
        self.entries
            .iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.hash)
    }

    fn remember(&mut self, id: StringId, hash: u64) {
        if self.entries.len() == STRING_HASH_CACHE_SIZE {
            self.entries.pop_front();
        }
        self.entries.push_back(CachedStringHash { id, hash });
    }
}

// Cache history is observationally irrelevant and is intentionally excluded
// from metadata equality/debug output. Clone preserves both seed and cache.
impl PartialEq for CollectionIndex {
    fn eq(&self, other: &Self) -> bool {
        self.buckets == other.buckets
    }
}

impl fmt::Debug for CollectionIndex {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("CollectionIndex")
            .field("buckets", &self.buckets)
            .finish_non_exhaustive()
    }
}

impl CollectionIndex {
    pub(super) fn hash(&self, heap: &Heap, key: &RawValue) -> u64 {
        let RawValue::String(id) = key else {
            return self.hash_uncached(heap, key);
        };
        if let Some(hash) = self
            .string_hashes
            .borrow()
            .as_ref()
            .and_then(|cache| cache.find(*id))
        {
            return hash;
        }
        let hash = self.hash_uncached(heap, key);
        self.string_hashes
            .borrow_mut()
            .get_or_insert_with(Box::default)
            .remember(*id, hash);
        hash
    }

    fn hash_uncached(&self, heap: &Heap, key: &RawValue) -> u64 {
        #[cfg(test)]
        self.hash_computations.set(self.hash_computations.get() + 1);
        let mut hasher = self.key_hasher.build_hasher();
        collection_key::hash(heap, key, &mut hasher);
        hasher.finish()
    }

    pub(super) fn find(
        &self,
        heap: &Heap,
        records: &CollectionRecords,
        key: &RawValue,
    ) -> Option<usize> {
        self.buckets
            .get(&self.hash(heap, key))?
            .iter()
            .copied()
            .find(|&index| {
                collection_key::same_value_zero(
                    heap,
                    &records.get(index).expect("indexed record exists").key,
                    key,
                )
            })
    }

    pub(super) fn insert(&mut self, heap: &Heap, key: &RawValue, index: usize) -> u64 {
        let hash = self.hash(heap, key);
        self.insert_hashed(hash, index);
        hash
    }

    /// Index one record under a hash computed before the caller borrowed the
    /// record set mutably.
    pub(super) fn insert_hashed(&mut self, hash: u64, index: usize) {
        self.buckets.entry(hash).or_default().push(index);
    }

    #[cfg(test)]
    pub(super) fn remove(&mut self, heap: &Heap, key: &RawValue, index: usize) {
        self.remove_hashed(self.hash(heap, key), index);
    }

    pub(super) fn remove_hashed(&mut self, hash: u64, index: usize) {
        let bucket = self
            .buckets
            .get_mut(&hash)
            .expect("live collection key has a hash bucket");
        let position = bucket
            .iter()
            .position(|&entry| entry == index)
            .expect("live collection record is indexed");
        bucket.swap_remove(position);
        if bucket.is_empty() {
            self.buckets.remove(&hash);
        } else if bucket.capacity() > bucket.len().saturating_mul(4).saturating_add(64) {
            bucket.shrink_to(bucket.len().saturating_mul(2).saturating_add(32));
        }
        if self.buckets.capacity() > self.buckets.len().saturating_mul(4).saturating_add(64) {
            self.buckets
                .shrink_to(self.buckets.len().saturating_mul(2).saturating_add(32));
        }
    }

    pub(super) fn clear(&mut self) {
        self.buckets.clear();
        self.buckets.shrink_to_fit();
        *self.string_hashes.borrow_mut() = None;
    }

    #[cfg(test)]
    pub(super) fn retained_capacities(&self) -> (usize, usize) {
        (
            self.buckets.capacity(),
            self.buckets.values().map(Vec::capacity).sum(),
        )
    }

    /// Publication validation; never run this full scan on an ordinary lookup.
    pub(super) fn validate(&self, heap: &Heap, records: &CollectionRecords) -> Result<(), HeapError> {
        let mut seen = std::collections::HashSet::new();
        for (&hash, indices) in &self.buckets {
            if indices.is_empty() {
                return Err(HeapError::Invariant("collection index has an empty bucket"));
            }
            for &index in indices {
                let key =
                    records
                        .get(index)
                        .map(|record| &record.key)
                        .ok_or(HeapError::Invariant(
                            "collection index points outside live records",
                        ))?;
                if !seen.insert(index) || self.hash_uncached(heap, key) != hash {
                    return Err(HeapError::Invariant(
                        "collection index does not match its records",
                    ));
                }
            }
        }
        if seen.len() != records.len() {
            return Err(HeapError::Invariant(
                "collection index is missing live records",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::heap::MapRecord;

    /// Build records around string handles allocated in the heap, the way the
    /// storage boundary does.
    fn string_key(heap: &mut Heap, text: &str) -> RawValue {
        RawValue::String(
            heap.allocate_string(crate::engine::value::JsString::try_from_utf8(text).unwrap())
                .unwrap(),
        )
    }

    fn record_store(heap: &Heap, values: Vec<RawValue>) -> CollectionRecords {
        let mut records = CollectionRecords::default();
        for key in values {
            let id = records.next_id();
            records.insert(heap, MapRecord {
                key,
                value: RawValue::Undefined,
            });
            assert_eq!(id + 1, records.next_id());
        }
        records
    }

    #[test]
    fn short_string_hash_is_memoized_by_node_identity() {
        let mut heap = Heap::new();
        let key = string_key(&mut heap, "short");
        let RawValue::String(id) = key else {
            panic!("string key expected");
        };
        let index = CollectionIndex::default();
        for _ in 0..3 {
            index.hash(&heap, &key);
        }
        assert_eq!(index.hash_computations.get(), 1);
        // Equal content in a different node is a distinct identity and hashes
        // independently, but content equality still maps to the same bucket.
        let alias = string_key(&mut heap, "short");
        assert_ne!(
            match alias {
                RawValue::String(alias) => alias,
                _ => unreachable!(),
            },
            id
        );
        assert_eq!(index.hash(&heap, &key), index.hash(&heap, &alias));
        assert_eq!(index.hash_computations.get(), 2);
    }

    #[test]
    fn long_string_hash_memo_is_bounded() {
        let mut heap = Heap::new();
        let key = string_key(&mut heap, &"x".repeat(1024));
        let index = CollectionIndex::default();
        let expected = index.hash(&heap, &key);
        for _ in 0..32 {
            assert_eq!(index.hash(&heap, &key), expected);
        }
        assert_eq!(index.hash_computations.get(), 1);
        let cloned = index.clone();
        assert_eq!(cloned.hash(&heap, &key), expected);
        let independent = CollectionIndex::default();
        assert_eq!(
            independent.hash(&heap, &key),
            independent.hash_uncached(&heap, &key)
        );
        for suffix in 0..32 {
            let key = string_key(&mut heap, &("x".repeat(1024) + &suffix.to_string()));
            index.hash(&heap, &key);
        }
        assert!(
            index.string_hashes.borrow().as_ref().unwrap().entries.len() <= STRING_HASH_CACHE_SIZE
        );
    }

    #[test]
    fn equal_keys_share_hash_across_number_and_string_representations() {
        let mut heap = Heap::new();
        let index = CollectionIndex::default();
        let string_left = string_key(&mut heap, "abc");
        let string_right = string_key(&mut heap, "abc");
        let bigint_left = RawValue::BigInt(
            heap.allocate_bigint(
                crate::engine::value::bigint::JsBigInt::parse_radix(
                    "123456789012345678901234567890",
                    10,
                )
                .unwrap(),
            )
            .unwrap(),
        );
        let bigint_right = RawValue::BigInt(
            heap.allocate_bigint(
                crate::engine::value::bigint::JsBigInt::parse_radix(
                    "123456789012345678901234567890",
                    10,
                )
                .unwrap(),
            )
            .unwrap(),
        );
        // Cross-kind comparisons reject before any pair moves into the table.
        let one_string = string_key(&mut heap, "1");
        assert!(!collection_key::same_value_zero(
            &heap,
            &RawValue::Int(1),
            &one_string
        ));
        assert!(!collection_key::same_value_zero(
            &heap,
            &RawValue::Int(1),
            &bigint_left
        ));
        let pairs = [
            (RawValue::Int(0), RawValue::Float(-0.0)),
            (RawValue::Int(42), RawValue::Float(42.0)),
            (
                RawValue::Float(f64::NAN),
                RawValue::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
            ),
            (string_left, string_right),
            (bigint_left, bigint_right),
        ];
        for (left, right) in pairs {
            assert!(collection_key::same_value_zero(&heap, &left, &right));
            assert_eq!(index.hash(&heap, &left), index.hash(&heap, &right));
        }
    }

    #[test]
    fn collision_candidates_are_compared_and_removal_preserves_the_others() {
        let mut heap = Heap::new();
        let mut index = CollectionIndex::default();
        let records = record_store(&heap, vec![RawValue::Int(1), RawValue::Int(2)]);
        // Force a collision to exercise the bucket path deterministically,
        // without relying on the randomized hasher finding one naturally.
        let hash = index.hash(&heap, &RawValue::Int(2));
        index.buckets.insert(hash, vec![0, 1]);
        assert_eq!(index.find(&heap, &records, &RawValue::Int(2)), Some(1));
        index.remove(&heap, &RawValue::Int(2), 1);
        assert_eq!(index.find(&heap, &records, &RawValue::Int(2)), None);
        assert_eq!(index.buckets[&hash], vec![0]);
    }

    #[test]
    fn publication_rejects_missing_or_stale_index_entries() {
        let mut heap = Heap::new();
        let mut index = CollectionIndex::default();
        let mut records = record_store(&heap, vec![RawValue::Int(1)]);
        assert!(index.validate(&heap, &records).is_err());
        index.insert(&heap, &RawValue::Int(1), 0);
        assert!(index.validate(&heap, &records).is_ok());
        records.get_mut(0).unwrap().key = RawValue::Int(2);
        assert!(index.validate(&heap, &records).is_err());
    }
}
