// Keep the Array-method oracle implementations in separate modules so their
// private helpers remain isolated while Cargo builds one integration target.

use crate::quickjs_oracle;

mod support {
    use std::ffi::OsStr;

    pub(super) use crate::runtime_observation::{
        error_string_property, primitive_value_text,
        property_callable_with_read_context as property_callable, take_exception_object,
    };

    use quickjs_oxide::{
        CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError, Value,
    };

    pub(super) fn compare_value_cases(group: &str, cases: &[(&str, &str)]) {
        let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
            eprintln!("SKIP {group} differential: set QJS_ORACLE to upstream qjs");
            return;
        };
        for &(description, source) in cases {
            let expected = observe_oracle(&oracle, source, description);
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            assert_eq!(
                observe_rust_eval(&runtime, &mut context, source, description),
                expected,
                "{group} drifted for {description}: {source:?}",
            );
        }
    }

    pub(super) fn observe_rust_eval(
        runtime: &Runtime,
        context: &mut Context,
        source: &str,
        description: &str,
    ) -> String {
        match context.eval(source) {
            Ok(value) => format!(
                "return|{}|{}",
                value_type(runtime, &value),
                primitive_value_text(value),
            ),
            Err(RuntimeError::Exception) => {
                let exception = context
                    .take_exception()
                    .unwrap_or_else(|error| {
                        panic!("take Rust exception for {description}: {error}")
                    })
                    .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
                match exception {
                    Value::Object(error) => format!(
                        "throw|object|{}|{}",
                        error_string_property(runtime, context, &error, "name", description),
                        error_string_property(runtime, context, &error, "message", description),
                    ),
                    value => format!(
                        "throw|{}|{}",
                        value_type(runtime, &value),
                        primitive_value_text(value),
                    ),
                }
            }
            Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
        }
    }

    pub(super) fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
        super::quickjs_oracle::observe_completion(oracle, source, description)
    }

    pub(super) fn data_descriptor_bits(descriptor: &CompleteOrdinaryPropertyDescriptor) -> String {
        let CompleteOrdinaryPropertyDescriptor::Data {
            writable,
            enumerable,
            configurable,
            ..
        } = descriptor
        else {
            panic!("expected a data descriptor");
        };
        format!(
            "D{}{}{}",
            Number(*writable),
            Number(*enumerable),
            Number(*configurable),
        )
    }

    pub(super) fn method_metadata(
        runtime: &Runtime,
        context: &mut Context,
        owner: &ObjectRef,
        function_prototype: &ObjectRef,
        name: &str,
    ) -> String {
        let key = runtime.intern_property_key(name).unwrap();
        let descriptor = runtime
            .get_own_property(owner, &key)
            .unwrap()
            .unwrap_or_else(|| panic!("missing Array.prototype.{name}"));
        let CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(function),
            writable,
            enumerable,
            configurable,
        } = &descriptor
        else {
            panic!("Array.prototype.{name} was not a function data property");
        };
        let callable = runtime
            .as_callable(function)
            .unwrap()
            .unwrap_or_else(|| panic!("Array.prototype.{name} was not callable"));
        let function_name = context
            .get_property(function, &runtime.intern_property_key("name").unwrap())
            .unwrap();
        let function_length = context
            .get_property(function, &runtime.intern_property_key("length").unwrap())
            .unwrap();
        let name_descriptor = runtime
            .get_own_property(function, &runtime.intern_property_key("name").unwrap())
            .unwrap()
            .unwrap_or_else(|| panic!("Array.{name} name descriptor was missing"));
        let length_descriptor = runtime
            .get_own_property(function, &runtime.intern_property_key("length").unwrap())
            .unwrap()
            .unwrap_or_else(|| panic!("Array.{name} length descriptor was missing"));
        format!(
            "{name}:{}:{}:D{}{}{}:{}:{}:{}:{}:{}",
            primitive_value_text(function_name),
            primitive_value_text(function_length),
            Number(*writable),
            Number(*enumerable),
            Number(*configurable),
            data_descriptor_bits(&name_descriptor),
            data_descriptor_bits(&length_descriptor),
            true,
            runtime.get_prototype_of(function).unwrap().as_ref() == Some(function_prototype),
            runtime.is_constructor(callable.as_object()).unwrap(),
        )
    }

    pub(super) fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
        let Value::Object(object) = context
            .eval(source)
            .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"))
        else {
            panic!("Rust {description} did not evaluate to an object");
        };
        object
    }

    pub(super) fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
        match value {
            Value::Undefined => "undefined",
            Value::Null => "object",
            Value::Bool(_) => "boolean",
            Value::Int(_) | Value::Float(_) => "number",
            Value::BigInt(_) => "bigint",
            Value::String(_) => "string",
            Value::Object(object) => {
                if runtime.as_callable(object).unwrap().is_some() {
                    "function"
                } else {
                    "object"
                }
            }
            Value::Symbol(_) => "symbol",
        }
    }

    pub(super) struct Number(pub(super) bool);

    impl std::fmt::Display for Number {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str(if self.0 { "1" } else { "0" })
        }
    }
}

#[path = "oracle/array/oracle_array_concat.rs"]
mod oracle_array_concat;
#[path = "oracle/array/oracle_array_copy_within.rs"]
mod oracle_array_copy_within;
#[path = "oracle/array/oracle_array_fill.rs"]
mod oracle_array_fill;
#[path = "oracle/array/oracle_array_find.rs"]
mod oracle_array_find;
#[path = "oracle/array/oracle_array_flatten.rs"]
mod oracle_array_flatten;
#[path = "oracle/array/oracle_array_iteration.rs"]
mod oracle_array_iteration;
#[path = "oracle/array/oracle_array_map_filter.rs"]
mod oracle_array_map_filter;
#[path = "oracle/array/oracle_array_mutators.rs"]
mod oracle_array_mutators;
#[path = "oracle/array/oracle_array_reduce.rs"]
mod oracle_array_reduce;
#[path = "oracle/array/oracle_array_reverse.rs"]
mod oracle_array_reverse;
#[path = "oracle/array/oracle_array_slice_splice.rs"]
mod oracle_array_slice_splice;
#[path = "oracle/array/oracle_array_sort.rs"]
mod oracle_array_sort;
#[path = "oracle/array/oracle_array_stringification.rs"]
mod oracle_array_stringification;
#[path = "oracle/array/oracle_array_with.rs"]
mod oracle_array_with;
