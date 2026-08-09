use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_string_toLowerCase`, its Unicode 17 case
// conversion/property tables, and the four GenericMagic String prototype
// entries (quickjs.c 46215-46304 and 46656-46659; libunicode.c 51-190 and
// 347-376). The two locale-named variants deliberately share the ordinary
// lower/upper kernels and never inspect their locale arguments.
//
// Every raw String observation is encoded as hexadecimal UTF-16 code units
// before crossing the process boundary, preserving NUL and lone surrogates.

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
function __capture(callback){
    try{return "return:"+callback()}
    catch(error){
        if(error!==null&&typeof error==="object")return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "the four case keys follow valueOf and precede Annex B and constructor",
        r#"(function(){
            var selected=["trimLeft","toString","valueOf","toLowerCase",
                "toUpperCase","toLocaleLowerCase","toLocaleUpperCase","anchor",
                "big","sup","constructor"];
            var keys=Object.getOwnPropertyNames(String.prototype)
                .concat(Object.getOwnPropertySymbols(String.prototype));
            var output=[],index=0;
            while(index<keys.length){
                var key=keys[index];
                if(key===Symbol.iterator)output.push("@@iterator");
                else if(selected.indexOf(key)>=0)output.push(key);
                index++;
            }
            return output.join(",");
        })()"#,
    ),
    (
        "descriptors names lengths function metadata and construct bits are exact",
        r#"(function(){
            var names=["toLowerCase","toUpperCase","toLocaleLowerCase",
                "toLocaleUpperCase"],output=[],index=0;
            while(index<names.length){
                var name=names[index],fn=String.prototype[name];
                output.push(name+":"+__bits(String.prototype,name)+":"+fn.name+":"+
                    fn.length+":"+Object.getOwnPropertyNames(fn).join(",")+":"+
                    __bits(fn,"length")+":"+__bits(fn,"name")+":"+
                    (Object.getPrototypeOf(fn)===Function.prototype)+":"+
                    (typeof fn)+":"+__isConstructor(fn));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "AutoInit identities are stable descriptor backed and mutually distinct",
        r#"(function(){
            var names=["toLowerCase","toUpperCase","toLocaleLowerCase",
                "toLocaleUpperCase"],functions=[],output=[],index=0,other;
            while(index<names.length){
                var name=names[index],first=String.prototype[name];
                functions.push(first);
                output.push(first===String.prototype[name]);
                output.push(first===Object.getOwnPropertyDescriptor(String.prototype,name).value);
                index++;
            }
            var distinct=true;
            index=0;
            while(index<functions.length){
                other=index+1;
                while(other<functions.length){
                    if(functions[index]===functions[other])distinct=false;
                    other++;
                }
                index++;
            }
            output.push(distinct,functions[0]===String.prototype.trim,
                functions[1]===String.prototype.valueOf,
                functions[2]===functions[0],functions[3]===functions[1]);
            return output.join("|");
        })()"#,
    ),
];

const PROPERTY_CASES: &[(&str, &str)] = &[
    (
        "lazy toLowerCase can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.toLowerCase;
            return [deleted,"toLowerCase" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"toLowerCase"),
                typeof String.prototype.toLowerCase,
                typeof String.prototype.toUpperCase].join("|");
        })()"#,
    ),
    (
        "lazy locale lower assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.toLocaleLowerCase=17;
            return [String.prototype.toLocaleLowerCase,
                __bits(String.prototype,"toLocaleLowerCase"),
                Object.prototype.hasOwnProperty.call(String.prototype,"toLocaleLowerCase"),
                typeof String.prototype.toLocaleUpperCase].join("|");
        })()"#,
    ),
    (
        "materialized methods remain independently deletable and replaceable",
        r#"(function(){
            var lower=String.prototype.toLowerCase;
            var upper=String.prototype.toUpperCase;
            var localeLower=String.prototype.toLocaleLowerCase;
            var localeUpper=String.prototype.toLocaleUpperCase;
            delete String.prototype.toUpperCase;
            String.prototype.toLocaleLowerCase=23;
            return [typeof lower,"toUpperCase" in String.prototype,
                String.prototype.toLocaleLowerCase,
                String.prototype.toLocaleUpperCase===localeUpper,
                String.prototype.toLowerCase===lower,
                localeLower===lower,localeUpper===upper].join("|");
        })()"#,
    ),
];

const OUTPUT_CASES: &[(&str, &str)] = &[
    (
        "ASCII primitives wrappers and ordinary objects convert generically",
        r#"(function(){return [
            "ABC xyz 123".toLowerCase(),
            "abc XYZ 123".toUpperCase(),
            String.prototype.toLowerCase.call(123),
            String.prototype.toUpperCase.call(true),
            String.prototype.toLowerCase.call(7n),
            String.prototype.toUpperCase.call(Object()),
            String.prototype.toLowerCase.call(new String("Wrap"))
        ].join("|")})()"#,
    ),
    (
        "Unicode simple and multi-code-point mappings use the pinned tables",
        r#"(function(){return [
            __units("\u0130\u212a\u0178\u1e9e\u03a3".toLowerCase()),
            __units("\u00df\ufb03\u0149\u0587\u0390\u03b0".toUpperCase()),
            __units("\u01f0\u1e96\u1e97\u1e98\u1e99".toUpperCase()),
            __units("\u2126\u212a\u212b".toLowerCase())
        ].join("|")})()"#,
    ),
    (
        "Unicode 17 Beria Erfe astral pairs are not host-Unicode dependent",
        r#"(function(){return [
            __units(String.fromCodePoint(0x16e40,0x16ea0).toLowerCase()),
            __units(String.fromCodePoint(0x16e60,0x16ebb).toUpperCase()),
            __units(String.fromCodePoint(0x16e5f,0x16eb8).toLowerCase()),
            __units(String.fromCodePoint(0x16e7f,0x16ed3).toUpperCase())
        ].join("|")})()"#,
    ),
    (
        "astral mappings and every unmatched surrogate code unit are preserved",
        r#"(function(){
            var source="\ud800A\udc00\ud801\udc00\ud801\udc28\udbff\udfff";
            return [source.length,__units(source.toLowerCase()),
                __units(source.toUpperCase()),
                __units("\ud801\udc00".toLowerCase()),
                __units("\ud801\udc28".toUpperCase())].join("|");
        })()"#,
    ),
    (
        "empty and already-cased strings retain exact output",
        r#"(function(){return [__units("".toLowerCase()),__units("".toUpperCase()),
            __units("already lower \u00df".toLowerCase()),
            __units("ALREADY UPPER \u212a".toUpperCase()),
            __units("\u0000\uffff".toLowerCase()),
            __units("\u0000\uffff".toUpperCase())].join("|")})()"#,
    ),
    (
        "rope conversion crosses the 8192-unit boundary and sigma context leaves",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var source=(grow("A",13)+"\u03a3")+("\u0301B"+grow("Z",13));
            var lower=source.toLowerCase(),upper=source.toUpperCase();
            return [source.length,lower.length,upper.length,
                __units(lower.slice(8188,8200)),__units(upper.slice(8188,8200)),
                __units(lower.slice(0,4)),__units(lower.slice(lower.length-4))].join("|");
        })()"#,
    ),
];

const FINAL_SIGMA_CASES: &[(&str, &str)] = &[
    (
        "Greek sigma requires a preceding cased code point and no following cased code point",
        r#"(function(){
            var values=["\u03a3","\u039f\u03a3","\u039f\u03a3\u0391",
                "A\u03a3","A\u03a3B","A \u03a3","A\u03a3 B"],output=[],index=0;
            while(index<values.length){output.push(__units(values[index].toLowerCase()));index++}
            return output.join("|");
        })()"#,
    ),
    (
        "Case_Ignorable code points are skipped on both sides of sigma",
        r#"(function(){
            var values=["A\u0301\u03a3","A\u03a3\u0301B","A'\u0301\u03a3",
                "A\u03a3'\u0301B","A\u00ad\u03a3","A\u03a3\u00adB"],output=[],index=0;
            while(index<values.length){output.push(__units(values[index].toLowerCase()));index++}
            return output.join("|");
        })()"#,
    ),
    (
        "surrogates stop context scans while astral cased characters participate",
        r#"(function(){
            var values=["A\ud800\u03a3","A\u03a3\ud800B",
                "\ud801\udc00\u03a3","A\u03a3\ud801\udc28",
                String.fromCodePoint(0x16ea0)+"\u03a3",
                "A\u03a3"+String.fromCodePoint(0x16ebb)],output=[],index=0;
            while(index<values.length){output.push(__units(values[index].toLowerCase()));index++}
            return output.join("|");
        })()"#,
    ),
];

const LOCALE_CASES: &[(&str, &str)] = &[
    (
        "locale-named methods are exact aliases in behavior but not identity",
        r#"(function(){
            var value="I\u0130i\u00df\u03a3\ud801\udc00";
            return [
                __units(value.toLowerCase()),__units(value.toLocaleLowerCase()),
                __units(value.toLocaleLowerCase("tr")),
                __units(value.toUpperCase()),__units(value.toLocaleUpperCase()),
                __units(value.toLocaleUpperCase("tr")),
                String.prototype.toLowerCase===String.prototype.toLocaleLowerCase,
                String.prototype.toUpperCase===String.prototype.toLocaleUpperCase
            ].join("|");
        })()"#,
    ),
    (
        "all locale arguments including throwing coercion hooks are completely ignored",
        r#"(function(){
            var hits=0,locale=Object(),extra=Object(),descriptor=Object();
            locale[Symbol.toPrimitive]=function(){hits++;throw "locale primitive"};
            descriptor.get=function(){hits++;throw "locale string"};
            Object.defineProperty(locale,"toString",descriptor);
            extra[Symbol.toPrimitive]=function(){hits++;throw "extra primitive"};
            return [__units("I\u0130".toLocaleLowerCase(locale,extra,Symbol("later"))),
                __units("i\u00df".toLocaleUpperCase(locale,extra,Symbol("later"))),hits].join("|");
        })()"#,
    ),
    (
        "receiver conversion uses a string hint before ignored locale values",
        r#"(function(){
            var log="",hits=0,receiver=Object(),locale=Object();
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "I\u0130"};
            locale[Symbol.toPrimitive]=function(){hits++;throw "locale"};
            var lower=String.prototype.toLocaleLowerCase.call(receiver,locale);
            var upper=String.prototype.toLocaleUpperCase.call(receiver,locale);
            return [__units(lower),__units(upper),log,hits].join("|");
        })()"#,
    ),
    (
        "receiver abrupt completion preserves identity and never touches locale",
        r#"(function(){
            var sentinel=Object(),receiver=Object(),locale=Object(),hits=0;
            receiver[Symbol.toPrimitive]=function(){throw sentinel};
            locale[Symbol.toPrimitive]=function(){hits++;throw "locale"};
            try{String.prototype.toLocaleLowerCase.call(receiver,locale)}
            catch(error){return [(error===sentinel),hits].join("|")}
            return "missing";
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "toLowerCase rejects a null receiver",
        "String.prototype.toLowerCase.call(null)",
    ),
    (
        "toUpperCase rejects an undefined receiver",
        "String.prototype.toUpperCase.call(undefined)",
    ),
    (
        "toLocaleLowerCase rejects a Symbol receiver before ignoring locale",
        "String.prototype.toLocaleLowerCase.call(Symbol('receiver'), Symbol('locale'))",
    ),
    (
        "toLocaleUpperCase rejects a Symbol receiver",
        "String.prototype.toLocaleUpperCase.call(Symbol('receiver'))",
    ),
    (
        "ordinary lower receiver ToPrimitive returning an object is rejected",
        r#"(function(){
            var receiver=Object();receiver[Symbol.toPrimitive]=function(){return Object()};
            return String.prototype.toLowerCase.call(receiver);
        })()"#,
    ),
    (
        "locale upper receiver ToPrimitive returning an object is rejected",
        r#"(function(){
            var receiver=Object();receiver[Symbol.toPrimitive]=function(){return Object()};
            return String.prototype.toLocaleUpperCase.call(receiver,"en");
        })()"#,
    ),
];

const CONSTRUCT_CASES: &[(&str, &str)] = &[(
    "all four case methods reject construction with their own exact name",
    r#"(function(){
        var names=["toLowerCase","toUpperCase","toLocaleLowerCase","toLocaleUpperCase"];
        var output=[],index=0;
        while(index<names.length){
            var fn=String.prototype[names[index]];
            output.push(names[index]+":"+__capture(function(){return new fn()}));
            index++;
        }
        return output.join("|");
    })()"#,
)];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "recursive receiver conversion throws catchably and recovers",
        r#"(function(){
            var lower=Object(),upper=Object(),lowerError="",upperError="";
            lower[Symbol.toPrimitive]=function(){return String.prototype.toLowerCase.call(lower)};
            upper[Symbol.toPrimitive]=function(){return String.prototype.toUpperCase.call(upper)};
            try{String.prototype.toLowerCase.call(lower)}catch(error){lowerError=error.name+":"+error.message}
            try{String.prototype.toUpperCase.call(upper)}catch(error){upperError=error.name+":"+error.message}
            return [lowerError,upperError,"AbC".toLowerCase(),"AbC".toUpperCase()].join("|");
        })()"#,
    ),
    (
        "case conversion and existing String methods share one recursion guard",
        r#"(function(){
            var value=Object(),depth=0,errorName="",errorMessage="";
            value[Symbol.toPrimitive]=function(){
                depth++;
                if(depth%8===0)return String.prototype.toLowerCase.call(value);
                if(depth%8===1)return String.prototype.toLocaleUpperCase.call(value,Symbol("ignored"));
                if(depth%8===2)return String.prototype.trim.call(value);
                if(depth%8===3)return "x".padEnd(value,"_");
                if(depth%8===4)return "x".repeat(value);
                if(depth%8===5)return "abcdef".slice(value,4);
                if(depth%8===6)return "abcdef".includes("a",value);
                return String.prototype.bold.call(value);
            };
            try{String.prototype.toUpperCase.call(value)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"AbC".toLowerCase(),"AbC".toLocaleUpperCase("tr"),
                " x ".trim(),"x".padEnd(3,"_"),"ok".repeat(2),
                "abcdef".slice(1,3),"abcdef".includes("bc"),"x".bold()].join("|");
        })()"#,
    ),
];

#[test]
fn string_case_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String case oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("properties", PROPERTY_CASES),
        ("outputs", OUTPUT_CASES),
        ("final sigma", FINAL_SIGMA_CASES),
        ("locale", LOCALE_CASES),
        ("construction", CONSTRUCT_CASES),
        ("stack", STACK_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|"),
                "{group} oracle vector unexpectedly threw for {description}: {observation:?}",
            );
        }
    }
    for &(description, source) in ERROR_CASES {
        let observation = observe_oracle(&oracle, source, description);
        assert!(
            observation.starts_with("throw|"),
            "error oracle vector unexpectedly returned for {description}: {observation:?}",
        );
    }
}

#[test]
fn string_case_graph_metadata_and_autoinit_match_pinned_quickjs() {
    compare_cases("String case graph", GRAPH_CASES);
}

#[test]
fn string_case_property_delete_and_override_match_pinned_quickjs() {
    compare_cases("String case properties", PROPERTY_CASES);
}

#[test]
fn string_case_ascii_unicode_expansions_astral_surrogates_and_ropes_match_pinned_quickjs() {
    compare_cases("String case outputs", OUTPUT_CASES);
}

#[test]
fn string_case_greek_final_sigma_context_matches_pinned_quickjs() {
    compare_cases("String case final sigma", FINAL_SIGMA_CASES);
}

#[test]
fn string_case_locale_arguments_are_ignored_and_receiver_order_is_exact() {
    compare_cases("String case locale", LOCALE_CASES);
}

#[test]
fn string_case_errors_and_nonconstructors_match_pinned_quickjs() {
    compare_cases("String case errors", ERROR_CASES);
    compare_cases("String case construction", CONSTRUCT_CASES);
}

#[test]
fn string_case_recursion_is_catchable_shared_and_recovers() {
    compare_cases("String case stack recovery", STACK_CASES);
}

#[test]
fn string_case_defining_realms_user_throw_identity_and_caller_construct_error_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let lower = property_callable(&runtime, &mut defining, &defining_prototype, "toLowerCase");
    let locale_upper = property_callable(
        &runtime,
        &mut defining,
        &defining_prototype,
        "toLocaleUpperCase",
    );
    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_eq!(
        runtime.get_prototype_of(lower.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    assert_native_error(
        &runtime,
        &mut caller,
        &lower,
        Value::Null,
        &[],
        &defining_type_error,
    );
    let receiver_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("receiver").unwrap()))
        .unwrap();
    assert_native_error(
        &runtime,
        &mut caller,
        &locale_upper,
        Value::Symbol(receiver_symbol),
        &[],
        &defining_type_error,
    );

    let sentinel = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "caseSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_receiver = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw caseSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(&lower, throwing_receiver, &[]),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "receiver conversion did not preserve the user-thrown value",
    );

    assert_eq!(caller.construct(&lower, &[]), Err(RuntimeError::Exception));
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_case_callables_are_per_realm_distinct_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let names = [
            "toLowerCase",
            "toUpperCase",
            "toLocaleLowerCase",
            "toLocaleUpperCase",
        ];
        let first_functions =
            names.map(|name| property_callable(&runtime, &mut first, &first_prototype, name));
        let second_functions =
            names.map(|name| property_callable(&runtime, &mut second, &second_prototype, name));
        for index in 0..names.len() {
            assert_eq!(
                first_functions[index],
                property_callable(&runtime, &mut first, &first_prototype, names[index]),
            );
            assert_ne!(first_functions[index], second_functions[index]);
            for other in (index + 1)..names.len() {
                assert_ne!(first_functions[index], first_functions[other]);
            }
        }
        assert_eq!(
            runtime
                .get_prototype_of(first_functions[0].as_object())
                .unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        first_functions
    };
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(retained);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn string_case_stack_overflow_uses_the_caller_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let lower = property_callable(&runtime, &mut defining, &defining_prototype, "toLowerCase");
    let defining_internal_error = intrinsic_prototype(&runtime, &mut defining, "InternalError");
    let caller_internal_error = intrinsic_prototype(&runtime, &mut caller, "InternalError");
    assert_ne!(defining_internal_error, caller_internal_error);

    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "foreignLower",
        Value::Object(lower.as_object().clone()),
    );
    let Value::Object(error) = caller
        .eval(
            r#"(function(){
                var receiver=Object(),localCall=Function.prototype.call;
                function invoke(){return localCall.call(foreignLower,receiver)}
                receiver[Symbol.toPrimitive]=function(){return invoke()};
                try{invoke()}catch(error){return error}
                return Object();
            })()"#,
        )
        .unwrap()
    else {
        panic!("recursive cross-realm String case conversion did not return an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_internal_error),
        "pre-dispatch stack overflow did not use the caller realm",
    );
    assert_eq!(
        caller
            .call(
                &lower,
                Value::String(JsString::try_from_utf8("AbC").unwrap()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("abc").unwrap()),
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

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
}

fn assert_native_error(
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

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    assert!(
        runtime
            .define_own_property(
                object,
                &runtime.intern_property_key(name).unwrap(),
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
