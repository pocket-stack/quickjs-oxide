use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 scoped RegExp modifiers.
// The parser and state restoration behavior lives in `libregexp.c`
// `re_parse_modifiers` / `re_parse_term` (1810-1950). These vectors avoid
// unrelated unsupported RegExp features so the modifier slice can close
// independently while still probing literal, constructor, Unicode, capture,
// quantifier, and stateful execution integration.

const PRELUDE: &str = r#"
function __completion(callback){
    try{return "return:"+String(callback())}
    catch(error){
        if(error!==null&&typeof error==="object")
            return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
"#;

const GRAMMAR_CASES: &[(&str, &str)] = &[
    (
        "literal duplicate add modifier reports the modifier error",
        r#"/(?ii:a)/"#,
    ),
    (
        "constructor duplicate remove modifier reports the modifier error",
        r#"(function(){
            return __completion(function(){return new RegExp("(?-ss:a)")});
        })()"#,
    ),
    (
        "add and remove overlap is invalid after both lists are parsed",
        r#"(function(){
            return __completion(function(){return new RegExp("(?im-ms:a)")});
        })()"#,
    ),
    (
        "empty add and remove lists are invalid",
        r#"(function(){
            return __completion(function(){return new RegExp("(?-:a)")});
        })()"#,
    ),
    (
        "missing modifier colon wins before parsing the body",
        r#"(function(){
            return __completion(function(){return new RegExp("(?i=a)")});
        })()"#,
    ),
    (
        "only i m and s enter the scoped modifier grammar",
        r#"(function(){
            return __completion(function(){return new RegExp("(?g:a)")});
        })()"#,
    ),
    (
        "duplicate add modifier wins before add remove overlap",
        r#"(function(){
            return __completion(function(){return new RegExp("(?iim-i:a)")});
        })()"#,
    ),
    (
        "unterminated modifier group reports the closing delimiter",
        r#"(function(){
            return __completion(function(){return new RegExp("(?i:a")});
        })()"#,
    ),
];

const SCOPED_MATCHING_CASES: &[(&str, &str)] = &[
    (
        "nested add remove and re-add restores each enclosing modifier state",
        r#"(function(){
            var regexp=/(?i:a(?-i:b(?i:c))d)e/;
            return [regexp.test("AbCDe"),regexp.test("ABCDe"),
                regexp.test("AbCdE")].join("|");
        })()"#,
    ),
    (
        "ignoreCase scopes over literals and classes then restores outside",
        r#"(function(){
            var regexp=/(?i:a[A-Z])(?-i:b)/;
            return [regexp.test("aZb"),regexp.test("Azb"),
                regexp.test("AZB")].join("|");
        })()"#,
    ),
    (
        "ignoreCase changes Unicode word boundary classification within scope",
        r#"(function(){
            var add=/(?i:\b\u212a\b)/u,
                remove=/(?-i:\b\u212a\b)/iu;
            return [add.test("\u212a"),remove.test("\u212a"),
                add.flags,remove.flags].join("|");
        })()"#,
    ),
    (
        "multiline add and remove select scoped anchor semantics",
        r#"(function(){
            var add=/(?m:^b$)/,
                remove=/(?-m:^b$)/m,
                input="a\nb\nc";
            return [add.test(input),remove.test(input),
                add.multiline,remove.multiline].join("|");
        })()"#,
    ),
    (
        "dotAll add and remove select scoped dot semantics",
        r#"(function(){
            var add=/(?s:a.b)/,
                remove=/(?-s:a.b)/s,
                input="a\nb";
            return [add.test(input),remove.test(input),
                add.dotAll,remove.dotAll].join("|");
        })()"#,
    ),
];

const CONSTRUCTION_AND_FRONTIER_CASES: &[(&str, &str)] = &[
    (
        "scoped modifiers do not change global flags or global exec state",
        r#"(function(){
            var regexp=/(?i:a)/g,
                first=regexp.exec("AaA"),afterFirst=regexp.lastIndex,
                second=regexp.exec("AaA"),afterSecond=regexp.lastIndex,
                third=regexp.exec("AaA"),afterThird=regexp.lastIndex,
                exhausted=regexp.exec("AaA");
            return [regexp.source,regexp.flags,regexp.global,regexp.ignoreCase,
                first[0],first.index,afterFirst,second[0],second.index,afterSecond,
                third[0],third.index,afterThird,exhausted,regexp.lastIndex].join("|");
        })()"#,
    ),
    (
        "literal and constructor paths preserve equivalent source and flags",
        r#"(function(){
            var source="(?im-s:^a.$)",
                literal=/(?im-s:^a.$)/s,
                constructed=new RegExp(source,"s"),
                input="x\nA!\ny";
            return [literal.source===constructed.source,literal.flags,
                constructed.flags,literal.test(input),constructed.test(input),
                literal.ignoreCase,literal.multiline,literal.dotAll].join("|");
        })()"#,
    ),
    (
        "scoped dot keeps legacy code-unit and Unicode code-point width",
        r#"(function(){
            var legacy=/(?s:(.))/.exec("\ud83d\ude00"),
                unicode=/(?s:(.))/u.exec("\ud83d\ude00");
            return [legacy[0].length,legacy[1].length,
                legacy[1].charCodeAt(0).toString(16),
                unicode[0].length,unicode[1].length,
                unicode[1].codePointAt(0).toString(16)].join("|");
        })()"#,
    ),
    (
        "scoped ignoreCase folds astral Unicode without changing outer flags",
        r#"(function(){
            var add=/(?i:\u{10400})/u,
                remove=/(?-i:\u{10400})/iu,
                lower="\u{10428}";
            return [add.test(lower),remove.test(lower),add.unicode,
                add.ignoreCase,remove.ignoreCase].join("|");
        })()"#,
    ),
    (
        "modifier groups remain noncapturing under alternation and quantification",
        r#"(function(){
            var regexp=/(?i:(a|b)+)(c)/,
                match=regexp.exec("ABc");
            return [match[0],match[1],match[2],match.length,
                regexp.test("ABC"),regexp.source,regexp.flags].join("|");
        })()"#,
    ),
];

#[test]
fn regexp_modifiers_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp modifiers oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("grammar", GRAMMAR_CASES),
        ("scoped matching", SCOPED_MATCHING_CASES),
        ("construction and frontier", CONSTRUCTION_AND_FRONTIER_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
}

#[test]
fn regexp_modifier_grammar_and_error_priority_match_pinned_quickjs() {
    compare_cases("RegExp modifier grammar", GRAMMAR_CASES);
}

#[test]
fn regexp_modifier_scoped_matching_matches_pinned_quickjs() {
    compare_cases("RegExp modifier scoped matching", SCOPED_MATCHING_CASES);
}

#[test]
fn regexp_modifier_construction_and_frontier_match_pinned_quickjs() {
    compare_cases(
        "RegExp modifier construction and frontier",
        CONSTRUCTION_AND_FRONTIER_CASES,
    );
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_rust_eval(&runtime, &mut context, source, description);
        let expected = observe_oracle(&oracle, source, description);
        if actual != expected {
            failures.push(format!(
                "{description}\nsource: {source:?}\noxide: {actual:?}\noracle: {expected:?}",
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

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    let source = observed_source(source);
    match context.eval(&source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value),
        ),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take Rust exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("Rust exception was missing for {description}"));
            match exception {
                Value::Object(error) => format!(
                    "throw|object|{}|{}",
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
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

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let source = observed_source(source);
    let wrapper = r#"
try {
  var value=std.evalScript(scriptArgs[0]);
  print('return|'+typeof value+'|'+String(value));
} catch(error) {
  if(error!==null&&typeof error==='object')print('throw|object|'+error.name+'|'+error.message);
  else print('throw|'+typeof error+'|'+String(error));
}
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, &source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
        .trim_end()
        .to_owned()
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &quickjs_oxide::ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read string property {name}: {error}"))
    else {
        panic!("{name} was not a string");
    };
    value.to_utf8_lossy()
}

fn value_type(runtime: &Runtime, value: &Value) -> &'static str {
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

fn primitive_value_text(value: Value) -> String {
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
