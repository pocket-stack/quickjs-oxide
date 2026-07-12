use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This target pins QuickJS 2026-06-04's `js_array_concat`, including
// `JS_ArraySpeciesCreate`, `@@isConcatSpreadable`, holes, and final length Set.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense Arrays and primitive arguments concatenate left to right",
        r#"(function(){
            var source=[1,2],other=[3,4];
            var result=source.concat(other,5,"x",null,undefined);
            return result.length+"|"+result[0]+"|"+result[1]+"|"+result[2]+"|"+
                result[3]+"|"+result[4]+"|"+result[5]+"|"+result[6]+"|"+
                (result[7]===undefined)+"|"+(result!==source)+"|"+source.length+"|"+other.length;
        })()"#,
    ),
    (
        "sparse Arrays preserve holes from every spread source",
        r#"(function(){
            var source=[1,,3],other=[4,,6],result=source.concat(other);
            return result.length+"|"+result[0]+"|"+(1 in result)+"|"+result[2]+"|"+
                result[3]+"|"+(4 in result)+"|"+result[5];
        })()"#,
    ),
    (
        "concat with no arguments copies only the receiver",
        r#"(function(){
            var source=[1,2],result=source.concat();
            result[0]=9;
            return result.length+"|"+result[0]+"|"+source[0]+"|"+(result!==source);
        })()"#,
    ),
    (
        "explicit undefined remains distinct from no argument",
        r#"(function(){
            var empty=[].concat(),one=[].concat(undefined);
            return empty.length+"|"+one.length+"|"+(0 in one)+"|"+(one[0]===undefined);
        })()"#,
    ),
    (
        "nested Arrays are flattened by exactly one level",
        r#"(function(){
            var nested=[[2,3]],result=[1].concat(nested);
            return result.length+"|"+result[0]+"|"+Array.isArray(result[1])+"|"+
                result[1].length+"|"+result[1][0]+"|"+result[1][1];
        })()"#,
    ),
];

const SPREAD_CASES: &[(&str, &str)] = &[
    (
        "Arrays can opt out and ordinary objects can opt into spreading",
        r#"(function(){
            var hidden=[2,3],ordinary=Object(),nulled=[6,7];
            hidden[Symbol.isConcatSpreadable]=false;
            ordinary[0]=4;ordinary[1]=5;ordinary.length=2;
            ordinary[Symbol.isConcatSpreadable]=true;
            nulled[Symbol.isConcatSpreadable]=null;
            var result=[1].concat(hidden,ordinary,nulled);
            return result.length+"|"+result[0]+"|"+(result[1]===hidden)+"|"+
                result[2]+"|"+result[3]+"|"+(result[4]===nulled);
        })()"#,
    ),
    (
        "object spreadability is truthy without coercion hooks",
        r#"(function(){
            var log="",marker=Object(),source=Object();
            marker.valueOf=function(){log+="v";throw 61};
            marker.toString=function(){log+="s";throw 62};
            source[0]="x";source.length=1;source[Symbol.isConcatSpreadable]=marker;
            var result=[].concat(source);
            return result.length+"|"+result[0]+"|"+log;
        })()"#,
    ),
    (
        "holes skip Get while inherited values become own result properties",
        r#"(function(){
            var proto=Object.create(Array.prototype),source=Array(3);
            proto[1]="p";source[2]="z";Object.setPrototypeOf(source,proto);
            source.__defineGetter__("0",function(){throw 63});delete source[0];
            var result=source.concat();
            return result.length+"|"+(0 in result)+"|"+result[1]+"|"+result[2]+"|"+
                Object.prototype.hasOwnProperty.call(result,"1");
        })()"#,
    ),
    (
        "species allocation precedes spreadability length and indexed access",
        r#"(function(){
            var log="",source=[1],ctor=Object(),speciesDescriptor=Object();
            var resultProto=Object();
            resultProto.__defineSetter__("length",function(value){log+="L"+value});
            function Species(length){log+="N"+length;return Object.create(resultProto)}
            speciesDescriptor.get=function(){log+="S";return Species};
            Object.defineProperty(ctor,Symbol.species,speciesDescriptor);
            source.__defineGetter__("constructor",function(){log+="C";return ctor});
            var spreadDescriptor=Object();
            spreadDescriptor.get=function(){log+="I";return undefined};
            Object.defineProperty(source,Symbol.isConcatSpreadable,spreadDescriptor);
            source.__defineGetter__("0",function(){log+="G";return 4});
            var result=source.concat(5);
            return result[0]+"|"+result[1]+"|"+log;
        })()"#,
    ),
    (
        "spread length is snapshotted while later Has and Get see mutation",
        r#"(function(){
            var source=Object(),log="";source.length=4;
            source[Symbol.isConcatSpreadable]=true;
            source.__defineGetter__("0",function(){
                log+="g0,";delete source[1];source[2]=3;source[4]=5;source.length=5;return 1;
            });
            source[1]=2;source[3]=4;
            var result=[].concat(source);
            return result.length+"|"+result[0]+"|"+(1 in result)+"|"+result[2]+"|"+
                result[3]+"|"+(4 in result)+"|"+log+"|"+source.length;
        })()"#,
    ),
    (
        "an indexed getter throw stops before later arguments and final length Set",
        r#"(function(){
            var log="",source=Object(),later=Object();source.length=2;
            source[Symbol.isConcatSpreadable]=true;
            source.__defineGetter__("0",function(){log+="G";throw 71});
            later.__defineGetter__(Symbol.isConcatSpreadable,function(){log+="L";return false});
            try{[].concat(source,later);return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const SPECIES_CASES: &[(&str, &str)] = &[
    (
        "custom species receive zero and ordinary results get a final length",
        r#"(function(){
            function Species(length){var result=Object();result.arg=length;return result}
            var ctor=Object(),source=[1,,3];ctor[Symbol.species]=Species;source.constructor=ctor;
            var result=source.concat(4),first=result[0];
            result[0]=8;var writable=result[0]===8;
            var enumerable=Object.prototype.propertyIsEnumerable.call(result,"0");
            var configurable=delete result[0];
            return result.arg+"|"+result.length+"|"+first+"|"+(1 in result)+"|"+
                result[2]+"|"+result[3]+"|"+writable+":"+enumerable+":"+configurable;
        })()"#,
    ),
    (
        "a custom result property under a source hole is not deleted",
        r#"(function(){
            function Species(){var result=Object();result[1]="seed";return result}
            var ctor=Object(),source=[1,,3];ctor[Symbol.species]=Species;source.constructor=ctor;
            var result=source.concat();
            return result.length+"|"+result[0]+"|"+result[1]+"|"+result[2];
        })()"#,
    ),
    (
        "indexed defines bypass setters while final length uses an inherited setter",
        r#"(function(){
            var indexHits=0,lengthLog="",proto=Object(),ctor=Object();
            proto.__defineSetter__("0",function(){indexHits++});
            proto.__defineSetter__("length",function(value){lengthLog+="L"+value});
            function Species(){return Object.create(proto)}
            ctor[Symbol.species]=Species;
            var source=[2];source.constructor=ctor;
            var result=source.concat(3);
            return result[0]+"|"+result[1]+"|"+indexHits+"|"+lengthLog+"|"+
                Object.prototype.hasOwnProperty.call(result,"0")+"|"+
                Object.prototype.hasOwnProperty.call(result,"length");
        })()"#,
    ),
    (
        "a rejected indexed definition preserves only the written prefix",
        r#"(function(){
            var captured,log="",ctor=Object();
            function Species(){
                var result=Object(),descriptor=Object();captured=result;
                descriptor.value=9;descriptor.writable=false;
                descriptor.enumerable=true;descriptor.configurable=false;
                Object.defineProperty(result,"1",descriptor);return result;
            }
            ctor[Symbol.species]=Species;
            var source=[1,2,3];source.constructor=ctor;
            source.__defineGetter__("2",function(){log+="G";throw 72});
            try{source.concat();return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+captured[0]+"|"+
                captured[1]+"|"+log+"|"+("length" in captured)}
        })()"#,
    ),
    (
        "a rejected final length Set leaves all indexed writes observable",
        r#"(function(){
            var captured,ctor=Object();
            function Species(){
                var result=Object(),descriptor=Object();captured=result;
                descriptor.value=0;descriptor.writable=false;
                descriptor.enumerable=false;descriptor.configurable=false;
                Object.defineProperty(result,"length",descriptor);return result;
            }
            ctor[Symbol.species]=Species;
            var source=[1,2];source.constructor=ctor;
            try{source.concat(3);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+captured[0]+"|"+
                captured[1]+"|"+captured[2]+"|"+captured.length}
        })()"#,
    ),
    (
        "generic receivers ignore constructor while default creation ignores global Array replacement",
        r#"(function(){
            var intrinsic=Array,concat=Array.prototype.concat,source=Object(),log="";
            source.__defineGetter__("constructor",function(){log+="C";throw 81});
            globalThis.Array=function(){throw 82};
            var result=concat.call(source);
            return intrinsic.isArray(result)+"|"+result.length+"|"+
                (result[0]===source)+"|"+log;
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receivers are single elements unless opted in",
        r#"(function(){
            var single=Object();single[0]="a";single.length=1;
            var one=Array.prototype.concat.call(single);
            single[Symbol.isConcatSpreadable]=true;
            var spread=Array.prototype.concat.call(single);
            return one.length+"|"+(one[0]===single)+"|"+spread.length+"|"+spread[0];
        })()"#,
    ),
    (
        "primitive String receiver is boxed once and primitive String arguments do not spread",
        r#"(function(){
            var result=Array.prototype.concat.call("ab","cd");
            return result.length+"|"+(typeof result[0])+"|"+
                Object.prototype.toString.call(result[0])+"|"+result[0]+"|"+
                (typeof result[1])+"|"+result[1];
        })()"#,
    ),
    (
        "boxed String objects can opt into UTF-16 spreading",
        r#"(function(){
            var source=Object("A\uD83D\uDCA9Z");
            source[Symbol.isConcatSpreadable]=true;
            var result=[].concat(source);
            return result.length+"|"+result[0].charCodeAt(0)+"|"+
                result[1].charCodeAt(0)+"|"+result[2].charCodeAt(0)+"|"+
                result[3].charCodeAt(0);
        })()"#,
    ),
    (
        "negative NaN and fractional spread lengths use ToLength",
        r#"(function(){
            function make(length,value){
                var source=Object();source.length=length;source[0]=value;
                source[Symbol.isConcatSpreadable]=true;return source;
            }
            var result=[].concat(make(-1,"n"),make(0/0,"z"),make(1.9,"f"));
            return result.length+"|"+result[0];
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER overflow is checked before indexed access",
        r#"(function(){
            var receiver=Object(),huge=Object(),log="";
            huge.length=9007199254740991;huge[Symbol.isConcatSpreadable]=true;
            huge.__defineGetter__("0",function(){log+="G";throw 91});
            try{Array.prototype.concat.call(receiver,huge);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("null receiver", "Array.prototype.concat.call(null)"),
    (
        "undefined receiver",
        "Array.prototype.concat.call(undefined)",
    ),
    (
        "primitive constructor",
        "(function(){var source=[1];source.constructor=1;return source.concat()})()",
    ),
    (
        "plain object species",
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=Object();source.constructor=ctor;return source.concat()})()",
    ),
    (
        "concat itself as species",
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=Array.prototype.concat;source.constructor=ctor;return source.concat()})()",
    ),
    (
        "Symbol spread length",
        "(function(){var value=Object();value.length=Symbol('length');value[Symbol.isConcatSpreadable]=true;return [].concat(value)})()",
    ),
    (
        "BigInt spread length",
        "(function(){var value=Object();value.length=1n;value[Symbol.isConcatSpreadable]=true;return [].concat(value)})()",
    ),
    (
        "spreadability getter throw",
        "(function(){var value=Object();value.__defineGetter__(Symbol.isConcatSpreadable,function(){throw 93});return [].concat(value)})()",
    ),
    (
        "spread length getter throw",
        "(function(){var value=Object();value[Symbol.isConcatSpreadable]=true;value.__defineGetter__('length',function(){throw 94});return [].concat(value)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','concat','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','copyWithin','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
function metadata(name) {
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
print('meta='+metadata('concat'));
"#;

#[test]
fn array_concat_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.concat oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("spreadability", SPREAD_CASES),
        ("species", SPECIES_CASES),
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
fn array_concat_values_match_pinned_quickjs() {
    compare_value_cases("Array.concat values", VALUE_CASES);
}

#[test]
fn array_concat_spreadability_holes_and_order_match_pinned_quickjs() {
    compare_value_cases("Array.concat spreadability/order", SPREAD_CASES);
}

#[test]
fn array_concat_species_and_target_writes_match_pinned_quickjs() {
    compare_value_cases("Array.concat species/results", SPECIES_CASES);
}

#[test]
fn array_concat_generic_receivers_and_limits_match_pinned_quickjs() {
    compare_value_cases("Array.concat generic receivers", GENERIC_CASES);
}

#[test]
fn array_concat_errors_match_pinned_quickjs() {
    compare_value_cases("Array.concat errors", ERROR_CASES);
}

#[test]
fn array_concat_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array.concat graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array.concat prototype order/metadata drifted",
    );
}

#[test]
fn array_concat_species_boxing_results_and_errors_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_string_prototype = defining.string_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_object_prototype = caller.object_prototype().unwrap();
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let concat = property_callable(&runtime, &mut defining, &defining_array_prototype, "concat");

    let source = eval_object(&mut caller, "[1]", "caller Array");
    let Value::Object(result) = caller
        .call(&concat, Value::Object(source), &[])
        .expect("cross-realm default Array.concat")
    else {
        panic!("cross-realm Array.concat did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "cross-realm default Array constructor was not replaced by the method realm Array",
    );

    let Value::Object(string_result) = caller
        .call(
            &concat,
            Value::String(JsString::try_from_utf8("a").unwrap()),
            &[],
        )
        .expect("cross-realm primitive Array.concat")
    else {
        panic!("primitive Array.concat did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&string_result).unwrap(),
        Some(defining_array_prototype),
        "primitive Array.concat result did not use the method realm Array",
    );
    let zero = runtime.intern_property_key("0").unwrap();
    let Value::Object(boxed) = caller.get_property(&string_result, &zero).unwrap() else {
        panic!("primitive Array.concat did not store its boxed receiver");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed).unwrap(),
        Some(defining_string_prototype),
        "Array.concat boxed its primitive receiver outside the defining realm",
    );

    let custom_source = eval_object(
        &mut caller,
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=function(length){globalThis.concatSpeciesLength=length;return Object()};source.constructor=ctor;return source})()",
        "caller Array with custom concat species",
    );
    let Value::Object(custom_result) = caller
        .call(&concat, Value::Object(custom_source), &[])
        .expect("cross-realm custom species Array.concat")
    else {
        panic!("custom species Array.concat did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_result).unwrap(),
        Some(caller_object_prototype.clone()),
        "Array.concat moved a custom species result into the method realm",
    );
    assert_eq!(
        caller.eval("concatSpeciesLength").unwrap(),
        Value::Int(0),
        "Array.concat passed a nonzero length to custom species",
    );

    caller
        .eval("globalThis.concatArgument=Object()")
        .expect("install caller concat argument");
    let argument = eval_object(&mut caller, "concatArgument", "caller concat argument");
    let empty = eval_object(&mut caller, "[]", "caller empty Array");
    let Value::Object(identity_result) = caller
        .call(
            &concat,
            Value::Object(empty.clone()),
            &[Value::Object(argument.clone())],
        )
        .expect("cross-realm object argument concat")
    else {
        panic!("object argument concat did not return an object");
    };
    let Value::Object(copied) = caller.get_property(&identity_result, &zero).unwrap() else {
        panic!("Array.concat did not retain its object argument");
    };
    assert_eq!(
        copied, argument,
        "Array.concat cloned or moved an object argument"
    );

    let bad = eval_object(
        &mut caller,
        "(function(){var source=[1];source.constructor=1;return source})()",
        "caller Array with primitive concat constructor",
    );
    assert!(matches!(
        caller.call(&concat, Value::Object(bad), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.concat species TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.concat native TypeError did not use the method defining realm",
    );

    let throwing_argument = eval_object(
        &mut caller,
        "(function(){var value=Object();value.__defineGetter__(Symbol.isConcatSpreadable,function(){throw new TypeError('caller spread getter')});return value})()",
        "caller throwing spreadability argument",
    );
    assert!(matches!(
        caller.call(
            &concat,
            Value::Object(empty),
            &[Value::Object(throwing_argument)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.concat spread getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.concat replaced a user getter throw with a defining-realm error",
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
    let implemented = [
        "at",
        "with",
        "concat",
        "every",
        "some",
        "forEach",
        "map",
        "filter",
        "reduce",
        "reduceRight",
        "fill",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "indexOf",
        "lastIndexOf",
        "includes",
        "copyWithin",
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
        .filter(|name| implemented.contains(&name.as_str()))
        .collect::<Vec<_>>();
    vec![
        format!("keys={}", names.join(",")),
        format!(
            "meta={}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                "concat",
            )
        ),
    ]
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS Array.concat graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Array.concat graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array.concat graph output was not UTF-8")
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
