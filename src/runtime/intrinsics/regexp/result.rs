//! Construction of builtin RegExp match result arrays.

use std::rc::Rc;

use crate::regexp::{CompiledRegExp, RegExpFlags, RegExpMatch};

use super::super::super::*;

impl Runtime {
    pub(super) fn build_regexp_result(
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

        let groups = group_names.map(|_| self.new_object(None)).transpose()?;
        let has_indices = program.flags().contains(RegExpFlags::HAS_INDICES);
        let indices_groups = if has_indices && group_names.is_some() {
            Some(self.new_object(None)?)
        } else {
            None
        };
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
                self.define_named_capture_result(
                    groups.as_ref().ok_or(RuntimeError::Invariant(
                        "compiled RegExp names had no groups result object",
                    ))?,
                    &key,
                    capture,
                )?;
                if let Some(index_value) = &index_value {
                    self.define_named_capture_result(
                        indices_groups.as_ref().ok_or(RuntimeError::Invariant(
                            "compiled RegExp names had no indices groups object",
                        ))?,
                        &key,
                        index_value.clone(),
                    )?;
                }
            }

            if let (Some(values), Some(value)) = (&mut indices_values, index_value) {
                values.push(value);
            }
        }

        let result = self.new_array_from_values(realm, captures)?;
        let complete = matched.capture(0).ok_or(RuntimeError::Invariant(
            "successful RegExp result omitted capture zero",
        ))?;
        self.define_regexp_result_property(
            &result,
            "index",
            Value::Int(i32::try_from(complete.start).map_err(|_| {
                RuntimeError::Invariant("RegExp match start exceeded signed String range")
            })?),
        )?;
        self.define_regexp_result_property(&result, "input", Value::String(input))?;
        self.define_regexp_result_property(
            &result,
            "groups",
            groups.map_or(Value::Undefined, Value::Object),
        )?;

        if let Some(values) = indices_values {
            let indices = self.new_array_from_values(realm, values)?;
            self.define_regexp_result_property(
                &indices,
                "groups",
                indices_groups.map_or(Value::Undefined, Value::Object),
            )?;
            self.define_regexp_result_property(&result, "indices", Value::Object(indices))?;
        }
        Ok(Value::Object(result))
    }

    /// QuickJS lets a participating duplicate-named capture replace an
    /// earlier `undefined`, while an unmatched duplicate never erases an
    /// already-defined value. Defining an existing property also preserves
    /// the first capture's insertion order.
    fn define_named_capture_result(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if matches!(value, Value::Undefined) && self.has_own_property(object, key)? {
            return Ok(());
        }
        self.define_regexp_result_property_with_key(object, key, value)
    }

    fn define_regexp_result_property(
        &self,
        object: &ObjectRef,
        name: &str,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key(name)?;
        self.define_regexp_result_property_with_key(object, &key, value)
    }

    fn define_regexp_result_property_with_key(
        &self,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<(), RuntimeError> {
        if !self.define_own_property(
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
            return Err(RuntimeError::Invariant(
                "fresh RegExp result property definition was rejected",
            ));
        }
        Ok(())
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
