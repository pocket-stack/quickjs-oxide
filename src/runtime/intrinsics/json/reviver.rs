//! QuickJS-shaped `JSON.parse` builtin and reviver internalization.

use super::super::super::*;
use super::parse::JsonParseRecord;

const MAX_JSON_REVIVER_DEPTH: usize = 128;

impl Runtime {
    pub(super) fn call_json_parse(
        &self,
        realm: ContextId,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let source = match self.native_to_js_string(realm, &arguments.readable[0])? {
            NativeConversion::Value(source) => source,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let reviver = match &arguments.readable[1] {
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

        // Pinned QuickJS allocates the reviver root holder before parsing.
        // Preserve that order even though the Rust parse record does not need
        // to embed the holder itself.
        let root = reviver
            .as_ref()
            .map(|_| self.new_ordinary_object_in_realm(realm))
            .transpose()?;
        let (parsed, record) = match self.parse_json_text(realm, &source, reviver.is_some())? {
            NativeConversion::Value(result) => result,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let Some(reviver) = reviver else {
            return Ok(Completion::Return(parsed));
        };
        let root = root.ok_or(RuntimeError::Invariant(
            "JSON reviver root holder was not allocated",
        ))?;
        let empty = self.intern_property_key("")?;
        match self.define_json_reviver_property(realm, &root, &empty, parsed)? {
            PropertyDefineOutcome::Defined(true) => {}
            PropertyDefineOutcome::Defined(false) => {
                return Err(RuntimeError::Invariant(
                    "fresh JSON reviver root definition was rejected",
                ));
            }
            PropertyDefineOutcome::Throw(value) => return Ok(Completion::Throw(value)),
        }
        self.internalize_json_property(realm, &root, &empty, &reviver, &source, record.as_ref(), 0)
    }

    #[allow(clippy::too_many_arguments)]
    fn internalize_json_property(
        &self,
        realm: ContextId,
        holder: &ObjectRef,
        name: &PropertyKey,
        reviver: &CallableRef,
        source: &JsString,
        record: Option<&JsonParseRecord>,
        depth: usize,
    ) -> Result<Completion, RuntimeError> {
        if depth > MAX_JSON_REVIVER_DEPTH {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        }

        let value = match self.get_property_in_realm(realm, holder, name)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(Completion::Throw(value)),
        };
        let record = record.filter(|record| record.matches(&value));
        // QuickJS allocates one fresh context object for every reviver call,
        // before recursively walking an object value.
        let context = self.new_ordinary_object_in_realm(realm)?;

        if let Value::Object(object) = &value {
            if self.is_array_object(object)? {
                let (length, _) = self.array_length_state(object)?;
                for index in 0..length {
                    let key = self.intern_property_key(&index.to_string())?;
                    let child_record = record.and_then(|record| {
                        usize::try_from(index)
                            .ok()
                            .and_then(|index| record.array_child(index))
                    });
                    let replacement = match self.internalize_json_property(
                        realm,
                        object,
                        &key,
                        reviver,
                        source,
                        child_record,
                        depth + 1,
                    )? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if matches!(replacement, Value::Undefined) {
                        let _ = self.delete_property(object, &key)?;
                    } else if let PropertyDefineOutcome::Throw(value) =
                        self.define_json_reviver_property(realm, object, &key, replacement)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
            } else {
                // Snapshot enumerable own string keys before invoking the
                // first child reviver. Later mutations neither add new keys to
                // the walk nor remove already snapshotted names.
                let mut keys = Vec::new();
                for key in self.own_property_keys(object)? {
                    if self.0.state.borrow().atoms.property_key_kind(key.atom())?
                        != PropertyKeyKind::String
                    {
                        continue;
                    }
                    let Some(descriptor) = self.get_own_property(object, &key)? else {
                        continue;
                    };
                    let enumerable = match descriptor {
                        CompleteOrdinaryPropertyDescriptor::Data { enumerable, .. }
                        | CompleteOrdinaryPropertyDescriptor::Accessor { enumerable, .. } => {
                            enumerable
                        }
                    };
                    if enumerable {
                        keys.push(key);
                    }
                }
                for key in keys {
                    let child_record = record.and_then(|record| record.object_child(&key));
                    let replacement = match self.internalize_json_property(
                        realm,
                        object,
                        &key,
                        reviver,
                        source,
                        child_record,
                        depth + 1,
                    )? {
                        Completion::Return(value) => value,
                        Completion::Throw(value) => return Ok(Completion::Throw(value)),
                    };
                    if matches!(replacement, Value::Undefined) {
                        let _ = self.delete_property(object, &key)?;
                    } else if let PropertyDefineOutcome::Throw(value) =
                        self.define_json_reviver_property(realm, object, &key, replacement)?
                    {
                        return Ok(Completion::Throw(value));
                    }
                }
            }
        } else if let Some((start, end)) = record.and_then(JsonParseRecord::primitive_span) {
            let source_value = Value::String(source.sub_string(start, end));
            let source_key = self.intern_property_key("source")?;
            match self.define_json_reviver_property(realm, &context, &source_key, source_value)? {
                PropertyDefineOutcome::Defined(true) => {}
                PropertyDefineOutcome::Defined(false) => {
                    return Err(RuntimeError::Invariant(
                        "fresh JSON reviver source definition was rejected",
                    ));
                }
                PropertyDefineOutcome::Throw(value) => return Ok(Completion::Throw(value)),
            }
        }

        let name = Value::String(self.0.state.borrow().atoms.to_js_string(name.atom())?);
        self.call_internal(
            realm,
            reviver,
            Value::Object(holder.clone()),
            &[name, value, Value::Object(context)],
        )
    }

    fn define_json_reviver_property(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        self.define_own_property_in_realm(
            Some(realm),
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
    }
}
