use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, PropertyKey,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_object_getOwnPropertyDescriptor` and
// `js_object_getOwnPropertyDescriptors`. Proxy descriptor traps and TypedArray
// integer-indexed exotic descriptors are intentionally outside this milestone
// because the Rust runtime does not publish either object family yet.

const DESCRIPTOR_CASES: &[(&str, &str)] = &[
    (
        "data and accessor field order flags identity and no getter execution",
        r#"(function(){
            function bits(object,key){
                var descriptor=Object.getOwnPropertyDescriptor(object,key);
                return (descriptor.writable?1:0)+""+(descriptor.enumerable?1:0)+(descriptor.configurable?1:0);
            }
            function fieldBits(descriptor){
                var names=Object.getOwnPropertyNames(descriptor),out=[];
                for(var i=0;i<names.length;i++)out[out.length]=names[i]+":"+bits(descriptor,names[i]);
                return out.join(",");
            }
            var target=Object(),payload=Object(),data=Object(),calls=0;
            data.value=payload;data.writable=false;data.enumerable=true;data.configurable=false;
            Object.defineProperty(target,"data",data);
            var getter=function(){calls++;return 17},setter=function(value){calls+=value},accessor=Object();
            accessor.get=getter;accessor.set=setter;accessor.enumerable=false;accessor.configurable=true;
            Object.defineProperty(target,"accessor",accessor);
            var first=Object.getOwnPropertyDescriptor(target,"data");
            var second=Object.getOwnPropertyDescriptor(target,"data");
            var access=Object.getOwnPropertyDescriptor(target,"accessor");
            return Object.getOwnPropertyNames(first).join(",")+"|"+fieldBits(first)+"|"+
                (first.value===payload)+":"+first.writable+":"+first.enumerable+":"+first.configurable+":"+(first!==second)+"|"+
                Object.getOwnPropertyNames(access).join(",")+"|"+fieldBits(access)+"|"+
                (access.get===getter)+":"+(access.set===setter)+":"+access.enumerable+":"+access.configurable+":"+calls;
        })()"#,
    ),
    (
        "missing and inherited properties return undefined without consulting getters",
        r#"(function(){
            var calls=0,proto=Object(),target=Object.create(proto);
            proto.inherited=1;
            target.__defineGetter__("value",function(){calls++;return 2});
            var own=Object.getOwnPropertyDescriptor(target,"value");
            return (Object.getOwnPropertyDescriptor(target,"missing")===undefined)+":"+
                (Object.getOwnPropertyDescriptor(target,"inherited")===undefined)+":"+
                (typeof own.get)+":"+(own.get===target.__lookupGetter__("value"))+":"+calls;
        })()"#,
    ),
    (
        "primitive String Array and symbol-key exotic surfaces",
        r#"(function(){
            function token(descriptor){
                if(descriptor===undefined)return "undefined";
                var value=descriptor.value;
                return (typeof value)+":"+value+":"+descriptor.writable+":"+descriptor.enumerable+":"+descriptor.configurable;
            }
            var symbol=Symbol("own"),target=Object(),array=[];
            target[symbol]=23;array[2]="C";
            var symbolDescriptor=Object.getOwnPropertyDescriptor(target,symbol);
            return token(Object.getOwnPropertyDescriptor(17,"x"))+"|"+
                token(Object.getOwnPropertyDescriptor(false,"x"))+"|"+
                token(Object.getOwnPropertyDescriptor(9n,"x"))+"|"+
                token(Object.getOwnPropertyDescriptor(Symbol("target"),"x"))+"|"+
                token(Object.getOwnPropertyDescriptor("ab","0"))+"|"+
                token(Object.getOwnPropertyDescriptor("ab","length"))+"|"+
                token(Object.getOwnPropertyDescriptor(array,"2"))+"|"+
                token(Object.getOwnPropertyDescriptor(array,"length"))+"|"+
                (symbolDescriptor.value===23)+":"+symbolDescriptor.writable+":"+
                symbolDescriptor.enumerable+":"+symbolDescriptor.configurable;
        })()"#,
    ),
    (
        "undefined key is converted to the undefined property name",
        r#"(function(){
            var target=Object(),descriptor=Object();
            descriptor.value=41;descriptor.writable=true;descriptor.enumerable=false;descriptor.configurable=true;
            Object.defineProperty(target,"undefined",descriptor);
            var result=Object.getOwnPropertyDescriptor(target);
            return result.value+":"+result.writable+":"+result.enumerable+":"+result.configurable;
        })()"#,
    ),
];

const CONVERSION_CASES: &[(&str, &str)] = &[
    (
        "ToObject rejects nullish targets before converting the key",
        r#"(function(){
            function row(target){
                var log="",key=Object();
                key[Symbol.toPrimitive]=function(hint){log+=hint;throw "key"};
                try{Object.getOwnPropertyDescriptor(target,key)}
                catch(error){return log+"|"+(typeof error==="object"?error.name+":"+error.message:error)}
                return "missing";
            }
            return row(null)+"||"+row(undefined);
        })()"#,
    ),
    (
        "property key @@toPrimitive uses string hint and thrown value is preserved",
        r#"(function(){
            var log="",target=Object(),key=Object();target.named=7;
            key[Symbol.toPrimitive]=function(hint){log+=hint;return "named"};
            var descriptor=Object.getOwnPropertyDescriptor(target,key);
            var throwing=Object();
            throwing[Symbol.toPrimitive]=function(hint){log+="/"+hint;throw 73};
            try{Object.getOwnPropertyDescriptor(target,throwing)}catch(error){
                return descriptor.value+":"+log+":"+(error===73);
            }
            return "missing";
        })()"#,
    ),
    (
        "bulk nullish targets throw while every other primitive is boxed",
        r#"(function(){
            function row(value){
                try{return Object.getOwnPropertyNames(Object.getOwnPropertyDescriptors(value)).join(",")}
                catch(error){return error.name+":"+error.message}
            }
            return row(null)+"|"+row(undefined)+"|"+row(19)+"|"+row(false)+"|"+
                row(21n)+"|"+row(Symbol("bulk"))+"|"+row("ab");
        })()"#,
    ),
    (
        "recursive property-key coercion remains valid below the host safety ceiling",
        r#"(function(){
            var target=Object();target.x=31;
            function recurse(depth){
                var key=Object();
                key[Symbol.toPrimitive]=function(hint){
                    if(hint!=="string")throw "bad hint";
                    if(depth!==0)recurse(depth-1);
                    return "x";
                };
                return Object.getOwnPropertyDescriptor(target,key).value;
            }
            return recurse(4)+":"+Object.getOwnPropertyDescriptor(target,"x").value;
        })()"#,
    ),
];

const BULK_CASES: &[(&str, &str)] = &[
    (
        "bulk includes every own string symbol and nonenumerable key in canonical order",
        r#"(function(){
            function bits(object,key){
                var descriptor=Object.getOwnPropertyDescriptor(object,key);
                return (descriptor.writable?1:0)+""+(descriptor.enumerable?1:0)+(descriptor.configurable?1:0);
            }
            var calls=0,proto=Object(),target=Object.create(proto),hidden=Object();
            var first=Symbol("first"),second=Symbol("second");
            proto.inherited="I";target.z="Z";target[10]="ten";target[first]="S1";target[2]="two";target.a="A";
            hidden.value="H";hidden.writable=false;hidden.enumerable=false;hidden.configurable=true;
            Object.defineProperty(target,"hidden",hidden);
            target.__defineGetter__("access",function(){calls++;return "getter"});
            target[second]="S2";
            var result=Object.getOwnPropertyDescriptors(target),names=Object.getOwnPropertyNames(result),symbols=Object.getOwnPropertySymbols(result);
            var outer=[];for(var i=0;i<names.length;i++)outer[outer.length]=names[i]+":"+bits(result,names[i]);
            var symbolOuter=[];for(var j=0;j<symbols.length;j++)symbolOuter[symbolOuter.length]=(symbols[j]===first?"first":"second")+":"+bits(result,symbols[j]);
            return names.join(",")+"|"+outer.join(",")+"|"+
                (symbols[0]===first)+":"+(symbols[1]===second)+":"+symbolOuter.join(",")+"|"+
                result[2].value+":"+result[10].value+":"+result.hidden.value+":"+result.hidden.enumerable+"|"+
                Object.getOwnPropertyNames(result.access).join(",")+":"+(result.access.get===target.__lookupGetter__("access"))+":"+calls+"|"+
                (result.inherited===undefined)+":"+result[first].value+":"+result[second].value;
        })()"#,
    ),
    (
        "bulk materializes Object AutoInit descriptors and preserves callable identity",
        r#"(function(){
            var bulk=Object.getOwnPropertyDescriptors(Object);
            var singular=bulk.getOwnPropertyDescriptor;
            var plural=bulk.getOwnPropertyDescriptors;
            return (singular.value===Object.getOwnPropertyDescriptor)+":"+
                (plural.value===Object.getOwnPropertyDescriptors)+":"+
                singular.writable+":"+singular.enumerable+":"+singular.configurable+":"+
                plural.writable+":"+plural.enumerable+":"+plural.configurable+":"+
                singular.value.name+":"+singular.value.length+":"+plural.value.name+":"+plural.value.length;
        })()"#,
    ),
    (
        "bulk returns fresh outer and nested descriptor objects",
        r#"(function(){
            var target=Object();target.x=1;
            var first=Object.getOwnPropertyDescriptors(target),second=Object.getOwnPropertyDescriptors(target);
            first.x.value=9;first.extra=3;
            return (first!==second)+":"+(first.x!==second.x)+":"+second.x.value+":"+
                (second.extra===undefined)+":"+target.x;
        })()"#,
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
  'isExtensible','preventExtensions','getOwnPropertyDescriptor','getOwnPropertyDescriptors'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('getOwnPropertyDescriptor='+meta('getOwnPropertyDescriptor'));
print('getOwnPropertyDescriptors='+meta('getOwnPropertyDescriptors'));
print('identity='+(Object.getOwnPropertyDescriptor===Object.getOwnPropertyDescriptor)+':' +
  (Object.getOwnPropertyDescriptors===Object.getOwnPropertyDescriptors)+':' +
  (Object.getOwnPropertyDescriptor!==Object.getOwnPropertyDescriptors));
['getOwnPropertyDescriptor','getOwnPropertyDescriptors'].forEach(function(name){var fn=Object[name];
  print(name+'-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
});
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var a=delete Object.getOwnPropertyDescriptor,b=delete Object.getOwnPropertyDescriptors;
print([a,b,'getOwnPropertyDescriptor' in Object,'getOwnPropertyDescriptors' in Object,
  typeof Object.getOwnPropertyDescriptor,typeof Object.getOwnPropertyDescriptors].join('|'));
"#;

#[test]
fn object_descriptor_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object descriptor oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("descriptors", DESCRIPTOR_CASES),
        ("conversion", CONVERSION_CASES),
        ("bulk", BULK_CASES),
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
fn object_descriptor_values_match_pinned_quickjs() {
    compare_cases("Object descriptor values", DESCRIPTOR_CASES);
}

#[test]
fn object_descriptor_conversion_order_and_errors_match_pinned_quickjs() {
    compare_cases("Object descriptor conversions", CONVERSION_CASES);
}

#[test]
fn object_descriptor_bulk_surface_matches_pinned_quickjs() {
    compare_cases("Object descriptor bulk surface", BULK_CASES);
}

#[test]
fn object_descriptor_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object descriptor graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle)
    );
}

#[test]
fn object_descriptor_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object descriptor AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(
        &oracle,
        FRESH_DELETE_ORACLE,
        "Object descriptor fresh delete",
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let mut values = Vec::new();
    for name in ["getOwnPropertyDescriptor", "getOwnPropertyDescriptors"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .delete_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["getOwnPropertyDescriptor", "getOwnPropertyDescriptors"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .has_own_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["getOwnPropertyDescriptor", "getOwnPropertyDescriptors"] {
        let key = runtime.intern_property_key(name).unwrap();
        let value = context.get_property(object.as_object(), &key).unwrap();
        values.push(value_type(&runtime, &value).to_owned());
    }
    assert_eq!(vec![values.join("|")], expected);
}

#[test]
fn object_descriptor_cross_realm_results_nested_objects_and_errors_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let singular = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "getOwnPropertyDescriptor",
    );
    let plural = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "getOwnPropertyDescriptors",
    );
    let defining_object_prototype = defining.object_prototype().unwrap();

    let source = eval_object(
        &mut caller,
        r#"(function(){var value=Object();value.x=1;return value})()"#,
    );
    let Value::Object(descriptor) = caller
        .call(
            &singular,
            Value::Undefined,
            &[
                Value::Object(source.clone()),
                Value::String(JsString::try_from_utf8("x").unwrap()),
            ],
        )
        .unwrap()
    else {
        panic!("cross-realm Object.getOwnPropertyDescriptor did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&descriptor).unwrap(),
        Some(defining_object_prototype.clone()),
    );

    let Value::Object(descriptors) = caller
        .call(&plural, Value::Undefined, &[Value::Object(source)])
        .unwrap()
    else {
        panic!("cross-realm Object.getOwnPropertyDescriptors did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&descriptors).unwrap(),
        Some(defining_object_prototype.clone()),
    );
    let nested = object_property(&runtime, &mut caller, &descriptors, "x");
    assert_eq!(
        runtime.get_prototype_of(&nested).unwrap(),
        Some(defining_object_prototype),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    assert_eq!(
        caller.call(
            &singular,
            Value::Undefined,
            &[
                Value::Null,
                Value::String(JsString::try_from_utf8("x").unwrap()),
            ],
        ),
        Err(RuntimeError::Exception),
    );
    let framework_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
        Some(defining_type_error),
    );

    let caller_range_error = intrinsic_prototype(&runtime, &mut caller, "RangeError");
    let target = eval_object(&mut caller, "Object()");
    let throwing_key = eval_object(
        &mut caller,
        r#"(function(){var key=Object();key[Symbol.toPrimitive]=function(){throw new RangeError("key")};return key})()"#,
    );
    assert_eq!(
        caller.call(
            &singular,
            Value::Undefined,
            &[Value::Object(target), Value::Object(throwing_key)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_range_error),
    );
}

#[test]
fn object_descriptor_methods_and_results_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (singular, plural, descriptors, nested) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_singular = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "getOwnPropertyDescriptor",
        );
        let first_singular_again = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "getOwnPropertyDescriptor",
        );
        let first_plural = property_callable(
            &runtime,
            &mut first,
            first_object.as_object(),
            "getOwnPropertyDescriptors",
        );
        let second_singular = property_callable(
            &runtime,
            &mut second,
            second_object.as_object(),
            "getOwnPropertyDescriptor",
        );
        assert_eq!(first_singular, first_singular_again);
        assert_ne!(first_singular, first_plural);
        assert_ne!(first_singular, second_singular);
        assert_eq!(
            runtime
                .get_prototype_of(first_singular.as_object())
                .unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_singular);

        let source = eval_object(
            &mut first,
            r#"(function(){var value=Object();value.x=1;return value})()"#,
        );
        let Value::Object(result) = first
            .call(&first_plural, Value::Undefined, &[Value::Object(source)])
            .unwrap()
        else {
            panic!("Object.getOwnPropertyDescriptors did not return an object");
        };
        let nested = object_property(&runtime, &mut first, &result, "x");
        (first_singular, first_plural, result, nested)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(singular);
    drop(plural);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(descriptors);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(nested);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn object_descriptor_records_the_current_proxy_and_typed_array_gap() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("typeof Proxy+'|'+typeof ArrayBuffer+'|'+typeof Uint8Array")
            .unwrap(),
        Value::String(JsString::try_from_utf8("undefined|undefined|undefined").unwrap()),
        "update this boundary when Proxy or TypedArray descriptor semantics are published",
    );
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
                    string_property(runtime, context, &error, "name"),
                    string_property(runtime, context, &error, "message"),
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
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut output = vec![format!("prefix={prefix}")];
    let mut methods = Vec::new();
    for name in ["getOwnPropertyDescriptor", "getOwnPropertyDescriptors"] {
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
    output.push(format!("identity=true:true:{}", methods[0] != methods[1]));
    for (name, function) in ["getOwnPropertyDescriptor", "getOwnPropertyDescriptors"]
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
    oracle_lines(oracle, GRAPH_ORACLE, "Object descriptor graph")
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

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    let Value::Object(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object property");
    };
    value
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
        Number(configurable)
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
