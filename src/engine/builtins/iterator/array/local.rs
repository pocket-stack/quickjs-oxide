//! Finish shared Array-next stages locally until an observable callback boundary.
use super::*;
use crate::engine::object::OrdinaryRead;
use crate::engine::value::conversion::number::NumberStep;

impl ArrayNextStep {
    /// The existing native activation and iterator root stay alive throughout
    /// this transaction. Only in-range own scalar data can complete here; no
    /// source owner, key, waiting payload or cleanup is created or released.
    pub(super) fn dense_immediate_next(
        runtime: &Runtime,
        iterator: &ObjectRef,
        source: crate::engine::heap::ObjectId,
        index: u32,
        kind: ArrayIteratorKind,
    ) -> Result<Option<Value>, RuntimeError> {
        use crate::engine::atom::{AtomKind, AtomSpelling};
        use crate::engine::heap::{ObjectKind, RawValue};
        if kind == ArrayIteratorKind::KeyAndValue
            || !iterator.belongs_to(runtime)
            || runtime.0.deferred_references.has_pending()
        {
            return Ok(None);
        }
        let Ok(mut state) = runtime.0.state.try_borrow_mut() else {
            return Ok(None);
        };
        if state.heap.has_pending_zero_cleanup() {
            return Ok(None);
        }
        let object = state.heap.object(source)?;
        if object.kind != ObjectKind::Array {
            return Ok(None);
        }
        let Some(raw) = object.dense_array_value(index) else {
            // Sparse/slow elements retain the ordinary lookup and its exact
            // post-increment getter/error ordering, even for Key mode.
            return Ok(None);
        };
        let value = match (kind, raw) {
            (ArrayIteratorKind::Key, _) => Runtime::array_length_value(index),
            (_, RawValue::Undefined) => Value::Undefined,
            (_, RawValue::Null) => Value::Null,
            (_, RawValue::Bool(value)) => Value::Bool(*value),
            (_, RawValue::Int(value)) => Value::Int(*value),
            (_, RawValue::Float(value)) => Value::Float(*value),
            _ => return Ok(None),
        };
        let shape = state.heap.shape(object.shape)?;
        let Some(first) = shape.entries().first() else {
            return Ok(None);
        };
        let length = first.atom;
        // Borrow the already-owned mandatory property name; a malformed or
        // unexpected layout falls back to the original interned-key accessor.
        // The stored unbranded index is re-branded at this table boundary.
        let length = state.atoms.brand(length)?;
        let info = state.atoms.resolve(length)?;
        let AtomSpelling::Text(text) = info.spelling else {
            return Ok(None);
        };
        if info.kind != AtomKind::String
            || text.len() != 6
            || !b"length"
                .iter()
                .enumerate()
                .all(|(i, b)| text.code_unit_at(i) == Some(u16::from(*b)))
        {
            return Ok(None);
        }
        let Some((length, _)) = Runtime::array_length_state_in_heap(&state.heap, source, length)?
        else {
            return Ok(None);
        };
        let Some(next_index) = live_next_index(index, length) else {
            // Completion releases the source edge and can drain cleanup.
            return Ok(None);
        };
        state
            .heap
            .set_array_iterator_index(iterator.object_id(), next_index)?;
        Ok(Some(value))
    }

    pub(crate) fn advance_local(
        mut self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Self, RuntimeError> {
        loop {
            self = match self {
                Self::Read { mut resume } => {
                    let (object, key) = resume.take_read();
                    // Move the existing read owner into its receiver wrapper;
                    // the resume independently retains the source across reads.
                    let receiver = Value::Object(object);
                    let Value::Object(object) = &receiver else {
                        unreachable!()
                    };
                    let read = runtime.prepare_ordinary_read_borrowed(object, &key, &receiver)?;
                    match read {
                        OrdinaryRead::Complete(value) => resume.resume(
                            runtime,
                            Completion::Return(value.unwrap_or(Value::Undefined)),
                        )?,
                        read => {
                            // Lookup may have materialized a lazy descriptor.
                            // Keep its selected getter/Proxy and never replay it.
                            return Ok(resume.prepared(read, key));
                        }
                    }
                }
                Self::Number { mut resume }
                    if !matches!(resume.requested_value, Some(Value::Object(_))) =>
                {
                    let value = resume.take_number();
                    let NumberStep::Complete(result) = NumberStep::start(runtime, realm, value)?
                    else {
                        return Err(RuntimeError::Invariant(
                            "primitive iterator length suspended",
                        ));
                    };
                    resume.number(runtime, result)?
                }
                Self::Complete(result) => {
                    #[cfg(feature = "profiling")]
                    crate::engine::api::profiling::record_owned_execution_event(
                        "array_next_completed_without_waiting_state",
                    );
                    return Ok(Self::Complete(result));
                }
                step => return Ok(step),
            };
        }
    }
}

#[cfg(test)]
mod dense_immediate_tests {
    use super::*;
    use crate::engine::heap::ObjectId;

    fn iterator(runtime: &Runtime, source: &str) -> (ObjectRef, ObjectId, ArrayIteratorKind) {
        let mut context = runtime.new_context();
        let Value::Object(iterator) = context.eval(source).unwrap() else {
            panic!("fixture must return an iterator");
        };
        let (source, index, kind) = runtime
            .0
            .state
            .borrow()
            .heap
            .array_iterator_state(iterator.object_id())
            .unwrap();
        assert_eq!(index, 0);
        drop(context);
        runtime.run_gc().unwrap();
        (iterator, source.unwrap(), kind)
    }

    #[test]
    fn callback_requests_keep_the_same_array_next_resume_allocation() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let iterator = context.eval("Array.prototype.values.call({get length(){return {valueOf(){return 1}}},get 0(){return 7}})").unwrap();
        let ArrayNextStep::PreparedRead { mut resume } = ArrayNextStep::start(
            &runtime,
            context.realm,
            &NativeInvocation::Call {
                this_value: iterator,
            },
        )
        .unwrap() else {
            panic!("length getter")
        };
        let address = &*resume.0 as *const ArrayNextResumeState;
        let read = resume.take_prepared();
        let key = resume.take_key();
        let NativeConversion::Value(Some(value)) = runtime
            .finish_prepared_read(context.realm, &key, read)
            .unwrap()
        else {
            panic!("length")
        };
        let ArrayNextStep::Number { mut resume } =
            resume.resume(&runtime, Completion::Return(value)).unwrap()
        else {
            panic!("number")
        };
        assert_eq!(&*resume.0 as *const ArrayNextResumeState, address);
        let value = resume.take_number();
        let result = runtime.native_to_number(context.realm, &value).unwrap();
        let ArrayNextStep::PreparedRead { mut resume } = resume.number(&runtime, result).unwrap()
        else {
            panic!("element getter")
        };
        assert_eq!(&*resume.0 as *const ArrayNextResumeState, address);
        let read = resume.take_prepared();
        let key = resume.take_key();
        let NativeConversion::Value(Some(value)) = runtime
            .finish_prepared_read(context.realm, &key, read)
            .unwrap()
        else {
            panic!("element")
        };
        assert!(matches!(
            resume.resume(&runtime, Completion::Return(value)).unwrap(),
            ArrayNextStep::Complete(NativeInvokeOutcome::IteratorNextRaw {
                value: Value::Int(7),
                done: false
            })
        ));
    }

    #[test]
    fn dense_immediate_next_keeps_source_owned_by_iterator_through_gc() {
        let runtime = Runtime::new();
        let (iterator, source, kind) = iterator(&runtime, "[undefined,null,true,7,1.5].values()");
        runtime.run_gc().unwrap();
        let owners = runtime
            .0
            .state
            .borrow()
            .heap
            .object_strong_count(source)
            .unwrap();
        for (index, expected) in [
            Value::Undefined,
            Value::Null,
            Value::Bool(true),
            Value::Int(7),
            Value::Float(1.5),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(
                ArrayNextStep::dense_immediate_next(
                    &runtime,
                    &iterator,
                    source,
                    index as u32,
                    kind
                )
                .unwrap(),
                Some(expected)
            );
            assert_eq!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .array_iterator_state(iterator.object_id())
                    .unwrap()
                    .1,
                index as u32 + 1
            );
            assert_eq!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .object_strong_count(source)
                    .unwrap(),
                owners
            );
            runtime.run_gc().unwrap();
        }
        assert!(
            ArrayNextStep::dense_immediate_next(&runtime, &iterator, source, 5, kind)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .array_iterator_state(iterator.object_id())
                .unwrap()
                .0,
            Some(source)
        );
        drop(iterator);
        runtime.run_gc().unwrap();
        assert!(runtime.0.state.borrow().heap.object(source).is_err());
    }

    #[test]
    fn dense_immediate_next_declines_without_mutating_index_or_draining_owners() {
        let runtime = Runtime::new();
        for expression in [
            "[{}].values()",
            "[Symbol('x')].values()",
            "['x'].values()",
            "[1n].values()",
            "[,1].values()",
            "[1].entries()",
            "new Uint8Array([1]).values()",
            "Array.prototype.values.call(new Proxy([1],{}))",
        ] {
            let (iterator, source, kind) = iterator(&runtime, expression);
            assert!(
                ArrayNextStep::dense_immediate_next(&runtime, &iterator, source, 0, kind)
                    .unwrap()
                    .is_none(),
                "{expression}"
            );
            assert_eq!(
                runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .array_iterator_state(iterator.object_id())
                    .unwrap()
                    .1,
                0,
                "{expression}"
            );
        }
        let (iterator, source, kind) = iterator(&runtime, "[1].values()");
        let released = runtime.new_object(None).unwrap();
        {
            let _borrow = runtime.0.state.borrow();
            drop(released);
        }
        assert!(runtime.0.deferred_references.has_pending());
        assert!(
            ArrayNextStep::dense_immediate_next(&runtime, &iterator, source, 0, kind)
                .unwrap()
                .is_none()
        );
        assert!(runtime.0.deferred_references.has_pending());
        assert_eq!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .array_iterator_state(iterator.object_id())
                .unwrap()
                .1,
            0
        );
        runtime.drain_deferred_references().unwrap();
        assert_eq!(
            ArrayNextStep::dense_immediate_next(&runtime, &iterator, source, 0, kind).unwrap(),
            Some(Value::Int(1))
        );
    }

    #[test]
    fn dense_immediate_next_keys_and_uint32_boundary_share_cursor_decision() {
        let runtime = Runtime::new();
        let (iterator, source, kind) = iterator(&runtime, "[{},{}].keys()");
        for index in 0..2 {
            assert_eq!(
                ArrayNextStep::dense_immediate_next(&runtime, &iterator, source, index, kind)
                    .unwrap(),
                Some(Value::Int(index as i32))
            );
        }
        assert!(
            ArrayNextStep::dense_immediate_next(&runtime, &iterator, source, 2, kind)
                .unwrap()
                .is_none()
        );
        assert_eq!(live_next_index(u32::MAX - 1, u32::MAX), Some(u32::MAX));
        assert_eq!(live_next_index(u32::MAX, u32::MAX), None);
        assert_eq!(live_next_index(0, 0), None);
    }
}
