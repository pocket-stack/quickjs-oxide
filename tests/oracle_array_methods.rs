// Keep the Array-method oracle implementations in separate modules so their
// private helpers remain isolated while Cargo builds one integration target.

use crate::quickjs_oracle;

mod support {
    use std::ffi::OsStr;

    use quickjs_oxide::{
        CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
        Value,
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

    pub(super) fn property_callable(
        runtime: &Runtime,
        context: &mut Context,
        object: &ObjectRef,
        name: &str,
    ) -> CallableRef {
        let key = runtime.intern_property_key(name).unwrap();
        let Value::Object(function) = context
            .get_property(object, &key)
            .unwrap_or_else(|error| panic!("read callable {name}: {error}"))
        else {
            panic!("{name} was not an object");
        };
        runtime
            .as_callable(&function)
            .unwrap()
            .unwrap_or_else(|| panic!("{name} was not callable"))
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

    pub(super) fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
        let Value::Object(error) = context
            .take_exception()
            .unwrap_or_else(|failure| panic!("take {description}: {failure}"))
            .unwrap_or_else(|| panic!("{description} was missing"))
        else {
            panic!("{description} was not an object");
        };
        error
    }

    pub(super) fn error_string_property(
        runtime: &Runtime,
        context: &mut Context,
        error: &ObjectRef,
        name: &str,
        description: &str,
    ) -> String {
        let key = runtime.intern_property_key(name).unwrap();
        let Value::String(value) = context
            .get_property(error, &key)
            .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
        else {
            panic!("Error.{name} was not a string for {description}");
        };
        value.to_utf8_lossy()
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

    pub(super) fn primitive_value_text(value: Value) -> String {
        match value {
            Value::Undefined => "undefined".to_owned(),
            Value::Null => "null".to_owned(),
            Value::Bool(value) => value.to_string(),
            Value::Int(value) => value.to_string(),
            Value::Float(value) => quickjs_oxide::value::number_to_string(value),
            Value::BigInt(value) => value.to_string(),
            Value::String(value) => value.to_utf8_lossy(),
            Value::Object(_) => "<object>".to_owned(),
            Value::Symbol(_) => "<symbol>".to_owned(),
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
