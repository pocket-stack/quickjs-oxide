//! Construction of builtin RegExp match result arrays.

use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::heap::{ContextId, ObjectData, ObjectPayload, PropertySlot};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use std::collections::HashMap;

use crate::engine::object::{ObjectRef, PropertyKey};
use crate::engine::value::{JsString, Value};
use crate::regexp::{CompiledRegExp, RegExpFlags, RegExpMatch};
use std::rc::Rc;

/// First-occurrence order and the participating value are separate rules for
/// duplicate group names. Keep their resolution here, before heap publication.
#[derive(Default)]
struct NamedCaptures {
    positions: HashMap<Atom, usize>,
    values: Vec<(PropertyKey, Value, Value)>,
}

impl NamedCaptures {
    fn record(&mut self, key: PropertyKey, capture: Value, indices: Value) {
        if let Some(&position) = self.positions.get(&key.atom()) {
            if !matches!(capture, Value::Undefined) {
                self.values[position].1 = capture;
                self.values[position].2 = indices;
            }
        } else {
            self.positions.insert(key.atom(), self.values.len());
            self.values.push((key, capture, indices));
        }
    }
}

impl Runtime {
    pub(crate) fn build_regexp_result(
        &self,
        realm: ContextId,
        input: JsString,
        program: Rc<CompiledRegExp>,
        matched: RegExpMatch,
    ) -> Result<Value, RuntimeError> {
        let capture_count = matched.captures().len();
        if usize::from(program.capture_count()) != capture_count {
            return Err(RuntimeError::Invariant(
                "compiled RegExp capture count did not align with its match",
            ));
        }
        let group_names = program.group_names();
        if let Some(group_names) = group_names
            && group_names.len() != capture_count.saturating_sub(1)
        {
            return Err(RuntimeError::Invariant(
                "compiled RegExp group names did not align with captures",
            ));
        }

        let has_indices = program.flags().contains(RegExpFlags::HAS_INDICES);
        let mut named = NamedCaptures::default();
        let mut captures = Vec::with_capacity(capture_count);
        let mut indices_values = has_indices.then(|| Vec::with_capacity(capture_count));

        for (capture_index, range) in matched.captures().iter().enumerate() {
            let capture = match range {
                Some(range) => Value::String(input.sub_string(range.start, range.end)),
                None => Value::Undefined,
            };
            captures.push(capture.clone());

            let index_value = if has_indices {
                Some(match range {
                    Some(range) => {
                        let start = i32::try_from(range.start).map_err(|_| {
                            RuntimeError::Invariant(
                                "RegExp capture start exceeded signed String range",
                            )
                        })?;
                        let end = i32::try_from(range.end).map_err(|_| {
                            RuntimeError::Invariant(
                                "RegExp capture end exceeded signed String range",
                            )
                        })?;
                        Value::Object(self.new_array_from_values(
                            realm,
                            vec![Value::Int(start), Value::Int(end)],
                        )?)
                    }
                    None => Value::Undefined,
                })
            } else {
                None
            };

            if capture_index > 0
                && let Some(Some(group_name)) =
                    group_names.and_then(|names| names.get(capture_index - 1))
            {
                let key = self.intern_property_key_js_string(group_name)?;
                named.record(
                    key,
                    capture,
                    index_value.clone().unwrap_or(Value::Undefined),
                );
            }

            if let (Some(values), Some(value)) = (&mut indices_values, index_value) {
                values.push(value);
            }
        }

        let groups = if group_names.is_some() {
            Value::Object(self.new_regexp_groups(&named, false)?)
        } else {
            Value::Undefined
        };
        let indices_groups = if has_indices && group_names.is_some() {
            Value::Object(self.new_regexp_groups(&named, true)?)
        } else {
            Value::Undefined
        };
        let result = self.new_array_from_values(realm, captures)?;
        let complete = matched.capture(0).ok_or(RuntimeError::Invariant(
            "successful RegExp result omitted capture zero",
        ))?;
        let mut properties = vec![
            (
                "index",
                Value::Int(i32::try_from(complete.start).map_err(|_| {
                    RuntimeError::Invariant("RegExp match start exceeded signed String range")
                })?),
            ),
            ("input", Value::String(input)),
            ("groups", groups),
        ];
        if let Some(values) = indices_values {
            let indices = self.new_array_from_values(realm, values)?;
            self.initialize_regexp_array_properties(&indices, &[("groups", indices_groups)])?;
            properties.push(("indices", Value::Object(indices)));
        }
        self.initialize_regexp_array_properties(&result, &properties)?;
        Ok(Value::Object(result))
    }

    fn new_regexp_groups(
        &self,
        named: &NamedCaptures,
        indices: bool,
    ) -> Result<ObjectRef, RuntimeError> {
        let entries = named
            .values
            .iter()
            .map(|(key, _, _)| ShapeEntry {
                atom: key.atom(),
                flags: PropertyFlags::data(true, true, true),
            })
            .collect::<Vec<_>>();
        let slots = named
            .values
            .iter()
            .map(|(_, capture, range)| {
                self.raw_property_value(if indices { range } else { capture })
                    .map(PropertySlot::Data)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let id = self.0.state.borrow_mut().allocate_object_with_layout(
            None,
            &entries,
            slots,
            ObjectData::ordinary,
        )?;
        Ok(ObjectRef::from_owned_handle(self.clone(), id))
    }

    /// Only called for privately held result Arrays. All keys are new named
    /// C/W/E properties, so no Array index/length rule or JS callback is skipped.
    fn initialize_regexp_array_properties(
        &self,
        object: &ObjectRef,
        properties: &[(&str, Value)],
    ) -> Result<(), RuntimeError> {
        let keys = properties
            .iter()
            .map(|(name, _)| self.intern_property_key(name))
            .collect::<Result<Vec<_>, _>>()?;
        let values = properties
            .iter()
            .map(|(_, value)| self.raw_property_value(value))
            .collect::<Result<Vec<_>, _>>()?;
        let mut state = self.0.state.borrow_mut();
        let data = state.heap.object(object.object_id())?;
        if !matches!(data.payload, ObjectPayload::Array { .. }) || !data.extensible {
            return Err(RuntimeError::Invariant(
                "RegExp result initialization requires a fresh Array",
            ));
        }
        let shape = state.heap.shape(data.shape)?;
        let prototype = shape.prototype();
        let mut entries = shape.entries().to_vec();
        let mut slots = data.slots.clone();
        for (key, value) in keys.iter().zip(values) {
            if state.atoms.array_index(key.atom())?.is_some()
                || entries.iter().any(|entry| entry.atom == key.atom())
            {
                return Err(RuntimeError::Invariant(
                    "RegExp result initialization requires new named keys",
                ));
            }
            entries.push(ShapeEntry {
                atom: key.atom(),
                flags: PropertyFlags::data(true, true, true),
            });
            slots.push(PropertySlot::Data(value));
        }
        state.replace_layout(object.object_id(), prototype, &entries, slots)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_eval_true(source: &str) {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval(source).expect("RegExp result probe threw"),
            Value::Bool(true),
        );
    }

    #[test]
    fn named_groups_are_null_prototype_cwe_objects_in_capture_order() {
        assert_eval_true(
            r#"
var result = /(?<first>a)(?<second>b)?/.exec("a");
var groups = result.groups;
var first = Object.getOwnPropertyDescriptor(groups, "first");
var second = Object.getOwnPropertyDescriptor(groups, "second");
Object.getPrototypeOf(groups) === null &&
Object.keys(groups).join(",") === "first,second" &&
groups.first === "a" && groups.second === undefined &&
first.writable === true && first.enumerable === true && first.configurable === true &&
second.writable === true && second.enumerable === true && second.configurable === true
"#,
        );
    }

    #[test]
    fn named_indices_reuse_the_capture_arrays_and_preserve_unmatched_values() {
        assert_eval_true(
            r#"
var result = /(?<first>a)(?<second>b)?/d.exec("a");
var groups = result.indices.groups;
var first = Object.getOwnPropertyDescriptor(groups, "first");
Object.getPrototypeOf(groups) === null &&
Object.keys(groups).join(",") === "first,second" &&
groups.first === result.indices[1] &&
groups.second === result.indices[2] && groups.second === undefined &&
first.writable === true && first.enumerable === true && first.configurable === true
"#,
        );
    }

    #[test]
    fn duplicate_names_keep_first_order_and_the_participating_value() {
        assert_eval_true(
            r#"
var left = /(?:(?<x>a)|(?<x>b))/.exec("a");
var right = /(?:(?<x>a)|(?<x>b))/.exec("b");
left.groups.x === "a" && right.groups.x === "b" &&
Object.keys(left.groups).join(",") === "x" &&
Object.keys(right.groups).join(",") === "x"
"#,
        );
    }

    #[test]
    fn named_groups_force_standard_string_replace_through_get_substitution() {
        assert_eval_true(
            r#"/b/[Symbol.replace]("b", "<$<x>>") === "<$<x>>" && /(?<x>b)/[Symbol.replace]("b", "<$<x>>") === "<b>""#,
        );
    }
}
