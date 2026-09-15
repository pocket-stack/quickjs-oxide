//! `RegExp.prototype[Symbol.matchAll]` and RegExp String Iterator.

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::heap::{ContextId, ObjectData};
use crate::engine::object::ObjectRef;

use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation, NativeInvokeOutcome};

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_matchAll`.
    ///
    /// The input conversion, species lookup, flags conversion, construction,
    /// original `lastIndex` read and matcher write deliberately retain their
    /// upstream order. The iterator caches `g` and full-Unicode mode from the
    /// original flags string rather than consulting the constructed matcher.
    pub(crate) fn call_regexp_symbol_match_all(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        super::match_all_protocol::finish(
            self,
            realm,
            super::match_all_protocol::RegExpMatchAllStep::start(
                self,
                realm,
                &invocation,
                arguments,
            )?,
        )
    }

    pub(super) fn new_regexp_string_iterator(
        &self,
        realm: ContextId,
        regexp: &ObjectRef,
        string: JsString,
        global: bool,
        full_unicode: bool,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !regexp.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("RegExp String Iterator matcher"));
        }
        let prototype_id = self.regexp_realm_data(realm)?.string_iterator_prototype;
        let prototype = ObjectRef::from_borrowed_handle(self.clone(), prototype_id)?;
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &[])?;
        let object = match state
            .heap
            .allocate_object(ObjectData::regexp_string_iterator(
                shape,
                Vec::new(),
                regexp.object_id(),
                string,
                global,
                full_unicode,
            )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    pub(crate) fn call_regexp_string_iterator_next(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        match self.call_regexp_string_iterator_next_raw(realm, invocation)? {
            NativeInvokeOutcome::Completion(completion) => Ok(completion),
            NativeInvokeOutcome::IteratorNextRaw { value, done } => Ok(Completion::Return(
                Value::Object(self.new_iterator_result(realm, value, done)?),
            )),
        }
    }

    /// Execute QuickJS's `JS_CFUNC_iterator_next` ABI without allocating the
    /// public iterator-result object. Exceptions leave the iterator's `done`
    /// bit unchanged so a later call retries from the matcher state produced
    /// by the failed observable operation.
    pub(crate) fn call_regexp_string_iterator_next_raw(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<NativeInvokeOutcome, RuntimeError> {
        super::iterator_next::finish(
            self,
            realm,
            super::iterator_next::RegExpIteratorStep::start(self, realm, &invocation)?,
        )
    }
}
