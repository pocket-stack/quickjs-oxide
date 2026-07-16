use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for observable RegExp semantics in pinned QuickJS
// 2026-06-04. The vectors deliberately stay below later String.prototype
// symbol hooks.

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+__bit(descriptor.configurable);
}
function __accessorBits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return (typeof descriptor.get)+":"+(descriptor.set===undefined)+":"+
        __bit(descriptor.enumerable)+__bit(descriptor.configurable);
}
function __isConstructor(value){
    try{Reflect.construct(function(){},[],value);return true}catch(_error){return false}
}
function __error(thunk){
    try{return "return:"+String(thunk())}
    catch(error){
        if(error!==null&&typeof error==="object")return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
function __show(value){
    if(value===undefined)return "undefined";
    if(value===null)return "null";
    if(typeof value==="number"){
        if(value!==value)return "NaN";
        if(value===0)return 1/value===-Infinity?"-0":"+0";
        if(value===Infinity)return "+Infinity";
        if(value===-Infinity)return "-Infinity";
    }
    return String(value);
}
function __callableMetadata(owner,key){
    var fn=owner[key];
    return String(key)+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
        __bits(owner,key)+":"+__bits(fn,"name")+":"+__bits(fn,"length");
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "RegExp graph descriptors prototype brands and species",
        r#"(function(){
            var species=Object.getOwnPropertyDescriptor(RegExp,Symbol.species);
            var instance=new RegExp("a","g");
            return [
                "global="+typeof RegExp+":"+__bits(globalThis,"RegExp"),
                "constructor="+RegExp.name+":"+RegExp.length+":"+__isConstructor(RegExp)+":"+
                    __bits(RegExp,"name")+":"+__bits(RegExp,"length")+":"+__bits(RegExp,"prototype"),
                "links="+(Object.getPrototypeOf(RegExp)===Function.prototype)+":"+
                    (Object.getPrototypeOf(RegExp.prototype)===Object.prototype)+":"+
                    (RegExp.prototype.constructor===RegExp)+":"+
                    (Object.getPrototypeOf(instance)===RegExp.prototype),
                "brands="+Object.prototype.toString.call(RegExp.prototype)+":"+
                    Object.prototype.toString.call(instance),
                "prototype="+__bits(RegExp.prototype,"constructor"),
                "species="+(typeof species.get)+":"+(species.set===undefined)+":"+
                    __bit(species.enumerable)+__bit(species.configurable)+":"+
                    species.get.name+":"+species.get.length+":"+__isConstructor(species.get)+":"+
                    (species.get.call(instance)===instance),
                "methods="+__bits(RegExp.prototype,"exec")+":"+
                    __bits(RegExp.prototype,"test")+":"+__bits(RegExp.prototype,"toString"),
                "accessors="+__accessorBits(RegExp.prototype,"source")+":"+
                    __accessorBits(RegExp.prototype,"flags")+":"+
                    __accessorBits(RegExp.prototype,"global")+":"+
                    __accessorBits(RegExp.prototype,"unicodeSets"),
                "lastIndex="+__bits(instance,"lastIndex")+":"+
                    Object.getOwnPropertyNames(instance).join(",")
            ].join("|");
        })()"#,
    ),
    (
        "RegExp native callable names lengths flags and constructor bits",
        r#"(function(){
            var names=["exec","test","toString"],accessors=["hasIndices","global",
                "ignoreCase","multiline","dotAll","unicode","unicodeSets","sticky",
                "source","flags"],output=["RegExp:"+RegExp.name+":"+RegExp.length+":"+
                __isConstructor(RegExp)],i,descriptor,getter;
            for(i=0;i<names.length;i++)output[output.length]=
                __callableMetadata(RegExp.prototype,names[i]);
            for(i=0;i<accessors.length;i++){
                descriptor=Object.getOwnPropertyDescriptor(RegExp.prototype,accessors[i]);
                getter=descriptor.get;
                output[output.length]=accessors[i]+":"+getter.name+":"+getter.length+":"+
                    __isConstructor(getter)+":"+__bits(getter,"name")+":"+
                    __bits(getter,"length")+":"+__accessorBits(RegExp.prototype,accessors[i]);
            }
            descriptor=Object.getOwnPropertyDescriptor(RegExp,Symbol.species);
            getter=descriptor.get;
            output[output.length]="species:"+getter.name+":"+getter.length+":"+
                __isConstructor(getter)+":"+__bits(getter,"name")+":"+__bits(getter,"length");
            return output.join("|");
        })()"#,
    ),
];

const CONSTRUCTOR_CASES: &[(&str, &str)] = &[
    (
        "RegExp call identity construct copy override and empty forms",
        r#"(function(){
            var original=RegExp("a","g");
            original.lastIndex=7;
            var same=RegExp(original),copy=new RegExp(original),override=new RegExp(original,"i");
            var emptyCall=RegExp(),emptyNew=new RegExp();
            return [same===original,copy!==original,copy.source,copy.flags,copy.lastIndex,
                Object.getPrototypeOf(copy)===RegExp.prototype,override.source,override.flags,
                override.lastIndex,emptyCall.source,emptyCall.flags,emptyNew.source,
                emptyNew.flags,emptyCall!==emptyNew].join("|");
        })()"#,
    ),
    (
        "RegExp identity observes match and constructor while exact branded copy ignores toString",
        r#"(function(){
            var log="",first=new RegExp("ab","g");
            Object.defineProperty(first,Symbol.match,{configurable:true,get:function(){log+="m";return true}});
            Object.defineProperty(first,"constructor",{configurable:true,get:function(){log+="c";return RegExp}});
            var same=RegExp(first),identityLog=log;
            var second=new RegExp("xy","i"),hit=false;
            second[Symbol.match]=false;
            second.toString=function(){hit=true;throw "must not stringify branded pattern"};
            var callCopy=RegExp(second),newCopy=new RegExp(second);
            return [same===first,identityLog,callCopy!==second,newCopy!==second,
                callCopy.source,callCopy.flags,newCopy.source,newCopy.flags,hit].join("|");
        })()"#,
    ),
    (
        "RegExp constructor conversion and derived prototype observation order",
        r#"(function(){
            var log="",pattern=Object(),flags=Object(),prototype={marker:42};
            Object.defineProperty(pattern,Symbol.match,{get:function(){log+="m";return false}});
            pattern.toString=function(){log+="p";return "a"};
            flags.toString=function(){log+="f";return "g"};
            var NewTarget=(function(){}).bind(null);
            Object.defineProperty(NewTarget,"prototype",{
                configurable:true,get:function(){log+="n";return prototype}
            });
            var result=Reflect.construct(RegExp,[pattern,flags],NewTarget);
            return [log,result.source,result.flags,result.lastIndex,
                Object.getPrototypeOf(result)===prototype,result.marker].join("|");
        })()"#,
    ),
    (
        "RegExp constructor error classes and left to right abrupt completion",
        r#"(function(){
            var log="",pattern=Object(),flags=Object(),boom={kind:"boom"};
            pattern.toString=function(){log+="p";return "["};
            flags.toString=function(){log+="f";return "gg"};
            var first=__error(function(){return new RegExp("a","gg")});
            var second=__error(function(){return new RegExp("a","z")});
            var third=__error(function(){return new RegExp("a","uv")});
            var fourth=__error(function(){return new RegExp("[","")});
            var fifth=__error(function(){return new RegExp(pattern,flags)});
            var abruptPattern=Object(),unreached=Object(),caught;
            abruptPattern.toString=function(){log+="x";throw boom};
            unreached.toString=function(){log+="BAD";return "g"};
            try{new RegExp(abruptPattern,unreached)}catch(error){caught=error===boom}
            return [first,second,third,fourth,fifth,log,caught].join("|");
        })()"#,
    ),
    (
        "RegExp constructor exposes pinned compile diagnostics",
        r#"(function(){
            return [
                __error(function(){return new RegExp(")")}),
                __error(function(){return new RegExp("(?x:a)")}),
                __error(function(){return new RegExp("\\!","u")}),
                __error(function(){return new RegExp("[\\!]","u")}),
                __error(function(){return new RegExp("\\-","u")}),
                __error(function(){return new RegExp("]","u")}),
                __error(function(){return new RegExp("{","u")}),
                __error(function(){return new RegExp("\\c","u")}),
                __error(function(){return new RegExp("\\x","u")}),
                __error(function(){return new RegExp("\\u","u")}),
                __error(function(){return new RegExp("^*")}),
                __error(function(){return new RegExp("^{","u")}),
                __error(function(){return new RegExp("^{1}","u")}),
                __error(function(){return new RegExp("\\b{1}","u")}),
                __error(function(){return new RegExp("a{1","u")}),
                __error(function(){return new RegExp("[\\01]","u")})
            ].join("|");
        })()"#,
    ),
    (
        "RegExp legacy class escape ranges fall back to unions while unicode rejects them",
        r#"(function(){
            var left=new RegExp("[\\d-a]"),right=new RegExp("[a-\\d]"),both=new RegExp("[\\d-\\w]"),
                identities=new RegExp("[\\8\\9\\k]");
            return [left.test("5"),left.test("-"),left.test("a"),left.test("b"),
                right.test("5"),right.test("-"),right.test("a"),right.test("b"),
                both.test("5"),both.test("-"),both.test("Z"),
                identities.test("8"),identities.test("9"),identities.test("k"),identities.test("7"),
                __error(function(){return new RegExp("[\\d-a]","u")}),
                __error(function(){return new RegExp("[a-\\d]","u")})].join("|");
        })()"#,
    ),
];

const LITERAL_CASES: &[(&str, &str)] = &[
    (
        "RegExp literals compile once and allocate a fresh branded value per evaluation",
        r#"(function(){
            function make(){return /a(b)?/dgi}
            var first=make(),second=make(),descriptor=
                Object.getOwnPropertyDescriptor(first,"lastIndex");
            first.lastIndex=1;
            var match=first.exec("zab");
            return [first!==second,first.source,first.flags,second.lastIndex,
                descriptor.writable,descriptor.enumerable,descriptor.configurable,
                match[0],match[1],match.index,first.lastIndex].join("|");
        })()"#,
    ),
    (
        "RegExp literals bypass the global constructor and mutable constructor prototype",
        r#"(function(){
            var intrinsic=RegExp.prototype,hits=0;
            RegExp.prototype={replacement:true};
            Object.defineProperty(globalThis,"RegExp",{configurable:true,get:function(){
                hits++;throw Error("observable constructor access");
            }});
            var literal=/a/g;
            return [hits,Object.getPrototypeOf(literal)===intrinsic,literal.source,
                literal.flags,literal.lastIndex].join("|");
        })()"#,
    ),
    (
        "RegExp literal source and flags preserve pinned canonical spelling",
        r#"(function(){
            var escaped=/a\/b/ims,empty=/(?:)/,newline=/\n/u;
            return [escaped.source,escaped.flags,empty.source,empty.flags,
                newline.source,newline.flags,newline.test("\n")].join("|");
        })()"#,
    ),
];

const ACCESSOR_CASES: &[(&str, &str)] = &[
    (
        "RegExp source flag accessors and canonical flags",
        r#"(function(){
            var value=new RegExp("a/b\nc\r","dgimsuy");
            return [value.source,value.flags,value.hasIndices,value.global,value.ignoreCase,
                value.multiline,value.dotAll,value.unicode,value.unicodeSets,value.sticky,
                RegExp.prototype.source,RegExp.prototype.flags,
                __show(RegExp.prototype.global),__show(RegExp.prototype.unicodeSets),
                RegExp.prototype.toString()].join("|");
        })()"#,
    ),
    (
        "RegExp flags getter is generic and reads d g i m s u v y in order",
        r#"(function(){
            var receiver=Object(),log="",names=["hasIndices","global","ignoreCase",
                "multiline","dotAll","unicode","unicodeSets","sticky"],letters="dgimsuvy",i;
            for(i=0;i<names.length;i++)(function(name,letter,index){
                Object.defineProperty(receiver,name,{get:function(){log+=letter;return index%2===0}});
            })(names[i],letters.charAt(i),i);
            var getter=Object.getOwnPropertyDescriptor(RegExp.prototype,"flags").get;
            return [getter.call(receiver),log,
                __error(function(){return getter.call(null)})].join("|");
        })()"#,
    ),
    (
        "RegExp source and individual flag getters reject wrong brands after prototype special case",
        r#"(function(){
            var source=Object.getOwnPropertyDescriptor(RegExp.prototype,"source").get;
            var global=Object.getOwnPropertyDescriptor(RegExp.prototype,"global").get;
            var indices=Object.getOwnPropertyDescriptor(RegExp.prototype,"hasIndices").get;
            return [source.call(RegExp.prototype),__show(global.call(RegExp.prototype)),
                __show(indices.call(RegExp.prototype)),
                __error(function(){return source.call({})}),
                __error(function(){return global.call({})}),
                __error(function(){return indices.call(1)})].join("|");
        })()"#,
    ),
    (
        "RegExp toString is generic and reads source before flags",
        r#"(function(){
            var receiver=Object(),log="";
            Object.defineProperty(receiver,"source",{get:function(){log+="s";return "body"}});
            Object.defineProperty(receiver,"flags",{get:function(){log+="f";return "gi"}});
            var first=RegExp.prototype.toString.call(receiver);
            return [first,log,
                RegExp.prototype.toString.call({source:17,flags:null}),
                __error(function(){return RegExp.prototype.toString.call(null)})].join("|");
        })()"#,
    ),
];

const EXEC_CASES: &[(&str, &str)] = &[
    (
        "RegExp exec publishes captures index input groups and d indices",
        r#"(function(){
            var match=new RegExp("(a)(b)?","d").exec("za"),indices=match.indices;
            return [match.length,match[0],match[1],__show(match[2]),match.index,match.input,
                __show(match.groups),Object.getPrototypeOf(match)===Array.prototype,
                Object.getOwnPropertyNames(match).join(","),
                __bits(match,"0"),__bits(match,"index"),__bits(match,"input"),
                __bits(match,"groups"),__bits(match,"indices"),
                indices.length,indices[0].join(","),indices[1].join(","),
                __show(indices[2]),__show(indices.groups),
                Object.getPrototypeOf(indices)===Array.prototype,
                Object.getPrototypeOf(indices[0])===Array.prototype].join("|");
        })()"#,
    ),
    (
        "RegExp exec searches leftmost and omits indices without d",
        r#"(function(){
            var regexp=new RegExp("(a)|(b)"),match=regexp.exec("zzb"),miss=regexp.exec("ccc");
            return [match[0],__show(match[1]),match[2],match.index,match.input,
                match.groups===undefined,("indices" in match),miss===null,
                regexp.lastIndex].join("|");
        })()"#,
    ),
    (
        "RegExp global and sticky exec update and reset lastIndex",
        r#"(function(){
            var global=new RegExp("a","g"),sticky=new RegExp("a","y"),output=[],match;
            global.lastIndex=1;match=global.exec("baac");
            output[output.length]=match.index+":"+global.lastIndex;
            match=global.exec("baac");output[output.length]=match.index+":"+global.lastIndex;
            match=global.exec("baac");output[output.length]=(match===null)+":"+global.lastIndex;
            sticky.lastIndex=1;match=sticky.exec("baac");
            output[output.length]=match.index+":"+sticky.lastIndex;
            sticky.lastIndex=3;match=sticky.exec("baac");
            output[output.length]=(match===null)+":"+sticky.lastIndex;
            return output.join("|");
        })()"#,
    ),
    (
        "RegExp exec ToString and lastIndex ToLength ordering",
        r#"(function(){
            var log="",regexp=new RegExp("a"),index=Object(),input=Object();
            index.valueOf=function(){log+="l";return 999};
            input.toString=function(){log+="s";return "ba"};
            regexp.lastIndex=index;
            var match=regexp.exec(input),unchanged=regexp.lastIndex===index;
            var global=new RegExp("a","g"),globalIndex=Object(),globalInput=Object();
            globalIndex.valueOf=function(){log+="L";return 1.9};
            globalInput.toString=function(){log+="S";return "baa"};
            global.lastIndex=globalIndex;
            var globalMatch=global.exec(globalInput);
            return [log,match.index,unchanged,globalMatch.index,global.lastIndex].join("|");
        })()"#,
    ),
    (
        "RegExp exec brand check precedes input conversion",
        r#"(function(){
            var hit=false,input=Object();
            input.toString=function(){hit=true;throw "must not convert"};
            var result=__error(function(){return RegExp.prototype.exec.call({},input)});
            return [result,hit].join("|");
        })()"#,
    ),
    (
        "RegExp global failure reset observes readonly lastIndex while non-global does not write",
        r#"(function(){
            var global=new RegExp("a","g"),plain=new RegExp("a");
            global.lastIndex=9;plain.lastIndex=9;
            Object.defineProperty(global,"lastIndex",{writable:false});
            Object.defineProperty(plain,"lastIndex",{writable:false});
            var first=__error(function(){return global.exec("a")});
            var second=plain.exec("a");
            return [first,global.lastIndex,second.index,plain.lastIndex].join("|");
        })()"#,
    ),
    (
        "RegExp global lastIndex clamps negatives and rejects infinity by resetting",
        r#"(function(){
            var negative=new RegExp("a","g"),fractional=new RegExp("a","g"),
                infinite=new RegExp("a","g"),a,b,c;
            negative.lastIndex=-3;a=negative.exec("ba");
            fractional.lastIndex=1.9;b=fractional.exec("baa");
            infinite.lastIndex=Infinity;c=infinite.exec("baa");
            return [a.index,negative.lastIndex,b.index,fractional.lastIndex,
                c===null,infinite.lastIndex].join("|");
        })()"#,
    ),
];

const TEST_CASES: &[(&str, &str)] = &[
    (
        "RegExp test uses custom exec and treats only null as failure",
        r#"(function(){
            var receiver=Object(),log="",input=Object(),count=0;
            input.toString=function(){log+="s";return "text"};
            Object.defineProperty(receiver,"exec",{get:function(){
                log+="g";
                return function(value){log+="e:"+(this===receiver)+":"+value;count++;return count===1?{}:null};
            }});
            var first=RegExp.prototype.test.call(receiver,input);
            var second=RegExp.prototype.test.call(receiver,input);
            return [first,second,log].join("|");
        })()"#,
    ),
    (
        "RegExp test rejects primitive custom exec results noncallable exec and nullish receivers",
        r#"(function(){
            var primitive={exec:function(){return 1}},noncallable={exec:17},missing={};
            return [
                __error(function(){return RegExp.prototype.test.call(primitive,"x")}),
                __error(function(){return RegExp.prototype.test.call(noncallable,"x")}),
                __error(function(){return RegExp.prototype.test.call(missing,"x")}),
                __error(function(){return RegExp.prototype.test.call(null,"x")}),
                __error(function(){return RegExp.prototype.test.call(undefined,"x")})
            ].join("|");
        })()"#,
    ),
    (
        "RegExp test gets custom exec before that method observes input and preserves abrupt completion",
        r#"(function(){
            var log="",receiver=Object(),input=Object(),boom={kind:"boom"},caught;
            input.toString=function(){log+="s";return "x"};
            Object.defineProperty(receiver,"exec",{get:function(){log+="g";throw boom}});
            try{RegExp.prototype.test.call(receiver,input)}catch(error){caught=error===boom}
            return [caught,log].join("|");
        })()"#,
    ),
    (
        "RegExp test honors overridden exec while builtin exec ignores it",
        r#"(function(){
            var regexp=new RegExp("a"),hit=0;
            regexp.exec=function(value){hit++;return value==="custom"?{}:null};
            var first=regexp.test("custom"),second=regexp.test("a");
            var builtin=RegExp.prototype.exec.call(regexp,"za");
            return [first,second,hit,builtin.index,builtin[0]].join("|");
        })()"#,
    ),
];

#[test]
fn regexp_intrinsic_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp intrinsic oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("constructor", CONSTRUCTOR_CASES),
        ("literal", LITERAL_CASES),
        ("accessors", ACCESSOR_CASES),
        ("exec", EXEC_CASES),
        ("test", TEST_CASES),
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
fn regexp_graph_and_native_metadata_match_pinned_quickjs() {
    compare_cases("RegExp graph", GRAPH_CASES);
}

#[test]
fn regexp_constructor_identity_copy_and_order_match_pinned_quickjs() {
    compare_cases("RegExp constructor", CONSTRUCTOR_CASES);
}

#[test]
fn regexp_literal_allocation_and_intrinsic_bypass_match_pinned_quickjs() {
    compare_cases("RegExp literals", LITERAL_CASES);
}

#[test]
fn regexp_literal_uses_the_bytecode_realm_and_is_fresh_on_every_execution() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_prototype = eval_object(
        &mut defining,
        "RegExp.prototype",
        "defining RegExp prototype",
    );
    let caller_prototype = eval_object(&mut caller, "RegExp.prototype", "caller RegExp prototype");
    let function = defining.compile("/realm/g").unwrap();

    // Mutating the constructor relationship after compilation must not affect
    // QuickJS's realm-canonical literal shape.
    defining
        .eval("RegExp.prototype={replacement:true}")
        .unwrap();
    let first = expect_object(caller.execute(&function).unwrap(), "first RegExp literal");
    let second = expect_object(caller.execute(&function).unwrap(), "second RegExp literal");

    assert_ne!(first, second);
    assert_eq!(
        runtime.get_prototype_of(&first).unwrap(),
        Some(defining_prototype)
    );
    assert_ne!(
        runtime.get_prototype_of(&first).unwrap(),
        Some(caller_prototype)
    );
}

#[test]
fn regexp_source_flags_and_accessors_match_pinned_quickjs() {
    compare_cases("RegExp accessors", ACCESSOR_CASES);
}

#[test]
fn regexp_exec_results_last_index_and_order_match_pinned_quickjs() {
    compare_cases("RegExp exec", EXEC_CASES);
}

#[test]
fn regexp_test_and_abstract_exec_match_pinned_quickjs() {
    compare_cases("RegExp test", TEST_CASES);
}

#[test]
fn regexp_cross_realm_prototypes_results_fallback_and_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let defining_constructor = regexp_constructor(&runtime, &mut defining);
    let defining_regexp_prototype = eval_object(
        &mut defining,
        "RegExp.prototype",
        "defining RegExp prototype",
    );
    let caller_regexp_prototype =
        eval_object(&mut caller, "RegExp.prototype", "caller RegExp prototype");
    let defining_array_prototype =
        eval_object(&mut defining, "Array.prototype", "defining Array prototype");
    let caller_array_prototype =
        eval_object(&mut caller, "Array.prototype", "caller Array prototype");
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
    assert_ne!(defining_regexp_prototype, caller_regexp_prototype);
    assert_ne!(defining_array_prototype, caller_array_prototype);
    assert_ne!(defining_type_error, caller_type_error);

    let pattern = Value::String(JsString::try_from_utf8("a").unwrap());
    let flags = Value::String(JsString::try_from_utf8("g").unwrap());
    let foreign_regexp = expect_object(
        caller
            .construct(&defining_constructor, &[pattern.clone(), flags])
            .unwrap(),
        "foreign RegExp construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&foreign_regexp).unwrap(),
        Some(defining_regexp_prototype.clone()),
        "RegExp construction did not use the constructor defining prototype",
    );

    let same = caller
        .call(
            &defining_constructor,
            Value::Undefined,
            &[Value::Object(foreign_regexp.clone())],
        )
        .unwrap();
    assert_eq!(
        same,
        Value::Object(foreign_regexp),
        "foreign RegExp function call did not take the identity shortcut",
    );

    let caller_new_target = eval_callable(
        &runtime,
        &mut caller,
        "(function(){var F=(function(){}).bind(null);F.prototype=17;return F})()",
        "caller primitive-prototype newTarget",
    );
    let fallback_regexp = expect_object(
        caller
            .construct_with_new_target(&defining_constructor, &caller_new_target, &[pattern])
            .unwrap(),
        "cross-realm RegExp fallback construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&fallback_regexp).unwrap(),
        Some(caller_regexp_prototype),
        "primitive newTarget.prototype did not fall back to the newTarget realm",
    );

    let exec = regexp_prototype_callable(&runtime, &mut defining, "exec");
    let caller_regexp = eval_object(&mut caller, "new RegExp('a')", "caller RegExp");
    let result = expect_object(
        caller
            .call(
                &exec,
                Value::Object(caller_regexp),
                &[Value::String(JsString::try_from_utf8("za").unwrap())],
            )
            .unwrap(),
        "foreign RegExp exec result",
    );
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype),
        "RegExp exec result did not use the builtin defining Array realm",
    );

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary object");
    assert_eq!(
        caller.call(&exec, Value::Object(ordinary), &[]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "foreign RegExp exec TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "RegExp builtin native error used the calling realm",
    );

    let test = regexp_prototype_callable(&runtime, &mut defining, "test");
    let throwing = eval_object(
        &mut caller,
        r#"(function(){
            var receiver=Object();
            receiver.exec=function(){throw new TypeError("caller exec")};
            return receiver;
        })()"#,
        "caller throwing custom exec receiver",
    );
    assert_eq!(
        caller.call(&test, Value::Object(throwing), &[]),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller custom exec TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp.test replaced a caller-realm user exception",
    );
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group}: set QJS_ORACLE to upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for &(description, source) in cases {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_rust_eval(&runtime, &mut context, source, description);
        let expected = observe_oracle(&oracle, source, description);
        if actual != expected {
            failures.push(format!(
                "{description}\nsource: {source:?}\noxide: {actual:?}\noracle: {expected:?}",
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{group} drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn observe_rust_eval(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    let source = observed_source(source);
    match context.eval(&source) {
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
    let source = observed_source(source);
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
        .args(["--std", "-e", wrapper, &source])
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

fn regexp_constructor(runtime: &Runtime, context: &mut Context) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, "RegExp")
}

fn regexp_prototype_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let prototype = eval_object(context, "RegExp.prototype", "RegExp prototype");
    property_callable(runtime, context, &prototype, name)
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
        .unwrap_or_else(|| panic!("{description} was not callable"))
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

fn expect_object(value: Value, description: &str) -> ObjectRef {
    let Value::Object(object) = value else {
        panic!("{description} was not an object");
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

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read string property {name}: {error}"))
    else {
        panic!("{name} was not a string");
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
