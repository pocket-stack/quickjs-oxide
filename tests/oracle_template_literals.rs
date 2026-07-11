use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    AccessorValue, Context, DescriptorField, OrdinaryPropertyDescriptor, Runtime, RuntimeError,
    Value,
};

const ORACLE_NORMALIZER: &str = r#"
var __qjo_type = typeof __qjo_value;
if (__qjo_type === "number") {
    if (__qjo_value !== __qjo_value) {
        print("number|NaN");
    } else if (__qjo_value === 0 && 1 / __qjo_value === -Infinity) {
        print("number|-0");
    } else if (__qjo_value === Infinity) {
        print("number|Infinity");
    } else if (__qjo_value === -Infinity) {
        print("number|-Infinity");
    } else {
        print("number|" + String(__qjo_value));
    }
} else if (__qjo_type === "string") {
    var __qjo_units = "";
    for (var __qjo_index = 0; __qjo_index < __qjo_value.length; __qjo_index++) {
        var __qjo_hex = __qjo_value.charCodeAt(__qjo_index).toString(16);
        if (__qjo_index !== 0) __qjo_units += ",";
        __qjo_units += ("0000" + __qjo_hex).slice(-4);
    }
    print("string|" + __qjo_value.length + "|" + __qjo_units);
} else if (__qjo_value === null) {
    print("object|null");
} else {
    print(__qjo_type + "|" + String(__qjo_value));
}
"#;

const VALUE_CASES: &[(&str, &str)] = &[
    ("empty no-substitution template", "``"),
    ("plain no-substitution template", "`quickjs`"),
    ("UTF-16 and astral text", "`é中🚀`"),
    ("numeric and Unicode escapes", r"`\0\x41\u0042\u{1F680}`"),
    ("lone surrogate escapes", r"`\uD800x\uDC00`"),
    ("simple escapes", r"`\b\t\n\v\f\r`"),
    ("escaped substitution opener", r"`\${value}`"),
    ("escaped template quote", r"`\``"),
    ("escaped backslash", r"`\\`"),
    ("physical LF normalizes to LF", "`a\nb`"),
    ("physical CR normalizes to LF", "`a\rb`"),
    ("physical CRLF normalizes to LF", "`a\r\nb`"),
    ("physical line separator remains", "`a\u{2028}b`"),
    ("physical paragraph separator remains", "`a\u{2029}b`"),
    ("LF line continuation disappears", "`a\\\nb`"),
    ("CR line continuation disappears", "`a\\\rb`"),
    ("CRLF line continuation disappears", "`a\\\r\nb`"),
    ("line separator continuation disappears", "`a\\\u{2028}b`"),
    (
        "paragraph separator continuation disappears",
        "`a\\\u{2029}b`",
    ),
    ("single interpolation", "`${42}`"),
    (
        "primitive interpolation sequence",
        "`a${1}b${true}c${null}d${void 0}e`",
    ),
    (
        "special Number interpolation",
        "`${-0}|${0/0}|${1/0}|${-1/0}`",
    ),
    (
        "BigInt interpolation",
        "`x${123456789012345678901234567890n}y`",
    ),
    ("astral interpolation", "`a${\"🚀\"}b`"),
    ("adjacent substitutions", "`${1}${2}`"),
    ("empty tail is omitted", "`a${1}`"),
    ("empty head remains the receiver", "`${1}z`"),
    ("raw right brace", "`a}b`"),
    ("escaped interpolation remains text", r"`a\${1}b`"),
    ("nested template", "`a${`b${2}c`}d`"),
    ("three nested templates", "`a${`b${`c${3}d`}e`}f`"),
    ("comma expression substitution", "`a${(1,2)}b`"),
    ("conditional substitution", "`a${false ? 1 : 2}b`"),
    ("division inside substitution", "`a${8 / 2}b`"),
    ("division after template", "`8` / 2"),
    ("nested template followed by division", "`${`8` / 2}`"),
    (
        "function body and call inside substitution",
        "`a${(function(){ return 7; })()}b`",
    ),
    (
        "template returned by substitution call",
        "`a${(function(){ return `b${3}c`; })()}d`",
    ),
    (
        "substitutions evaluate before native concat coercion",
        "(function(){ var log=''; var a=function(){}, b=function(){}; a.toString=function(){log=log+'a';return 'A';}; b.toString=function(){log=log+'b';return 'B';}; var one=function(){log=log+'1';return a;}; var two=function(){log=log+'2';return b;}; var value=`x${one()}y${two()}z`; return value+'|'+log; })()",
    ),
    (
        "no-substitution template skips concat",
        "TemplateStringPrototype.concat=1; `plain`",
    ),
    (
        "concat receives primitive receiver and raw arguments",
        "TemplateStringPrototype.concat=function(a,b,c,d,e){'use strict';return typeof this+'|'+this+'|'+typeof a+'|'+a+'|'+typeof b+'|'+b+'|'+typeof c+'|'+c+'|'+typeof d+'|'+d+'|'+typeof e;}; `a${42}b${true}c`",
    ),
    (
        "empty later cooked segments are omitted",
        "TemplateStringPrototype.concat=function(a,b,c,d){'use strict';return this+'|'+a+'|'+b+'|'+typeof c+'|'+typeof d;}; `${1}${2}`",
    ),
    (
        "concat call waits for every substitution",
        "(function(){var log='';TemplateStringPrototype.concat=function(a,b,c,d){'use strict';log=log+'h';return 'hook:'+this+':'+a+':'+b+':'+c+':'+d;};var one=function(){log=log+'1';return 10;};var two=function(){log=log+'2';return 20;};var value=`a${one()}b${two()}c`;return value+'|'+log;})()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("empty unterminated template", "`"),
    ("unterminated template text", "`abc"),
    ("unterminated substitution", "`a${1"),
    ("unterminated continuation", "`a${1}"),
    ("empty substitution", "`${}`"),
    ("extra expression token", "`a${1 2}b`"),
    ("semicolon in substitution", "`a${1;2}b`"),
    ("mismatched parenthesis", "`a${(1}b`"),
    ("empty later substitution", "`a${1}b${}c`"),
    ("short hex escape", r"`\x`"),
    ("non-hex escape", r"`\xG0`"),
    ("short Unicode escape", r"`\u`"),
    ("non-hex Unicode escape", r"`\u00G0`"),
    ("empty braced Unicode escape", r"`\u{}`"),
    ("out-of-range Unicode escape", r"`\u{110000}`"),
    ("legacy octal escape", r"`\1`"),
    ("legacy zero-prefixed escape", r"`\01`"),
    ("legacy eight escape", r"`\8`"),
    ("malformed tail escape", r"`a${1}\xG0`"),
    ("multiline empty substitution", "`a${\n}`"),
    ("multiline unterminated continuation", "`a${\n1\n`"),
    (
        "initial malformed escape beats substitution grammar",
        r"`\x${1 2}\x`",
    ),
    (
        "substitution grammar beats later malformed escape",
        r"`a${1 2}\x`",
    ),
    ("malformed tail follows valid substitution", r"`a${1}\x`"),
    ("nested malformed escape", "`a${`\\x`}b`"),
];

#[test]
fn untagged_template_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP template differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        expose_string_prototype(&runtime, &mut context);
        let value = context
            .eval(source)
            .unwrap_or_else(|error| panic!("Rust rejected {description:?} ({source:?}): {error}"));
        assert_eq!(
            normalize_rust_value(&value),
            oracle_value_observation(&oracle, source, description),
            "value mismatch for {description:?} ({source:?})"
        );
    }
}

#[test]
fn untagged_template_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP template diagnostic differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        assert_eq!(
            rust_error_observation(source),
            oracle_error_observation(&oracle, source),
            "diagnostic mismatch for {description:?} ({source:?})"
        );
    }
}

#[test]
fn template_concat_lookup_order_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP template getter differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        (
            "getter precedes expressions and hook follows them",
            "Function.trace='';(function(){var one=function(){Function.trace=Function.trace+'1';return 10;};var two=function(){Function.trace=Function.trace+'2';return 20;};var value=`a${one()}b${two()}c`;return value+'|'+Function.trace;})()",
        ),
        (
            "no-substitution template skips getter",
            "Function.trace='';(function(){var value=`plain`;return value+'|'+Function.trace;})()",
        ),
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        install_concat_getter(&runtime, &mut context);
        let value = context
            .eval(source)
            .unwrap_or_else(|error| panic!("Rust rejected {description:?}: {error}"));
        assert_eq!(
            normalize_rust_value(&value),
            oracle_getter_value_observation(&oracle, source, description),
            "concat getter order drifted for {description:?}"
        );
    }
}

#[test]
fn template_concat_getter_fault_sites_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP template getter stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (description, source) in [
        (
            "new statement replaces a previous operator marker",
            "1+2;\n`a${3}b`",
        ),
        (
            "composite template inherits expression entry",
            "1 + `a${2}b`",
        ),
        (
            "function-body expression entry",
            "(function(){ 1+2;\n`a${3}b`; })()",
        ),
    ] {
        assert_eq!(
            rust_getter_fault_location(source),
            oracle_getter_fault_location(&oracle, source, description),
            "concat getter fault site drifted for {description:?}"
        );
    }
}

#[test]
fn template_stack_limit_uses_reachable_bytecode_like_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP template stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for (count, suffix) in [(65_533, "limit"), (65_536, "wrapped-argc")] {
        let substitutions = "${0}".repeat(count);
        let reachable = format!("`{substitutions}`");
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.compile(&reachable), Err(RuntimeError::Exception));
        assert_eq!(
            take_rust_error_name_message(&runtime, &mut context),
            "InternalError|stack overflow"
        );
        let upstream = run_oracle_file(&oracle, &reachable, &format!("{suffix}-reachable"));
        assert!(
            !upstream.status.success(),
            "QuickJS accepted stack overflow at {count} substitutions"
        );
        assert!(
            String::from_utf8(upstream.stderr)
                .unwrap()
                .starts_with("InternalError: stack overflow"),
            "QuickJS stack-overflow category drifted at {count} substitutions"
        );

        let unreachable = format!("(function(){{return 1;{reachable};}})");
        context
            .compile(&unreachable)
            .expect("Rust rejected unreachable oversized template");
        let upstream = run_oracle_file(&oracle, &unreachable, &format!("{suffix}-unreachable"));
        assert!(
            upstream.status.success(),
            "QuickJS rejected unreachable oversized template: {}",
            String::from_utf8_lossy(&upstream.stderr)
        );
    }
}

#[test]
fn tagged_templates_remain_an_explicit_boundary() {
    for source in [
        "tag`x`",
        "tag\n`x`",
        "(tag)`x`",
        "Function.name`x`",
        "Function()`x`",
        "`tag``${1}`",
        r"tag`\x`",
        "tag\n`\\8`",
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(source), Err(RuntimeError::Exception));
        let observation = take_rust_error(&runtime, &mut context);
        assert!(
            observation
                .starts_with("SyntaxError|tagged template literals are not implemented yet|"),
            "tagged boundary drifted for {source:?}: {observation}"
        );
    }
}

fn expose_string_prototype(runtime: &Runtime, context: &mut Context) {
    let prototype = context.string_prototype().unwrap();
    let global = context.global_object().unwrap();
    let key = runtime
        .intern_property_key("TemplateStringPrototype")
        .unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Object(prototype)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn install_concat_getter(runtime: &Runtime, context: &mut Context) {
    install_concat_getter_source(
        runtime,
        context,
        "(function(){Function.trace=Function.trace+'g';return function(a,b,c,d){'use strict';Function.trace=Function.trace+'h';return 'hook:'+this+':'+a+':'+b+':'+c+':'+d;};})",
    );
}

fn install_throwing_concat_getter(runtime: &Runtime, context: &mut Context) {
    install_concat_getter_source(
        runtime,
        context,
        "(function(){throw new Error('template getter')})",
    );
}

fn install_concat_getter_source(runtime: &Runtime, context: &mut Context, source: &str) {
    let getter = context.eval(source).unwrap();
    let Value::Object(getter) = getter else {
        panic!("concat getter source did not produce a function object");
    };
    let getter = runtime.as_callable(&getter).unwrap().unwrap();
    let prototype = context.string_prototype().unwrap();
    let concat = runtime.intern_property_key("concat").unwrap();
    assert!(
        context
            .define_own_property(
                &prototype,
                &concat,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn rust_getter_fault_location(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    install_throwing_concat_getter(&runtime, &mut context);
    assert_eq!(
        context.eval_with_filename(source, "<evalScript>"),
        Err(RuntimeError::Exception)
    );
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("template getter did not throw an Error object");
    };
    let stack = runtime.intern_property_key("stack").unwrap();
    let Value::String(stack) = context.get_property(&error, &stack).unwrap() else {
        panic!("template getter Error.stack was not a string");
    };
    eval_script_location(&stack.to_utf8_lossy())
}

fn oracle_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!(
        "globalThis.TemplateStringPrototype = String.prototype;\nvar __qjo_value = std.evalScript(scriptArgs[0]);\n{ORACLE_NORMALIZER}"
    );
    let output = Command::new(oracle)
        .args(["--std", "-e", &script, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description:?}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS rejected {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS value output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn oracle_getter_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!(
        "Object.defineProperty(String.prototype, 'concat', {{get:function(){{Function.trace=Function.trace+'g';return function(a,b,c,d){{'use strict';Function.trace=Function.trace+'h';return 'hook:'+this+':'+a+':'+b+':'+c+':'+d;}};}}, configurable:true}});\nvar __qjo_value = std.evalScript(scriptArgs[0]);\n{ORACLE_NORMALIZER}"
    );
    let output = Command::new(oracle)
        .args(["--std", "-e", &script, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description:?}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS rejected {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS getter output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn oracle_getter_fault_location(oracle: &OsStr, source: &str, description: &str) -> String {
    let setup = "Object.defineProperty(String.prototype,'concat',{get:function(){throw new Error('template getter')},configurable:true});std.evalScript(scriptArgs[0]);";
    let output = Command::new(oracle)
        .args(["--std", "-e", setup, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description:?}: {error}"));
    assert!(
        !output.status.success(),
        "QuickJS getter unexpectedly completed {description:?}"
    );
    let stderr = String::from_utf8(output.stderr).expect("QuickJS getter stack was not UTF-8");
    eval_script_location(&stderr)
}

fn eval_script_location(stack: &str) -> String {
    stack
        .lines()
        .find_map(|line| {
            line.trim()
                .split_once("(<evalScript>:")
                .and_then(|(_, location)| location.strip_suffix(')'))
        })
        .unwrap_or_else(|| panic!("stack had no evalScript frame: {stack:?}"))
        .to_owned()
}

fn rust_error_observation(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    take_rust_error(&runtime, &mut context)
}

fn take_rust_error(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust parser did not materialize an Error object");
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(context, "name") else {
        panic!("Rust Error.name was not a string");
    };
    let Value::String(message) = read(context, "message") else {
        panic!("Rust Error.message was not a string");
    };
    let Value::Int(line) = read(context, "lineNumber") else {
        panic!("Rust Error.lineNumber was not an integer");
    };
    let Value::Int(column) = read(context, "columnNumber") else {
        panic!("Rust Error.columnNumber was not an integer");
    };
    format!(
        "{}|{}|{line}:{column}",
        name.to_utf8_lossy(),
        message.to_utf8_lossy()
    )
}

fn take_rust_error_name_message(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("Rust compiler did not materialize an Error object");
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(context, "name") else {
        panic!("Rust Error.name was not a string");
    };
    let Value::String(message) = read(context, "message") else {
        panic!("Rust Error.message was not a string");
    };
    format!("{}|{}", name.to_utf8_lossy(), message.to_utf8_lossy())
}

fn run_oracle_file(oracle: &OsStr, source: &str, suffix: &str) -> std::process::Output {
    let path = std::env::temp_dir().join(format!(
        "quickjs-oxide-template-stack-{}-{suffix}.js",
        std::process::id()
    ));
    std::fs::write(&path, source).expect("write temporary QuickJS source");
    let output = Command::new(oracle)
        .args(["--stack-size", "32M"])
        .arg(&path)
        .output()
        .expect("run QuickJS stack oracle");
    std::fs::remove_file(path).expect("remove temporary QuickJS source");
    output
}

fn oracle_error_observation(oracle: &OsStr, source: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {source:?}: {error}"));
    assert!(!output.status.success(), "QuickJS accepted {source:?}");
    let stderr = String::from_utf8(output.stderr).expect("QuickJS error output was not UTF-8");
    let mut lines = stderr.lines();
    let first = lines
        .find(|line| line.starts_with("SyntaxError: "))
        .unwrap_or_else(|| panic!("QuickJS emitted no SyntaxError for {source:?}: {stderr}"));
    let location = lines
        .find_map(|line| line.trim().strip_prefix("at <cmdline>:"))
        .unwrap_or_else(|| panic!("QuickJS emitted no location for {source:?}: {stderr}"));
    format!(
        "SyntaxError|{}|{location}",
        first.strip_prefix("SyntaxError: ").unwrap()
    )
}

fn normalize_rust_value(value: &Value) -> String {
    match value {
        Value::Undefined => "undefined|undefined".to_owned(),
        Value::Null => "object|null".to_owned(),
        Value::Bool(value) => format!("boolean|{value}"),
        Value::Int(value) => normalize_number(f64::from(*value)),
        Value::Float(value) => normalize_number(*value),
        Value::BigInt(value) => format!("bigint|{value}"),
        Value::String(value) => {
            let units = value
                .utf16_units()
                .map(|unit| format!("{unit:04x}"))
                .collect::<Vec<_>>()
                .join(",");
            format!("string|{}|{units}", value.len())
        }
        Value::Symbol(_) => "symbol|<identity>".to_owned(),
        Value::Object(_) => "object|<identity>".to_owned(),
    }
}

#[allow(clippy::float_cmp)]
fn normalize_number(value: f64) -> String {
    if value.is_nan() {
        "number|NaN".to_owned()
    } else if value == 0.0 && value.is_sign_negative() {
        "number|-0".to_owned()
    } else if value == f64::INFINITY {
        "number|Infinity".to_owned()
    } else if value == f64::NEG_INFINITY {
        "number|-Infinity".to_owned()
    } else {
        format!("number|{}", number_to_string(value))
    }
}
