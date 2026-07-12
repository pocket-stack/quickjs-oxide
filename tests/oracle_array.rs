use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::value::number_to_string;
use quickjs_oxide::{
    CompleteOrdinaryPropertyDescriptor, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

// This target deliberately describes the complete first Array vertical slice,
// rather than the parser-only boundary which exists today.  Keep the cases
// usable without the global Object/Reflect builtins on the Rust side: source
// cases return primitives, while descriptor and own-key observations use the
// public host API and compare them with an equivalent QuickJS observer.

const CONSTRUCTOR_AND_LENGTH_CASES: &[(&str, &str)] = &[
    (
        "constructor metadata and isArray",
        "(function(){function X(){};return typeof Array+'|'+Array.name+'|'+Array.length+'|'+(Array.prototype.constructor===Array)+'|'+Array.isArray([])+'|'+Array.isArray(new X)})()",
    ),
    (
        "call and construct empty arrays",
        "(function(){var a=Array(),b=new Array;return a.length+'|'+b.length+'|'+(a instanceof Array)+'|'+(b instanceof Array)})()",
    ),
    (
        "single numeric argument creates holes",
        "(function(){var a=Array(3);return a.length+'|'+(0 in a)+'|'+(1 in a)+'|'+(2 in a)})()",
    ),
    (
        "single string argument is an element",
        "(function(){var a=Array('3');return a.length+'|'+a[0]+'|'+(0 in a)})()",
    ),
    (
        "multiple constructor arguments become dense elements",
        "(function(){var a=new Array(4,5,6);return a.length+'|'+a[0]+'|'+a[1]+'|'+a[2]})()",
    ),
    (
        "single object argument preserves identity",
        "(function(){function X(){this.marker=7};var x=new X,a=Array(x);return a.length+'|'+(a[0]===x)+'|'+a[0].marker})()",
    ),
    (
        "indexed assignment grows length",
        "(function(){var a=[];a[2]=7;return a.length+'|'+(0 in a)+'|'+(1 in a)+'|'+(2 in a)+'|'+a[2]})()",
    ),
    (
        "delete creates a hole without shrinking length",
        "(function(){var a=[1,2,3];var d=delete a[2];return d+'|'+a.length+'|'+(2 in a)+'|'+a[2]})()",
    ),
    (
        "length shrink deletes higher indices",
        "(function(){var a=[0,1,2,3];a.length=2;return a.length+'|'+a[0]+'|'+a[1]+'|'+(2 in a)+'|'+(3 in a)})()",
    ),
    (
        "zero length truncates every indexed property",
        "(function(){var a=[1,2];a.extra=3;a.length=0;return a.length+'|'+(0 in a)+'|'+(1 in a)+'|'+a.extra})()",
    ),
    (
        "negative zero is a valid zero length",
        "(function(){var a=[1];a.length=-0;return a.length+'|'+(1/a.length)+'|'+(0 in a)})()",
    ),
    (
        "array-index boundary distinguishes 2^32 minus one",
        "(function(){var a=[];a[4294967294]=7;a[4294967295]=8;return a.length+'|'+a[4294967294]+'|'+a[4294967295]})()",
    ),
    (
        "Array.from consumes an iterable and maps with thisArg",
        "(function(){function T(){this.bias=10};var a=Array.from([1,2],function(v,i){return this.bias+v+i},new T);return a.length+'|'+a[0]+'|'+a[1]})()",
    ),
    (
        "Array.from falls back to array-like indexing",
        "(function(){function X(){this[0]='x';this[2]='z';this.length=3};var a=Array.from(new X);return a.length+'|'+a[0]+'|'+a[1]+'|'+a[2]+'|'+(1 in a)})()",
    ),
    (
        "Array.of and species preserve constructor identities",
        "(function(){var a=Array.of(3,4);return a.length+'|'+a[0]+'|'+a[1]+'|'+(Array[Symbol.species]===Array)})()",
    ),
    (
        "Array.from closes an iterator when its mapper throws",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{Array.from(new X(l),function(){throw 7})}catch(e){return e+'|'+l.s}})()",
    ),
    (
        "hole lookup continues through Array prototype",
        "(function(){Array.prototype[1]=9;var a=[0,,2];return a.length+'|'+a[1]+'|'+(1 in a)})()",
    ),
];

const LITERAL_CASES: &[(&str, &str)] = &[
    (
        "dense array literal",
        "(function(){var a=[1,2,3];return a.length+'|'+a[0]+'|'+a[1]+'|'+a[2]})()",
    ),
    (
        "dense literal crosses the 32 element lowering boundary",
        "(function(){var a=[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32,33,34,35,36,37,38,39];return a.length+'|'+a[0]+'|'+a[31]+'|'+a[32]+'|'+a[39]})()",
    ),
    (
        "leading middle and trailing elisions",
        "(function(){var a=[,1,,3,,];return a.length+'|'+(0 in a)+'|'+a[1]+'|'+(2 in a)+'|'+a[3]+'|'+(4 in a)})()",
    ),
    (
        "trailing comma does not add an elision",
        "(function(){var a=[1,],b=[1,,];return a.length+'|'+(1 in a)+'|'+b.length+'|'+(1 in b)})()",
    ),
    (
        "nested arrays retain independent identity",
        "(function(){var a=[[1],[2]];return a.length+'|'+a[0][0]+'|'+a[1][0]+'|'+(a[0]!==a[1])})()",
    ),
    (
        "String spread advances by Unicode code point",
        "(function(){var a=[0,...'A\\uD83D\\uDCA9\\uD800',3];return a.length+'|'+a[0]+'|'+a[1]+'|'+a[2].length+'|'+a[2].charCodeAt(0)+'|'+a[3].charCodeAt(0)+'|'+a[4]})()",
    ),
    (
        "spreading a sparse array materializes undefined",
        "(function(){var source=[,2],a=[...source];return a.length+'|'+(0 in a)+'|'+a[0]+'|'+a[1]})()",
    ),
    (
        "spread and later elision share one growing length",
        "(function(){var a=[...'ab',,3,];return a.length+'|'+a[0]+'|'+a[1]+'|'+(2 in a)+'|'+a[3]})()",
    ),
    (
        "custom iterable spread preserves value order",
        "(function(){function R(v,d){this.value=v;this.done=d};function I(){this.i=0};I.prototype.next=function(){this.i++;return new R(this.i,this.i>2)};function X(){};X.prototype[Symbol.iterator]=function(){return new I};var a=[0,...new X,3];return a.length+'|'+a[0]+'|'+a[1]+'|'+a[2]+'|'+a[3]})()",
    ),
    (
        "literal element expressions run left to right",
        "(function(){Function.arrayLiteralLog='';function f(v){Function.arrayLiteralLog+=v;return v};var a=[f(1),f(2),f(3)];return a[0]+a[1]+a[2]+'|'+Function.arrayLiteralLog})()",
    ),
];

const ITERATOR_CASES: &[(&str, &str)] = &[
    (
        "for-of visits values and holes",
        "(function(){var s='';for(var v of [1,,3])s+=typeof v+':'+v+'|';return s})()",
    ),
    (
        "values iterator observes holes as undefined",
        "(function(){var i=[10,,30].values(),a=i.next(),b=i.next(),c=i.next(),d=i.next();return a.done+':'+a.value+'|'+b.done+':'+b.value+'|'+c.done+':'+c.value+'|'+d.done+':'+d.value})()",
    ),
    (
        "keys iterator visits every index below length",
        "(function(){var i=Array(3).keys(),a=i.next(),b=i.next(),c=i.next(),d=i.next();return a.value+'|'+b.value+'|'+c.value+'|'+d.done+'|'+d.value})()",
    ),
    (
        "entries iterator returns fresh key value arrays",
        "(function(){var i=[4,,6].entries(),a=i.next(),b=i.next(),c=i.next();return Array.isArray(a.value)+'|'+a.value[0]+':'+a.value[1]+'|'+b.value[0]+':'+b.value[1]+'|'+c.value[0]+':'+c.value[1]+'|'+(a.value!==b.value)})()",
    ),
    (
        "values is the default iterator and iterator is self iterable",
        "(function(){var a=[],i=a[Symbol.iterator]();return (a.values===a[Symbol.iterator])+'|'+(i[Symbol.iterator]()===i)+'|'+i[Symbol.toStringTag]})()",
    ),
    (
        "detached next works with its branded receiver",
        "(function(){var i=[7].values(),next=i.next,a=next.call(i),b=next.call(i);return a.done+'|'+a.value+'|'+b.done+'|'+b.value})()",
    ),
    (
        "completed iterator remains completed after append",
        "(function(){var a=[1],i=a.values();i.next();var before=i.next();a[1]=2;var after=i.next();return before.done+'|'+before.value+'|'+after.done+'|'+after.value})()",
    ),
    (
        "iterator observes append before completion",
        "(function(){var a=[1],i=a.values(),first=i.next();a[1]=2;var second=i.next(),done=i.next();return first.value+'|'+second.value+'|'+done.done})()",
    ),
    (
        "iterator observes length shrink before the next step",
        "(function(){var a=[1,2,3],i=a.values(),first=i.next();a.length=1;var done=i.next();return first.value+'|'+done.done+'|'+done.value})()",
    ),
    (
        "iterator observes deletion as undefined",
        "(function(){var a=[1,2],i=a.values(),first=i.next();delete a[1];var second=i.next();return first.value+'|'+second.done+'|'+second.value+'|'+(1 in a)})()",
    ),
    (
        "Array iterator methods are generic over array-like objects",
        "(function(){function X(){this[0]='x';this[2]='z';this.length=3};var i=Array.prototype.values.call(new X),a=i.next(),b=i.next(),c=i.next(),d=i.next();return a.value+'|'+b.value+'|'+c.value+'|'+d.done})()",
    ),
    (
        "for-of captures a fresh lexical binding for each array element",
        "(function(){var f,g,n=0;for(let value of [4,5]){n++;if(n===1)f=function(){return value};else g=function(){return value}}return f()+'|'+g()})()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("negative constructor length", "Array(-1)"),
    ("fractional constructor length", "Array(1.5)"),
    ("NaN constructor length", "Array(0/0)"),
    ("2^32 constructor length", "Array(4294967296)"),
    (
        "negative assigned length",
        "(function(){var a=[];a.length=-1;return a})()",
    ),
    (
        "fractional assigned length",
        "(function(){var a=[];a.length=1.5;return a})()",
    ),
    (
        "Symbol assigned length",
        "(function(){var a=[];a.length=Symbol();return a})()",
    ),
    ("non iterable literal spread", "[...1]"),
    (
        "non callable spread iterator method",
        "(function(){function X(){};X.prototype[Symbol.iterator]=1;return [...new X]})()",
    ),
    (
        "iterator next requires an Array Iterator receiver",
        "(function(){function X(){};var next=[].values().next;return next.call(new X)})()",
    ),
];

const SYNTAX_CASES: &[(&str, &str)] = &[
    ("unterminated empty array", "["),
    ("unterminated dense array", "[1,2"),
    ("missing comma between elements", "[1 2]"),
    ("spread has no operand", "[...]"),
    ("spread comma has no operand", "[...,]"),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "Array constructor invalid length keeps the call site",
        "(function outer(){return (function inner(){return Array(-1)})()})()",
    ),
    (
        "array literal spread error keeps the spread site",
        "(function outer(){return (function inner(){return [...1]})()})()",
    ),
    (
        "Array Iterator brand error keeps native and authored frames",
        "(function outer(){function X(){};var next=[].values().next;return (function inner(){return next.call(new X)})()})()",
    ),
];

const HOST_SNAPSHOT_CASES: &[(&str, &str)] = &[
    ("empty array own shape", "[]"),
    ("dense array own shape", "[10,20,30]"),
    ("sparse array own shape", "[,10,,30,,]"),
    ("single length constructor own shape", "Array(4)"),
    (
        "32 boundary literal own shape",
        "[0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31,32]",
    ),
    ("spread literal own shape", "[0,...'ab',3]"),
    (
        "numeric keys precede length and later strings",
        "(function(){var a=[];a.foo=1;a[3]=3;a[1]=1;return a})()",
    ),
];

#[test]
fn array_constructor_length_and_index_values_match_pinned_quickjs() {
    compare_value_cases("Array constructor/length", CONSTRUCTOR_AND_LENGTH_CASES);
}

#[test]
fn array_literal_dense_sparse_and_spread_values_match_pinned_quickjs() {
    compare_value_cases("array literals", LITERAL_CASES);
}

#[test]
fn array_iterators_and_for_of_match_pinned_quickjs() {
    compare_value_cases("Array iterators", ITERATOR_CASES);
}

#[test]
fn array_runtime_errors_match_pinned_quickjs() {
    compare_value_cases("Array errors", ERROR_CASES);
}

#[test]
fn array_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in SYNTAX_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn array_full_strip_source_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["--strip-source"], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn array_host_own_keys_and_descriptors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array host descriptor differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in HOST_SNAPSHOT_CASES {
        let expected = oracle_snapshot(&oracle, source, description);
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let array = eval_object(&mut context, source, description);
        assert_eq!(
            array_snapshot(&runtime, &mut context, &array),
            expected,
            "Array own-key/descriptor drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn array_constructor_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array constructor graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_constructor_graph(&oracle);
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        rust_constructor_graph(&runtime, &mut context),
        expected,
        "Array constructor/prototype/global descriptor graph drifted",
    );
}

#[test]
fn array_host_definitions_use_array_set_length_semantics() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array host mutation differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_host_mutation(&oracle);
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        rust_host_mutation(&runtime, &mut context),
        expected,
        "host property operations bypassed ArraySetLength/index semantics",
    );
}

#[test]
fn array_literal_iterator_and_errors_use_the_bytecode_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining
        .eval("Array.prototype.arrayRealm='defining';TypeError.prototype.arrayRealm='defining'")
        .expect("mark defining Array and TypeError prototypes");
    caller
        .eval("Array.prototype.arrayRealm='caller';TypeError.prototype.arrayRealm='caller'")
        .expect("mark caller Array and TypeError prototypes");
    let bytecode = defining
        .compile("(function(){var a=[1,2],realm=a.arrayRealm;function X(){};var next=a.values().next;try{next.call(new X)}catch(e){return realm+'|'+e.arrayRealm}})()")
        .expect("compile defining-realm Array probe");
    assert_eq!(
        caller
            .execute(&bytecode)
            .expect("execute Array probe cross realm"),
        Value::String(JsString::try_from_utf8("defining|defining").unwrap()),
    );
}

fn compare_value_cases(group: &str, cases: &[(&str, &str)]) {
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

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(source) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_value_text(value)
        ),
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
                value => format!(
                    "throw|{}|{}",
                    value_type(runtime, &value),
                    primitive_value_text(value)
                ),
            }
        }
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, source])
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
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"))
    else {
        panic!("Rust {description} did not evaluate to an object");
    };
    object
}

fn array_snapshot(runtime: &Runtime, context: &mut Context, array: &ObjectRef) -> String {
    runtime
        .own_property_keys(array)
        .expect("Array own keys")
        .into_iter()
        .map(|key| {
            let key_text = string_units(&runtime.property_key_to_js_string(&key).unwrap());
            let descriptor = context
                .get_own_property(array, &key)
                .expect("Array own descriptor")
                .expect("Array own key disappeared during snapshot");
            format!("{key_text}={}", descriptor_token(descriptor))
        })
        .collect::<Vec<_>>()
        .join(";")
}

fn descriptor_token(descriptor: CompleteOrdinaryPropertyDescriptor) -> String {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data {
            value,
            writable,
            enumerable,
            configurable,
        } => format!(
            "D:{}:{}:{}:{}",
            value_token(value),
            Number(writable),
            Number(enumerable),
            Number(configurable),
        ),
        CompleteOrdinaryPropertyDescriptor::Accessor {
            get,
            set,
            enumerable,
            configurable,
        } => format!(
            "A:{}:{}:{}:{}",
            Number(get.is_some()),
            Number(set.is_some()),
            Number(enumerable),
            Number(configurable),
        ),
    }
}

fn oracle_snapshot(oracle: &OsStr, source: &str, description: &str) -> String {
    let script = r#"
function units(s) {
  var out='';
  for (var i=0;i<s.length;i++) {
    if (i) out+=',';
    out+=('0000'+s.charCodeAt(i).toString(16)).slice(-4);
  }
  return out;
}
function valueToken(value) {
  if (value === undefined) return 'u';
  if (value === null) return 'n';
  if (typeof value === 'boolean') return value ? 'b1' : 'b0';
  if (typeof value === 'number') {
    if (value !== value) return 'dNaN';
    if (value === 0 && 1/value === -Infinity) return 'd-0';
    return 'd'+String(value);
  }
  if (typeof value === 'string') return 's'+units(value);
  if (typeof value === 'bigint') return 'i'+String(value);
  if (typeof value === 'symbol') return 'y';
  return 'o';
}
function descToken(d) {
  if ('value' in d)
    return 'D:'+valueToken(d.value)+':'+Number(d.writable)+':'+Number(d.enumerable)+':'+Number(d.configurable);
  return 'A:'+Number(d.get!==undefined)+':'+Number(d.set!==undefined)+':'+Number(d.enumerable)+':'+Number(d.configurable);
}
var array=std.evalScript(scriptArgs[0]);
var keys=Reflect.ownKeys(array),out='';
for(var i=0;i<keys.length;i++) {
  if(i) out+=';';
  out+=units(String(keys[i]))+'='+descToken(Object.getOwnPropertyDescriptor(array,keys[i]));
}
print(out);
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", script, source])
        .output()
        .unwrap_or_else(|error| panic!("could not snapshot QuickJS {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS snapshot failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS snapshot was not UTF-8 for {description}: {error}"))
        .trim_end()
        .to_owned()
}

fn rust_constructor_graph(runtime: &Runtime, context: &mut Context) -> String {
    let Value::Object(constructor) = context.eval("Array").expect("evaluate global Array") else {
        panic!("global Array was not an object");
    };
    let global = context.global_object().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let length = own_descriptor(runtime, context, &constructor, "length");
    let name = own_descriptor(runtime, context, &constructor, "name");
    let prototype_descriptor = own_descriptor(runtime, context, &constructor, "prototype");
    let Value::Object(array_prototype) = descriptor_value(&prototype_descriptor) else {
        panic!("Array.prototype descriptor did not contain an object");
    };
    let global_descriptor = own_descriptor(runtime, context, &global, "Array");
    let back_descriptor = own_descriptor(runtime, context, &array_prototype, "constructor");
    let callable = runtime.as_callable(&constructor).unwrap().is_some();
    let constructable = runtime.is_constructor(&constructor).unwrap();
    let constructor_prototype = runtime.get_prototype_of(&constructor).unwrap();
    let array_parent = runtime.get_prototype_of(&array_prototype).unwrap();
    let global_identity = matches!(descriptor_value(&global_descriptor), Value::Object(value) if value == constructor);
    let back_identity =
        matches!(descriptor_value(&back_descriptor), Value::Object(value) if value == constructor);
    let global_token = format!("{}:{global_identity}", descriptor_token(global_descriptor));
    let back_token = format!("{}:{back_identity}", descriptor_token(back_descriptor));
    format!(
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        callable,
        constructable,
        constructor_prototype.as_ref() == Some(&function_prototype),
        array_parent.as_ref() == Some(&object_prototype),
        descriptor_token(length),
        descriptor_token(name),
        descriptor_token(prototype_descriptor),
        global_token,
        back_token,
    )
}

fn oracle_constructor_graph(oracle: &OsStr) -> String {
    let script = r#"
function valueToken(value) {
  if (value === undefined) return 'u';
  if (value === null) return 'n';
  if (typeof value === 'boolean') return value ? 'b1' : 'b0';
  if (typeof value === 'number') return 'd'+String(value);
  if (typeof value === 'string') {
    var out='';for(var i=0;i<value.length;i++){if(i)out+=',';out+=('0000'+value.charCodeAt(i).toString(16)).slice(-4)}return 's'+out;
  }
  if (typeof value === 'bigint') return 'i'+String(value);
  if (typeof value === 'symbol') return 'y';
  return 'o';
}
function token(d) {
  if ('value' in d) return 'D:'+valueToken(d.value)+':'+Number(d.writable)+':'+Number(d.enumerable)+':'+Number(d.configurable);
  return 'A:'+Number(d.get!==undefined)+':'+Number(d.set!==undefined)+':'+Number(d.enumerable)+':'+Number(d.configurable);
}
var p=Array.prototype;
var globalDescriptor=Object.getOwnPropertyDescriptor(globalThis,'Array');
var backDescriptor=Object.getOwnPropertyDescriptor(p,'constructor');
print(
  (typeof Array==='function')+'|'+
  (typeof Array==='function')+'|'+
  (Object.getPrototypeOf(Array)===Function.prototype)+'|'+
  (Object.getPrototypeOf(p)===Object.prototype)+'|'+
  token(Object.getOwnPropertyDescriptor(Array,'length'))+'|'+
  token(Object.getOwnPropertyDescriptor(Array,'name'))+'|'+
  token(Object.getOwnPropertyDescriptor(Array,'prototype'))+'|'+
  token(globalDescriptor)+':'+(globalDescriptor.value===Array)+'|'+
  token(backDescriptor)+':'+(backDescriptor.value===Array)
);
"#;
    command_stdout(oracle, script, "Array constructor graph")
}

fn rust_host_mutation(runtime: &Runtime, context: &mut Context) -> String {
    let array = eval_object(context, "[]", "host mutation array");
    let index_two = runtime.intern_property_key("2").unwrap();
    let index_four = runtime.intern_property_key("4").unwrap();
    let index_six = runtime.intern_property_key("6").unwrap();
    let length = runtime.intern_property_key("length").unwrap();

    let set_two = context
        .set_property(&array, &index_two, Value::Int(7))
        .expect("set Array index 2");
    let define_four = context
        .define_own_property(
            &array,
            &index_four,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Int(9)),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .expect("define fixed Array index 4");
    let delete_two = runtime
        .delete_property(&array, &index_two)
        .expect("delete Array index 2");
    let shrink = context
        .set_property(&array, &length, Value::Int(1))
        .expect("attempt Array length shrink");
    let freeze_length = context
        .define_own_property(
            &array,
            &length,
            &OrdinaryPropertyDescriptor {
                writable: DescriptorField::Present(false),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .expect("make Array length non-writable");
    let define_six = context
        .define_own_property(
            &array,
            &index_six,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::Int(11)),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )
        .expect("attempt index past non-writable Array length");
    format!(
        "{set_two}|{define_four}|{delete_two}|{shrink}|{freeze_length}|{define_six}|{}",
        array_snapshot(runtime, context, &array),
    )
}

fn oracle_host_mutation(oracle: &OsStr) -> String {
    let script = r#"
function units(s){var out='';for(var i=0;i<s.length;i++){if(i)out+=',';out+=('0000'+s.charCodeAt(i).toString(16)).slice(-4)}return out}
function valueToken(value){if(value===undefined)return'u';if(value===null)return'n';if(typeof value==='boolean')return value?'b1':'b0';if(typeof value==='number')return'd'+String(value);if(typeof value==='string')return's'+units(value);if(typeof value==='bigint')return'i'+String(value);if(typeof value==='symbol')return'y';return'o'}
function descToken(d){if('value'in d)return'D:'+valueToken(d.value)+':'+Number(d.writable)+':'+Number(d.enumerable)+':'+Number(d.configurable);return'A:'+Number(d.get!==undefined)+':'+Number(d.set!==undefined)+':'+Number(d.enumerable)+':'+Number(d.configurable)}
function snapshot(a){var keys=Reflect.ownKeys(a),out='';for(var i=0;i<keys.length;i++){if(i)out+=';';out+=units(String(keys[i]))+'='+descToken(Object.getOwnPropertyDescriptor(a,keys[i]))}return out}
var a=[];
var setTwo=Reflect.set(a,'2',7);
var defineFour=Reflect.defineProperty(a,'4',{value:9});
var deleteTwo=delete a[2];
var shrink=Reflect.set(a,'length',1);
var freezeLength=Reflect.defineProperty(a,'length',{writable:false});
var defineSix=Reflect.defineProperty(a,'6',{value:11,writable:true,enumerable:true,configurable:true});
print(setTwo+'|'+defineFour+'|'+deleteTwo+'|'+shrink+'|'+freezeLength+'|'+defineSix+'|'+snapshot(a));
"#;
    command_stdout(oracle, script, "Array host mutation")
}

fn own_descriptor(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CompleteOrdinaryPropertyDescriptor {
    let key = runtime.intern_property_key(name).unwrap();
    context
        .get_own_property(object, &key)
        .unwrap_or_else(|error| panic!("get own {name}: {error}"))
        .unwrap_or_else(|| panic!("missing own {name}"))
}

fn descriptor_value(descriptor: &CompleteOrdinaryPropertyDescriptor) -> Value {
    match descriptor {
        CompleteOrdinaryPropertyDescriptor::Data { value, .. } => value.clone(),
        CompleteOrdinaryPropertyDescriptor::Accessor { .. } => {
            panic!("expected data property descriptor")
        }
    }
}

fn command_stdout(oracle: &OsStr, script: &str, description: &str) -> String {
    let output = Command::new(oracle)
        .args(["--std", "-e", script])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} output was not UTF-8: {error}"))
        .trim_end()
        .to_owned()
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

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}

fn error_string_property(
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
        Value::Float(value) => number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}

fn value_token(value: Value) -> String {
    match value {
        Value::Undefined => "u".to_owned(),
        Value::Null => "n".to_owned(),
        Value::Bool(value) => format!("b{}", Number(value)),
        Value::Int(value) => format!("d{value}"),
        Value::Float(value) if value.is_nan() => "dNaN".to_owned(),
        Value::Float(value) if value == 0.0 && value.is_sign_negative() => "d-0".to_owned(),
        Value::Float(value) => format!("d{}", number_to_string(value)),
        Value::BigInt(value) => format!("i{value}"),
        Value::String(value) => format!("s{}", string_units(&value)),
        Value::Object(_) => "o".to_owned(),
        Value::Symbol(_) => "y".to_owned(),
    }
}

fn string_units(value: &JsString) -> String {
    value
        .utf16_units()
        .map(|unit| format!("{unit:04x}"))
        .collect::<Vec<_>>()
        .join(",")
}

struct Number(bool);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
