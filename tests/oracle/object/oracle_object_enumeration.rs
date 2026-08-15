use crate::object_graph_observation::{
    data_bits, data_descriptor, eval_object, global_callable, int_property, intrinsic_prototype,
    oracle_lines, own_key_names,
};
use crate::quickjs_oracle::observe_completion as observe_oracle;
use crate::runtime_completion_oracle::compare_eval_completion_cases as compare_cases;
use crate::runtime_observation::{
    property_callable, string_property, take_pending_exception_object as take_exception_object,
};
use crate::runtime_oracle::value_type;
use std::ffi::OsStr;

use quickjs_oxide::{Context, ObjectRef, Runtime, RuntimeError, Value};

// Pins QuickJS 2026-06-04 `js_object_keys` for Object.keys/values/entries.
// Proxy ownKeys/getOwnPropertyDescriptor/get traps are intentionally outside
// this milestone because the Rust runtime does not yet publish Proxy objects.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "ordinary order enumerable filtering inherited and symbol keys",
        r#"(function(){
            function rows(values){var out=[];for(var i=0;i<values.length;i++)out[out.length]=values[i][0]+":"+values[i][1];return out.join(",")}
            var proto=Object();proto.inherited="I";
            var object=Object.create(proto),hidden=Object(),symbol=Symbol("hidden");
            object.b="B";object[2]="two";object[1]="one";object.a="A";
            hidden.value="H";hidden.enumerable=false;hidden.configurable=true;
            Object.defineProperty(object,"hidden",hidden);object[symbol]="S";
            return Object.keys(object).join(",")+"|"+Object.values(object).join(",")+"|"+
                rows(Object.entries(object))+"|"+object.inherited+"|"+Object.keys(object).length;
        })()"#,
    ),
    (
        "delete and readd moves a string key after retained strings",
        r#"(function(){
            var object=Object();object.first=1;object.second=2;object.third=3;
            delete object.second;object.second=4;
            return Object.keys(object).join(",")+"|"+Object.values(object).join(",")+"|"+
                Object.entries(object)[2][0]+":"+Object.entries(object)[2][1];
        })()"#,
    ),
    (
        "primitive ToObject families and String indices",
        r#"(function(){
            function row(value){return Object.keys(value).join(",")+":"+Object.values(value).join(",")+":"+Object.entries(value).length}
            return row("ab")+"|"+row(17)+"|"+row(false)+"|"+row(19n)+"|"+row(Symbol("s"));
        })()"#,
    ),
    (
        "Array and String exotic own-key surfaces",
        r#"(function(){
            function rows(values){var out=[];for(var i=0;i<values.length;i++)out[out.length]=values[i][0]+":"+values[i][1];return out.join(",")}
            var array=[];array[2]="C";array[0]="A";array.extra="E";
            var hidden=Object();hidden.value="H";hidden.enumerable=false;Object.defineProperty(array,"hidden",hidden);
            var string=Object("ab");string[4]="D";string.extra="X";
            return Object.keys(array).join(",")+"|"+Object.values(array).join(",")+"|"+rows(Object.entries(array))+"|"+
                Object.keys(string).join(",")+"|"+Object.values(string).join(",")+"|"+rows(Object.entries(string));
        })()"#,
    ),
    (
        "numeric strings precede ordinary strings and symbols stay excluded",
        r#"(function(){
            var object=Object(),first=Symbol("first"),second=Symbol("second");
            object.z="z";object[10]="ten";object[first]="S1";object[2]="two";
            object.a="a";object[second]="S2";object[1]="one";
            return Object.keys(object).join(",")+"|"+Object.values(object).join(",")+"|"+
                Object.getOwnPropertySymbols(object).length;
        })()"#,
    ),
];

const MUTATION_CASES: &[(&str, &str)] = &[
    (
        "keys skips Get while values and entries recheck later properties",
        r#"(function(){
            function run(kind){
                var log="",object=Object(),hidden=Object();
                object.__defineGetter__("a",function(){
                    log+="a";delete object.b;
                    var visible=Object();visible.value="C";visible.writable=true;visible.enumerable=true;visible.configurable=true;
                    Object.defineProperty(object,"c",visible);object.late="L";return "A";
                });
                object.b="B";hidden.value="C";hidden.writable=true;hidden.enumerable=false;hidden.configurable=true;
                Object.defineProperty(object,"c",hidden);
                var result=Object[kind](object),out=[];
                for(var i=0;i<result.length;i++)out[out.length]=kind==="entries"?result[i][0]+":"+result[i][1]:result[i];
                return kind+":"+out.join(",")+":"+log+":"+object.hasOwnProperty("late");
            }
            return run("keys")+"|"+run("values")+"|"+run("entries");
        })()"#,
    ),
    (
        "a getter can redefine the value source of a later snapshotted key",
        r#"(function(){
            function run(kind){
                var log="",object=Object();
                object.__defineGetter__("a",function(){
                    log+="a";object.__defineGetter__("b",function(){log+="b";return "B2"});return "A";
                });
                object.b="B1";
                var result=Object[kind](object),out=[];
                for(var i=0;i<result.length;i++)out[out.length]=kind==="entries"?result[i][0]+":"+result[i][1]:result[i];
                return out.join(",")+":"+log;
            }
            return run("keys")+"|"+run("values")+"|"+run("entries");
        })()"#,
    ),
    (
        "low-depth values and entries getter recursion stays on the normal path",
        r#"(function(){
            var remaining=4,calls=0,object=Object(),descriptor=Object();
            descriptor.enumerable=true;
            descriptor.get=function(){
                calls++;
                if(remaining===0)return "leaf";
                remaining--;
                if(remaining%2===0)return Object.values(object)[0];
                return Object.entries(object)[0][1];
            };
            Object.defineProperty(object,"value",descriptor);
            return Object.values(object)[0]+"|"+calls;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("Object.keys missing argument", "Object.keys()"),
    ("Object.values null", "Object.values(null)"),
    ("Object.entries undefined", "Object.entries(undefined)"),
    (
        "Object.values preserves primitive getter throw",
        r#"(function(){var object=Object();object.__defineGetter__("x",function(){throw 73});return Object.values(object)})()"#,
    ),
    (
        "Object.entries preserves Error getter throw",
        r#"(function(){var object=Object();object.__defineGetter__("x",function(){throw new RangeError("entry")});return Object.entries(object)})()"#,
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
  'defineProperties','getOwnPropertyNames','getOwnPropertySymbols','groupBy','keys','values','entries'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('keys='+meta('keys'));
print('values='+meta('values'));
print('entries='+meta('entries'));
print('identity='+(Object.keys===Object.keys)+':' +(Object.values===Object.values)+':' +
  (Object.entries===Object.entries)+':' +(Object.keys!==Object.values)+':' +(Object.values!==Object.entries));
['keys','values','entries'].forEach(function(name){var fn=Object[name];
  print(name+'-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
});
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var a=delete Object.keys,b=delete Object.values,c=delete Object.entries;
print([a,b,c,'keys' in Object,'values' in Object,'entries' in Object,
  typeof Object.keys,typeof Object.values,typeof Object.entries].join('|'));
"#;

#[test]
fn object_enumeration_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object enumeration oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("mutation", MUTATION_CASES),
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
    assert_eq!(oracle_graph_observations(&oracle).len(), 8);
}

#[test]
fn object_enumeration_values_match_pinned_quickjs() {
    compare_cases("Object enumeration values", VALUE_CASES);
}

#[test]
fn object_enumeration_mutation_order_matches_pinned_quickjs() {
    compare_cases("Object enumeration mutation order", MUTATION_CASES);
}

#[test]
fn object_enumeration_errors_match_pinned_quickjs() {
    compare_cases("Object enumeration errors", ERROR_CASES);
}

#[test]
fn object_enumeration_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object enumeration graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle)
    );
}

#[test]
fn object_enumeration_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object enumeration AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(
        &oracle,
        FRESH_DELETE_ORACLE,
        "Object enumeration fresh delete",
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let mut values = Vec::new();
    for name in ["keys", "values", "entries"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .delete_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["keys", "values", "entries"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .has_own_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["keys", "values", "entries"] {
        let key = runtime.intern_property_key(name).unwrap();
        let value = context.get_property(object.as_object(), &key).unwrap();
        values.push(value_type(&runtime, &value).to_owned());
    }
    assert_eq!(vec![values.join("|")], expected);
}

#[test]
fn object_enumeration_cross_realm_results_and_error_realms_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let keys = property_callable(&runtime, &mut defining, defining_object.as_object(), "keys");
    let values = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "values",
    );
    let entries = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "entries",
    );
    let defining_array = defining.array_prototype().unwrap();

    let source = eval_object(
        &mut caller,
        "(function(){var value=Object();value.x=1;return value})()",
    );
    let Value::Object(keys_result) = caller
        .call(&keys, Value::Undefined, &[Value::Object(source.clone())])
        .unwrap()
    else {
        panic!("cross-realm Object.keys did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&keys_result).unwrap(),
        Some(defining_array.clone())
    );

    let Value::Object(entries_result) = caller
        .call(&entries, Value::Undefined, &[Value::Object(source)])
        .unwrap()
    else {
        panic!("cross-realm Object.entries did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&entries_result).unwrap(),
        Some(defining_array.clone())
    );
    let pair = object_index(&runtime, &mut caller, &entries_result, 0);
    assert_eq!(
        runtime.get_prototype_of(&pair).unwrap(),
        Some(defining_array)
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    assert_eq!(
        caller.call(&keys, Value::Undefined, &[Value::Null]),
        Err(RuntimeError::Exception),
    );
    let framework_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
        Some(defining_type_error)
    );

    let caller_range_error = intrinsic_prototype(&runtime, &mut caller, "RangeError");
    let throwing = eval_object(
        &mut caller,
        r#"(function(){var value=Object();value.__defineGetter__("x",function(){throw new RangeError("user")});return value})()"#,
    );
    assert_eq!(
        caller.call(&values, Value::Undefined, &[Value::Object(throwing)]),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_range_error)
    );
}

#[test]
fn object_enumeration_methods_are_per_realm_and_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (keys, values, entries) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_keys = property_callable(&runtime, &mut first, first_object.as_object(), "keys");
        let first_keys_again =
            property_callable(&runtime, &mut first, first_object.as_object(), "keys");
        let first_values =
            property_callable(&runtime, &mut first, first_object.as_object(), "values");
        let first_entries =
            property_callable(&runtime, &mut first, first_object.as_object(), "entries");
        let second_keys =
            property_callable(&runtime, &mut second, second_object.as_object(), "keys");
        assert_eq!(first_keys, first_keys_again);
        assert_ne!(first_keys, first_values);
        assert_ne!(first_values, first_entries);
        assert_ne!(first_keys, second_keys);
        assert_eq!(
            runtime.get_prototype_of(first_keys.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_keys);
        (first_keys, first_values, first_entries)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(keys);
    drop(values);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(entries);
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
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut output = vec![format!("prefix={prefix}")];
    let mut methods = Vec::new();
    for name in ["keys", "values", "entries"] {
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
    output.push(format!(
        "identity=true:true:true:{}:{}",
        methods[0] != methods[1],
        methods[1] != methods[2],
    ));
    for (name, function) in ["keys", "values", "entries"].into_iter().zip(methods) {
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
    oracle_lines(oracle, GRAPH_ORACLE, "Object enumeration graph")
}

fn object_index(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    index: u32,
) -> ObjectRef {
    let Value::Object(value) = context
        .get_property(
            object,
            &runtime.intern_property_key(&index.to_string()).unwrap(),
        )
        .unwrap()
    else {
        panic!("index {index} was not an object");
    };
    value
}
