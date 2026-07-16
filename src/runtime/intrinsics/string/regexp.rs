//! RegExp-backed String prototype methods.

use super::*;

#[derive(Clone, Copy)]
enum StringRegExpProtocol {
    Match,
    Search,
}

impl StringRegExpProtocol {
    const fn symbol(self) -> WellKnownSymbol {
        match self {
            Self::Match => WellKnownSymbol::Match,
            Self::Search => WellKnownSymbol::Search,
        }
    }

    const fn invocation_invariant(self) -> &'static str {
        match self {
            Self::Match => "String match did not receive a generic-magic invocation",
            Self::Search => "String search did not receive a generic-magic invocation",
        }
    }

    const fn argument_invariant(self) -> &'static str {
        match self {
            Self::Match => "String match pattern argv was not padded",
            Self::Search => "String search pattern argv was not padded",
        }
    }
}

impl Runtime {
    pub(in crate::runtime) fn call_string_prototype_match(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_regexp_protocol(realm, invocation, arguments, StringRegExpProtocol::Match)
    }

    pub(in crate::runtime) fn call_string_prototype_search(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        self.call_string_regexp_protocol(realm, invocation, arguments, StringRegExpProtocol::Search)
    }

    /// Rust port of the `Symbol.match` and `Symbol.search` branches in pinned
    /// QuickJS `js_string_match`.
    ///
    /// Object patterns may intercept the operation before receiver coercion.
    /// The fallback converts the receiver, constructs with the defining
    /// realm's retained intrinsic RegExp constructor, and dynamically invokes
    /// the selected protocol on that fresh value so prototype mutations remain
    /// observable.
    fn call_string_regexp_protocol(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
        protocol: StringRegExpProtocol,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(protocol.invocation_invariant()));
        };
        if matches!(this_value, Value::Undefined | Value::Null) {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "cannot convert to object",
            )?));
        }
        let pattern = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(protocol.argument_invariant()))?;
        let protocol_key = PropertyKey::from(self.well_known_symbol(protocol.symbol()));

        if let Value::Object(pattern_object) = pattern {
            let method = match self.get_property_in_realm(realm, pattern_object, &protocol_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if !matches!(method, Value::Undefined | Value::Null) {
                return self.call_string_regexp_method(
                    realm,
                    pattern_object.clone(),
                    method,
                    std::slice::from_ref(&this_value),
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
        let method = match self.get_property_in_realm(realm, &regexp, &protocol_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        self.call_string_regexp_method(realm, regexp, method, &[Value::String(source)])
    }

    pub(super) fn call_string_regexp_method(
        &self,
        realm: ContextId,
        receiver: ObjectRef,
        method: Value,
        arguments: &[Value],
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
        self.call_internal(realm, &callable, Value::Object(receiver), arguments)
    }
}
