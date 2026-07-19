//! Strict, allocation-direct JSON parser from pinned QuickJS.
//!
//! This is intentionally not routed through the JavaScript lexer or an
//! external serialization crate. `JSON.parse` has a smaller lexical grammar,
//! preserves arbitrary UTF-16 code units, allocates realm-correct objects as
//! input is consumed, and records exact source spans for the reviver.

use super::super::super::*;

const MAX_JSON_PARSE_DEPTH: usize = 256;

pub(super) struct JsonParseRecord {
    original: Value,
    kind: JsonParseRecordKind,
}

enum JsonParseRecordKind {
    Primitive { start: usize, end: usize },
    Array(Vec<JsonParseRecord>),
    Object(JsonObjectParseRecord),
}

struct JsonObjectParseRecord {
    entries: Vec<JsonObjectParseRecordEntry>,
    /// Pinned QuickJS starts its record hash table while adding member nine.
    /// Linear lookup returns the first duplicate; hashed lookup returns the
    /// newest duplicate because new entries are linked at the bucket head.
    hashed: bool,
}

struct JsonObjectParseRecordEntry {
    key: PropertyKey,
    record: JsonParseRecord,
}

impl JsonParseRecord {
    pub(super) fn matches(&self, value: &Value) -> bool {
        self.original.same_value(value)
    }

    pub(super) fn primitive_span(&self) -> Option<(usize, usize)> {
        match self.kind {
            JsonParseRecordKind::Primitive { start, end } => Some((start, end)),
            JsonParseRecordKind::Array(_) | JsonParseRecordKind::Object(_) => None,
        }
    }

    pub(super) fn array_child(&self, index: usize) -> Option<&Self> {
        let JsonParseRecordKind::Array(elements) = &self.kind else {
            return None;
        };
        elements.get(index)
    }

    pub(super) fn object_child(&self, key: &PropertyKey) -> Option<&Self> {
        let JsonParseRecordKind::Object(object) = &self.kind else {
            return None;
        };
        let mut entries = object.entries.iter();
        if object.hashed {
            entries
                .rev()
                .find(|entry| &entry.key == key)
                .map(|entry| &entry.record)
        } else {
            entries
                .find(|entry| &entry.key == key)
                .map(|entry| &entry.record)
        }
    }
}

enum JsonParseFailure {
    Syntax(String),
    Runtime(RuntimeError),
}

impl From<RuntimeError> for JsonParseFailure {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

type JsonParseResult<T> = Result<T, JsonParseFailure>;

struct JsonParser<'a> {
    runtime: &'a Runtime,
    realm: ContextId,
    units: Vec<u16>,
    cursor: usize,
    retain_record: bool,
}

impl Runtime {
    pub(super) fn parse_json_text(
        &self,
        realm: ContextId,
        source: &JsString,
        retain_record: bool,
    ) -> Result<NativeConversion<(Value, Option<JsonParseRecord>)>, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let mut parser = JsonParser {
            runtime: self,
            realm,
            units: source.utf16_units().collect(),
            cursor: 0,
            retain_record,
        };
        match parser.parse_document() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(JsonParseFailure::Syntax(message)) => Ok(NativeConversion::Throw(
                self.new_native_error(realm, NativeErrorKind::Syntax, &message)?,
            )),
            Err(JsonParseFailure::Runtime(error)) => Err(error),
        }
    }
}

impl JsonParser<'_> {
    fn parse_document(&mut self) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        self.skip_whitespace();
        let result = self.parse_value(0)?;
        self.skip_whitespace();
        if self.cursor != self.units.len() {
            // QuickJS lexes the next token before reporting trailing data, so
            // malformed trailing strings/numbers retain their lexical error.
            self.validate_trailing_token()?;
            return self.syntax("unexpected data at the end");
        }
        Ok(result)
    }

    fn parse_value(&mut self, depth: usize) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        if depth > MAX_JSON_PARSE_DEPTH {
            return self.syntax("stack overflow");
        }
        self.skip_whitespace();
        let Some(unit) = self.peek() else {
            return self.syntax("Unexpected end of JSON input");
        };
        match unit {
            unit if unit == u16::from(b'{') => self.parse_object(depth),
            unit if unit == u16::from(b'[') => self.parse_array(depth),
            unit if unit == u16::from(b'"') => {
                let start = self.cursor;
                let string = self.parse_string()?;
                let end = self.cursor;
                let value = Value::String(string);
                let record = self.primitive_record(value.clone(), start, end);
                Ok((value, record))
            }
            unit if unit == u16::from(b'-') || is_ascii_digit(unit) => self.parse_number_value(),
            unit if is_ascii_identifier_start(unit) => self.parse_identifier_value(),
            unit if unit >= 0x80 => self.syntax("unexpected character"),
            _ => self.syntax(&format!("unexpected token: '{}'", display_unit(unit))),
        }
    }

    fn parse_object(&mut self, depth: usize) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        self.cursor += 1;
        let object = self.runtime.new_ordinary_object_in_realm(self.realm)?;
        let mut entries = Vec::new();
        self.skip_whitespace();
        if self.consume_ascii(b'}') {
            let value = Value::Object(object);
            let record = self.retain_record.then(|| JsonParseRecord {
                original: value.clone(),
                kind: JsonParseRecordKind::Object(JsonObjectParseRecord {
                    entries,
                    hashed: false,
                }),
            });
            return Ok((value, record));
        }

        loop {
            self.skip_whitespace();
            if self.peek() != Some(u16::from(b'"')) {
                return self.syntax("expecting property name");
            }
            let name = self.parse_string()?;
            let key = self
                .runtime
                .intern_property_key_js_string(&name)
                .map_err(RuntimeError::from)?;
            self.skip_whitespace();
            if !self.consume_ascii(b':') {
                return self.syntax("expecting ':'");
            }
            let (property_value, child_record) = self.parse_value(depth + 1)?;
            self.define_json_property(&object, &key, property_value)?;
            if let Some(record) = child_record {
                entries.push(JsonObjectParseRecordEntry { key, record });
            }

            self.skip_whitespace();
            if self.consume_ascii(b',') {
                continue;
            }
            if !self.consume_ascii(b'}') {
                return self.syntax("expecting '}'");
            }
            break;
        }

        let value = Value::Object(object);
        let record = self.retain_record.then(|| {
            let hashed = entries.len() >= 9;
            JsonParseRecord {
                original: value.clone(),
                kind: JsonParseRecordKind::Object(JsonObjectParseRecord { entries, hashed }),
            }
        });
        Ok((value, record))
    }

    fn parse_array(&mut self, depth: usize) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        self.cursor += 1;
        let array = self.runtime.new_array(self.realm)?;
        let mut elements = Vec::new();
        self.skip_whitespace();
        if self.consume_ascii(b']') {
            let value = Value::Object(array);
            let record = self.retain_record.then(|| JsonParseRecord {
                original: value.clone(),
                kind: JsonParseRecordKind::Array(elements),
            });
            return Ok((value, record));
        }

        let mut index = 0_u32;
        loop {
            let (element, child_record) = self.parse_value(depth + 1)?;
            let key = self
                .runtime
                .intern_property_key(&index.to_string())
                .map_err(RuntimeError::from)?;
            self.define_json_property(&array, &key, element)?;
            if let Some(record) = child_record {
                elements.push(record);
            }
            index = index.checked_add(1).ok_or_else(|| {
                JsonParseFailure::Runtime(RuntimeError::Engine(Error::new(
                    ErrorKind::Range,
                    "invalid array length",
                )))
            })?;

            self.skip_whitespace();
            if self.consume_ascii(b',') {
                continue;
            }
            if !self.consume_ascii(b']') {
                return self.syntax("expecting ']'");
            }
            break;
        }

        let value = Value::Object(array);
        let record = self.retain_record.then(|| JsonParseRecord {
            original: value.clone(),
            kind: JsonParseRecordKind::Array(elements),
        });
        Ok((value, record))
    }

    fn parse_string(&mut self) -> JsonParseResult<JsString> {
        debug_assert_eq!(self.peek(), Some(u16::from(b'"')));
        self.cursor += 1;
        let mut output = Vec::new();
        loop {
            let Some(unit) = self.peek() else {
                return self.syntax("Unexpected end of JSON input");
            };
            self.cursor += 1;
            match unit {
                unit if unit == u16::from(b'"') => break,
                unit if unit < 0x20 => {
                    return self.syntax("Bad control character in string literal");
                }
                unit if unit == u16::from(b'\\') => {
                    let Some(escaped) = self.peek() else {
                        return self.syntax("Unexpected end of JSON input");
                    };
                    self.cursor += 1;
                    match escaped {
                        unit if unit == u16::from(b'"')
                            || unit == u16::from(b'\\')
                            || unit == u16::from(b'/') =>
                        {
                            output.push(unit)
                        }
                        unit if unit == u16::from(b'b') => output.push(0x08),
                        unit if unit == u16::from(b'f') => output.push(0x0c),
                        unit if unit == u16::from(b'n') => output.push(0x0a),
                        unit if unit == u16::from(b'r') => output.push(0x0d),
                        unit if unit == u16::from(b't') => output.push(0x09),
                        unit if unit == u16::from(b'u') => {
                            let mut value = 0_u16;
                            for _ in 0..4 {
                                let Some(hex) = self.peek().and_then(hex_value) else {
                                    return self.syntax("Bad Unicode escape");
                                };
                                self.cursor += 1;
                                value = (value << 4) | u16::from(hex);
                            }
                            output.push(value);
                        }
                        _ => return self.syntax("Bad escaped character"),
                    }
                }
                _ => output.push(unit),
            }
        }
        Ok(JsString::from_owned_utf16(output))
    }

    fn parse_number_value(&mut self) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        let start = self.cursor;
        if self.consume_ascii(b'-') && self.peek().is_none() {
            return self.syntax("Unexpected token '");
        }
        let Some(first) = self.peek() else {
            return self.syntax("Unexpected end of JSON input");
        };
        if first == u16::from(b'0') {
            self.cursor += 1;
            if self.peek().is_some_and(is_ascii_digit) {
                return self.syntax("Unexpected number");
            }
        } else if (u16::from(b'1')..=u16::from(b'9')).contains(&first) {
            self.cursor += 1;
            while self.peek().is_some_and(is_ascii_digit) {
                self.cursor += 1;
            }
        } else {
            return self.syntax(&format!("Unexpected token '{}'", display_unit(first)));
        }

        if self.consume_ascii(b'.') {
            if !self.peek().is_some_and(is_ascii_digit) {
                return self.syntax("Unterminated fractional number");
            }
            while self.peek().is_some_and(is_ascii_digit) {
                self.cursor += 1;
            }
        }
        if self
            .peek()
            .is_some_and(|unit| unit == u16::from(b'e') || unit == u16::from(b'E'))
        {
            self.cursor += 1;
            if self
                .peek()
                .is_some_and(|unit| unit == u16::from(b'+') || unit == u16::from(b'-'))
            {
                self.cursor += 1;
            }
            if !self.peek().is_some_and(is_ascii_digit) {
                return self.syntax("Exponent part is missing a number");
            }
            while self.peek().is_some_and(is_ascii_digit) {
                self.cursor += 1;
            }
        }

        let end = self.cursor;
        let spelling = JsString::from_owned_utf16(self.units[start..end].to_vec());
        let value = Value::number(crate::number_parse::parse_float(&spelling));
        let record = self.primitive_record(value.clone(), start, end);
        Ok((value, record))
    }

    fn parse_identifier_value(&mut self) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        let start = self.cursor;
        self.cursor += 1;
        while self.peek().is_some_and(is_ascii_identifier_continue) {
            self.cursor += 1;
        }
        let end = self.cursor;
        let spelling = &self.units[start..end];
        let value = if ascii_eq(spelling, b"true") {
            Value::Bool(true)
        } else if ascii_eq(spelling, b"false") {
            Value::Bool(false)
        } else if ascii_eq(spelling, b"null") {
            Value::Null
        } else {
            let token = spelling
                .iter()
                .map(|unit| char::from_u32(u32::from(*unit)).unwrap_or('\u{fffd}'))
                .collect::<String>();
            return self.syntax(&format!("unexpected token: '{token}'"));
        };
        let record = self.primitive_record(value.clone(), start, end);
        Ok((value, record))
    }

    fn primitive_record(
        &self,
        original: Value,
        start: usize,
        end: usize,
    ) -> Option<JsonParseRecord> {
        self.retain_record.then(|| JsonParseRecord {
            original,
            kind: JsonParseRecordKind::Primitive { start, end },
        })
    }

    fn define_json_property(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> JsonParseResult<()> {
        if !self.runtime.define_own_property(
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(JsonParseFailure::Runtime(RuntimeError::Invariant(
                "fresh JSON property definition was rejected",
            )));
        }
        Ok(())
    }

    fn validate_trailing_token(&mut self) -> JsonParseResult<()> {
        let Some(unit) = self.peek() else {
            return Ok(());
        };
        if unit >= 0x80 {
            return self.syntax("unexpected character");
        }
        if unit == u16::from(b'"') {
            let _ = self.parse_string()?;
        } else if unit == u16::from(b'-') || is_ascii_digit(unit) {
            let _ = self.parse_number_value()?;
        }
        Ok(())
    }

    fn skip_whitespace(&mut self) {
        while self
            .peek()
            .is_some_and(|unit| matches!(unit, 0x09 | 0x0a | 0x0d | 0x20))
        {
            self.cursor += 1;
        }
    }

    fn consume_ascii(&mut self, byte: u8) -> bool {
        if self.peek() == Some(u16::from(byte)) {
            self.cursor += 1;
            true
        } else {
            false
        }
    }

    fn peek(&self) -> Option<u16> {
        self.units.get(self.cursor).copied()
    }

    fn syntax<T>(&self, message: &str) -> JsonParseResult<T> {
        Err(JsonParseFailure::Syntax(message.to_owned()))
    }
}

fn is_ascii_digit(unit: u16) -> bool {
    (u16::from(b'0')..=u16::from(b'9')).contains(&unit)
}

fn is_ascii_identifier_start(unit: u16) -> bool {
    unit == u16::from(b'_')
        || unit == u16::from(b'$')
        || (u16::from(b'a')..=u16::from(b'z')).contains(&unit)
        || (u16::from(b'A')..=u16::from(b'Z')).contains(&unit)
}

fn is_ascii_identifier_continue(unit: u16) -> bool {
    is_ascii_identifier_start(unit) || is_ascii_digit(unit)
}

fn hex_value(unit: u16) -> Option<u8> {
    match unit {
        unit if (u16::from(b'0')..=u16::from(b'9')).contains(&unit) => {
            Some((unit - u16::from(b'0')) as u8)
        }
        unit if (u16::from(b'a')..=u16::from(b'f')).contains(&unit) => {
            Some((unit - u16::from(b'a') + 10) as u8)
        }
        unit if (u16::from(b'A')..=u16::from(b'F')).contains(&unit) => {
            Some((unit - u16::from(b'A') + 10) as u8)
        }
        _ => None,
    }
}

fn ascii_eq(units: &[u16], bytes: &[u8]) -> bool {
    units.len() == bytes.len()
        && units
            .iter()
            .zip(bytes)
            .all(|(unit, byte)| *unit == u16::from(*byte))
}

fn display_unit(unit: u16) -> char {
    char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}')
}
