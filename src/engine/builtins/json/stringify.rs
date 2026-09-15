//! Pinned QuickJS `JSON.stringify` traversal and quoting semantics.
//!
//! The serializer keeps its own ancestor stack, property-list snapshot and
//! UTF-16 output buffer. User callbacks always cross the normal runtime call
//! boundary, so `toJSON`, replacer and accessor throws retain their JavaScript
//! values and defining realms.

use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::atom::AtomError;
use crate::engine::heap::{ContextId, HeapError, ObjectPayload, PrimitiveObjectData};
use crate::engine::object::{CallableRef, ObjectRef, PropertyKey};
use crate::engine::value::{JsString, JsStringBuilder, JsStringError, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::NativeArguments;

enum JsonStringifyFailure {
    Throw(Value),
    Runtime(RuntimeError),
}

impl From<RuntimeError> for JsonStringifyFailure {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<JsStringError> for JsonStringifyFailure {
    fn from(error: JsStringError) -> Self {
        Self::Runtime(error.into())
    }
}

impl From<AtomError> for JsonStringifyFailure {
    fn from(error: AtomError) -> Self {
        Self::Runtime(error.into())
    }
}

impl From<HeapError> for JsonStringifyFailure {
    fn from(error: HeapError) -> Self {
        Self::Runtime(error.into())
    }
}

impl From<Error> for JsonStringifyFailure {
    fn from(error: Error) -> Self {
        Self::Runtime(error.into())
    }
}

type JsonStringifyResult<T> = Result<T, JsonStringifyFailure>;

#[derive(Clone)]
enum JsonWrapperKind {
    String,
    Number,
    Boolean(bool),
    BigInt(crate::engine::value::bigint::JsBigInt),
    Other,
}

mod operation;
pub(crate) use operation::{StringifyResume, StringifyStep};

pub(crate) struct JsonStringifier {
    realm: ContextId,
    replacer: Option<CallableRef>,
    property_list: Option<Vec<JsString>>,
    gap: JsString,
    to_json_key: Option<PropertyKey>,
    stack: Vec<ObjectRef>,
    output: JsStringBuilder,
    tasks: Vec<JsonSerializeTask>,
    root: Value,
    space: Value,
}
enum JsonSerializeTask {
    Value {
        value: Value,
        indent: JsString,
    },
    ArrayElement {
        array: ObjectRef,
        index: u64,
        length: u64,
        indent: JsString,
        next_indent: JsString,
    },
    ObjectProperty {
        object: ObjectRef,
        keys: Vec<JsString>,
        index: usize,
        has_content: bool,
        indent: JsString,
        next_indent: JsString,
    },
}
impl Runtime {
    pub(crate) fn call_json_stringify(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        operation::finish(self, realm, StringifyStep::start(self, realm, arguments)?)
    }
    fn json_wrapper_kind(&self, object: &ObjectRef) -> Result<JsonWrapperKind, RuntimeError> {
        let state = self.0.state.borrow();
        Ok(match &state.heap.object(object.object_id())?.payload {
            ObjectPayload::Primitive(PrimitiveObjectData::String(_)) => JsonWrapperKind::String,
            ObjectPayload::Primitive(PrimitiveObjectData::Number(_)) => JsonWrapperKind::Number,
            ObjectPayload::Primitive(PrimitiveObjectData::Boolean(value)) => {
                JsonWrapperKind::Boolean(*value)
            }
            ObjectPayload::Primitive(PrimitiveObjectData::BigInt(value)) => {
                JsonWrapperKind::BigInt(value.clone())
            }
            ObjectPayload::Ordinary
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
            | ObjectPayload::Primitive(PrimitiveObjectData::Symbol(_))
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::ArrayBuffer(_)
            | ObjectPayload::SharedArrayBuffer(_)
            | ObjectPayload::DataView(_)
            | ObjectPayload::TypedArray(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::AsyncFunctionState(_)
            | ObjectPayload::Generator { .. }
            | ObjectPayload::AsyncGenerator(_) => JsonWrapperKind::Other,
        })
    }
}
impl JsonStringifier {
    fn pop_object(&mut self, expected: &ObjectRef) {
        let popped = self.stack.pop();
        debug_assert!(popped.as_ref().is_some_and(|value| value == expected));
    }
    fn append_separator(&mut self, indent: &JsString) -> JsonStringifyResult<()> {
        if !self.gap.is_empty() {
            self.output.push_utf8("\n")?;
            self.output.push_js_string(indent)?;
        }
        Ok(())
    }

    fn append_quoted(&mut self, value: &JsString) -> JsonStringifyResult<()> {
        self.output.push_utf8("\"")?;
        let mut units = value.utf16_units().peekable();
        while let Some(unit) = units.next() {
            match unit {
                0x0008 => self.output.push_utf8("\\b")?,
                0x0009 => self.output.push_utf8("\\t")?,
                0x000a => self.output.push_utf8("\\n")?,
                0x000c => self.output.push_utf8("\\f")?,
                0x000d => self.output.push_utf8("\\r")?,
                0x0022 => self.output.push_utf8("\\\"")?,
                0x005c => self.output.push_utf8("\\\\")?,
                0x0000..=0x001f | 0xd800..=0xdfff => {
                    if (0xd800..=0xdbff).contains(&unit)
                        && units
                            .peek()
                            .is_some_and(|next| (0xdc00..=0xdfff).contains(next))
                    {
                        self.output.push_code_point(u32::from(unit))?;
                        self.output.push_code_point(u32::from(
                            units.next().expect("peeked low surrogate disappeared"),
                        ))?;
                    } else {
                        self.output.push_utf8(&format!("\\u{unit:04x}"))?;
                    }
                }
                _ => self.output.push_code_point(u32::from(unit))?,
            }
        }
        self.output.push_utf8("\"")?;
        Ok(())
    }
}
