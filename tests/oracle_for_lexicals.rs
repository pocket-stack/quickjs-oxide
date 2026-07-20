use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "simple lexical head and outer shadow",
        "(function(){let value=9,sum=0;for(let value=0;value<4;value++)sum+=value;return value+'|'+sum;})()",
    ),
    (
        "head declarations initialize left to right",
        "(function(){let result='';for(let left=1,right=left+1;left<2;left++,right++)result=left+'|'+right;return result;})()",
    ),
    (
        "head and body lexicals shadow parameters while of remains a name",
        "(function(value){let result='';for(let of=0,value=1;of<1;of++){let value=2;result=of+'|'+value;}return result+'|'+value;})(9)",
    ),
    (
        "let without initializer becomes undefined",
        "(function(){for(let value;;){return typeof value+'|'+value;}})()",
    ),
    (
        "head anonymous function receives its binding name",
        "(function(){let observed;for(let named=function(){};;){observed=named.name;break;}return observed;})()",
    ),
    (
        "normal fallthrough creates per-iteration capture cells",
        "(function(){let first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};}return first()+'|'+second()+'|'+third();})()",
    ),
    (
        "mutating an earlier iteration does not change the next",
        "(function(){let firstRead,firstWrite,secondRead;for(let value=0;value<2;value++){if(value===0){firstRead=function(){return value};firstWrite=function(next){value=next};}else secondRead=function(){return value};}firstWrite(9);return firstRead()+'|'+secondRead();})()",
    ),
    (
        "initializer and first body capture different cells",
        "(function(){let initialRead,initialWrite,bodyRead;for(let value=(initialRead=function(){return value},initialWrite=function(next){value=next},0);value<1;value++){bodyRead=function(){return value};}initialWrite(9);return initialRead()+'|'+bodyRead();})()",
    ),
    (
        "update and following body capture the same cell",
        "(function(){let updateWrite,bodyRead;for(let value=0;value<2;(updateWrite=updateWrite||function(next){value=next},value++)){if(value===1)bodyRead=function(){return value};}updateWrite(7);return bodyRead();})()",
    ),
    (
        "unconditional continue preserves the pinned shared-head-cell quirk",
        "(function(){let first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};continue;}return first()+'|'+second()+'|'+third();})()",
    ),
    (
        "one continue shares only the crossed head cell",
        "(function(){let first,second,third;for(let value=0;value<3;value++){if(value===0)first=function(){return value};else if(value===1)second=function(){return value};else third=function(){return value};if(value===0)continue;}return first()+'|'+second()+'|'+third();})()",
    ),
    (
        "normal fallthrough closes a head cell when update is absent",
        "(function(){let first,second;for(let value=0;value<2;){if(value===0)first=function(){return value};else second=function(){return value};value++;}return first()+'|'+second();})()",
    ),
    (
        "continue shares the head cell when update is absent",
        "(function(){let first,second;for(let value=0;value<2;){if(value===0)first=function(){return value};else second=function(){return value};value++;continue;}return first()+'|'+second();})()",
    ),
    (
        "an absent test keeps the unmoved update cell boundary",
        "(function(){let first,second;for(let value=0;;value++){if(value===0)first=function(){return value};else {second=function(){return value};break;}}return first()+'|'+second();})()",
    ),
    (
        "labeled continue closes switch cells but not the head cell",
        "(function(){let first,second;outer:for(let value=0;value<2;value++){switch(0){case 0:const inner=value;const read=function(){return inner+'|'+value};if(value===0){first=read;continue outer;}second=read;break;}}return first()+';'+second();})()",
    ),
    (
        "labeled break preserves escaped switch and head values",
        "(function(){let read;outer:for(let value=0;value<3;value++){switch(0){case 0:const inner=value;read=function(){return inner+'|'+value};break outer;}}return read();})()",
    ),
    (
        "labeled continue exits an inner lexical for",
        "(function(){let outerFirst,outerSecond,innerRead;outer:for(let outer=0;outer<2;outer++){for(let inner=0;inner<1;inner++){innerRead=function(){return inner};if(outer===0){outerFirst=function(){return outer};continue outer;}outerSecond=function(){return outer};}}return outerFirst()+'|'+outerSecond()+'|'+innerRead();})()",
    ),
    (
        "Function constructor lexical for cells",
        "Function('let first,second;for(let value=0;value<2;value++){if(value===0)first=function(){return value};else second=function(){return value};}return first()+\"|\"+second();')()",
    ),
    (
        "contextual bare let remains an expression",
        "(function(){var let=1;for(let;;){break;}return let;})()",
    ),
    (
        "first var declaration scope quirk remains accepted",
        "(function(){{var value;}{for(var value;false;);let value;}return value===undefined;})()",
    ),
    (
        "parenthesized in is accepted in a lexical initializer",
        "(function(){for(let value=('name' in Function);false;){}return 1;})()",
    ),
    (
        "function body inside a NoIn initializer restores ordinary grammar",
        "(function(){var read;for(let initializer=(read=function(){let value='name' in Function;return value});false;){}return read();})()",
    ),
    (
        "classic for nested array binding supports a pattern default",
        "(function(){let result;for(let [[value]=[42]]=[];(result=value,false);){}return result;})()",
    ),
];

const SCRIPT_VALUE_CASES: &[(&str, &str, &str)] = &[
    (
        "script lexical for stays local",
        "Function.scopeLog='';for(let value=0;value<3;value++)Function.scopeLog+=value;",
        "Function.scopeLog+'|'+typeof value",
    ),
    (
        "script lexical for capture survives loop scope exit",
        "Function.saved=undefined;for(let value=0;value<1;value++){Function.saved=function(){return value};}",
        "Function.saved()+'|'+typeof value",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "head self initializer is in the temporal dead zone",
        "(function forHeadTdz(){\n  for(let value=value;false;){}\n})()",
    ),
    (
        "later declarator is in the temporal dead zone",
        "(function forLaterTdz(){\n  for(let first=second,second=1;false;){}\n})()",
    ),
    (
        "initializer closure observes a later declarator TDZ",
        "(function forClosureTdz(){\n  for(let read=function forRead(){return value},value=read();false;){}\n})()",
    ),
    (
        "relocated const update is readonly",
        "(function forConstUpdate(){\n  for(const value=0;value<1;value=1){}\n})()",
    ),
    (
        "captured const remains readonly in the relocated update",
        "(function forCapturedConstUpdate(){\n  let read;\n  for(const value=0;value<1;value=1){\n    read=function(){return value};\n  }\n})()",
    ),
    (
        "conditional update true branch keeps its readonly site",
        "(function forConstConditionalUpdate(flag){\n  for(const value=0;value<1;flag?value=1:value=2){}\n})(true)",
    ),
    (
        "conditional update false branch keeps its readonly site",
        "(function forConstConditionalUpdate(flag){\n  for(const value=0;value<1;flag?value=1:value=2){}\n})(false)",
    ),
    (
        "continue reaches the relocated readonly update",
        "(function forConstContinue(){\n  for(const value=0;value<1;value=1){\n    continue;\n  }\n})()",
    ),
    (
        "const body write is readonly",
        "(function forConstBody(){\n  for(const value=0;;){\n    value=1;\n  }\n})()",
    ),
    (
        "captured const writer stays readonly after break",
        "(function forConstWriterOuter(){let write;for(const value=0;value<1;){write=function forConstWriterInner(){value=1};break;}write();})()",
    ),
    (
        "Function constructor head TDZ",
        "Function('for(let value=value;false;){}')()",
    ),
    (
        "first var scope quirk still resolves an initializer through the later lexical",
        "(function forVarScopeTdz(){\n  {var value;}\n  {for(var value=0;false;);let value;}\n})()",
    ),
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "duplicate head lexical",
        "(function(){\n  for(let value=0,value=1;false;){}\n})",
    ),
    (
        "const head requires initializer",
        "(function(){\n  for(const value;false;){}\n})",
    ),
    (
        "descendant body var conflicts with head lexical",
        "(function(){\n  for(let value=0;value<1;value++){var value;}\n})",
    ),
    (
        "nested switch var conflicts with head lexical",
        "(function(){\n  for(let value=0;value<1;value++){switch(0){case 0:var value;}}\n})",
    ),
    (
        "strict eval head binding",
        "(function(){\n  'use strict';\n  for(let eval=0;false;){}\n})",
    ),
    (
        "strict arguments head binding",
        "(function(){\n  'use strict';\n  for(let arguments=0;false;){}\n})",
    ),
    (
        "let cannot bind itself in a head",
        "(function(){\n  for(let let=0;false;){}\n})",
    ),
    (
        "unparenthesized in stops a lexical NoIn initializer",
        "(function(){\n  for(let value='name' in Function;false;){}\n})",
    ),
];

struct BoundaryCase {
    description: &'static str,
    source: &'static str,
    rust_message: &'static str,
}

const BOUNDARY_CASES: &[BoundaryCase] = &[BoundaryCase {
    description: "classic for lexical nested object destructuring",
    source: "(function(){for(let [{value}]=[{value:1}];false;){}return 1;})()",
    rust_message: "object destructuring bindings are not implemented yet",
}];

#[test]
fn classic_for_lexical_values_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical-for differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in VALUE_CASES {
        assert_eq!(
            rust_value_observation(source, description),
            oracle_value_observation(&oracle, source, description),
            "lexical-for value drifted for {description}: {source:?}",
        );
    }
    for &(description, setup, observation) in SCRIPT_VALUE_CASES {
        let rust_source = format!("{setup}\n({observation})");
        assert_eq!(
            rust_value_observation(&rust_source, description),
            oracle_script_value_observation(&oracle, setup, observation, description),
            "script lexical-for value drifted for {description}: {rust_source:?}",
        );
    }
}

#[test]
fn script_for_capture_survives_a_following_eval_like_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical-for cross-eval differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let setup = "Function.saved=undefined;for(let value=0;value<1;value++){Function.saved=function(){return value};}";
    let observation = "Function.saved()+'|'+typeof value";

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(setup)
        .unwrap_or_else(|error| panic!("Rust rejected lexical-for setup: {error}"));
    let value = context
        .eval(observation)
        .unwrap_or_else(|error| panic!("Rust rejected lexical-for observation: {error}"));
    assert_eq!(
        normalize_rust_value(value),
        oracle_two_eval_value_observation(&oracle, setup, observation),
    );
}

#[test]
fn classic_for_lexical_error_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical-for Error differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn classic_for_lexical_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical-for StripDebug differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in ERROR_CASES {
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn classic_for_lexical_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical-for parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in SYNTAX_ERROR_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn nonclassic_and_destructuring_for_boundaries_remain_explicit() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP lexical-for boundary differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for case in BOUNDARY_CASES {
        let output = run_cli(&oracle, &[], case.source, case.description);
        assert!(
            output.status.success(),
            "pinned QuickJS rejected {} ({:?}): {}",
            case.description,
            case.source,
            String::from_utf8_lossy(&output.stderr),
        );
        assert_eq!(
            rust_error_observation(case.source, case.description),
            format!("SyntaxError|{}", case.rust_message),
            "Rust boundary drifted for {} ({:?})",
            case.description,
            case.source,
        );
    }
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

fn rust_value_observation(source: &str, description: &str) -> String {
    let runtime = Runtime::new();
    let value = runtime
        .new_context()
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"));
    normalize_rust_value(value)
}

fn normalize_rust_value(value: Value) -> String {
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

fn oracle_value_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let script =
        format!("var __qjo_value=({source});print(typeof __qjo_value+'|'+String(__qjo_value));");
    let output = Command::new(oracle)
        .args(["-e", &script])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "pinned QuickJS rejected {description} ({source:?}): {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!("QuickJS emitted non-UTF-8 output for {description}: {error}")
        })
        .trim_end()
        .to_owned()
}

fn oracle_script_value_observation(
    oracle: &OsStr,
    setup: &str,
    observation: &str,
    description: &str,
) -> String {
    let script = format!(
        "{setup}\nvar __qjo_value=({observation});print(typeof __qjo_value+'|'+String(__qjo_value));"
    );
    let output = Command::new(oracle)
        .args(["-e", &script])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "pinned QuickJS rejected {description} ({script:?}): {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS script value output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn oracle_two_eval_value_observation(oracle: &OsStr, setup: &str, observation: &str) -> String {
    let script = "std.evalScript(scriptArgs[0]);var __qjo_value=std.evalScript(scriptArgs[1]);print(typeof __qjo_value+'|'+String(__qjo_value));";
    let output = Command::new(oracle)
        .args(["--std", "-e", script, setup, observation])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS cross-eval probe: {error}"));
    assert!(
        output.status.success(),
        "pinned QuickJS rejected cross-eval probe: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS cross-eval output was not UTF-8")
        .trim_end()
        .to_owned()
}

fn rust_error_observation(source: &str, description: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval_with_filename(source, "<cmdline>"),
        Err(RuntimeError::Exception),
        "Rust unexpectedly accepted {description}: {source:?}",
    );
    let Value::Object(error) = context
        .take_exception()
        .expect("take Rust exception")
        .expect("Rust exception is present")
    else {
        panic!("Rust did not materialize an Error object for {description}");
    };
    let name = error_string_property(&runtime, &mut context, &error, "name", description);
    let message = error_string_property(&runtime, &mut context, &error, "message", description);
    format!("{name}|{message}")
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
