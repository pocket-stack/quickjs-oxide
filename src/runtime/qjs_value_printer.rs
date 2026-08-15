//! Side-effect-free value rendering used by the optional qjs host.
//!
//! This is the Rust counterpart of pinned QuickJS's `JS_PrintValue`.  It is
//! intentionally separate from ECMAScript `ToString`: diagnostic rendering
//! must not invoke accessors, proxy traps, coercion hooks, or lazy intrinsics.

use super::*;
use crate::heap::{CollectionIteratorCurrentIndices, RegExpObjectData};
use std::cell::OnceCell;
use std::collections::BTreeSet;

const MAX_DEPTH: usize = 2;
const MAX_STRING_LENGTH: usize = 1_000;
const MAX_ITEM_COUNT: usize = 100;

#[derive(Clone)]
enum PrintablePropertyValue {
    Data(RawValue),
    Accessor { get: bool, set: bool },
    AutoInit,
}

#[derive(Clone)]
struct PrintableProperty {
    atom: Atom,
    value: PrintablePropertyValue,
}

#[derive(Clone)]
struct CollectionEntry {
    key: Option<RawValue>,
    value: RawValue,
}

#[derive(Clone, Copy)]
enum CollectionKind {
    Map,
    Set,
}

enum PrintableBody {
    Ordinary,
    Array {
        dense: Option<Vec<RawValue>>,
        dense_count: usize,
        length: u32,
    },
    TypedArray,
    Function,
    Map {
        records: Vec<CollectionEntry>,
        size: usize,
    },
    Set {
        records: Vec<CollectionEntry>,
        size: usize,
    },
    RegExp(RegExpObjectData),
    Date(f64),
    Error,
}

struct ObjectPrintSnapshot {
    class_name: &'static str,
    body: PrintableBody,
    properties: Vec<PrintableProperty>,
    property_count: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CommaState {
    Empty,
    Comma,
    Suffix,
}

struct QjsValuePrinter<'runtime, 'output> {
    runtime: &'runtime Runtime,
    output: &'output mut Vec<u8>,
    stack: Vec<ObjectId>,
    collection_iterator_current_indices: OnceCell<CollectionIteratorCurrentIndices>,
    length_atom: Atom,
    name_atom: Atom,
    message_atom: Atom,
    stack_atom: Atom,
}

impl Runtime {
    /// Render one value with pinned `JS_PrintValue` semantics. The qjs stdout
    /// helpers keep their special raw top-level String branch outside this
    /// formatter, while uncaught String exceptions intentionally enter it and
    /// are quoted just like upstream.
    pub fn qjs_print_value_bytes(&self, value: &Value) -> Result<Vec<u8>, RuntimeError> {
        let mut output = Vec::new();
        self.qjs_print_value_into_bytes(value, &mut output)?;
        Ok(output)
    }

    /// Append one pinned `JS_PrintValue` rendering to an existing byte buffer.
    /// The qjs host uses this to assemble a line without allocating and then
    /// copying a temporary vector for every non-String argument.
    pub(in crate::runtime) fn qjs_print_value_into_bytes(
        &self,
        value: &Value,
        output: &mut Vec<u8>,
    ) -> Result<(), RuntimeError> {
        let length = self.intern_property_key("length")?;
        let name = self.intern_property_key("name")?;
        let message = self.intern_property_key("message")?;
        let stack = self.intern_property_key("stack")?;
        let mut printer = QjsValuePrinter {
            runtime: self,
            output,
            stack: Vec::with_capacity(MAX_DEPTH),
            collection_iterator_current_indices: OnceCell::new(),
            length_atom: length.atom(),
            name_atom: name.atom(),
            message_atom: message.atom(),
            stack_atom: stack.atom(),
        };
        let raw = self.raw_property_value(value)?;
        printer.print_raw_value(&raw)
    }
}

impl QjsValuePrinter<'_, '_> {
    fn print_raw_value(&mut self, value: &RawValue) -> Result<(), RuntimeError> {
        match value {
            RawValue::Undefined => self.push_ascii("undefined"),
            RawValue::Null => self.push_ascii("null"),
            RawValue::Bool(false) => self.push_ascii("false"),
            RawValue::Bool(true) => self.push_ascii("true"),
            RawValue::Int(value) => self.push_ascii(&value.to_string()),
            RawValue::Float(value) => self.print_float(*value),
            RawValue::BigInt(value) => {
                self.push_ascii(&value.to_string());
                self.output.push(b'n');
            }
            RawValue::String(value) => self.print_string(value),
            RawValue::Symbol(atom) => {
                self.push_ascii("Symbol(");
                self.print_atom(*atom)?;
                self.output.push(b')');
            }
            RawValue::Object(object) => self.print_object(*object)?,
            RawValue::Uninitialized => self.push_ascii("uninitialized"),
            RawValue::Exception => self.push_ascii("exception"),
            RawValue::Private(_) => {
                return Err(RuntimeError::Invariant(
                    "private-name identity reached qjs value printing",
                ));
            }
        }
        Ok(())
    }

    fn print_float(&mut self, value: f64) {
        if value == 0.0 && value.is_sign_negative() {
            self.push_ascii("-0");
        } else {
            self.push_ascii(&crate::value::number_to_string(value));
        }
    }

    fn print_string(&mut self, value: &JsString) {
        self.output.push(b'"');
        self.print_value_string_units(value, MAX_STRING_LENGTH, b'"');
        self.output.push(b'"');
        if value.len() > MAX_STRING_LENGTH {
            let remaining = value.len() - MAX_STRING_LENGTH;
            self.push_ascii("... ");
            self.push_ascii(&remaining.to_string());
            self.push_ascii(" more character");
            if remaining > 1 {
                self.output.push(b's');
            }
        }
    }

    fn print_value_string_units(&mut self, value: &JsString, limit: usize, separator: u8) {
        let mut position = 0;
        value.for_each_flat_leaf(|leaf| {
            if position < limit {
                self.print_logical_string_units(leaf, leaf.len().min(limit - position), separator);
            }
            position += leaf.len();
        });
        debug_assert_eq!(position, value.len());
    }

    fn print_logical_string_units(&mut self, value: &JsString, limit: usize, separator: u8) {
        let units = value.utf16_units().take(limit).collect::<Vec<_>>();
        let mut index = 0;
        while index < units.len() {
            let unit = units[index];
            index += 1;
            match unit {
                0x09 => self.push_ascii("\\t"),
                0x0d => self.push_ascii("\\r"),
                0x0a => self.push_ascii("\\n"),
                0x08 => self.push_ascii("\\b"),
                0x0c => self.push_ascii("\\f"),
                0x5c => self.push_ascii("\\\\"),
                unit if unit == u16::from(separator) => {
                    self.output.push(b'\\');
                    self.output.push(separator);
                }
                0x20..=0x7e => self.output.push(unit as u8),
                0x00..=0x1f | 0x7f..=0x9f => self.push_unicode_escape(unit),
                0xd800..=0xdbff => {
                    let Some(low) = units.get(index).copied() else {
                        self.push_unicode_escape(unit);
                        continue;
                    };
                    if !(0xdc00..=0xdfff).contains(&low) {
                        self.push_unicode_escape(unit);
                        continue;
                    }
                    index += 1;
                    let code_point =
                        0x1_0000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00);
                    self.push_code_point(code_point);
                }
                0xdc00..=0xdfff => self.push_unicode_escape(unit),
                _ => self.push_code_point(u32::from(unit)),
            }
        }
    }

    fn push_unicode_escape(&mut self, unit: u16) {
        const HEX: &[u8; 16] = b"0123456789abcdef";
        self.push_ascii("\\u");
        for shift in [12, 8, 4, 0] {
            self.output.push(HEX[usize::from((unit >> shift) & 0x0f)]);
        }
    }

    fn push_code_point(&mut self, code_point: u32) {
        let character = char::from_u32(code_point)
            .expect("qjs string printer combined only valid Unicode scalars");
        let mut buffer = [0_u8; 4];
        self.output
            .extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
    }

    fn print_atom(&mut self, atom: Atom) -> Result<(), RuntimeError> {
        enum OwnedSpelling {
            Integer(u32),
            Text(JsString),
        }

        let spelling = {
            let state = self.runtime.0.state.borrow();
            match state.atoms.resolve(atom)?.spelling {
                AtomSpelling::Integer(value) => OwnedSpelling::Integer(value),
                AtomSpelling::Text(value) => OwnedSpelling::Text(value.clone()),
                AtomSpelling::NoDescription => OwnedSpelling::Text(JsString::from_static("")),
            }
        };
        match spelling {
            OwnedSpelling::Integer(value) => self.push_ascii(&value.to_string()),
            OwnedSpelling::Text(value) => self.print_atom_string(&value),
        }
        Ok(())
    }

    fn print_atom_string(&mut self, value: &JsString) {
        let is_identifier = !value.is_empty()
            && value.utf16_units().enumerate().all(|(index, unit)| {
                (u16::from(b'a')..=u16::from(b'z')).contains(&unit)
                    || (u16::from(b'A')..=u16::from(b'Z')).contains(&unit)
                    || unit == u16::from(b'_')
                    || unit == u16::from(b'$')
                    || (index > 0 && (u16::from(b'0')..=u16::from(b'9')).contains(&unit))
            });
        if is_identifier {
            self.output
                .extend(value.utf16_units().map(|unit| unit as u8));
        } else {
            self.output.push(b'"');
            // QuickJS atoms are flat even when their source description was a
            // rope, so atom spelling intentionally decodes as one logical
            // String instead of preserving value-rope leaf boundaries.
            self.print_logical_string_units(value, value.len(), b'"');
            self.output.push(b'"');
        }
    }

    fn print_class_name(&mut self, class_name: &'static str) {
        let value = JsString::from_static(class_name);
        self.print_atom_string(&value);
    }

    fn print_object(&mut self, object: ObjectId) -> Result<(), RuntimeError> {
        if let Some(index) = self.stack.iter().position(|candidate| *candidate == object) {
            self.push_ascii("[circular ");
            self.push_ascii(&index.to_string());
            self.output.push(b']');
            return Ok(());
        }

        if self.stack.len() >= MAX_DEPTH {
            let class_name = self.depth_class_name(object)?;
            self.output.push(b'[');
            self.print_class_name(class_name);
            self.output.push(b']');
            return Ok(());
        }

        let snapshot = self.snapshot_object(object)?;
        self.stack.push(object);
        let result = self.print_object_snapshot(object, &snapshot);
        let popped = self.stack.pop();
        debug_assert_eq!(popped, Some(object));
        result
    }

    /// Read only the class identity needed by QuickJS's depth fallback. This
    /// must stay independent from enumerable-property and collection-record
    /// snapshots: `js_print_value` returns at the depth check before either
    /// potentially attacker-sized traversal begins.
    fn depth_class_name(&self, object: ObjectId) -> Result<&'static str, RuntimeError> {
        let state = self.runtime.0.state.borrow();
        let object = state.heap.object(object)?;
        let class_name = match &object.payload {
            ObjectPayload::Ordinary => match object.kind {
                ObjectKind::Iterator => "Iterator",
                _ => "Object",
            },
            ObjectPayload::RawJson => "Object",
            ObjectPayload::Array { .. } => "Array",
            ObjectPayload::Arguments { .. } => "Arguments",
            ObjectPayload::ArrayIterator { .. } => "Array Iterator",
            ObjectPayload::ForInIterator(_) => "ForInIterator",
            ObjectPayload::Primitive(value) => match value {
                PrimitiveObjectData::Number(_) => "Number",
                PrimitiveObjectData::String(_) => "String",
                PrimitiveObjectData::Boolean(_) => "Boolean",
                PrimitiveObjectData::Symbol(_) => "Symbol",
                PrimitiveObjectData::BigInt(_) => "BigInt",
            },
            ObjectPayload::Date(_) => "Date",
            ObjectPayload::RegExp(_) => "RegExp",
            ObjectPayload::RegExpStringIterator { .. } => "RegExp String Iterator",
            ObjectPayload::Map { .. } => "Map",
            ObjectPayload::MapIterator { .. } => "Map Iterator",
            ObjectPayload::Set { .. } => "Set",
            ObjectPayload::SetIterator { .. } => "Set Iterator",
            ObjectPayload::WeakMap { .. } => "WeakMap",
            ObjectPayload::WeakSet { .. } => "WeakSet",
            ObjectPayload::WeakRef { .. } => "WeakRef",
            ObjectPayload::FinalizationRegistry(_) => "FinalizationRegistry",
            ObjectPayload::GlobalObject { .. } => "Object",
            ObjectPayload::Error => "Error",
            ObjectPayload::StringIterator { .. } => "String Iterator",
            ObjectPayload::IteratorHelper(_) => "Iterator Helper",
            ObjectPayload::IteratorWrap(_) => "Iterator Wrap",
            ObjectPayload::AsyncFromSyncIterator(_) => "",
            ObjectPayload::IteratorConcat(_) => "Iterator Concat",
            ObjectPayload::Proxy(_) => "Object",
            ObjectPayload::ArrayBuffer(_) => "ArrayBuffer",
            ObjectPayload::SharedArrayBuffer(_) => "SharedArrayBuffer",
            ObjectPayload::DataView(_) => "DataView",
            ObjectPayload::TypedArray(data) => data.element.name(),
            ObjectPayload::NativeFunction {
                internal: Some(InternalCallableData::PromiseResolving { kind, .. }),
                ..
            } => match kind {
                PromiseResolvingKind::Resolve => "PromiseResolveFunction",
                PromiseResolvingKind::Reject => "PromiseRejectFunction",
            },
            ObjectPayload::NativeFunction { .. } | ObjectPayload::BoundFunction { .. } => {
                "Function"
            }
            ObjectPayload::BytecodeFunction { bytecode, .. } => match state
                .heap
                .function_bytecode(*bytecode)?
                .metadata
                .function_kind
            {
                FunctionKind::Normal => "Function",
                FunctionKind::Generator => "GeneratorFunction",
                FunctionKind::Async => "AsyncFunction",
                FunctionKind::AsyncGenerator => "AsyncGeneratorFunction",
            },
            ObjectPayload::Generator { .. } => "Generator",
            ObjectPayload::AsyncGenerator(_) => "AsyncGenerator",
            ObjectPayload::AsyncFunctionState(_) => "AsyncFunction",
            ObjectPayload::Promise(_) => "Promise",
        };
        Ok(class_name)
    }

    fn print_object_snapshot(
        &mut self,
        object: ObjectId,
        snapshot: &ObjectPrintSnapshot,
    ) -> Result<(), RuntimeError> {
        let mut comma_state = CommaState::Empty;
        let mut is_array = false;

        match &snapshot.body {
            PrintableBody::Array {
                dense,
                dense_count,
                length,
            } => {
                is_array = true;
                self.push_ascii("[ ");
                if let Some(values) = dense {
                    for value in values {
                        self.print_comma(&mut comma_state);
                        self.print_raw_value(value)?;
                    }
                    if values.len() < *dense_count {
                        self.print_more_items(&mut comma_state, dense_count - values.len());
                    }
                    if *dense_count < *length as usize {
                        let empty = *length as usize - *dense_count;
                        self.print_comma(&mut comma_state);
                        self.output.push(b'<');
                        self.push_ascii(&empty.to_string());
                        self.push_ascii(" empty item");
                        if empty > 1 {
                            self.output.push(b's');
                        }
                        self.output.push(b'>');
                    }
                }
            }
            PrintableBody::TypedArray => {
                is_array = true;
                let typed = self
                    .runtime
                    .qjs_typed_array_print_snapshot(object, MAX_ITEM_COUNT as u32)?;
                self.print_class_name(typed.element.name());
                self.output.push(b'(');
                self.push_ascii(&typed.length.to_string());
                self.push_ascii(") [ ");
                for value in &typed.values {
                    self.print_comma(&mut comma_state);
                    match value {
                        Value::BigInt(value) => self.push_ascii(&value.to_string()),
                        _ => {
                            let raw = self.runtime.raw_property_value(value)?;
                            self.print_raw_value(&raw)?;
                        }
                    }
                }
                if typed.values.len() < typed.length as usize {
                    self.print_more_items(
                        &mut comma_state,
                        typed.length as usize - typed.values.len(),
                    );
                }
            }
            PrintableBody::Function => {
                self.push_ascii("[Function ");
                let name = self.raw_string_property(object, self.name_atom)?;
                let name = name.as_ref().map(Self::c_string_bytes).transpose()?;
                match name {
                    Some(name) if !name.is_empty() => self.output.extend_from_slice(&name),
                    Some(_) | None => self.push_ascii("(anonymous)"),
                }
                self.output.push(b']');
                comma_state = CommaState::Suffix;
            }
            PrintableBody::Map { records, size } => {
                self.push_ascii("Map(");
                self.push_ascii(&size.to_string());
                self.push_ascii(") { ");
                let mut printed = 0;
                for record in records {
                    self.print_comma(&mut comma_state);
                    let Some(key) = &record.key else {
                        continue;
                    };
                    self.print_raw_value(key)?;
                    self.push_ascii(" => ");
                    self.print_raw_value(&record.value)?;
                    printed += 1;
                }
                if printed < *size {
                    self.print_more_items(&mut comma_state, size - printed);
                }
            }
            PrintableBody::Set { records, size } => {
                self.push_ascii("Set(");
                self.push_ascii(&size.to_string());
                self.push_ascii(") { ");
                let mut printed = 0;
                for record in records {
                    self.print_comma(&mut comma_state);
                    let Some(key) = &record.key else {
                        continue;
                    };
                    self.print_raw_value(key)?;
                    printed += 1;
                }
                if printed < *size {
                    self.print_more_items(&mut comma_state, size - printed);
                }
            }
            PrintableBody::RegExp(regexp) => {
                self.print_regexp(regexp);
                comma_state = CommaState::Suffix;
            }
            PrintableBody::Date(value) => {
                if let Some(value) = intrinsics::date::qjs_print_iso_string(*value) {
                    self.push_ascii(&value);
                    comma_state = CommaState::Suffix;
                } else {
                    self.print_class_name(snapshot.class_name);
                    self.push_ascii(" { ");
                }
            }
            PrintableBody::Error => {
                self.print_error(object)?;
                comma_state = CommaState::Suffix;
            }
            PrintableBody::Ordinary => {
                if snapshot.class_name != "Object" || self.object_needs_object_prefix(object)? {
                    self.print_class_name(snapshot.class_name);
                    self.output.push(b' ');
                }
                self.push_ascii("{ ");
            }
        }

        for property in &snapshot.properties {
            self.print_comma(&mut comma_state);
            self.print_atom(property.atom)?;
            self.push_ascii(": ");
            match &property.value {
                PrintablePropertyValue::Data(value) => self.print_raw_value(value)?,
                PrintablePropertyValue::Accessor { get, set } => match (*get, *set) {
                    (true, true) => self.push_ascii("[Getter/Setter]"),
                    (false, true) => self.push_ascii("[Setter]"),
                    (true, false) | (false, false) => self.push_ascii("[Getter]"),
                },
                PrintablePropertyValue::AutoInit => self.push_ascii("[autoinit]"),
            }
        }
        if snapshot.properties.len() < snapshot.property_count {
            self.print_more_items(
                &mut comma_state,
                snapshot.property_count - snapshot.properties.len(),
            );
        }

        if is_array {
            self.push_ascii(" ]");
        } else if comma_state != CommaState::Suffix {
            self.push_ascii(" }");
        }
        Ok(())
    }

    fn object_needs_object_prefix(&self, object: ObjectId) -> Result<bool, RuntimeError> {
        let state = self.runtime.0.state.borrow();
        let object = state.heap.object(object)?;
        Ok(object.kind != ObjectKind::Ordinary
            || !matches!(object.payload, ObjectPayload::Ordinary))
    }

    fn print_regexp(&mut self, regexp: &RegExpObjectData) {
        let RegExpObjectData::Compiled { pattern, program } = regexp else {
            self.push_ascii("[uninitialized_regexp]");
            return;
        };
        self.output.push(b'/');
        if pattern.is_empty() {
            self.push_ascii("(?:)");
        } else {
            let units = pattern.utf16_units().collect::<Vec<_>>();
            let mut in_class = false;
            let mut index = 0;
            while index < units.len() {
                let mut first = units[index];
                index += 1;
                let mut second = None;
                match first {
                    0x5c => {
                        if let Some(next) = units.get(index).copied() {
                            second = Some(next);
                            index += 1;
                        }
                    }
                    0x5d => in_class = false,
                    0x5b if !in_class => {
                        if units.get(index).copied() == Some(0x5d) {
                            second = Some(0x5d);
                            index += 1;
                        }
                        in_class = true;
                    }
                    0x0a => {
                        first = 0x5c;
                        second = Some(u16::from(b'n'));
                    }
                    0x0d => {
                        first = 0x5c;
                        second = Some(u16::from(b'r'));
                    }
                    0x2f if !in_class => {
                        first = 0x5c;
                        second = Some(0x2f);
                    }
                    _ => {}
                }
                self.push_regexp_unit(first);
                if let Some(second) = second {
                    self.push_regexp_unit(second);
                }
            }
        }
        self.output.push(b'/');
        let bits = program.flags().bits();
        for (bit, flag) in b"gimsuydv".iter().copied().enumerate() {
            if bits & (1_u16 << bit) != 0 {
                self.output.push(flag);
            }
        }
    }

    fn push_regexp_unit(&mut self, unit: u16) {
        // Preserve the pinned printer's narrowing bug: `js_print_regexp`
        // passes every UTF-16 code unit to `js_putc(char)`, truncating the
        // high byte instead of using JS_ToCString's UTF-8/WTF-8 transport.
        self.output.push(unit as u8);
    }

    fn print_error(&mut self, object: ObjectId) -> Result<(), RuntimeError> {
        let name = self.raw_string_property(object, self.name_atom)?;
        match name {
            Some(name) => self.output.extend_from_slice(&Self::c_string_bytes(&name)?),
            None => self.push_ascii("Error"),
        }

        if let Some(message) = self.raw_string_property(object, self.message_atom)? {
            let message = Self::c_string_bytes(&message)?;
            if !message.is_empty() {
                self.push_ascii(": ");
                self.output.extend_from_slice(&message);
            }
        }

        if let Some(stack) = self.raw_string_property(object, self.stack_atom)? {
            let mut stack = Self::c_string_bytes(&stack)?;
            if stack.last() == Some(&b'\n') {
                stack.pop();
            }
            self.output.push(b'\n');
            self.output.extend_from_slice(&stack);
        }
        Ok(())
    }

    fn raw_string_property(
        &self,
        object: ObjectId,
        atom: Atom,
    ) -> Result<Option<JsString>, RuntimeError> {
        let state = self.runtime.0.state.borrow();
        raw_string_property_one_level(&state, object, atom)
    }

    fn c_string_bytes(value: &JsString) -> Result<Vec<u8>, RuntimeError> {
        let mut bytes = value.try_to_wtf8_bytes().map_err(|_| {
            RuntimeError::Engine(Error::new(
                ErrorKind::Internal,
                "qjs value printer could not allocate a C string",
            ))
        })?;
        if let Some(nul) = bytes.iter().position(|byte| *byte == 0) {
            bytes.truncate(nul);
        }
        Ok(bytes)
    }

    fn snapshot_object(&self, object: ObjectId) -> Result<ObjectPrintSnapshot, RuntimeError> {
        let state = self.runtime.0.state.borrow();
        let object_data = state.heap.object(object)?;
        let shape = state.heap.shape(object_data.shape)?;
        if shape.entries().len() != object_data.slots.len() {
            return Err(RuntimeError::Invariant(
                "qjs printer observed a shape/slot length mismatch",
            ));
        }

        let mut properties = Vec::with_capacity(MAX_ITEM_COUNT);
        let mut property_count = 0;
        let arguments_fast_len = match &object_data.payload {
            ObjectPayload::Arguments { fast_len, .. } => *fast_len,
            _ => None,
        };
        for (entry, slot) in shape.entries().iter().zip(&object_data.slots) {
            if !entry.flags.enumerable {
                continue;
            }
            if let Some(fast_len) = arguments_fast_len {
                if state
                    .atoms
                    .array_index(entry.atom)?
                    .is_some_and(|index| index < fast_len)
                {
                    // QuickJS keeps the fast Arguments prefix in shape slots,
                    // but JS_PrintValue intentionally visits only its ordinary
                    // slow properties. Once fast_len becomes None these same
                    // entries become printer-visible.
                    continue;
                }
            }
            property_count += 1;
            if properties.len() >= MAX_ITEM_COUNT {
                continue;
            }
            let value = match slot {
                PropertySlot::Data(value) => PrintablePropertyValue::Data(value.clone()),
                PropertySlot::VarRef(var_ref) => {
                    PrintablePropertyValue::Data(state.heap.var_ref(*var_ref)?.value.clone())
                }
                PropertySlot::Accessor { get, set } => PrintablePropertyValue::Accessor {
                    get: get.is_some(),
                    set: set.is_some(),
                },
                PropertySlot::AutoInit(_) => PrintablePropertyValue::AutoInit,
            };
            properties.push(PrintableProperty {
                atom: entry.atom,
                value,
            });
        }

        let (class_name, body) = match &object_data.payload {
            ObjectPayload::Ordinary => {
                let class_name = match object_data.kind {
                    ObjectKind::Iterator => "Iterator",
                    _ => "Object",
                };
                (class_name, PrintableBody::Ordinary)
            }
            ObjectPayload::RawJson => ("Object", PrintableBody::Ordinary),
            ObjectPayload::Array { dense } => {
                let length = shape
                    .find(self.length_atom)
                    .and_then(|index| object_data.slots.get(index as usize))
                    .and_then(|slot| match slot {
                        PropertySlot::Data(RawValue::Int(value)) => Some(*value as u32),
                        PropertySlot::Data(RawValue::Float(value)) => Some(*value as u32),
                        _ => None,
                    })
                    .unwrap_or(0);
                let dense_count = dense.as_ref().map_or(0, Vec::len);
                let dense = dense.as_ref().map(|values| {
                    values
                        .iter()
                        .take(MAX_ITEM_COUNT)
                        .cloned()
                        .collect::<Vec<_>>()
                });
                (
                    "Array",
                    PrintableBody::Array {
                        dense,
                        dense_count,
                        length,
                    },
                )
            }
            ObjectPayload::Arguments { .. } => ("Arguments", PrintableBody::Ordinary),
            ObjectPayload::ArrayIterator { .. } => ("Array Iterator", PrintableBody::Ordinary),
            ObjectPayload::ForInIterator(_) => ("ForInIterator", PrintableBody::Ordinary),
            ObjectPayload::Primitive(value) => {
                let class_name = match value {
                    PrimitiveObjectData::Number(_) => "Number",
                    PrimitiveObjectData::String(_) => "String",
                    PrimitiveObjectData::Boolean(_) => "Boolean",
                    PrimitiveObjectData::Symbol(_) => "Symbol",
                    PrimitiveObjectData::BigInt(_) => "BigInt",
                };
                (class_name, PrintableBody::Ordinary)
            }
            ObjectPayload::Date(value) => ("Date", PrintableBody::Date(*value)),
            ObjectPayload::RegExp(value) => ("RegExp", PrintableBody::RegExp(value.clone())),
            ObjectPayload::RegExpStringIterator { .. } => {
                ("RegExp String Iterator", PrintableBody::Ordinary)
            }
            ObjectPayload::Map {
                records: source,
                live_indices,
                size,
            } => {
                let records = self.snapshot_collection_records(
                    &state,
                    object,
                    source,
                    live_indices,
                    CollectionKind::Map,
                );
                (
                    "Map",
                    PrintableBody::Map {
                        records,
                        size: *size,
                    },
                )
            }
            ObjectPayload::MapIterator { .. } => ("Map Iterator", PrintableBody::Ordinary),
            ObjectPayload::Set {
                records: source,
                live_indices,
                size,
            } => {
                let records = self.snapshot_collection_records(
                    &state,
                    object,
                    source,
                    live_indices,
                    CollectionKind::Set,
                );
                (
                    "Set",
                    PrintableBody::Set {
                        records,
                        size: *size,
                    },
                )
            }
            ObjectPayload::SetIterator { .. } => ("Set Iterator", PrintableBody::Ordinary),
            ObjectPayload::WeakMap { .. } => ("WeakMap", PrintableBody::Ordinary),
            ObjectPayload::WeakSet { .. } => ("WeakSet", PrintableBody::Ordinary),
            ObjectPayload::WeakRef { .. } => ("WeakRef", PrintableBody::Ordinary),
            ObjectPayload::FinalizationRegistry(_) => {
                ("FinalizationRegistry", PrintableBody::Ordinary)
            }
            ObjectPayload::GlobalObject { .. } => ("Object", PrintableBody::Ordinary),
            ObjectPayload::Error => ("Error", PrintableBody::Error),
            ObjectPayload::StringIterator { .. } => ("String Iterator", PrintableBody::Ordinary),
            ObjectPayload::IteratorHelper(_) => ("Iterator Helper", PrintableBody::Ordinary),
            ObjectPayload::IteratorWrap(_) => ("Iterator Wrap", PrintableBody::Ordinary),
            ObjectPayload::AsyncFromSyncIterator(_) => ("", PrintableBody::Ordinary),
            ObjectPayload::IteratorConcat(_) => ("Iterator Concat", PrintableBody::Ordinary),
            ObjectPayload::Proxy(_) => ("Object", PrintableBody::Ordinary),
            ObjectPayload::ArrayBuffer(_) => ("ArrayBuffer", PrintableBody::Ordinary),
            ObjectPayload::SharedArrayBuffer(_) => ("SharedArrayBuffer", PrintableBody::Ordinary),
            ObjectPayload::DataView(_) => ("DataView", PrintableBody::Ordinary),
            ObjectPayload::TypedArray(data) => (data.element.name(), PrintableBody::TypedArray),
            ObjectPayload::NativeFunction {
                internal: Some(crate::heap::InternalCallableData::PromiseResolving { kind, .. }),
                ..
            } => {
                let class_name = match kind {
                    PromiseResolvingKind::Resolve => "PromiseResolveFunction",
                    PromiseResolvingKind::Reject => "PromiseRejectFunction",
                };
                (class_name, PrintableBody::Function)
            }
            ObjectPayload::NativeFunction { .. } | ObjectPayload::BoundFunction { .. } => {
                ("Function", PrintableBody::Function)
            }
            ObjectPayload::BytecodeFunction { bytecode, .. } => {
                let class_name = match state
                    .heap
                    .function_bytecode(*bytecode)?
                    .metadata
                    .function_kind
                {
                    FunctionKind::Normal => "Function",
                    FunctionKind::Generator => "GeneratorFunction",
                    FunctionKind::Async => "AsyncFunction",
                    FunctionKind::AsyncGenerator => "AsyncGeneratorFunction",
                };
                (class_name, PrintableBody::Function)
            }
            ObjectPayload::Generator { .. } => ("Generator", PrintableBody::Ordinary),
            ObjectPayload::AsyncGenerator(_) => ("AsyncGenerator", PrintableBody::Ordinary),
            ObjectPayload::AsyncFunctionState(_) => ("AsyncFunction", PrintableBody::Ordinary),
            ObjectPayload::Promise(_) => ("Promise", PrintableBody::Ordinary),
        };

        Ok(ObjectPrintSnapshot {
            class_name,
            body,
            properties,
            property_count,
        })
    }

    fn snapshot_collection_records(
        &self,
        state: &RuntimeState,
        object: ObjectId,
        source: &[crate::heap::MapRecord],
        live_indices: &BTreeSet<usize>,
        kind: CollectionKind,
    ) -> Vec<CollectionEntry> {
        // QuickJS unlinks ordinary deleted records. Oxide keeps stable slots
        // for iterator mutation semantics, so use the live index plus the
        // small set of genuinely retained zombies instead of walking all
        // historical tombstones.
        let mut visible_indices = live_indices
            .iter()
            .copied()
            .take(MAX_ITEM_COUNT)
            .collect::<BTreeSet<_>>();
        let iterator_indices = self
            .collection_iterator_current_indices
            .get_or_init(|| state.heap.collection_iterator_current_indices());
        visible_indices.extend(
            match kind {
                CollectionKind::Map => iterator_indices.map(object),
                CollectionKind::Set => iterator_indices.set(object),
            }
            .into_iter()
            .flatten()
            .copied(),
        );
        visible_indices.extend(
            state
                .active_collection_records
                .iter()
                .filter_map(|active| match (kind, active) {
                    (
                        CollectionKind::Map,
                        ActiveCollectionRecord::Map {
                            object: source,
                            index,
                        },
                    ) if *source == object => Some(*index),
                    (
                        CollectionKind::Set,
                        ActiveCollectionRecord::Set {
                            object: source,
                            index,
                        },
                    ) if *source == object => Some(*index),
                    _ => None,
                }),
        );

        let mut records = Vec::new();
        let mut live = 0;
        for index in visible_indices {
            let Some(record) = source.get(index) else {
                continue;
            };
            records.push(CollectionEntry {
                key: record.key.clone(),
                value: record.value.clone(),
            });
            if record.key.is_some() {
                live += 1;
                if live >= MAX_ITEM_COUNT {
                    break;
                }
            }
        }
        records
    }

    fn print_comma(&mut self, state: &mut CommaState) {
        match *state {
            CommaState::Empty => {}
            CommaState::Comma => self.push_ascii(", "),
            CommaState::Suffix => self.push_ascii(" { "),
        }
        *state = CommaState::Comma;
    }

    fn print_more_items(&mut self, state: &mut CommaState, count: usize) {
        self.print_comma(state);
        self.push_ascii("... ");
        self.push_ascii(&count.to_string());
        self.push_ascii(" more item");
        if count > 1 {
            self.output.push(b's');
        }
    }

    fn push_ascii(&mut self, value: &str) {
        debug_assert!(value.is_ascii());
        self.output.extend_from_slice(value.as_bytes());
    }
}
