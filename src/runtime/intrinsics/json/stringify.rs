//! Pinned QuickJS `JSON.stringify` traversal and quoting semantics.
//!
//! The serializer keeps its own ancestor stack, property-list snapshot and
//! UTF-16 output buffer. User callbacks always cross the normal runtime call
//! boundary, so `toJSON`, replacer and accessor throws retain their JavaScript
//! values and defining realms.

use super::super::super::*;

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
    BigInt(crate::bigint::JsBigInt),
    Other,
}

struct JsonStringifier<'a> {
    runtime: &'a Runtime,
    realm: ContextId,
    replacer: Option<CallableRef>,
    property_list: Option<Vec<JsString>>,
    gap: JsString,
    to_json_key: PropertyKey,
    stack: Vec<ObjectRef>,
    output: JsStringBuilder,
}

enum JsonSerializeTask {
    Value {
        value: Value,
        indent: JsString,
    },
    ArrayElement {
        array: ObjectRef,
        index: u32,
        length: u32,
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
    pub(super) fn call_json_stringify(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match self.run_json_stringify(realm, arguments) {
            Ok(Some(result)) => Ok(Completion::Return(Value::String(result))),
            Ok(None) => Ok(Completion::Return(Value::Undefined)),
            Err(JsonStringifyFailure::Throw(value)) => Ok(Completion::Throw(value)),
            Err(JsonStringifyFailure::Runtime(error)) => Err(error),
        }
    }

    fn run_json_stringify(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> JsonStringifyResult<Option<JsString>> {
        self.0.state.borrow().heap.context(realm)?;
        let replacer_value = &arguments.readable[1];
        let replacer = match replacer_value {
            Value::Object(object) => self.as_callable(object)?,
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Symbol(_) => None,
        };
        let property_list = if replacer.is_none() {
            self.json_stringify_property_list(realm, replacer_value)?
        } else {
            None
        };
        let gap = self.json_stringify_gap(realm, &arguments.readable[2])?;

        // Pinned QuickJS creates the root wrapper after replacer and space
        // normalization, then exposes its configurable/enumerable/writable
        // empty-name data property to the root replacer call.
        let wrapper = self.new_ordinary_object_in_realm(realm)?;
        let empty_key = self.intern_property_key("")?;
        if !self.define_own_property(
            &wrapper,
            &empty_key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(arguments.readable[0].clone()),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "fresh JSON.stringify root definition was rejected",
            )
            .into());
        }

        let empty = JsString::from_static("");
        let mut stringifier = JsonStringifier {
            runtime: self,
            realm,
            replacer,
            property_list,
            gap,
            to_json_key: self.intern_property_key("toJSON")?,
            stack: Vec::new(),
            output: JsStringBuilder::new(256),
        };
        let Some(value) =
            stringifier.check_value(&wrapper, arguments.readable[0].clone(), &empty)?
        else {
            return Ok(None);
        };
        stringifier.serialize_value(value, empty)?;
        Ok(Some(stringifier.output.finish()?))
    }

    fn json_stringify_property_list(
        &self,
        realm: ContextId,
        replacer: &Value,
    ) -> JsonStringifyResult<Option<Vec<JsString>>> {
        let Value::Object(object) = replacer else {
            return Ok(None);
        };
        if !self.is_array_object(object)? {
            return Ok(None);
        }
        let (length, _) = self.array_length_state(object)?;
        let mut property_list = Vec::new();
        for index in 0..length {
            let key = self.intern_property_key(&index.to_string())?;
            let value = match self.get_property_in_realm(realm, object, &key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
            };
            let item = match value {
                value @ (Value::String(_) | Value::Int(_) | Value::Float(_)) => {
                    Some(self.json_stringify_to_string(realm, &value)?)
                }
                Value::Object(item) => match self.json_wrapper_kind(&item)? {
                    JsonWrapperKind::String | JsonWrapperKind::Number => {
                        Some(self.json_stringify_to_string(realm, &Value::Object(item))?)
                    }
                    JsonWrapperKind::Boolean(_)
                    | JsonWrapperKind::BigInt(_)
                    | JsonWrapperKind::Other => None,
                },
                Value::Undefined
                | Value::Null
                | Value::Bool(_)
                | Value::BigInt(_)
                | Value::Symbol(_) => None,
            };
            if let Some(item) = item
                && !property_list.iter().any(|present| present == &item)
            {
                property_list.push(item);
            }
        }
        Ok(Some(property_list))
    }

    fn json_stringify_gap(&self, realm: ContextId, space: &Value) -> JsonStringifyResult<JsString> {
        let normalized = match space {
            Value::Object(object) => match self.json_wrapper_kind(object)? {
                JsonWrapperKind::Number => {
                    Some(Value::number(self.json_stringify_to_number(realm, space)?))
                }
                JsonWrapperKind::String => {
                    Some(Value::String(self.json_stringify_to_string(realm, space)?))
                }
                JsonWrapperKind::Boolean(_)
                | JsonWrapperKind::BigInt(_)
                | JsonWrapperKind::Other => None,
            },
            Value::Int(_) | Value::Float(_) | Value::String(_) => Some(space.clone()),
            Value::Undefined
            | Value::Null
            | Value::Bool(_)
            | Value::BigInt(_)
            | Value::Symbol(_) => None,
        };
        match normalized {
            Some(Value::Int(value)) => {
                let count = value.clamp(0, 10) as usize;
                Ok(JsString::try_from_utf8(&" ".repeat(count))?)
            }
            Some(Value::Float(value)) => {
                let count = if value.is_nan() || value <= 0.0 {
                    0
                } else if value >= 10.0 {
                    10
                } else {
                    value.trunc() as usize
                };
                Ok(JsString::try_from_utf8(&" ".repeat(count))?)
            }
            Some(Value::String(value)) => Ok(value.sub_string(0, value.len().min(10))),
            Some(
                Value::Undefined
                | Value::Null
                | Value::Bool(_)
                | Value::BigInt(_)
                | Value::Symbol(_)
                | Value::Object(_),
            )
            | None => Ok(JsString::from_static("")),
        }
    }

    fn json_stringify_to_string(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> JsonStringifyResult<JsString> {
        match self.native_to_js_string(realm, value)? {
            NativeConversion::Value(value) => Ok(value),
            NativeConversion::Throw(value) => Err(JsonStringifyFailure::Throw(value)),
        }
    }

    fn json_stringify_to_number(
        &self,
        realm: ContextId,
        value: &Value,
    ) -> JsonStringifyResult<f64> {
        match self.native_to_number(realm, value)? {
            NativeConversion::Value(value) => Ok(value),
            NativeConversion::Throw(value) => Err(JsonStringifyFailure::Throw(value)),
        }
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
            | ObjectPayload::RawJson
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(PrimitiveObjectData::Symbol(_))
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => JsonWrapperKind::Other,
        })
    }
}

impl JsonStringifier<'_> {
    fn check_value(
        &self,
        holder: &ObjectRef,
        mut value: Value,
        key: &JsString,
    ) -> JsonStringifyResult<Option<Value>> {
        if matches!(value, Value::Object(_) | Value::BigInt(_)) {
            let method = match self.runtime.get_value_property_in_realm(
                self.realm,
                value.clone(),
                &self.to_json_key,
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
            };
            if let Value::Object(object) = method
                && let Some(callable) = self.runtime.as_callable(&object)?
            {
                value = match self.runtime.call_internal(
                    self.realm,
                    &callable,
                    value,
                    &[Value::String(key.clone())],
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
                };
            }
        }

        if let Some(replacer) = &self.replacer {
            value = match self.runtime.call_internal(
                self.realm,
                replacer,
                Value::Object(holder.clone()),
                &[Value::String(key.clone()), value],
            )? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
            };
        }

        match &value {
            Value::Undefined | Value::Symbol(_) => Ok(None),
            Value::Object(object) if self.runtime.as_callable(object)?.is_some() => Ok(None),
            Value::Null
            | Value::Bool(_)
            | Value::Int(_)
            | Value::Float(_)
            | Value::BigInt(_)
            | Value::String(_)
            | Value::Object(_) => Ok(Some(value)),
        }
    }

    fn serialize_value(&mut self, value: Value, indent: JsString) -> JsonStringifyResult<()> {
        let mut tasks = vec![JsonSerializeTask::Value { value, indent }];
        while let Some(task) = tasks.pop() {
            match task {
                JsonSerializeTask::Value { mut value, indent } => loop {
                    match value {
                        Value::String(value) => {
                            self.append_quoted(&value)?;
                            break;
                        }
                        Value::Int(value) => {
                            self.output.push_utf8(&value.to_string())?;
                            break;
                        }
                        Value::Float(value) if value.is_finite() => {
                            let string = Value::Float(value).to_js_string()?;
                            self.output.push_js_string(&string)?;
                            break;
                        }
                        Value::Float(_) => {
                            self.output.push_utf8("null")?;
                            break;
                        }
                        Value::Bool(true) => {
                            self.output.push_utf8("true")?;
                            break;
                        }
                        Value::Bool(false) => {
                            self.output.push_utf8("false")?;
                            break;
                        }
                        Value::Null => {
                            self.output.push_utf8("null")?;
                            break;
                        }
                        Value::BigInt(_) => {
                            return self.throw_native(
                                NativeErrorKind::Type,
                                "Do not know how to serialize a BigInt",
                            );
                        }
                        Value::Object(object) => {
                            if self.runtime.is_raw_json_object(&object)? {
                                self.serialize_raw_json(&object)?;
                                break;
                            }
                            match self.runtime.json_wrapper_kind(&object)? {
                                JsonWrapperKind::String => {
                                    let value = self.runtime.json_stringify_to_string(
                                        self.realm,
                                        &Value::Object(object),
                                    )?;
                                    self.append_quoted(&value)?;
                                    break;
                                }
                                JsonWrapperKind::Number => {
                                    let number = self.runtime.json_stringify_to_number(
                                        self.realm,
                                        &Value::Object(object),
                                    )?;
                                    value = Value::number(number);
                                }
                                JsonWrapperKind::Boolean(boolean) => {
                                    value = Value::Bool(boolean);
                                }
                                JsonWrapperKind::BigInt(bigint) => {
                                    value = Value::BigInt(bigint);
                                }
                                JsonWrapperKind::Other => {
                                    self.begin_object(object, indent, &mut tasks)?;
                                    break;
                                }
                            }
                        }
                        Value::Undefined | Value::Symbol(_) => break,
                    }
                },
                JsonSerializeTask::ArrayElement {
                    array,
                    index,
                    length,
                    indent,
                    next_indent,
                } => {
                    self.serialize_array_element(
                        array,
                        index,
                        length,
                        indent,
                        next_indent,
                        &mut tasks,
                    )?;
                }
                JsonSerializeTask::ObjectProperty {
                    object,
                    keys,
                    index,
                    has_content,
                    indent,
                    next_indent,
                } => {
                    self.serialize_object_property(
                        object,
                        keys,
                        index,
                        has_content,
                        indent,
                        next_indent,
                        &mut tasks,
                    )?;
                }
            }
        }
        Ok(())
    }

    fn begin_object(
        &mut self,
        object: ObjectRef,
        indent: JsString,
        tasks: &mut Vec<JsonSerializeTask>,
    ) -> JsonStringifyResult<()> {
        if self.stack.iter().any(|ancestor| ancestor == &object) {
            return self.throw_native(NativeErrorKind::Type, "circular reference");
        }
        let next_indent = indent.try_concat(&self.gap)?;
        self.stack.push(object.clone());
        if self.runtime.is_array_object(&object)? {
            let (length, _) = self.runtime.array_length_state(&object)?;
            self.output.push_utf8("[")?;
            tasks.push(JsonSerializeTask::ArrayElement {
                array: object,
                index: 0,
                length,
                indent,
                next_indent,
            });
        } else {
            let keys = self.object_keys(&object)?;
            self.output.push_utf8("{")?;
            tasks.push(JsonSerializeTask::ObjectProperty {
                object,
                keys,
                index: 0,
                has_content: false,
                indent,
                next_indent,
            });
        }
        Ok(())
    }

    fn serialize_raw_json(&mut self, object: &ObjectRef) -> JsonStringifyResult<()> {
        let key = self.runtime.intern_property_key("rawJSON")?;
        let value = match self
            .runtime
            .get_property_in_realm(self.realm, object, &key)?
        {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
        };
        let Value::String(source) = value else {
            return Err(
                RuntimeError::Invariant("Raw JSON branded object lost its source string").into(),
            );
        };
        self.output.push_js_string(&source)?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn serialize_array_element(
        &mut self,
        array: ObjectRef,
        index: u32,
        length: u32,
        indent: JsString,
        next_indent: JsString,
        tasks: &mut Vec<JsonSerializeTask>,
    ) -> JsonStringifyResult<()> {
        if index == length {
            if length > 0 && !self.gap.is_empty() {
                self.output.push_utf8("\n")?;
                self.output.push_js_string(&indent)?;
            }
            self.output.push_utf8("]")?;
            self.pop_object(&array);
            return Ok(());
        }
        if index > 0 {
            self.output.push_utf8(",")?;
        }
        self.append_separator(&next_indent)?;
        let name = index.to_string();
        let key = self.runtime.intern_property_key(&name)?;
        let value = match self
            .runtime
            .get_property_in_realm(self.realm, &array, &key)?
        {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
        };
        let name = JsString::try_from_utf8(&name)?;
        let value = self.check_value(&array, value, &name)?;
        tasks.push(JsonSerializeTask::ArrayElement {
            array,
            index: index + 1,
            length,
            indent,
            next_indent: next_indent.clone(),
        });
        match value {
            Some(value) => tasks.push(JsonSerializeTask::Value {
                value,
                indent: next_indent,
            }),
            None => self.output.push_utf8("null")?,
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn serialize_object_property(
        &mut self,
        object: ObjectRef,
        keys: Vec<JsString>,
        mut index: usize,
        has_content: bool,
        indent: JsString,
        next_indent: JsString,
        tasks: &mut Vec<JsonSerializeTask>,
    ) -> JsonStringifyResult<()> {
        while let Some(name) = keys.get(index) {
            index += 1;
            let key = self.runtime.intern_property_key_js_string(name)?;
            let value = match self
                .runtime
                .get_property_in_realm(self.realm, &object, &key)?
            {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Err(JsonStringifyFailure::Throw(value)),
            };
            let Some(value) = self.check_value(&object, value, name)? else {
                continue;
            };
            if has_content {
                self.output.push_utf8(",")?;
            }
            self.append_separator(&next_indent)?;
            self.append_quoted(name)?;
            self.output.push_utf8(":")?;
            if !self.gap.is_empty() {
                self.output.push_utf8(" ")?;
            }
            tasks.push(JsonSerializeTask::ObjectProperty {
                object,
                keys,
                index,
                has_content: true,
                indent,
                next_indent: next_indent.clone(),
            });
            tasks.push(JsonSerializeTask::Value {
                value,
                indent: next_indent,
            });
            return Ok(());
        }
        if has_content && !self.gap.is_empty() {
            self.output.push_utf8("\n")?;
            self.output.push_js_string(&indent)?;
        }
        self.output.push_utf8("}")?;
        self.pop_object(&object);
        Ok(())
    }

    fn pop_object(&mut self, expected: &ObjectRef) {
        let popped = self.stack.pop();
        debug_assert!(popped.as_ref().is_some_and(|value| value == expected));
    }

    fn object_keys(&self, object: &ObjectRef) -> JsonStringifyResult<Vec<JsString>> {
        if let Some(property_list) = &self.property_list {
            return Ok(property_list.clone());
        }
        let mut result = Vec::new();
        for key in self.runtime.own_property_keys(object)? {
            if self
                .runtime
                .0
                .state
                .borrow()
                .atoms
                .property_key_kind(key.atom())?
                != PropertyKeyKind::String
            {
                continue;
            }
            let Some(descriptor) = self.runtime.get_own_property(object, &key)? else {
                continue;
            };
            let enumerable = match descriptor {
                CompleteOrdinaryPropertyDescriptor::Data { enumerable, .. }
                | CompleteOrdinaryPropertyDescriptor::Accessor { enumerable, .. } => enumerable,
            };
            if enumerable {
                result.push(
                    self.runtime
                        .0
                        .state
                        .borrow()
                        .atoms
                        .to_js_string(key.atom())?,
                );
            }
        }
        Ok(result)
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

    fn throw_native<T>(&self, kind: NativeErrorKind, message: &str) -> JsonStringifyResult<T> {
        Err(JsonStringifyFailure::Throw(
            self.runtime.new_native_error(self.realm, kind, message)?,
        ))
    }
}
