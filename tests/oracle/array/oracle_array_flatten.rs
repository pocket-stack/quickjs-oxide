use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04 `JS_FlattenIntoArray` and its shared
// magic-selected `flatMap` / `flat` wrapper.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "flat defaults to one level and compacts every outer hole",
        r#"(function(){
            var source=[1,[2,,[3,4]],,5],result=source.flat();
            return result.length+"|"+result[0]+"|"+result[1]+"|"+
                Array.isArray(result[2])+"|"+result[2][0]+"|"+result[2][1]+"|"+
                result[3]+"|"+source.length;
        })()"#,
    ),
    (
        "flat depth zero one two and infinity preserve the pinned nesting",
        r#"(function(){
            var source=[1,[2,[3,[4]]]],a=source.flat(0),b=source.flat(1);
            var c=source.flat(2),d=source.flat(Infinity);
            return a.length+":"+Array.isArray(a[1])+"|"+
                b.length+":"+Array.isArray(b[2])+"|"+
                c.length+":"+Array.isArray(c[3])+"|"+d.length+":"+d.join(",");
        })()"#,
    ),
    (
        "flat depth uses default undefined and saturating Int32 conversion",
        r#"(function(){
            function tag(depth){var result=[1,[2,[3]]].flat(depth);
                return result.length+":"+Array.isArray(result[result.length-1])}
            return tag(undefined)+"|"+tag(0/0)+"|"+tag(null)+"|"+tag(1.9)+"|"+
                tag(-1.9)+"|"+tag(Infinity)+"|"+tag(-Infinity)+"|"+tag("2");
        })()"#,
    ),
    (
        "holes compact while inherited values remain visible at every level",
        r#"(function(){
            Array.prototype[1]="P";
            var inner=Array(3);inner[0]="a";inner[2]=["c"];
            var source=[inner,,["d"]],result=source.flat(2);
            var text=result.length+"|"+result.join(",");
            delete Array.prototype[1];return text;
        })()"#,
    ),
    (
        "only genuine Arrays flatten and generic receivers return a base Array",
        r#"(function(){
            var spreadable=Object();spreadable[0]="x";spreadable.length=1;
            spreadable[Symbol.isConcatSpreadable]=true;
            var first=[spreadable,[1]].flat();
            var generic=Object();generic[0]=[2];generic[2]=3;generic.length=3;
            generic.__defineGetter__("constructor",function(){throw 71});
            var second=Array.prototype.flat.call(generic);
            var third=Array.prototype.flat.call("ab");
            return (first[0]===spreadable)+"|"+first[1]+"|"+
                second.length+":"+second.join(",")+":"+Array.isArray(second)+"|"+
                third.length+":"+third.join(",");
        })()"#,
    ),
    (
        "flatMap binds thisArg and passes value index and boxed source",
        r#"(function(){
            var source=[10,,30],receiver=Object(),seen="";receiver.offset=1;
            var result=source.flatMap(function(value,index,object){
                seen+=value+":"+index+":"+(object===source)+":"+(this===receiver)+";";
                return [value+this.offset,index];
            },receiver);
            return result.length+"|"+result.join(",")+"|"+seen;
        })()"#,
    ),
    (
        "flatMap maps only outer present elements and compacts returned holes",
        r#"(function(){
            var calls=0,result=[1,2].flatMap(function(value){
                calls++;var mapped=Array(3);mapped[0]=value;mapped[2]=[value+10];return mapped;
            });
            return calls+"|"+result.length+"|"+result[0]+"|"+
                Array.isArray(result[1])+":"+result[1][0]+"|"+result[2]+"|"+
                Array.isArray(result[3])+":"+result[3][0];
        })()"#,
    ),
    (
        "flatMap boxes primitive receivers before passing the callback source",
        r#"(function(){
            var seen="",result=Array.prototype.flatMap.call("ab",function(value,index,source){
                seen+=typeof source+":"+source.length+":"+source[index]+";";return [value];
            });
            return result.length+"|"+result.join(",")+"|"+seen;
        })()"#,
    ),
    (
        "flatMap snapshots outer length but observes later deletion and replacement",
        r#"(function(){
            var source=[1,2,3],seen="";
            var result=source.flatMap(function(value,index){
                seen+=index;if(index===0){delete source[1];source[2]=9;source[3]=4}
                return [value];
            });
            return result.length+"|"+result.join(",")+"|"+seen+"|"+source.length;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "generic length depth and indexed Get run in pinned order",
        r#"(function(){
            var log="",length=Object(),depth=Object(),source=Object();
            length.valueOf=function(){log+="N";return 1};
            depth.valueOf=function(){log+="D";return 0};
            source.__defineGetter__("length",function(){log+="L";return length});
            source.__defineGetter__("0",function(){log+="G";return [1]});
            var result=Array.prototype.flat.call(source,depth);
            return log+"|"+result.length+"|"+Array.isArray(result[0]);
        })()"#,
    ),
    (
        "flat converts depth then creates species before indexed reads",
        r#"(function(){
            var log="",source=[1,2],ctor=Object(),depth=Object();
            depth.valueOf=function(){log+="D";return 1};
            source.__defineGetter__("0",function(){log+="G";return [1]});
            function Species(){log+="A";source[1]=9;return Object()}
            ctor.__defineGetter__(Symbol.species,function(){log+="S";return Species});
            source.__defineGetter__("constructor",function(){log+="C";return ctor});
            var result=source.flat(depth);
            return log+"|"+result[0]+"|"+result[1]+"|"+("length" in result);
        })()"#,
    ),
    (
        "flatMap rejects its mapper before constructor and species lookup",
        r#"(function(){
            var log="",source=[1];
            source.__defineGetter__("constructor",function(){log+="C";throw 72});
            try{source.flatMap(1);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "flatMap creates species before alternating source Gets and mapper calls",
        r#"(function(){
            var log="",source=[1,2],ctor=Object();
            source.__defineGetter__("0",function(){log+="G0";return 1});
            source.__defineGetter__("1",function(){log+="G1";return 2});
            function Species(){log+="A";return Object()}
            ctor.__defineGetter__(Symbol.species,function(){log+="S";return Species});
            source.__defineGetter__("constructor",function(){log+="C";return ctor});
            var result=source.flatMap(function(value,index){log+="M"+index;return [value]});
            return log+"|"+result[0]+result[1]+"|"+("length" in result);
        })()"#,
    ),
    (
        "flatten traverses nested arrays depth first before the next outer index",
        r#"(function(){
            var log="",nested=[1,2],source=[nested,3];
            nested.__defineGetter__("0",function(){log+="N0";return 1});
            nested.__defineGetter__("1",function(){log+="N1";return 2});
            source.__defineGetter__("0",function(){log+="S0";return nested});
            source.__defineGetter__("1",function(){log+="S1";return 3});
            var result=source.flat();return log+"|"+result.join(",");
        })()"#,
    ),
    (
        "each nested Array length is snapshotted when that frame is entered",
        r#"(function(){
            var log="",nested=[1,2],source=[nested];
            nested.__defineGetter__("0",function(){log+="G0";nested[2]=3;return 1});
            nested.__defineGetter__("1",function(){log+="G1";nested.length=1;return 2});
            var result=source.flat();return log+"|"+result.length+"|"+result.join(",");
        })()"#,
    ),
];

const SPECIES_CASES: &[(&str, &str)] = &[
    (
        "custom species receives zero and CreateDataProperty skips inherited setters",
        r#"(function(){
            var captured,ctor=Object(),proto=Object(),setterHits=0,called="";
            proto.__defineSetter__("0",function(){setterHits++});
            function Species(length){called+="N"+length;captured=Object.create(proto);return captured}
            ctor[Symbol.species]=Species;var source=[1,[2]];source.constructor=ctor;
            var result=source.flat();
            return (result===captured)+"|"+called+"|"+setterHits+"|"+
                result[0]+result[1]+"|"+("length" in result)+"|"+
                Object.prototype.hasOwnProperty.call(result,"0");
        })()"#,
    ),
    (
        "species may alias the result to the source and later reads see writes",
        r#"(function(){
            var source=[1,[2,3],4],ctor=Object();function Species(){return source}
            ctor[Symbol.species]=Species;source.constructor=ctor;
            var result=source.flat();
            return (result===source)+"|"+source.length+"|"+source.join(",");
        })()"#,
    ),
    (
        "a rejected custom result definition preserves the completed prefix",
        r#"(function(){
            var captured,ctor=Object(),descriptor=Object();
            function Species(){
                captured=Object();descriptor.value="fixed";descriptor.writable=false;
                descriptor.enumerable=true;descriptor.configurable=false;
                Object.defineProperty(captured,"1",descriptor);return captured;
            }
            ctor[Symbol.species]=Species;var source=[1,2,3];source.constructor=ctor;
            try{source.flat(0);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+captured[0]+"|"+
                captured[1]+"|"+("length" in captured)}
        })()"#,
    ),
    (
        "generic flatten ignores constructor while using a defining realm base Array",
        r#"(function(){
            var source=Object();source.length=1;source[0]=[7];
            source.__defineGetter__("constructor",function(){throw 73});
            var result=Array.prototype.flat.call(source);
            return result.length+"|"+result[0]+"|"+Array.isArray(result)+"|"+
                (Object.getPrototypeOf(result)===Array.prototype);
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("flat null receiver", "Array.prototype.flat.call(null)"),
    (
        "flat undefined receiver",
        "Array.prototype.flat.call(undefined)",
    ),
    ("flat Symbol depth", "[1].flat(Symbol('depth'))"),
    ("flat BigInt depth", "[1].flat(1n)"),
    ("flatMap omitted mapper", "[1].flatMap()"),
    ("flatMap null mapper", "[1].flatMap(null)"),
    ("flatMap ordinary object mapper", "[1].flatMap(Object())"),
    (
        "flatMap preserves primitive mapper throw",
        "[1].flatMap(function(){throw 77})",
    ),
    (
        "flat preserves source getter throw",
        r#"(function(){var source=[1];source.__defineGetter__("0",function(){throw 78});
            return source.flat()})()"#,
    ),
    (
        "flat preserves species getter throw",
        r#"(function(){var source=[1],ctor=Object();
            ctor.__defineGetter__(Symbol.species,function(){throw 79});source.constructor=ctor;
            return source.flat()})()"#,
    ),
];

const RECURSION_CASES: &[(&str, &str)] = &[
    (
        "four nested mapper reentries complete before the local safety ceiling",
        r#"(function(){
            function nest(count){
                return [1].flatMap(function(value){return count?nest(count-1):[value]})
            }
            return nest(4)[0];
        })()"#,
    ),
    (
        "self-referential infinity flatten throws a catchable stack overflow",
        r#"(function(){var source=[];source[0]=source;
            try{source.flat(Infinity);return "missing"}
            catch(error){return error.name+"|"+error.message}})()"#,
    ),
    (
        "deep nesting straddles the pinned stack guard",
        r#"(function(){
            function run(count){var source=[1];
                for(var index=0;index<count;index++)source=[source];
                try{var result=source.flat(Infinity);return "ok:"+result[0]}
                catch(error){return error.name+":"+error.message}}
            return run(1000)+"|"+run(10000);
        })()"#,
    ),
    (
        "recursive indexed getters stay catchable at the native-call boundary",
        r#"(function(){var source=[1];
            source.__defineGetter__("0",function(){return source.flat(Infinity)});
            try{source.flat(Infinity);return "missing"}
            catch(error){return error.name+"|"+error.message}})()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['copyWithin','flatMap','flat','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
function meta(name) {
  var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,name),fn=descriptor.value;
  var constructable;
  try { Reflect.construct(function(){},[],fn); constructable=true; }
  catch(error) { constructable=false; }
  return name+':'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
    (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable;
}
print('keys='+names.join(','));
print('meta='+meta('flatMap')+'|'+meta('flat'));
"#;

#[test]
fn array_flatten_basic_rust_smoke() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval(r#"[[1,2],[3,[4]],,5].flat().join("|")"#)
        .expect("evaluate basic Array.flat smoke");
    assert_eq!(primitive_value_text(value), "1|2|3|4|5");
    let value = context
        .eval(r#"[1,2,,3].flatMap(function(x,i){return [x,i]}).join("|")"#)
        .expect("evaluate basic Array.flatMap smoke");
    assert_eq!(primitive_value_text(value), "1|0|2|1|3|3");
}

#[test]
fn array_flatten_recursive_mapper_stack_overflow_is_catchable_without_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval(
            r#"(function(){
                function nest(count){
                    return [1].flatMap(function(value){return count?nest(count-1):[value]})
                }
                try{nest(40);return "missing"}
                catch(error){return error.name+"|"+error.message}
            })()"#,
        )
        .expect("recursive Array.flatMap completion stays catchable");
    assert_eq!(primitive_value_text(value), "InternalError|stack overflow",);
}

#[test]
fn array_flatten_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array flatten oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("species", SPECIES_CASES),
        ("errors", ERROR_CASES),
        ("recursion", RECURSION_CASES),
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
fn array_flatten_values_depth_holes_and_mapper_match_pinned_quickjs() {
    compare_value_cases("Array flatten values", VALUE_CASES);
}

#[test]
fn array_flatten_observable_order_matches_pinned_quickjs() {
    compare_value_cases("Array flatten order", ORDER_CASES);
}

#[test]
fn array_flatten_species_partial_writes_and_errors_match_pinned_quickjs() {
    compare_value_cases("Array flatten species", SPECIES_CASES);
    compare_value_cases("Array flatten errors", ERROR_CASES);
}

#[test]
fn array_flatten_recursion_guard_matches_pinned_completion_class() {
    compare_value_cases("Array flatten recursion", RECURSION_CASES);
}

#[test]
fn array_flatten_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array flatten graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array flatten prototype order/metadata drifted",
    );
}

#[test]
fn array_flatten_results_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let flat = property_callable(&runtime, &mut defining, &defining_array_prototype, "flat");
    let flat_map = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "flatMap",
    );

    let receiver = eval_object(&mut caller, "[[10],[20]]", "caller nested Array");
    let Value::Object(result) = caller
        .call(&flat, Value::Object(receiver.clone()), &[])
        .expect("cross-realm Array.flat call")
    else {
        panic!("cross-realm Array.flat result was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "Array.flat result did not use the native defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype),
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 10);
    assert_eq!(int_property(&runtime, &mut caller, &result, "1"), 20);

    assert!(matches!(
        caller.call(&flat_map, Value::Object(receiver.clone()), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.flatMap mapper TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.flatMap native TypeError did not use the method defining realm",
    );

    let callback = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw new TypeError('caller mapper')})",
        "caller throwing mapper",
    );
    assert!(matches!(
        caller.call(
            &flat_map,
            Value::Object(receiver),
            &[Value::Object(callback.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.flatMap user TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.flatMap replaced a user mapper throw with a defining-realm error",
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
    let implemented = ["copyWithin", "flatMap", "flat", "values", "keys", "entries"];
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
        .filter(|name| implemented.contains(&name.as_str()))
        .collect::<Vec<_>>();
    vec![
        format!("keys={}", names.join(",")),
        format!(
            "meta={}|{}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                "flatMap",
            ),
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                "flat",
            ),
        ),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS Array flatten graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array flatten graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array flatten graph output was not UTF-8")
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
    let descriptor = runtime
        .get_own_property(owner, &key)
        .unwrap()
        .unwrap_or_else(|| panic!("missing Array.prototype.{name}"));
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(function),
        writable,
        enumerable,
        configurable,
    } = &descriptor
    else {
        panic!("Array.prototype.{name} was not a function data property");
    };
    let callable = runtime
        .as_callable(function)
        .unwrap()
        .unwrap_or_else(|| panic!("Array.prototype.{name} was not callable"));
    let function_name = context
        .get_property(function, &runtime.intern_property_key("name").unwrap())
        .unwrap();
    let function_length = context
        .get_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap();
    let name_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("name").unwrap())
        .unwrap()
        .unwrap_or_else(|| panic!("Array.{name} name descriptor was missing"));
    let length_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap()
        .unwrap_or_else(|| panic!("Array.{name} length descriptor was missing"));
    format!(
        "{name}:{}:{}:D{}{}{}:{}:{}:{}:{}:{}",
        primitive_value_text(function_name),
        primitive_value_text(function_length),
        Number(*writable),
        Number(*enumerable),
        Number(*configurable),
        data_descriptor_bits(&name_descriptor),
        data_descriptor_bits(&length_descriptor),
        true,
        runtime.get_prototype_of(function).unwrap().as_ref() == Some(function_prototype),
        runtime.is_constructor(callable.as_object()).unwrap(),
    )
}

fn data_descriptor_bits(descriptor: &CompleteOrdinaryPropertyDescriptor) -> String {
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    } = descriptor
    else {
        panic!("expected a data descriptor");
    };
    format!(
        "D{}{}{}",
        Number(*writable),
        Number(*enumerable),
        Number(*configurable),
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

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"))
    else {
        panic!("Rust {description} did not evaluate to an object");
    };
    object
}

fn eval_callable(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> CallableRef {
    let object = eval_object(context, source, description);
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("Rust {description} was not callable"))
}

fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
    let Value::Object(error) = context
        .take_exception()
        .unwrap_or_else(|failure| panic!("take {description}: {failure}"))
        .unwrap_or_else(|| panic!("{description} was missing"))
    else {
        panic!("{description} was not an object");
    };
    error
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Int(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an Int property");
    };
    value
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
