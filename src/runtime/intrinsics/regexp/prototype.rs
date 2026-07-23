//! `%RegExp.prototype%` accessors and generic `toString`.

use crate::heap::{RegExpFlagKind, RegExpNativeKind, RegExpObjectData};
use crate::regexp::RegExpFlags;

use super::super::super::*;

impl Runtime {
    pub(super) fn call_regexp_accessor(
        &self,
        realm: ContextId,
        kind: RegExpNativeKind,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp accessor did not receive a getter invocation",
            ));
        };
        match kind {
            RegExpNativeKind::Source => self.call_regexp_source(realm, &this_value),
            RegExpNativeKind::Flags => self.call_regexp_flags(realm, &this_value),
            RegExpNativeKind::Flag(flag) => self.call_regexp_flag(realm, &this_value, flag),
            RegExpNativeKind::Constructor
            | RegExpNativeKind::Escape
            | RegExpNativeKind::Species
            | RegExpNativeKind::Exec
            | RegExpNativeKind::Compile
            | RegExpNativeKind::Test
            | RegExpNativeKind::ToString
            | RegExpNativeKind::Replace
            | RegExpNativeKind::Match
            | RegExpNativeKind::MatchAll
            | RegExpNativeKind::Search
            | RegExpNativeKind::Split => Err(RuntimeError::Invariant(
                "non-accessor RegExp selector reached accessor dispatch",
            )),
        }
    }

    pub(super) fn call_regexp_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp.prototype.toString did not receive a generic invocation",
            ));
        };
        let Value::Object(object) = &this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };

        let source_key = self.intern_property_key("source")?;
        let source = match self.get_property_in_realm(realm, object, &source_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let source = match self.native_to_js_string(realm, &source)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let mut output = JsStringBuilder::new(source.len().saturating_add(2));
        output.push_utf8("/")?;
        output.push_js_string(&source)?;
        output.push_utf8("/")?;

        // The flags Get happens only after source has been converted and the
        // first two separators have been appended, matching QuickJS's string
        // buffer path.
        let flags_key = self.intern_property_key("flags")?;
        let flags = match self.get_property_in_realm(realm, object, &flags_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let flags = match self.native_to_js_string(realm, &flags)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        output.push_js_string(&flags)?;
        Ok(Completion::Return(Value::String(output.finish()?)))
    }

    fn call_regexp_source(
        &self,
        realm: ContextId,
        this_value: &Value,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        if object.object_id() == self.regexp_realm_data(realm)?.prototype {
            return Ok(Completion::Return(Value::String(JsString::from_static(
                "(?:)",
            ))));
        }
        let pattern = {
            let state = self.0.state.borrow();
            match &state.heap.object(object.object_id())?.payload {
                ObjectPayload::RegExp(RegExpObjectData::Compiled { pattern, .. }) => {
                    Some(pattern.clone())
                }
                ObjectPayload::RegExp(RegExpObjectData::Uninitialized) => {
                    return Err(RuntimeError::Invariant(
                        "observable RegExp object was not initialized",
                    ));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::SetIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Date(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::Generator { .. } => None,
            }
        };
        let Some(pattern) = pattern else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "RegExp object expected",
            )?));
        };
        if pattern.is_empty() {
            return Ok(Completion::Return(Value::String(JsString::from_static(
                "(?:)",
            ))));
        }
        Ok(Completion::Return(Value::String(escape_regexp_source(
            &pattern,
        )?)))
    }

    fn call_regexp_flag(
        &self,
        realm: ContextId,
        this_value: &Value,
        flag: RegExpFlagKind,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let flags = {
            let state = self.0.state.borrow();
            match &state.heap.object(object.object_id())?.payload {
                ObjectPayload::RegExp(RegExpObjectData::Compiled { program, .. }) => {
                    Some(program.flags())
                }
                ObjectPayload::RegExp(RegExpObjectData::Uninitialized) => {
                    return Err(RuntimeError::Invariant(
                        "observable RegExp object was not initialized",
                    ));
                }
                ObjectPayload::Ordinary
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::SetIterator { .. }
                | ObjectPayload::ForInIterator(_)
                | ObjectPayload::Primitive(_)
                | ObjectPayload::Date(_)
                | ObjectPayload::GlobalObject { .. }
                | ObjectPayload::Error
                | ObjectPayload::StringIterator { .. }
                | ObjectPayload::RegExpStringIterator { .. }
                | ObjectPayload::NativeFunction { .. }
                | ObjectPayload::BoundFunction { .. }
                | ObjectPayload::BytecodeFunction { .. }
                | ObjectPayload::Generator { .. } => None,
            }
        };
        if let Some(flags) = flags {
            return Ok(Completion::Return(Value::Bool(
                flags.contains(regexp_flag_mask(flag)),
            )));
        }
        if object.object_id() == self.regexp_realm_data(realm)?.prototype {
            return Ok(Completion::Return(Value::Undefined));
        }
        Ok(Completion::Throw(self.new_native_error(
            realm,
            NativeErrorKind::Type,
            "RegExp object expected",
        )?))
    }

    fn call_regexp_flags(
        &self,
        realm: ContextId,
        this_value: &Value,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };
        let mut output = String::with_capacity(8);
        for (name, character) in [
            ("hasIndices", 'd'),
            ("global", 'g'),
            ("ignoreCase", 'i'),
            ("multiline", 'm'),
            ("dotAll", 's'),
            ("unicode", 'u'),
            ("unicodeSets", 'v'),
            ("sticky", 'y'),
        ] {
            let key = self.intern_property_key(name)?;
            let value = match self.get_property_in_realm(realm, object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            if value.to_boolean() {
                output.push(character);
            }
        }
        Ok(Completion::Return(Value::String(JsString::try_from_utf8(
            &output,
        )?)))
    }
}

fn regexp_flag_mask(flag: RegExpFlagKind) -> RegExpFlags {
    match flag {
        RegExpFlagKind::HasIndices => RegExpFlags::HAS_INDICES,
        RegExpFlagKind::Global => RegExpFlags::GLOBAL,
        RegExpFlagKind::IgnoreCase => RegExpFlags::IGNORE_CASE,
        RegExpFlagKind::Multiline => RegExpFlags::MULTILINE,
        RegExpFlagKind::DotAll => RegExpFlags::DOT_ALL,
        RegExpFlagKind::Unicode => RegExpFlags::UNICODE,
        RegExpFlagKind::UnicodeSets => RegExpFlags::UNICODE_SETS,
        RegExpFlagKind::Sticky => RegExpFlags::STICKY,
    }
}

/// Escape the exact source spelling used by pinned `js_regexp_get_source`.
/// It intentionally leaves U+2028/U+2029 untouched because the pinned C loop
/// only rewrites LF and CR.
fn escape_regexp_source(pattern: &JsString) -> Result<JsString, JsStringError> {
    const BACKSLASH: u16 = b'\\' as u16;
    const CLOSE_BRACKET: u16 = b']' as u16;
    const OPEN_BRACKET: u16 = b'[' as u16;
    const LINE_FEED: u16 = b'\n' as u16;
    const CARRIAGE_RETURN: u16 = b'\r' as u16;
    const SLASH: u16 = b'/' as u16;

    let units = pattern.utf16_units().collect::<Vec<_>>();
    let mut output = JsStringBuilder::new(units.len());
    let mut in_class = false;
    let mut index = 0;
    while index < units.len() {
        let mut first = units[index];
        index += 1;
        let mut second = None;
        match first {
            BACKSLASH => {
                if let Some(next) = units.get(index).copied() {
                    second = Some(next);
                    index += 1;
                }
            }
            CLOSE_BRACKET => in_class = false,
            OPEN_BRACKET if !in_class => {
                if units.get(index).copied() == Some(CLOSE_BRACKET) {
                    second = Some(CLOSE_BRACKET);
                    index += 1;
                }
                in_class = true;
            }
            LINE_FEED => {
                first = BACKSLASH;
                second = Some(u16::from(b'n'));
            }
            CARRIAGE_RETURN => {
                first = BACKSLASH;
                second = Some(u16::from(b'r'));
            }
            SLASH if !in_class => {
                first = BACKSLASH;
                second = Some(SLASH);
            }
            _ => {}
        }
        output.push_code_point(u32::from(first))?;
        if let Some(second) = second {
            output.push_code_point(u32::from(second))?;
        }
    }
    output.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_escaping_tracks_classes_and_escaped_brackets() {
        let pattern = JsString::try_from_utf8("a/b[/]\\[/\n\r").unwrap();
        assert_eq!(
            escape_regexp_source(&pattern).unwrap().to_utf8_lossy(),
            "a\\/b[/]\\[\\/\\n\\r"
        );
        assert_eq!(
            escape_regexp_source(&JsString::try_from_utf8("[]/]").unwrap())
                .unwrap()
                .to_utf8_lossy(),
            "[]/]"
        );
    }
}
