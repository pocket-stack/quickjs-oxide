use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for the pinned QuickJS 2026-06-04 treatment of U+180E
// MONGOLIAN VOWEL SEPARATOR. Unicode 17 classifies it as a format character,
// not ECMAScript whitespace: the lexer must reject it between tokens while
// literals and comments retain it as ordinary source content. The same choice
// flows through numeric parsing, trimming, Final_Sigma context, and RegExp
// dot/space classes. JavaScript eval and JSON are intentionally outside this
// slice because they remain independent runtime frontiers.

const PRELUDE: &str = r#"
function __units(value){
    value=String(value);
    var output="",index=0;
    while(index<value.length){
        var unit=value.charCodeAt(index).toString(16);
        while(unit.length<4)unit="0"+unit;
        if(index)output+=",";
        output+=unit;
        index++;
    }
    return output;
}
function __number(value){
    if(value!==value)return "NaN";
    if(value===0&&1/value===-Infinity)return "-0";
    return String(value);
}
"#;

const RAW_TOKEN_CASES: &[(&str, &str)] = &[("U+180E is not a token separator", "var\u{180e}foo;")];

const SOURCE_CONTENT_CASES: &[(&str, &str)] = &[(
    "comments accept U+180E while string template and RegExp literals preserve it",
    r#"(function(){
        var string="᠎",template=`᠎`,regexp=/᠎/,single=0,multi=0;
        //᠎ U+180E in a single-line comment
        single++;
        /*᠎ U+180E in a multi-line comment */
        multi++;
        return [
            __units(string),__units(template),__units(regexp.source),
            regexp.test(string),single,multi
        ].join("|");
    })()"#,
)];

const NUMBER_CASES: &[(&str, &str)] = &[(
    "Number rejects and prefix parsers stop at leading or trailing U+180E",
    r#"(function(){
        return [
            __number(Number("\u180e")),
            __number(Number("\u180e1")),
            __number(Number("1\u180e")),
            __number(parseInt("\u180e12",10)),
            __number(parseInt("12\u180e",10)),
            __number(parseFloat("\u180e1.5")),
            __number(parseFloat("1.5\u180e"))
        ].join("|");
    })()"#,
)];

const TRIM_CASES: &[(&str, &str)] = &[(
    "trim scans stop at U+180E independently from either side",
    r#"(function(){
        var both=" \t\u180e \tX\t \u180e\t ",
            leading=" \t\u180e \tX",
            trailing="X\t \u180e\t ";
        return [
            __units(both.trim()),
            __units(leading.trimStart()),__units(leading.trimEnd()),
            __units(trailing.trimStart()),__units(trailing.trimEnd()),
            __units("\u180e".trim())
        ].join("|");
    })()"#,
)];

const FINAL_SIGMA_CASES: &[(&str, &str)] = &[(
    "U+180E is Case_Ignorable for ordinary and locale-named lowercase",
    r#"(function(){
        var values=[
            "A\u180e\u03a3","A\u180e\u03a3B",
            "A\u03a3\u180e","A\u03a3\u180eB",
            "A\u180e\u03a3\u180e","A\u180e\u03a3\u180eB"
        ],output=[],index=0;
        while(index<values.length){
            output.push(__units(values[index].toLowerCase()));
            output.push(__units(values[index].toLocaleLowerCase()));
            index++;
        }
        return output.join("|");
    })()"#,
)];

const REGEXP_CASES: &[(&str, &str)] = &[(
    "dot and non-whitespace match U+180E under legacy Unicode and dotAll modes",
    r#"(function(){
        var flags=["","u","s","su"],output=[],index=0,value="\u180e";
        while(index<flags.length){
            var flag=flags[index++],dot=new RegExp("^.$",flag),
                space=new RegExp("^\\s$",flag),nonspace=new RegExp("^\\S$",flag);
            output.push([
                flag||"empty",dot.test(value),space.test(value),nonspace.test(value),
                dot.dotAll,dot.unicode
            ].join(":"));
        }
        return output.join("|");
    })()"#,
)];

#[test]
fn unicode_u180e_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP U+180E oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for (group, cases) in case_groups() {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            if group == "raw token" {
                assert!(
                    observation.starts_with("throw|object|SyntaxError|"),
                    "{group} oracle vector did not throw SyntaxError for {description}: {observation:?}",
                );
            } else {
                assert!(
                    observation.starts_with("return|"),
                    "{group} oracle vector unexpectedly threw for {description}: {observation:?}",
                );
            }
        }
    }
}

#[test]
fn unicode_u180e_raw_token_diagnostic_matches_pinned_quickjs() {
    compare_cases("U+180E raw token", RAW_TOKEN_CASES);
}

#[test]
fn unicode_u180e_source_content_matches_pinned_quickjs() {
    compare_cases("U+180E source content", SOURCE_CONTENT_CASES);
}

#[test]
fn unicode_u180e_number_parsing_matches_pinned_quickjs() {
    compare_cases("U+180E number parsing", NUMBER_CASES);
}

#[test]
fn unicode_u180e_trim_boundaries_match_pinned_quickjs() {
    compare_cases("U+180E trim boundaries", TRIM_CASES);
}

#[test]
fn unicode_u180e_final_sigma_context_matches_pinned_quickjs() {
    compare_cases("U+180E Final_Sigma context", FINAL_SIGMA_CASES);
}

#[test]
fn unicode_u180e_regexp_classes_match_pinned_quickjs() {
    compare_cases("U+180E RegExp classes", REGEXP_CASES);
}

fn case_groups() -> [(&'static str, &'static [(&'static str, &'static str)]); 6] {
    [
        ("raw token", RAW_TOKEN_CASES),
        ("source content", SOURCE_CONTENT_CASES),
        ("number parsing", NUMBER_CASES),
        ("trim", TRIM_CASES),
        ("Final_Sigma", FINAL_SIGMA_CASES),
        ("RegExp", REGEXP_CASES),
    ]
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
    object: &ObjectRef,
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
