use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, PropertyKey,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_object_assign` and `JS_CopyDataProperties`.
// Ordinary sources use QuickJS's enumerable-at-snapshot optimization; Proxy
// sources instead recheck each descriptor. Proxy, TypedArray and
// module-namespace exotic paths are recorded below but deliberately do not
// require support from the current Rust object model; Arguments is locked by
// its dedicated differential.

const CONVERSION_CASES: &[(&str, &str)] = &[
    (
        "actual argc controls missing and single-argument calls while this is ignored",
        r#"(function(){
            var missing;
            try{Object.assign()}catch(error){missing=error.name+":"+error.message}
            var target=Object(),receiver=Object();target.kept=1;
            return missing+":"+(Object.assign(target)===target)+":"+
                (Object.assign.call(receiver,target)===target)+":"+target.kept;
        })()"#,
    ),
    (
        "target is boxed before sources and the exact target is returned",
        r#"(function(){
            var target=Object(),source=Object(),last=Object();source.first=1;last.second=2;
            var returned=Object.assign(target,source,null,undefined,last);
            return (returned===target)+":"+target.first+":"+target.second+":"+
                Object.getOwnPropertyNames(target).join(",");
        })()"#,
    ),
    (
        "primitive targets are boxed while preserving primitive payloads",
        r#"(function(){
            function source(value){var object=Object();object.x=value;return object}
            var marker=Symbol("marker");
            var number=Object.assign(17,source(1));
            var boolean=Object.assign(false,source(2));
            var string=Object.assign("ab",source(3));
            var bigint=Object.assign(19n,source(4));
            var symbol=Object.assign(marker,source(5));
            return [
                typeof number,number.valueOf(),number.x,
                typeof boolean,boolean.valueOf(),boolean.x,
                typeof string,string.valueOf(),string[0]+string[1],string.x,
                typeof bigint,bigint.valueOf(),bigint.x,
                typeof symbol,symbol.valueOf()===marker,symbol.x
            ].join(":");
        })()"#,
    ),
    (
        "nullish sources are skipped and primitive sources contribute only String indices",
        r#"(function(){
            var symbol=Symbol("source"),target=Object();
            var returned=Object.assign(target,null,undefined,17,false,23n,symbol,"ab");
            return (returned===target)+":"+Object.getOwnPropertyNames(target).join(",")+":"+
                target[0]+target[1]+":"+Object.getOwnPropertySymbols(target).length;
        })()"#,
    ),
    (
        "multiple sources are converted and copied strictly left to right",
        r#"(function(){
            var log="",first=Object(),second=Object(),third=Object();
            first.__defineGetter__("x",function(){log+="first|";return 1});
            second.__defineGetter__("x",function(){log+="second|";return 2});
            third.__defineGetter__("y",function(){log+="third|";return 3});
            var target=Object.assign(Object(),first,second,third);
            return log+target.x+":"+target.y;
        })()"#,
    ),
];

const PROPERTY_CASES: &[(&str, &str)] = &[
    (
        "own enumerable strings and Symbols copy in canonical order",
        r#"(function(){
            var proto=Object(),source=Object.create(proto),hidden=Object();
            var first=Symbol("first"),second=Symbol("second");
            proto.inherited="I";source.z="Z";source[10]="ten";source[first]="S1";
            source[2]="two";source.a="A";
            hidden.value="H";hidden.writable=true;hidden.enumerable=false;hidden.configurable=true;
            Object.defineProperty(source,"hidden",hidden);source[second]="S2";
            var target=Object.assign(Object(),source),names=Object.getOwnPropertyNames(target);
            var symbols=Object.getOwnPropertySymbols(target);
            return names.join(",")+"|"+target[2]+":"+target[10]+":"+target.z+":"+target.a+"|"+
                (symbols[0]===first)+":"+(symbols[1]===second)+":"+target[first]+":"+target[second]+"|"+
                (target.hidden===undefined)+":"+(target.inherited===undefined);
        })()"#,
    ),
    (
        "new target properties use assignment data flags and existing properties are updated",
        r#"(function(){
            function bits(object,key){var d=Object.getOwnPropertyDescriptor(object,key);return d.writable+":"+d.enumerable+":"+d.configurable}
            var target=Object(),fixed=Object();target.existing=1;
            fixed.value=2;fixed.writable=true;fixed.enumerable=false;fixed.configurable=true;
            Object.defineProperty(target,"fixed",fixed);
            var source=Object();source.existing=3;source.created=4;source.fixed=5;
            Object.assign(target,source);
            return target.existing+":"+target.created+":"+target.fixed+"|"+
                bits(target,"existing")+"|"+bits(target,"created")+"|"+bits(target,"fixed");
        })()"#,
    ),
    (
        "Set invokes inherited setters instead of defining an own property",
        r#"(function(){
            var log="",proto=Object(),target=Object.create(proto),source=Object();
            proto.__defineSetter__("x",function(value){log+="set:"+value+":"+(this===target)});
            source.x=41;
            var returned=Object.assign(target,source);
            return (returned===target)+":"+log+":"+target.hasOwnProperty("x");
        })()"#,
    ),
    (
        "the inherited __proto__ setter is used instead of CreateDataProperty",
        r#"(function(){
            var target=Object(),nextPrototype=Object(),source=Object(),descriptor=Object();
            descriptor.value=nextPrototype;descriptor.writable=true;
            descriptor.enumerable=true;descriptor.configurable=true;
            Object.defineProperty(source,"__proto__",descriptor);
            var returned=Object.assign(target,source);
            return (returned===target)+":"+(Object.getPrototypeOf(target)===nextPrototype)+":"+
                target.hasOwnProperty("__proto__");
        })()"#,
    ),
    (
        "assigning an object to itself preserves values and identities",
        r#"(function(){
            var log="",stored=7,value=Object(),symbol=Symbol("self"),descriptor=Object();
            descriptor.enumerable=true;descriptor.configurable=true;
            descriptor.get=function(){log+="g";return stored};
            descriptor.set=function(next){log+="s";stored=next};
            Object.defineProperty(value,"x",descriptor);value[symbol]=value;
            var returned=Object.assign(value,value);
            return (returned===value)+":"+stored+":"+log+":"+(value[symbol]===value);
        })()"#,
    ),
];

const MUTATION_CASES: &[(&str, &str)] = &[
    (
        "each ordinary source snapshots enumerable own keys before any Get",
        r#"(function(){
            var getterThis=false,proto=Object(),source=Object.create(proto),target=Object();
            proto.b="inherited";
            source.__defineGetter__("a",function(){getterThis=this===source;delete source.b;source.late="late";return "A"});
            source.b="own";
            Object.assign(target,source);
            return Object.getOwnPropertyNames(target).join(",")+":"+target.a+":"+target.b+":"+
                (target.late===undefined)+":"+source.hasOwnProperty("b")+":"+getterThis;
        })()"#,
    ),
    (
        "pinned ordinary fast path does not recheck later enumerability",
        r#"(function(){
            var source=Object(),target=Object(),hidden=Object();
            source.__defineGetter__("a",function(){
                var b=Object();b.value="B";b.writable=true;b.enumerable=false;b.configurable=true;
                Object.defineProperty(source,"b",b);
                var h=Object();h.value="H";h.writable=true;h.enumerable=true;h.configurable=true;
                Object.defineProperty(source,"hidden",h);
                return "A";
            });
            source.b="before";
            hidden.value="hidden";hidden.writable=true;hidden.enumerable=false;hidden.configurable=true;
            Object.defineProperty(source,"hidden",hidden);
            Object.assign(target,source);
            return Object.getOwnPropertyNames(target).join(",")+":"+target.a+":"+target.b+":"+
                (target.hidden===undefined)+":"+Object.getOwnPropertyDescriptor(source,"b").enumerable+":"+
                Object.getOwnPropertyDescriptor(source,"hidden").enumerable;
        })()"#,
    ),
    (
        "Get completes before Set for every string and Symbol key",
        r#"(function(){
            var log="",source=Object(),target=Object(),symbol=Symbol("order");
            source.__defineGetter__("a",function(){log+="get-a|";return "A"});
            source.__defineGetter__("b",function(){log+="get-b|";return "B"});
            source.__defineGetter__(symbol,function(){log+="get-symbol|";return "S"});
            target.__defineSetter__("a",function(value){log+="set-a:"+value+"|"});
            target.__defineSetter__("b",function(value){log+="set-b:"+value+"|"});
            target.__defineSetter__(symbol,function(value){log+="set-symbol:"+value+"|"});
            Object.assign(target,source);
            return log+target.hasOwnProperty("a")+":"+target.hasOwnProperty("b");
        })()"#,
    ),
    (
        "a target setter can affect the Get of a later snapshotted key",
        r#"(function(){
            var log="",proto=Object(),source=Object.create(proto),target=Object();
            proto.b="prototype";source.a="A";source.b="own";
            target.__defineSetter__("a",function(value){log+="set-a:"+value+"|";delete source.b});
            target.__defineSetter__("b",function(value){log+="set-b:"+value+"|"});
            Object.assign(target,source);
            return log+source.hasOwnProperty("b");
        })()"#,
    ),
    (
        "a later source is snapshotted only when its turn begins",
        r#"(function(){
            var log="",first=Object(),second=Object(),target=Object();
            first.__defineGetter__("a",function(){log+="first|";second.added="later";return "A"});
            second.b="B";
            Object.assign(target,first,second);
            return log+Object.getOwnPropertyNames(target).join(",")+":"+target.added;
        })()"#,
    ),
];

const PARTIAL_AND_ERROR_CASES: &[(&str, &str)] = &[
    (
        "nullish targets throw before any source getter executes",
        r#"(function(){
            var calls=0,source=Object();source.__defineGetter__("x",function(){calls++;return 1});
            function row(target){try{Object.assign(target,source);return "missing"}catch(error){return error.name+":"+error.message+":"+calls}}
            return row(null)+"|"+row(undefined);
        })()"#,
    ),
    (
        "source getter throw preserves prior writes and stops later keys",
        r#"(function(){
            var target=Object(),source=Object();source.a=1;
            source.__defineGetter__("b",function(){throw 73});source.c=3;
            try{Object.assign(target,source)}catch(error){
                return (error===73)+":"+target.a+":"+(target.b===undefined)+":"+(target.c===undefined)+":"+
                    Object.getOwnPropertyNames(target).join(",");
            }
            return "missing";
        })()"#,
    ),
    (
        "target setter throw happens after source Get and preserves earlier writes",
        r#"(function(){
            var log="",target=Object(),source=Object();source.a=1;
            source.__defineGetter__("b",function(){log+="get-b|";return 2});source.c=3;
            target.__defineSetter__("b",function(value){log+="set-b:"+value+"|";throw new RangeError("setter")});
            try{Object.assign(target,source)}catch(error){
                return error.name+":"+error.message+"|"+log+"|"+target.a+":"+(target.c===undefined);
            }
            return "missing";
        })()"#,
    ),
    (
        "read only and non extensible targets reject through Set",
        r#"(function(){
            function readonly(){
                var target=Object(),descriptor=Object();descriptor.value=1;descriptor.writable=false;
                descriptor.enumerable=true;descriptor.configurable=true;Object.defineProperty(target,"x",descriptor);
                var source=Object();source.x=2;
                try{Object.assign(target,source)}catch(error){return error.name+":"+error.message+":"+target.x}
                return "missing";
            }
            function closed(){
                var target=Object();target.kept=1;Object.preventExtensions(target);
                var source=Object();source.kept=2;source.added=3;
                try{Object.assign(target,source)}catch(error){return error.name+":"+error.message+":"+target.kept+":"+(target.added===undefined)}
                return "missing";
            }
            return readonly()+"|"+closed();
        })()"#,
    ),
    (
        "String indices and accessors without setters reject with Throw enabled",
        r#"(function(){
            function stringIndex(){
                var source=Object();source[0]="changed";
                try{Object.assign("ab",source)}catch(error){return error.name+":"+error.message}
                return "missing";
            }
            function noSetter(){
                var target=Object(),descriptor=Object(),source=Object();
                descriptor.enumerable=true;descriptor.configurable=true;
                descriptor.get=function(){return 1};
                Object.defineProperty(target,"x",descriptor);source.x=2;
                try{Object.assign(target,source)}catch(error){return error.name+":"+error.message}
                return "missing";
            }
            return stringIndex()+"|"+noSetter();
        })()"#,
    ),
    (
        "failure in a later source retains all earlier-source mutations",
        r#"(function(){
            var target=Object(),first=Object(),second=Object();first.a=1;second.b=2;
            second.__defineGetter__("c",function(){throw "stop"});
            try{Object.assign(target,first,second)}catch(error){
                return error+":"+target.a+":"+target.b+":"+(target.c===undefined);
            }
            return "missing";
        })()"#,
    ),
];

// Only pinned QuickJS executes these vectors. They document the exotic branch
// where `JS_CopyDataProperties` cannot apply its ordinary enumerable-at-snapshot
// optimization and therefore rechecks every descriptor. The Rust differential
// becomes active when the corresponding object families are published.
const EXOTIC_ORACLE_ONLY_CASES: &[(&str, &str)] = &[
    (
        "Proxy ownKeys snapshot is followed by descriptor recheck then Get",
        r#"(function(){
            var log="",visible=true,base={a:"A",b:"B"};
            var source=new Proxy(base,{
                ownKeys:function(){log+="ownKeys|";return ["a","b"]},
                getOwnPropertyDescriptor:function(target,key){log+="desc-"+key+":"+visible+"|";return {value:target[key],writable:true,enumerable:visible,configurable:true}},
                get:function(target,key){log+="get-"+key+"|";if(key==="a")visible=false;return target[key]}
            });
            var target=Object.assign({},source);
            return log+Object.getOwnPropertyNames(target).join(",")+":"+target.a+":"+(target.b===undefined);
        })()"#,
    ),
    (
        "TypedArray integer indices are copied as enumerable own keys",
        r#"(function(){
            var target=Object.assign({},new Uint8Array([7,8]));
            return Object.getOwnPropertyNames(target).join(",")+":"+target[0]+":"+target[1];
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
  'isExtensible','preventExtensions','getOwnPropertyDescriptor','getOwnPropertyDescriptors','is','assign'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
print('assign='+meta('assign'));
print('identity='+(Object.assign===Object.assign));
var fn=Object.assign;
print('assign-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var deleted=delete Object.assign;
print([deleted,'assign' in Object,Object.prototype.hasOwnProperty.call(Object,'assign'),typeof Object.assign].join('|'));
"#;

#[test]
fn object_assign_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.assign oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("conversions", CONVERSION_CASES),
        ("properties", PROPERTY_CASES),
        ("mutations", MUTATION_CASES),
        ("partial errors", PARTIAL_AND_ERROR_CASES),
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
        oracle_lines(&oracle, FRESH_DELETE_ORACLE, "Object.assign fresh delete").len(),
        1,
    );
}

#[test]
fn object_assign_conversions_match_pinned_quickjs() {
    compare_cases("Object.assign conversions", CONVERSION_CASES);
}

#[test]
fn object_assign_property_selection_matches_pinned_quickjs() {
    compare_cases("Object.assign properties", PROPERTY_CASES);
}

#[test]
fn object_assign_snapshot_get_set_order_matches_pinned_quickjs() {
    compare_cases("Object.assign mutation order", MUTATION_CASES);
}

#[test]
fn object_assign_partial_mutation_and_errors_match_pinned_quickjs() {
    compare_cases("Object.assign partial mutation", PARTIAL_AND_ERROR_CASES);
}

#[test]
fn object_assign_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.assign graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
    );
}

#[test]
fn object_assign_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object.assign AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(&oracle, FRESH_DELETE_ORACLE, "Object.assign fresh delete");

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let key = runtime.intern_property_key("assign").unwrap();
    let deleted = runtime
        .delete_property(object.as_object(), &key)
        .unwrap()
        .to_string();
    let Value::Bool(in_object) = context.eval("'assign' in Object").unwrap() else {
        panic!("Object.assign inherited-presence probe was not boolean");
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
fn object_assign_cross_realm_targets_and_error_realms_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let assign = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "assign",
    );

    let target = caller.new_object().unwrap();
    let source = eval_object(
        &mut caller,
        "(function(){var value=Object();value.x=1;return value})()",
    );
    let returned = caller
        .call(
            &assign,
            Value::Undefined,
            &[Value::Object(target.clone()), Value::Object(source)],
        )
        .unwrap();
    assert_eq!(returned, Value::Object(target.clone()));
    assert_eq!(
        caller
            .get_property(&target, &runtime.intern_property_key("x").unwrap())
            .unwrap(),
        Value::Int(1),
    );

    let defining_number_prototype = intrinsic_prototype(&runtime, &mut defining, "Number");
    let source = eval_object(
        &mut caller,
        "(function(){var value=Object();value.x=2;return value})()",
    );
    let Value::Object(boxed) = caller
        .call(
            &assign,
            Value::Undefined,
            &[Value::Int(17), Value::Object(source)],
        )
        .unwrap()
    else {
        panic!("cross-realm primitive Object.assign target was not boxed");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed).unwrap(),
        Some(defining_number_prototype),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    assert_eq!(
        caller.call(&assign, Value::Undefined, &[Value::Null]),
        Err(RuntimeError::Exception),
    );
    let framework_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&framework_error).unwrap(),
        Some(defining_type_error.clone()),
    );

    let readonly = eval_object(
        &mut caller,
        r#"(function(){var target=Object(),descriptor=Object();descriptor.value=1;descriptor.writable=false;descriptor.enumerable=true;descriptor.configurable=true;Object.defineProperty(target,"x",descriptor);return target})()"#,
    );
    let readonly_source = eval_object(
        &mut caller,
        "(function(){var value=Object();value.x=2;return value})()",
    );
    assert_eq!(
        caller.call(
            &assign,
            Value::Undefined,
            &[Value::Object(readonly), Value::Object(readonly_source),],
        ),
        Err(RuntimeError::Exception),
    );
    let set_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&set_error).unwrap(),
        Some(defining_type_error),
    );

    let caller_range_error = intrinsic_prototype(&runtime, &mut caller, "RangeError");
    let throwing = eval_object(
        &mut caller,
        r#"(function(){var value=Object();value.__defineGetter__("x",function(){throw new RangeError("source")});return value})()"#,
    );
    assert_eq!(
        caller.call(
            &assign,
            Value::Undefined,
            &[Value::Object(target), Value::Object(throwing)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_range_error.clone()),
    );

    let setter_target = eval_object(
        &mut caller,
        r#"(function(){var target=Object(),descriptor=Object();descriptor.enumerable=true;descriptor.configurable=true;descriptor.set=function(){throw new RangeError("target")};Object.defineProperty(target,"x",descriptor);return target})()"#,
    );
    let setter_source = eval_object(
        &mut caller,
        "(function(){var value=Object();value.x=3;return value})()",
    );
    assert_eq!(
        caller.call(
            &assign,
            Value::Undefined,
            &[Value::Object(setter_target), Value::Object(setter_source),],
        ),
        Err(RuntimeError::Exception),
    );
    let setter_error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&setter_error).unwrap(),
        Some(caller_range_error),
    );
}

#[test]
fn object_assign_method_and_boxed_result_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (assign, boxed) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_assign =
            property_callable(&runtime, &mut first, first_object.as_object(), "assign");
        let first_assign_again =
            property_callable(&runtime, &mut first, first_object.as_object(), "assign");
        let second_assign =
            property_callable(&runtime, &mut second, second_object.as_object(), "assign");
        assert_eq!(first_assign, first_assign_again);
        assert_ne!(first_assign, second_assign);
        assert_eq!(
            runtime.get_prototype_of(first_assign.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_assign);

        let source = first.new_object().unwrap();
        let Value::Object(boxed) = first
            .call(
                &first_assign,
                Value::Undefined,
                &[Value::Int(3), Value::Object(source)],
            )
            .unwrap()
        else {
            panic!("Object.assign primitive target was not boxed");
        };
        (first_assign, boxed)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(assign);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(boxed);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn object_assign_records_current_proxy_typed_array_and_namespace_gap() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("typeof Proxy+'|'+typeof ArrayBuffer+'|'+typeof Uint8Array")
            .unwrap(),
        Value::String(JsString::try_from_utf8("undefined|undefined|undefined").unwrap()),
        "activate the exotic oracle vectors when these Object.assign sources are published",
    );
    // Module namespace objects still need their own-key exotic integration
    // before the full CopyDataProperties surface is done.
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
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let key = runtime.intern_property_key("assign").unwrap();
    let descriptor = data_descriptor(&runtime, object.as_object(), &key);
    let Value::Object(function) = descriptor.0 else {
        panic!("Object.assign was not an object");
    };
    let callable = runtime.as_callable(&function).unwrap();
    let function_again = property_callable(&runtime, &mut context, object.as_object(), "assign");
    let mut output = vec![
        format!("prefix={prefix}"),
        format!(
            "assign={}:{}:{}:{}:{}:{}:{}",
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
        "assign-props={}:{}",
        data_bits(length.1, length.2, length.3),
        data_bits(function_name.1, function_name.2, function_name.3),
    ));
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_lines(oracle, GRAPH_ORACLE, "Object.assign graph")
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
