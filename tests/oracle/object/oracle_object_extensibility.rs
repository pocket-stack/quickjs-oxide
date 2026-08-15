use crate::object_graph_observation::{
    data_bits, data_descriptor, global_callable, int_property, intrinsic_prototype, oracle_lines,
    own_key_names,
};
use crate::quickjs_oracle::observe_completion as observe_oracle;
use crate::runtime_completion_oracle::compare_eval_completion_cases as compare_cases;
use crate::runtime_observation::{
    property_callable, string_property, take_pending_exception_object as take_exception_object,
};
use crate::runtime_oracle::value_type;
use std::ffi::OsStr;

use quickjs_oxide::{Runtime, RuntimeError, Value};

// Pins QuickJS 2026-06-04 `js_object_isExtensible` and
// `js_object_preventExtensions`. Proxy [[IsExtensible]]/[[PreventExtensions]]
// traps and Resizable ArrayBuffer-backed TypedArrays participate in the same
// differential.

const PRIMITIVE_CASES: &[(&str, &str)] = &[
    (
        "primitive identity including nullish Symbol BigInt minus zero and NaN",
        r#"(function(){
            function sameValue(a,b){return a===b?(a!==0||1/a===1/b):(a!==a&&b!==b)}
            var symbol=Symbol("identity"),bigint=9007199254740993n,minusZero=-0,nan=NaN;
            var values=[undefined,null,false,17,"oxide",bigint,symbol,minusZero,nan],out=[];
            for(var i=0;i<values.length;i++){
                var value=values[i],returned=Object.preventExtensions(value);
                out[out.length]=typeof value+":"+sameValue(returned,value)+":"+Object.isExtensible(value);
            }
            return out.join("|")+"|missing:"+sameValue(Object.preventExtensions(),undefined)+":"+Object.isExtensible();
        })()"#,
    ),
    (
        "primitive calls do not box or consult wrapper prototypes",
        r#"(function(){
            Number.prototype.marker=1;Boolean.prototype.marker=2;
            BigInt.prototype.marker=4;Symbol.prototype.marker=5;
            var symbol=Symbol("plain");
            return [Object.preventExtensions(3)===3,Object.preventExtensions(true)===true,
                Object.preventExtensions("x")==="x",Object.preventExtensions(8n)===8n,
                Object.preventExtensions(symbol)===symbol,
                Object.isExtensible(3),Object.isExtensible(true),Object.isExtensible("x"),
                Object.isExtensible(8n),Object.isExtensible(symbol)].join(":");
        })()"#,
    ),
];

const OBJECT_CASES: &[(&str, &str)] = &[
    (
        "ordinary Array String wrapper Function and Error objects share the operation",
        r#"(function(){
            function sample(){}
            var values=[Object(),[1,2],Object("ab"),sample,new Error("oxide")],labels=["object","array","string","function","error"],out=[];
            for(var i=0;i<values.length;i++){
                var value=values[i],before=Object.isExtensible(value);
                var first=Object.preventExtensions(value),second=Object.preventExtensions(value);
                out[out.length]=labels[i]+":"+before+":"+(first===value)+":"+(second===value)+":"+Object.isExtensible(value);
            }
            return out.join("|")+"|payload:"+values[1].join(",")+":"+values[2][0]+values[2][1]+":"+
                (values[3]===sample)+":"+values[4].message;
        })()"#,
    ),
    (
        "prevention is identity preserving and idempotent across aliases",
        r#"(function(){
            var value=Object(),alias=value;value.x=1;
            var a=Object.preventExtensions(value),b=Object.preventExtensions(alias),c=Object.preventExtensions(a);
            return (a===value)+":"+(b===value)+":"+(c===value)+":"+(a===b)+":"+
                Object.isExtensible(value)+":"+Object.isExtensible(alias)+":"+Object.keys(value).join(",");
        })()"#,
    ),
    (
        "non-extensible Object materializes an existing AutoInit and global state persists",
        r#"(function(){
            var prevent=Object.preventExtensions,objectConstructor=Object;
            var sameObject=prevent(objectConstructor)===objectConstructor;
            var isExtensible=Object.isExtensible;
            var lazyMaterialized=typeof isExtensible==="function";
            var objectState=isExtensible(objectConstructor);
            globalThis.extensibilityExisting=1;
            var sameGlobal=prevent(globalThis)===globalThis;
            globalThis.extensibilityExisting=2;
            var strictAdd;
            try{(function(){"use strict";globalThis.extensibilityLate=3})()}
            catch(error){strictAdd=error.name+":"+error.message}
            return [sameObject,lazyMaterialized,objectState,sameGlobal,
                isExtensible(globalThis),globalThis.extensibilityExisting,
                globalThis.hasOwnProperty("extensibilityLate")].join(":")+"|"+strictAdd;
        })()"#,
    ),
];

const MUTATION_CASES: &[(&str, &str)] = &[
    (
        "existing properties update and delete while additions and prototype changes reject",
        r#"(function(){
            var firstProto=Object(),secondProto=Object();firstProto.first=true;secondProto.second=true;
            var object=Object.create(firstProto);
            var keptDescriptor=Object();keptDescriptor.value=1;keptDescriptor.writable=true;keptDescriptor.enumerable=true;keptDescriptor.configurable=true;
            var removedDescriptor=Object();removedDescriptor.value=2;removedDescriptor.writable=true;removedDescriptor.enumerable=true;removedDescriptor.configurable=true;
            Object.defineProperty(object,"kept",keptDescriptor);
            Object.defineProperty(object,"removed",removedDescriptor);
            Object.preventExtensions(object);
            object.kept=3;
            var deleted=delete object.removed;
            object.sloppy=4;
            var strictAdd,defineAdd,differentPrototype;
            try{(function(){"use strict";object.strict=5})()}catch(error){strictAdd=error.name+":"+error.message}
            var addedDescriptor=Object();addedDescriptor.value=6;
            try{Object.defineProperty(object,"defined",addedDescriptor)}catch(error){defineAdd=error.name+":"+error.message}
            var samePrototype=Object.setPrototypeOf(object,firstProto)===object;
            try{Object.setPrototypeOf(object,secondProto)}catch(error){differentPrototype=error.name+":"+error.message}
            var updateDescriptor=Object();updateDescriptor.value=7;Object.defineProperty(object,"kept",updateDescriptor);
            return Object.isExtensible(object)+":"+object.kept+":"+deleted+":"+
                object.hasOwnProperty("removed")+":"+object.hasOwnProperty("sloppy")+":"+
                object.hasOwnProperty("strict")+":"+object.hasOwnProperty("defined")+":"+
                samePrototype+":"+(Object.getPrototypeOf(object)===firstProto)+"|"+
                strictAdd+"|"+defineAdd+"|"+differentPrototype;
        })()"#,
    ),
    (
        "Array and String exotic existing surfaces survive prevention",
        r#"(function(){
            var array=["a","b"],string=Object("xy");
            array.extra=1;string.extra=2;
            Object.preventExtensions(array);Object.preventExtensions(string);
            array[0]="A";array.extra=3;string.extra=4;
            var arrayDelete=delete array[1],stringDelete=delete string.extra;
            array[2]="C";string.late="L";
            return Object.isExtensible(array)+":"+array.length+":"+array[0]+":"+arrayDelete+":"+
                array.hasOwnProperty(1)+":"+array.hasOwnProperty(2)+":"+array.extra+"|"+
                Object.isExtensible(string)+":"+string[0]+string[1]+":"+stringDelete+":"+
                string.hasOwnProperty("extra")+":"+string.hasOwnProperty("late");
        })()"#,
    ),
];

const EXOTIC_CASES: &[(&str, &str)] = &[
    (
        "Proxy isExtensible forwards the trap and preserves its order",
        r#"(function(){
            var log="",base=Object();
            var proxy=new Proxy(base,{isExtensible:function(target){
                log+="is|";return Reflect.isExtensible(target);
            }});
            return Object.isExtensible(proxy)+"|"+log;
        })()"#,
    ),
    (
        "Proxy preventExtensions forwards the trap and returns the proxy",
        r#"(function(){
            var log="",base=Object();
            var proxy=new Proxy(base,{preventExtensions:function(target){
                log+="prevent|";return Reflect.preventExtensions(target);
            }});
            return (Object.preventExtensions(proxy)===proxy)+"|"+
                Object.isExtensible(base)+"|"+log;
        })()"#,
    ),
    (
        "Proxy isExtensible invariant rejects a result inconsistent with its target",
        r#"(function(){
            var log="",base=Object();
            var proxy=new Proxy(base,{isExtensible:function(){
                log+="is|";return false;
            }});
            try{Object.isExtensible(proxy)}catch(error){
                return log+error.name+":"+error.message;
            }
            return "missing";
        })()"#,
    ),
    (
        "Resizable ArrayBuffer backed TypedArray rejects preventExtensions",
        r#"(function(){
            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var value=new Uint8Array(buffer);
            try{Object.preventExtensions(value)}catch(error){
                return "prevent-throw|"+error.name+"|"+error.message+"|"+
                    Object.isExtensible(value);
            }
            return "missing";
        })()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
function bits(descriptor){return 'D:'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable)}
function isConstructor(value){try{Reflect.construct(function(){},[],value);return true}catch(_){return false}}
function meta(name){
  var descriptor=Object.getOwnPropertyDescriptor(Object,name),value=descriptor.value;
  return value.name+':'+value.length+':' +(Object.getPrototypeOf(value)===Function.prototype)+':' +
    (typeof value==='function')+':'+isConstructor(value)+':' +Object.getOwnPropertyNames(value).join(',')+':'+bits(descriptor);
}
var selected=['length','name','prototype','create','getPrototypeOf','setPrototypeOf','defineProperty',
  'defineProperties','getOwnPropertyNames','getOwnPropertySymbols','groupBy','keys','values','entries',
  'isExtensible','preventExtensions'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('isExtensible='+meta('isExtensible'));
print('preventExtensions='+meta('preventExtensions'));
print('identity='+(Object.isExtensible===Object.isExtensible)+':' +
  (Object.preventExtensions===Object.preventExtensions)+':' +(Object.isExtensible!==Object.preventExtensions));
['isExtensible','preventExtensions'].forEach(function(name){var fn=Object[name];
  print(name+'-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
});
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var a=delete Object.isExtensible,b=delete Object.preventExtensions;
print([a,b,'isExtensible' in Object,'preventExtensions' in Object,
  typeof Object.isExtensible,typeof Object.preventExtensions].join('|'));
"#;

#[test]
fn object_extensibility_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object extensibility oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("primitive", PRIMITIVE_CASES),
        ("object", OBJECT_CASES),
        ("mutation", MUTATION_CASES),
        ("exotic", EXOTIC_CASES),
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
fn object_extensibility_primitives_match_pinned_quickjs() {
    compare_cases("Object extensibility primitives", PRIMITIVE_CASES);
}

#[test]
fn object_extensibility_object_families_match_pinned_quickjs() {
    compare_cases("Object extensibility object families", OBJECT_CASES);
}

#[test]
fn object_extensibility_mutations_match_pinned_quickjs() {
    compare_cases("Object extensibility mutations", MUTATION_CASES);
}

#[test]
fn object_extensibility_exotic_surfaces_match_pinned_quickjs() {
    compare_cases("Object extensibility exotic surfaces", EXOTIC_CASES);
}

#[test]
fn object_extensibility_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object extensibility graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle)
    );
}

#[test]
fn object_extensibility_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object extensibility AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(
        &oracle,
        FRESH_DELETE_ORACLE,
        "Object extensibility fresh delete",
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let mut values = Vec::new();
    for name in ["isExtensible", "preventExtensions"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .delete_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["isExtensible", "preventExtensions"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .has_own_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["isExtensible", "preventExtensions"] {
        let key = runtime.intern_property_key(name).unwrap();
        let value = context.get_property(object.as_object(), &key).unwrap();
        values.push(value_type(&runtime, &value).to_owned());
    }
    assert_eq!(vec![values.join("|")], expected);
}

#[test]
fn object_extensibility_cross_realm_calls_and_constructor_error_realm_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let is_extensible = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "isExtensible",
    );
    let prevent_extensions = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "preventExtensions",
    );
    let source = caller.new_object().unwrap();

    assert_eq!(
        caller
            .call(
                &is_extensible,
                Value::Undefined,
                &[Value::Object(source.clone())],
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        caller
            .call(
                &prevent_extensions,
                Value::Undefined,
                &[Value::Object(source.clone())],
            )
            .unwrap(),
        Value::Object(source.clone()),
    );
    assert_eq!(
        caller
            .call(&is_extensible, Value::Undefined, &[Value::Object(source)],)
            .unwrap(),
        Value::Bool(false),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    for method in [&is_extensible, &prevent_extensions] {
        assert_eq!(caller.construct(method, &[]), Err(RuntimeError::Exception));
        let error = take_exception_object(&mut caller);
        assert_eq!(
            runtime.get_prototype_of(&error).unwrap(),
            Some(caller_type_error.clone()),
            "non-constructor rejection must use the caller realm",
        );
    }
}

#[test]
fn object_extensibility_methods_are_per_realm_and_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (is_extensible, prevent_extensions) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_is = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "isExtensible",
        );
        let first_is_again = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "isExtensible",
        );
        let first_prevent = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "preventExtensions",
        );
        let second_is = property_callable(
            &runtime,
            &mut second,
            second_object.as_object(),
            "isExtensible",
        );
        assert_eq!(first_is, first_is_again);
        assert_ne!(first_is, first_prevent);
        assert_ne!(first_is, second_is);
        assert_eq!(
            runtime.get_prototype_of(first_is.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_is);
        (first_is, first_prevent)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(is_extensible);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(prevent_extensions);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
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
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut output = vec![format!("prefix={prefix}")];
    let mut methods = Vec::new();
    for name in ["isExtensible", "preventExtensions"] {
        let key = runtime.intern_property_key(name).unwrap();
        let descriptor = data_descriptor(&runtime, object.as_object(), &key);
        let Value::Object(function) = descriptor.0 else {
            panic!("Object.{name} was not an object");
        };
        let callable = runtime.as_callable(&function).unwrap();
        assert!(callable.is_some());
        methods.push(function.clone());
        output.push(format!(
            "{name}={}:{}:{}:{}:{}:{}:{}",
            string_property(&runtime, &mut context, &function, "name"),
            int_property(&runtime, &mut context, &function, "length"),
            runtime.get_prototype_of(&function).unwrap().as_ref() == Some(&function_prototype),
            callable.is_some(),
            runtime.is_constructor(&function).unwrap(),
            own_key_names(&runtime, &function).join(","),
            data_bits(descriptor.1, descriptor.2, descriptor.3),
        ));
    }
    output.push(format!("identity=true:true:{}", methods[0] != methods[1],));
    for (name, function) in ["isExtensible", "preventExtensions"]
        .into_iter()
        .zip(methods)
    {
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
            "{name}-props={}:{}",
            data_bits(length.1, length.2, length.3),
            data_bits(function_name.1, function_name.2, function_name.3),
        ));
    }
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_lines(oracle, GRAPH_ORACLE, "Object extensibility graph")
}
