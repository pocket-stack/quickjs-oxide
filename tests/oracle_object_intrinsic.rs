use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, PropertyKey, Runtime,
    RuntimeError, Value,
};

// This differential pins the first coherent `%Object%` vertical slice to
// QuickJS 2026-06-04. Source cases deliberately use only the selected static
// prefix (`create` through `getOwnPropertySymbols`), the complete
// `Object.prototype` table, and intrinsics which already precede this slice.
// Graph/descriptor reflection uses the host API on the Rust side so the test
// does not accidentally require the later Object.getOwnPropertyDescriptor
// static-table entry merely to observe this milestone.

const CONSTRUCTOR_CASES: &[(&str, &str)] = &[
    (
        "Object call and new allocate ordinary objects",
        r#"(function(){
            var a=Object(),b=new Object(),c=Object(null),d=Object(undefined);
            return [
                Object.getPrototypeOf(a)===Object.prototype,
                Object.getPrototypeOf(b)===Object.prototype,
                Object.getPrototypeOf(c)===Object.prototype,
                Object.getPrototypeOf(d)===Object.prototype,
                a!==b,b!==c,c!==d
            ];
        })()"#,
    ),
    (
        "Object preserves objects and boxes every primitive family",
        r#"(function(){
            var original=Object.create(null),symbol=Symbol("object"),big=7n;
            var number=Object(3),boolean=Object(false),string=Object("A"),
                boxedSymbol=Object(symbol),boxedBig=Object(big);
            return [
                Object(original)===original,new Object(original)===original,
                Object.prototype.toString.call(number),number.valueOf(),
                Object.prototype.toString.call(boolean),boolean.valueOf(),
                Object.prototype.toString.call(string),string.valueOf(),string.length,
                Object.prototype.toString.call(boxedSymbol),boxedSymbol.valueOf()===symbol,
                Object.prototype.toString.call(boxedBig),boxedBig.valueOf()===big
            ];
        })()"#,
    ),
    (
        "Object call ignores this and repeated primitive boxing is fresh",
        r#"(function(){
            var receiver=Object.create(null),a=Object.call(receiver),
                b=Object.call(receiver,1),c=Object.call(receiver,1);
            return [
                Object.getPrototypeOf(a)===Object.prototype,
                Object.prototype.toString.call(b),b!==c,b.valueOf(),c.valueOf()
            ];
        })()"#,
    ),
];

const CREATE_AND_PROTOTYPE_CASES: &[(&str, &str)] = &[
    (
        "Object.create installs prototype and descriptor properties",
        r#"(function(){
            var proto=Object(),descriptors=Object.create(null),x=Object(),y=Object();
            proto.inherited=9;
            x.value=7;x.writable=false;x.enumerable=true;x.configurable=false;
            y.value=8;y.writable=true;y.enumerable=false;y.configurable=true;
            descriptors.x=x;descriptors.y=y;
            var object=Object.create(proto,descriptors),nullObject=Object.create(null);
            var write=(function(){"use strict";try{object.x=10;return "missing"}catch(e){return e.name}})();
            return [
                Object.getPrototypeOf(object)===proto,object.inherited,object.x,object.y,
                object.propertyIsEnumerable("x"),object.propertyIsEnumerable("y"),
                write,delete object.x,delete object.y,
                Object.getPrototypeOf(nullObject)===null
            ];
        })()"#,
    ),
    (
        "Object.create rejects the prototype before reading properties",
        r#"(function(){
            var log="",properties=Object();
            properties.__defineGetter__("x",function(){log+="x";throw 8});
            try{Object.create(1,properties)}catch(error){return log+"|"+error.name+"|"+error.message}
            return "missing";
        })()"#,
    ),
    (
        "Object getPrototypeOf boxes primitives and rejects nullish values",
        r#"(function(){
            var numberPrototype=Object.getPrototypeOf(Object(0));
            var stringPrototype=Object.getPrototypeOf(Object(""));
            var booleanPrototype=Object.getPrototypeOf(Object(false));
            function observe(value){try{return Object.getPrototypeOf(value)===Object.prototype?"object":
                Object.getPrototypeOf(value)===numberPrototype?"number":
                Object.getPrototypeOf(value)===stringPrototype?"string":
                Object.getPrototypeOf(value)===booleanPrototype?"boolean":"other"}
                catch(error){return error.name+":"+error.message}}
            return [observe(Object()),observe(1),observe("x"),observe(false),observe(null),observe(undefined)];
        })()"#,
    ),
    (
        "Object.setPrototypeOf returns the target and handles primitives",
        r#"(function(){
            function observe(thunk){try{return thunk()}catch(error){return error.name+":"+error.message}}
            var first=Object(),second=Object(),fixed=Object.create(null);
            var returned=Object.setPrototypeOf(first,second);
            var same=Object.setPrototypeOf(fixed,null);
            var thrower=Function.prototype.__lookupGetter__("caller");
            return [
                returned===first,Object.getPrototypeOf(first)===second,same===fixed,
                Object.setPrototypeOf(1,second),Object.setPrototypeOf("x",second),
                observe(function(){return Object.setPrototypeOf(null,second)}),
                observe(function(){return Object.setPrototypeOf(Object(),1)}),
                observe(function(){return Object.setPrototypeOf(second,first)}),
                observe(function(){return Object.setPrototypeOf(Object.prototype,Object())}),
                observe(function(){return Object.setPrototypeOf(thrower,Object())})
            ];
        })()"#,
    ),
];

const DEFINE_PROPERTY_CASES: &[(&str, &str)] = &[
    (
        "Object.defineProperty applies data and accessor descriptors",
        r#"(function(){
            var object=Object(),stored=0,data=Object(),accessor=Object();
            data.value=7;data.writable=false;data.enumerable=true;data.configurable=false;
            accessor.get=function(){return stored+1};
            accessor.set=function(value){stored=value};
            accessor.enumerable=false;accessor.configurable=true;
            var dataResult=Object.defineProperty(object,"fixed",data);
            var accessorResult=Object.defineProperty(object,"value",accessor);
            object.value=9;
            var strictWrite=(function(){"use strict";try{object.fixed=8;return "missing"}catch(e){return e.name}})();
            return [
                dataResult===object,accessorResult===object,object.fixed,strictWrite,
                object.propertyIsEnumerable("fixed"),delete object.fixed,
                object.value,stored,object.propertyIsEnumerable("value"),delete object.value
            ];
        })()"#,
    ),
    (
        "property key precedes complete descriptor field conversion",
        r#"(function(){
            var log="",target=Object(),key=Object(),descriptor=Object();
            key.toString=function(){log+="k";return "x"};
            descriptor.__defineGetter__("enumerable",function(){log+="e";return true});
            descriptor.__defineGetter__("configurable",function(){log+="c";return true});
            descriptor.__defineGetter__("value",function(){log+="v";return 1});
            descriptor.__defineGetter__("writable",function(){log+="w";return true});
            descriptor.__defineGetter__("get",function(){log+="g";return function(){}});
            descriptor.__defineGetter__("set",function(){log+="s";return function(){}});
            try{Object.defineProperty(target,key,descriptor)}
            catch(error){return log+"|"+error.name+"|"+error.message+"|"+target.hasOwnProperty("x")}
            return "missing";
        })()"#,
    ),
    (
        "descriptor field throws preserve the pinned accessor overwrite quirk",
        r#"(function(){
            function row(field){
                var descriptor=Object();
                descriptor.__defineGetter__(field,function(){throw "sentinel"});
                try{Object.defineProperty(Object(),"x",descriptor)}
                catch(error){
                    return field+"|"+(typeof error==="object"?error.name+":"+error.message:error);
                }
                return "missing";
            }
            return [row("enumerable"),row("get"),row("set")];
        })()"#,
    ),
    (
        "defineProperty validates target then key then descriptor",
        r#"(function(){
            function row(mode){
                var log="",key=Object(),descriptor=Object();
                key.toString=function(){log+="k";if(mode===1)throw "key";return "x"};
                descriptor.__defineGetter__("value",function(){log+="v";throw "descriptor"});
                try{
                    if(mode===0)Object.defineProperty(null,key,descriptor);
                    else if(mode===2)Object.defineProperty(Object(),key,1);
                    else Object.defineProperty(Object(),key,descriptor);
                }catch(error){return log+":"+(typeof error==="object"?error.name:error)}
                return "missing";
            }
            return [row(0),row(1),row(2),row(3)];
        })()"#,
    ),
    (
        "QuickJS defineProperties converts and defines sequentially",
        r#"(function(){
            var log="",target=Object(),descriptors=Object(),a=Object();
            a.value=1;a.writable=true;a.enumerable=true;a.configurable=true;
            descriptors.a=a;
            descriptors.__defineGetter__("b",function(){log+="b";throw "stop"});
            try{Object.defineProperties(target,descriptors)}catch(error){
                return [log,error,target.hasOwnProperty("a"),target.a,target.hasOwnProperty("b")];
            }
            return "missing";
        })()"#,
    ),
    (
        "Object.create shares defineProperties partial failure behavior",
        r#"(function(){
            var log="",descriptors=Object(),a=Object();
            a.value=1;a.writable=true;a.enumerable=true;a.configurable=true;
            descriptors.a=a;
            descriptors.__defineGetter__("b",function(){log+="b";throw 23});
            try{Object.create(null,descriptors)}catch(error){return log+"|"+error}
            return "missing";
        })()"#,
    ),
];

const OWN_KEY_ARRAY_CASES: &[(&str, &str)] = &[
    (
        "own property name and symbol arrays partition ordinary keys",
        r#"(function(){
            function list(array){var text="";for(var i=0;i<array.length;i++){if(i)text+=",";text+=array[i]}return text}
            var first=Symbol("first"),second=Symbol("second"),object=Object(),hidden=Object();
            object.beta=1;object["4294967295"]=2;object["2"]=3;object["0"]=4;
            hidden.value=5;Object.defineProperty(object,"hidden",hidden);
            object[first]=6;object.alpha=7;object[second]=8;
            var names=Object.getOwnPropertyNames(object),symbols=Object.getOwnPropertySymbols(object);
            return [
                Array.isArray(names),Object.getPrototypeOf(names)===Array.prototype,list(names),
                Array.isArray(symbols),Object.getPrototypeOf(symbols)===Array.prototype,
                symbols.length,symbols[0]===first,symbols[1]===second,
                list(Object.getOwnPropertyNames(names)),list(Object.getOwnPropertyNames(symbols))
            ];
        })()"#,
    ),
    (
        "Array exotic own keys become dense key arrays",
        r#"(function(){
            function list(array){var text="";for(var i=0;i<array.length;i++){if(i)text+=",";text+=array[i]}return text}
            var symbol=Symbol("array"),array=[10,,30];array.foo=4;array[symbol]=5;
            var names=Object.getOwnPropertyNames(array),symbols=Object.getOwnPropertySymbols(array);
            return [
                list(names),names.length,0 in names,1 in names,2 in names,3 in names,
                symbols.length,symbols[0]===symbol,
                list(Object.getOwnPropertyNames(names)),list(Object.getOwnPropertyNames(symbols))
            ];
        })()"#,
    ),
    (
        "String exotic virtual keys merge with stored strings and symbols",
        r#"(function(){
            function list(array){var text="";for(var i=0;i<array.length;i++){if(i)text+=",";text+=array[i]}return text}
            var symbol=Symbol("string"),wrapper=Object("A\uD83D\uDCA9\uD800");
            wrapper[8]=8;wrapper.foo=9;wrapper[symbol]=10;
            var names=Object.getOwnPropertyNames(wrapper),symbols=Object.getOwnPropertySymbols(wrapper);
            var primitiveNames=Object.getOwnPropertyNames("A\uD83D\uDCA9");
            return [
                list(names),list(primitiveNames),symbols.length,symbols[0]===symbol,
                Array.isArray(names),Object.getPrototypeOf(names)===Array.prototype,
                list(Object.getOwnPropertyNames(names))
            ];
        })()"#,
    ),
    (
        "own key statics box primitives and reject nullish values",
        r#"(function(){
            function observe(thunk){try{var value=thunk();return value.length}catch(error){return error.name+":"+error.message}}
            return [
                observe(function(){return Object.getOwnPropertyNames(1)}),
                observe(function(){return Object.getOwnPropertySymbols(1)}),
                observe(function(){return Object.getOwnPropertyNames(null)}),
                observe(function(){return Object.getOwnPropertySymbols(undefined)})
            ];
        })()"#,
    ),
];

const PROTOTYPE_SUFFIX_CASES: &[(&str, &str)] = &[
    (
        "core Object prototype methods preserve nullish behavior",
        r#"(function(){
            function observe(method,value){
                try{return method.call(value)}catch(error){return error.name+":"+error.message}
            }
            return [
                observe(Object.prototype.toString,null),
                observe(Object.prototype.toString,undefined),
                observe(Object.prototype.toLocaleString,null),
                observe(Object.prototype.toLocaleString,undefined),
                observe(Object.prototype.valueOf,null),
                observe(Object.prototype.valueOf,undefined)
            ];
        })()"#,
    ),
    (
        "hasOwnProperty and propertyIsEnumerable distinguish own descriptors",
        r#"(function(){
            var symbol=Symbol("own"),parent=Object(),hidden=Object();parent.inherited=1;
            var object=Object.create(parent);object.visible=2;hidden.value=3;
            Object.defineProperty(object,"hidden",hidden);object[symbol]=4;
            return [
                object.hasOwnProperty("visible"),object.hasOwnProperty("hidden"),
                object.hasOwnProperty("inherited"),object.hasOwnProperty(symbol),
                object.propertyIsEnumerable("visible"),object.propertyIsEnumerable("hidden"),
                object.propertyIsEnumerable("inherited"),object.propertyIsEnumerable(symbol)
            ];
        })()"#,
    ),
    (
        "property probes convert keys before nullish receivers",
        r#"(function(){
            function run(method){
                var log="",key=Object();key.toString=function(){log+="k";throw 17};
                try{method.call(null,key)}catch(error){return log+":"+error}
                return "missing";
            }
            return [run(Object.prototype.hasOwnProperty),run(Object.prototype.propertyIsEnumerable)];
        })()"#,
    ),
    (
        "isPrototypeOf walks chains and short circuits primitive candidates",
        r#"(function(){
            function observe(thunk){try{return thunk()}catch(error){return error.name+":"+error.message}}
            var root=Object(),middle=Object.create(root),leaf=Object.create(middle);
            return [
                root.isPrototypeOf(leaf),middle.isPrototypeOf(leaf),leaf.isPrototypeOf(root),
                Object.prototype.isPrototypeOf.call(null,1),
                observe(function(){return Object.prototype.isPrototypeOf.call(null,leaf)})
            ];
        })()"#,
    ),
    (
        "legacy __proto__ getter and setter preserve pinned validation",
        r#"(function(){
            function observe(thunk){try{return thunk()}catch(error){return error.name+":"+error.message}}
            var parent=Object(),object=Object(),getter=Object.prototype.__lookupGetter__("__proto__"),
                setter=Object.prototype.__lookupSetter__("__proto__");
            var first=setter.call(object,parent),after=object.__proto__===parent;
            var ignored=setter.call(object,1),still=object.__proto__===parent;
            return [
                first,after,ignored,still,getter.call(object)===parent,
                getter.call(1)===Number.prototype,setter.call(1,parent),
                observe(function(){return getter.call(null)}),
                observe(function(){return setter.call(null,parent)}),
                observe(function(){return setter.call(object,object)})
            ];
        })()"#,
    ),
    (
        "legacy accessor definition and lookup walk the prototype chain",
        r#"(function(){
            var stored=0,parent=Object(),getter=function(){return 41},setter=function(value){stored=value};
            parent.__defineGetter__("answer",getter);parent.__defineSetter__("answer",setter);
            var child=Object.create(parent),before=child.answer;child.answer=9;
            var inheritedGet=child.__lookupGetter__("answer"),inheritedSet=child.__lookupSetter__("answer");
            var data=Object();data.value=3;data.writable=true;data.configurable=true;
            Object.defineProperty(child,"answer",data);
            return [
                before,stored,inheritedGet===getter,inheritedSet===setter,
                parent.propertyIsEnumerable("answer"),
                child.__lookupGetter__("answer"),child.__lookupSetter__("answer"),child.answer
            ];
        })()"#,
    ),
    (
        "legacy accessor helpers preserve receiver callable and key order",
        r#"(function(){
            function row(mode){
                var log="",receiver=Object(),key=Object(),fn=function(){};
                key.toString=function(){log+="k";throw "key"};
                try{
                    if(mode===0)Object.prototype.__defineGetter__.call(null,key,fn);
                    if(mode===1)receiver.__defineGetter__(key,1);
                    if(mode===2)receiver.__defineGetter__(key,fn);
                    if(mode===3)Object.prototype.__lookupGetter__.call(null,key);
                    if(mode===4)receiver.__lookupGetter__(key);
                }catch(error){return log+":"+(typeof error==="object"?error.name:error)}
                return "missing";
            }
            return [row(0),row(1),row(2),row(3),row(4)];
        })()"#,
    ),
];

const STATIC_PREFIX: &[&str] = &[
    "create",
    "getPrototypeOf",
    "setPrototypeOf",
    "defineProperty",
    "defineProperties",
    "getOwnPropertyNames",
    "getOwnPropertySymbols",
];

const PROTOTYPE_DATA_METHODS: &[&str] = &[
    "toString",
    "toLocaleString",
    "valueOf",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
];

const GRAPH_ORACLE: &str = r#"
function bits(descriptor) {
    if ("value" in descriptor)
        return "D:" + Number(descriptor.writable) + Number(descriptor.enumerable) + Number(descriptor.configurable);
    return "A:" + Number(descriptor.enumerable) + Number(descriptor.configurable);
}
function isConstructor(value) {
    try { Reflect.construct(function(){}, [], value); return true; }
    catch (_) { return false; }
}
function callableMeta(value) {
    return value.name + ":" + value.length + ":" +
           (Object.getPrototypeOf(value) === Function.prototype) + ":" +
           (typeof value === "function") + ":" + isConstructor(value);
}
function methodMeta(owner, key) {
    var descriptor = Object.getOwnPropertyDescriptor(owner, key);
    return String(key) + ":" + callableMeta(descriptor.value) + ":" + bits(descriptor);
}
var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "Object");
var prototypeDescriptor = Object.getOwnPropertyDescriptor(Object, "prototype");
var constructorDescriptor = Object.getOwnPropertyDescriptor(Object.prototype, "constructor");
print("graph=" + [
    globalDescriptor.value === Object,
    prototypeDescriptor.value === Object.prototype,
    constructorDescriptor.value === Object,
    Object.getPrototypeOf(Object) === Function.prototype,
    Object.getPrototypeOf(Object.prototype) === null,
    bits(globalDescriptor),bits(prototypeDescriptor),bits(constructorDescriptor)
].join("|"));
var selected = ["length","name","prototype","create","getPrototypeOf","setPrototypeOf",
                "defineProperty","defineProperties","getOwnPropertyNames","getOwnPropertySymbols"];
print("ctor-prefix=" + Reflect.ownKeys(Object).filter(function(key) {
    return selected.indexOf(key) >= 0;
}).map(String).join(","));
print("proto-keys=" + Reflect.ownKeys(Object.prototype).map(String).join(","));
print("static-meta=" + ["create","getPrototypeOf","setPrototypeOf","defineProperty",
    "defineProperties","getOwnPropertyNames","getOwnPropertySymbols"].map(function(key) {
        return methodMeta(Object,key);
    }).join("|"));
print("proto-meta=" + ["toString","toLocaleString","valueOf","hasOwnProperty","isPrototypeOf",
    "propertyIsEnumerable","__defineGetter__","__defineSetter__","__lookupGetter__",
    "__lookupSetter__"].map(function(key) { return methodMeta(Object.prototype,key); }).join("|"));
var protoDescriptor = Object.getOwnPropertyDescriptor(Object.prototype,"__proto__");
print("proto-accessor=__proto__:" + bits(protoDescriptor) + ":" +
      callableMeta(protoDescriptor.get) + ":" + callableMeta(protoDescriptor.set));
"#;

#[test]
fn object_intrinsic_graph_and_descriptors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle)
    );
}

#[test]
fn object_constructor_values_match_pinned_quickjs() {
    compare_value_cases("Object constructor", CONSTRUCTOR_CASES);
}

#[test]
fn object_create_and_prototype_statics_match_pinned_quickjs() {
    compare_value_cases(
        "Object create/prototype statics",
        CREATE_AND_PROTOTYPE_CASES,
    );
}

#[test]
fn object_descriptor_conversion_matches_pinned_quickjs() {
    compare_value_cases("Object descriptor conversion", DEFINE_PROPERTY_CASES);
}

#[test]
fn object_own_key_arrays_match_pinned_quickjs() {
    compare_value_cases("Object own-key arrays", OWN_KEY_ARRAY_CASES);
}

#[test]
fn complete_object_prototype_suffix_matches_pinned_quickjs() {
    compare_value_cases("Object.prototype suffix", PROTOTYPE_SUFFIX_CASES);
}

#[test]
fn object_oracle_vectors_execute_on_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("constructor", CONSTRUCTOR_CASES),
        ("create/prototype", CREATE_AND_PROTOTYPE_CASES),
        ("descriptors", DEFINE_PROPERTY_CASES),
        ("own keys", OWN_KEY_ARRAY_CASES),
        ("prototype suffix", PROTOTYPE_SUFFIX_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion: {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 6);
}

#[test]
fn object_constructor_custom_new_target_and_cross_realm_fallback_are_pinned() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let constructor = global_callable(&runtime, &mut defining, "Object");
    let defining_object_prototype = defining.object_prototype().unwrap();
    let caller_object_prototype = caller.object_prototype().unwrap();
    let defining_boolean_prototype = defining.boolean_prototype().unwrap();
    let defining_array_prototype = defining.array_prototype().unwrap();

    let Value::Object(called) = caller
        .call(&constructor, Value::Undefined, &[])
        .expect("cross-realm Object call")
    else {
        panic!("Object() did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&called).unwrap().as_ref(),
        Some(&defining_object_prototype),
        "Object() did not allocate in its defining realm",
    );

    let Value::Object(constructed) = caller
        .construct(&constructor, &[])
        .expect("cross-realm new Object")
    else {
        panic!("new Object() did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&constructed).unwrap().as_ref(),
        Some(&defining_object_prototype),
        "new Object() did not allocate in its defining realm",
    );

    let Value::Object(boxed_boolean) = caller
        .call(&constructor, Value::Undefined, &[Value::Bool(true)])
        .expect("cross-realm Object Boolean boxing")
    else {
        panic!("Object(true) did not return a wrapper");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed_boolean).unwrap().as_ref(),
        Some(&defining_boolean_prototype),
        "Object primitive boxing did not use the constructor's defining realm",
    );

    let target = eval_callable(&runtime, &mut caller, "(function ForeignObjectTarget(){})");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let custom_prototype = caller.new_object().unwrap();
    assert!(
        caller
            .set_property(
                target.as_object(),
                &prototype_key,
                Value::Object(custom_prototype.clone()),
            )
            .unwrap()
    );
    let Value::Object(custom) = caller
        .construct_with_new_target(&constructor, &target, &[Value::Int(7)])
        .expect("Object custom new.target prototype")
    else {
        panic!("Object custom new.target did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom).unwrap().as_ref(),
        Some(&custom_prototype),
        "Object custom new.target ignored its object prototype",
    );

    assert!(
        caller
            .set_property(target.as_object(), &prototype_key, Value::Int(1))
            .unwrap()
    );
    let Value::Object(fallback) = caller
        .construct_with_new_target(&constructor, &target, &[])
        .expect("Object custom new.target fallback")
    else {
        panic!("Object custom new.target fallback did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&fallback).unwrap().as_ref(),
        Some(&caller_object_prototype),
        "non-object new.target.prototype did not fall back to new.target's realm",
    );

    let names = property_callable(
        &runtime,
        &mut defining,
        constructor.as_object(),
        "getOwnPropertyNames",
    );
    let sample = caller.new_object().unwrap();
    let Value::Object(name_array) = caller
        .call(&names, Value::Undefined, &[Value::Object(sample.clone())])
        .expect("cross-realm Object.getOwnPropertyNames")
    else {
        panic!("Object.getOwnPropertyNames did not return an Array");
    };
    assert_eq!(
        runtime.get_prototype_of(&name_array).unwrap().as_ref(),
        Some(&defining_array_prototype),
        "Object own-key result Array did not use the native's defining realm",
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
        Ok(Value::Object(object)) if runtime.is_array_object(&object).unwrap() => format!(
            "return|array|{}",
            array_value_text(runtime, context, &object, description),
        ),
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
  if (Array.isArray(value)) {
    var text = '';
    for (var index = 0; index < value.length; index++) {
      if (index) text += ',';
      text += String(value[index]);
    }
    print('return|array|' + text);
  } else {
    print('return|' + typeof value + '|' + String(value));
  }
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

fn array_value_text(
    runtime: &Runtime,
    context: &mut Context,
    array: &ObjectRef,
    description: &str,
) -> String {
    let length_key = runtime.intern_property_key("length").unwrap();
    let length = context
        .get_property(array, &length_key)
        .unwrap_or_else(|error| panic!("read result Array length for {description}: {error}"));
    let length = match length {
        Value::Int(value) if value >= 0 => value as usize,
        Value::Float(value) if value >= 0.0 && value.fract() == 0.0 => value as usize,
        value => panic!(
            "result Array length for {description} was invalid: {}",
            primitive_value_text(value),
        ),
    };
    (0..length)
        .map(|index| {
            let key = runtime.intern_property_key(&index.to_string()).unwrap();
            let value = context.get_property(array, &key).unwrap_or_else(|error| {
                panic!("read result Array[{index}] for {description}: {error}")
            });
            primitive_value_text(value)
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn rust_graph_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let object_prototype = context.object_prototype().unwrap();
    let object_key = runtime.intern_property_key("Object").unwrap();
    let global_descriptor = data_descriptor(&runtime, &global, &object_key);
    let Value::Object(constructor_object) = global_descriptor.0.clone() else {
        panic!("global Object descriptor did not contain an object");
    };
    let constructor = runtime
        .as_callable(&constructor_object)
        .unwrap()
        .expect("global Object was not callable");
    let prototype_key = runtime.intern_property_key("prototype").unwrap();
    let prototype_descriptor = data_descriptor(&runtime, &constructor_object, &prototype_key);
    let constructor_key = runtime.intern_property_key("constructor").unwrap();
    let back_descriptor = data_descriptor(&runtime, &object_prototype, &constructor_key);
    let mut output = vec![format!(
        "graph={}|{}|{}|{}|{}|{}|{}|{}",
        matches!(&global_descriptor.0, Value::Object(value) if value == &constructor_object),
        matches!(&prototype_descriptor.0, Value::Object(value) if value == &object_prototype),
        matches!(&back_descriptor.0, Value::Object(value) if value == &constructor_object),
        runtime
            .get_prototype_of(&constructor_object)
            .unwrap()
            .as_ref()
            == Some(&function_prototype),
        runtime
            .get_prototype_of(&object_prototype)
            .unwrap()
            .is_none(),
        data_bits(
            global_descriptor.1,
            global_descriptor.2,
            global_descriptor.3
        ),
        data_bits(
            prototype_descriptor.1,
            prototype_descriptor.2,
            prototype_descriptor.3,
        ),
        data_bits(back_descriptor.1, back_descriptor.2, back_descriptor.3),
    )];

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
    ];
    let constructor_prefix = own_key_names(&runtime, &constructor_object)
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    output.push(format!("ctor-prefix={constructor_prefix}"));
    output.push(format!(
        "proto-keys={}",
        own_key_names(&runtime, &object_prototype).join(","),
    ));
    output.push(format!(
        "static-meta={}",
        STATIC_PREFIX
            .iter()
            .map(|name| method_meta(
                &runtime,
                &mut context,
                &constructor_object,
                &function_prototype,
                name,
            ))
            .collect::<Vec<_>>()
            .join("|"),
    ));
    output.push(format!(
        "proto-meta={}",
        PROTOTYPE_DATA_METHODS
            .iter()
            .map(|name| method_meta(
                &runtime,
                &mut context,
                &object_prototype,
                &function_prototype,
                name,
            ))
            .collect::<Vec<_>>()
            .join("|"),
    ));

    let proto_key = runtime.intern_property_key("__proto__").unwrap();
    let CompleteOrdinaryPropertyDescriptor::Accessor {
        get: Some(getter),
        set: Some(setter),
        enumerable,
        configurable,
    } = runtime
        .get_own_property(&object_prototype, &proto_key)
        .unwrap()
        .expect("Object.prototype.__proto__ was absent")
    else {
        panic!("Object.prototype.__proto__ was not a complete accessor");
    };
    output.push(format!(
        "proto-accessor=__proto__:A:{}{}:{}:{}",
        Number(enumerable),
        Number(configurable),
        callable_meta(&runtime, &mut context, &getter, &function_prototype),
        callable_meta(&runtime, &mut context, &setter, &function_prototype),
    ));
    let _ = constructor;
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS Object graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Object graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Object graph output was not UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn method_meta(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    function_prototype: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let (value, writable, enumerable, configurable) = data_descriptor(runtime, owner, &key);
    let Value::Object(function) = value else {
        panic!("{name} was not an object");
    };
    let callable = runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"));
    format!(
        "{name}:{}:{}",
        callable_meta(runtime, context, &callable, function_prototype),
        data_bits(writable, enumerable, configurable),
    )
}

fn callable_meta(
    runtime: &Runtime,
    context: &mut Context,
    callable: &CallableRef,
    function_prototype: &ObjectRef,
) -> String {
    let name_key = runtime.intern_property_key("name").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let name = context
        .get_property(callable.as_object(), &name_key)
        .expect("read callable name");
    let length = context
        .get_property(callable.as_object(), &length_key)
        .expect("read callable length");
    format!(
        "{}:{}:{}:true:{}",
        primitive_value_text(name),
        primitive_value_text(length),
        runtime
            .get_prototype_of(callable.as_object())
            .unwrap()
            .as_ref()
            == Some(function_prototype),
        runtime.is_constructor(callable.as_object()).unwrap(),
    )
}

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (Value, bool, bool, bool) {
    let descriptor = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("expected own property descriptor");
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = descriptor
    else {
        panic!("expected a data property descriptor");
    };
    (value, writable, enumerable, configurable)
}

fn data_bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "D:{}{}{}",
        Number(writable),
        Number(enumerable),
        Number(configurable),
    )
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

fn global_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, name)
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

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(function) = context.eval(source).expect("evaluate callable") else {
        panic!("callable source did not return an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .expect("evaluated object was not callable")
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
