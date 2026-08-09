use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04 named RegExp captures.
//
// The vectors deliberately cover the QuickJS implementation, including its
// observable parser quirks: named-capture scopes are a wrapping global u8
// advanced by every alternative, and the lexical forward-reference scan skips
// the first byte in a named capture body.  Runtime cases pin the capture-name
// trailer consumers (`groups`, `indices.groups`, and replacement) rather than
// testing only the matcher in isolation.

const PRELUDE: &str = r#"
function __repeat(value,count){
    var result="";
    while(count-->0)result+=value;
    return result;
}
function __descriptor(value,name){
    var descriptor=Object.getOwnPropertyDescriptor(value,name);
    return [descriptor.writable,descriptor.enumerable,descriptor.configurable].join(",");
}
"#;

const GRAMMAR_AND_NAME_CASES: &[(&str, &str)] = &[
    (
        "constructor accepts a named capture without Unicode mode",
        r#"(function(){var r=new RegExp("(?<x>a)");return r.source})()"#,
    ),
    (
        "an empty capture name reports invalid group name",
        r#"new RegExp("(?<>a)")"#,
    ),
    (
        "a decimal capture-name start reports invalid group name",
        r#"new RegExp("(?<1>a)")"#,
    ),
    (
        "only Unicode escapes are accepted inside a capture name",
        r#"new RegExp("(?<\\x61>a)")"#,
    ),
    (
        "an escaped lone surrogate reports invalid group name",
        r#"new RegExp("(?<\\uD800>a)")"#,
    ),
    (
        "an undefined named backreference reports group name not defined",
        r#"new RegExp("(?<a>a)\\k<b>")"#,
    ),
    (
        "a bare named-backreference marker reports expecting group name",
        r#"new RegExp("(?<a>a)\\k")"#,
    ),
    (
        "a malformed named backreference reports invalid group name",
        r#"new RegExp("(?<a>a)\\k<>")"#,
    ),
    (
        "raw and escaped spellings collide after name decoding",
        r#"new RegExp("(?<\\u0061>a)(?<a>b)")"#,
    ),
    (
        "duplicate names in one alternative are rejected",
        r#"new RegExp("(?<x>a)(?<x>b)")"#,
    ),
    (
        "duplicate name wins over the capture-count limit",
        r#"(function(){var p="(?<x>a)"+__repeat("()",253)+"(?<x>b)";return new RegExp(p)})()"#,
    ),
    (
        "invalid name wins over the capture-count limit",
        r#"(function(){var p=__repeat("()",254)+"(?<1>a)";return new RegExp(p)})()"#,
    ),
    (
        "QuickJS accepts exactly 254 user capture slots",
        r#"(function(){var r=new RegExp(__repeat("()",254));return r.source.length>0})()"#,
    ),
    (
        "the 255th user capture reports too many captures",
        r#"new RegExp(__repeat("()",255))"#,
    ),
    (
        "a 122-byte ASCII group name fits the fixed parser buffer",
        r#"(function(){var n=__repeat("a",122),r=new RegExp("(?<"+n+">x)");return r.exec("x").groups[n]})()"#,
    ),
    (
        "a 123-byte ASCII group name exceeds the fixed parser buffer",
        r#"new RegExp("(?<"+__repeat("a",123)+">x)")"#,
    ),
    (
        "invalid flags are diagnosed before invalid group grammar",
        r#"new RegExp("(?<>a)","uu")"#,
    ),
    (
        "regexp literals report the same duplicate-name diagnostic",
        r#"/(?<x>a)(?<x>b)/"#,
    ),
    (
        "the wrapping alternative scope collides after 256 bars",
        r#"(function(){var p="(?<x>a)"+__repeat("|z",255)+"|(?<x>b)";return new RegExp(p)})()"#,
    ),
    (
        "the forward-name scan skips a nested group at body byte zero",
        r#"new RegExp("\\k<y>(?<x>(?<y>a))","u")"#,
    ),
    (
        "a multibyte final name scalar still obeys the pre-write buffer guard",
        r#"new RegExp("(?<"+__repeat("a",122)+"𐒤>x)")"#,
    ),
    (
        "the capture prepass stops before a late name after 254 user captures",
        r#"(function(){var p="\\k<missing>"+__repeat("()",254)+"(?<x>a)";return new RegExp(p)})()"#,
    ),
];

const CAPTURE_AND_BACKREFERENCE_CASES: &[(&str, &str)] = &[
    (
        "raw and escaped Unicode names share captures and references",
        r#"(function(){var m=/(?<π>a)(?<\u{10400}>b)\k<\u03c0>\k<𐐀>/u.exec("abab");return [m[0],m.groups.π,m.groups["𐐀"],Object.keys(m.groups).join(",")].join("|")})()"#,
    ),
    (
        "an escaped surrogate-pair name is canonicalized to its scalar",
        r#"(function(){var m=/(?<\uD801\uDC00>a)\k<𐐀>/u.exec("aa");return [m[0],m.groups["𐐀"]].join("|")})()"#,
    ),
    (
        "a forward named reference is empty until its capture participates",
        r#"(function(){var m=/\k<a>(?<a>b)\w\k<a>/u.exec("bab");return [m[0],m[1],m.groups.a].join("|")})()"#,
    ),
    (
        "a named self-reference is empty while entering its capture",
        r#"(function(){var m=/(?<a>\k<a>\w)../u.exec("bab");return [m[0],m[1],m.groups.a].join("|")})()"#,
    ),
    (
        "Annex B treats k as an identity escape when no names exist",
        r#"(function(){var r=new RegExp("\\k<x>"),m=r.exec("k<x>");return [r.source,m[0]].join("|")})()"#,
    ),
    (
        "Annex B preserves every malformed k marker when no names exist",
        r#"(function(){var p=["\\k","\\k<>","\\k<->","\\k<"],out=[],i,r;for(i=0;i<p.length;i++){r=new RegExp(p[i]);out.push(r.source,r.test(p[i].slice(1)))}return out.join("|")})()"#,
    ),
    (
        "duplicate alternatives select the first participating backreference",
        r#"(function(){var r=/(?:(?<x>a)|(?<x>b))\k<x>/,a=r.exec("aa"),b=r.exec("bb");return [a[0],a[1],String(a[2]),a.groups.x,b[0],String(b[1]),b[2],b.groups.x].join("|")})()"#,
    ),
    (
        "repeated duplicate captures reset stale alternatives",
        r#"(function(){var m=/(?:(?:(?<x>a)|(?<x>b))\k<x>){2}/.exec("aabb");return [m[0],String(m[1]),m[2],m.groups.x].join("|")})()"#,
    ),
    (
        "QuickJS scope quirk lets sequential duplicates compile after a bar",
        r#"(function(){var r=/(?<x>a)(?:z|y)(?<x>b)\k<x>/,a=r.exec("azba"),b=r.exec("azbb");return [a[0],a[1],a[2],a.groups.x,b===null].join("|")})()"#,
    ),
    (
        "named captures retain reverse-quantifier semantics in lookbehind",
        r#"(function(){var a=/(?<=(?<x>\w){3})f/u.exec("abcdef"),b=/(?<=(?<x>\w)+)f/u.exec("abcdef");return [a[0],a.groups.x,b[0],b.groups.x].join("|")})()"#,
    ),
    (
        "a named backreference can execute backward inside lookbehind",
        r#"(function(){var m=/(?<x>a)b(?<=\k<x>b)c/.exec("zabc");return [m[0],m.groups.x,m.index].join("|")})()"#,
    ),
    (
        "ignoreCase canonicalization applies to named backreferences",
        r#"(function(){var m=/(?<x>k)\k<x>/iu.exec("kK");return [m[0],m.groups.x].join("|")})()"#,
    ),
    (
        "an unmatched named capture stays undefined in both result views",
        r#"(function(){var m=/(?<a>a).|(?<x>x)/.exec("ab");return [m[0],m[1],String(m[2]),m.groups.a,String(m.groups.x)].join("|")})()"#,
    ),
];

const GROUPS_AND_INDICES_CASES: &[(&str, &str)] = &[
    (
        "groups is null-prototype with ordinary data descriptors",
        r#"(function(){var m=/(?<__proto__>a)(?<x>b)?/.exec("a");return [Object.getPrototypeOf(m.groups)===null,m.groups.__proto__,String(m.groups.x),__descriptor(m,"groups"),__descriptor(m.groups,"__proto__"),__descriptor(m.groups,"x")].join("|")})()"#,
    ),
    (
        "groups remains an own undefined property without named captures",
        r#"(function(){var m=/a/.exec("a");return [Object.prototype.hasOwnProperty.call(m,"groups"),String(m.groups),__descriptor(m,"groups")].join("|")})()"#,
    ),
    (
        "every declared name exists even when its alternative is unmatched",
        r#"(function(){var g=/(?<a>a)|(?<b>b)/.exec("a").groups;return [Object.keys(g).join(","),g.a,String(g.b),Object.prototype.hasOwnProperty.call(g,"b")].join("|")})()"#,
    ),
    (
        "duplicate-name property order follows first source occurrence",
        r#"(function(){var r=/(?<y>a)(?<x>a)|(?<x>b)(?<y>b)/,a=r.exec("aa").groups,b=r.exec("bb").groups;return [Object.keys(a).join(","),a.y,a.x,Object.keys(b).join(","),b.y,b.x].join("|")})()"#,
    ),
    (
        "a later participating duplicate replaces an earlier undefined value",
        r#"(function(){var a=/(?<x>a)|(?<x>b)/.exec("b"),b=/(?<x>b)|(?<x>a)/.exec("b");return [a.groups.x,b.groups.x].join("|")})()"#,
    ),
    (
        "indices.groups mirrors names with a null prototype and descriptors",
        r#"(function(){var m=/(?<a>.)(?<b>.)/d.exec("ab"),g=m.indices.groups;return [Object.getPrototypeOf(g)===null,g.a.join(","),g.b.join(","),__descriptor(m.indices,"groups"),__descriptor(g,"a")].join("|")})()"#,
    ),
    (
        "indices.groups publishes undefined for an unmatched named capture",
        r#"(function(){var g=/(?<a>a)|(?<b>b)/d.exec("a").indices.groups;return [g.a.join(","),String(g.b),Object.keys(g).join(",")].join("|")})()"#,
    ),
    (
        "duplicate indices select the participating capture",
        r#"(function(){var r=/(?<x>a)|(?<x>b)/d,a=r.exec("..a"),b=r.exec("..b");return [a.indices.groups.x.join(","),b.indices.groups.x.join(",")].join("|")})()"#,
    ),
    (
        "prototype setters cannot intercept result or group definitions",
        r#"(function(){var hits=0;Object.defineProperty(Array.prototype,"groups",{set:function(){hits++},configurable:true});Object.defineProperty(Object.prototype,"x",{set:function(){hits++},configurable:true});var m=/(?<x>a)/.exec("a");delete Array.prototype.groups;delete Object.prototype.x;return [hits,m.groups.x].join("|")})()"#,
    ),
];

const REPLACEMENT_CASES: &[(&str, &str)] = &[
    (
        "a native named regexp falls back from the direct replacement helper",
        r#""abcd".replace(/(?<fst>.)(?<snd>.)/,"$<snd>$<fst>")"#,
    ),
    (
        "global named replacement rebuilds groups for every match",
        r#""abcd".replace(/(?<fst>.)(?<snd>.)/g,"$<snd>$<fst>")"#,
    ),
    (
        "missing and unmatched names substitute the empty string",
        r#"(function(){var r=/(?<a>a).|(?<x>x)/;return ["ab".replace(r,"$<x>"),"ab".replace(r,"$<missing>")].join("|")})()"#,
    ),
    (
        "without named captures dollar-angle remains literal on the direct path",
        r#""ab".replace(/(a)/,"$<x>")"#,
    ),
    (
        "an unclosed named replacement token remains literal",
        r#""ab".replace(/(?<x>a)/,"$<x")"#,
    ),
    (
        "duplicate names expose the participating value to replacement",
        r#"(function(){var r=/(?<x>a)|(?<x>b)/g;return "ab".replace(r,"[$<x>][$1][$2]")})()"#,
    ),
    (
        "functional replacement receives the exact groups object last",
        r#"(function(){var seen;var out="ab".replace(/(?<a>a)(?<b>b)/,function(match,a,b,index,input,groups){seen=[match,a,b,index,input,groups.a,groups.b,Object.getPrototypeOf(groups)===null,arguments.length].join("|");return groups.b+groups.a});return out+";"+seen})()"#,
    ),
    (
        "generic replacement performs a fresh groups Get for every token",
        r#"(function(){var reads=0,done=false,groups={};Object.defineProperty(groups,"x",{get:function(){reads++;return reads}});var rx={flags:"",exec:function(){if(done)return null;done=true;return {0:"a",length:1,index:0,groups:groups}}};var out=RegExp.prototype[Symbol.replace].call(rx,"a","$<x>$<x>");return [out,reads].join("|")})()"#,
    ),
    (
        "generic replacement reads inherited names and stringifies the value",
        r#"(function(){var done=false,log="",proto={x:{toString:function(){log+="S";return "X"}}},groups=Object.create(proto),rx={flags:"",exec:function(){if(done)return null;done=true;return {0:"a",length:1,index:0,groups:groups}}};var out=RegExp.prototype[Symbol.replace].call(rx,"a","[$<x>]");return out+"|"+log})()"#,
    ),
    (
        "escaped Unicode capture names are available to dollar-angle replacement",
        r#""a".replace(/(?<\u03c0>a)/u,"<$<π>>")"#,
    ),
];

const CONSTRUCTION_AND_COMPILE_CASES: &[(&str, &str)] = &[
    (
        "literal and constructor paths publish equivalent named metadata",
        r#"(function(){var a=/(?<x>a)/,b=new RegExp("(?<x>a)"),ma=a.exec("a"),mb=b.exec("a");return [a.source===b.source,ma.groups.x,mb.groups.x].join("|")})()"#,
    ),
    (
        "RegExp construction from a RegExp preserves named metadata",
        r#"(function(){var a=/(?<x>a)/d,b=new RegExp(a),m=b.exec("a");return [a!==b,b.source,b.flags,m.groups.x,m.indices.groups.x.join(",")].join("|")})()"#,
    ),
    (
        "explicit constructor flags recompile the copied named source",
        r#"(function(){var a=/(?<x>a)/,b=new RegExp(a,"id"),m=b.exec("A");return [b.flags,m.groups.x,m.indices.groups.x.join(",")].join("|")})()"#,
    ),
    (
        "RegExp compile replaces unnamed metadata with named metadata",
        r#"(function(){var r=/old/g;r.lastIndex=9;var result=r.compile("(?<x>a)","d"),m=r.exec("a");return [result===r,r.source,r.flags,r.lastIndex,m.groups.x,m.indices.groups.x.join(",")].join("|")})()"#,
    ),
    (
        "RegExp compile copies named bytecode metadata from its argument",
        r#"(function(){var source=/(?<x>a)/d,target=/old/g;target.compile(source);var m=target.exec("a");return [target.source,target.flags,m.groups.x,m.indices.groups.x.join(",")].join("|")})()"#,
    ),
];

#[test]
fn regexp_named_groups_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp named-groups oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for (group, cases) in case_groups() {
        for (index, &(description, source)) in cases.iter().enumerate() {
            let observation = observe_oracle(&oracle, source, description);
            let should_return = group != "grammar and names" || matches!(index, 0 | 12 | 14);
            if should_return {
                assert!(
                    observation.starts_with("return|"),
                    "{group} oracle vector unexpectedly threw for {description}: {observation:?}",
                );
            } else {
                assert!(
                    observation.starts_with("throw|object|SyntaxError|"),
                    "{group} oracle error vector changed for {description}: {observation:?}",
                );
            }
        }
    }
}

#[test]
fn regexp_named_group_grammar_and_names_match_pinned_quickjs() {
    compare_cases(
        "RegExp named-group grammar and names",
        GRAMMAR_AND_NAME_CASES,
    );
}

#[test]
fn regexp_named_captures_and_backreferences_match_pinned_quickjs() {
    compare_cases(
        "RegExp named captures and backreferences",
        CAPTURE_AND_BACKREFERENCE_CASES,
    );
}

#[test]
fn regexp_named_groups_and_indices_match_pinned_quickjs() {
    compare_cases("RegExp groups and indices.groups", GROUPS_AND_INDICES_CASES);
}

#[test]
fn regexp_named_group_replacement_matches_pinned_quickjs() {
    compare_cases("RegExp named replacement", REPLACEMENT_CASES);
}

#[test]
fn regexp_named_group_construction_and_compile_match_pinned_quickjs() {
    compare_cases(
        "RegExp named construction and compile",
        CONSTRUCTION_AND_COMPILE_CASES,
    );
}

#[test]
fn regexp_named_group_results_use_the_exec_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype =
        eval_object(&mut defining, "Array.prototype", "defining Array prototype");
    let caller_array_prototype =
        eval_object(&mut caller, "Array.prototype", "caller Array prototype");
    let exec = eval_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype.exec",
        "defining RegExp exec",
    );
    let regexp = eval_object(
        &mut caller,
        "new RegExp('(?<x>a)','d')",
        "caller named RegExp",
    );
    let result = expect_object(
        caller
            .call(
                &exec,
                Value::Object(regexp),
                &[Value::String(
                    quickjs_oxide::JsString::try_from_utf8("za").unwrap(),
                )],
            )
            .expect("cross-realm named RegExp exec"),
        "cross-realm named RegExp result",
    );
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype),
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype),
    );

    let groups = object_property(&runtime, &mut caller, &result, "groups");
    assert_eq!(runtime.get_prototype_of(&groups).unwrap(), None);
    assert_eq!(string_property(&runtime, &mut caller, &groups, "x"), "a",);
    let indices = object_property(&runtime, &mut caller, &result, "indices");
    assert_eq!(
        runtime.get_prototype_of(&indices).unwrap(),
        Some(eval_object(
            &mut defining,
            "Array.prototype",
            "defining Array prototype after exec",
        )),
    );
    let indices_groups = object_property(&runtime, &mut caller, &indices, "groups");
    assert_eq!(runtime.get_prototype_of(&indices_groups).unwrap(), None);
}

fn case_groups() -> [(&'static str, &'static [(&'static str, &'static str)]); 5] {
    [
        ("grammar and names", GRAMMAR_AND_NAME_CASES),
        (
            "captures and backreferences",
            CAPTURE_AND_BACKREFERENCE_CASES,
        ),
        ("groups and indices", GROUPS_AND_INDICES_CASES),
        ("replacement", REPLACEMENT_CASES),
        ("construction and compile", CONSTRUCTION_AND_COMPILE_CASES),
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

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    expect_object(
        context
            .eval(source)
            .unwrap_or_else(|error| panic!("evaluate {description}: {error}")),
        description,
    )
}

fn eval_callable(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> CallableRef {
    let callable = eval_object(context, source, description);
    runtime
        .as_callable(&callable)
        .unwrap()
        .unwrap_or_else(|| panic!("{description} was not callable"))
}

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(value) = value else {
        panic!("{description} was not an object");
    };
    value
}

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    let key = runtime.intern_property_key(name).unwrap();
    expect_object(
        context
            .get_property(object, &key)
            .unwrap_or_else(|error| panic!("read object property {name}: {error}")),
        name,
    )
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
