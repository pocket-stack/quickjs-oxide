use quickjs_oxide::{Context, ObjectRef, Runtime, RuntimeError, Value};

use crate::runtime_observation::{
    checked_value_type, error_string_property, plain_value_type, primitive_value_text,
    primitive_value_text_with_rust_float, string_property, string_property_with_read_context,
};
use crate::runtime_oracle::{error_string_property as runtime_error_string_property, value_type};

#[derive(Clone, Copy)]
enum ErrorPropertyStyle {
    Compact,
    ReadContext,
    CaseContext,
    RuntimeCaseContext,
}

#[derive(Clone, Copy)]
enum EngineFailureStyle {
    Source,
    DescriptionOnly,
}

/// Observe the public runtime's completion protocol without adding a
/// domain-specific source prelude.
pub(crate) fn observe_eval_completion(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    observe_eval_completion_with(
        runtime,
        context,
        source,
        description,
        value_type,
        primitive_value_text,
        ErrorPropertyStyle::Compact,
        EngineFailureStyle::Source,
    )
}

/// Observe a completion whose Error-property failures include the case name.
pub(crate) fn observe_eval_completion_with_error_context(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    observe_eval_completion_with(
        runtime,
        context,
        source,
        description,
        value_type,
        primitive_value_text,
        ErrorPropertyStyle::CaseContext,
        EngineFailureStyle::Source,
    )
}

/// Observe a legacy vector which intentionally uses Rust float spelling and
/// the older shared Error-property diagnostic contract.
pub(crate) fn observe_legacy_float_eval_completion(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    observe_eval_completion_with(
        runtime,
        context,
        source,
        description,
        value_type,
        primitive_value_text_with_rust_float,
        ErrorPropertyStyle::RuntimeCaseContext,
        EngineFailureStyle::Source,
    )
}

/// Observe a legacy vector while retaining its explicit callable-inspection
/// assertion and Rust float spelling.
pub(crate) fn observe_checked_legacy_float_eval_completion(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    observe_eval_completion_with(
        runtime,
        context,
        source,
        description,
        checked_value_type,
        primitive_value_text_with_rust_float,
        ErrorPropertyStyle::RuntimeCaseContext,
        EngineFailureStyle::Source,
    )
}

/// Observe a completion after prepending a domain-owned source prelude. The
/// caller keeps ownership of the prelude and its value spelling.
pub(crate) fn observe_read_context_eval_completion_with_prelude(
    runtime: &Runtime,
    context: &mut Context,
    prelude: &str,
    source: &str,
    description: &str,
) -> String {
    let source = source_with_prelude(prelude, source);
    observe_eval_completion_with(
        runtime,
        context,
        &source,
        description,
        value_type,
        primitive_value_text,
        ErrorPropertyStyle::ReadContext,
        EngineFailureStyle::Source,
    )
}

fn observe_eval_completion_without_source_diagnostic(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    observe_eval_completion_with(
        runtime,
        context,
        source,
        description,
        value_type,
        primitive_value_text,
        ErrorPropertyStyle::Compact,
        EngineFailureStyle::DescriptionOnly,
    )
}

#[allow(clippy::too_many_arguments)]
fn observe_eval_completion_with(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
    value_type: fn(&Runtime, &Value) -> &'static str,
    value_text: fn(Value) -> String,
    error_property_style: ErrorPropertyStyle,
    engine_failure_style: EngineFailureStyle,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            value_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    observed_error_string_property(
                        runtime,
                        context,
                        &error,
                        "name",
                        description,
                        error_property_style,
                    ),
                    observed_error_string_property(
                        runtime,
                        context,
                        &error,
                        "message",
                        description,
                        error_property_style,
                    ),
                ),
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    value_text(value),
                ),
            }
        }
        Err(error) => match engine_failure_style {
            EngineFailureStyle::Source => {
                panic!("Rust engine failure for {description} ({source:?}): {error}")
            }
            EngineFailureStyle::DescriptionOnly => {
                panic!("Rust engine failure for {description}: {error}")
            }
        },
    }
}

fn observed_error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
    description: &str,
    style: ErrorPropertyStyle,
) -> String {
    match style {
        ErrorPropertyStyle::Compact => string_property(runtime, context, error, name),
        ErrorPropertyStyle::ReadContext => {
            string_property_with_read_context(runtime, context, error, name)
        }
        ErrorPropertyStyle::CaseContext => {
            error_string_property(runtime, context, error, name, description)
        }
        ErrorPropertyStyle::RuntimeCaseContext => {
            runtime_error_string_property(runtime, context, error, name, description)
        }
    }
}

fn source_with_prelude(prelude: &str, source: &str) -> String {
    format!("{prelude}\n{source}")
}

/// Exercise the shared observation contracts from an existing oracle test so
/// helper extraction cannot silently change every downstream vector at once.
pub(crate) fn assert_runtime_completion_helper_contracts() {
    assert_eq!(primitive_value_text(Value::Float(f64::NAN)), "NaN");
    assert_eq!(primitive_value_text(Value::Float(-0.0)), "0");
    assert_eq!(
        primitive_value_text(Value::Float(f64::INFINITY)),
        "Infinity"
    );
    assert_eq!(primitive_value_text(Value::Float(1.5)), "1.5");
    assert_eq!(
        primitive_value_text_with_rust_float(Value::Float(-0.0)),
        "-0"
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let callable = context.eval("(function(){})").unwrap();
    let ordinary = context.eval("({})").unwrap();
    assert_eq!(checked_value_type(&runtime, &callable), "function");
    assert_eq!(checked_value_type(&runtime, &ordinary), "object");
    assert_eq!(plain_value_type(&callable), "object");

    assert_eq!(
        observe_eval_completion(
            &runtime,
            &mut context,
            "throw 42",
            "thrown primitive canary"
        ),
        "throw|number|42"
    );
    assert_eq!(
        observe_eval_completion(
            &runtime,
            &mut context,
            "throw new Error('boom')",
            "Error object canary",
        ),
        "throw|object|Error|boom"
    );

    let joined = source_with_prelude("var marker = 1;", "marker + 41");
    assert_eq!(joined, "var marker = 1;\nmarker + 41");
    assert_eq!(
        observe_read_context_eval_completion_with_prelude(
            &runtime,
            &mut context,
            "var marker = 1;",
            "marker + 41",
            "prelude identity canary",
        ),
        "return|number|42"
    );
}

/// Observe QuickJS after prepending a domain-owned source prelude.
pub(crate) fn observe_quickjs_completion_with_prelude(
    prelude: &str,
    oracle: &std::ffi::OsStr,
    source: &str,
    description: &str,
) -> String {
    let source = source_with_prelude(prelude, source);
    crate::quickjs_oracle::observe_completion(oracle, &source, description)
}

/// Compare source/completion vectors against the pinned QuickJS oracle.
pub(crate) fn compare_eval_completion_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_eval_completion(&runtime, &mut context, source, description),
            crate::quickjs_oracle::observe_completion(&oracle, source, description),
            "{group} drifted for {description}: {source:?}",
        );
    }
}

/// Compare prefixed vectors which retain their compact, fail-fast diagnostics.
pub(crate) fn compare_eval_completion_cases_with_prelude(
    prelude: &str,
    group: &str,
    cases: &[(&str, &str)],
) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = source_with_prelude(prelude, source);
        assert_eq!(
            observe_eval_completion_without_source_diagnostic(
                &runtime,
                &mut context,
                &source,
                description,
            ),
            crate::quickjs_oracle::observe_completion(&oracle, &source, description),
            "{group} drifted for {description}",
        );
    }
}

/// Compare prefixed vectors while collecting every completion mismatch in the
/// group before failing.
pub(crate) fn compare_read_context_eval_completion_cases_with_prelude(
    prelude: &str,
    group: &str,
    cases: &[(&str, &str)],
) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for &(description, original_source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = source_with_prelude(prelude, original_source);
        let actual = observe_eval_completion_with(
            &runtime,
            &mut context,
            &source,
            description,
            value_type,
            primitive_value_text,
            ErrorPropertyStyle::ReadContext,
            EngineFailureStyle::Source,
        );
        let expected = crate::quickjs_oracle::observe_completion(&oracle, &source, description);
        if actual != expected {
            failures.push(format!(
                "{description}\nsource: {original_source:?}\noxide: {actual:?}\noracle: {expected:?}",
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{group} drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}
