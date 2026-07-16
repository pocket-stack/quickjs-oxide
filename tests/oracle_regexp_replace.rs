use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for pinned QuickJS 2026-06-04
// `js_regexp_Symbol_replace` (`quickjs.c` 48622-48815), its direct matcher
// fast path (`quickjs.c` 48101-48215), abstract RegExpExec
// (`quickjs.c` 48217-48236), and shared GetSubstitution
// (`quickjs.c` 45661-45779).

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+
        __bit(descriptor.configurable);
}
function __isConstructor(value){
    try{Reflect.construct(function(){},[],value);return true}
    catch(_error){return false}
}
function __show(value){
    if(value===undefined)return "undefined";
    if(value===null)return "null";
    if(typeof value==="number"){
        if(value!==value)return "NaN";
        if(value===0)return 1/value===-Infinity?"-0":"+0";
    }
    return String(value);
}
function __completion(callback){
    try{return "return:"+String(callback())}
    catch(error){
        if(error!==null&&typeof error==="object")
            return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
function __units(value){
    var string=String(value),output=[],index=0;
    while(index<string.length){
        output[index]=string.charCodeAt(index).toString(16);
        index++;
    }
    return string.length+"["+output.join(",")+"]";
}
"#;

const METADATA_CASES: &[(&str, &str)] = &[
    (
        "RegExp Symbol.replace exposes pinned descriptor metadata and Symbol order",
        r#"(function(){
            var fn=RegExp.prototype[Symbol.replace],
                keys=Reflect.ownKeys(RegExp.prototype),
                selected=[],index=0,key;
            while(index<keys.length){
                key=keys[index++];
                if(key==="exec"||key==="compile"||key==="test"||key==="toString"||
                    key==="constructor"||key===Symbol.replace||key===Symbol.match||
                    key===Symbol.search||key===Symbol.split)
                    selected[selected.length]=String(key);
            }
            return [
                selected.join(","),
                __bits(RegExp.prototype,Symbol.replace),fn.name,fn.length,
                Object.getOwnPropertyNames(fn).join(","),
                __bits(fn,"name"),__bits(fn,"length"),
                __isConstructor(fn),
                Object.prototype.hasOwnProperty.call(fn,"prototype")
            ].join("|");
        })()"#,
    ),
    (
        "RegExp Symbol.replace AutoInit identity is stable and independently replaceable",
        r#"(function(){
            var first=RegExp.prototype[Symbol.replace],
                second=RegExp.prototype[Symbol.replace],
                stable=first===second,
                deleted=delete RegExp.prototype[Symbol.replace];
            RegExp.prototype[Symbol.replace]=17;
            return [stable,deleted,RegExp.prototype[Symbol.replace],
                __bits(RegExp.prototype,Symbol.replace),
                typeof first,first.name,first.length].join("|");
        })()"#,
    ),
];

const PREFIX_AND_ABSTRACT_EXEC_CASES: &[(&str, &str)] = &[
    (
        "RegExp Symbol.replace validates receiver then converts input replacement and flags",
        r#"(function(){
            var primitiveLog="",primitiveInput=Object(),primitiveReplacement=Object();
            primitiveInput.toString=function(){primitiveLog+="BAD-input;";return "x"};
            primitiveReplacement.toString=function(){primitiveLog+="BAD-replacement;";return "y"};
            var primitive=__completion(function(){
                return RegExp.prototype[Symbol.replace].call(
                    1,primitiveInput,primitiveReplacement);
            });

            var log="",receiver=Object(),input=Object(),replacement=Object(),flags=Object();
            input.toString=function(){log+="input-string;";return "abc"};
            replacement.toString=function(){log+="replacement-string;";return "X"};
            Object.defineProperty(receiver,"flags",{get:function(){
                log+="flags-get;";return flags;
            }});
            flags.toString=function(){log+="flags-string;";return ""};
            Object.defineProperty(receiver,"exec",{get:function(){
                log+="exec-get;";
                return function(value){
                    log+="exec-call:"+(this===receiver)+":"+value+":"+arguments.length+";";
                    return null;
                };
            }});
            var result=RegExp.prototype[Symbol.replace].call(receiver,input,replacement);
            return [primitive,primitiveLog,result,log].join("|");
        })()"#,
    ),
    (
        "abstract RegExpExec accepts null and rejects primitive results after the exact Get",
        r#"(function(){
            function run(result){
                var log="",receiver=Object();receiver.flags="";
                Object.defineProperty(receiver,"exec",{get:function(){
                    log+="get;";
                    return function(value){
                        log+="call:"+(this===receiver)+":"+value+";";
                        return result;
                    };
                }});
                return __completion(function(){
                    return RegExp.prototype[Symbol.replace].call(receiver,"abc","X");
                })+":"+log;
            }
            var ordinary=Object();ordinary.flags="";ordinary.exec=null;
            return [
                run(null),run(1),run("match"),
                __completion(function(){
                    return RegExp.prototype[Symbol.replace].call(ordinary,"abc","X");
                })
            ].join("|");
        })()"#,
    ),
    (
        "callable replacement is not converted before flags or execution",
        r#"(function(){
            var log="",receiver=Object(),calls=0;
            receiver.flags="";
            receiver.exec=function(){
                calls++;
                return {length:1,0:"b",index:1,groups:undefined};
            };
            function replacement(match,position,input){
                "use strict";
                log+="call:"+(this===undefined)+":"+match+":"+position+":"+input+":"+
                    arguments.length+";";
                var result=Object();
                result.toString=function(){log+="result-string;";return "X"};
                return result;
            }
            replacement.toString=function(){log+="BAD-function-string;";throw "function"};
            var result=RegExp.prototype[Symbol.replace].call(receiver,"abc",replacement);
            return result+"|"+calls+"|"+log;
        })()"#,
    ),
    (
        "Function.prototype.call trampoline preserves both logical native stack frames",
        r#"(function(){
            function countNativeCalls(stack){
                var text=String(stack),needle="at call (native)",count=0,index=0;
                while((index=text.indexOf(needle,index))>=0){
                    count++;index+=needle.length;
                }
                return count;
            }
            function capture(callback){
                try{callback();return "return"}
                catch(error){
                    return [
                        error.name,error.message,countNativeCalls(error.stack),
                        String(error.stack).indexOf("at boom")>=0
                    ].join(":");
                }
            }
            var thrown=capture(function(){
                Function.prototype.call.call(function boom(){throw Error("x")},null);
            });
            var noncallable=capture(function(){
                Function.prototype.call.call(1,null);
            });
            var c=Function.prototype.call,f=function(){return String(42)};
            var deep=c.call(
                c,c,c,c,c,c,c,c,c,c,c,c,c,c,c,c,c,c,c,c,f,null
            );
            return thrown+"|"+noncallable+"|"+deep;
        })()"#,
    ),
];

const TWO_PHASE_CASES: &[(&str, &str)] = &[
    (
        "global replacement collects all exec results before processing and rereads result zero",
        r#"(function(){
            var log="",receiver=Object(),state=7,calls=0,group1=Object(),group2=Object();
            receiver.flags="g";
            Object.defineProperty(receiver,"lastIndex",{
                get:function(){log+="last-get:"+__show(state)+";";return state},
                set:function(value){log+="last-set:"+__show(value)+";";state=value}
            });
            function result(label,index,matched,capture,groups){
                var value=Object();
                Object.defineProperty(value,"length",{get:function(){
                    log+=label+"-length;";return 2;
                }});
                Object.defineProperty(value,"0",{get:function(){
                    log+=label+"-zero;";return matched;
                }});
                Object.defineProperty(value,"1",{get:function(){
                    log+=label+"-capture;";return capture;
                }});
                Object.defineProperty(value,"index",{get:function(){
                    log+=label+"-index;";return index;
                }});
                Object.defineProperty(value,"groups",{get:function(){
                    log+=label+"-groups;";return groups;
                }});
                return value;
            }
            var first=result("first",0,"a","A",group1),
                second=result("second",1,"b","B",group2);
            Object.defineProperty(receiver,"exec",{get:function(){
                log+="exec-get;";
                return function(){
                    calls++;log+="exec-call:"+calls+";";
                    if(calls===1)return first;
                    if(calls===2)return second;
                    return null;
                };
            }});
            function replacement(match,capture,position,input,groups){
                "use strict";
                log+="replace:"+match+":"+capture+":"+position+":"+input+":"+
                    (groups===group1?"g1":groups===group2?"g2":"bad")+":"+
                    arguments.length+";";
                return "X";
            }
            var output=RegExp.prototype[Symbol.replace].call(receiver,"ab",replacement);
            return [output,state,calls,log].join("|");
        })()"#,
    ),
    (
        "global zero is converted once during collection and again during processing",
        r#"(function(){
            var log="",receiver=Object(),calls=0,zero=Object(),result=Object();
            receiver.flags="g";receiver.lastIndex=0;
            zero.toString=function(){log+="zero-string;";return "a"};
            Object.defineProperty(result,"0",{get:function(){log+="zero-get;";return zero}});
            result.length=1;result.index=0;result.groups=undefined;
            receiver.exec=function(){calls++;log+="exec:"+calls+";";return calls===1?result:null};
            var output=RegExp.prototype[Symbol.replace].call(receiver,"a","X");
            return [output,calls,log].join("|");
        })()"#,
    ),
];

const LENGTH_CAPTURE_GROUP_CASES: &[(&str, &str)] = &[
    (
        "QuickJS applies ToUint32 to result length before capture enumeration",
        r#"(function(){
            function run(rawLength,template){
                var log="",receiver=Object(),length=Object(),result=Object();
                receiver.flags="";
                length.valueOf=function(){log+="length-value;";return rawLength};
                Object.defineProperty(result,"length",{get:function(){
                    log+="length-get;";return length;
                }});
                result[0]="a";result.index=0;result.groups=undefined;
                Object.defineProperty(result,"1",{get:function(){log+="one;";return "ONE"}});
                Object.defineProperty(result,"2",{get:function(){log+="two;";return "TWO"}});
                receiver.exec=function(){return result};
                return RegExp.prototype[Symbol.replace].call(receiver,"a",template)+":"+log;
            }
            return [
                run(4294967297,"<$1>"),
                run(3.9,"<$1|$2|$3|$01|$20>")
            ].join("|");
        })()"#,
    ),
    (
        "capture conversion precedes groups and named substitutions observe boxed primitives",
        r#"(function(){
            var log="",receiver=Object(),capture=Object(),result=Object();
            receiver.flags="";
            capture.toString=function(){log+="capture-string;";return "C"};
            result.length=2;result[0]="a";result.index=0;
            Object.defineProperty(result,"1",{get:function(){log+="capture-get;";return capture}});
            Object.defineProperty(result,"groups",{get:function(){
                log+="groups-get;";return "123";
            }});
            receiver.exec=function(){return result};
            var output=RegExp.prototype[Symbol.replace].call(
                receiver,"a","$1:$<length>:$<0>:$<missing>");
            return output+"|"+log;
        })()"#,
    ),
    (
        "functional replacement receives stringified captures raw groups position and input",
        r#"(function(){
            var log="",receiver=Object(),capture=Object(),groups=Object(),result=Object();
            receiver.flags="";
            capture.toString=function(){log+="capture-string;";return "C"};
            result.length=3;result[0]="a";result.index=0;result[1]=capture;
            result[2]=undefined;result.groups=groups;
            receiver.exec=function(){return result};
            function replacement(match,first,second,position,input,named){
                "use strict";
                log+="call:"+(this===undefined)+":"+match+":"+first+":"+
                    (second===undefined)+":"+position+":"+input+":"+
                    (named===groups)+":"+arguments.length+";";
                return {toString:function(){log+="result-string;";return "X"}};
            }
            var output=RegExp.prototype[Symbol.replace].call(receiver,"a",replacement);
            return output+"|"+log;
        })()"#,
    ),
    (
        "real RegExp captures expand dollar before after match and numeric tokens",
        r#"(function(){
            var regexp=/(b)(c)?/;
            return [
                regexp[Symbol.replace]("abcd","$$|$&|$`|$'|$1|$2|$3|$01|$20"),
                /(a)|(b)/g[Symbol.replace]("ab","<$1:$2>"),
                /(z)?/[Symbol.replace]("a","<$1>")
            ].join("|");
        })()"#,
    ),
    (
        "functional apply limit is checked after every capture and groups observation",
        r#"(function(){
            var log="",receiver=Object(),result=Object(),lastCapture=Object(),
                groups=Object(),calls=0,error;
            receiver.flags="";
            result.length=65533;result[0]="a";result.index=0;
            lastCapture.toString=function(){log+="last-string;";return "LAST"};
            Object.defineProperty(result,"65532",{get:function(){
                log+="last-get;";return lastCapture;
            }});
            Object.defineProperty(result,"groups",{get:function(){
                log+="groups-get;";return groups;
            }});
            receiver.exec=function(){return result};
            function replacement(){
                calls++;log+="BAD-call;";return "X";
            }
            try{
                RegExp.prototype[Symbol.replace].call(receiver,"a",replacement);
                return "missing|"+log+"|"+calls;
            }catch(caught){
                error=caught;
            }
            return [
                error.name,error.message,
                Object.getPrototypeOf(error)===RangeError.prototype,
                log,calls
            ].join("|");
        })()"#,
    ),
];

const POSITION_CASES: &[(&str, &str)] = &[(
    "backward custom results still perform replacement work before being ignored",
    r#"(function(){
        var log="",receiver=Object(),calls=0,results=[
            {length:1,0:"0",index:3,groups:undefined},
            {length:1,0:"0",index:1,groups:undefined}
        ];
        receiver.flags="g";receiver.lastIndex=0;
        receiver.exec=function(){
            calls++;log+="exec:"+calls+";";
            return calls<=2?results[calls-1]:null;
        };
        function replacement(match,position,input){
            log+="replace:"+match+":"+position+":"+input+";";
            return position===3?"X":"Y";
        }
        var output=RegExp.prototype[Symbol.replace].call(receiver,"abcde",replacement);
        return [output,calls,log].join("|");
    })()"#,
)];

const LAST_INDEX_CASES: &[(&str, &str)] = &[
    (
        "empty global matches advance one UTF-16 unit or one u-v code point",
        r#"(function(){
            function run(flags,current){
                var log="",receiver=Object(),state=99,calls=0,result=Object();
                receiver.flags=flags;
                Object.defineProperty(receiver,"lastIndex",{
                    get:function(){log+="get:"+__show(state)+";";return state},
                    set:function(value){log+="set:"+__show(value)+";";state=value}
                });
                result.length=1;result[0]="";result.index=0;result.groups=undefined;
                receiver.exec=function(){
                    calls++;log+="exec:"+calls+";";
                    if(calls===1){state=current;return result}
                    return null;
                };
                var input=String.fromCharCode(0xd83d,0xde00,0xd800);
                var output=RegExp.prototype[Symbol.replace].call(receiver,input,"X");
                return flags+":"+__show(state)+":"+calls+":"+__units(output)+":"+log;
            }
            return [
                run("g",0),run("gu",0),run("gv",0),
                run("gu",1),run("gu",2)
            ].join("|");
        })()"#,
    ),
    (
        "builtin global and sticky replacements preserve pinned lastIndex transitions",
        r#"(function(){
            var global=/a/g,stickyHit=/a/y,stickyMiss=/a/y;
            global.lastIndex=7;stickyHit.lastIndex=1;stickyMiss.lastIndex=0;
            return [
                global[Symbol.replace]("baac","X"),global.lastIndex,
                stickyHit[Symbol.replace]("ba","X"),stickyHit.lastIndex,
                stickyMiss[Symbol.replace]("ba","X"),stickyMiss.lastIndex
            ].join("|");
        })()"#,
    ),
];

const FAST_PATH_CASES: &[(&str, &str)] = &[(
    "standard-regexp fast path activates only after exec AutoInit materialization",
    r#"(function(){
            var hits=0,regexp=/a/;
            Object.defineProperty(regexp,"ignoreCase",{get:function(){
                hits++;return false;
            }});
            var first=regexp[Symbol.replace]("a","X"),
                second=regexp[Symbol.replace]("a","Y");
            return [first,second,hits,
                RegExp.prototype.exec===RegExp.prototype.exec].join("|");
        })()"#,
)];

const RECURSION_CASES: &[(&str, &str)] = &[(
    "RegExp Symbol.replace exec recursion overflows catchably and the runtime recovers",
    r#"(function(){
        function recurse(depth){
            var receiver=Object();receiver.flags="";
            receiver.exec=function(){
                if(depth!==0)recurse(depth-1);
                return null;
            };
            return RegExp.prototype[Symbol.replace].call(receiver,"x","y");
        }
        var finite=recurse(8),completion;
        try{recurse(Infinity)}
        catch(error){completion=error.name+":"+error.message}
        return finite+"|"+completion+"|"+/b/[Symbol.replace]("abc","X")+"|"+
            /a/g[Symbol.replace]("aaa","X");
    })()"#,
)];

#[test]
fn regexp_replace_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp replace oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("metadata", METADATA_CASES),
        ("prefix and abstract exec", PREFIX_AND_ABSTRACT_EXEC_CASES),
        ("two-phase execution", TWO_PHASE_CASES),
        ("length captures and groups", LENGTH_CAPTURE_GROUP_CASES),
        ("positions", POSITION_CASES),
        ("lastIndex", LAST_INDEX_CASES),
        ("fast path", FAST_PATH_CASES),
        ("recursion", RECURSION_CASES),
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
fn regexp_replace_metadata_matches_pinned_quickjs() {
    compare_cases("RegExp replace metadata", METADATA_CASES);
}

#[test]
fn regexp_replace_prefix_and_abstract_exec_match_pinned_quickjs() {
    compare_cases(
        "RegExp replace prefix and abstract exec",
        PREFIX_AND_ABSTRACT_EXEC_CASES,
    );
}

#[test]
fn regexp_replace_two_phase_execution_matches_pinned_quickjs() {
    compare_cases("RegExp replace two-phase execution", TWO_PHASE_CASES);
}

#[test]
fn regexp_replace_length_captures_and_groups_match_pinned_quickjs() {
    compare_cases(
        "RegExp replace length captures and groups",
        LENGTH_CAPTURE_GROUP_CASES,
    );
}

#[test]
fn regexp_replace_position_and_last_index_match_pinned_quickjs() {
    compare_cases("RegExp replace positions", POSITION_CASES);
    compare_cases("RegExp replace lastIndex", LAST_INDEX_CASES);
}

#[test]
#[ignore = "R1i standard-RegExp direct matcher parity"]
fn regexp_replace_standard_fast_path_matches_pinned_quickjs() {
    compare_cases("RegExp replace standard fast path", FAST_PATH_CASES);
}

#[test]
fn regexp_replace_intrinsic_uses_its_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let Some(replace) = eval_optional_callable(
        &runtime,
        &mut defining,
        "RegExp.prototype[Symbol.replace]",
        "defining RegExp Symbol.replace",
    ) else {
        eprintln!("SKIP RegExp replace defining-realm lock: intrinsic is not published yet");
        return;
    };

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

    assert_eq!(
        caller.call(
            &replace,
            Value::Int(1),
            &[string_value("x"), string_value("y")]
        ),
        Err(RuntimeError::Exception),
    );
    let native_error =
        take_exception_object(&mut caller, "defining RegExp Symbol.replace TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "RegExp Symbol.replace native TypeError used the caller realm",
    );

    let foreign = eval_object(
        &mut caller,
        r#"(function(){
            var value=Object();value.flags="";
            value.exec=function(){return null};
            return value;
        })()"#,
        "caller abstract RegExp receiver",
    );
    assert_eq!(
        caller
            .call(
                &replace,
                Value::Object(foreign),
                &[string_value("subject"), string_value("X")],
            )
            .unwrap(),
        string_value("subject"),
        "defining RegExp Symbol.replace rejected a foreign ordinary receiver",
    );

    let throwing = eval_object(
        &mut caller,
        r#"(function(){
            var value=Object();value.flags="";
            Object.defineProperty(value,"exec",{get:function(){
                throw new TypeError("caller");
            }});
            return value;
        })()"#,
        "caller throwing exec getter",
    );
    assert_eq!(
        caller.call(
            &replace,
            Value::Object(throwing),
            &[string_value("subject"), string_value("X")],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller exec getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "RegExp Symbol.replace replaced a caller-realm user exception",
    );
}

#[test]
fn regexp_replace_recursion_matches_pinned_quickjs() {
    std::thread::Builder::new()
        .name("regexp-replace-oracle-stack".into())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| compare_cases("RegExp replace recursion", RECURSION_CASES))
        .unwrap()
        .join()
        .unwrap();
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

fn eval_optional_callable(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> Option<CallableRef> {
    let value = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"));
    let Value::Object(object) = value else {
        return None;
    };
    runtime
        .as_callable(&object)
        .unwrap_or_else(|error| panic!("inspect {description}: {error}"))
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

fn string_value(value: &str) -> Value {
    Value::String(JsString::try_from_utf8(value).unwrap())
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
