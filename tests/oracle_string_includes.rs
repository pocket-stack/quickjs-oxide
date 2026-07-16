use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, PropertyKey, Runtime, RuntimeError, Value, WellKnownSymbol,
};

// Pins QuickJS 2026-06-04 `js_string_includes` (quickjs.c 45516-45578),
// `js_is_regexp` (47487-47499), and the adjacent function-list entries
// (46634-46636). The three methods deliberately share one magic-selected
// kernel but retain distinct position defaults and search ranges.
//
// Rust-side vectors avoid Proxy and object-literal syntax. Constructor-created
// genuine RegExp values exercise the published R1a brand; the oracle-only
// vectors below preserve the remaining Proxy boundary.

const CASE_PRELUDE: &str = r#"
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    return (descriptor.writable?"1":"0")+
           (descriptor.enumerable?"1":"0")+
           (descriptor.configurable?"1":"0");
}
function __isConstructor(value){
    try{new value();return true}catch(_){return false}
}
function __units(value){
    value=String(value);
    var output="",index=0;
    while(index<value.length){
        var unit=value.charCodeAt(index).toString(16);
        while(unit.length<4)unit="0"+unit;
        if(index)output+=",";
        output+=unit;
        index++;
    }
    return output;
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "prototype table order reaches the includes family",
        r#"(function(){
            var selected=["length","at","charCodeAt","charAt","concat","codePointAt",
                "isWellFormed","toWellFormed","indexOf","lastIndexOf","includes",
                "endsWith","startsWith","toString","valueOf","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype),output=[],index=0;
            while(index<keys.length){
                if(selected.indexOf(keys[index])>=0)output.push(keys[index]);
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "method descriptors and function metadata are exact",
        r#"(function(){
            var names=["includes","endsWith","startsWith"],output=[],index=0;
            while(index<names.length){
                var name=names[index],fn=String.prototype[name];
                output.push(name+":"+__bits(String.prototype,name)+":"+fn.name+":"+fn.length+
                    ":"+Object.getOwnPropertyNames(fn).join(",")+":"+
                    __bits(fn,"length")+":"+__bits(fn,"name")+":"+
                    (Object.getPrototypeOf(fn)===Function.prototype)+":"+
                    (typeof fn)+":"+__isConstructor(fn));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "AutoInit materializes one stable callable per property",
        r#"(function(){
            var first=String.prototype.includes,again=String.prototype.includes;
            var descriptor=Object.getOwnPropertyDescriptor(String.prototype,"includes");
            return [(first===again),(first===descriptor.value),
                (first===String.prototype.endsWith),(first===String.prototype.startsWith)].join("|");
        })()"#,
    ),
];

const AUTOINIT_CASES: &[(&str, &str)] = &[
    (
        "lazy includes can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.includes;
            return [deleted,"includes" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"includes"),
                typeof String.prototype.includes].join("|");
        })()"#,
    ),
    (
        "lazy endsWith assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.endsWith=17;
            return [String.prototype.endsWith,__bits(String.prototype,"endsWith"),
                Object.getOwnPropertyNames(String.prototype).indexOf("endsWith")].join("|");
        })()"#,
    ),
    (
        "materialized startsWith remains deletable",
        r#"(function(){
            var fn=String.prototype.startsWith,deleted=delete String.prototype.startsWith;
            return [typeof fn,deleted,"startsWith" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"startsWith")].join("|");
        })()"#,
    ),
];

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "includes values and clamped positions",
        r#"(function(){return [
            "abc".includes(),"abc".includes(""),"abc".includes("",3),
            "abc".includes("",4),"abc".includes("bc"),"abc".includes("bc",1),
            "abc".includes("bc",2),"abc".includes("a",-1),"abc".includes("a",NaN),
            "abc".includes("a",-Infinity),"abc".includes("a",Infinity),
            "abc".includes("c",2.9),"abc".includes("a",4294967297)
        ].join("|")})()"#,
    ),
    (
        "startsWith values and clamped positions",
        r#"(function(){return [
            "abc".startsWith(),"abc".startsWith(""),"abc".startsWith("",3),
            "abc".startsWith("",4),"abc".startsWith("ab"),"abc".startsWith("bc",1),
            "abc".startsWith("bc",2),"abc".startsWith("a",-1),
            "abc".startsWith("a",NaN),"abc".startsWith("a",-Infinity),
            "abc".startsWith("c",Infinity),"abc".startsWith("c",2.9),
            "abc".startsWith("a",4294967297)
        ].join("|")})()"#,
    ),
    (
        "endsWith values and end positions",
        r#"(function(){return [
            "abc".endsWith(),"abc".endsWith(""),"abc".endsWith("",0),
            "abc".endsWith("",4),"abc".endsWith("bc"),"abc".endsWith("ab",2),
            "abc".endsWith("bc",2),"abc".endsWith("a",1),"abc".endsWith("a",-1),
            "abc".endsWith("",NaN),"abc".endsWith("abc",Infinity),
            "abc".endsWith("a",1.9),"abc".endsWith("abc",4294967297)
        ].join("|")})()"#,
    ),
    (
        "UTF-16 searches operate on raw code-unit boundaries",
        r#"(function(){
            var source="A\ud83d\ude00\ud800Z";
            return [source.includes("\ud83d"),source.includes("\ude00"),
                source.includes("\ude00\ud800"),source.includes("\ud800Z"),
                source.startsWith("\ud83d",1),source.startsWith("\ude00",2),
                source.startsWith("\ud800",3),source.endsWith("\ude00",3),
                source.endsWith("\ud800",4),source.endsWith("\ud800Z")].join("|");
        })()"#,
    ),
    (
        "large rope searches cross leaf and rebalance boundaries",
        r#"(function(){
            var source="",index=0;
            while(index<9000){source+="a";index++}
            source+="b\ud800Z";
            return [source.length,source.includes("ab",8998),source.includes("b\ud800",9000),
                source.startsWith("ab",8999),source.startsWith("b\ud800",9000),
                source.endsWith("b\ud800Z"),source.endsWith("ab",9001),
                source.includes("\ud800Z",9001)].join("|");
        })()"#,
    ),
    (
        "borrowed methods remain generic and ignore this method owner",
        r#"(function(){return [
            String.prototype.includes.call(123,"2"),
            String.prototype.startsWith.call(true,"tr"),
            String.prototype.endsWith.call(1n,"1")
        ].join("|")})()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "receiver then match then search then position is the exact order",
        r#"(function(){
            var log="",receiver=Object(),search=Object(),position=Object(),descriptor=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "abc"};
            descriptor.configurable=true;
            descriptor.get=function(){log+="get-match;";return false};
            Object.defineProperty(search,Symbol.match,descriptor);
            search[Symbol.toPrimitive]=function(hint){log+="search:"+hint+";";return "b"};
            position[Symbol.toPrimitive]=function(hint){log+="position:"+hint+";";return 1};
            var result=String.prototype.includes.call(receiver,search,position);
            return result+"|"+log;
        })()"#,
    ),
    (
        "all three selectors share the same observable conversion prefix",
        r#"(function(){
            var log="";
            function run(name){
                var receiver=Object(),search=Object(),position=Object(),descriptor=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+=name+":r:"+hint+";";return "abc"};
                descriptor.get=function(){log+=name+":m;";return null};
                descriptor.configurable=true;Object.defineProperty(search,Symbol.match,descriptor);
                search[Symbol.toPrimitive]=function(hint){log+=name+":s:"+hint+";";return "b"};
                position[Symbol.toPrimitive]=function(hint){log+=name+":p:"+hint+";";return 1};
                return String.prototype[name].call(receiver,search,position);
            }
            return [run("includes"),run("startsWith"),run("endsWith"),log].join("|");
        })()"#,
    ),
    (
        "non-object search skips Symbol.match lookup entirely",
        r#"(function(){
            var hits=0,descriptor=Object();
            descriptor.configurable=true;descriptor.get=function(){hits++;throw "wrong"};
            Object.defineProperty(String.prototype,Symbol.match,descriptor);
            var result="abc".includes("b");
            delete String.prototype[Symbol.match];
            return result+"|"+hits;
        })()"#,
    ),
    (
        "undefined match falls through while false and null override IsRegExp",
        r#"(function(){
            function run(match){
                var log="",search=Object();search[Symbol.match]=match;
                search[Symbol.toPrimitive]=function(hint){log+=hint+";";return "b"};
                return "abc".includes(search)+":"+log;
            }
            return [run(undefined),run(false),run(null)].join("|");
        })()"#,
    ),
    (
        "truthy match short-circuits search and position conversion",
        r#"(function(){
            var log="",search=Object(),position=Object();
            search[Symbol.match]=true;
            search[Symbol.toPrimitive]=function(){log+="search;";throw "wrong"};
            position[Symbol.toPrimitive]=function(){log+="position;";throw "wrong"};
            try{"abc".includes(search,position)}catch(error){return error.name+":"+error.message+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "truthy object match uses ToBoolean without coercing that object",
        r#"(function(){
            var log="",search=Object(),match=Object();
            match[Symbol.toPrimitive]=function(){log+="match-convert;";return false};
            search[Symbol.match]=match;
            search[Symbol.toPrimitive]=function(){log+="search-convert;";return "b"};
            try{"abc".startsWith(search)}catch(error){return error.name+":"+error.message+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "explicit undefined position is not converted",
        r#"(function(){return [
            "abc".includes("a",undefined),"abc".startsWith("a",undefined),
            "abc".endsWith("c",undefined)
        ].join("|")})()"#,
    ),
    (
        "receiver failure precedes every search hook",
        r#"(function(){
            var log="",receiver=Object(),search=Object(),descriptor=Object();
            receiver[Symbol.toPrimitive]=function(){log+="receiver;";throw "receiver-throw"};
            descriptor.get=function(){log+="match;";return false};descriptor.configurable=true;
            Object.defineProperty(search,Symbol.match,descriptor);
            try{String.prototype.endsWith.call(receiver,search)}catch(error){return error+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "search conversion failure prevents position conversion",
        r#"(function(){
            var log="",search=Object(),position=Object();search[Symbol.match]=false;
            search[Symbol.toPrimitive]=function(){log+="search;";throw "search-throw"};
            position[Symbol.toPrimitive]=function(){log+="position;";return 0};
            try{"abc".includes(search,position)}catch(error){return error+"|"+log}
            return "missing";
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "includes rejects null receiver",
        "String.prototype.includes.call(null,'x')",
    ),
    (
        "startsWith rejects undefined receiver",
        "String.prototype.startsWith.call(undefined,'x')",
    ),
    (
        "endsWith rejects Symbol search",
        "'abc'.endsWith(Symbol('x'))",
    ),
    (
        "truthy Symbol.match rejects search",
        r#"(function(){var search=Object();search[Symbol.match]=1;return "abc".includes(search)})()"#,
    ),
    (
        "Symbol.match getter throw is preserved",
        r#"(function(){
            var search=Object(),descriptor=Object();descriptor.get=function(){throw 91};
            descriptor.configurable=true;Object.defineProperty(search,Symbol.match,descriptor);
            return "abc".startsWith(search);
        })()"#,
    ),
    (
        "search ToPrimitive object result is rejected",
        r#"(function(){
            var search=Object();search[Symbol.match]=false;
            search[Symbol.toPrimitive]=function(){return Object()};
            return "abc".endsWith(search);
        })()"#,
    ),
    ("includes rejects BigInt position", "'abc'.includes('a',1n)"),
    (
        "startsWith rejects Symbol position",
        "'abc'.startsWith('a',Symbol('p'))",
    ),
    (
        "position conversion throw is preserved",
        r#"(function(){
            var position=Object();position[Symbol.toPrimitive]=function(){throw "position"};
            return "abc".endsWith("c",position);
        })()"#,
    ),
    (
        "includes is not a constructor",
        "new String.prototype.includes()",
    ),
    (
        "endsWith is not a constructor",
        "new String.prototype.endsWith()",
    ),
    (
        "startsWith is not a constructor",
        "new String.prototype.startsWith()",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "recursive Symbol.match getter throws catchably and runtime recovers",
        r#"(function(){
            var search=Object(),descriptor=Object(),errorName="",errorMessage="";
            descriptor.configurable=true;
            descriptor.get=function(){return "x".includes(search)};
            Object.defineProperty(search,Symbol.match,descriptor);
            try{"x".includes(search)}catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"abc".includes("b"),
                "abc".startsWith("a"),"abc".endsWith("c")].join("|");
        })()"#,
    ),
    (
        "interleaved includes family recursion cannot bypass recovery guard",
        r#"(function(){
            var search=Object(),descriptor=Object(),depth=0,errorName="";
            descriptor.configurable=true;
            descriptor.get=function(){
                depth++;
                if(depth%3===0)return "x".includes(search);
                if(depth%3===1)return "x".startsWith(search);
                return "x".endsWith(search);
            };
            Object.defineProperty(search,Symbol.match,descriptor);
            try{"x".includes(search)}catch(error){errorName=error.name}
            return [errorName,"recovered".includes("cover"),"recovered".endsWith("red")].join("|");
        })()"#,
    ),
];

const REGEXP_CASES: &[(&str, &str)] = &[
    (
        "real RegExp is rejected before position conversion",
        r#"(function(){
            var log="",position=Object();position[Symbol.toPrimitive]=function(){log+="position;";return 0};
            try{"abc".includes(RegExp("b"),position)}catch(error){return error.name+":"+error.message+"|"+log}
            return "missing";
        })()"#,
    ),
    (
        "RegExp Symbol.match false and null override the internal brand",
        r#"(function(){
            var first=RegExp("b");first[Symbol.match]=false;
            var second=RegExp("b");second[Symbol.match]=null;
            return ["abc/b/".includes(first),"abc/b/".startsWith(second,3)].join("|");
        })()"#,
    ),
    (
        "RegExp undefined match falls back to its internal brand",
        r#"(function(){
            var value=RegExp("b");value[Symbol.match]=undefined;
            try{"abc".endsWith(value)}catch(error){return error.name+":"+error.message}
            return "missing";
        })()"#,
    ),
];

const EXOTIC_ORACLE_ONLY_CASES: &[(&str, &str)] = &[
    (
        "Proxy exposes match Get then search ToPrimitive order",
        r#"(function(){
            var log="",target=Object();target[Symbol.match]=false;
            target[Symbol.toPrimitive]=function(hint){log+="convert:"+hint+";";return "b"};
            var search=new Proxy(target,{get:function(object,key,receiver){
                log+="get:"+String(key)+";";return Reflect.get(object,key,receiver)}});
            return "abc".includes(search)+"|"+log;
        })()"#,
    ),
    (
        "Proxy receiver conversion completes before Proxy search match trap",
        r#"(function(){
            var log="",receiverTarget=Object(),searchTarget=Object();
            receiverTarget[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "abc"};
            searchTarget[Symbol.match]=false;
            searchTarget[Symbol.toPrimitive]=function(hint){log+="search:"+hint+";";return "b"};
            var receiver=new Proxy(receiverTarget,{get:function(object,key,recv){
                log+="receiver-get:"+String(key)+";";return Reflect.get(object,key,recv)}});
            var search=new Proxy(searchTarget,{get:function(object,key,recv){
                log+="search-get:"+String(key)+";";return Reflect.get(object,key,recv)}});
            return String.prototype.startsWith.call(receiver,search,1)+"|"+log;
        })()"#,
    ),
];

#[test]
fn string_includes_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String includes-family oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("AutoInit", AUTOINIT_CASES),
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("errors", ERROR_CASES),
        ("stack", STACK_CASES),
        ("RegExp", REGEXP_CASES),
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
}

#[test]
fn string_includes_graph_and_autoinit_match_pinned_quickjs() {
    compare_cases("String includes-family graph", GRAPH_CASES);
    compare_cases("String includes-family AutoInit", AUTOINIT_CASES);
}

#[test]
fn string_includes_values_positions_utf16_and_ropes_match_pinned_quickjs() {
    compare_cases("String includes-family values", VALUE_CASES);
}

#[test]
fn string_includes_is_regexp_and_conversion_order_match_pinned_quickjs() {
    compare_cases("String includes-family order", ORDER_CASES);
    compare_cases("String includes-family RegExp", REGEXP_CASES);
}

#[test]
fn string_includes_errors_match_pinned_quickjs() {
    compare_cases("String includes-family errors", ERROR_CASES);
}

#[test]
fn string_includes_recursion_is_catchable_and_runtime_recovers() {
    compare_cases("String includes-family stack recovery", STACK_CASES);
}

#[test]
fn string_includes_defining_realms_and_user_throw_identity_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let includes = property_callable(&runtime, &mut defining, &defining_prototype, "includes");
    let starts_with = property_callable(&runtime, &mut defining, &defining_prototype, "startsWith");
    let ends_with = property_callable(&runtime, &mut defining, &defining_prototype, "endsWith");
    assert_eq!(
        runtime.get_prototype_of(includes.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_native_type_error(
        &runtime,
        &mut caller,
        &includes,
        Value::Null,
        &[Value::String(JsString::try_from_utf8("x").unwrap())],
        &defining_type_error,
    );
    let search_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("search").unwrap()))
        .unwrap();
    assert_native_type_error(
        &runtime,
        &mut caller,
        &starts_with,
        Value::String(JsString::try_from_utf8("abc").unwrap()),
        &[Value::Symbol(search_symbol)],
        &defining_type_error,
    );
    let regexp_like = caller.new_object().unwrap();
    define_data_key(
        &runtime,
        &regexp_like,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Match)),
        Value::Bool(true),
    );
    assert_native_type_error(
        &runtime,
        &mut caller,
        &ends_with,
        Value::String(JsString::try_from_utf8("abc").unwrap()),
        &[Value::Object(regexp_like)],
        &defining_type_error,
    );
    let position_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("position").unwrap()))
        .unwrap();
    assert_native_type_error(
        &runtime,
        &mut caller,
        &includes,
        Value::String(JsString::try_from_utf8("abc").unwrap()),
        &[
            Value::String(JsString::try_from_utf8("a").unwrap()),
            Value::Symbol(position_symbol),
        ],
        &defining_type_error,
    );

    let sentinel = caller.new_object().unwrap();
    let sentinel_key = runtime.intern_property_key("includesSentinel").unwrap();
    assert!(
        caller
            .set_property(
                &caller.global_object().unwrap(),
                &sentinel_key,
                Value::Object(sentinel.clone()),
            )
            .unwrap()
    );
    let throwing_getter = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw includesSentinel})",
    );
    let search = caller.new_object().unwrap();
    define_accessor_key(
        &runtime,
        &search,
        &PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Match)),
        Some(throwing_getter),
    );
    assert_eq!(
        caller.call(
            &includes,
            Value::String(JsString::try_from_utf8("abc").unwrap()),
            &[Value::Object(search)],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "Symbol.match getter throw identity was not preserved",
    );

    assert_eq!(
        caller.construct(&includes, &[]),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_includes_callables_are_per_realm_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let first_method = property_callable(&runtime, &mut first, &first_prototype, "includes");
        let first_again = property_callable(&runtime, &mut first, &first_prototype, "includes");
        let second_method = property_callable(&runtime, &mut second, &second_prototype, "includes");
        assert_eq!(first_method, first_again);
        assert_ne!(first_method, second_method);
        assert_eq!(
            runtime.get_prototype_of(first_method.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        first_method
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_includes_records_current_proxy_and_module_boundaries() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.eval("typeof RegExp+'|'+typeof Proxy").unwrap(),
        Value::String(JsString::try_from_utf8("function|undefined").unwrap()),
        "move the remaining oracle-only vectors into the differential when Proxy lands",
    );
    // Module namespace exotic Get and Proxy invariant paths remain explicit
    // object-model boundaries. Neither is needed for the ordinary IsRegExp
    // algorithm covered above.
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let source = format!("{CASE_PRELUDE}\n{source}");
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, &source, description),
            observe_oracle_source(&oracle, &source, description),
            "{group} drifted for {description}",
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
        Err(error) => panic!("Rust engine failure for {description}: {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
    let source = format!("{CASE_PRELUDE}\n{source}");
    observe_oracle_source(oracle, &source, description)
}

fn observe_oracle_source(oracle: &OsStr, source: &str, description: &str) -> String {
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
    let global = context.global_object().unwrap();
    let constructor = property_callable(runtime, context, &global, constructor_name);
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

fn eval_callable(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{source:?} was not callable"))
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
}

fn assert_native_type_error(
    runtime: &Runtime,
    context: &mut Context,
    method: &CallableRef,
    this_value: Value,
    arguments: &[Value],
    expected_prototype: &ObjectRef,
) {
    assert_eq!(
        context.call(method, this_value, arguments),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(context))
            .unwrap()
            .as_ref(),
        Some(expected_prototype),
    );
}

fn define_data_key(runtime: &Runtime, object: &ObjectRef, key: &PropertyKey, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_accessor_key(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
    getter: Option<CallableRef>,
) {
    assert!(
        runtime
            .define_own_property(
                object,
                key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(match getter {
                        Some(getter) => AccessorValue::Callable(getter),
                        None => AccessorValue::Undefined,
                    }),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
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
