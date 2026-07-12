use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This differential pins the first Array.prototype algorithm slice after the
// iterator surface to QuickJS 2026-06-04. Source probes avoid later Array
// methods and later Object reflection APIs on the Rust side. The graph probe
// uses the host API so own-key order and lazy native metadata remain observable
// without pulling Object.getOwnPropertyDescriptor into this milestone.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense at and search",
        r#"(function(){
            var a=[10,20,30];
            return a.at(0)+"|"+a.at(-1)+"|"+a.includes(20)+"|"+a.indexOf(20)+"|"+a.lastIndexOf(20);
        })()"#,
    ),
    (
        "holes differ between includes and index search",
        r#"(function(){
            var a=[,1,,];
            return a.length+"|"+(a.at(0)===undefined)+"|"+(a.at(2)===undefined)+"|"+
                a.includes(undefined)+"|"+a.indexOf(undefined)+"|"+a.lastIndexOf(undefined);
        })()"#,
    ),
    (
        "sparse length and a distant own element",
        r#"(function(){
            var a=[];a.length=6;a[4]="x";
            return a.at(-2)+"|"+(a.at(1)===undefined)+"|"+a.includes("x")+"|"+
                a.indexOf("x")+"|"+a.lastIndexOf("x");
        })()"#,
    ),
    (
        "holes continue through the prototype chain",
        r#"(function(){
            Array.prototype[1]=9;var a=[0,,2];
            return a.at(1)+"|"+a.includes(9)+"|"+a.indexOf(9)+"|"+a.lastIndexOf(9);
        })()"#,
    ),
    (
        "same value zero and strict equality search values",
        r#"(function(){
            var symbol=Symbol("needle"),other=Symbol("needle"),a=[0/0,0,7n,symbol];
            return a.includes(0/0)+"|"+a.indexOf(0/0)+"|"+a.lastIndexOf(0/0)+"|"+
                a.includes(-0)+"|"+a.indexOf(-0)+"|"+a.includes(7n)+"|"+
                a.indexOf(7n)+"|"+a.includes(7)+"|"+a.includes(symbol)+"|"+
                a.indexOf(symbol)+"|"+a.includes(other);
        })()"#,
    ),
    (
        "at integer conversion and saturation",
        r#"(function(){
            function show(value){return value===undefined?"u":value}
            var a=[10,20,30];
            return show(a.at())+"|"+show(a.at(undefined))+"|"+show(a.at(0/0))+"|"+
                show(a.at(1.9))+"|"+show(a.at(-1.9))+"|"+show(a.at(1/0))+"|"+
                show(a.at(-1/0))+"|"+show(a.at(1e100))+"|"+show(a.at(-1e100));
        })()"#,
    ),
];

const FROM_INDEX_CASES: &[(&str, &str)] = &[
    (
        "omitted and explicit undefined fromIndex",
        r#"(function(){
            var a=[1,2,1];
            return a.includes(1)+"|"+a.includes(1,undefined)+"|"+
                a.indexOf(1)+"|"+a.indexOf(1,undefined)+"|"+
                a.lastIndexOf(1)+"|"+a.lastIndexOf(1,undefined);
        })()"#,
    ),
    (
        "forward fromIndex clamp matrix",
        r#"(function(){
            var a=[0,1,2,1,4];
            return a.indexOf(1,0/0)+"|"+a.indexOf(1,1/0)+"|"+a.indexOf(1,-1/0)+"|"+
                a.indexOf(1,1.9)+"|"+a.indexOf(1,2.9)+"|"+a.indexOf(1,-2.9)+"|"+
                a.indexOf(1,-1.9)+"|"+a.indexOf(1,1e100)+"|"+a.indexOf(1,-1e100)+"|"+
                a.includes(1,2.9)+"|"+a.includes(1,-2.9);
        })()"#,
    ),
    (
        "reverse fromIndex clamp matrix",
        r#"(function(){
            var a=[0,1,2,1,4];
            return a.lastIndexOf(1,0/0)+"|"+a.lastIndexOf(1,1/0)+"|"+
                a.lastIndexOf(1,-1/0)+"|"+a.lastIndexOf(1,2.9)+"|"+
                a.lastIndexOf(1,-2.9)+"|"+a.lastIndexOf(1,-1.9)+"|"+
                a.lastIndexOf(1,1e100)+"|"+a.lastIndexOf(1,-1e100);
        })()"#,
    ),
    (
        "zero length skips fromIndex conversion",
        r#"(function(){
            var index=Object(),a=[];index.valueOf=function(){throw "converted"};
            return a.includes(1,index)+"|"+a.indexOf(1,index)+"|"+a.lastIndexOf(1,index);
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "length number index and element access order",
        r#"(function(){
            function row(method,isAt,isReverse){
                var log="",length=Object(),index=Object(),receiver=Object();
                length.valueOf=function(){log+="N";return 3};
                index.valueOf=function(){log+="F";return isReverse?1/0:(isAt?-1:0)};
                receiver.__defineGetter__("length",function(){log+="L";return length});
                receiver.__defineGetter__("0",function(){log+="0";return "a"});
                receiver.__defineGetter__("1",function(){log+="1";return "b"});
                receiver.__defineGetter__("2",function(){log+="2";return "c"});
                var result=isAt?method.call(receiver,index):method.call(receiver,"missing",index);
                return result+":"+log;
            }
            return row(Array.prototype.at,true,false)+"|"+
                row(Array.prototype.includes,false,false)+"|"+
                row(Array.prototype.indexOf,false,false)+"|"+
                row(Array.prototype.lastIndexOf,false,true);
        })()"#,
    ),
    (
        "length and conversion throws preserve order",
        r#"(function(){
            function row(method,isAt,stage){
                var log="",length=Object(),index=Object(),receiver=Object();
                length.valueOf=function(){log+="N";if(stage===1)throw "number";return 1};
                index.valueOf=function(){log+="F";if(stage===2)throw "index";return 0};
                receiver.__defineGetter__("length",function(){log+="L";if(stage===0)throw "length";return length});
                receiver.__defineGetter__("0",function(){log+="0";if(stage===3)throw "element";return "x"});
                try{
                    if(isAt)return method.call(receiver,index);
                    return method.call(receiver,"missing",index);
                }catch(error){return log+":"+error}
            }
            return row(Array.prototype.at,true,0)+"|"+row(Array.prototype.at,true,1)+"|"+
                row(Array.prototype.at,true,2)+"|"+row(Array.prototype.at,true,3)+"|"+
                row(Array.prototype.includes,false,0)+"|"+row(Array.prototype.includes,false,1)+"|"+
                row(Array.prototype.includes,false,2)+"|"+row(Array.prototype.includes,false,3);
        })()"#,
    ),
    (
        "index search gets present accessors and propagates their throws",
        r#"(function(){
            function row(method){
                var log="",receiver=Object();receiver.length=1;
                receiver.__defineGetter__("0",function(){log+="0";throw "element"});
                try{method.call(receiver,"x")}catch(error){return log+":"+error}
            }
            return row(Array.prototype.indexOf)+"|"+row(Array.prototype.lastIndexOf);
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary object receiver",
        r#"(function(){
            var receiver=Object();receiver[0]="a";receiver[2]="c";receiver.length=3;
            return Array.prototype.at.call(receiver,-1)+"|"+
                (Array.prototype.at.call(receiver,1)===undefined)+"|"+
                Array.prototype.includes.call(receiver,undefined)+"|"+
                Array.prototype.indexOf.call(receiver,undefined)+"|"+
                Array.prototype.lastIndexOf.call(receiver,"a");
        })()"#,
    ),
    (
        "string receiver exposes UTF-16 indexed properties",
        r#"(function(){
            var string="A\uD83D\uDCA9Z",second=Array.prototype.at.call(string,1),third=Array.prototype.at.call(string,2);
            return Array.prototype.at.call(string,-1)+"|"+second.charCodeAt(0)+"|"+third.charCodeAt(0)+"|"+
                Array.prototype.includes.call(string,"Z")+"|"+
                Array.prototype.indexOf.call(string,"Z")+"|"+
                Array.prototype.lastIndexOf.call(string,"A");
        })()"#,
    ),
    (
        "number boolean bigint and symbol receivers box to zero length",
        r#"(function(){
            var symbol=Symbol("receiver");
            return (Array.prototype.at.call(7,0)===undefined)+"|"+
                Array.prototype.includes.call(false,false)+"|"+
                Array.prototype.indexOf.call(7n,7n)+"|"+
                Array.prototype.lastIndexOf.call(symbol,symbol);
        })()"#,
    ),
    (
        "nullish receivers are rejected",
        r#"(function(){
            function row(method,value){try{method.call(value,0);return "missing"}catch(error){return error.name}}
            return row(Array.prototype.at,null)+"|"+row(Array.prototype.includes,undefined)+"|"+
                row(Array.prototype.indexOf,null)+"|"+row(Array.prototype.lastIndexOf,undefined);
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("at rejects null", "Array.prototype.at.call(null,0)"),
    (
        "includes rejects undefined",
        "Array.prototype.includes.call(undefined,0)",
    ),
    (
        "at rejects a Symbol index",
        "Array.prototype.at.call([1],Symbol('index'))",
    ),
    (
        "indexOf rejects a Symbol fromIndex",
        "Array.prototype.indexOf.call([1],1,Symbol('fromIndex'))",
    ),
];

const GRAPH_ORACLE: &str = r#"
var selected=['at','indexOf','lastIndexOf','includes'];
var implemented=['at','indexOf','lastIndexOf','includes','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
print('keys='+names.join(','));
var metadata=[];
for(var i=0;i<selected.length;i++) {
  var name=selected[i],descriptor=Object.getOwnPropertyDescriptor(Array.prototype,name),fn=descriptor.value;
  var constructor;
  try { Reflect.construct(function(){},[],fn); constructor=true; }
  catch(error) { constructor=false; }
  metadata[metadata.length]=name+':'+fn.name+':'+fn.length+':D:'+
    Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable)+':'+
    (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructor;
}
print('meta='+metadata.join('|'));
"#;

#[test]
fn array_search_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array search oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("fromIndex", FROM_INDEX_CASES),
        ("order", ORDER_CASES),
        ("generic", GENERIC_CASES),
        ("errors", ERROR_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 2);
}

#[test]
fn array_search_values_match_pinned_quickjs() {
    compare_value_cases("Array at/search values", VALUE_CASES);
}

#[test]
fn array_search_from_index_matches_pinned_quickjs() {
    compare_value_cases("Array search fromIndex", FROM_INDEX_CASES);
}

#[test]
fn array_search_observable_order_matches_pinned_quickjs() {
    compare_value_cases("Array search observable order", ORDER_CASES);
}

#[test]
fn array_search_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array search generic receivers", GENERIC_CASES);
}

#[test]
fn array_search_errors_match_pinned_quickjs() {
    compare_value_cases("Array search errors", ERROR_CASES);
}

#[test]
fn array_search_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array search graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_graph_observations(&oracle);
    assert_eq!(
        rust_graph_observations(),
        expected,
        "Array search prototype order/metadata drifted",
    );
}

#[test]
fn array_search_errors_use_the_native_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining
        .eval("TypeError.prototype.arraySearchRealm='defining'")
        .expect("mark defining TypeError prototype");
    caller
        .eval("TypeError.prototype.arraySearchRealm='caller'")
        .expect("mark caller TypeError prototype");
    let defining_array_prototype = defining.array_prototype().unwrap();
    let marker_key = runtime.intern_property_key("arraySearchRealm").unwrap();

    for name in ["at", "includes", "indexOf", "lastIndexOf"] {
        let method = property_callable(&runtime, &mut defining, &defining_array_prototype, name);
        assert!(matches!(
            caller.call(&method, Value::Null, &[Value::Undefined]),
            Err(RuntimeError::Exception),
        ));
        let Value::Object(error) = caller
            .take_exception()
            .unwrap_or_else(|failure| panic!("take cross-realm {name} exception: {failure}"))
            .unwrap_or_else(|| panic!("cross-realm {name} exception was missing"))
        else {
            panic!("cross-realm {name} exception was not an object");
        };
        assert_eq!(
            caller.get_property(&error, &marker_key).unwrap(),
            Value::String(JsString::try_from_utf8("defining").unwrap()),
            "{name} allocated its TypeError in the caller realm",
        );
    }
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

fn rust_graph_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let selected = [
        "at",
        "indexOf",
        "lastIndexOf",
        "includes",
        "values",
        "keys",
        "entries",
    ];
    let names = runtime
        .own_property_keys(&array_prototype)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>();
    let metadata = ["at", "indexOf", "lastIndexOf", "includes"]
        .iter()
        .map(|name| {
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                name,
            )
        })
        .collect::<Vec<_>>()
        .join("|");
    vec![
        format!("keys={}", names.join(",")),
        format!("meta={metadata}"),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS Array search graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Array search graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array search graph output was not UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn method_metadata(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    function_prototype: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(function),
        writable,
        enumerable,
        configurable,
    } = runtime
        .get_own_property(owner, &key)
        .unwrap()
        .unwrap_or_else(|| panic!("missing Array.prototype.{name}"))
    else {
        panic!("Array.prototype.{name} was not a function data property");
    };
    let callable = runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("Array.prototype.{name} was not callable"));
    let name_key = runtime.intern_property_key("name").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let function_name = context.get_property(&function, &name_key).unwrap();
    let function_length = context.get_property(&function, &length_key).unwrap();
    format!(
        "{name}:{}:{}:D:{}{}{}:{}:{}:{}",
        primitive_value_text(function_name),
        primitive_value_text(function_length),
        Number(writable),
        Number(enumerable),
        Number(configurable),
        true,
        runtime.get_prototype_of(&function).unwrap().as_ref() == Some(function_prototype),
        runtime.is_constructor(callable.as_object()).unwrap(),
    )
}

fn property_callable(
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
        Value::Float(value) => quickjs_oxide::value::number_to_string(value),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}

struct Number(bool);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
