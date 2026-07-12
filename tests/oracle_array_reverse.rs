use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04 `Array.prototype.reverse` and
// `Array.prototype.toReversed`. The former mutates generic array-like receivers
// pair by pair; the latter is a dense defining-realm base-Array copy and never
// consults constructor or @@species.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense reverse mutates in place while toReversed leaves its source unchanged",
        r#"(function(){
            var source=[1,2,3,4],same=source.reverse()===source;
            var copy=source.toReversed();
            return same+"|"+source.length+"|"+source[0]+source[1]+source[2]+source[3]+"|"+
                copy.length+"|"+copy[0]+copy[1]+copy[2]+copy[3]+"|"+
                source[0]+source[1]+source[2]+source[3]+"|"+Array.isArray(copy);
        })()"#,
    ),
    (
        "toReversed copies inherited values and densifies holes",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            Array.prototype[2]="inherited";
            var source=Array(4);source[0]="a";source[3]="d";
            var result=source.toReversed();
            var observation=result.length+"|"+result[0]+"|"+result[1]+"|"+
                (result[2]===undefined)+"|"+result[3]+"|"+
                own(result,0)+own(result,1)+own(result,2)+own(result,3)+"|"+
                own(source,1)+own(source,2);
            delete Array.prototype[2];
            return observation;
        })()"#,
    ),
    (
        "sparse reverse preserves holes through all presence combinations",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=Array(7);source[0]="a";source[2]="c";source[6]="g";
            source.reverse();
            return source.length+"|"+source[0]+"|"+own(source,1)+"|"+
                source[4]+"|"+own(source,5)+"|"+source[6]+"|"+
                own(source,0)+own(source,2)+own(source,4)+own(source,6);
        })()"#,
    ),
];

const REVERSE_ORDER_CASES: &[(&str, &str)] = &[
    (
        "reverse reads both ends before applying the four presence branches",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=Object(),descriptor=Object(),log="",lowerWrite,upperWrite;
            source.length=8;
            descriptor.get=function(){log+="g0";return "L0"};
            descriptor.set=function(value){log+="s0"+value;lowerWrite=value};
            descriptor.enumerable=true;descriptor.configurable=true;
            Object.defineProperty(source,"0",descriptor);
            descriptor=Object();
            descriptor.get=function(){log+="g7";return "U7"};
            descriptor.set=function(value){log+="s7"+value;upperWrite=value};
            descriptor.enumerable=true;descriptor.configurable=true;
            Object.defineProperty(source,"7",descriptor);
            source[6]="U6";
            source[2]="L2";
            var same=Array.prototype.reverse.call(source)===source;
            return same+"|"+log+"|"+lowerWrite+"|"+upperWrite+"|"+
                own(source,1)+":"+source[1]+":"+own(source,6)+"|"+
                own(source,2)+":"+own(source,5)+":"+source[5]+"|"+
                own(source,3)+":"+own(source,4);
        })()"#,
    ),
    (
        "failed second Set preserves the first Set",
        r#"(function(){
            var source=Object(),descriptor=Object();source.length=2;source[0]="lower";
            descriptor.value="upper";descriptor.writable=false;
            descriptor.enumerable=true;descriptor.configurable=false;
            Object.defineProperty(source,"1",descriptor);
            try{Array.prototype.reverse.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source[0]+"|"+source[1]}
        })()"#,
    ),
    (
        "failed upper Delete preserves the lower Set",
        r#"(function(){
            var source=Object(),descriptor=Object();source.length=2;
            descriptor.value="upper";descriptor.writable=true;
            descriptor.enumerable=true;descriptor.configurable=false;
            Object.defineProperty(source,"1",descriptor);
            try{Array.prototype.reverse.call(source);return "missing"}
            catch(error){
                return error.name+"|"+error.message+"|"+source[0]+"|"+source[1]+"|"+
                    Object.prototype.hasOwnProperty.call(source,"0");
            }
        })()"#,
    ),
    (
        "reverse snapshots length before indexed getters mutate it",
        r#"(function(){
            var source=Object(),descriptor=Object(),reported=3,log="",lowWrite,highWrite;
            descriptor.get=function(){log+="L";return reported};descriptor.configurable=true;
            Object.defineProperty(source,"length",descriptor);
            descriptor=Object();
            descriptor.get=function(){log+="g0";reported=5;source[4]="ignored";return "low"};
            descriptor.set=function(value){log+="s0";lowWrite=value};descriptor.configurable=true;
            Object.defineProperty(source,"0",descriptor);
            descriptor=Object();
            descriptor.get=function(){log+="g2";return "high"};
            descriptor.set=function(value){log+="s2";highWrite=value};descriptor.configurable=true;
            Object.defineProperty(source,"2",descriptor);
            Array.prototype.reverse.call(source);
            var beforeLengthRead=log;
            return beforeLengthRead+"|"+source.length+"|"+log+"|"+
                lowWrite+"|"+highWrite+"|"+source[4]+"|"+(3 in source);
        })()"#,
    ),
    (
        "MAX_SAFE length uses the full u64 first pair before failing",
        r#"(function(){
            var source=Object(),descriptor=Object(),log="";source.length=9007199254740991;
            descriptor.get=function(){log+="L";return "low"};descriptor.configurable=true;
            Object.defineProperty(source,"0",descriptor);
            descriptor=Object();
            descriptor.get=function(){log+="U";return "upper"};descriptor.configurable=true;
            Object.defineProperty(source,"9007199254740990",descriptor);
            try{Array.prototype.reverse.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
];

const COPY_ORDER_CASES: &[(&str, &str)] = &[
    (
        "toReversed performs descending Has and Get queries",
        r#"(function(){
            var source=Object(),log="";source.length=4;
            source.__defineGetter__("0",function(){log+="0";return "a"});
            source.__defineGetter__("1",function(){log+="1";return "b"});
            source.__defineGetter__("2",function(){log+="2";return "c"});
            source.__defineGetter__("3",function(){log+="3";return "d"});
            var result=Array.prototype.toReversed.call(source);
            return log+"|"+result[0]+result[1]+result[2]+result[3];
        })()"#,
    ),
    (
        "descending getters observe mutations from earlier high indices",
        r#"(function(){
            var source=Object(),log="";source.length=3;source[0]="old";source[1]="middle";
            source.__defineGetter__("2",function(){log+="2";source[0]="new";return "tail"});
            source.__defineGetter__("1",function(){log+="1";return "middle"});
            var result=Array.prototype.toReversed.call(source);
            return log+"|"+result[0]+"|"+result[1]+"|"+result[2]+"|"+source[0];
        })()"#,
    ),
    (
        "constructor and species are not consulted",
        r#"(function(){
            var log="",first=[1,2],second=[3,4],ctor=Object();
            ctor.__defineGetter__(Symbol.species,function(){log+="s";throw 81});
            first.constructor=ctor;
            second.__defineGetter__("constructor",function(){log+="c";throw 82});
            var a=first.toReversed(),b=second.toReversed();
            return a[0]+"|"+a[1]+"|"+b[0]+"|"+b[1]+"|"+log+"|"+
                Array.isArray(a)+"|"+Array.isArray(b);
        })()"#,
    ),
    (
        "INT32_MAX plus one fails before indexed access",
        r#"(function(){
            var source=Object(),log="";source.length=2147483648;
            source.__defineGetter__("2147483647",function(){log+="H";throw 83});
            source.__defineGetter__("0",function(){log+="L";throw 84});
            try{Array.prototype.toReversed.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like receivers support both methods",
        r#"(function(){
            var source=Object();source[0]="a";source[2]="c";source.length=3;
            var copy=Array.prototype.toReversed.call(source);
            var same=Array.prototype.reverse.call(source)===source;
            return copy.length+"|"+copy[0]+"|"+(copy[1]===undefined)+"|"+copy[2]+"|"+
                same+"|"+source[0]+"|"+(source[1]===undefined)+"|"+source[2];
        })()"#,
    ),
    (
        "number and boolean primitives are boxed or copied in the defining realm",
        r#"(function(){
            var reversed=Array.prototype.reverse.call(7);
            var copied=Array.prototype.toReversed.call(false);
            return Object.prototype.toString.call(reversed)+"|"+reversed.valueOf()+"|"+
                Object.prototype.hasOwnProperty.call(reversed,"length")+"|"+
                Array.isArray(copied)+"|"+copied.length;
        })()"#,
    ),
    (
        "String reverse rejects immutable indices while toReversed copies UTF-16 units",
        r#"(function(){
            var errorText;
            try{Array.prototype.reverse.call("ab");errorText="missing"}
            catch(error){errorText=error.name+":"+error.message}
            var result=Array.prototype.toReversed.call("A\uD83D\uDCA9Z");
            return errorText+"|"+result.length+"|"+
                result[0].charCodeAt(0)+","+result[1].charCodeAt(0)+","+
                result[2].charCodeAt(0)+","+result[3].charCodeAt(0)+"|"+
                Object.prototype.hasOwnProperty.call(result,"0");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "reverse null receiver",
        "Array.prototype.reverse.call(null)",
    ),
    (
        "toReversed undefined receiver",
        "Array.prototype.toReversed.call(undefined)",
    ),
    (
        "reverse Symbol length",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.reverse.call(source)})()",
    ),
    (
        "toReversed BigInt length",
        "(function(){var source=Object();source.length=1n;return Array.prototype.toReversed.call(source)})()",
    ),
    (
        "reverse getter user throw",
        "(function(){var source=Object();source.length=2;source.__defineGetter__('0',function(){throw 91});return Array.prototype.reverse.call(source)})()",
    ),
    (
        "toReversed getter user throw",
        "(function(){var source=Object();source.length=2;source.__defineGetter__('1',function(){throw 92});return Array.prototype.toReversed.call(source)})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','concat','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','join','toString','toLocaleString','pop','push','shift','unshift','reverse','toReversed','copyWithin','values','keys','entries'];
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
print('meta='+metadata('reverse'));
print('meta='+metadata('toReversed'));
"#;

#[test]
fn array_reverse_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array reverse oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("reverse order", REVERSE_ORDER_CASES),
        ("copy order", COPY_ORDER_CASES),
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
    assert_eq!(oracle_graph_observations(&oracle).len(), 3);
}

#[test]
fn array_reverse_dense_sparse_and_copy_values_match_pinned_quickjs() {
    compare_value_cases("Array reverse values", VALUE_CASES);
}

#[test]
fn array_reverse_pair_order_partial_mutation_and_u64_keys_match_pinned_quickjs() {
    compare_value_cases("Array reverse pair order", REVERSE_ORDER_CASES);
}

#[test]
fn array_to_reversed_query_order_species_and_dense_limit_match_pinned_quickjs() {
    compare_value_cases("Array.toReversed order and limits", COPY_ORDER_CASES);
}

#[test]
fn array_reverse_generic_primitive_and_string_receivers_match_pinned_quickjs() {
    compare_value_cases("Array reverse generic receivers", GENERIC_CASES);
}

#[test]
fn array_reverse_errors_match_pinned_quickjs() {
    compare_value_cases("Array reverse errors", ERROR_CASES);
}

#[test]
fn array_reverse_prototype_order_metadata_and_constructability_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array reverse graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array reverse prototype order/metadata drifted",
    );
}

#[test]
fn array_reverse_boxing_results_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
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
    let defining_range_error = eval_object(
        &mut defining,
        "RangeError.prototype",
        "defining RangeError prototype",
    );
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let reverse = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "reverse",
    );
    let to_reversed = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "toReversed",
    );

    let Value::Object(boxed) = caller
        .call(&reverse, Value::Int(7), &[])
        .expect("cross-realm primitive Array.reverse")
    else {
        panic!("cross-realm primitive Array.reverse did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed).unwrap(),
        Some(defining_number_prototype),
        "Array.reverse boxed its primitive receiver outside the defining realm",
    );

    let receiver = eval_object(&mut caller, "[1,2,3]", "caller Array receiver");
    let Value::Object(result) = caller
        .call(&to_reversed, Value::Object(receiver), &[])
        .expect("cross-realm Array.toReversed")
    else {
        panic!("cross-realm Array.toReversed did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "Array.toReversed result did not use the native defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype),
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 3);
    assert_eq!(int_property(&runtime, &mut caller, &result, "2"), 1);

    let too_long = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=2147483648;return source})()",
        "caller oversized array-like",
    );
    assert!(matches!(
        caller.call(&to_reversed, Value::Object(too_long), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.toReversed RangeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_range_error),
        "Array.toReversed RangeError did not use the method defining realm",
    );

    let fixed_upper = eval_object(
        &mut caller,
        "(function(){var source=Object(),descriptor=Object();source.length=2;descriptor.value='upper';descriptor.writable=true;descriptor.enumerable=true;descriptor.configurable=false;Object.defineProperty(source,'1',descriptor);return source})()",
        "caller non-configurable reverse upper",
    );
    assert!(matches!(
        caller.call(&reverse, Value::Object(fixed_upper), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.reverse Delete TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.reverse native TypeError did not use the method defining realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=2;source.__defineGetter__('0',function(){throw new TypeError('caller getter')});return source})()",
        "caller throwing reverse receiver",
    );
    assert!(matches!(
        caller.call(&reverse, Value::Object(throwing_receiver), &[]),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.reverse user getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error.clone()),
        "Array.reverse replaced a user getter throw with a defining-realm error",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=2;source.__defineGetter__('1',function(){throw new TypeError('caller copy getter')});return source})()",
        "caller throwing toReversed receiver",
    );
    assert!(matches!(
        caller.call(&to_reversed, Value::Object(throwing_receiver), &[]),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.toReversed user getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.toReversed replaced a user getter throw with a defining-realm error",
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
        "reverse",
        "toReversed",
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
    for name in ["reverse", "toReversed"] {
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
            panic!("could not run QuickJS Array reverse graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array reverse graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array reverse graph output was not UTF-8")
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
