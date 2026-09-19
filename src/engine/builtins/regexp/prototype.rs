//! `%RegExp.prototype%` accessors and generic `toString`.

use crate::engine::api::error::NativeErrorKind;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::builtins::native::{RegExpFlagKind, RegExpNativeKind};
use crate::engine::heap::{ContextId, ObjectPayload, RegExpObjectData};
use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, JsStringBuilder, JsStringError, Value};
use crate::engine::vm::call::NativeInvocation;
use crate::engine::vm::{Completion, ToPrimitiveHint};
use crate::regexp::RegExpFlags;

impl Runtime {
    pub(crate) fn call_regexp_accessor(
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

    pub(crate) fn call_regexp_to_string(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        finish_presentation(
            self,
            realm,
            RegExpPresentationStep::start(self, realm, RegExpNativeKind::ToString, &invocation)?,
        )
    }

    fn call_regexp_source(
        &self,
        realm: ContextId,
        this_value: &Value,
    ) -> Result<Completion, RuntimeError> {
        let Value::Object(object) = this_value else {
            return Ok(Completion::Throw(self.new_native_error_jsvalue(
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
                | ObjectPayload::ArrayBuffer(_)
                | ObjectPayload::SharedArrayBuffer(_)
                | ObjectPayload::DataView(_)
                | ObjectPayload::TypedArray(_)
                | ObjectPayload::Proxy(_)
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::AsyncFromSyncIterator(_)
                | ObjectPayload::IteratorConcat(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::WeakMap { .. }
                | ObjectPayload::WeakSet { .. }
                | ObjectPayload::WeakRef { .. }
                | ObjectPayload::FinalizationRegistry(_)
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
                | ObjectPayload::AsyncFunctionState(_)
                | ObjectPayload::Generator { .. }
                | ObjectPayload::AsyncGenerator(_) => None,
            }
        };
        let Some(pattern) = pattern else {
            return Ok(Completion::Throw(self.new_native_error_jsvalue(
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
            return Ok(Completion::Throw(self.new_native_error_jsvalue(
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
                | ObjectPayload::ArrayBuffer(_)
                | ObjectPayload::SharedArrayBuffer(_)
                | ObjectPayload::DataView(_)
                | ObjectPayload::TypedArray(_)
                | ObjectPayload::Proxy(_)
                | ObjectPayload::RawJson
                | ObjectPayload::Promise(_)
                | ObjectPayload::Array { .. }
                | ObjectPayload::Arguments { .. }
                | ObjectPayload::ArrayIterator { .. }
                | ObjectPayload::IteratorHelper(_)
                | ObjectPayload::IteratorWrap(_)
                | ObjectPayload::AsyncFromSyncIterator(_)
                | ObjectPayload::IteratorConcat(_)
                | ObjectPayload::Map { .. }
                | ObjectPayload::MapIterator { .. }
                | ObjectPayload::Set { .. }
                | ObjectPayload::WeakMap { .. }
                | ObjectPayload::WeakSet { .. }
                | ObjectPayload::WeakRef { .. }
                | ObjectPayload::FinalizationRegistry(_)
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
                | ObjectPayload::AsyncFunctionState(_)
                | ObjectPayload::Generator { .. }
                | ObjectPayload::AsyncGenerator(_) => None,
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
        Ok(Completion::Throw(self.new_native_error_jsvalue(
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
        finish_presentation(
            self,
            realm,
            RegExpPresentationStep::start(
                self,
                realm,
                RegExpNativeKind::Flags,
                &NativeInvocation::Getter {
                    this_value: this_value.clone(),
                },
            )?,
        )
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

const FLAG_PROPERTIES: [(&str, char); 8] = [
    ("hasIndices", 'd'),
    ("global", 'g'),
    ("ignoreCase", 'i'),
    ("multiline", 'm'),
    ("dotAll", 's'),
    ("unicode", 'u'),
    ("unicodeSets", 'v'),
    ("sticky", 'y'),
];
pub(crate) enum RegExpPresentationStep {
    Complete(Completion),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: RegExpPresentationResume,
    },
    Primitive {
        value: Value,
        resume: RegExpPresentationResume,
    },
}
pub(crate) struct RegExpPresentationResume(Box<RegExpPresentationResumeState>);
impl std::ops::Deref for RegExpPresentationResume {
    type Target = RegExpPresentationResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpPresentationResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpPresentationResume>() <= 8);
pub(crate) struct RegExpPresentationResumeState {
    realm: ContextId,
    object: ObjectRef,
    phase: PresentationPhase,
}
enum PresentationPhase {
    Source,
    SourceString,
    Flags(JsStringBuilder),
    FlagsString(JsStringBuilder),
    Flag { index: usize, output: String },
}
impl RegExpPresentationStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: RegExpNativeKind,
        invocation: &NativeInvocation,
    ) -> Result<Self, RuntimeError> {
        let this_value = match (kind, invocation) {
            (RegExpNativeKind::ToString, NativeInvocation::Call { this_value }) => this_value,
            (
                RegExpNativeKind::Flags | RegExpNativeKind::Source | RegExpNativeKind::Flag(_),
                NativeInvocation::Getter { this_value },
            ) => this_value,
            _ => {
                return Err(RuntimeError::Invariant(
                    "RegExp presentation received an invalid invocation",
                ));
            }
        };
        if matches!(kind, RegExpNativeKind::Source) {
            return Ok(Self::Complete(
                runtime.call_regexp_source(realm, this_value)?,
            ));
        }
        if let RegExpNativeKind::Flag(flag) = kind {
            return Ok(Self::Complete(
                runtime.call_regexp_flag(realm, this_value, flag)?,
            ));
        }
        let Value::Object(object) = this_value else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not an object")?,
            )));
        };
        let (phase, name) = if matches!(kind, RegExpNativeKind::ToString) {
            (PresentationPhase::Source, "source")
        } else {
            (
                PresentationPhase::Flag {
                    index: 0,
                    output: String::with_capacity(8),
                },
                FLAG_PROPERTIES[0].0,
            )
        };
        Ok(Self::Read {
            object: object.clone(),
            key: runtime.intern_property_key(name)?,
            resume: RegExpPresentationResume(Box::new(RegExpPresentationResumeState {
                realm,
                object: object.clone(),
                phase,
            })),
        })
    }
}
impl RegExpPresentationResume {
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpPresentationStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpPresentationStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            PresentationPhase::Source => Ok(RegExpPresentationStep::Primitive {
                value,
                resume: {
                    let updated_0 = PresentationPhase::SourceString;
                    self.0.phase = updated_0;
                    self
                },
            }),
            PresentationPhase::Flags(output) => Ok(RegExpPresentationStep::Primitive {
                value,
                resume: {
                    let updated_0 = PresentationPhase::FlagsString(output);
                    self.0.phase = updated_0;
                    self
                },
            }),
            PresentationPhase::SourceString => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp source conversion returned an object",
                    ));
                }
                let source = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpPresentationStep::Complete(Completion::Throw(value)));
                    }
                };
                let mut output = JsStringBuilder::new(source.len().saturating_add(2));
                output.push_utf8("/")?;
                output.push_js_string(&source)?;
                output.push_utf8("/")?;
                Ok(RegExpPresentationStep::Read {
                    object: self.0.object.clone(),
                    key: runtime.intern_property_key("flags")?,
                    resume: {
                        let updated_0 = PresentationPhase::Flags(output);
                        self.0.phase = updated_0;
                        self
                    },
                })
            }
            PresentationPhase::FlagsString(mut output) => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp flags conversion returned an object",
                    ));
                }
                let flags = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpPresentationStep::Complete(Completion::Throw(value)));
                    }
                };
                output.push_js_string(&flags)?;
                Ok(RegExpPresentationStep::Complete(Completion::Return(
                    Value::String(output.finish()?),
                )))
            }
            PresentationPhase::Flag { index, mut output } => {
                if runtime.value_to_boolean(&value)? {
                    output.push(FLAG_PROPERTIES[index].1);
                }
                let index = index + 1;
                if index == FLAG_PROPERTIES.len() {
                    return Ok(RegExpPresentationStep::Complete(Completion::Return(
                        Value::String(JsString::try_from_utf8(&output)?),
                    )));
                }
                Ok(RegExpPresentationStep::Read {
                    object: self.0.object.clone(),
                    key: runtime.intern_property_key(FLAG_PROPERTIES[index].0)?,
                    resume: {
                        let updated_0 = PresentationPhase::Flag { index, output };
                        self.0.phase = updated_0;
                        self
                    },
                })
            }
        }
    }
}
fn finish_presentation(
    runtime: &Runtime,
    realm: ContextId,
    mut step: RegExpPresentationStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            RegExpPresentationStep::Complete(result) => return Ok(result),
            RegExpPresentationStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            RegExpPresentationStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::String)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpPresentationStep>() <= 64);

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
