use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, PropertyKey,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_object_hasOwn`. Unlike the legacy
// `Object.prototype.hasOwnProperty`, the static first performs ToObject on its
// first argument and only then ToPropertyKey on its second argument. The final
// query is an own-property descriptor probe and must not Get the property.
// Proxy, TypedArray and module-namespace exotic own-property paths are recorded
// below without requiring their unpublished Rust intrinsics; Arguments is
// locked by this target and its dedicated differential.

const OWN_CASES: &[(&str, &str)] = &[
    (
        "ordinary own data accessor non-enumerable and Symbol properties",
        r#"(function(){
            var hits=0,proto=Object(),object=Object.create(proto),hidden=Object(),accessor=Object();
            var ownSymbol=Symbol("own"),inheritedSymbol=Symbol("inherited");
            proto.inherited=1;proto[inheritedSymbol]=2;object.visible=3;object[ownSymbol]=4;
            hidden.value=5;hidden.writable=false;hidden.enumerable=false;hidden.configurable=true;
            Object.defineProperty(object,"hidden",hidden);
            accessor.enumerable=true;accessor.configurable=true;
            accessor.get=function(){hits++;throw "getter"};
            Object.defineProperty(object,"accessor",accessor);
            return [
                Object.hasOwn.call(null,object,"visible"),Object.hasOwn(object,"hidden"),
                Object.hasOwn(object,"accessor"),hits,Object.hasOwn(object,"inherited"),
                Object.hasOwn(object,ownSymbol),Object.hasOwn(object,inheritedSymbol),
                Object.hasOwn(object,"missing")
            ].join(":");
        })()"#,
    ),
    (
        "primitive property keys use ToPropertyKey while Symbols retain identity",
        r#"(function(){
            var object=Object(),symbol=Symbol("key"),other=Symbol("key");
            object[0]="zero";object.NaN="nan";object.true="boolean";
            object.null="null";object.undefined="undefined";object[1]="bigint";object[symbol]="symbol";
            return [
                Object.hasOwn(object,-0),Object.hasOwn(object,NaN),Object.hasOwn(object,true),
                Object.hasOwn(object,null),Object.hasOwn(object,undefined),Object.hasOwn(object,1n),
                Object.hasOwn(object,symbol),Object.hasOwn(object,other)
            ].join(":");
        })()"#,
    ),
    (
        "primitive targets are boxed and String virtual indices are own",
        r#"(function(){
            var symbol=Symbol("target");
            return [
                Object.hasOwn("𝄞a",0),Object.hasOwn("𝄞a",1),Object.hasOwn("𝄞a",2),
                Object.hasOwn("𝄞a",3),Object.hasOwn("𝄞a","length"),
                Object.hasOwn(17,"valueOf"),Object.hasOwn(false,"valueOf"),
                Object.hasOwn(1n,"valueOf"),Object.hasOwn(symbol,"description")
            ].join(":");
        })()"#,
    ),
    (
        "Array holes length and function lazy properties retain ownness",
        r#"(function(){
            var array=[];array[2]=7;
            function ordinary(){}
            var bound=ordinary.bind(null);
            return [
                Object.hasOwn(array,0),Object.hasOwn(array,1),Object.hasOwn(array,2),
                Object.hasOwn(array,"length"),Object.hasOwn(ordinary,"length"),
                Object.hasOwn(ordinary,"name"),Object.hasOwn(ordinary,"prototype"),
                Object.hasOwn(bound,"length"),Object.hasOwn(bound,"name"),
                Object.hasOwn(bound,"prototype")
            ].join(":");
        })()"#,
    ),
    (
        "ownness tracks descriptor creation deletion and prototype shadowing",
        r#"(function(){
            var proto=Object(),object=Object.create(proto),descriptor=Object();proto.x=1;
            var before=Object.hasOwn(object,"x");
            descriptor.value=2;descriptor.writable=false;descriptor.enumerable=false;descriptor.configurable=true;
            Object.defineProperty(object,"x",descriptor);
            var defined=Object.hasOwn(object,"x"),deleted=delete object.x;
            return before+":"+defined+":"+deleted+":"+Object.hasOwn(object,"x")+":"+(object.x===1);
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "nullish target rejection precedes every property-key hook",
        r#"(function(){
            function run(target){
                var log="",key=Object();
                key[Symbol.toPrimitive]=function(hint){log+="key:"+hint+";";return "x"};
                try{Object.hasOwn(target,key)}catch(error){return log+error.name+":"+error.message}
                return "missing";
            }
            return run(null)+"|"+run(undefined);
        })()"#,
    ),
    (
        "target objects are not converted and key conversion completes before lookup",
        r#"(function(){
            var log="",target=Object(),key=Object();target.old=1;
            target.__defineGetter__(Symbol.toPrimitive,function(){log+="target-hook;";throw "target"});
            key[Symbol.toPrimitive]=function(hint){
                log+="key:"+hint+";";delete target.old;target.late=2;return "late";
            };
            var result=Object.hasOwn(target,key);
            return log+"|"+result+":"+Object.hasOwn(target,"old")+":"+Object.hasOwn(target,"late");
        })()"#,
    ),
    (
        "ordinary key conversion falls back from toString to valueOf",
        r#"(function(){
            var log="",target=Object(),key=Object();target.x=1;key[Symbol.toPrimitive]=undefined;
            key.__defineGetter__("toString",function(){
                log+="get-toString;";return function(){log+="call-toString;";return Object()};
            });
            key.__defineGetter__("valueOf",function(){
                log+="get-valueOf;";return function(){log+="call-valueOf;";return "x"};
            });
            return Object.hasOwn(target,key)+"|"+log;
        })()"#,
    ),
    (
        "own descriptor lookup does not read accessor values",
        r#"(function(){
            var calls=0,target=Object(),descriptor=Object();
            descriptor.enumerable=true;descriptor.configurable=true;
            descriptor.get=function(){calls++;throw "get"};
            Object.defineProperty(target,"x",descriptor);
            return Object.hasOwn(target,"x")+":"+calls;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("missing target", "Object.hasOwn()"),
    ("undefined target", "Object.hasOwn(undefined,'x')"),
    ("null target", "Object.hasOwn(null,'x')"),
    (
        "property-key conversion preserves a primitive throw",
        r#"(function(){var key=Object();key[Symbol.toPrimitive]=function(){throw "key"};return Object.hasOwn(Object(),key)})()"#,
    ),
    (
        "property-key conversion rejects an object result",
        r#"(function(){var key=Object();key[Symbol.toPrimitive]=function(){return Object()};return Object.hasOwn(Object(),key)})()"#,
    ),
    (
        "property-key conversion rejects a non-callable exotic hook",
        r#"(function(){var key=Object();key[Symbol.toPrimitive]=1;return Object.hasOwn(Object(),key)})()"#,
    ),
    (
        "hasOwn is not a constructor",
        "new Object.hasOwn(Object(),'x')",
    ),
];

const EXOTIC_ORACLE_ONLY_CASES: &[(&str, &str)] = &[
    (
        "Proxy own lookup follows key conversion and only invokes the descriptor trap",
        r#"(function(){
            var log="",base=Object(),key=Object();base.x=1;
            key[Symbol.toPrimitive]=function(hint){log+="key:"+hint+";";return "x"};
            var proxy=new Proxy(base,{
                getOwnPropertyDescriptor:function(target,name){
                    log+="descriptor:"+String(name)+";";return Reflect.getOwnPropertyDescriptor(target,name);
                },
                get:function(){log+="get;";throw "get"}
            });
            return Object.hasOwn(proxy,key)+"|"+log;
        })()"#,
    ),
    (
        "Proxy descriptor invariant failures remain observable",
        r#"(function(){
            var log="",base=Object.preventExtensions(Object());
            var proxy=new Proxy(base,{getOwnPropertyDescriptor:function(){
                log+="descriptor;";
                return {value:1,writable:true,enumerable:true,configurable:true};
            }});
            try{Object.hasOwn(proxy,"x")}catch(error){return log+error.name+":"+error.message}
            return "missing";
        })()"#,
    ),
    (
        "TypedArray integer-indexed elements are own but length is inherited",
        r#"(function(){
            var value=new Uint8Array([7,8]);
            return [Object.hasOwn(value,0),Object.hasOwn(value,1),Object.hasOwn(value,2),
                Object.hasOwn(value,"-0"),Object.hasOwn(value,"length")].join(":");
        })()"#,
    ),
    (
        "mapped arguments expose and can delete indexed own properties",
        r#"(function(value){
            var before=Object.hasOwn(arguments,0),length=Object.hasOwn(arguments,"length");
            var callee=Object.hasOwn(arguments,"callee"),deleted=delete arguments[0];
            return before+":"+length+":"+callee+":"+deleted+":"+Object.hasOwn(arguments,0)+":"+value;
        })(7)"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
function bit(value){return value?1:0}
function bits(descriptor){return 'D:'+bit(descriptor.writable)+bit(descriptor.enumerable)+bit(descriptor.configurable)}
function isConstructor(value){try{Reflect.construct(function(){},[],value);return true}catch(_){return false}}
function meta(name){
  var descriptor=Object.getOwnPropertyDescriptor(Object,name),value=descriptor.value;
  return value.name+':'+value.length+':' +(Object.getPrototypeOf(value)===Function.prototype)+':' +
    (typeof value==='function')+':'+isConstructor(value)+':' +Object.getOwnPropertyNames(value).join(',')+':'+bits(descriptor);
}
var selected=['length','name','prototype','create','getPrototypeOf','setPrototypeOf','defineProperty',
  'defineProperties','getOwnPropertyNames','getOwnPropertySymbols','groupBy','keys','values','entries',
  'isExtensible','preventExtensions','getOwnPropertyDescriptor','getOwnPropertyDescriptors','is','assign',
  'seal','freeze','isSealed','isFrozen','fromEntries','hasOwn'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('hasOwn='+meta('hasOwn'));
print('identity='+(Object.hasOwn===Object.hasOwn));
var fn=Object.hasOwn;
print('hasOwn-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var deleted=delete Object.hasOwn;
print([deleted,'hasOwn' in Object,Object.prototype.hasOwnProperty.call(Object,'hasOwn'),typeof Object.hasOwn].join('|'));
"#;

#[test]
fn object_has_own_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.hasOwn oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("own values", OWN_CASES),
        ("conversion order", ORDER_CASES),
        ("errors", ERROR_CASES),
        ("exotic boundary", EXOTIC_ORACLE_ONLY_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 4);
    assert_eq!(
        oracle_lines(&oracle, FRESH_DELETE_ORACLE, "Object.hasOwn fresh delete").len(),
        1,
    );
}

#[test]
fn object_has_own_values_match_pinned_quickjs() {
    compare_cases("Object.hasOwn values", OWN_CASES);
}

#[test]
fn object_has_own_conversion_and_lookup_order_match_pinned_quickjs() {
    compare_cases("Object.hasOwn conversion order", ORDER_CASES);
}

#[test]
fn object_has_own_errors_match_pinned_quickjs() {
    compare_cases("Object.hasOwn errors", ERROR_CASES);
}

#[test]
fn object_has_own_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.hasOwn graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Object.hasOwn graph or metadata drifted",
    );
}

#[test]
fn object_has_own_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.hasOwn AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(&oracle, FRESH_DELETE_ORACLE, "Object.hasOwn fresh delete");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let key = runtime.intern_property_key("hasOwn").unwrap();
    let deleted = runtime
        .delete_property(object.as_object(), &key)
        .unwrap()
        .to_string();
    let Value::Bool(in_object) = context.eval("'hasOwn' in Object").unwrap() else {
        panic!("Object.hasOwn inherited-presence probe was not boolean");
    };
    let own = runtime
        .has_own_property(object.as_object(), &key)
        .unwrap()
        .to_string();
    let kind = value_type(
        &runtime,
        &context.get_property(object.as_object(), &key).unwrap(),
    );
    assert_eq!(
        vec![format!("{deleted}|{in_object}|{own}|{kind}")],
        expected,
    );
}

#[test]
fn object_has_own_cross_realm_objects_errors_and_user_throws_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let has_own = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "hasOwn",
    );

    let prototype = caller.new_object().unwrap();
    let key = runtime.intern_property_key("x").unwrap();
    assert!(
        caller
            .set_property(&prototype, &key, Value::Int(1))
            .unwrap()
    );
    let inherited = caller.new_object_with_prototype(Some(&prototype)).unwrap();
    let own = caller.new_object().unwrap();
    assert!(caller.set_property(&own, &key, Value::Int(2)).unwrap());
    let key_value = Value::String(JsString::try_from_utf8("x").unwrap());
    assert_eq!(
        caller
            .call(
                &has_own,
                Value::Object(prototype),
                &[Value::Object(own), key_value.clone()],
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        caller
            .call(
                &has_own,
                Value::Undefined,
                &[Value::Object(inherited), key_value],
            )
            .unwrap(),
        Value::Bool(false),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_eq!(
        caller.call(
            &has_own,
            Value::Undefined,
            &[
                Value::Null,
                Value::String(JsString::try_from_utf8("x").unwrap()),
            ],
        ),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native ToObject rejection did not use the defining realm",
    );

    let sentinel = caller.new_object().unwrap();
    let sentinel_key = runtime.intern_property_key("hasOwnSentinel").unwrap();
    assert!(
        caller
            .set_property(
                &caller.global_object().unwrap(),
                &sentinel_key,
                Value::Object(sentinel.clone()),
            )
            .unwrap()
    );
    let throwing_key = eval_object(
        &mut caller,
        r#"(function(){
            var key=Object();
            key[Symbol.toPrimitive]=function(){throw hasOwnSentinel};
            return key;
        })()"#,
    );
    let target = caller.new_object().unwrap();
    assert_eq!(
        caller.call(
            &has_own,
            Value::Undefined,
            &[Value::Object(target), Value::Object(throwing_key)],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "property-key conversion throw identity was not preserved",
    );

    assert_eq!(
        caller.construct(&has_own, &[]),
        Err(RuntimeError::Exception),
    );
    let constructor_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&constructor_error).unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn object_has_own_method_is_per_realm_and_retain_then_releases_its_realm() {
    let runtime = Runtime::new();
    let has_own = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_method =
            property_callable(&runtime, &mut first, first_object.as_object(), "hasOwn");
        let first_method_again =
            property_callable(&runtime, &mut first, first_object.as_object(), "hasOwn");
        let second_method =
            property_callable(&runtime, &mut second, second_object.as_object(), "hasOwn");
        assert_eq!(first_method, first_method_again);
        assert_ne!(first_method, second_method);
        assert_eq!(
            runtime.get_prototype_of(first_method.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_method);
        first_method
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(has_own);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn object_has_own_records_current_exotic_object_gap() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("typeof Proxy+'|'+typeof ArrayBuffer+'|'+typeof Uint8Array")
            .unwrap(),
        Value::String(JsString::try_from_utf8("undefined|undefined|undefined").unwrap()),
        "activate the exotic Object.hasOwn oracle vectors as these intrinsics are published",
    );
    // Module namespace objects remain a separate module boundary. Their oracle
    // vectors should join the differential once that object kind exists in the
    // Rust heap.
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
  var value=std.evalScript(scriptArgs[0]);
  print('return|'+typeof value+'|'+String(value));
} catch(error) {
  if(error!==null&&typeof error==='object')print('throw|object|'+error.name+'|'+error.message);
  else print('throw|'+typeof error+'|'+String(error));
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
    let function_prototype = context.function_prototype().unwrap();
    let object = global_callable(&runtime, &mut context, "Object");
    let selected = [
        "length",
        "name",
        "prototype",
        "create",
        "getPrototypeOf",
        "setPrototypeOf",
        "defineProperty",
        "defineProperties",
        "getOwnPropertyNames",
        "getOwnPropertySymbols",
        "groupBy",
        "keys",
        "values",
        "entries",
        "isExtensible",
        "preventExtensions",
        "getOwnPropertyDescriptor",
        "getOwnPropertyDescriptors",
        "is",
        "assign",
        "seal",
        "freeze",
        "isSealed",
        "isFrozen",
        "fromEntries",
        "hasOwn",
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let key = runtime.intern_property_key("hasOwn").unwrap();
    let descriptor = data_descriptor(&runtime, object.as_object(), &key);
    let Value::Object(function) = descriptor.0 else {
        panic!("Object.hasOwn was not an object");
    };
    let callable = runtime.as_callable(&function).unwrap();
    let function_again = property_callable(&runtime, &mut context, object.as_object(), "hasOwn");
    let mut output = vec![
        format!("prefix={prefix}"),
        format!(
            "hasOwn={}:{}:{}:{}:{}:{}:{}",
            string_property(&runtime, &mut context, &function, "name"),
            int_property(&runtime, &mut context, &function, "length"),
            runtime.get_prototype_of(&function).unwrap().as_ref() == Some(&function_prototype),
            callable.is_some(),
            runtime.is_constructor(&function).unwrap(),
            own_key_names(&runtime, &function).join(","),
            data_bits(descriptor.1, descriptor.2, descriptor.3),
        ),
        format!("identity={}", function_again.as_object() == &function),
    ];
    let length = data_descriptor(
        &runtime,
        &function,
        &runtime.intern_property_key("length").unwrap(),
    );
    let function_name = data_descriptor(
        &runtime,
        &function,
        &runtime.intern_property_key("name").unwrap(),
    );
    output.push(format!(
        "hasOwn-props={}:{}",
        data_bits(length.1, length.2, length.3),
        data_bits(function_name.1, function_name.2, function_name.3),
    ));
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_lines(oracle, GRAPH_ORACLE, "Object.hasOwn graph")
}

fn oracle_lines(oracle: &OsStr, source: &str, description: &str) -> Vec<String> {
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

fn global_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, name)
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

fn intrinsic_prototype(
    runtime: &Runtime,
    context: &mut Context,
    constructor_name: &str,
) -> ObjectRef {
    let constructor = global_callable(runtime, context, constructor_name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{constructor_name}.prototype was not an object");
    };
    prototype
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
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

fn data_bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "D:{}{}{}",
        Number(writable),
        Number(enumerable),
        Number(configurable),
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

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
