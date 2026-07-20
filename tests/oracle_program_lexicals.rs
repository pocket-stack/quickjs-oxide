use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "basic Program lexical environment",
        "let local=1,named=function(){};const fixed=2;local+=2;typeof local+'|'+local+'|'+fixed+'|'+named.name+'|'+typeof globalThis.local+'|'+delete local+'|'+delete fixed",
    ),
    (
        "strict Program lexicals remain global lexicals",
        "'use strict';let local=1;const fixed=2;local+'|'+fixed+'|'+typeof globalThis.local",
    ),
    (
        "Program lexical capture and mutation",
        "let value=1,read=function(){return value},write=function(next){value=next};write(7);read()+'|'+value",
    ),
    (
        "nested block shadows a Program lexical",
        "let value=1;{let value=2;Function.blockValue=value;}value+'|'+Function.blockValue",
    ),
    (
        "dynamic Function resolves an initialized Program lexical",
        "let value=5;Function('return value')()",
    ),
    (
        "let without initializer becomes undefined",
        "let value;typeof value+'|'+(value===undefined)",
    ),
    (
        "Program lexical flat array bindings stay off the global object",
        "let [first=1,...rest]=[];const [third]=[3];first+'|'+rest.length+'|'+third+'|'+typeof globalThis.first+'|'+typeof globalThis.rest+'|'+typeof globalThis.third",
    ),
    (
        "Program lexical nested array bindings stay off the global object",
        "let [[first]=[40],...[second,third]]=[undefined,1,2];const [[fourth]]=[[2]];first+second+third+fourth+'|'+typeof globalThis.first",
    ),
    (
        "declaration preserves an earlier script completion",
        "9;let value=1",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "read before Program lexical initialization",
        "value;\nlet value=1;",
    ),
    (
        "typeof before Program lexical initialization",
        "typeof value;\nlet value=1;",
    ),
    ("Program const assignment", "const value=1;\nvalue=2;"),
    (
        "fixed global property blocks a lexical declaration",
        "let NaN=1;",
    ),
    (
        "declaration preflight keeps source-order conflict priority",
        "let first=function(){return Infinity},NaN=1,Infinity=2;",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    ("duplicate Program lexical", "let value=1;\nlet value=2;"),
    ("Program const requires initializer", "const value;"),
    ("strict Program eval binding", "'use strict';\nlet eval=1;"),
    (
        "strict Program arguments binding",
        "'use strict';\nconst arguments=1;",
    ),
    ("Program let cannot bind itself", "let let=1;"),
];

struct BoundaryCase {
    description: &'static str,
    source: &'static str,
    rust_message: &'static str,
}

const BOUNDARY_CASES: &[BoundaryCase] = &[BoundaryCase {
    description: "Program lexical nested object destructuring",
    source: "let [{value}]=[{value:1}];value",
    rust_message: "object destructuring bindings are not implemented yet",
}];

#[test]
fn program_lexical_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program lexical differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = observe_rust_eval(&runtime, &mut context, source, description);
        let quickjs = observe_oracle_sequence(&oracle, &[source], description);
        assert_eq!(rust, quickjs, "Program lexical drifted for {description}");
    }
}

#[test]
fn program_lexical_cross_eval_state_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program lexical state differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let sequences: &[(&str, &[&str])] = &[
        (
            "persistent mutable and const bindings",
            &[
                "let crossValue=40;const crossFixed=2;Function.saved=function(){return ++crossValue+crossFixed};undefined",
                "Function.saved()+'|'+Function.saved()+'|'+crossValue+'|'+crossFixed+'|'+typeof globalThis.crossValue",
                "crossFixed=3",
                "let crossValue=9",
            ],
        ),
        (
            "existing global property splits from the lexical cell",
            &[
                "globalThis.splitValue=10;Function.splitRead=function(){return splitValue};undefined",
                "let splitValue=1",
                "Function.splitRead()+'|'+splitValue+'|'+globalThis.splitValue",
                "delete globalThis.splitValue",
                "Function.splitRead()+'|'+splitValue+'|'+typeof globalThis.splitValue",
            ],
        ),
        (
            "hidden unresolved VarRef becomes the Program lexical cell",
            &[
                "Function.hiddenRead=function(){return hiddenValue};Function.hiddenWrite=function(next){hiddenValue=next};undefined",
                "let hiddenValue=1",
                "Function.hiddenWrite(2);hiddenValue+'|'+Function.hiddenRead()+'|'+typeof globalThis.hiddenValue",
            ],
        ),
        (
            "auto-init global property stays separate from the Program lexical cell",
            &[
                "let Number=1",
                "Number+'|'+typeof globalThis.Number",
                "Number=2;Number",
                "delete Number",
            ],
        ),
        (
            "failed initializer keeps the pinned poisoned lexical",
            &[
                "Function.failedRead=function(){return failedValue};let failedValue=(function(){throw 17})()",
                "Function.failedRead()",
                "failedValue",
                "typeof failedValue",
                "delete failedValue",
                "let failedValue=1",
            ],
        ),
        (
            "poisoned lexical distinguishes typed and precompiled global descriptors",
            &[
                "Function.oldPoisonedRead=function(){return poisonedValue};Function.oldPoisonedWrite=function(next){poisonedValue=next};undefined",
                "Function.declaredPoisonedRead=function(){return poisonedValue};let poisonedValue=(function(){throw 17})()",
                "Function.declaredPoisonedRead()",
                "Function.oldPoisonedRead()",
                "Function.oldPoisonedWrite(1)",
                "poisonedValue",
                "typeof poisonedValue",
                "delete poisonedValue",
                "let poisonedValue=2",
            ],
        ),
        (
            "partial initialization preserves initialized and poisoned cells",
            &[
                "Function.readPartialFirst=function(){return partialFirst};Function.readPartialSecond=function(){return partialSecond};let partialFirst=1,partialSecond=(function(){throw 19})()",
                "Function.readPartialFirst()+'|'+partialFirst",
                "Function.readPartialSecond()",
                "partialSecond",
                "typeof partialSecond",
                "let partialFirst=2",
                "let partialSecond=2",
            ],
        ),
        (
            "parse failure creates no Program binding",
            &[
                "let parseOnly=1;let parseOnly=2",
                "typeof parseOnly",
                "let parseOnly=3;parseOnly",
            ],
        ),
        (
            "preflight conflict is atomic and source ordered",
            &[
                "let untouched=function(){return Infinity},NaN=1,Infinity=2",
                "typeof untouched",
                "let untouched=4;untouched",
            ],
        ),
        (
            "parse and preflight failures prevent authored side effects",
            &[
                "globalThis.atomicMarker=0",
                "atomicMarker=1;let parseResidue=1;let parseResidue=2",
                "atomicMarker+'|'+typeof parseResidue",
                "atomicMarker=2;let preflightResidue=(atomicMarker=3),NaN=1",
                "atomicMarker+'|'+typeof preflightResidue",
                "let parseResidue=4,preflightResidue=5;parseResidue+'|'+preflightResidue+'|'+atomicMarker",
            ],
        ),
    ];

    for &(description, sources) in sequences {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let rust = sources
            .iter()
            .map(|source| observe_rust_eval(&runtime, &mut context, source, description))
            .collect::<Vec<_>>()
            .join("\n");
        let quickjs = observe_oracle_sequence(&oracle, sources, description);
        assert_eq!(
            rust, quickjs,
            "Program lexical state drifted for {description}"
        );
    }
}

#[test]
fn program_lexical_full_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program lexical stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn program_lexical_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program lexical parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn selected_unsupported_program_declaration_boundaries_stay_explicit() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Program declaration boundaries: set QJS_ORACLE to upstream qjs");
        return;
    };

    for case in BOUNDARY_CASES {
        let quickjs = run_cli(&oracle, &[], case.source, case.description);
        assert!(
            quickjs.status.success(),
            "pinned QuickJS rejected {}: {}",
            case.description,
            String::from_utf8_lossy(&quickjs.stderr),
        );

        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval(case.source),
            Err(RuntimeError::Exception),
            "Rust unexpectedly accepted {}",
            case.description,
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!(
                "Rust boundary did not throw an Error for {}",
                case.description
            );
        };
        assert_eq!(
            error_string_property(&runtime, &mut context, &error, "name", case.description),
            "SyntaxError"
        );
        assert_eq!(
            error_string_property(&runtime, &mut context, &error, "message", case.description),
            case.rust_message
        );
    }
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!("return|{}", normalize_value(value)),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
                ),
                value => format!("throw|{}", normalize_value(value)),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn normalize_value(value: Value) -> String {
    match value {
        Value::Undefined => "undefined|undefined".to_owned(),
        Value::Null => "object|null".to_owned(),
        Value::Bool(value) => format!("boolean|{value}"),
        Value::Int(value) => format!("number|{value}"),
        Value::Float(value) => format!("number|{value}"),
        Value::BigInt(value) => format!("bigint|{value}"),
        Value::String(value) => format!("string|{}", value.to_utf8_lossy()),
        Value::Object(_) => "object|<object>".to_owned(),
        Value::Symbol(_) => "symbol|<symbol>".to_owned(),
    }
}

fn observe_oracle_sequence(oracle: &OsStr, sources: &[&str], description: &str) -> String {
    let wrapper = r#"
(function () {
for (var index = 0; index < scriptArgs.length; index++) {
  try {
    var value = std.evalScript(scriptArgs[index]);
    print('return|' + typeof value + '|' + String(value));
  } catch (error) {
    if (error !== null && typeof error === 'object')
      print('throw|object|' + error.name + '|' + error.message);
    else
      print('throw|' + typeof error + '|' + String(error));
  }
}
})();
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper])
        .args(sources)
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS sequence failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn compare_cli(oracle: &OsStr, options: &[&str], source: &str, description: &str) {
    let rust = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        options,
        source,
        description,
    );
    let quickjs = run_cli(oracle, options, source, description);
    assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(rust.stdout, quickjs.stdout, "{description}");
    assert_eq!(rust.stderr, quickjs.stderr, "{description}");
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
    };
    value.to_utf8_lossy()
}

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}
