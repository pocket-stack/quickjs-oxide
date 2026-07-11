use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

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
} else if (__qjo_type === "symbol") {
    print("symbol|<identity>");
} else if (__qjo_type === "object" || __qjo_type === "function") {
    print("object|<identity>");
} else {
    print(__qjo_type + "|" + String(__qjo_value));
}
"#;

const VALUE_CASES: &[(&str, &str)] = &[
    ("empty switch resets completion", "1; switch(0){}"),
    (
        "unmatched switch resets completion",
        "1; switch(0){case 1:2;}",
    ),
    ("default supplies completion", "switch(0){default:3;}"),
    (
        "matching case falls through later bodies",
        "switch(2){case 1:1;case 2:2;case 3:3;}",
    ),
    (
        "break preserves selected body completion",
        "switch(2){case 1:1;case 2:2;break;case 3:3;}",
    ),
    (
        "default before cases falls through",
        "switch(9){default:4;case 1:1;case 2:2;}",
    ),
    (
        "default middle no match enters default",
        "switch(9){case 1:1;default:4;break;case 2:2;}",
    ),
    (
        "case after default wins before default",
        "switch(2){case 1:1;default:4;break;case 2:2;}",
    ),
    (
        "case before default falls through it",
        "switch(1){case 1:1;default:4;case 2:2;}",
    ),
    (
        "no match tests cases after default before entering it",
        "(function(){var log='';switch(9){case (log+='a',1):log+='A';break;default:log+='D';break;case (log+='b',2):log+='B';}return log;})()",
    ),
    (
        "later match skips the earlier default body",
        "(function(){var log='';switch(2){case (log+='a',1):log+='A';break;default:log+='D';break;case (log+='b',2):log+='B';}return log;})()",
    ),
    (
        "consecutive case clauses share a body",
        "switch(2){case 1:case 2:7;break;default:8;}",
    ),
    (
        "selector is evaluated once",
        "(function(){var log='';var value=(log+='s',2);switch(value){case 2:log+='b';break;}return log;})()",
    ),
    (
        "case tests stop after first match",
        "(function(){var log='';switch((log+='s',2)){case (log+='a',1):log+='A';break;case (log+='b',2):log+='B';break;case (log+='c',3):log+='C';}return log;})()",
    ),
    (
        "all case tests run on no match",
        "(function(){var log='';switch((log+='s',9)){case (log+='a',1):break;case (log+='b',2):break;default:log+='d';}return log;})()",
    ),
    (
        "strict comparison does not coerce selector",
        "(function(){var log='';var value=function(){};value.valueOf=function(){log+='v';return 1;};switch(value){case 1:log+='m';break;default:log+='d';}return log;})()",
    ),
    (
        "strict comparison separates Number and BigInt",
        "switch(1){case 1n:2;default:3;}",
    ),
    (
        "strict comparison equates signed zero",
        "switch(-0){case 0:4;break;default:5;}",
    ),
    (
        "strict comparison does not match NaN",
        "switch(NaN){case NaN:6;break;default:7;}",
    ),
    (
        "strict comparison preserves object identity",
        "switch(Function){case Function:8;break;default:9;}",
    ),
    (
        "distinct function identities do not match",
        "switch(function(){}){case function(){}:8;default:9;}",
    ),
    (
        "global symbols compare by identity",
        "switch(Symbol.for('switch-key')){case Symbol.for('switch-key'):10;break;default:11;}",
    ),
    (
        "unique symbols with equal descriptions do not match",
        "switch(Symbol('switch-key')){case Symbol('switch-key'):10;default:11;}",
    ),
    (
        "case accepts full comma and in expression",
        "switch(true){case (0,'prototype' in Function):12;break;default:13;}",
    ),
    (
        "immediate break leaves reset completion",
        "1; switch(0){default:break;}",
    ),
    (
        "nested switch resets prior case completion",
        "switch(1){case 1:4;switch(0){case 1:5;}break;}",
    ),
    (
        "unlabeled break selects nearest switch",
        "(function(){var log='';while(true){log+='w';switch(1){case 1:log+='s';break;}log+='a';break;}return log;})()",
    ),
    (
        "labeled break crosses retained selector",
        "outer:{switch(1){case 1:2;break outer;default:3;}4;}7",
    ),
    (
        "labeled break crosses nested selectors",
        "outer:{switch(1){case 1:switch(2){case 2:break outer;}9;}8;}7",
    ),
    (
        "continue crosses switch into outer loop",
        "(function(){var i=0;var log='';outer:while(i++<2){switch(i){case 1:log+='a';continue outer;default:log+='b';break;}log+='c';}return i+'|'+log;})()",
    ),
    (
        "break to outer loop label crosses switch",
        "(function(){var i=0;outer:while(true){i++;switch(i){case 1:break outer;}}return i;})()",
    ),
    (
        "line break keeps break label unconsumed",
        "Function.switchAsi=0;outer:{switch(1){case 1:break\nouter;}Function.switchAsi=1;}Function.switchAsi",
    ),
    (
        "line break keeps continue label unconsumed",
        "(function(){var i=0;var value=0;outer:while(i++<1){switch(1){case 1:continue\nouter;}value=9;}return i+'|'+value;})()",
    ),
    (
        "unselected var declaration remains hoisted",
        "(function(){switch(0){case 1:var value=3;}return typeof value+'|'+value;})()",
    ),
    (
        "selected var declaration initializes",
        "(function(){switch(1){case 1:var value=3;}return value;})()",
    ),
    (
        "return abandons retained selector",
        "(function(){switch(1){case 1:return 4;default:return 5;}})()",
    ),
    (
        "function fallthrough abandons no temporary",
        "(function(){switch(1){case 1:2;break;}})()",
    ),
    (
        "switch works in dynamic Function body",
        "Function('x','switch(x){case 1:return 6;default:return 7;}')(1)",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("statement before first clause", "switch(0){1;}"),
    ("keyword before first clause", "switch(0){else:1;}"),
    ("duplicate default", "switch(0){default:1;default:2;}"),
    ("missing case expression", "switch(0){case:1;}"),
    (
        "case keyword cannot be a case expression",
        "switch(0){case case:;}",
    ),
    (
        "default keyword cannot be a case expression",
        "switch(0){case default:;}",
    ),
    ("missing case colon", "switch(0){case 0 1;}"),
    (
        "continue cannot target switch",
        "switch(0){case 0:continue;}",
    ),
    (
        "continue cannot target regular switch label",
        "label:switch(0){case 0:continue label;}",
    ),
    ("missing discriminant close paren", "switch(0 {case 0:1;}"),
    ("missing switch body brace", "switch(0) case 0:1;"),
    ("escaped case is not a clause", "switch(0){c\\u0061se 0:1;}"),
    ("case outside switch", "case 0:1;"),
    ("default outside switch", "default:1;"),
];

const THROW_CASES: &[(&str, &str)] = &[
    ("explicit body throw", "switch(1){case 1:throw 4;}"),
    (
        "selector throw",
        "switch((function(){throw 5;})()){case 1:1;}",
    ),
    (
        "case expression throw",
        "switch(0){case (function(){throw 6;})():1;}",
    ),
    (
        "nested switch throw",
        "switch(1){case 1:switch(2){case 2:throw Function;}}",
    ),
];

#[test]
fn switch_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP switch value differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
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
fn switch_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP switch diagnostic differential: set QJS_ORACLE to upstream qjs");
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
fn switch_abrupt_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP switch abrupt differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in THROW_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(source), Err(RuntimeError::Exception));
        let value = context
            .take_exception()
            .unwrap()
            .expect("Rust switch throw did not publish its value");
        assert_eq!(
            normalize_rust_value(&value),
            oracle_throw_observation(&oracle, source, description),
            "throw mismatch for {description:?} ({source:?})"
        );
    }
}

fn oracle_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!("var __qjo_value = std.evalScript(scriptArgs[0]);\n{ORACLE_NORMALIZER}");
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

fn oracle_throw_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = format!(
        "try {{ std.evalScript(scriptArgs[0]); throw 'switch did not throw'; }} catch (__qjo_value) {{ {ORACLE_NORMALIZER} }}"
    );
    let output = Command::new(oracle)
        .args(["--std", "-e", &script, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS throw for {description:?}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS throw probe failed for {description:?} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS throw output was not UTF-8")
        .trim_end()
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
