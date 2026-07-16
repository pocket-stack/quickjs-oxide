//! Legacy `%RegExp.prototype.compile%` mutation.

use crate::heap::RegExpObjectData;

use super::super::super::*;

impl Runtime {
    /// Pinned QuickJS `js_regexp_compile`.
    ///
    /// This deliberately uses the concrete RegExp brand rather than
    /// `IsRegExp`: neither `@@match` nor public source/flag properties are
    /// observed. Compilation is transactional, but the internal replacement
    /// precedes the final throwing `lastIndex` Set and is not rolled back when
    /// that Set fails.
    pub(super) fn call_regexp_compile(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp.prototype.compile did not receive a generic invocation",
            ));
        };

        // The receiver brand check is the first semantic operation, before
        // reading or converting either argument.
        let Some(_) = self.genuine_regexp(&this_value)? else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "RegExp object expected",
            )?));
        };
        let Value::Object(regexp) = &this_value else {
            return Err(RuntimeError::Invariant(
                "genuine RegExp snapshot accepted a primitive receiver",
            ));
        };
        let pattern_argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp compile pattern argv was not padded",
        ))?;
        let flags_argument = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "RegExp compile flags argv was not padded",
        ))?;

        let (pattern, program) = if let Some(genuine) = self.genuine_regexp(pattern_argument)? {
            if !matches!(flags_argument, Value::Undefined) {
                return Ok(Completion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Type,
                    "flags must be undefined",
                )?));
            }
            (genuine.pattern, genuine.program)
        } else {
            let pattern = if matches!(pattern_argument, Value::Undefined) {
                JsString::from_static("")
            } else {
                match self.native_to_js_string(realm, pattern_argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            };
            let flags = if matches!(flags_argument, Value::Undefined) {
                JsString::from_static("")
            } else {
                match self.native_to_js_string(realm, flags_argument)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            };
            let program = Self::compile_regexp_program(&pattern, &flags)?;
            (pattern, program)
        };

        let previous = self.0.state.borrow_mut().heap.replace_regexp_data(
            regexp.object_id(),
            RegExpObjectData::Compiled { pattern, program },
        )?;
        if !matches!(previous, RegExpObjectData::Compiled { .. }) {
            return Err(RuntimeError::Invariant(
                "observable RegExp object was not initialized",
            ));
        }

        let last_index = self.intern_property_key("lastIndex")?;
        if let Some(value) =
            self.set_property_or_throw(realm, regexp, &last_index, Value::Int(0))?
        {
            return Ok(Completion::Throw(value));
        }
        Ok(Completion::Return(this_value))
    }
}
