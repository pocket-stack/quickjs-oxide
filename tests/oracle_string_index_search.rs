use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

// Pins the first String.prototype slice after `toWellFormed` to QuickJS
// 2026-06-04. The Rust runtime intentionally does not publish the incomplete
// global String constructor yet, so executable Rust-side probes obtain these
// generic methods through a primitive string value rather than `String.prototype`.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "upstream ordinary search matrix",
        r#"(function(){
            return "abcabc".indexOf("cab")+"|"+"abcabc".indexOf("cab2")+"|"+
                "abc".indexOf("c")+"|"+"abcabc".lastIndexOf("abc")+"|"+
                "abcabc".lastIndexOf("missing")+"|"+"aaaa".indexOf("aa")+"|"+
                "aaaa".lastIndexOf("aa");
        })()"#,
    ),
    (
        "indexOf ordinary and empty-needle position matrix",
        r#"(function(){
            function row(needle){
                var positions=[undefined,0/0,-1/0,-1,-0,0,1,2,3,4,1/0,1.9,2.9,-1.9];
                var out=["aaa".indexOf(needle)];
                for(var i=0;i<positions.length;i++)
                    out[out.length]="aaa".indexOf(needle,positions[i]);
                return out.join(",");
            }
            return row("a")+"|"+row("");
        })()"#,
    ),
    (
        "lastIndexOf ordinary and empty-needle position matrix",
        r#"(function(){
            function row(needle){
                var positions=[undefined,0/0,-1/0,-1,-0,0,1,2,3,4,1/0,1.9,2.9,-1.9];
                var out=["aaa".lastIndexOf(needle)];
                for(var i=0;i<positions.length;i++)
                    out[out.length]="aaa".lastIndexOf(needle,positions[i]);
                return out.join(",");
            }
            return row("a")+"|"+row("");
        })()"#,
    ),
    (
        "missing search and generic primitive receivers",
        r#"(function(){
            var index="".indexOf,last="".lastIndexOf;
            return "undefined".indexOf()+"|"+"xundefinedx".lastIndexOf()+"|"+
                index.call(12323,"23")+"|"+last.call(true,"u")+"|"+
                index.call(717n,"17")+"|"+last.call(false,"false");
        })()"#,
    ),
    (
        "UTF-16 code-unit and surrogate searches",
        r#"(function(){
            var source="\ud83d\ude00\ud83dX\ude00";
            return source.length+"|"+source.indexOf("\ud83d")+"|"+
                source.lastIndexOf("\ud83d")+"|"+source.indexOf("\ude00")+"|"+
                source.lastIndexOf("\ude00")+"|"+source.indexOf("\ud83d\ude00")+"|"+
                source.lastIndexOf("\ud83d\ude00")+"|"+
                source.indexOf("\ude00\ud83d")+"|"+source.indexOf("\ude00",2)+"|"+
                source.lastIndexOf("\ud83d",1);
        })()"#,
    ),
    (
        "rope source and needle cross 512 and 8192-code-unit boundaries",
        r#"(function(){
            function grow(character,power){
                var value=character;
                for(var i=0;i<power;i++)value=value+value;
                return value;
            }
            var small=grow("a",9),large=grow("b",13);
            var source=small+"XYZ"+large+"Q"+small;
            var ropeNeedle=grow("b",9)+"Q";
            return source.length+"|"+source.indexOf("aXYZb")+"|"+
                source.lastIndexOf("bQa")+"|"+source.indexOf(ropeNeedle)+"|"+
                source.lastIndexOf("bb",8194)+"|"+source.indexOf("XYZ",512)+"|"+
                source.lastIndexOf("XYZ",511);
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "receiver search and position conversions use string string number hints",
        r#"(function(){
            function run(method){
                var log="",receiver=Object(),search=Object(),position=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="r:"+hint+",";return "abcabc"};
                search[Symbol.toPrimitive]=function(hint){log+="s:"+hint+",";return "bc"};
                position[Symbol.toPrimitive]=function(hint){log+="p:"+hint+",";return 1.9};
                return method.call(receiver,search,position)+":"+log;
            }
            return run("".indexOf)+"|"+run("".lastIndexOf);
        })()"#,
    ),
    (
        "abrupt conversions stop later receiver search and position work",
        r#"(function(){
            function run(method,stage){
                var log="",receiver=Object(),search=Object(),position=Object();
                receiver[Symbol.toPrimitive]=function(hint){
                    log+="r:"+hint+",";if(stage===0)throw "receiver";return "abcabc";
                };
                search[Symbol.toPrimitive]=function(hint){
                    log+="s:"+hint+",";if(stage===1)throw "search";return "bc";
                };
                position[Symbol.toPrimitive]=function(hint){
                    log+="p:"+hint+",";if(stage===2)throw "position";return 1;
                };
                try{return method.call(receiver,search,position)+":"+log}
                catch(error){return "throw:"+error+":"+log}
            }
            return run("".indexOf,0)+"|"+run("".indexOf,1)+"|"+
                run("".indexOf,2)+"|"+run("".lastIndexOf,0)+"|"+
                run("".lastIndexOf,1)+"|"+run("".lastIndexOf,2);
        })()"#,
    ),
    (
        "omitted position skips conversion while explicit undefined is converted",
        r#"(function(){
            function run(method){
                var log="",position=Object();
                position[Symbol.toPrimitive]=function(hint){log+=hint+",";return undefined};
                var omitted=method.call("aaa","a");
                var converted=method.call("aaa","a",position);
                return omitted+":"+converted+":"+log;
            }
            return run("".indexOf)+"|"+run("".lastIndexOf);
        })()"#,
    ),
    (
        "index searches never inspect Symbol.match",
        r#"(function(){
            function run(method){
                var log="",needle=Object();
                needle.__defineGetter__(Symbol.match,function(){log+="match,";throw "match"});
                needle.toString=function(){log+="string,";return "bc"};
                return method.call("abcabc",needle)+":"+log;
            }
            return run("".indexOf)+"|"+run("".lastIndexOf);
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "indexOf rejects null receiver",
        r#"(function(){var method="".indexOf;return method.call(null,"x")})()"#,
    ),
    (
        "lastIndexOf rejects undefined receiver",
        r#"(function(){var method="".lastIndexOf;return method.call(undefined,"x")})()"#,
    ),
    (
        "indexOf rejects Symbol receiver",
        r#"(function(){var method="".indexOf;return method.call(Symbol("receiver"),"x")})()"#,
    ),
    (
        "indexOf rejects Symbol search",
        r#""x".indexOf(Symbol("search"))"#,
    ),
    (
        "lastIndexOf rejects Symbol search",
        r#""x".lastIndexOf(Symbol("search"))"#,
    ),
    (
        "indexOf rejects Symbol position",
        r#""x".indexOf("x",Symbol("position"))"#,
    ),
    (
        "lastIndexOf rejects Symbol position",
        r#""x".lastIndexOf("x",Symbol("position"))"#,
    ),
    ("indexOf rejects BigInt position", r#""x".indexOf("x",1n)"#),
    (
        "lastIndexOf rejects BigInt position",
        r#""x".lastIndexOf("x",1n)"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
function bits(descriptor) {
  return 'D:'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
function isConstructor(value) {
  try { Reflect.construct(function(){}, [], value); return true; }
  catch (_) { return false; }
}
function callableMeta(owner,name) {
  var descriptor=Object.getOwnPropertyDescriptor(owner,name),value=descriptor.value;
  return value.name+':'+value.length+':' +
    (Object.getPrototypeOf(value)===Function.prototype)+':' +
    (typeof value==='function')+':'+isConstructor(value)+':' +
    Object.getOwnPropertyNames(value).join(',')+':'+bits(descriptor);
}
var selected=['length','at','charCodeAt','charAt','concat','codePointAt',
  'isWellFormed','toWellFormed','indexOf','lastIndexOf','toString','valueOf'];
print('keys='+Reflect.ownKeys(String.prototype).filter(function(key){
  return selected.indexOf(key)>=0;
}).map(String).join(','));
print('indexOf='+callableMeta(String.prototype,'indexOf'));
print('lastIndexOf='+callableMeta(String.prototype,'lastIndexOf'));
print('identity='+(String.prototype.indexOf===String.prototype.indexOf)+':' +
  (String.prototype.lastIndexOf===String.prototype.lastIndexOf)+':' +
  (String.prototype.indexOf!==String.prototype.lastIndexOf));
print('index-props='+bits(Object.getOwnPropertyDescriptor(String.prototype.indexOf,'length'))+':' +
  bits(Object.getOwnPropertyDescriptor(String.prototype.indexOf,'name')));
print('last-props='+bits(Object.getOwnPropertyDescriptor(String.prototype.lastIndexOf,'length'))+':' +
  bits(Object.getOwnPropertyDescriptor(String.prototype.lastIndexOf,'name')));
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var removed=delete String.prototype.indexOf;
print(removed+'|'+('indexOf' in String.prototype)+'|'+String.prototype.indexOf);
"#;

#[test]
fn string_index_search_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String index search oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("errors", ERROR_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 6);
}

#[test]
fn string_index_search_values_match_pinned_quickjs() {
    compare_cases("String index search values", VALUE_CASES);
}

#[test]
fn string_index_search_conversion_order_matches_pinned_quickjs() {
    compare_cases("String index search conversion order", ORDER_CASES);
}

#[test]
fn string_index_search_errors_match_pinned_quickjs() {
    compare_cases("String index search errors", ERROR_CASES);
}

#[test]
fn string_index_search_graph_and_auto_init_identity_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String index search graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "String index search graph or lazy identity drifted",
    );
}

#[test]
fn string_index_search_auto_init_can_be_deleted_before_first_get() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String index search fresh delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_fresh_delete_observation(),
        oracle_script_lines(&oracle, FRESH_DELETE_ORACLE, "fresh indexOf delete")
            .into_iter()
            .next()
            .expect("fresh-delete oracle emitted no line"),
        "String.prototype.indexOf AutoInit deletion drifted",
    );
}

#[test]
fn string_index_search_cross_realm_errors_and_user_throws_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_string = defining.string_prototype().unwrap();
    let index_of = property_callable(&runtime, &mut defining, &defining_string, "indexOf");
    let last_index_of = property_callable(&runtime, &mut defining, &defining_string, "lastIndexOf");

    assert_eq!(
        caller
            .call(
                &index_of,
                Value::String(js_string("abcabc")),
                &[Value::String(js_string("cab"))],
            )
            .unwrap(),
        Value::Int(2),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    assert_eq!(
        caller.call(&index_of, Value::Null, &[Value::String(js_string("x"))],),
        Err(RuntimeError::Exception),
    );
    let framework_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
        Some(defining_type_error),
        "null-receiver error did not use the method's defining realm",
    );

    let caller_range_error = intrinsic_prototype(&runtime, &mut caller, "RangeError");
    let throwing_search = caller.new_object().unwrap();
    define_method(
        &runtime,
        &mut caller,
        &throwing_search,
        "toString",
        "(function(){throw new RangeError('search')})",
    );
    assert_eq!(
        caller.call(
            &index_of,
            Value::String(js_string("abc")),
            &[Value::Object(throwing_search)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_range_error),
        "user search conversion throw lost its callable realm",
    );

    let sentinel = caller.new_object().unwrap();
    let caller_global = caller.global_object().unwrap();
    define_data(
        &runtime,
        &caller_global,
        "stringSearchSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_position = caller.new_object().unwrap();
    let position_conversion = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw stringSearchSentinel})",
    );
    define_to_primitive(&runtime, &throwing_position, position_conversion);
    assert_eq!(
        caller.call(
            &last_index_of,
            Value::String(js_string("abc")),
            &[
                Value::String(js_string("a")),
                Value::Object(throwing_position),
            ],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "user position conversion throw identity changed",
    );
}

#[test]
fn string_index_search_methods_are_per_realm_and_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (index_of, last_index_of) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_string = first.string_prototype().unwrap();
        let second_string = second.string_prototype().unwrap();
        let first_index = property_callable(&runtime, &mut first, &first_string, "indexOf");
        let first_index_again = property_callable(&runtime, &mut first, &first_string, "indexOf");
        let first_last = property_callable(&runtime, &mut first, &first_string, "lastIndexOf");
        let second_index = property_callable(&runtime, &mut second, &second_string, "indexOf");
        assert_eq!(
            first_index, first_index_again,
            "AutoInit identity was unstable"
        );
        assert_ne!(
            first_index, first_last,
            "the magic variants shared identity"
        );
        assert_ne!(
            first_index, second_index,
            "different realms shared a method object"
        );
        assert_eq!(
            runtime.get_prototype_of(first_index.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        assert_eq!(
            runtime.get_prototype_of(second_index.as_object()).unwrap(),
            Some(second.function_prototype().unwrap()),
        );
        drop(second_index);
        (first_index, first_last)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(index_of);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "lastIndexOf should still retain the shared defining realm",
    );
    drop(last_index_of);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle(&oracle, source, description),
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
    let prototype = context.string_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let selected = [
        "length",
        "at",
        "charCodeAt",
        "charAt",
        "concat",
        "codePointAt",
        "isWellFormed",
        "toWellFormed",
        "indexOf",
        "lastIndexOf",
        "toString",
        "valueOf",
    ];
    let keys = own_key_names(&runtime, &prototype)
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let index_of = property_callable(&runtime, &mut context, &prototype, "indexOf");
    let index_again = property_callable(&runtime, &mut context, &prototype, "indexOf");
    let last_index_of = property_callable(&runtime, &mut context, &prototype, "lastIndexOf");
    vec![
        format!("keys={keys}"),
        format!(
            "indexOf={}",
            callable_meta(
                &runtime,
                &mut context,
                &prototype,
                "indexOf",
                &index_of,
                &function_prototype,
            )
        ),
        format!(
            "lastIndexOf={}",
            callable_meta(
                &runtime,
                &mut context,
                &prototype,
                "lastIndexOf",
                &last_index_of,
                &function_prototype,
            )
        ),
        format!(
            "identity={}:{}:{}",
            index_of == index_again,
            last_index_of == property_callable(&runtime, &mut context, &prototype, "lastIndexOf"),
            index_of != last_index_of,
        ),
        format!(
            "index-props={}:{}",
            descriptor_bits(data_descriptor(
                &runtime,
                index_of.as_object(),
                &runtime.intern_property_key("length").unwrap(),
            )),
            descriptor_bits(data_descriptor(
                &runtime,
                index_of.as_object(),
                &runtime.intern_property_key("name").unwrap(),
            )),
        ),
        format!(
            "last-props={}:{}",
            descriptor_bits(data_descriptor(
                &runtime,
                last_index_of.as_object(),
                &runtime.intern_property_key("length").unwrap(),
            )),
            descriptor_bits(data_descriptor(
                &runtime,
                last_index_of.as_object(),
                &runtime.intern_property_key("name").unwrap(),
            )),
        ),
    ]
}

fn callable_meta(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    name: &str,
    callable: &CallableRef,
    function_prototype: &ObjectRef,
) -> String {
    let outer = data_descriptor(runtime, owner, &runtime.intern_property_key(name).unwrap());
    format!(
        "{}:{}:{}:{}:{}:{}:{}",
        string_property(runtime, context, callable.as_object(), "name"),
        int_property(runtime, context, callable.as_object(), "length"),
        runtime
            .get_prototype_of(callable.as_object())
            .unwrap()
            .as_ref()
            == Some(function_prototype),
        runtime.as_callable(callable.as_object()).unwrap().is_some(),
        runtime.is_constructor(callable.as_object()).unwrap(),
        own_key_names(runtime, callable.as_object()).join(","),
        descriptor_bits(outer),
    )
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_script_lines(oracle, GRAPH_ORACLE, "String index search graph")
}

fn oracle_script_lines(oracle: &OsStr, source: &str, description: &str) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} output was not UTF-8: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn rust_fresh_delete_observation() -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let prototype = context.string_prototype().unwrap();
    let key = runtime.intern_property_key("indexOf").unwrap();
    let removed = runtime.delete_property(&prototype, &key).unwrap();
    let present = runtime.has_own_property(&prototype, &key).unwrap();
    let value = context.get_property(&prototype, &key).unwrap();
    let text = match value {
        Value::Undefined => "undefined",
        _ => "present",
    };
    format!("{removed}|{present}|{text}")
}

fn define_to_primitive(runtime: &Runtime, object: &ObjectRef, callable: CallableRef) {
    define_data_key(
        runtime,
        object,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive)),
        Value::Object(callable.as_object().clone()),
    );
}

fn define_method(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
    source: &str,
) {
    let callable = eval_callable(runtime, context, source);
    define_data(
        runtime,
        object,
        name,
        Value::Object(callable.as_object().clone()),
    );
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    define_data_key(
        runtime,
        object,
        &runtime.intern_property_key(name).unwrap(),
        value,
    );
}

fn define_data_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{source:?} was not callable"))
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    name: &str,
) -> CallableRef {
    let Value::Object(object) = context
        .get_property(owner, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let global = context.global_object().unwrap();
    let constructor = property_callable(runtime, context, &global, name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("operation did not throw an Error object");
    };
    error
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
    };
    value.to_utf8_lossy()
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let Value::Int(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an Int property");
    };
    value
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (Value, bool, bool, bool) {
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("missing data descriptor")
    else {
        panic!("property was not a data descriptor");
    };
    (value, writable, enumerable, configurable)
}

fn descriptor_bits(descriptor: (Value, bool, bool, bool)) -> String {
    format!(
        "D:{}{}{}",
        Number(descriptor.1),
        Number(descriptor.2),
        Number(descriptor.3),
    )
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

fn js_string(value: &str) -> JsString {
    JsString::try_from_utf8(value).unwrap()
}

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
