use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This target pins QuickJS 2026-06-04's `js_array_join` and
// `js_array_toString`, including UTF-16 assembly, locale dispatch, recursive
// array stringification, and the intrinsic Object.prototype.toString fallback.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "dense sparse nullish and nested values stringify without flattening",
        r#"(function(){
            var nested=[2,null,,4],source=[1,,null,undefined,nested];
            return source.join("|")+"~"+source.toString()+"~"+source.toLocaleString();
        })()"#,
    ),
    (
        "default undefined null empty and multi-code-unit separators stay distinct",
        r#"(function(){
            var source=[1,2,3];
            return source.join()+"|"+source.join(undefined)+"|"+source.join(null)+"|"+
                source.join("")+"|"+source.join("::");
        })()"#,
    ),
    (
        "join preserves every UTF-16 code unit in elements and separator",
        r#"(function(){
            function codes(value){
                var result="";
                for(var i=0;i<value.length;i++){
                    if(i)result+=",";
                    result+=value.charCodeAt(i);
                }
                return result;
            }
            var value=["A\uD83D\uDCA9","\uD800","Z"].join("\uDC00\uD83D\uDCA9");
            return value.length+"|"+codes(value);
        })()"#,
    ),
    (
        "toString observes a replaced join on an Array",
        r#"(function(){
            var log="",source=[1,2];
            source.join=function(argument){log+=(this===source)+":"+(argument===undefined);return "custom"};
            return source.toString()+"|"+log;
        })()"#,
    ),
    (
        "locale stringification covers primitive and nested elements",
        r#"(function(){
            var symbol=Symbol("s"),nested=[2,3];
            return [1,"x",true,4n,symbol,nested].toLocaleString();
        })()"#,
    ),
    (
        "deep acyclic nesting is not mistaken for recursive overflow",
        r#"(function(){
            var value="x";
            for(var i=0;i<20;i++)value=[value];
            return value.join();
        })()"#,
    ),
];

const ORDER_AND_GENERIC_CASES: &[(&str, &str)] = &[
    (
        "length then separator then indexed Get and conversion have pinned order",
        r#"(function(){
            var log="",proto=Object(),source=Object.create(proto),separator=Object();
            source.__defineGetter__("length",function(){log+="L";return 3});
            separator.toString=function(){log+="S";return "|"};
            source.__defineGetter__("0",function(){
                var element=Object();log+="G0";
                element.toString=function(){
                    log+="C0";
                    var descriptor=Object();descriptor.value="late";descriptor.configurable=true;
                    Object.defineProperty(source,"2",descriptor);
                    source[3]="ignored";
                    return "a";
                };
                return element;
            });
            proto.__defineGetter__("1",function(){
                var element=Object();log+="G1";
                element.toString=function(){log+="C1";return "p"};
                return element;
            });
            source.__defineGetter__("2",function(){log+="G2";return "old"});
            var result=Array.prototype.join.call(source,separator);
            return result+"|"+log+"|"+(source[3]==="ignored");
        })()"#,
    ),
    (
        "zero length still converts an explicit separator after length",
        r#"(function(){
            var log="",source=Object(),separator=Object();
            source.__defineGetter__("length",function(){log+="L";return 0});
            separator.toString=function(){log+="S";return "-"};
            return Array.prototype.join.call(source,separator)+"|"+log;
        })()"#,
    ),
    (
        "holes use ordinary Get and therefore see inherited indexed values",
        r#"(function(){
            var proto=Object(),source=Object.create(proto);
            source.length=4;proto[0]="p0";proto[2]="p2";source[3]="own";
            return Array.prototype.join.call(source,"|");
        })()"#,
    ),
    (
        "String primitives are boxed and traversed as UTF-16 array-likes",
        r#"(function(){
            function codes(value){
                var result="";
                for(var i=0;i<value.length;i++){
                    if(i)result+=",";
                    result+=value.charCodeAt(i);
                }
                return result;
            }
            var value=Array.prototype.join.call("A\uD83D\uDCA9Z","|");
            return value.length+"|"+codes(value);
        })()"#,
    ),
    (
        "number boolean bigint and symbol receivers have zero array-like length",
        r#"(function(){
            return Array.prototype.join.call(3)+"|"+
                Array.prototype.join.call(false)+"|"+
                Array.prototype.join.call(4n)+"|"+
                Array.prototype.join.call(Symbol("receiver"));
        })()"#,
    ),
    (
        "fractional negative and NaN lengths use ToLength once",
        r#"(function(){
            function observed(length){var value=Object();value.length=length;value[0]="a";value[1]="b";return Array.prototype.join.call(value,"|")}
            return observed(2.9)+"~"+observed(-2)+"~"+observed(0/0);
        })()"#,
    ),
];

const TO_STRING_CASES: &[(&str, &str)] = &[
    (
        "a callable join result is returned without conversion",
        r#"(function(){
            var objectResult=Object(),symbolResult=Symbol("result"),log="";
            var first=Object(),second=Object();
            first.join=function(argument){log+=(this===first)+":"+(argument===undefined);return objectResult};
            second.join=function(){return symbolResult};
            return (Array.prototype.toString.call(first)===objectResult)+"|"+
                (Array.prototype.toString.call(second)===symbolResult)+"|"+log;
        })()"#,
    ),
    (
        "a non-callable join uses intrinsic Object toString and toStringTag",
        r#"(function(){
            var source=Object(),log="",saved=Object.prototype.toString;
            source.join=1;
            source.__defineGetter__(Symbol.toStringTag,function(){log+="T";return "Tagged"});
            Object.prototype.toString=function(){throw 31};
            var result=Array.prototype.toString.call(source);
            Object.prototype.toString=saved;
            return result+"|"+log;
        })()"#,
    ),
    (
        "an Array with a non-callable join uses the intrinsic Array tag",
        r#"(function(){
            var source=[];source.join=null;
            return Array.prototype.toString.call(source);
        })()"#,
    ),
    (
        "toString boxes a primitive before reading join",
        r#"(function(){
            var saved=Number.prototype.join,log="";
            Number.prototype.join=function(argument){log+=(typeof this)+":"+this.valueOf()+":"+(argument===undefined);return 17};
            var result=Array.prototype.toString.call(6);
            if(saved===undefined)delete Number.prototype.join;else Number.prototype.join=saved;
            return result+"|"+log;
        })()"#,
    ),
];

const LOCALE_CASES: &[(&str, &str)] = &[
    (
        "locale ignores arguments calls methods with zero args and ToStrings results",
        r#"(function(){
            var log="",element=Object(),ignored=Object();
            ignored.toString=function(){log+="I";throw 41};
            element.__defineGetter__("toLocaleString",function(){
                log+="G";
                return function(argument){
                    log+="M"+(argument===undefined)+":"+(this===element);
                    var result=Object();result.toString=function(){log+="C";return "localized"};
                    return result;
                };
            });
            var result=Array.prototype.toLocaleString.call([element,null,undefined],ignored);
            return result+"|"+log;
        })()"#,
    ),
    (
        "locale snapshots length and observes later indexed mutations",
        r#"(function(){
            var log="",source=Object();source.length=2;
            var first=Object();source[0]=first;
            first.toLocaleString=function(){
                var second=Object(),third=Object();log+="M0";
                second.toLocaleString=function(){log+="M1";return "late"};
                third.toLocaleString=function(){log+="M2";return "ignored"};
                source[1]=second;source[2]=third;
                source.length=3;return "first";
            };
            return Array.prototype.toLocaleString.call(source)+"|"+log+"|"+source.length;
        })()"#,
    ),
    (
        "locale uses inherited indexed values and Get on holes",
        r#"(function(){
            var log="",proto=Object(),source=Object.create(proto);source.length=2;
            proto.__defineGetter__("0",function(){
                var element=Object();log+="G";
                element.toLocaleString=function(){log+="M";return "p"};
                return element;
            });
            return Array.prototype.toLocaleString.call(source)+"|"+log;
        })()"#,
    ),
    (
        "locale method null and undefined results are ToStringed rather than skipped",
        r#"(function(){
            var first=Object(),second=Object();
            first.toLocaleString=function(){return null};
            second.toLocaleString=function(){return undefined};
            return [first,second].toLocaleString();
        })()"#,
    ),
    (
        "locale preserves UTF-16 returned by element conversion",
        r#"(function(){
            var element=Object();element.toLocaleString=function(){return "\uD800\uD83D\uDCA9"};
            var value=[element,element].toLocaleString(),result=value.length+"";
            for(var i=0;i<value.length;i++)result+="|"+value.charCodeAt(i);
            return result;
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("join null receiver", "Array.prototype.join.call(null)"),
    (
        "join undefined receiver",
        "Array.prototype.join.call(undefined)",
    ),
    (
        "toString null receiver",
        "Array.prototype.toString.call(null)",
    ),
    (
        "toLocaleString undefined receiver",
        "Array.prototype.toLocaleString.call(undefined)",
    ),
    (
        "Symbol length fails before separator conversion",
        r#"(function(){
            var source=Object(),separator=Object();source.length=Symbol("length");
            separator.toString=function(){throw 51};
            return Array.prototype.join.call(source,separator);
        })()"#,
    ),
    (
        "Symbol separator cannot be converted",
        "(function(){var source=Object();source.length=0;return Array.prototype.join.call(source,Symbol('separator'))})()",
    ),
    (
        "Symbol join element cannot be converted",
        "(function(){var source=Object();source[0]=Symbol('element');source.length=1;return Array.prototype.join.call(source)})()",
    ),
    (
        "missing locale method is not callable",
        "(function(){var source=Object();source[0]=Object.create(null);source.length=1;return Array.prototype.toLocaleString.call(source)})()",
    ),
    (
        "non-callable locale method is rejected",
        "(function(){var source=Object(),element=Object();element.toLocaleString=1;source[0]=element;source.length=1;return Array.prototype.toLocaleString.call(source)})()",
    ),
    (
        "Symbol locale result cannot be converted",
        "(function(){var source=Object(),element=Object();element.toLocaleString=function(){return Symbol('result')};source[0]=element;source.length=1;return Array.prototype.toLocaleString.call(source)})()",
    ),
    (
        "indexed getter throw stops before later conversion",
        r#"(function(){
            var log="",source=Object();source.length=2;
            source.__defineGetter__("0",function(){
                var element=Object();log+="G0";
                element.toString=function(){log+="C0";throw new RangeError("element boom")};
                return element;
            });
            source.__defineGetter__("1",function(){log+="G1";return 2});
            try{Array.prototype.join.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "toString join getter primitive throw is preserved",
        r#"(function(){
            var source=Object();source.__defineGetter__("join",function(){throw 57});
            return Array.prototype.toString.call(source);
        })()"#,
    ),
    ("join is not constructable", "new (Array.prototype.join)()"),
    (
        "toString is not constructable",
        "new (Array.prototype.toString)()",
    ),
    (
        "toLocaleString is not constructable",
        "new (Array.prototype.toLocaleString)()",
    ),
];

const CYCLE_CASES: &[(&str, &str)] = &[
    (
        "direct join recursion is catchable InternalError stack overflow",
        r#"(function(){
            var source=[];source[0]=source;
            try{source.join();return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
    (
        "direct toString recursion is catchable InternalError stack overflow",
        r#"(function(){
            var source=[];source[0]=source;
            try{source.toString();return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
    (
        "mutual locale recursion is catchable InternalError stack overflow",
        r#"(function(){
            var first=[],second=[];first[0]=second;second[0]=first;
            try{first.toLocaleString();return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','concat','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','join','toString','toLocaleString','copyWithin','values','keys','entries'];
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
print('meta='+metadata('join'));
print('meta='+metadata('toString'));
print('meta='+metadata('toLocaleString'));
"#;

#[test]
fn array_stringification_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array stringification oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order/generic", ORDER_AND_GENERIC_CASES),
        ("toString", TO_STRING_CASES),
        ("locale", LOCALE_CASES),
        ("errors", ERROR_CASES),
        ("cycles", CYCLE_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 4);
}

#[test]
fn array_stringification_values_holes_nested_and_utf16_match_pinned_quickjs() {
    compare_value_cases("Array stringification values", VALUE_CASES);
}

#[test]
fn array_join_order_snapshots_inheritance_and_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array.join order/generic", ORDER_AND_GENERIC_CASES);
}

#[test]
fn array_to_string_dispatch_and_intrinsic_fallback_match_pinned_quickjs() {
    compare_value_cases("Array.toString dispatch/fallback", TO_STRING_CASES);
}

#[test]
fn array_to_locale_string_dispatch_and_conversion_match_pinned_quickjs() {
    compare_value_cases("Array.toLocaleString dispatch", LOCALE_CASES);
}

#[test]
fn array_stringification_errors_match_pinned_quickjs() {
    compare_value_cases("Array stringification errors", ERROR_CASES);
}

#[test]
fn array_stringification_cycles_match_pinned_quickjs() {
    compare_value_cases("Array stringification cycles", CYCLE_CASES);
}

#[test]
fn array_stringification_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array stringification graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array stringification prototype order/metadata drifted",
    );
}

#[test]
fn array_stringification_boxing_errors_user_throws_and_overflow_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
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
    let caller_internal_error = eval_object(
        &mut caller,
        "InternalError.prototype",
        "caller InternalError prototype",
    );
    let join = property_callable(&runtime, &mut defining, &defining_array_prototype, "join");

    defining
        .eval("Number.prototype.length=1;Number.prototype[0]='defining-box'")
        .expect("install defining realm Number array-like properties");
    caller
        .eval("Number.prototype.length=1;Number.prototype[0]='caller-box'")
        .expect("install caller realm Number array-like properties");
    assert_eq!(
        caller
            .call(&join, Value::Int(7), &[])
            .expect("cross-realm primitive Array.join"),
        Value::String(JsString::try_from_utf8("defining-box").unwrap()),
        "Array.join boxed a primitive receiver outside its defining realm",
    );

    let bad_length = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=Symbol('length');return source})()",
        "caller object with Symbol length",
    );
    assert!(matches!(
        caller.call(&join, Value::Object(bad_length), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "cross-realm Array.join TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.join native TypeError did not use the method defining realm",
    );

    let throwing_length = eval_object(
        &mut caller,
        "(function(){var source=Object();source.__defineGetter__('length',function(){throw new TypeError('caller length')});return source})()",
        "caller object with throwing length getter",
    );
    assert!(matches!(
        caller.call(&join, Value::Object(throwing_length), &[]),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "cross-realm Array.join user throw");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.join replaced a user getter throw with a defining-realm error",
    );

    let cycle = eval_object(
        &mut caller,
        "(function(){var source=[];source[0]=source;return source})()",
        "caller recursive Array",
    );
    assert!(matches!(
        caller.call(&join, Value::Object(cycle), &[]),
        Err(RuntimeError::Exception),
    ));
    let overflow = take_exception_object(&mut caller, "cross-realm Array.join overflow");
    assert_eq!(
        runtime.get_prototype_of(&overflow).unwrap(),
        Some(caller_internal_error),
        "recursive Array.join did not allocate overflow in the recursively invoked method realm",
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
    for name in ["join", "toString", "toLocaleString"] {
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
            panic!("could not run QuickJS Array stringification graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array stringification graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array stringification graph output was not UTF-8")
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
