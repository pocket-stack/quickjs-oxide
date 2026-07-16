//! Shared replacement-template expansion for String and RegExp intrinsics.

use crate::value::ReplacementStringBuffer;

use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SubstitutionStatus {
    Complete,
    BufferFailed,
}

pub(super) struct SubstitutionInput<'a> {
    pub(super) matched: &'a JsString,
    pub(super) input: &'a JsString,
    pub(super) position: usize,
    pub(super) captures: Option<&'a [Value]>,
    pub(super) named_captures: Option<&'a ObjectRef>,
    pub(super) replacement: &'a JsString,
}

impl Runtime {
    /// Rust port of pinned QuickJS `js_string_GetSubstitution`.
    ///
    /// `captures`, when present, contains the already-observed and converted
    /// capture values with the complete match at index zero. Named capture
    /// properties remain lazy: every `$<name>` occurrence performs its own Get
    /// and ToString in the defining realm.
    pub(super) fn append_get_substitution(
        &self,
        realm: ContextId,
        buffer: &mut ReplacementStringBuffer,
        substitution: SubstitutionInput<'_>,
    ) -> Result<Result<SubstitutionStatus, Value>, RuntimeError> {
        let SubstitutionInput {
            matched,
            input,
            position,
            captures,
            named_captures,
            replacement,
        } = substitution;
        let replacement = replacement.linearize();
        let capture_count = captures.map_or(0, <[Value]>::len);
        let matched_len = matched.len();
        let mut cursor = 0_usize;

        while cursor < replacement.len() {
            let Some(dollar) = (cursor..replacement.len())
                .find(|index| replacement.code_unit_at(*index) == Some(u16::from(b'$')))
            else {
                buffer.append_range(&replacement, cursor, replacement.len());
                break;
            };
            if dollar + 1 >= replacement.len() {
                buffer.append_range(&replacement, cursor, replacement.len());
                break;
            }

            buffer.append_range(&replacement, cursor, dollar);
            let token_start = dollar;
            let mut next = dollar + 2;
            let token = replacement
                .code_unit_at(dollar + 1)
                .expect("replacement token was bounded by String length");

            match token {
                unit if unit == u16::from(b'$') => {
                    buffer.append_code_unit(u16::from(b'$'));
                }
                unit if unit == u16::from(b'&') => {
                    buffer.append_js_string(matched);
                    if buffer.error().is_some() {
                        return Ok(Ok(SubstitutionStatus::BufferFailed));
                    }
                }
                unit if unit == u16::from(b'`') => {
                    buffer.append_range(input, 0, position);
                }
                unit if unit == u16::from(b'\'') => {
                    let suffix = position.saturating_add(matched_len);
                    if suffix < input.len() {
                        buffer.append_range(input, suffix, input.len());
                    }
                }
                unit if (u16::from(b'0')..=u16::from(b'9')).contains(&unit) => {
                    let mut capture_index = usize::from(unit - u16::from(b'0'));
                    if next < replacement.len() {
                        let second = replacement
                            .code_unit_at(next)
                            .expect("second replacement digit was bounded");
                        if (u16::from(b'0')..=u16::from(b'9')).contains(&second) {
                            let pair = capture_index * 10 + usize::from(second - u16::from(b'0'));
                            if (1..capture_count).contains(&pair) {
                                capture_index = pair;
                                next += 1;
                            }
                        }
                    }
                    if (1..capture_count).contains(&capture_index) {
                        let capture = &captures
                            .expect("non-zero capture count omitted capture storage")
                            [capture_index];
                        match capture {
                            Value::Undefined => {}
                            Value::String(value) => {
                                buffer.append_js_string(value);
                                if buffer.error().is_some() {
                                    return Ok(Ok(SubstitutionStatus::BufferFailed));
                                }
                            }
                            _ => {
                                return Err(RuntimeError::Invariant(
                                    "replacement capture was not pre-converted",
                                ));
                            }
                        }
                    } else {
                        buffer.append_range(&replacement, token_start, next);
                    }
                }
                unit if unit == u16::from(b'<') && named_captures.is_some() => {
                    let Some(close) = (next..replacement.len())
                        .find(|index| replacement.code_unit_at(*index) == Some(u16::from(b'>')))
                    else {
                        buffer.append_range(&replacement, token_start, next);
                        cursor = next;
                        continue;
                    };
                    let name = replacement.sub_string(next, close);
                    let key = self.intern_property_key_js_string(&name)?;
                    let capture = match self.get_property_in_realm(
                        realm,
                        named_captures.expect("named captures disappeared"),
                        &key,
                    )? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Err(value)),
                    };
                    if !matches!(capture, Value::Undefined) {
                        // QuickJS's concat-value helper avoids a second
                        // exception once this StringBuffer has already failed,
                        // but the property Get above remains observable.
                        if buffer.error().is_some() {
                            return Ok(Ok(SubstitutionStatus::BufferFailed));
                        }
                        let capture = match self.native_to_js_string(realm, &capture)? {
                            NativeConversion::Value(value) => value,
                            NativeConversion::Throw(value) => return Ok(Err(value)),
                        };
                        buffer.append_js_string(&capture);
                        if buffer.error().is_some() {
                            return Ok(Ok(SubstitutionStatus::BufferFailed));
                        }
                    }
                    next = close + 1;
                }
                _ => {
                    buffer.append_range(&replacement, token_start, next);
                }
            }
            cursor = next;
        }

        Ok(Ok(SubstitutionStatus::Complete))
    }

    pub(super) fn finish_replacement_buffer(
        &self,
        realm: ContextId,
        buffer: ReplacementStringBuffer,
    ) -> Result<NativeConversion<JsString>, RuntimeError> {
        match buffer.finish() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(error @ (JsStringError::TooLong | JsStringError::OutOfMemory)) => {
                let message = match error {
                    JsStringError::TooLong => "string too long",
                    JsStringError::OutOfMemory => "out of memory",
                };
                Ok(NativeConversion::Throw(self.new_native_error(
                    realm,
                    NativeErrorKind::Internal,
                    message,
                )?))
            }
        }
    }
}
