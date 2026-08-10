use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Differential lock for the synchronous Iterator helpers surface in pinned
// QuickJS 2026-06-04. Every case catches its own JavaScript failures and
// returns an ASCII-only observation, so host-side differences cannot mask a
// semantic mismatch. Iterator.concat has its own deeper differential suite,
// but its static constructor slot remains part of this shared intrinsic graph.
const CASES: &[Case] = &[
    Case {
        group: "intrinsic graph",
        description: "constructor prototype metadata and descriptor order",
        source: r#"(function(){
            try {
                function bit(value){return value?"1":"0"}
                function bits(descriptor){
                    return bit(descriptor.writable)+bit(descriptor.enumerable)+
                        bit(descriptor.configurable);
                }
                function keyName(key){
                    if(key===Symbol.iterator)return "@@iterator";
                    if(key===Symbol.toStringTag)return "@@toStringTag";
                    return String(key);
                }
                function keys(object){
                    var own=Reflect.ownKeys(object),result=[],index,key;
                    for(index=0;index<own.length;index++){
                        key=own[index];
                        result.push(keyName(key));
                    }
                    return result.join(",");
                }
                var names=["drop","filter","flatMap","map","take","every","find",
                    "forEach","some","reduce","toArray"];
                var methods=[],index,name,method,descriptor;
                for(index=0;index<names.length;index++){
                    name=names[index];
                    method=Iterator.prototype[name];
                    descriptor=Object.getOwnPropertyDescriptor(Iterator.prototype,name);
                    methods.push(name+":"+method.name+":"+method.length+":"+bits(descriptor));
                }
                var globalDescriptor=Object.getOwnPropertyDescriptor(globalThis,"Iterator");
                var constructorDescriptor=
                    Object.getOwnPropertyDescriptor(Iterator.prototype,"constructor");
                var tagDescriptor=
                    Object.getOwnPropertyDescriptor(Iterator.prototype,Symbol.toStringTag);
                return [
                    "global="+bits(globalDescriptor)+":"+(globalDescriptor.value===Iterator),
                    "ctor="+typeof Iterator+":"+Iterator.name+":"+Iterator.length,
                    "links="+(Object.getPrototypeOf(Iterator)===Function.prototype)+":"+
                        (Object.getPrototypeOf(Iterator.prototype)===Object.prototype),
                    "static="+keys(Iterator),
                    "prototype="+bits(Object.getOwnPropertyDescriptor(Iterator,"prototype"))+
                        ":"+keys(Iterator.prototype),
                    "methods="+methods.join(";"),
                    "constructor-accessor="+bit(!("value" in constructorDescriptor))+
                        bit(constructorDescriptor.enumerable)+
                        bit(constructorDescriptor.configurable)+":"+
                        (constructorDescriptor.get===constructorDescriptor.set)+":"+
                        constructorDescriptor.get.name.length+":"+
                        constructorDescriptor.get.length+":"+
                        constructorDescriptor.set.name.length+":"+
                        constructorDescriptor.set.length,
                    "iterator="+
                        (Iterator.prototype[Symbol.iterator].call(Iterator.prototype)===
                            Iterator.prototype)+":"+
                        Iterator.prototype[Symbol.iterator].length+":"+
                        bits(Object.getOwnPropertyDescriptor(
                            Iterator.prototype,Symbol.iterator)),
                    "tag="+bit(!("value" in tagDescriptor))+
                        bit(tagDescriptor.enumerable)+bit(tagDescriptor.configurable)+":"+
                        (Iterator.prototype[Symbol.toStringTag]==="Iterator")+":"+
                        (Object.prototype.toString.call(Iterator.prototype)==="[object Iterator]")
                ].join("|");
            } catch(error) {
                return "case-error:"+(error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "global=101:true|ctor=function:Iterator:0|links=true:true|",
            "static=length,name,concat,from,prototype|",
            "prototype=000:drop,filter,flatMap,map,take,every,find,forEach,some,",
            "reduce,toArray,constructor,@@iterator,@@toStringTag|",
            "methods=drop:drop:1:101;filter:filter:1:101;flatMap:flatMap:1:101;",
            "map:map:1:101;take:take:1:101;every:every:1:101;find:find:1:101;",
            "forEach:forEach:1:101;some:some:1:101;reduce:reduce:1:101;",
            "toArray:toArray:0:101|constructor-accessor=101:true:0:0:0:0|",
            "iterator=true:0:101|tag=101:true:true",
        ),
    },
    Case {
        group: "basic helpers",
        description: "lazy transforms and eager consumers compose",
        source: r#"(function(){
            try {
                var lazy=[1,2,3].values()
                    .map(function(value,index){return value*10+index})
                    .filter(function(value){return value>10})
                    .flatMap(function(value){return [value,value+1].values()})
                    .toArray();
                var dropped=[1,2,3].values().drop(1).toArray();
                var taken=[1,2,3].values().take(2).toArray();
                var every=[2,4,6].values().every(function(value){return value%2===0});
                var find=[1,4,6].values().find(function(value){return value%2===0});
                var some=[1,3,6].values().some(function(value){return value%2===0});
                var indices=[],sum=0;
                var forEachResult=[1,2,3].values().forEach(function(value,index){
                    sum+=value;
                    indices.push(index);
                });
                var reduced=[1,2,3].values().reduce(function(left,right){
                    return left+right;
                });
                var reducedInitial=[1,2,3].values().reduce(function(left,right){
                    return left+right;
                },10);
                return "lazy="+lazy.join(",")+
                    "|drop="+dropped.join(",")+
                    "|take="+taken.join(",")+
                    "|every="+every+
                    "|find="+find+
                    "|some="+some+
                    "|forEach="+String(forEachResult)+":"+sum+":"+indices.join(",")+
                    "|reduce="+reduced+":"+reducedInitial+
                    "|toArray="+[5,6].values().toArray().join(",");
            } catch(error) {
                return "case-error:"+(error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "lazy=21,22,32,33|drop=2,3|take=1,2|every=true|find=4|some=true|",
            "forEach=undefined:6:0,1,2|reduce=6:16|toArray=5,6",
        ),
    },
    Case {
        group: "Iterator.from",
        description: "wrapper return lookup is dynamic and preserves result identity",
        source: r#"(function(){
            try {
                var marker={done:true,value:42},argc=-1,receiver=false,touched=0;
                var source={
                    next:function(){return {done:false,value:0}},
                    return:function(){
                        argc=arguments.length;
                        receiver=this===source;
                        return marker;
                    }
                };
                var wrapped=Iterator.from(source);
                var first=wrapped.return("ignored");
                source.return=function(){throw "boom"};
                var dynamicError="missing";
                try{wrapped.return()}catch(error){dynamicError=String(error)}
                source.return=null;
                var absent=wrapped.return("ignored");
                Object.defineProperty(source,"return",{
                    configurable:true,
                    get:function(){touched++;return null}
                });
                var returnMethod=Object.getPrototypeOf(wrapped).return;
                var brandError="missing";
                try{returnMethod.call(source)}
                catch(error){brandError=error&&error.name?error.name:String(error)}
                return "first="+(first===marker)+":"+argc+":"+receiver+":"+first.done+
                    "|throw="+dynamicError+
                    "|absent="+absent.done+":"+(absent.value===undefined)+
                    "|brand="+brandError+":"+touched;
            } catch(error) {
                return "case-error:"+(error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: "first=true:0:true:true|throw=boom|absent=true:true|brand=TypeError:0",
    },
    Case {
        group: "numeric limits",
        description: "huge finite helper limits use QuickJS signed low bits",
        source: r#"(function(){
            try {
                function errorName(thunk){
                    try{thunk();return "none"}
                    catch(error){return error&&error.name?error.name:String(error)}
                }
                var rangeTake=errorName(function(){[1].values().take(2**63)});
                var rangeDrop=errorName(function(){[1].values().drop(2**63)});
                var dropE100=[7].values().drop(1e100).next();
                var takeE100=[7].values().take(1e100).next();
                var dropP64=[8].values().drop(2**64).next();
                var takeP64=[8].values().take(2**64).next();
                return "range="+rangeTake+","+rangeDrop+
                    "|e100="+String(dropE100.value)+":"+dropE100.done+","+
                        String(takeE100.value)+":"+takeE100.done+
                    "|p64="+String(dropP64.value)+":"+dropP64.done+","+
                        String(takeP64.value)+":"+takeP64.done;
            } catch(error) {
                return "case-error:"+(error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "range=RangeError,RangeError|e100=7:false,undefined:true|",
            "p64=8:false,undefined:true",
        ),
    },
    Case {
        group: "flatMap close",
        description: "inner normal-close errors replace step errors before outer close",
        source: r#"(function(){
            try {
                var firstLog=[];
                var firstOuter={
                    next:function(){return {done:false,value:0}},
                    return:function(){firstLog.push("outer-return");return {}}
                };
                var firstInner={
                    next:function(){firstLog.push("inner-next");throw "inner-next"},
                    return:function(){firstLog.push("inner-return");throw "inner-return"}
                };
                var firstHelper=Iterator.prototype.flatMap.call(
                    firstOuter,function(){return firstInner});
                var firstError="missing";
                try{firstHelper.next()}catch(error){firstError=String(error)}

                var secondLog=[],returnGets=0;
                var secondOuter={
                    next:function(){return {done:false,value:0}},
                    return:function(){secondLog.push("outer-return");return {}}
                };
                var secondInner={
                    next:function(){return {done:false,value:1}}
                };
                Object.defineProperty(secondInner,"return",{
                    configurable:true,
                    get:function(){
                        returnGets++;
                        secondLog.push("get-return-"+returnGets);
                        throw returnGets===1?"first-return-get":"second-return-get";
                    }
                });
                var secondHelper=Iterator.prototype.flatMap.call(
                    secondOuter,function(){return secondInner});
                secondHelper.next();
                var secondError="missing";
                try{secondHelper.return()}catch(error){secondError=String(error)}
                return firstError+":"+firstLog.join(",")+"|"+
                    secondError+":"+secondLog.join(",");
            } catch(error) {
                return "case-error:"+(error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "inner-return:inner-next,inner-return,outer-return|",
            "second-return-get:get-return-1,get-return-2,outer-return",
        ),
    },
    Case {
        group: "String fallback",
        description: "a primitive String remains the wrapped iterator and call receiver",
        source: r#"(function(){
            var iteratorDescriptor=
                Object.getOwnPropertyDescriptor(String.prototype,Symbol.iterator);
            var nextDescriptor=Object.getOwnPropertyDescriptor(String.prototype,"next");
            var returnDescriptor=Object.getOwnPropertyDescriptor(String.prototype,"return");
            try {
                String.prototype[Symbol.iterator]=null;
                var missing=Iterator.from("abc"),missingError="missing";
                try{missing.next()}
                catch(error){missingError=error&&error.name?error.name:String(error)}

                var nextThis,returnThis,returnArgc=-1;
                var marker={done:true,value:42};
                String.prototype.next=function(){
                    "use strict";
                    nextThis=this;
                    return {done:false,value:7};
                };
                String.prototype.return=function(){
                    "use strict";
                    returnThis=this;
                    returnArgc=arguments.length;
                    return marker;
                };
                delete String.prototype[Symbol.iterator];
                var wrapped=Iterator.from("abc");
                var next=wrapped.next();
                var returned=wrapped.return("ignored");
                return "missing="+missingError+
                    "|next="+next.value+":"+next.done+":"+typeof nextThis+":"+
                        (nextThis==="abc")+
                    "|return="+typeof returnThis+":"+(returnThis==="abc")+":"+
                        returnArgc+":"+(returned===marker);
            } catch(error) {
                return "case-error:"+(error&&error.name?error.name:String(error));
            } finally {
                if(iteratorDescriptor===undefined){
                    delete String.prototype[Symbol.iterator];
                }else{
                    Object.defineProperty(
                        String.prototype,Symbol.iterator,iteratorDescriptor);
                }
                if(nextDescriptor===undefined){
                    delete String.prototype.next;
                }else{
                    Object.defineProperty(String.prototype,"next",nextDescriptor);
                }
                if(returnDescriptor===undefined){
                    delete String.prototype.return;
                }else{
                    Object.defineProperty(String.prototype,"return",returnDescriptor);
                }
            }
        })()"#,
        expected: "missing=TypeError|next=7:false:string:true|return=string:true:0:true",
    },
];

#[test]
fn iterator_helpers_match_pinned_expectations() {
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let actual = observe_oxide(&mut context, case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "Iterator helpers pinned expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn iterator_helpers_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Iterator helpers oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES {
        let actual = observe_oracle(&oracle, case);
        if actual != case.expected {
            failures.push(format!(
                "{} / {}\nsource: {:?}\nactual: {:?}\nexpected: {:?}",
                case.group, case.description, case.source, actual, case.expected,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "pinned QuickJS Iterator helpers vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn iterator_helpers_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Iterator helpers differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for case in CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let oxide = observe_oxide(&mut context, case);
        let quickjs = observe_oracle(&oracle, case);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {}\nsource: {:?}\noxide: {:?}\nquickjs: {:?}",
                case.group, case.description, case.source, oxide, quickjs,
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "Iterator helpers semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn observe_oxide(context: &mut Context, case: &Case) -> String {
    let value = context
        .eval(case.source)
        .unwrap_or_else(|error| panic!("Oxide observer failed for {}: {error}", case.description));
    let Value::String(value) = value else {
        panic!(
            "Oxide observer returned a non-String for {}",
            case.description
        );
    };
    value.to_utf8_lossy()
}

fn observe_oracle(oracle: &OsStr, case: &Case) -> String {
    let wrapper = r#"
var value = std.evalScript(scriptArgs[0]);
if (typeof value !== "string")
  throw new TypeError("Iterator helper observer returned a non-String");
print(value);
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, case.source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {}: {error}", case.description));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {}: {}",
        case.description,
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| {
            panic!(
                "QuickJS output was not UTF-8 for {}: {error}",
                case.description
            )
        })
        .trim_end()
        .to_owned()
}
