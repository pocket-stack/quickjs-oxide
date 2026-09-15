//! Non-owning key lookup for insertion-ordered strong collections.
//!
//! Records own keys and GC edges; this index owns only hashes and record IDs.
//! The collection storage boundary maintains both together. Lookups borrow records
//! without rooting every candidate or invoking JavaScript. Public iterator cursors remain the responsibility of ordered record storage.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::fmt;
use std::hash::{BuildHasher, Hasher};

use super::{CollectionRecords, HeapError, RawValue};
use crate::engine::value::{JsString, WeakJsString, collection_key};

#[derive(Clone, Default)]
pub struct CollectionIndex {
    buckets: HashMap<u64, Vec<usize>>,
    // Allocate the cache header only for long string keys. It is scoped to
    // this index's hasher seed and keeps no String payload alive.
    string_hashes: RefCell<Option<Box<StringHashCache>>>,
    #[cfg(test)]
    hash_computations: std::cell::Cell<usize>,
}

const MIN_CACHED_STRING_UNITS: usize = 256;
const STRING_HASH_CACHE_SIZE: usize = 8;

#[derive(Clone)]
struct CachedStringHash {
    string: WeakJsString,
    hash: u64,
}

#[derive(Clone, Default)]
struct StringHashCache {
    entries: VecDeque<CachedStringHash>,
}

impl StringHashCache {
    fn find(&self, string: &JsString) -> Option<u64> {
        self.entries
            .iter()
            .find(|entry| entry.string.same_representation(string))
            .map(|entry| entry.hash)
    }

    fn remember(&mut self, string: &JsString, hash: u64) {
        if self.entries.len() == STRING_HASH_CACHE_SIZE {
            self.entries.pop_front();
        }
        self.entries.push_back(CachedStringHash {
            string: string.downgrade(),
            hash,
        });
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
    fn hash(&self, key: &RawValue) -> u64 {
        let RawValue::String(string) = key else {
            return self.hash_uncached(key);
        };
        if string.len() < MIN_CACHED_STRING_UNITS {
            return self.hash_uncached(key);
        }
        if let Some(hash) = self
            .string_hashes
            .borrow()
            .as_ref()
            .and_then(|cache| cache.find(string))
        {
            return hash;
        }
        let hash = self.hash_uncached(key);
        self.string_hashes
            .borrow_mut()
            .get_or_insert_with(Box::default)
            .remember(string, hash);
        hash
    }

    fn hash_uncached(&self, key: &RawValue) -> u64 {
        #[cfg(test)]
        self.hash_computations.set(self.hash_computations.get() + 1);
        let mut hasher = self.buckets.hasher().build_hasher();
        collection_key::hash(key, &mut hasher);
        hasher.finish()
    }

    pub(super) fn find(&self, records: &CollectionRecords, key: &RawValue) -> Option<usize> {
        self.buckets
            .get(&self.hash(key))?
            .iter()
            .copied()
            .find(|&index| {
                collection_key::same_value_zero(
                    &records.get(index).expect("indexed record exists").key,
                    key,
                )
            })
    }

    pub(super) fn insert(&mut self, key: &RawValue, index: usize) {
        self.buckets.entry(self.hash(key)).or_default().push(index);
    }

    pub(super) fn remove(&mut self, key: &RawValue, index: usize) {
        let hash = self.hash(key);
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
    pub(super) fn validate(&self, records: &CollectionRecords) -> Result<(), HeapError> {
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
                if !seen.insert(index) || self.hash_uncached(key) != hash {
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

    fn record_store(values: Vec<MapRecord>) -> CollectionRecords {
        let mut records = CollectionRecords::default();
        for record in values {
            records.insert(record);
        }
        records
    }
    use crate::engine::value::{JsString, bigint::JsBigInt};

    #[test]
    fn long_string_hash_memo_is_bounded_and_does_not_own_keys() {
        let index = CollectionIndex::default();
        let key = JsString::try_from_utf8(&"x".repeat(1024)).unwrap();
        let weak = key.downgrade();
        let expected = index.hash(&RawValue::String(key.clone()));
        for _ in 0..32 {
            assert_eq!(index.hash(&RawValue::String(key.clone())), expected);
        }
        assert_eq!(index.hash_computations.get(), 1);
        let cloned = index.clone();
        assert_eq!(cloned.hash(&RawValue::String(key.clone())), expected);
        let independent = CollectionIndex::default();
        assert_eq!(
            independent.hash(&RawValue::String(key.clone())),
            independent.hash_uncached(&RawValue::String(key.clone()))
        );
        drop(key);
        assert!(
            weak.upgrade().is_none(),
            "hash memo must not retain a String payload"
        );
        for suffix in 0..32 {
            let key = JsString::try_from_utf8(&("x".repeat(1024) + &suffix.to_string())).unwrap();
            index.hash(&RawValue::String(key));
        }
        assert!(
            index.string_hashes.borrow().as_ref().unwrap().entries.len() <= STRING_HASH_CACHE_SIZE
        );
    }

    #[test]
    fn equal_keys_share_hash_across_number_and_string_representations() {
        let index = CollectionIndex::default();
        let pairs = [
            (RawValue::Int(0), RawValue::Float(-0.0)),
            (RawValue::Int(42), RawValue::Float(42.0)),
            (
                RawValue::Float(f64::NAN),
                RawValue::Float(f64::from_bits(0x7ff8_0000_0000_0042)),
            ),
            (
                RawValue::String(JsString::from_static("abc")),
                RawValue::String(JsString::try_from_utf16([97, 98, 99]).unwrap()),
            ),
            (
                RawValue::BigInt(
                    JsBigInt::parse_radix("123456789012345678901234567890", 10).unwrap(),
                ),
                RawValue::BigInt(
                    JsBigInt::parse_radix("123456789012345678901234567890", 10).unwrap(),
                ),
            ),
        ];
        for (left, right) in pairs {
            assert!(collection_key::same_value_zero(&left, &right));
            assert_eq!(index.hash(&left), index.hash(&right));
        }
        assert!(!collection_key::same_value_zero(
            &RawValue::Int(1),
            &RawValue::String(JsString::from_static("1"))
        ));
        assert!(!collection_key::same_value_zero(
            &RawValue::Int(1),
            &RawValue::BigInt(JsBigInt::one())
        ));
    }

    #[test]
    fn collision_candidates_are_compared_and_removal_preserves_the_others() {
        let mut index = CollectionIndex::default();
        let records = record_store(vec![
            MapRecord {
                key: RawValue::Int(1),
                value: RawValue::Undefined,
            },
            MapRecord {
                key: RawValue::Int(2),
                value: RawValue::Undefined,
            },
        ]);
        // Force a collision to exercise the bucket path deterministically,
        // without relying on the randomized hasher finding one naturally.
        let hash = index.hash(&RawValue::Int(2));
        index.buckets.insert(hash, vec![0, 1]);
        assert_eq!(index.find(&records, &RawValue::Int(2)), Some(1));
        index.remove(&RawValue::Int(2), 1);
        assert_eq!(index.find(&records, &RawValue::Int(2)), None);
        assert_eq!(index.buckets[&hash], vec![0]);
    }

    #[test]
    fn publication_rejects_missing_or_stale_index_entries() {
        let mut index = CollectionIndex::default();
        let mut records = record_store(vec![MapRecord {
            key: RawValue::Int(1),
            value: RawValue::Undefined,
        }]);
        assert!(index.validate(&records).is_err());
        index.insert(&RawValue::Int(1), 0);
        assert!(index.validate(&records).is_ok());
        records.get_mut(0).unwrap().key = RawValue::Int(2);
        assert!(index.validate(&records).is_err());
    }
}
