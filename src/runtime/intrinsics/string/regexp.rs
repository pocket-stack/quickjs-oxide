//! RegExp-backed String prototype methods.

use super::*;

impl Runtime {
    /// Rust port of the `Symbol.search` branch in pinned QuickJS
    /// `js_string_match`.
    ///
    /// Object patterns may intercept the operation before receiver coercion.
    /// The fallback converts the receiver, constructs with the defining
    /// realm's retained intrinsic RegExp constructor, and dynamically invokes
    /// `@@search` on that fresh value so prototype mutations remain observable.
    pub(in crate::runtime) fn call_string_prototype_search(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "String search did not receive a generic-magic invocation",
            ));
        };
        if matches!(this_value, Value::Undefined | Value::Null) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to object",
            )?));
        }
        let pattern = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "String search pattern argv was not padded",
        ))?;
        let search = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Search));

        if let Value::Object(pattern_object) = pattern {
            let method = match self.get_property_in_realm(realm, pattern_object, &search)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !matches!(method, Value::Undefined | Value::Null) {
                return self.call_string_search_method(
                    realm,
                    pattern_object.clone(),
                    method,
                    this_value,
                );
            }
        }

        let source = match self.native_to_js_string(realm, &this_value)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let regexp = match self.construct_intrinsic_regexp(realm, pattern.clone())? {
            Completion::Return(Value::Object(value)) => value,
            Completion::Return(_) => {
                return Err(RuntimeError::Invariant(
                    "intrinsic RegExp constructor returned a primitive",
                ));
            }
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let method = match self.get_property_in_realm(realm, &regexp, &search)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.call_string_search_method(realm, regexp, method, Value::String(source))
    }

    fn call_string_search_method(
        &self,
        realm: ContextId,
        receiver: ObjectRef,
        method: Value,
        input: Value,
    ) -> Result<Completion, RuntimeError> {
        let callable = match method {
            Value::Object(object) => self.as_callable(&object)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => None,
        };
        let Some(callable) = callable else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a function",
            )?));
        };
        self.call_internal(
            realm,
            &callable,
            Value::Object(receiver),
            std::slice::from_ref(&input),
        )
    }
}
