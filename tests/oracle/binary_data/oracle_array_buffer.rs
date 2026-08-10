use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, RuntimeError, Value};

struct Case {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

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
function __completion(thunk){
    try{return "return:"+String(thunk())}
    catch(error){return "throw:"+error.name+":"+error.message}
}
function __keys(object){return Reflect.ownKeys(object).map(String).join(",")}
function __metadata(owner,key){
    var fn=owner[key];
    return String(key)+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
        __bits(owner,key)+":"+__keys(fn);
}
"#;

const CASES: &[Case] = &[
    Case {
        description: "constructor prototype descriptors and method graph",
        source: r#"(function(){
            var byteDescriptor=Object.getOwnPropertyDescriptor(
                ArrayBuffer.prototype,"byteLength");
            var speciesDescriptor=Object.getOwnPropertyDescriptor(
                ArrayBuffer,Symbol.species);
            return [
                "global="+__bits(globalThis,"ArrayBuffer"),
                "ctor="+ArrayBuffer.name+":"+ArrayBuffer.length+":"+
                    __isConstructor(ArrayBuffer)+":"+__keys(ArrayBuffer),
                "links="+(Object.getPrototypeOf(ArrayBuffer)===Function.prototype)+":"+
                    (Object.getPrototypeOf(ArrayBuffer.prototype)===Object.prototype)+":"+
                    (ArrayBuffer.prototype.constructor===ArrayBuffer),
                "proto="+__keys(ArrayBuffer.prototype),
                "methods="+["resize","slice","transfer","transferToFixedLength"].
                    map(function(key){return __metadata(ArrayBuffer.prototype,key)}).join(";"),
                "byte="+__bits(ArrayBuffer.prototype,"byteLength")+":"+
                    byteDescriptor.get.name+":"+byteDescriptor.get.length+":"+
                    __isConstructor(byteDescriptor.get),
                "species="+__bits(ArrayBuffer,Symbol.species)+":"+
                    speciesDescriptor.get.name+":"+speciesDescriptor.get.length+":"+
                    speciesDescriptor.get.call(7),
                "tag="+__bits(ArrayBuffer.prototype,Symbol.toStringTag)+":"+
                    ArrayBuffer.prototype[Symbol.toStringTag],
                "view="+ArrayBuffer.isView(new ArrayBuffer(0))+":"+
                    ArrayBuffer.isView({})
            ].join("|");
        })()"#,
        expected: "global=101|ctor=ArrayBuffer:1:true:length,name,isView,prototype,Symbol(Symbol.species)|links=true:true:true|proto=byteLength,maxByteLength,resizable,detached,resize,slice,transfer,transferToFixedLength,constructor,Symbol(Symbol.toStringTag)|methods=resize:resize:1:false:101:length,name;slice:slice:2:false:101:length,name;transfer:transfer:0:false:101:length,name;transferToFixedLength:transferToFixedLength:0:false:101:length,name|byte=001:get byteLength:0:false|species=001:get [Symbol.species]:0:7|tag=001:ArrayBuffer|view=false:false",
    },
    Case {
        description: "constructor coercion newTarget and QuickJS Int64 quirks",
        source: r#"(function(){
            var log="",custom=Object.create(ArrayBuffer.prototype);
            var NewTarget=(function(){}).bind(null);
            Object.defineProperty(NewTarget,"prototype",{
                get:function(){log+="p";return custom}
            });
            var length={valueOf:function(){log+="l";return 3.9}};
            var maximum={valueOf:function(){log+="v";return 7.9}};
            var options={};
            Object.defineProperty(options,"maxByteLength",{
                get:function(){log+="m";return maximum}
            });
            var buffer=Reflect.construct(ArrayBuffer,[length,options],NewTarget);
            var negative=__completion(function(){
                return Reflect.construct(
                    ArrayBuffer,[0,{maxByteLength:-1}],NewTarget);
            });
            var infinity=new ArrayBuffer(0,{maxByteLength:Infinity});
            var wrapped=new ArrayBuffer(
                0,{maxByteLength:18446744073709551616});
            return [
                log,Object.getPrototypeOf(buffer)===custom,
                buffer.byteLength,buffer.maxByteLength,buffer.resizable,
                negative,infinity.maxByteLength,infinity.resizable,
                wrapped.maxByteLength,wrapped.resizable,
                new ArrayBuffer(2,1).byteLength
            ].join("|");
        })()"#,
        expected: "lmvpp|true|3|7|true|throw:RangeError:invalid max array buffer length|0|true|0|true|2",
    },
    Case {
        description: "resize transfer and fixed-length transfer state",
        source: r#"(function(){
            var fixed=new ArrayBuffer(4);
            var resizable=new ArrayBuffer(4,{maxByteLength:8}),log="";
            resizable.constructor={
                get [Symbol.species](){log+="species";throw 1}
            };
            resizable.resize({
                valueOf:function(){log+="resize";return 6}
            });
            var moved=resizable.transfer({
                valueOf:function(){log+="transfer";return 7}
            });
            var fixedMoved=moved.transferToFixedLength(3);
            return [
                log,resizable.detached,resizable.byteLength,
                resizable.maxByteLength,resizable.resizable,
                moved.detached,moved.byteLength,moved.maxByteLength,moved.resizable,
                fixedMoved.byteLength,fixedMoved.maxByteLength,fixedMoved.resizable,
                moved.detached,fixed.transfer().resizable
            ].join("|");
        })()"#,
        expected: "resizetransfer|true|0|8|true|true|0|8|true|3|3|false|true|false",
    },
    Case {
        description: "slice species ordering and post-constructor revalidation",
        source: r#"(function(){
            var log="",source=new ArrayBuffer(8,{maxByteLength:12});
            var start={valueOf:function(){log+="s";return 2}};
            var end={valueOf:function(){log+="e";return 6}};
            source.constructor={
                get [Symbol.species](){
                    log+="g";
                    return function(length){
                        log+="c"+length;
                        return new ArrayBuffer(length+1);
                    };
                }
            };
            var target=source.slice(start,end);
            var same=new ArrayBuffer(2);
            same.constructor={
                get [Symbol.species](){return function(){return same}}
            };
            var detached=new ArrayBuffer(4);
            detached.constructor={
                get [Symbol.species](){
                    return function(){
                        detached.transfer();
                        return new ArrayBuffer(4);
                    };
                }
            };
            var shrunk=new ArrayBuffer(8,{maxByteLength:8});
            shrunk.constructor={
                get [Symbol.species](){
                    return function(){
                        shrunk.resize(1);
                        return new ArrayBuffer(8);
                    };
                }
            };
            return [
                log,target.byteLength,target.resizable,
                __completion(function(){return same.slice(0)}),
                __completion(function(){return detached.slice(0)}),
                __completion(function(){return shrunk.slice(2,6)}),
                __completion(function(){
                    var buffer=new ArrayBuffer(4);
                    buffer.constructor={
                        get [Symbol.species](){
                            return function(){return {}};
                        }
                    };
                    return buffer.slice(0);
                }),
                __completion(function(){
                    var buffer=new ArrayBuffer(4);
                    buffer.constructor={
                        get [Symbol.species](){
                            return function(){return new ArrayBuffer(1)};
                        }
                    };
                    return buffer.slice(0);
                })
            ].join("|");
        })()"#,
        expected: "segc4|5|false|throw:TypeError:cannot use identical ArrayBuffer|throw:TypeError:ArrayBuffer is detached|throw:TypeError:ArrayBuffer is detached|throw:TypeError:ArrayBuffer object expected|throw:TypeError:new ArrayBuffer is too small",
    },
    Case {
        description: "detached getters brand errors and toStringTag fallback",
        source: r#"(function(){
            var source=new ArrayBuffer(4,{maxByteLength:8});
            var moved=source.transfer();
            var tagged=new ArrayBuffer(0);
            var before=Object.prototype.toString.call(tagged);
            delete ArrayBuffer.prototype[Symbol.toStringTag];
            return [
                source.byteLength,source.maxByteLength,source.resizable,
                source.detached,moved.byteLength,moved.maxByteLength,
                moved.resizable,moved.detached,
                __completion(function(){return source.resize(1)}),
                __completion(function(){return source.slice(0)}),
                __completion(function(){return source.transfer()}),
                before,Object.prototype.toString.call(tagged),
                __completion(function(){
                    return Object.getOwnPropertyDescriptor(
                        ArrayBuffer.prototype,"byteLength").get.call({});
                })
            ].join("|");
        })()"#,
        expected: "0|8|true|true|4|8|true|false|throw:TypeError:ArrayBuffer is detached|throw:TypeError:ArrayBuffer is detached|throw:TypeError:ArrayBuffer is detached|[object ArrayBuffer]|[object Object]|throw:TypeError:ArrayBuffer object expected",
    },
];

#[test]
fn array_buffer_vectors_match_frozen_observations() {
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            case.expected,
            "{}",
            case.description
        );
    }
}

#[test]
fn array_buffer_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP ArrayBuffer oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    for case in CASES {
        assert_eq!(
            oracle_observation(&oracle, case),
            case.expected,
            "{}",
            case.description,
        );
    }
}

#[test]
fn array_buffer_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP ArrayBuffer differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            oracle_observation(&oracle, case),
            "{}",
            case.description,
        );
    }
}

#[test]
fn context_detach_array_buffer_uses_the_same_idempotent_core() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let buffer = context
        .eval("globalThis.__buffer=new ArrayBuffer(4,{maxByteLength:8});__buffer")
        .unwrap();
    context.detach_array_buffer(&Value::Undefined).unwrap();
    let ordinary = Value::Object(context.new_object().unwrap());
    context.detach_array_buffer(&ordinary).unwrap();
    context.detach_array_buffer(&buffer).unwrap();
    context.detach_array_buffer(&buffer).unwrap();
    let Value::String(observation) = context
        .eval(
            "[__buffer.byteLength,__buffer.maxByteLength,\
             __buffer.resizable,__buffer.detached].join('|')",
        )
        .unwrap()
    else {
        panic!("detach observation was not a string");
    };
    assert_eq!(observation.to_utf8_lossy(), "0|8|true|true");
}

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn oxide_observation(case: &Case) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(&observed_source(case.source)) {
        Ok(Value::String(value)) => value.to_utf8_lossy(),
        Ok(value) => panic!(
            "Oxide returned a non-string for {}: {value:?}",
            case.description,
        ),
        Err(RuntimeError::Exception) => {
            let exception = context.take_exception().unwrap();
            panic!("Oxide threw for {}: {exception:?}", case.description,);
        }
        Err(error) => panic!("Oxide failed for {}: {error}", case.description),
    }
}

fn oracle_observation(oracle: &OsStr, case: &Case) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print(String(value));
} catch (error) {
  print("UNEXPECTED THROW: " + error.name + ": " + error.message);
}
"#;
    let source = observed_source(case.source);
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, &source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {}: {error}", case.description));
    assert!(
        output.status.success(),
        "QuickJS failed for {}: {}",
        case.description,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}
