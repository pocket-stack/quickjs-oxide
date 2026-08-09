use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 Annex-B `js_string_CreateHTML` and its selector/tag
// table (quickjs.c 46546-46615), plus the thirteen GenericMagic String
// prototype entries (46661-46674). QuickJS converts the receiver first. The
// four attribute variants then use `JS_ToStringCheckObject` for argv[0], so a
// missing, explicit undefined, or null attribute is rejected. The other nine
// variants ignore every argument. Attribute text replaces only raw U+0022
// with the six ASCII code units of `&quot;`; source text is never escaped.
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
        "thirteen String keys precede constructor while the iterator Symbol remains last",
        r#"(function(){
            var selected=["trimLeft","toString","valueOf","anchor","big","blink",
                "bold","fixed","fontcolor","fontsize","italics","link","small",
                "strike","sub","sup","constructor"];
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
        "all descriptors names lengths GenericMagic-facing metadata and construct bits are exact",
        r#"(function(){
            var names=["anchor","big","blink","bold","fixed","fontcolor","fontsize",
                "italics","link","small","strike","sub","sup"];
            var lengths=[1,0,0,0,0,1,1,0,1,0,0,0,0],output=[],index=0;
            while(index<names.length){
                var name=names[index],fn=String.prototype[name];
                output.push(name+":"+__bits(String.prototype,name)+":"+fn.name+":"+
                    fn.length+":"+(fn.length===lengths[index])+":"+
                    Object.getOwnPropertyNames(fn).join(",")+":"+
                    __bits(fn,"length")+":"+__bits(fn,"name")+":"+
                    (Object.getPrototypeOf(fn)===Function.prototype)+":"+
                    (typeof fn)+":"+__isConstructor(fn));
                index++;
            }
            return output.join("|");
        })()"#,
    ),
    (
        "AutoInit identities are stable descriptor-backed and mutually distinct",
        r#"(function(){
            var names=["anchor","big","blink","bold","fixed","fontcolor","fontsize",
                "italics","link","small","strike","sub","sup"];
            var functions=[],output=[],index=0,other;
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
                functions[12]===String.prototype.valueOf);
            return output.join("|");
        })()"#,
    ),
];

const PROPERTY_CASES: &[(&str, &str)] = &[
    (
        "lazy anchor can be deleted before materialization",
        r#"(function(){
            var deleted=delete String.prototype.anchor;
            return [deleted,"anchor" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"anchor"),
                typeof String.prototype.anchor].join("|");
        })()"#,
    ),
    (
        "lazy bold assignment becomes an ordinary replacement",
        r#"(function(){
            String.prototype.bold=17;
            return [String.prototype.bold,__bits(String.prototype,"bold"),
                Object.prototype.hasOwnProperty.call(String.prototype,"bold"),
                typeof String.prototype.big].join("|");
        })()"#,
    ),
    (
        "a materialized link remains deletable without affecting its peers",
        r#"(function(){
            var link=String.prototype.link,anchor=String.prototype.anchor;
            var deleted=delete String.prototype.link;
            return [typeof link,deleted,"link" in String.prototype,
                Object.prototype.hasOwnProperty.call(String.prototype,"link"),
                String.prototype.anchor===anchor,typeof String.prototype.fontcolor].join("|");
        })()"#,
    ),
    (
        "overwriting multiple properties never aliases another selector",
        r#"(function(){
            var big=String.prototype.big,sup=String.prototype.sup;
            String.prototype.anchor=23;
            String.prototype.sub=29;
            var before=[String.prototype.big===big,String.prototype.sup===sup,
                String.prototype.anchor,String.prototype.sub];
            delete String.prototype.big;
            return before.concat(["big" in String.prototype,
                String.prototype.sup===sup]).join("|");
        })()"#,
    ),
];

const OUTPUT_CASES: &[(&str, &str)] = &[
    (
        "all thirteen selectors use their exact tag and attribute mapping",
        r#"(function(){
            var source="A&B";
            return [source.anchor("spot"),source.big(),source.blink(),source.bold(),
                source.fixed(),source.fontcolor("red"),source.fontsize(7),source.italics(),
                source.link("url"),source.small(),source.strike(),source.sub(),source.sup()].join("|");
        })()"#,
    ),
    (
        "empty source and empty attributes retain exact opening and closing markup",
        r#"(function(){return [
            "".anchor(""),"".fontcolor(""),"".fontsize(""),"".link(""),
            "".big(),"".bold(),"".fixed(),"".italics(),"".sup()
        ].join("|")})()"#,
    ),
    (
        "generic primitive wrapper and ordinary object receivers convert",
        r#"(function(){return [
            String.prototype.big.call(123),
            String.prototype.bold.call(true),
            String.prototype.fixed.call(7n),
            String.prototype.italics.call(Object()),
            String.prototype.sup.call(new String("xy")),
            String.prototype.link.call(42,"target")
        ].join("|")})()"#,
    ),
    (
        "attribute primitives and object-produced null or undefined stringify after the direct check",
        r#"(function(){
            var nullResult=Object(),undefinedResult=Object();
            nullResult[Symbol.toPrimitive]=function(){return null};
            undefinedResult[Symbol.toPrimitive]=function(){return undefined};
            return ["x".anchor(0),"x".fontcolor(false),"x".fontsize(7n),
                "x".link(Object()),"x".anchor(nullResult),
                "x".link(undefinedResult)].join("|");
        })()"#,
    ),
];

const UTF16_ESCAPE_CASES: &[(&str, &str)] = &[
    (
        "only double quotes are escaped in attributes",
        r#"(function(){
            var attribute="a\"b&<>'\u0000Z",source="\"&<>source";
            return [__units(source.anchor(attribute)),__units(source.fontcolor(attribute)),
                __units(source.fontsize("\"\"\"")),__units(source.link("&<>"))].join("|");
        })()"#,
    ),
    (
        "source and attribute preserve astral and lone surrogate code units",
        r#"(function(){
            var source="A\ud800\ud83d\ude00\udc00\u0000Z";
            var attribute="Q\udc00\ud83d\ude00\ud800\"R";
            return [source.length,attribute.length,__units(source.anchor(attribute)),
                __units(source.big()),__units(source.link(attribute))].join("|");
        })()"#,
    ),
    (
        "rope source and attribute cross 8192-unit and quote-expansion boundaries",
        r#"(function(){
            function grow(character,power){
                var value=character,index=0;
                while(index<power){value=value+value;index++}
                return value;
            }
            var source=(grow("s",13)+"\ud800")+("\udc00"+grow("t",13));
            var attribute=(grow("q",13)+"\"")+("\ud83d\ude00"+grow("r",13));
            var result=source.link(attribute),openEnd=result.indexOf(">")+1;
            return [source.length,attribute.length,result.length,openEnd,
                __units(result.slice(0,10)),__units(result.slice(8187,8205)),
                __units(result.slice(openEnd-12,openEnd+8)),
                __units(result.slice(openEnd+source.length-5))].join("|");
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "attribute variants convert receiver then argv0 with string hints and ignore extras",
        r#"(function(){
            function run(name){
                var log="",receiver=Object(),attribute=Object(),extra=Object();
                receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "R"};
                attribute[Symbol.toPrimitive]=function(hint){log+="attribute:"+hint+";";return "A\"B"};
                extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";throw "extra"};
                return String.prototype[name].call(receiver,attribute,extra,Symbol("later"))+":"+log;
            }
            return [run("anchor"),run("fontcolor"),run("fontsize"),run("link")].join("|");
        })()"#,
    ),
    (
        "ordinary ToPrimitive fallback uses toString before valueOf for both values",
        r#"(function(){
            var log="",receiver=Object(),attribute=Object();
            receiver.toString=function(){log+="receiver:toString;";return "R"};
            receiver.valueOf=function(){log+="receiver:valueOf;";throw "wrong receiver order"};
            attribute.toString=function(){log+="attribute:toString;";return "A"};
            attribute.valueOf=function(){log+="attribute:valueOf;";throw "wrong attribute order"};
            return String.prototype.link.call(receiver,attribute)+"|"+log;
        })()"#,
    ),
    (
        "receiver abrupt completion preserves identity and prevents attribute conversion",
        r#"(function(){
            var sentinel=Object(),receiver=Object(),attribute=Object(),log="";
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";throw sentinel};
            attribute[Symbol.toPrimitive]=function(hint){log+="attribute:"+hint+";";return "A"};
            try{String.prototype.anchor.call(receiver,attribute)}
            catch(error){return [(error===sentinel),log].join("|")}
            return "missing";
        })()"#,
    ),
    (
        "attribute abrupt completion follows receiver and preserves identity",
        r#"(function(){
            var sentinel=Object(),receiver=Object(),attribute=Object(),extra=Object(),log="";
            receiver[Symbol.toPrimitive]=function(hint){log+="receiver:"+hint+";";return "R"};
            attribute[Symbol.toPrimitive]=function(hint){log+="attribute:"+hint+";";throw sentinel};
            extra[Symbol.toPrimitive]=function(hint){log+="extra:"+hint+";";return "X"};
            try{String.prototype.fontcolor.call(receiver,attribute,extra)}
            catch(error){return [(error===sentinel),log].join("|")}
            return "missing";
        })()"#,
    ),
    (
        "all nine no-attribute variants ignore every supplied argument",
        r#"(function(){
            var names=["big","blink","bold","fixed","italics","small","strike","sub","sup"];
            var extra=Object(),hits=0,output=[],index=0;
            extra[Symbol.toPrimitive]=function(hint){hits++;throw "extra"};
            while(index<names.length){
                output.push(String.prototype[names[index]].call("R",extra,Symbol("later"),1n));
                index++;
            }
            output.push(hits);
            return output.join("|");
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "no-attribute method rejects null receiver",
        "String.prototype.bold.call(null)",
    ),
    (
        "attribute method rejects undefined receiver before its missing attribute",
        "String.prototype.anchor.call(undefined)",
    ),
    (
        "no-attribute method rejects Symbol receiver",
        "String.prototype.sup.call(Symbol('receiver'))",
    ),
    (
        "anchor rejects a missing attribute",
        "String.prototype.anchor.call('x')",
    ),
    (
        "link rejects an explicit undefined attribute",
        "'x'.link(undefined)",
    ),
    ("fontcolor rejects a null attribute", "'x'.fontcolor(null)"),
    (
        "fontsize rejects a Symbol attribute",
        "'x'.fontsize(Symbol('attribute'))",
    ),
    (
        "receiver ToPrimitive returning an object is rejected",
        r#"(function(){
            var receiver=Object();receiver[Symbol.toPrimitive]=function(){return Object()};
            return String.prototype.big.call(receiver);
        })()"#,
    ),
    (
        "attribute ToPrimitive returning an object is rejected",
        r#"(function(){
            var attribute=Object();attribute[Symbol.toPrimitive]=function(){return Object()};
            return "x".anchor(attribute);
        })()"#,
    ),
];

const CONSTRUCT_CASES: &[(&str, &str)] = &[(
    "all thirteen methods reject construction with their own exact name",
    r#"(function(){
        var names=["anchor","big","blink","bold","fixed","fontcolor","fontsize",
            "italics","link","small","strike","sub","sup"];
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
        "recursive receiver and attribute conversions throw catchably and recover",
        r#"(function(){
            var receiver=Object(),attribute=Object(),receiverError="",attributeError="";
            receiver[Symbol.toPrimitive]=function(){return String.prototype.bold.call(receiver)};
            try{String.prototype.bold.call(receiver)}catch(error){receiverError=error.name+":"+error.message}
            attribute[Symbol.toPrimitive]=function(){return "x".anchor(attribute)};
            try{"x".anchor(attribute)}catch(error){attributeError=error.name+":"+error.message}
            return [receiverError,attributeError,"x".bold(),"x".anchor("n")].join("|");
        })()"#,
    ),
    (
        "CreateHTML and existing String methods share one recursion guard",
        r#"(function(){
            var value=Object(),depth=0,errorName="",errorMessage="";
            value[Symbol.toPrimitive]=function(){
                depth++;
                if(depth%7===0)return String.prototype.bold.call(value);
                if(depth%7===1)return "x".anchor(value);
                if(depth%7===2)return String.prototype.trim.call(value);
                if(depth%7===3)return "x".padEnd(value,"_");
                if(depth%7===4)return "x".repeat(value);
                if(depth%7===5)return "abcdef".slice(value,4);
                return "abcdef".includes("a",value);
            };
            try{String.prototype.big.call(value)}
            catch(error){errorName=error.name;errorMessage=error.message}
            return [errorName,errorMessage,"x".big(),"x".link("u")," x ".trim(),
                "x".padEnd(3,"_"),"ok".repeat(2),"abcdef".slice(1,3),
                "abcdef".includes("bc")].join("|");
        })()"#,
    ),
];

#[test]
fn string_create_html_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String CreateHTML oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("properties", PROPERTY_CASES),
        ("outputs", OUTPUT_CASES),
        ("UTF-16 and escaping", UTF16_ESCAPE_CASES),
        ("order", ORDER_CASES),
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
fn string_create_html_graph_metadata_and_autoinit_match_pinned_quickjs() {
    compare_cases("String CreateHTML graph", GRAPH_CASES);
}

#[test]
fn string_create_html_property_delete_and_override_match_pinned_quickjs() {
    compare_cases("String CreateHTML properties", PROPERTY_CASES);
}

#[test]
fn string_create_html_all_selector_outputs_match_pinned_quickjs() {
    compare_cases("String CreateHTML outputs", OUTPUT_CASES);
}

#[test]
fn string_create_html_quote_only_escape_utf16_and_ropes_match_pinned_quickjs() {
    compare_cases("String CreateHTML UTF-16 and escaping", UTF16_ESCAPE_CASES);
}

#[test]
fn string_create_html_conversion_order_ignored_extras_and_throw_identity_match_pinned_quickjs() {
    compare_cases("String CreateHTML conversion order", ORDER_CASES);
}

#[test]
fn string_create_html_errors_and_nonconstructors_match_pinned_quickjs() {
    compare_cases("String CreateHTML errors", ERROR_CASES);
    compare_cases("String CreateHTML construction", CONSTRUCT_CASES);
}

#[test]
fn string_create_html_recursion_is_catchable_shared_and_recovers() {
    compare_cases("String CreateHTML stack recovery", STACK_CASES);
}

#[test]
fn string_create_html_defining_realms_user_throw_identity_and_caller_construct_error_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let anchor = property_callable(&runtime, &mut defining, &defining_prototype, "anchor");
    let big = property_callable(&runtime, &mut defining, &defining_prototype, "big");
    let defining_type_error = intrinsic_prototype(&runtime, &mut defining, "TypeError");
    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_ne!(defining_type_error, caller_type_error);
    assert_eq!(
        runtime.get_prototype_of(anchor.as_object()).unwrap(),
        Some(defining.function_prototype().unwrap()),
    );

    assert_native_error(
        &runtime,
        &mut caller,
        &big,
        Value::Null,
        &[],
        &defining_type_error,
    );
    assert_native_error(
        &runtime,
        &mut caller,
        &anchor,
        Value::String(JsString::try_from_utf8("x").unwrap()),
        &[],
        &defining_type_error,
    );
    let attribute_symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("attribute").unwrap()))
        .unwrap();
    assert_native_error(
        &runtime,
        &mut caller,
        &anchor,
        Value::String(JsString::try_from_utf8("x").unwrap()),
        &[Value::Symbol(attribute_symbol)],
        &defining_type_error,
    );

    let sentinel = caller.new_object().unwrap();
    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "htmlSentinel",
        Value::Object(sentinel.clone()),
    );
    let throwing_attribute = caller
        .eval(
            r#"(function(){
                var value=Object();
                value[Symbol.toPrimitive]=function(){throw htmlSentinel};
                return value;
            })()"#,
        )
        .unwrap();
    assert_eq!(
        caller.call(
            &anchor,
            Value::String(JsString::try_from_utf8("x").unwrap()),
            &[throwing_attribute],
        ),
        Err(RuntimeError::Exception),
    );
    assert_eq!(
        caller.take_exception().unwrap(),
        Some(Value::Object(sentinel)),
        "attribute conversion did not preserve the user-thrown value",
    );

    assert_eq!(caller.construct(&anchor, &[]), Err(RuntimeError::Exception));
    assert_eq!(
        runtime
            .get_prototype_of(&take_exception_object(&mut caller))
            .unwrap(),
        Some(caller_type_error),
        "non-constructor rejection did not use the caller realm",
    );
}

#[test]
fn string_create_html_callables_are_per_realm_distinct_and_collectable() {
    let runtime = Runtime::new();
    let retained = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_prototype = first.string_prototype().unwrap();
        let second_prototype = second.string_prototype().unwrap();
        let names = [
            "anchor",
            "big",
            "blink",
            "bold",
            "fixed",
            "fontcolor",
            "fontsize",
            "italics",
            "link",
            "small",
            "strike",
            "sub",
            "sup",
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
fn string_create_html_stack_overflow_uses_the_caller_realm_and_recovers() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = defining.string_prototype().unwrap();
    let bold = property_callable(&runtime, &mut defining, &defining_prototype, "bold");
    let defining_internal_error = intrinsic_prototype(&runtime, &mut defining, "InternalError");
    let caller_internal_error = intrinsic_prototype(&runtime, &mut caller, "InternalError");
    assert_ne!(defining_internal_error, caller_internal_error);

    define_data(
        &runtime,
        &caller.global_object().unwrap(),
        "foreignBold",
        Value::Object(bold.as_object().clone()),
    );
    let Value::Object(error) = caller
        .eval(
            r#"(function(){
                var receiver=Object(),localCall=Function.prototype.call;
                function invoke(){return localCall.call(foreignBold,receiver)}
                receiver[Symbol.toPrimitive]=function(){return invoke()};
                try{invoke()}catch(error){return error}
                return Object();
            })()"#,
        )
        .unwrap()
    else {
        panic!("recursive cross-realm CreateHTML did not return an error object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_internal_error),
        "pre-dispatch stack overflow did not use the caller realm",
    );
    assert_eq!(
        caller
            .call(
                &bold,
                Value::String(JsString::try_from_utf8("x").unwrap()),
                &[],
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("<b>x</b>").unwrap()),
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
