use super::*;

/// Lazy operation retained by a genuine Iterator Helper object.
///
/// The eager consumers (`every`, `find`, `forEach`, and `some`) do not
/// allocate a helper payload and therefore use [`IteratorConsumerKind`]
/// instead.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IteratorHelperKind {
    Drop,
    Filter,
    FlatMap,
    Map,
    Take,
}

/// Eager operation selected by the shared Iterator consumer implementation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IteratorConsumerKind {
    Every,
    Find,
    ForEach,
    Some,
}

/// Resume operation shared by Iterator Helper and Iterator Wrap prototypes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IteratorResumeKind {
    Next,
    Return,
}

/// Complete hidden state of one genuine Iterator Helper.
///
/// Object-only fields use typed handles; the cached `next` and callback remain
/// raw arena-owned values because property lookup can produce any ECMAScript
/// value. QuickJS keeps all four edges alive until finalization even after
/// `done` becomes true.
#[derive(Clone, Debug, PartialEq)]
pub struct IteratorHelperData {
    pub source: ObjectId,
    pub next: RawValue,
    pub callback: RawValue,
    pub inner: Option<ObjectId>,
    pub count: i64,
    pub kind: IteratorHelperKind,
    pub executing: bool,
    pub done: bool,
}

/// Hidden state of an Iterator created by `Iterator.from`.
#[derive(Clone, Debug, PartialEq)]
pub struct IteratorWrapData {
    pub source: RawValue,
    pub next: RawValue,
}

/// Hidden state of the branded adapter created when `GetAsyncIterator`
/// falls back to a synchronous iterator.
///
/// The source is known to be an object after `GetIterator`, while `next`
/// remains an arbitrary cached ECMAScript value until the first call.
#[derive(Clone, Debug, PartialEq)]
pub struct AsyncFromSyncIteratorData {
    pub sync_iterator: ObjectId,
    pub next: RawValue,
}

/// One eagerly validated iterable/method pair retained by `Iterator.concat`.
///
/// Consumed slots become `None` so their edges can be released immediately,
/// matching QuickJS's advancing finalizer boundary without shifting the
/// remaining vector.
#[derive(Clone, Debug, PartialEq)]
pub struct IteratorConcatItem {
    pub iterable: ObjectId,
    pub method: RawValue,
}

/// Hidden state of the lazy iterator returned by `Iterator.concat`.
#[derive(Clone, Debug, PartialEq)]
pub struct IteratorConcatData {
    pub items: Vec<Option<IteratorConcatItem>>,
    pub index: usize,
    pub iterator: Option<ObjectId>,
    pub next: RawValue,
    pub running: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::heap) enum IteratorHelperRawValueField {
    Next,
    Callback,
}

#[cfg(test)]
impl IteratorHelperRawValueField {
    pub(in crate::engine::heap) fn get_mut(self, data: &mut IteratorHelperData) -> &mut RawValue {
        match self {
            Self::Next => &mut data.next,
            Self::Callback => &mut data.callback,
        }
    }
}

pub(in crate::engine::heap) fn validate_iterator_helper_data(
    heap: &Heap,
    data: &IteratorHelperData,
) -> Result<(), HeapError> {
    heap.object(data.source)?;
    if let Some(inner) = data.inner {
        heap.object(inner)?;
    }
    if !is_map_storable_value(&data.next)
        || !is_map_storable_value(&data.callback)
        || data.count < 0
        || (data.kind != IteratorHelperKind::FlatMap && data.inner.is_some())
    {
        return Err(HeapError::Invariant(
            "Iterator Helper payload has invalid hidden state",
        ));
    }
    match data.kind {
        IteratorHelperKind::Drop | IteratorHelperKind::Take => {
            if !matches!(data.callback, RawValue::Undefined) {
                return Err(HeapError::Invariant(
                    "limit Iterator Helper unexpectedly retains a callback",
                ));
            }
        }
        IteratorHelperKind::Filter | IteratorHelperKind::FlatMap | IteratorHelperKind::Map => {
            let RawValue::Object(callback) = data.callback else {
                return Err(HeapError::Invariant(
                    "callback Iterator Helper does not retain a callable",
                ));
            };
            if !object_data_is_callable(heap.object(callback)?) {
                return Err(HeapError::Invariant(
                    "callback Iterator Helper does not retain a callable",
                ));
            }
        }
    }
    Ok(())
}

pub(in crate::engine::heap) fn validate_iterator_wrap_data(
    heap: &Heap,
    data: &IteratorWrapData,
) -> Result<(), HeapError> {
    if !is_map_storable_value(&data.source) || !is_map_storable_value(&data.next) {
        return Err(HeapError::Invariant(
            "Iterator Wrap payload contains an internal value sentinel",
        ));
    }
    for edge in raw_value_edges(&data.source)
        .into_iter()
        .chain(raw_value_edges(&data.next))
    {
        let RawId::Object(object) = edge else {
            unreachable!("RawValue only owns object edges")
        };
        heap.object(object)?;
    }
    Ok(())
}

pub(in crate::engine::heap) fn validate_iterator_concat_data(
    heap: &Heap,
    data: &IteratorConcatData,
) -> Result<(), HeapError> {
    if data.index > data.items.len() || !is_map_storable_value(&data.next) {
        return Err(HeapError::Invariant(
            "Iterator Concat payload has invalid hidden state",
        ));
    }
    for (index, item) in data.items.iter().enumerate() {
        if (index < data.index) != item.is_none() {
            return Err(HeapError::Invariant(
                "Iterator Concat released-input boundary is inconsistent",
            ));
        }
        let Some(item) = item else {
            continue;
        };
        heap.object(item.iterable)?;
        let RawValue::Object(method) = item.method else {
            return Err(HeapError::Invariant(
                "Iterator Concat input method is not callable",
            ));
        };
        if !object_data_is_callable(heap.object(method)?) {
            return Err(HeapError::Invariant(
                "Iterator Concat input method is not callable",
            ));
        }
    }
    if let Some(iterator) = data.iterator {
        heap.object(iterator)?;
        if data.index >= data.items.len() {
            return Err(HeapError::Invariant(
                "Iterator Concat retains an iterator after exhaustion",
            ));
        }
    } else if !matches!(data.next, RawValue::Undefined) {
        return Err(HeapError::Invariant(
            "Iterator Concat caches next without a current iterator",
        ));
    }
    for edge in raw_value_edges(&data.next) {
        let RawId::Object(object) = edge else {
            unreachable!("RawValue only owns object edges")
        };
        heap.object(object)?;
    }
    Ok(())
}
