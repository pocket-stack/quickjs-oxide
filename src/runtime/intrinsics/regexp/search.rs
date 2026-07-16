//! `RegExp.prototype[Symbol.search]`.

use super::super::super::*;

impl Runtime {
    /// Rust port of pinned QuickJS `js_regexp_Symbol_search`.
    ///
    /// The method is intentionally generic over every object receiver. It
    /// snapshots `lastIndex`, forces positive zero only when SameValue says it
    /// is necessary, executes through the observable RegExpExec protocol, and
    /// restores the snapshot only after both execution and the post-exec read
    /// succeed. The returned match index is not numerically coerced.
    pub(super) fn call_regexp_symbol_search(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value: regexp } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp @@search did not receive a generic invocation",
            ));
        };
        let Value::Object(regexp) = regexp else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let input = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp @@search input argv was not padded",
        ))?;
        let input = match self.native_to_js_string(realm, input)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let last_index = self.intern_property_key("lastIndex")?;
        let previous = match self.get_property_in_realm(realm, &regexp, &last_index)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !previous.same_value(&Value::Int(0))
            && let Some(value) =
                self.set_property_or_throw(realm, &regexp, &last_index, Value::Int(0))?
        {
            return Ok(Completion::Throw(value));
        }

        let result = match self.regexp_exec_abstract(
            realm,
            Value::Object(regexp.clone()),
            Value::String(input),
        )? {
            Completion::Return(value) => value,
            // QuickJS does not restore lastIndex when RegExpExec throws.
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let current = match self.get_property_in_realm(realm, &regexp, &last_index)? {
            Completion::Return(value) => value,
            // Nor does it restore when the post-exec lastIndex Get throws.
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        if !current.same_value(&previous)
            && let Some(value) =
                self.set_property_or_throw(realm, &regexp, &last_index, previous)?
        {
            return Ok(Completion::Throw(value));
        }

        match result {
            Value::Null => Ok(Completion::Return(Value::Int(-1))),
            Value::Object(result) => {
                let index = self.intern_property_key("index")?;
                self.get_property_in_realm(realm, &result, &index)
            }
            Value::Undefined
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => Err(RuntimeError::Invariant(
                "RegExpExec returned neither an object nor null",
            )),
        }
    }
}
