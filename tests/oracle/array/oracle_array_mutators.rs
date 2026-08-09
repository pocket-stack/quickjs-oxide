use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This target pins QuickJS 2026-06-04's shared `js_array_pop` and
// `js_array_push` kernels. It covers both magic selectors for each kernel:
// pop/shift and push/unshift.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense mutators return the removed value or new length",
        r#"(function(){
            var source=[1,2,3];
            var popped=source.pop(),shifted=source.shift();
            var pushed=source.push(4,5),unshifted=source.unshift(0,-1);
            return popped+"|"+shifted+"|"+pushed+"|"+unshifted+"|"+
                source.length+"|"+source[0]+"|"+source[1]+"|"+source[2]+"|"+
                source[3]+"|"+source[4];
        })()"#,
    ),
    (
        "shift and pop preserve sparse holes while removing endpoints",
        r#"(function(){
            var source=Array(4);source[1]="b";source[3]="d";
            var shifted=source.shift(),popped=source.pop();
            return (shifted===undefined)+"|"+popped+"|"+source.length+"|"+
                source[0]+"|"+(1 in source)+"|"+(2 in source)+"|"+
                Object.prototype.hasOwnProperty.call(source,"0");
        })()"#,
    ),
    (
        "push distinguishes no arguments from explicit undefined",
        r#"(function(){
            var empty=[],zero=empty.push(),one=empty.push(undefined);
            var ignored=[7,8],removed=Array.prototype.pop.call(ignored,1,2,3);
            return zero+"|"+one+"|"+empty.length+"|"+(0 in empty)+"|"+
                (empty[0]===undefined)+"|"+removed+"|"+ignored.length+"|"+ignored[0];
        })()"#,
    ),
    (
        "unshift inserts actual arguments before sparse existing elements",
        r#"(function(){
            var source=Array(3);source[1]="middle";
            var length=source.unshift("first",undefined);
            return length+"|"+source.length+"|"+source[0]+"|"+
                (source[1]===undefined)+"|"+
                Object.prototype.hasOwnProperty.call(source,"1")+"|"+
                (2 in source)+"|"+source[3]+"|"+(4 in source);
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like objects support all four generic mutators",
        r#"(function(){
            var source=Object();source[0]="a";source[1]="b";source.length=2;
            var popped=Array.prototype.pop.call(source);
            var pushed=Array.prototype.push.call(source,"c");
            var shifted=Array.prototype.shift.call(source);
            var unshifted=Array.prototype.unshift.call(source,"x","y");
            return popped+"|"+pushed+"|"+shifted+"|"+unshifted+"|"+
                source.length+"|"+source[0]+"|"+source[1]+"|"+source[2];
        })()"#,
    ),
    (
        "number primitives are boxed for indexed and length Sets",
        r#"(function(){
            var captured,value,log="";Number.prototype.length=0;
            Number.prototype.__defineSetter__("0",function(argument){
                captured=this;value=argument;log+="I";
            });
            var result=Array.prototype.push.call(7,"x");
            return result+"|"+Object.prototype.toString.call(captured)+"|"+
                captured.valueOf()+"|"+(value==="x")+"|"+captured.length+"|"+
                Object.prototype.hasOwnProperty.call(captured,"length")+"|"+
                Object.prototype.hasOwnProperty.call(captured,"0")+"|"+log;
        })()"#,
    ),
    (
        "String push writes through a prototype setter before final length fails",
        r#"(function(){
            var captured,log="",stringPrototype=Object.getPrototypeOf(Object("ab"));
            stringPrototype.__defineSetter__("2",function(value){
                captured=this;log+="S"+value;
            });
            try{Array.prototype.push.call("ab","c");return "missing"}
            catch(error){
                return error.name+"|"+error.message+"|"+log+"|"+
                    Object.prototype.toString.call(captured)+"|"+captured[0]+captured[1]+"|"+
                    captured.length+"|"+Object.prototype.hasOwnProperty.call(captured,"2");
            }
        })()"#,
    ),
    (
        "empty primitive receivers still receive a final zero length Set",
        r#"(function(){
            var numberBox,booleanBox,log="";
            Number.prototype.length=0;Boolean.prototype.length=0;
            Number.prototype.__defineSetter__("length",function(value){numberBox=this;log+="N"+value});
            Boolean.prototype.__defineSetter__("length",function(value){booleanBox=this;log+="B"+value});
            var first=Array.prototype.pop.call(3),second=Array.prototype.shift.call(false);
            return (first===undefined)+"|"+(second===undefined)+"|"+
                Object.prototype.toString.call(numberBox)+"|"+
                Object.prototype.toString.call(booleanBox)+"|"+log;
        })()"#,
    ),
];

const LENGTH_AND_ORDER_CASES: &[(&str, &str)] = &[
    (
        "push reads and coerces length before ascending Sets and final length",
        r#"(function(){
            var source=Object(),lengthValue=Object(),first=Object(),log="",descriptor=Object();
            lengthValue.valueOf=function(){log+="N";return 1.9};
            first.valueOf=function(){log+="V";throw 61};
            first.toString=function(){log+="T";throw 62};
            descriptor.get=function(){log+="L";return lengthValue};
            descriptor.set=function(value){log+="F"+value};
            Object.defineProperty(source,"length",descriptor);
            source.__defineSetter__("1",function(value){log+="1"+(value===first)});
            source.__defineSetter__("2",function(value){log+="2"+(value===undefined)});
            var result=Array.prototype.push.call(source,first,undefined);
            return result+"|"+log;
        })()"#,
    ),
    (
        "push with no arguments still performs length Get conversion and Set",
        r#"(function(){
            var source=Object(),number=Object(),descriptor=Object(),log="";
            number.valueOf=function(){log+="N";return 2.8};
            descriptor.get=function(){log+="L";return number};
            descriptor.set=function(value){log+="S"+value};
            Object.defineProperty(source,"length",descriptor);
            return Array.prototype.push.call(source)+"|"+log;
        })()"#,
    ),
    (
        "empty pop converts length then Sets zero without indexed access",
        r#"(function(){
            var source=Object(),number=Object(),descriptor=Object(),log="";
            number.valueOf=function(){log+="N";return -4};
            descriptor.get=function(){log+="L";return number};
            descriptor.set=function(value){log+="S"+value};
            Object.defineProperty(source,"length",descriptor);
            source.__defineGetter__("0",function(){log+="G";throw 71});
            var result=Array.prototype.pop.call(source);
            return (result===undefined)+"|"+log;
        })()"#,
    ),
    (
        "fractional NaN and infinite lengths use QuickJS ToLength",
        r#"(function(){
            function pushed(length){var source=Object();source.length=length;return Array.prototype.push.call(source)}
            function popped(length){var source=Object();source.length=length;return Array.prototype.pop.call(source)}
            return pushed(2.9)+"|"+pushed(0/0)+"|"+pushed(-1)+"|"+
                popped(0/0)+"|"+popped(-1)+"|"+popped(1/0);
        })()"#,
    ),
];

const MOVE_CASES: &[(&str, &str)] = &[
    (
        "shift uses Has Get Set and Delete in ascending order",
        r#"(function(){
            var source=Object(),proto=Object(),descriptor=Object(),log="";
            Object.setPrototypeOf(source,proto);
            descriptor.get=function(){log+="L";return 4};
            descriptor.set=function(value){log+="N"+value};
            Object.defineProperty(source,"length",descriptor);
            descriptor=Object();descriptor.get=function(){log+="R";return "head"};
            descriptor.set=function(value){log+="S0"+value};
            Object.defineProperty(source,"0",descriptor);
            source[1]="delete-me";
            proto.__defineGetter__("3",function(){log+="G3";return "inherited"});
            var result=Array.prototype.shift.call(source);
            return result+"|"+log+"|"+
                Object.prototype.hasOwnProperty.call(source,"1")+"|"+
                Object.prototype.hasOwnProperty.call(source,"2")+"|"+source[2]+"|"+
                (3 in source)+"|"+Object.prototype.hasOwnProperty.call(source,"3");
        })()"#,
    ),
    (
        "unshift copies backwards then writes arguments forwards",
        r#"(function(){
            var source=Object(),proto=Object(),descriptor=Object(),log="";
            Object.setPrototypeOf(source,proto);source[0]="a";source[3]="delete-me";
            descriptor.get=function(){log+="L";return 3};
            descriptor.set=function(value){log+="N"+value};
            Object.defineProperty(source,"length",descriptor);
            descriptor=Object();descriptor.get=function(){log+="G2";return "p2"};
            descriptor.set=function(value){log+="S2"+value};
            Object.defineProperty(proto,"2",descriptor);
            proto.__defineSetter__("4",function(value){log+="S4"+value});
            var result=Array.prototype.unshift.call(source,"x","y");
            return result+"|"+log+"|"+source[0]+"|"+source[1]+"|"+
                source[2]+"|"+Object.prototype.hasOwnProperty.call(source,"2")+"|"+
                Object.prototype.hasOwnProperty.call(source,"3")+"|"+
                Object.prototype.hasOwnProperty.call(source,"4");
        })()"#,
    ),
    (
        "shift snapshots length while getters can mutate later sources",
        r#"(function(){
            var source=Object(),moved,descriptor=Object(),log="";source.length=3;
            descriptor.get=function(){log+="G0";source[1]="late";source[3]="ignored";source.length=4;return "head"};
            descriptor.set=function(value){log+="S0";moved=value};
            Object.defineProperty(source,"0",descriptor);
            source[2]="tail";
            var result=Array.prototype.shift.call(source);
            return result+"|"+source.length+"|"+moved+"|"+source[1]+"|"+
                (2 in source)+"|"+source[3]+"|"+log;
        })()"#,
    ),
];

const PARTIAL_AND_LIMIT_CASES: &[(&str, &str)] = &[
    (
        "failed unshift Set preserves the already moved suffix",
        r#"(function(){
            var source=Object(),descriptor=Object();
            source[0]="a";source[1]="b";source[2]="c";source.length=3;
            descriptor.value="locked";descriptor.writable=false;
            descriptor.enumerable=true;descriptor.configurable=false;
            Object.defineProperty(source,"3",descriptor);
            try{Array.prototype.unshift.call(source,"x","y");return "missing"}
            catch(error){
                return error.name+"|"+error.message+"|"+source.length+"|"+
                    source[0]+"|"+source[1]+"|"+source[2]+"|"+source[3]+"|"+source[4];
            }
        })()"#,
    ),
    (
        "failed shift Delete preserves earlier Set and skips final length",
        r#"(function(){
            var source=Object(),descriptor=Object();source[0]="a";source.length=3;
            descriptor.value="b";descriptor.writable=true;
            descriptor.enumerable=true;descriptor.configurable=false;
            Object.defineProperty(source,"1",descriptor);
            try{Array.prototype.shift.call(source);return "missing"}
            catch(error){
                return error.name+"|"+error.message+"|"+source.length+"|"+
                    source[0]+"|"+source[1]+"|"+(2 in source);
            }
        })()"#,
    ),
    (
        "failed final length Set leaves a pushed indexed property",
        r#"(function(){
            var source=Object(),descriptor=Object();source[0]="a";
            descriptor.value=1;descriptor.writable=false;
            descriptor.enumerable=true;descriptor.configurable=true;
            Object.defineProperty(source,"length",descriptor);
            try{Array.prototype.push.call(source,"b");return "missing"}
            catch(error){
                return error.name+"|"+error.message+"|"+source.length+"|"+
                    source[0]+"|"+source[1]+"|"+
                    Object.prototype.hasOwnProperty.call(source,"1");
            }
        })()"#,
    ),
    (
        "MAX_SAFE overflow throws before indexed or final length writes",
        r#"(function(){
            var source=Object(),descriptor=Object(),log="";
            descriptor.get=function(){log+="L";return 9007199254740991};
            descriptor.set=function(value){log+="N"+value};
            Object.defineProperty(source,"length",descriptor);
            source.__defineSetter__("9007199254740991",function(){log+="W"});
            try{Array.prototype.push.call(source,"x");return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "MAX_SAFE pop and push retain full Int64 property keys",
        r#"(function(){
            var tail=Object(),head=Object();
            tail.length=9007199254740991;tail[9007199254740990]="last";
            tail[4294967294]="sentinel";
            var popped=Array.prototype.pop.call(tail);
            head.length=9007199254740990;
            var pushed=Array.prototype.push.call(head,"next");
            return popped+"|"+tail.length+"|"+
                Object.prototype.hasOwnProperty.call(tail,"9007199254740990")+"|"+
                tail[4294967294]+"|"+pushed+"|"+head.length+"|"+
                head[9007199254740990]+"|"+
                Object.prototype.hasOwnProperty.call(head,"9007199254740990");
        })()"#,
    ),
    (
        "maximum Uint32 Array length writes an ordinary key before RangeError",
        r#"(function(){
            var source=[];source.length=4294967295;
            try{source.push("x");return "missing"}
            catch(error){
                return error.name+"|"+error.message+"|"+source.length+"|"+
                    source[4294967295]+"|"+
                    Object.prototype.hasOwnProperty.call(source,"4294967295");
            }
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("pop null receiver", "Array.prototype.pop.call(null)"),
    (
        "push undefined receiver",
        "Array.prototype.push.call(undefined,1)",
    ),
    ("shift null receiver", "Array.prototype.shift.call(null)"),
    (
        "unshift undefined receiver",
        "Array.prototype.unshift.call(undefined,1)",
    ),
    (
        "Symbol length",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.pop.call(source)})()",
    ),
    (
        "BigInt length",
        "(function(){var source=Object();source.length=1n;return Array.prototype.push.call(source,2)})()",
    ),
    (
        "String pop cannot delete a non-configurable indexed property",
        "Array.prototype.pop.call('ab')",
    ),
    (
        "length getter user throw",
        "(function(){var source=Object();source.__defineGetter__('length',function(){throw 91});return Array.prototype.unshift.call(source,1)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','concat','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','join','toString','toLocaleString','pop','push','shift','unshift','copyWithin','values','keys','entries'];
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
print('meta='+metadata('pop'));
print('meta='+metadata('push'));
print('meta='+metadata('shift'));
print('meta='+metadata('unshift'));
"#;

#[test]
fn array_mutator_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array mutator oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("generic", GENERIC_CASES),
        ("length/order", LENGTH_AND_ORDER_CASES),
        ("moves", MOVE_CASES),
        ("partial/limits", PARTIAL_AND_LIMIT_CASES),
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
    assert_eq!(oracle_graph_observations(&oracle).len(), 5);
}

#[test]
fn array_mutator_values_holes_returns_and_actual_argc_match_pinned_quickjs() {
    compare_value_cases("Array mutator values", VALUE_CASES);
}

#[test]
fn array_mutator_generic_and_primitive_receivers_match_pinned_quickjs() {
    compare_value_cases("Array mutator generic receivers", GENERIC_CASES);
}

#[test]
fn array_mutator_length_coercion_and_set_order_match_pinned_quickjs() {
    compare_value_cases("Array mutator length/order", LENGTH_AND_ORDER_CASES);
}

#[test]
fn array_shift_and_unshift_move_order_match_pinned_quickjs() {
    compare_value_cases("Array shift/unshift moves", MOVE_CASES);
}

#[test]
fn array_mutator_partial_writes_and_limits_match_pinned_quickjs() {
    compare_value_cases(
        "Array mutator partial writes/limits",
        PARTIAL_AND_LIMIT_CASES,
    );
}

#[test]
fn array_mutator_errors_match_pinned_quickjs() {
    compare_value_cases("Array mutator errors", ERROR_CASES);
}

#[test]
fn array_mutator_prototype_order_metadata_and_constructability_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array mutator graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array mutator prototype order/metadata drifted",
    );
}

#[test]
fn array_mutator_boxing_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_number_prototype = eval_object(
        &mut defining,
        "Number.prototype",
        "defining Number prototype",
    );
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
    let pop = property_callable(&runtime, &mut defining, &defining_array_prototype, "pop");
    let push = property_callable(&runtime, &mut defining, &defining_array_prototype, "push");

    defining
        .eval(
            "Number.prototype.length=0;Number.prototype.__defineSetter__('0',function(value){globalThis.mutatorBox=this;globalThis.mutatorValue=value})",
        )
        .expect("install defining realm primitive push observers");
    assert_eq!(
        caller
            .call(
                &push,
                Value::Int(7),
                &[Value::String(JsString::try_from_utf8("x").unwrap())],
            )
            .expect("cross-realm primitive Array.push"),
        Value::Int(1),
    );
    let boxed = eval_object(&mut defining, "mutatorBox", "captured primitive push box");
    assert_eq!(
        runtime.get_prototype_of(&boxed).unwrap(),
        Some(defining_number_prototype),
        "Array.push boxed its primitive receiver outside the defining realm",
    );
    assert_eq!(
        defining.eval("mutatorValue").unwrap(),
        Value::String(JsString::try_from_utf8("x").unwrap()),
        "Array.push changed an argument while crossing realms",
    );

    let too_long = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=9007199254740991;return source})()",
        "caller MAX_SAFE array-like",
    );
    assert!(matches!(
        caller.call(&push, Value::Object(too_long), &[Value::Int(1)]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.push MAX_SAFE TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error.clone()),
        "Array.push native TypeError did not use the method defining realm",
    );

    let fixed_tail = eval_object(
        &mut caller,
        "(function(){var source=Object(),descriptor=Object();source.length=1;descriptor.value='tail';descriptor.writable=true;descriptor.enumerable=true;descriptor.configurable=false;Object.defineProperty(source,'0',descriptor);return source})()",
        "caller non-configurable pop tail",
    );
    assert!(matches!(
        caller.call(&pop, Value::Object(fixed_tail), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.pop Delete TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.pop native TypeError did not use the method defining realm",
    );

    let throwing_length = eval_object(
        &mut caller,
        "(function(){var source=Object();source.__defineGetter__('length',function(){throw new TypeError('caller length')});return source})()",
        "caller throwing mutator length",
    );
    assert!(matches!(
        caller.call(&push, Value::Object(throwing_length), &[Value::Int(1)]),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.push length getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.push replaced a user getter throw with a defining-realm error",
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
        "join",
        "toString",
        "toLocaleString",
        "pop",
        "push",
        "shift",
        "unshift",
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
    let mut observations = vec![format!("keys={}", names.join(","))];
    for name in ["pop", "push", "shift", "unshift"] {
        observations.push(format!(
            "meta={}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                name,
            )
        ));
    }
    observations
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS Array mutator graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array mutator graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array mutator graph output was not UTF-8")
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
