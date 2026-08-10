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
function __keys(object){return Reflect.ownKeys(object).map(String).join(",")}
function __metadata(owner,key){
    var fn=owner[key];
    return String(key)+":"+fn.name+":"+fn.length+":"+__isConstructor(fn)+":"+
        __bits(owner,key)+":"+__keys(fn);
}
function __accessor(owner,key){
    var descriptor=Object.getOwnPropertyDescriptor(owner,key);
    var getter=descriptor.get;
    return String(key)+":"+getter.name+":"+getter.length+":"+
        __isConstructor(getter)+":"+__bits(owner,key)+":"+__keys(getter);
}
function __norm(value){
    if(typeof value==="bigint")return String(value)+"n";
    if(typeof value==="number"){
        if(value!==value)return "NaN";
        if(Object.is(value,-0))return "-0";
        if(value===Infinity)return "Infinity";
        if(value===-Infinity)return "-Infinity";
    }
    return String(value);
}
function __outcome(thunk){
    try{return "return:"+__norm(thunk())}
    catch(error){return "throw:"+error.name}
}
function __hex(view,start,length){
    var result="";
    for(var index=0;index<length;index++){
        var byte=view.getUint8(start+index).toString(16);
        result+=(byte.length===1?"0":"")+byte;
    }
    return result;
}
"#;

const CASES: &[Case] = &[
    Case {
        description: "constructor prototype descriptors and complete method graph",
        source: r#"(function(){
            var getters=["buffer","byteLength","byteOffset"];
            var suffixes=[
                "Int8","Uint8","Int16","Uint16","Int32","Uint32",
                "BigInt64","BigUint64","Float16","Float32","Float64"
            ];
            var methods=[];
            suffixes.forEach(function(suffix){
                methods.push(__metadata(DataView.prototype,"get"+suffix));
            });
            suffixes.forEach(function(suffix){
                methods.push(__metadata(DataView.prototype,"set"+suffix));
            });
            var view=new DataView(new ArrayBuffer(1));
            return [
                "global="+__bits(globalThis,"DataView"),
                "ctor="+DataView.name+":"+DataView.length+":"+
                    __isConstructor(DataView)+":"+__keys(DataView),
                "links="+(Object.getPrototypeOf(DataView)===Function.prototype)+":"+
                    (Object.getPrototypeOf(DataView.prototype)===Object.prototype)+":"+
                    (DataView.prototype.constructor===DataView),
                "proto="+__keys(DataView.prototype),
                "accessors="+getters.map(function(key){
                    return __accessor(DataView.prototype,key);
                }).join(";"),
                "methods="+methods.join(";"),
                "tag="+__bits(DataView.prototype,Symbol.toStringTag)+":"+
                    DataView.prototype[Symbol.toStringTag],
                "brand="+ArrayBuffer.isView(view)+":"+
                    ArrayBuffer.isView(Object.create(DataView.prototype))+":"+
                    ArrayBuffer.isView(new Proxy(view,{}))
            ].join("|");
        })()"#,
        expected: concat!(
            "global=101|ctor=DataView:1:true:length,name,prototype|links=true:true:true|",
            "proto=buffer,byteLength,byteOffset,getInt8,getUint8,getInt16,getUint16,",
            "getInt32,getUint32,getBigInt64,getBigUint64,getFloat16,getFloat32,getFloat64,",
            "setInt8,setUint8,setInt16,setUint16,setInt32,setUint32,setBigInt64,setBigUint64,",
            "setFloat16,setFloat32,setFloat64,constructor,Symbol(Symbol.toStringTag)|",
            "accessors=buffer:get buffer:0:false:001:length,name;",
            "byteLength:get byteLength:0:false:001:length,name;",
            "byteOffset:get byteOffset:0:false:001:length,name|",
            "methods=getInt8:getInt8:1:false:101:length,name;",
            "getUint8:getUint8:1:false:101:length,name;",
            "getInt16:getInt16:1:false:101:length,name;",
            "getUint16:getUint16:1:false:101:length,name;",
            "getInt32:getInt32:1:false:101:length,name;",
            "getUint32:getUint32:1:false:101:length,name;",
            "getBigInt64:getBigInt64:1:false:101:length,name;",
            "getBigUint64:getBigUint64:1:false:101:length,name;",
            "getFloat16:getFloat16:1:false:101:length,name;",
            "getFloat32:getFloat32:1:false:101:length,name;",
            "getFloat64:getFloat64:1:false:101:length,name;",
            "setInt8:setInt8:2:false:101:length,name;",
            "setUint8:setUint8:2:false:101:length,name;",
            "setInt16:setInt16:2:false:101:length,name;",
            "setUint16:setUint16:2:false:101:length,name;",
            "setInt32:setInt32:2:false:101:length,name;",
            "setUint32:setUint32:2:false:101:length,name;",
            "setBigInt64:setBigInt64:2:false:101:length,name;",
            "setBigUint64:setBigUint64:2:false:101:length,name;",
            "setFloat16:setFloat16:2:false:101:length,name;",
            "setFloat32:setFloat32:2:false:101:length,name;",
            "setFloat64:setFloat64:2:false:101:length,name|",
            "tag=001:DataView|brand=true:false:false",
        ),
    },
    Case {
        description: "all eleven element formats preserve numeric boundaries and endian bytes",
        source: r#"(function(){
            var specs=[
                ["Int8",1,255],
                ["Uint8",1,-1],
                ["Int16",2,32769],
                ["Uint16",2,-2],
                ["Int32",4,2147483649],
                ["Uint32",4,-1],
                ["BigInt64",8,9223372036854775809n],
                ["BigUint64",8,-1n],
                ["Float16",2,65504],
                ["Float32",4,3.402823669209385e38],
                ["Float64",8,-0]
            ];
            var observations=specs.map(function(spec){
                var suffix=spec[0],width=spec[1],value=spec[2];
                var view=new DataView(new ArrayBuffer(width*2));
                view["set"+suffix](0,value,false);
                var big=__hex(view,0,width)+":"+
                    __norm(view["get"+suffix](0,false));
                view["set"+suffix](width,value,true);
                var little=__hex(view,width,width)+":"+
                    __norm(view["get"+suffix](width,true));
                return suffix+"="+big+":"+little;
            });
            var edges=new DataView(new ArrayBuffer(8));
            edges.setInt8(0,Infinity);
            edges.setFloat16(2,NaN);
            return observations.join("|")+"|edges="+
                __norm(edges.getInt8(0))+":"+
                __norm(edges.getFloat16(2))+":"+
                __outcome(function(){return edges.setBigInt64(0,1)} )+":"+
                __outcome(function(){return edges.setInt32(0,1n)});
        })()"#,
        expected: concat!(
            "Int8=ff:-1:ff:-1|Uint8=ff:255:ff:255|",
            "Int16=8001:-32767:0180:-32767|Uint16=fffe:65534:feff:65534|",
            "Int32=80000001:-2147483647:01000080:-2147483647|",
            "Uint32=ffffffff:4294967295:ffffffff:4294967295|",
            "BigInt64=8000000000000001:-9223372036854775807n:",
            "0100000000000080:-9223372036854775807n|",
            "BigUint64=ffffffffffffffff:18446744073709551615n:",
            "ffffffffffffffff:18446744073709551615n|",
            "Float16=7bff:65504:ff7b:65504|",
            "Float32=7f800000:Infinity:0000807f:Infinity|",
            "Float64=8000000000000000:-0:0000000000000080:-0|",
            "edges=0:NaN:throw:TypeError:throw:TypeError",
        ),
    },
    Case {
        description: "fixed and tracking views become out of bounds and recover across RAB resize",
        source: r#"(function(){
            function state(label,view,buffer){
                return label+":"+
                    __outcome(function(){return view.byteOffset})+":"+
                    __outcome(function(){return view.byteLength})+":"+
                    (view.buffer===buffer);
            }
            var buffer=new ArrayBuffer(8,{maxByteLength:16});
            var fixed=new DataView(buffer,2,4);
            var tracking=new DataView(buffer,2);
            fixed.setUint16(0,0x1234);
            tracking.setUint8(5,0x7f);
            var result=[
                state("initial-fixed",fixed,buffer),
                state("initial-tracking",tracking,buffer)
            ];
            buffer.resize(4);
            result.push(state("shrink4-fixed",fixed,buffer));
            result.push(state("shrink4-tracking",tracking,buffer));
            result.push("shrink4-read="+
                __outcome(function(){return fixed.getUint8(0)})+":"+
                __outcome(function(){return tracking.getUint16(0)}));
            buffer.resize(8);
            result.push(state("grow8-fixed",fixed,buffer));
            result.push(state("grow8-tracking",tracking,buffer));
            result.push("grow8-read="+fixed.getUint16(0)+":"+
                tracking.getUint8(5));
            buffer.resize(2);
            result.push(state("equal-offset",tracking,buffer));
            result.push("equal-offset-read="+
                __outcome(function(){return tracking.getUint8(0)}));
            buffer.resize(1);
            result.push(state("shrink1-fixed",fixed,buffer));
            result.push(state("shrink1-tracking",tracking,buffer));
            buffer.resize(10);
            result.push(state("grow10-fixed",fixed,buffer));
            result.push(state("grow10-tracking",tracking,buffer));
            result.push("grow10-read="+fixed.getUint16(0)+":"+
                tracking.getUint8(5));
            return result.join("|");
        })()"#,
        expected: concat!(
            "initial-fixed:return:2:return:4:true|",
            "initial-tracking:return:2:return:6:true|",
            "shrink4-fixed:throw:TypeError:throw:TypeError:true|",
            "shrink4-tracking:return:2:return:2:true|",
            "shrink4-read=throw:TypeError:return:4660|",
            "grow8-fixed:return:2:return:4:true|",
            "grow8-tracking:return:2:return:6:true|grow8-read=4660:0|",
            "equal-offset:return:2:return:0:true|equal-offset-read=throw:RangeError|",
            "shrink1-fixed:throw:TypeError:throw:TypeError:true|",
            "shrink1-tracking:throw:TypeError:throw:TypeError:true|",
            "grow10-fixed:return:2:return:4:true|",
            "grow10-tracking:return:2:return:8:true|grow10-read=0:0",
        ),
    },
    Case {
        description: "detach preserves the DataView brand and buffer edge while invalidating access",
        source: r#"(function(){
            var buffer=new ArrayBuffer(8,{maxByteLength:16});
            var fixed=new DataView(buffer,2,4);
            var tracking=new DataView(buffer,2);
            fixed.setUint32(0,0x12345678);
            var moved=buffer.transfer();
            var bufferGetter=Object.getOwnPropertyDescriptor(
                DataView.prototype,"buffer").get;
            return [
                "source="+buffer.byteLength+":"+buffer.maxByteLength+":"+
                    buffer.resizable+":"+buffer.detached,
                "moved="+moved.byteLength+":"+moved.maxByteLength+":"+
                    moved.resizable+":"+moved.detached,
                "edges="+(fixed.buffer===buffer)+":"+
                    (tracking.buffer===buffer),
                "fixed="+__outcome(function(){return fixed.byteOffset})+":"+
                    __outcome(function(){return fixed.byteLength})+":"+
                    __outcome(function(){return fixed.getUint32(0)})+":"+
                    __outcome(function(){return fixed.setUint32(0,1)}),
                "tracking="+__outcome(function(){return tracking.byteOffset})+":"+
                    __outcome(function(){return tracking.byteLength})+":"+
                    __outcome(function(){return tracking.getUint8(0)}),
                "brand="+ArrayBuffer.isView(fixed)+":"+
                    ArrayBuffer.isView(tracking)+":"+
                    Object.prototype.toString.call(fixed),
                "wrong="+__outcome(function(){return bufferGetter.call({})})+":"+
                    __outcome(function(){
                        return DataView.prototype.getUint8.call({},0);
                    })
            ].join("|");
        })()"#,
        expected: concat!(
            "source=0:16:true:true|moved=8:16:true:false|edges=true:true|",
            "fixed=throw:TypeError:throw:TypeError:throw:TypeError:throw:TypeError|",
            "tracking=throw:TypeError:throw:TypeError:throw:TypeError|",
            "brand=true:true:[object DataView]|wrong=throw:TypeError:throw:TypeError",
        ),
    },
    Case {
        description: "constructor observes buffer and coercions before newTarget then revalidates",
        source: r#"(function(){
            var result=[];
            (function(){
                var log="",buffer=new ArrayBuffer(8,{maxByteLength:16});
                var custom=Object.create(DataView.prototype);
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    get:function(){log+="p";return custom}
                });
                var offset={valueOf:function(){log+="o";return 2}};
                var length={valueOf:function(){log+="l";return 3}};
                var view=Reflect.construct(
                    DataView,[buffer,offset,length],NewTarget);
                result.push("success="+log+":"+
                    (Object.getPrototypeOf(view)===custom)+":"+
                    view.byteOffset+":"+view.byteLength);
            })();
            (function(){
                var log="",offset={valueOf:function(){log+="o";return 0}};
                result.push("brand="+
                    __outcome(function(){return new DataView({},offset)})+
                    ":"+log);
            })();
            (function(){
                var log="",buffer=new ArrayBuffer(4);
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    get:function(){log+="p";return DataView.prototype}
                });
                var offset={valueOf:function(){
                    log+="o";throw new RangeError("offset");
                }};
                var length={valueOf:function(){log+="l";return 1}};
                result.push("offset-throw="+__outcome(function(){
                    return Reflect.construct(
                        DataView,[buffer,offset,length],NewTarget);
                })+":"+log);
            })();
            (function(){
                var log="",buffer=new ArrayBuffer(4);
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    get:function(){log+="p";return DataView.prototype}
                });
                var offset={valueOf:function(){
                    log+="o";buffer.transfer();return 0;
                }};
                var length={valueOf:function(){log+="l";return 1}};
                result.push("offset-detach="+__outcome(function(){
                    return Reflect.construct(
                        DataView,[buffer,offset,length],NewTarget);
                })+":"+log);
            })();
            (function(){
                var log="",buffer=new ArrayBuffer(4);
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    get:function(){log+="p";return DataView.prototype}
                });
                var length={valueOf:function(){
                    log+="l";buffer.transfer();return 1;
                }};
                result.push("length-detach="+__outcome(function(){
                    return Reflect.construct(
                        DataView,[buffer,0,length],NewTarget);
                })+":"+log);
            })();
            (function(){
                var log="",buffer=new ArrayBuffer(8,{maxByteLength:8});
                var NewTarget=(function(){}).bind(null);
                Object.defineProperty(NewTarget,"prototype",{
                    get:function(){
                        log+="p";buffer.resize(1);return DataView.prototype;
                    }
                });
                var offset={valueOf:function(){log+="o";return 2}};
                var length={valueOf:function(){log+="l";return 3}};
                result.push("prototype-shrink="+__outcome(function(){
                    return Reflect.construct(
                        DataView,[buffer,offset,length],NewTarget);
                })+":"+log);
            })();
            (function(){
                var rab=new ArrayBuffer(8,{maxByteLength:16});
                var fixed=new ArrayBuffer(8);
                result.push("omitted="+
                    new DataView(rab,2).byteLength+":"+
                    new DataView(rab,2,undefined).byteLength+":"+
                    new DataView(fixed,2).byteLength);
            })();
            result.push("call="+
                __outcome(function(){return DataView(new ArrayBuffer(0))}));
            result.push("ranges="+
                __outcome(function(){
                    return new DataView(new ArrayBuffer(4),-1);
                })+":"+
                __outcome(function(){
                    return new DataView(new ArrayBuffer(4),2,3);
                }));
            return result.join("|");
        })()"#,
        expected: concat!(
            "success=olp:true:2:3|brand=throw:TypeError:|",
            "offset-throw=throw:RangeError:o|offset-detach=throw:TypeError:o|",
            "length-detach=throw:TypeError:lp|",
            "prototype-shrink=throw:RangeError:olp|",
            "omitted=6:6:6|call=throw:TypeError|",
            "ranges=throw:RangeError:throw:RangeError",
        ),
    },
];

#[test]
fn data_view_vectors_match_frozen_observations() {
    for case in CASES {
        assert_eq!(
            oxide_observation(case),
            case.expected,
            "{}",
            case.description,
        );
    }
}

#[test]
fn data_view_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP DataView oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let observations = CASES
        .iter()
        .map(|case| oracle_observation(&oracle, case))
        .collect::<Vec<_>>();
    let expected = CASES.iter().map(|case| case.expected).collect::<Vec<_>>();
    assert_eq!(observations, expected);
}

#[test]
fn data_view_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP DataView differential: set QJS_ORACLE to pinned upstream qjs");
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
            panic!("Oxide threw for {}: {exception:?}", case.description);
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
