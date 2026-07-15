use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, PropertyKey, Runtime,
    RuntimeError, Value, WellKnownSymbol,
};

const METHODS: &[(&str, usize)] = &[
    ("min", 2),
    ("max", 2),
    ("abs", 1),
    ("floor", 1),
    ("ceil", 1),
    ("round", 1),
    ("sqrt", 1),
    ("acos", 1),
    ("asin", 1),
    ("atan", 1),
    ("atan2", 2),
    ("cos", 1),
    ("exp", 1),
    ("log", 1),
    ("pow", 2),
    ("sin", 1),
    ("tan", 1),
    ("trunc", 1),
    ("sign", 1),
    ("cosh", 1),
    ("sinh", 1),
    ("tanh", 1),
    ("acosh", 1),
    ("asinh", 1),
    ("atanh", 1),
    ("expm1", 1),
    ("log1p", 1),
    ("log2", 1),
    ("log10", 1),
    ("cbrt", 1),
    ("hypot", 2),
    ("random", 0),
    ("f16round", 1),
    ("fround", 1),
    ("imul", 2),
    ("clz32", 1),
    ("sumPrecise", 1),
];

const CONSTANTS: &[&str] = &[
    "E", "LN10", "LN2", "LOG2E", "LOG10E", "PI", "SQRT1_2", "SQRT2",
];

const GRAPH_ORACLE: &str = r#"
function bits(descriptor) {
    return "D:" + Number(descriptor.writable) +
           Number(descriptor.enumerable) + Number(descriptor.configurable);
}
function isConstructor(value) {
    try { Reflect.construct(function(){}, [], value); return true; }
    catch (_) { return false; }
}
function methodMeta(key) {
    var descriptor = Object.getOwnPropertyDescriptor(Math, key);
    var value = descriptor.value;
    return key + ":" + value.name + ":" + value.length + ":" +
           (Object.getPrototypeOf(value) === Function.prototype) + ":" +
           (typeof value === "function") + ":" + isConstructor(value) + ":" +
           bits(descriptor) + ":" + bits(Object.getOwnPropertyDescriptor(value, "name")) +
           ":" + bits(Object.getOwnPropertyDescriptor(value, "length"));
}
var methods = [
    "min","max","abs","floor","ceil","round","sqrt",
    "acos","asin","atan","atan2","cos","exp","log","pow","sin","tan",
    "trunc","sign","cosh","sinh","tanh","acosh","asinh","atanh",
    "expm1","log1p","log2","log10","cbrt","hypot","random",
    "f16round","fround","imul","clz32","sumPrecise"
];
var constants = ["E","LN10","LN2","LOG2E","LOG10E","PI","SQRT1_2","SQRT2"];
var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "Math");
print("graph=" + [
    globalDescriptor.value === Math,
    Object.getPrototypeOf(Math) === Object.prototype,
    Object.prototype.toString.call(Math),
    Object.isExtensible(Math),
    bits(globalDescriptor)
].join("|"));
print("keys=" + Reflect.ownKeys(Math).map(String).join(","));
print("methods=" + methods.map(methodMeta).join("|"));
print("constants=" + constants.map(function(key) {
    var descriptor = Object.getOwnPropertyDescriptor(Math, key);
    return key + ":" + String(descriptor.value) + ":" + bits(descriptor);
}).join("|"));
var tag = Object.getOwnPropertyDescriptor(Math, Symbol.toStringTag);
print("tag=" + String(tag.value) + ":" + bits(tag));
"#;

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "classic unary functions preserve their pinned boundaries",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.abs(-3.5)),show(Math.abs(-0)),show(Math.floor(-1.25)),
                show(Math.ceil(-1.25)),show(Math.round(-1.5)),show(Math.sqrt(9)),
                show(Math.sqrt(-1)),show(Math.acos(1)),show(Math.asin(0)),
                show(Math.atan(Infinity)),show(Math.atan(-Infinity)),
                show(Math.cos(0)),show(Math.exp(0)),show(Math.log(1)),
                show(Math.sin(-0)),show(Math.tan(-0))
            ];
        })()"#,
    ),
    (
        "ES2015 unary functions preserve signed zero NaN and infinity",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.trunc(-1.9)),show(Math.trunc(-0.1)),
                show(Math.sign(-4)),show(Math.sign(-0)),show(Math.sign(NaN)),
                show(Math.cosh(0)),show(Math.sinh(-0)),show(Math.tanh(Infinity)),
                show(Math.tanh(-Infinity)),show(Math.acosh(1)),show(Math.acosh(0)),
                show(Math.asinh(-0)),show(Math.atanh(-0)),show(Math.atanh(1)),
                show(Math.expm1(-0)),show(Math.log1p(-0)),show(Math.log1p(-2)),
                show(Math.log2(8)),show(Math.log10(1000)),show(Math.cbrt(-8)),
                show(Math.cbrt(-0))
            ];
        })()"#,
    ),
    (
        "round implements ties toward positive infinity and large-number edges",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.round(-0)),show(Math.round(-0.1)),show(Math.round(-0.5)),
                show(Math.round(-0.5000000000000001)),show(Math.round(0.49999999999999994)),
                show(Math.round(0.5)),show(Math.round(1.5)),show(Math.round(-1.5)),
                show(Math.round(4503599627370495.5)),
                show(Math.round(-4503599627370495.5)),
                show(Math.round(NaN)),show(Math.round(Infinity)),show(Math.round(-Infinity))
            ];
        })()"#,
    ),
    (
        "atan2 quadrants and signed zeros match QuickJS libm dispatch",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.atan2(0,0)),show(Math.atan2(-0,0)),
                show(Math.atan2(0,-0)),show(Math.atan2(-0,-0)),
                show(Math.atan2(Infinity,Infinity)),show(Math.atan2(-Infinity,Infinity)),
                show(Math.atan2(Infinity,-Infinity)),show(Math.atan2(-Infinity,-Infinity)),
                show(Math.atan2(NaN,1)),show(Math.atan2(1,NaN))
            ];
        })()"#,
    ),
    (
        "pow shares QuickJS unit infinity and signed-zero rules",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.pow(1,Infinity)),show(Math.pow(-1,Infinity)),
                show(Math.pow(1,-Infinity)),show(Math.pow(-1,-Infinity)),
                show(Math.pow(-0,3)),show(Math.pow(-0,2)),show(Math.pow(-0,-3)),
                show(Math.pow(-0,-2)),show(Math.pow(-2,0.5)),show(Math.pow(NaN,0)),
                show(Math.pow(2,NaN)),show(Math.pow(Infinity,-1)),
                show(Math.pow(-Infinity,-3)),show(Math.pow(-Infinity,2)),
                show(Math.pow(2,10))
            ];
        })()"#,
    ),
    (
        "min and max preserve empty identity NaN and signed zero",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.min()),show(Math.max()),show(Math.min(3,-2,7)),
                show(Math.max(3,-2,7)),show(Math.min(0,-0)),show(Math.min(-0,0)),
                show(Math.max(0,-0)),show(Math.max(-0,0)),show(Math.min(NaN,1)),
                show(Math.max(1,NaN)),show(Math.min("4",null,true)),
                show(Math.max("4",null,true))
            ];
        })()"#,
    ),
    (
        "min and max coerce every argument left to right even after NaN",
        r#"(function(){
            var log="";
            function box(name,value){
                var object=Object();
                object.valueOf=function(){log+=name;return value};
                return object;
            }
            var minimum=Math.min(box("a",3),box("b",NaN),box("c",1));
            var first=log;log="";
            var maximum=Math.max(box("d",3),box("e",NaN),box("f",9));
            var second=log;log="";
            var abrupt;
            try{
                Math.min(box("g",NaN),box("h",2),{
                    valueOf:function(){log+="i";throw "stop"}
                });
            }catch(error){abrupt=error}
            return [minimum!==minimum,first,maximum!==maximum,second,abrupt,log];
        })()"#,
    ),
    (
        "unary and binary numeric conversion is ordered and abrupt",
        r#"(function(){
            var log="";
            function box(name,value){
                var object=Object();
                object.valueOf=function(){log+=name;return value};
                return object;
            }
            var absolute=Math.abs(box("a",-4));
            var power=Math.pow(box("b",2),box("c",5));
            var angle=Math.atan2(box("d",0),box("e",-0));
            var before=log,thrown;
            try{Math.pow(box("f",2),{valueOf:function(){log+="g";throw "boom"}})}
            catch(error){thrown=error}
            var bigint,symbol;
            try{Math.abs(1n)}catch(error){bigint=error.name+":"+error.message}
            try{Math.abs(Symbol("x"))}catch(error){symbol=error.name+":"+error.message}
            return [absolute,power,angle,before,thrown,log,bigint,symbol];
        })()"#,
    ),
    (
        "f16round covers normal subnormal overflow and ties-to-even",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.f16round()),show(Math.f16round(0)),show(Math.f16round(-0)),
                show(Math.f16round(Infinity)),show(Math.f16round(-Infinity)),
                show(Math.f16round(NaN)),show(Math.f16round(65504)),
                show(Math.f16round(65520)),show(Math.f16round(0.00006103515625)),
                show(Math.f16round(5.960464477539063e-8)),
                show(Math.f16round(2.9802322387695312e-8)),
                show(Math.f16round(8.940696716308594e-8)),
                show(Math.f16round(1.00048828125)),
                show(Math.f16round(1.00146484375)),
                show(Math.f16round(-1.00048828125))
            ];
        })()"#,
    ),
    (
        "fround covers signed zero infinities and binary32 ties-to-even",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.fround()),show(Math.fround(0)),show(Math.fround(-0)),
                show(Math.fround(Infinity)),show(Math.fround(-Infinity)),show(Math.fround(NaN)),
                show(Math.fround(1.0000000596046448)),
                show(Math.fround(1.0000001788139343)),show(Math.fround(3.4028236e38)),
                show(Math.fround(1.401298464324817e-45)),
                show(Math.fround(7.006492321624085e-46))
            ];
        })()"#,
    ),
    (
        "hypot handles arity scaling infinity NaN and ordered conversion",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            var log="";
            function box(name,value){
                var object=Object();
                object.valueOf=function(){log+=name;return value};
                return object;
            }
            var converted=Math.hypot(box("a",3),box("b",4),box("c",12));
            return [
                show(Math.hypot()),show(Math.hypot(-0)),show(Math.hypot(3,4)),
                show(Math.hypot(3,4,12)),show(Math.hypot(NaN,Infinity)),
                show(Math.hypot(Infinity,NaN)),show(Math.hypot(NaN,2)),
                show(converted),log
            ];
        })()"#,
    ),
    (
        "imul and clz32 apply ToUint32 and 32-bit wrapping",
        r#"(function(){
            var log="";
            function box(name,value){
                var object=Object();
                object.valueOf=function(){log+=name;return value};
                return object;
            }
            var product=Math.imul(box("a",4294967295),box("b",5));
            var leading=Math.clz32(box("c",65536));
            return [
                Math.imul(),Math.imul(4294967295,5),Math.imul(2147483647,2),
                Math.imul(2147483648,2),Math.imul("3.9","4.1"),product,
                Math.clz32(),Math.clz32(0),Math.clz32(1),Math.clz32(-1),
                Math.clz32(65536),Math.clz32(4294967296),leading,log
            ];
        })()"#,
    ),
    (
        "random stays in range uses a 52-bit grid and advances its stream",
        r#"(function(){
            var first=Math.random(),previous=first,inRange=true,onGrid=true,changed=false;
            for(var i=0;i<128;i++){
                var value=Math.random();
                if(!(value>=0&&value<1))inRange=false;
                if(!Number.isInteger(value*4503599627370496))onGrid=false;
                if(value!==previous)changed=true;
                previous=value;
            }
            return [first>=0&&first<1,Number.isInteger(first*4503599627370496),
                    inRange,onGrid,changed];
        })()"#,
    ),
    (
        "sumPrecise handles zero accuracy NaN and infinities",
        r#"(function(){
            function show(x){
                if(x!==x)return "NaN";
                if(x===0)return 1/x===-Infinity?"-0":"+0";
                if(x===Infinity)return "+Infinity";
                if(x===-Infinity)return "-Infinity";
                return String(x);
            }
            return [
                show(Math.sumPrecise([])),show(Math.sumPrecise([0])),
                show(Math.sumPrecise([-0])),show(Math.sumPrecise([0,-0])),
                show(Math.sumPrecise([1,-1])),show(Math.sumPrecise([1e100,1,-1e100])),
                show(Math.sumPrecise([0.1,0.2,-0.3])),
                show(Math.sumPrecise([Infinity])),show(Math.sumPrecise([-Infinity])),
                show(Math.sumPrecise([Infinity,-Infinity])),show(Math.sumPrecise([NaN])),
                show(Math.sumPrecise([Infinity,NaN])),show(Math.sumPrecise([NaN,-Infinity]))
            ];
        })()"#,
    ),
    (
        "sumPrecise accepts generic iterables and caches next once",
        r#"(function(){
            var log="",index=0,iterator=Object(),iterable=Object();
            Object.defineProperty(iterator,"next",{
                configurable:true,
                get:function(){
                    log+="get-next,";
                    return function(){
                        log+="next,";
                        if(index<3)return {value:[10,20,12][index++],done:false};
                        return {value:99,done:true};
                    };
                }
            });
            iterator.return=function(){log+="return,";return {done:true}};
            iterable[Symbol.iterator]=function(){log+="iterator,";return iterator};
            return [Math.sumPrecise(iterable),log];
        })()"#,
    ),
    (
        "sumPrecise rejects non-Numbers and closes while preserving its TypeError",
        r#"(function(){
            var log="",step=0,iterator=Object(),iterable=Object(),observed;
            iterator.next=function(){
                log+="next,";
                if(step++===0)return {value:1,done:false};
                return {value:"2",done:false};
            };
            iterator.return=function(){log+="return,";throw "close boom"};
            iterable[Symbol.iterator]=function(){log+="iterator,";return iterator};
            try{Math.sumPrecise(iterable)}
            catch(error){observed=error.name+":"+error.message}
            var boxed,bigint,boolean;
            try{Math.sumPrecise([new Number(1)])}catch(error){boxed=error.name+":"+error.message}
            try{Math.sumPrecise([1n])}catch(error){bigint=error.name+":"+error.message}
            try{Math.sumPrecise([true])}catch(error){boolean=error.name+":"+error.message}
            return [observed,log,boxed,bigint,boolean];
        })()"#,
    ),
    (
        "sumPrecise does not close when iterator next itself throws",
        r#"(function(){
            var log="",iterator=Object(),iterable=Object(),observed;
            iterator.next=function(){log+="next,";throw "next boom"};
            iterator.return=function(){log+="return,";return {done:true}};
            iterable[Symbol.iterator]=function(){log+="iterator,";return iterator};
            try{Math.sumPrecise(iterable)}catch(error){observed=error}
            return [observed,log];
        })()"#,
    ),
    (
        "sumPrecise preserves the pinned 129-term wrapping quirk",
        r#"(function(){
            var values=[],x=0.000976562499999999891579782751449556599254719913005828857421875;
            for(var i=0;i<129;i++)values[i]=x;
            return [Math.sumPrecise(values),129*x];
        })()"#,
    ),
];

#[test]
fn math_intrinsic_graph_and_descriptors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Math graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Math graph, key order, descriptors, or native metadata drifted",
    );
}

#[test]
fn math_values_match_pinned_quickjs() {
    compare_value_cases("Math values", VALUE_CASES);
}

#[test]
fn math_methods_are_not_constructable() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let math = math_object(&runtime, &mut context);
    for &(name, _) in METHODS {
        let callable = property_callable(&runtime, &mut context, &math, name);
        assert!(
            !runtime.is_constructor(callable.as_object()).unwrap(),
            "Math.{name} unexpectedly carried the constructor bit",
        );
        assert!(matches!(
            context.construct(&callable, &[]),
            Err(RuntimeError::Exception)
        ));
        context
            .take_exception()
            .unwrap_or_else(|error| panic!("take new Math.{name} exception: {error}"))
            .unwrap_or_else(|| panic!("new Math.{name} did not publish an exception"));
    }
}

#[test]
fn math_native_errors_use_the_method_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
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
    assert_ne!(defining_type_error, caller_type_error);

    let math = math_object(&runtime, &mut defining);
    let abs = property_callable(&runtime, &mut defining, &math, "abs");
    let sum_precise = property_callable(&runtime, &mut defining, &math, "sumPrecise");

    let symbol = runtime.new_symbol(None).unwrap();
    assert!(matches!(
        caller.call(&abs, Value::Undefined, &[Value::Symbol(symbol)]),
        Err(RuntimeError::Exception),
    ));
    let native_conversion = take_exception_object(&mut caller, "cross-realm Math.abs TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_conversion).unwrap(),
        Some(defining_type_error.clone()),
        "Math.abs allocated its ToNumber TypeError in the calling realm",
    );

    let invalid_items = eval_object(&mut caller, "[1,'2']", "caller invalid Number iterable");
    assert!(matches!(
        caller.call(
            &sum_precise,
            Value::Undefined,
            &[Value::Object(invalid_items)],
        ),
        Err(RuntimeError::Exception),
    ));
    let native_item = take_exception_object(&mut caller, "cross-realm sumPrecise TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_item).unwrap(),
        Some(defining_type_error),
        "Math.sumPrecise allocated its item TypeError in the calling realm",
    );

    let throwing_argument = eval_object(
        &mut caller,
        "(function(){var value=Object();value.valueOf=function(){throw new TypeError('caller')};return value})()",
        "caller throwing numeric argument",
    );
    assert!(matches!(
        caller.call(&abs, Value::Undefined, &[Value::Object(throwing_argument)],),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "caller conversion TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Math.abs replaced an argument conversion throw with a native error",
    );
}

#[test]
fn math_rust_smoke_runs_without_an_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let value = context
        .eval("Math.sumPrecise([20,22])")
        .expect("execute Math smoke");
    assert_eq!(
        value.as_number().map(f64::to_bits),
        Some(42.0_f64.to_bits())
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
    let object_prototype = context.object_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let math_key = runtime.intern_property_key("Math").unwrap();
    let Value::Object(math) = context.get_property(&global, &math_key).unwrap() else {
        panic!("global Math was not an object");
    };
    let global_descriptor = data_descriptor(&runtime, &global, &math_key);
    let object_to_string = property_callable(&runtime, &mut context, &object_prototype, "toString");
    let tag_text = primitive_value_text(
        context
            .call(&object_to_string, Value::Object(math.clone()), &[])
            .unwrap(),
    );
    let mut output = vec![format!(
        "graph={}|{}|{}|{}|{}",
        matches!(&global_descriptor.0, Value::Object(value) if value == &math),
        runtime.get_prototype_of(&math).unwrap().as_ref() == Some(&object_prototype),
        tag_text,
        runtime.is_extensible(&math).unwrap(),
        data_bits(
            global_descriptor.1,
            global_descriptor.2,
            global_descriptor.3,
        ),
    )];
    output.push(format!("keys={}", own_key_names(&runtime, &math).join(","),));
    output.push(format!(
        "methods={}",
        METHODS
            .iter()
            .map(|&(name, length)| method_meta(&runtime, &math, &function_prototype, name, length,))
            .collect::<Vec<_>>()
            .join("|"),
    ));
    output.push(format!(
        "constants={}",
        CONSTANTS
            .iter()
            .map(|&name| {
                let key = runtime.intern_property_key(name).unwrap();
                let (value, writable, enumerable, configurable) =
                    data_descriptor(&runtime, &math, &key);
                format!(
                    "{name}:{}:{}",
                    primitive_value_text(value),
                    data_bits(writable, enumerable, configurable),
                )
            })
            .collect::<Vec<_>>()
            .join("|"),
    ));
    let tag_key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    let (tag, writable, enumerable, configurable) = data_descriptor(&runtime, &math, &tag_key);
    output.push(format!(
        "tag={}:{}",
        primitive_value_text(tag),
        data_bits(writable, enumerable, configurable),
    ));
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .expect("run QuickJS Math intrinsic graph oracle");
    assert!(
        output.status.success(),
        "QuickJS Math graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Math graph oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn method_meta(
    runtime: &Runtime,
    math: &ObjectRef,
    function_prototype: &ObjectRef,
    name: &str,
    expected_length: usize,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let (value, writable, enumerable, configurable) = data_descriptor(runtime, math, &key);
    let Value::Object(function) = value else {
        panic!("Math.{name} was not an object");
    };
    let callable = runtime
        .as_callable(&function)
        .unwrap()
        .unwrap_or_else(|| panic!("Math.{name} was not callable"));
    let name_key = runtime.intern_property_key("name").unwrap();
    let length_key = runtime.intern_property_key("length").unwrap();
    let (function_name, name_writable, name_enumerable, name_configurable) =
        data_descriptor(runtime, &function, &name_key);
    let (function_length, length_writable, length_enumerable, length_configurable) =
        data_descriptor(runtime, &function, &length_key);
    assert_eq!(
        function_length.as_number(),
        Some(expected_length as f64),
        "Math.{name}.length was wrong before oracle comparison",
    );
    format!(
        "{name}:{}:{}:{}:true:{}:{}:{}:{}",
        primitive_value_text(function_name),
        primitive_value_text(function_length),
        runtime.get_prototype_of(&function).unwrap().as_ref() == Some(function_prototype),
        runtime.is_constructor(callable.as_object()).unwrap(),
        data_bits(writable, enumerable, configurable),
        data_bits(name_writable, name_enumerable, name_configurable),
        data_bits(length_writable, length_enumerable, length_configurable,),
    )
}

fn math_object(runtime: &Runtime, context: &mut Context) -> ObjectRef {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("Math").unwrap();
    let Value::Object(math) = context.get_property(&global, &key).unwrap() else {
        panic!("global Math was not an object");
    };
    math
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
    let to_string_tag = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            if key == to_string_tag {
                "Symbol(Symbol.toStringTag)".to_owned()
            } else {
                runtime
                    .property_key_to_js_string(&key)
                    .unwrap()
                    .to_utf8_lossy()
            }
        })
        .collect()
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
