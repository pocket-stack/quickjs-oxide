//! Strict, allocation-direct JSON parser from pinned QuickJS.
//!
//! This is intentionally not routed through the JavaScript lexer or an
//! external serialization crate. `JSON.parse` has a smaller lexical grammar,
//! preserves arbitrary UTF-16 code units, allocates realm-correct objects as
//! input is consumed, and records exact source spans for the reviver.

use super::super::super::*;

const MAX_JSON_PARSE_DEPTH: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum JsonParseMode {
    Strict,
    QuickJsExtended,
}

impl JsonParseMode {
    const fn is_extended(self) -> bool {
        matches!(self, Self::QuickJsExtended)
    }
}

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

struct JsonSyntaxFailure {
    message: String,
    /// UTF-16 source-unit offset used as QuickJS's `token.ptr` equivalent.
    offset: usize,
}

enum JsonParseFailure {
    Syntax(JsonSyntaxFailure),
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
    /// UTF-16-unit offsets whose DEL carrier represents one malformed source
    /// byte. Genuine U+007F input has no entry and therefore keeps its normal
    /// JSON string semantics.
    invalid_unit_offsets: Vec<usize>,
    /// The explicitly sized host buffer used by JSON module parsing. Keeping
    /// this borrowed rather than copying it lets diagnostics recover the
    /// exact QuickJS byte column, including CESU-8 and malformed sequences.
    raw_source: Option<&'a [u8]>,
    cursor: usize,
    retain_record: bool,
    mode: JsonParseMode,
}

#[derive(Clone, Copy)]
enum JsonModuleSource<'source> {
    Text(&'source JsString),
    Bytes(&'source [u8]),
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
            invalid_unit_offsets: Vec::new(),
            raw_source: None,
            cursor: 0,
            retain_record,
            mode: JsonParseMode::Strict,
        };
        match parser.parse_document() {
            Ok(value) => Ok(NativeConversion::Value(value)),
            Err(JsonParseFailure::Syntax(failure)) => {
                // `js_json_parse` passes the synthetic filename `<input>` to
                // `JS_ParseJSON3`.  Its `js_parse_error_v` path constructs the
                // SyntaxError without a backtrace, then prepends that exact
                // token location before the active native/bytecode frames.
                let position = parser.source_location(failure.offset)?;
                let exception = self.new_native_error_without_backtrace_from_error(
                    realm,
                    NativeErrorKind::Syntax,
                    &Error::new(ErrorKind::Syntax, failure.message),
                )?;
                self.ensure_error_backtrace(
                    &exception,
                    false,
                    Some(ExplicitBacktraceLocation {
                        filename: JsString::from_static("<input>"),
                        position,
                    }),
                )?;
                Ok(NativeConversion::Throw(exception))
            }
            Err(JsonParseFailure::Runtime(error)) => Err(error),
        }
    }

    /// Parse one strict JSON module payload while retaining the pinned
    /// QuickJS filename and token-start diagnostic location.
    pub(in crate::runtime) fn parse_json_module_text(
        &self,
        realm: ContextId,
        source: &JsString,
        filename: &JsString,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        self.parse_json_module_source_with_mode(
            realm,
            JsonModuleSource::Text(source),
            filename,
            JsonParseMode::Strict,
        )
    }

    /// Parse one strict JSON module from an explicitly sized byte buffer.
    ///
    /// Unlike an intermediate [`JsString`], this preserves malformed UTF-8,
    /// surrogate encodings, and byte-oriented QuickJS diagnostics.
    pub(in crate::runtime) fn parse_json_module_bytes(
        &self,
        realm: ContextId,
        source: &[u8],
        filename: &JsString,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        self.parse_json_module_source_with_mode(
            realm,
            JsonModuleSource::Bytes(source),
            filename,
            JsonParseMode::Strict,
        )
    }

    /// Parse one host-selected QuickJS extended-JSON module payload.
    ///
    /// This is deliberately separate from `JSON.parse` and strict JSON
    /// modules: only an attributes-aware host loader may select this mode.
    pub(in crate::runtime) fn parse_json5_module_text(
        &self,
        realm: ContextId,
        source: &JsString,
        filename: &JsString,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        self.parse_json_module_source_with_mode(
            realm,
            JsonModuleSource::Text(source),
            filename,
            JsonParseMode::QuickJsExtended,
        )
    }

    /// Parse one host-selected QuickJS extended-JSON module from an explicitly
    /// sized byte buffer.
    pub(in crate::runtime) fn parse_json5_module_bytes(
        &self,
        realm: ContextId,
        source: &[u8],
        filename: &JsString,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        self.parse_json_module_source_with_mode(
            realm,
            JsonModuleSource::Bytes(source),
            filename,
            JsonParseMode::QuickJsExtended,
        )
    }

    fn parse_json_module_source_with_mode<'source>(
        &self,
        realm: ContextId,
        source: JsonModuleSource<'source>,
        filename: &JsString,
        mode: JsonParseMode,
    ) -> Result<NativeConversion<Value>, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let mut parser = match source {
            JsonModuleSource::Text(source) => JsonParser {
                runtime: self,
                realm,
                units: source.utf16_units().collect(),
                invalid_unit_offsets: Vec::new(),
                raw_source: None,
                cursor: 0,
                retain_record: false,
                mode,
            },
            JsonModuleSource::Bytes(source) => {
                JsonParser::try_from_raw_bytes(self, realm, source, mode)?
            }
        };
        match parser.parse_document() {
            Ok((value, None)) => Ok(NativeConversion::Value(value)),
            Ok((_, Some(_))) => Err(RuntimeError::Invariant(
                "JSON module parsing unexpectedly retained a parse record",
            )),
            Err(JsonParseFailure::Syntax(failure)) => {
                let position = parser.source_location(failure.offset)?;
                let exception = self.new_native_error_without_backtrace_from_error(
                    realm,
                    NativeErrorKind::Syntax,
                    &Error::new(ErrorKind::Syntax, failure.message),
                )?;
                self.ensure_error_backtrace(
                    &exception,
                    false,
                    Some(ExplicitBacktraceLocation {
                        filename: filename.clone(),
                        position,
                    }),
                )?;
                Ok(NativeConversion::Throw(exception))
            }
            Err(JsonParseFailure::Runtime(error)) => Err(error),
        }
    }
}

impl<'a> JsonParser<'a> {
    fn try_from_raw_bytes(
        runtime: &'a Runtime,
        realm: ContextId,
        source: &'a [u8],
        mode: JsonParseMode,
    ) -> Result<Self, RuntimeError> {
        let mut units = Vec::new();
        units
            .try_reserve_exact(source.len())
            .map_err(|_| JsStringError::OutOfMemory)?;
        let mut invalid_unit_offsets = Vec::new();
        let mut byte_offset = 0;
        while byte_offset < source.len() {
            let byte = source[byte_offset];
            if byte < 0x80 {
                units.push(u16::from(byte));
                byte_offset += 1;
                continue;
            }

            match crate::value::decode_quickjs_utf8(&source[byte_offset..]) {
                Some((code_point, consumed)) if code_point <= 0x10_ffff => {
                    if code_point <= 0xffff {
                        units.push(code_point as u16);
                    } else {
                        let scalar = code_point - 0x1_0000;
                        units.push(0xd800 | ((scalar >> 10) as u16));
                        units.push(0xdc00 | ((scalar & 0x3ff) as u16));
                    }
                    byte_offset += consumed;
                }
                Some(_) | None => {
                    invalid_unit_offsets
                        .try_reserve(1)
                        .map_err(|_| JsStringError::OutOfMemory)?;
                    invalid_unit_offsets.push(units.len());
                    // DEL occupies one unit but is semantically inert while
                    // its offset is present in `invalid_unit_offsets`.
                    units.push(u16::from(b'\x7f'));
                    byte_offset += 1;
                }
            }
        }

        Ok(Self {
            runtime,
            realm,
            units,
            invalid_unit_offsets,
            raw_source: Some(source),
            cursor: 0,
            retain_record: false,
            mode,
        })
    }

    fn parse_document(&mut self) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        self.skip_whitespace()?;
        let result = self.parse_value(0)?;
        self.skip_whitespace()?;
        if self.cursor != self.units.len() {
            // QuickJS lexes the next token before reporting trailing data, so
            // malformed trailing strings/numbers retain their lexical error.
            let trailing_start = self.cursor;
            self.validate_current_token_lexically()?;
            return self.syntax_at(trailing_start, "unexpected data at the end");
        }
        Ok(result)
    }

    fn parse_value(&mut self, depth: usize) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        if depth > MAX_JSON_PARSE_DEPTH {
            return self.syntax("stack overflow");
        }
        self.skip_whitespace()?;
        if self.current_unit_is_invalid() {
            return self.syntax("unexpected character");
        }
        let Some(unit) = self.peek() else {
            return self.syntax("Unexpected end of JSON input");
        };
        match unit {
            unit if unit == u16::from(b'{') => self.parse_object(depth),
            unit if unit == u16::from(b'[') => self.parse_array(depth),
            unit if unit == u16::from(b'"')
                || (self.mode.is_extended() && unit == u16::from(b'\'')) =>
            {
                let start = self.cursor;
                let string = self.parse_string(unit)?;
                let end = self.cursor;
                let value = Value::String(string);
                let record = self.primitive_record(value.clone(), start, end);
                Ok((value, record))
            }
            unit if unit == u16::from(b'-')
                || is_ascii_digit(unit)
                || (self.mode.is_extended()
                    && (unit == u16::from(b'+')
                        || (unit == u16::from(b'.')
                            && self.peek_at(1).is_some_and(is_ascii_digit)))) =>
            {
                self.parse_number_value()
            }
            unit if is_ascii_identifier_start(unit) => self.parse_identifier_value(),
            unit if unit >= 0x80 => self.syntax("unexpected character"),
            0 => self.syntax("unexpected token: ''"),
            _ => self.syntax(&format!("unexpected token: '{}'", display_unit(unit))),
        }
    }

    fn parse_object(&mut self, depth: usize) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        self.cursor += 1;
        let object = self.runtime.new_ordinary_object_in_realm(self.realm)?;
        let mut entries = Vec::new();
        self.skip_whitespace()?;
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
            self.skip_whitespace()?;
            if self.current_unit_is_invalid() {
                return self.syntax("unexpected character");
            }
            let name = match self.peek() {
                Some(unit)
                    if unit == u16::from(b'"')
                        || (self.mode.is_extended() && unit == u16::from(b'\'')) =>
                {
                    self.parse_string(unit)?
                }
                Some(unit) if self.mode.is_extended() && is_ascii_identifier_start(unit) => {
                    self.parse_identifier_name()
                }
                Some(unit) if unit >= 0x80 => return self.syntax("unexpected character"),
                _ => {
                    self.validate_current_token_lexically()?;
                    return self.syntax("expecting property name");
                }
            };
            let key = self
                .runtime
                .intern_property_key_js_string(&name)
                .map_err(RuntimeError::from)?;
            self.skip_whitespace()?;
            self.validate_current_token_lexically()?;
            if !self.consume_ascii(b':') {
                return self.syntax("expecting ':'");
            }
            let (property_value, child_record) = self.parse_value(depth + 1)?;
            self.define_json_property(&object, &key, property_value)?;
            if let Some(record) = child_record {
                entries.push(JsonObjectParseRecordEntry { key, record });
            }

            self.skip_whitespace()?;
            self.validate_current_token_lexically()?;
            if self.consume_ascii(b',') {
                self.skip_whitespace()?;
                if self.mode.is_extended() && self.consume_ascii(b'}') {
                    break;
                }
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
        self.skip_whitespace()?;
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
            self.runtime.append_fresh_array_value(&array, element)?;
            if let Some(record) = child_record {
                elements.push(record);
            }
            index = index.checked_add(1).ok_or_else(|| {
                JsonParseFailure::Runtime(RuntimeError::Engine(Error::new(
                    ErrorKind::Range,
                    "invalid array length",
                )))
            })?;

            self.skip_whitespace()?;
            self.validate_current_token_lexically()?;
            if self.consume_ascii(b',') {
                self.skip_whitespace()?;
                if self.mode.is_extended() && self.consume_ascii(b']') {
                    break;
                }
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

    fn parse_string(&mut self, separator: u16) -> JsonParseResult<JsString> {
        debug_assert_eq!(self.peek(), Some(separator));
        let token_start = self.cursor;
        self.cursor += 1;
        let mut output = Vec::new();
        loop {
            let Some(unit) = self.peek() else {
                return self.syntax_at(token_start, "Unexpected end of JSON input");
            };
            let unit_offset = self.cursor;
            self.cursor += 1;
            if self.invalid_unit_at(unit_offset) {
                return self.syntax_at(unit_offset, "Bad UTF-8 sequence");
            }
            match unit {
                unit if unit == separator => break,
                unit if unit < 0x20 => {
                    return self.syntax_at(unit_offset, "Bad control character in string literal");
                }
                unit if unit == u16::from(b'\\') => {
                    let Some(escaped) = self.peek() else {
                        return self.syntax_at(token_start, "Unexpected end of JSON input");
                    };
                    let escaped_offset = self.cursor;
                    self.cursor += 1;
                    match escaped {
                        unit if unit == separator
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
                        unit if unit == u16::from(b'v') && self.mode.is_extended() => {
                            output.push(0x0b)
                        }
                        unit if unit == u16::from(b'\n') && self.mode.is_extended() => continue,
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
                        _ => return self.syntax_at(escaped_offset, "Bad escaped character"),
                    }
                }
                _ => output.push(unit),
            }
        }
        Ok(JsString::from_owned_utf16(output))
    }

    fn parse_number_value(&mut self) -> JsonParseResult<(Value, Option<JsonParseRecord>)> {
        let start = self.cursor;
        let negative = if self.consume_ascii(b'-') {
            true
        } else {
            if self.mode.is_extended() {
                self.consume_ascii(b'+');
            }
            false
        };
        if self.cursor != start && self.peek().is_none() {
            return self.syntax("Unexpected token '");
        }

        if self.mode.is_extended() {
            if ascii_starts_with(&self.units[self.cursor..], b"Infinity") {
                self.cursor += b"Infinity".len();
                let value = Value::number(if negative {
                    f64::NEG_INFINITY
                } else {
                    f64::INFINITY
                });
                let record = self.primitive_record(value.clone(), start, self.cursor);
                return Ok((value, record));
            }
            if ascii_starts_with(&self.units[self.cursor..], b"NaN") {
                self.cursor += b"NaN".len();
                let value = Value::number(f64::NAN);
                let record = self.primitive_record(value.clone(), start, self.cursor);
                return Ok((value, record));
            }

            if self.peek() == Some(u16::from(b'0')) {
                let radix = match self.peek_at(1) {
                    Some(unit) if unit == u16::from(b'x') || unit == u16::from(b'X') => Some(16),
                    Some(unit) if unit == u16::from(b'o') || unit == u16::from(b'O') => Some(8),
                    Some(unit) if unit == u16::from(b'b') || unit == u16::from(b'B') => Some(2),
                    _ => None,
                };
                if let Some(radix) = radix {
                    self.cursor += 2;
                    let digits_start = self.cursor;
                    if self
                        .peek()
                        .and_then(hex_value)
                        .is_none_or(|digit| u32::from(digit) >= radix)
                    {
                        let Some(unit) = self.peek() else {
                            return self.syntax("Unexpected token '");
                        };
                        let message = self.unexpected_percent_c_message_at(self.cursor, unit)?;
                        return self.syntax(&message);
                    }
                    while self
                        .peek()
                        .and_then(hex_value)
                        .is_some_and(|digit| u32::from(digit) < radix)
                    {
                        self.cursor += 1;
                    }
                    let digits =
                        JsString::from_owned_utf16(self.units[digits_start..self.cursor].to_vec());
                    let mut number = crate::number_parse::parse_int(
                        &digits,
                        i32::try_from(radix).expect("JSON radix fits i32"),
                    );
                    if negative {
                        number = -number;
                    }
                    let value = Value::number(number);
                    let record = self.primitive_record(value.clone(), start, self.cursor);
                    return Ok((value, record));
                }
            }
        }

        let digits_start = self.cursor;
        let Some(first) = self.peek() else {
            return self.syntax("Unexpected end of JSON input");
        };
        if self.mode.is_extended() && first == u16::from(b'.') {
            // The fractional scanner below consumes the leading decimal point.
        } else if first == u16::from(b'0') {
            self.cursor += 1;
            if self.peek().is_some_and(is_ascii_digit) {
                return self.syntax_at(digits_start, "Unexpected number");
            }
        } else if (u16::from(b'1')..=u16::from(b'9')).contains(&first) {
            self.cursor += 1;
            while self.peek().is_some_and(is_ascii_digit) {
                self.cursor += 1;
            }
        } else {
            let message = self.unexpected_percent_c_message_at(self.cursor, first)?;
            return self.syntax(&message);
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

    fn parse_identifier_name(&mut self) -> JsString {
        let start = self.cursor;
        debug_assert!(self.peek().is_some_and(is_ascii_identifier_start));
        self.cursor += 1;
        while self.peek().is_some_and(is_ascii_identifier_continue) {
            self.cursor += 1;
        }
        JsString::from_owned_utf16(self.units[start..self.cursor].to_vec())
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
        } else if self.mode.is_extended() && ascii_eq(spelling, b"NaN") {
            Value::number(f64::NAN)
        } else if self.mode.is_extended() && ascii_eq(spelling, b"Infinity") {
            Value::number(f64::INFINITY)
        } else {
            let token = spelling
                .iter()
                .map(|unit| char::from_u32(u32::from(*unit)).unwrap_or('\u{fffd}'))
                .collect::<String>();
            return self.syntax_at(start, &format!("unexpected token: '{token}'"));
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

    /// QuickJS keeps one token of lookahead. Container punctuation errors are
    /// therefore reported only after the intervening token has been lexed;
    /// malformed strings and numbers retain their more specific diagnostics.
    fn validate_current_token_lexically(&mut self) -> JsonParseResult<()> {
        let saved_cursor = self.cursor;
        let Some(unit) = self.peek() else {
            return Ok(());
        };
        let result = if self.current_unit_is_invalid() || unit >= 0x80 {
            self.syntax("unexpected character")
        } else if unit == u16::from(b'"') || (self.mode.is_extended() && unit == u16::from(b'\'')) {
            self.parse_string(unit).map(|_| ())
        } else if unit == u16::from(b'-')
            || is_ascii_digit(unit)
            || (self.mode.is_extended()
                && (unit == u16::from(b'+')
                    || (unit == u16::from(b'.') && self.peek_at(1).is_some_and(is_ascii_digit))))
        {
            self.parse_number_value().map(|_| ())
        } else {
            Ok(())
        };
        if result.is_ok() {
            self.cursor = saved_cursor;
        }
        result
    }

    fn skip_whitespace(&mut self) -> JsonParseResult<()> {
        loop {
            while self.peek().is_some_and(|unit| {
                matches!(unit, 0x09 | 0x0a | 0x0d | 0x20)
                    || (self.mode.is_extended() && matches!(unit, 0x0b | 0x0c))
            }) {
                self.cursor += 1;
            }
            if !self.mode.is_extended() || self.peek() != Some(u16::from(b'/')) {
                return Ok(());
            }
            match self.peek_at(1) {
                Some(unit) if unit == u16::from(b'/') => {
                    self.cursor += 2;
                    while self
                        .peek()
                        .is_some_and(|unit| !matches!(unit, 0x0a | 0x0d | 0x2028 | 0x2029))
                    {
                        self.cursor += 1;
                    }
                    if self
                        .peek()
                        .is_some_and(|unit| matches!(unit, 0x2028 | 0x2029))
                    {
                        self.cursor += 1;
                    }
                }
                Some(unit) if unit == u16::from(b'*') => {
                    let comment_start = self.cursor;
                    self.cursor += 2;
                    loop {
                        let Some(unit) = self.peek() else {
                            return self.syntax_at(comment_start, "unexpected end of comment");
                        };
                        if unit == u16::from(b'*') && self.peek_at(1) == Some(u16::from(b'/')) {
                            self.cursor += 2;
                            break;
                        }
                        self.cursor += 1;
                    }
                }
                _ => return Ok(()),
            }
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

    fn peek_at(&self, offset: usize) -> Option<u16> {
        self.units.get(self.cursor.checked_add(offset)?).copied()
    }

    fn current_unit_is_invalid(&self) -> bool {
        self.invalid_unit_at(self.cursor)
    }

    fn invalid_unit_at(&self, offset: usize) -> bool {
        self.invalid_unit_offsets.binary_search(&offset).is_ok()
    }

    fn display_percent_c_unit_at(&self, offset: usize, unit: u16) -> char {
        if self.invalid_unit_at(offset) {
            '\u{fffd}'
        } else {
            display_percent_c_unit(unit)
        }
    }

    fn unexpected_percent_c_message_at(&self, offset: usize, unit: u16) -> JsonParseResult<String> {
        if unit == 0 {
            // `vsnprintf("...%c...", 0)` terminates the C string before the
            // format's closing quote, including for an embedded source NUL.
            return Ok("Unexpected token '".to_owned());
        }
        let omit_closing_quote = if self.invalid_unit_at(offset) {
            let raw_source =
                self.raw_source
                    .ok_or(JsonParseFailure::Runtime(RuntimeError::Invariant(
                        "JSON invalid-unit marker has no raw source",
                    )))?;
            let byte_offset = json_raw_byte_offset(raw_source, offset)?;
            let byte = *raw_source
                .get(byte_offset)
                .ok_or(JsonParseFailure::Runtime(RuntimeError::Invariant(
                    "JSON invalid-unit byte offset is invalid",
                )))?;
            // QuickJS materializes the `vsnprintf` bytes through its malformed
            // UTF-8 decoder. A lone continuation consumes the following ASCII
            // quote as part of the replacement span; invalid lead bytes do not.
            (0x80..=0xbf).contains(&byte)
        } else {
            false
        };
        let token = self.display_percent_c_unit_at(offset, unit);
        Ok(if omit_closing_quote {
            format!("Unexpected token '{token}")
        } else {
            format!("Unexpected token '{token}'")
        })
    }

    fn source_location(&self, offset: usize) -> Result<LineColumn, RuntimeError> {
        let Some(raw_source) = self.raw_source else {
            return json_source_location(&self.units, offset);
        };
        let byte_offset = json_raw_byte_offset(raw_source, offset)?;
        QuickJsSourceLocator::from_bytes(raw_source)
            .locate_byte_offset(byte_offset)
            .map_err(|_| RuntimeError::Invariant("JSON diagnostic byte offset is invalid"))
    }

    fn syntax<T>(&self, message: &str) -> JsonParseResult<T> {
        self.syntax_at(self.cursor, message)
    }

    fn syntax_at<T>(&self, offset: usize, message: &str) -> JsonParseResult<T> {
        debug_assert!(offset <= self.units.len());
        Err(JsonParseFailure::Syntax(JsonSyntaxFailure {
            message: message.to_owned(),
            offset,
        }))
    }
}

fn json_source_location(units: &[u16], offset: usize) -> Result<LineColumn, RuntimeError> {
    if offset > units.len() {
        return Err(RuntimeError::Invariant(
            "JSON diagnostic offset is outside its source",
        ));
    }

    let mut line = 0_u32;
    let mut column = 0_u32;
    let mut cursor = 0;
    while cursor < offset {
        let unit = units[cursor];
        if unit == u16::from(b'\n') {
            line = line
                .checked_add(1)
                .ok_or(RuntimeError::Invariant("JSON diagnostic line overflowed"))?;
            column = 0;
            cursor += 1;
            continue;
        }

        column = column
            .checked_add(1)
            .ok_or(RuntimeError::Invariant("JSON diagnostic column overflowed"))?;
        cursor += if (0xd800..=0xdbff).contains(&unit)
            && cursor + 1 < offset
            && (0xdc00..=0xdfff).contains(&units[cursor + 1])
        {
            2
        } else {
            1
        };
    }
    Ok(LineColumn::new(line, column))
}

/// Translate the parser's decoded UTF-16 cursor back to the exact byte
/// boundary used by QuickJS's `JSParseState`. Valid non-BMP scalars contribute
/// two parser units but one raw source column; CESU-8 surrogate pairs remain
/// two separately encoded units and therefore two columns.
fn json_raw_byte_offset(source: &[u8], unit_offset: usize) -> Result<usize, RuntimeError> {
    let mut byte_cursor = 0;
    let mut unit_cursor = 0;
    while byte_cursor < source.len() {
        if unit_cursor == unit_offset {
            return Ok(byte_cursor);
        }

        let byte = source[byte_cursor];
        let (consumed, produced_units) = if byte < 0x80 {
            (1, 1)
        } else {
            match crate::value::decode_quickjs_utf8(&source[byte_cursor..]) {
                Some((code_point, consumed)) if code_point <= 0xffff => (consumed, 1),
                Some((code_point, consumed)) if code_point <= 0x10_ffff => (consumed, 2),
                Some(_) | None => (1, 1),
            }
        };
        if unit_offset < unit_cursor + produced_units {
            // No JSON grammar error can point between the two UTF-16 units of
            // one valid non-BMP scalar. Retain a deterministic source start if
            // a future caller nevertheless requests that interior position.
            return Ok(byte_cursor);
        }
        unit_cursor += produced_units;
        byte_cursor += consumed;
    }

    if unit_cursor == unit_offset {
        Ok(source.len())
    } else {
        Err(RuntimeError::Invariant(
            "JSON diagnostic offset is outside its raw source",
        ))
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

fn ascii_starts_with(units: &[u16], bytes: &[u8]) -> bool {
    units.len() >= bytes.len()
        && units
            .iter()
            .zip(bytes)
            .take(bytes.len())
            .all(|(unit, byte)| *unit == u16::from(*byte))
}

fn display_unit(unit: u16) -> char {
    char::from_u32(u32::from(unit)).unwrap_or('\u{fffd}')
}

/// QuickJS's numeric diagnostics format one raw UTF-8 byte with `%c`.
/// A non-ASCII leading byte is invalid UTF-8 on its own and becomes U+FFFD
/// when the resulting error string is materialized.
fn display_percent_c_unit(unit: u16) -> char {
    if unit >= 0x80 {
        '\u{fffd}'
    } else {
        display_unit(unit)
    }
}
