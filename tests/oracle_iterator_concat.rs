use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

struct Case {
    group: &'static str,
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// Differential lock for `Iterator.concat` in pinned QuickJS 2026-06-04.
// Every observer catches its own JavaScript failures and returns ASCII-only
// text, so host formatting and exception transport cannot hide a semantic
// mismatch.
const CASES: &[Case] = &[
    Case {
        group: "intrinsic graph",
        description: "static metadata hidden prototype descriptors tag and exact brands",
        source: r#"(function(){
            try {
                function bit(value){return value?"1":"0"}
                function bits(descriptor){
                    return bit(descriptor.writable)+bit(descriptor.enumerable)+
                        bit(descriptor.configurable);
                }
                function keyName(key){
                    if(key===Symbol.toStringTag)return "@@toStringTag";
                    return String(key);
                }
                function keys(object){
                    return Reflect.ownKeys(object).map(keyName).join(",");
                }
                function errorName(thunk){
                    try{thunk();return "none"}
                    catch(error){
                        return error&&error.name?error.name:String(error);
                    }
                }
                var empty=Iterator.concat();
                var prototype=Object.getPrototypeOf(empty);
                var helperPrototype=Object.getPrototypeOf(
                    [1].values().map(function(value){return value}));
                var wrapPrototype=Object.getPrototypeOf(
                    Iterator.from({next:function(){return {done:true}}}));
                var nextDescriptor=
                    Object.getOwnPropertyDescriptor(prototype,"next");
                var returnDescriptor=
                    Object.getOwnPropertyDescriptor(prototype,"return");
                var tagDescriptor=
                    Object.getOwnPropertyDescriptor(prototype,Symbol.toStringTag);
                var concatDescriptor=
                    Object.getOwnPropertyDescriptor(Iterator,"concat");
                return "static="+keys(Iterator)+
                    "|concat="+Iterator.concat.name+":"+Iterator.concat.length+":"+
                        bits(concatDescriptor)+":"+
                        errorName(function(){new Iterator.concat()})+
                    "|proto="+
                        (Object.getPrototypeOf(prototype)===Iterator.prototype)+":"+
                        (prototype!==Iterator.prototype)+":"+
                        (prototype!==helperPrototype)+":"+
                        (prototype!==wrapPrototype)+":"+keys(prototype)+
                    "|methods="+
                        prototype.next.name+":"+prototype.next.length+":"+
                            bits(nextDescriptor)+","+
                        prototype.return.name+":"+prototype.return.length+":"+
                            bits(returnDescriptor)+
                    "|tag="+tagDescriptor.value+":"+bits(tagDescriptor)+":"+
                        Object.prototype.toString.call(empty)+
                    "|brand="+
                        errorName(function(){prototype.next.call({})})+":"+
                        errorName(function(){prototype.return.call({})});
            } catch(error) {
                return "case-error:"+
                    (error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "static=length,name,concat,from,prototype|",
            "concat=concat:0:101:TypeError|",
            "proto=true:true:true:true:next,return,@@toStringTag|",
            "methods=next:0:101,return:0:101|",
            "tag=Iterator Concat:001:[object Iterator Concat]|",
            "brand=TypeError:TypeError",
        ),
    },
    Case {
        group: "lazy sequencing",
        description: "iterator getters are eager while opening and cached next calls are lazy",
        source: r#"(function(){
            try {
                var log=[];
                function make(label,values){
                    var index=0,iterator;
                    var iterable={};
                    Object.defineProperty(iterable,Symbol.iterator,{
                        configurable:true,
                        get:function(){
                            log.push("getiter-"+label);
                            return function(){
                                log.push("open-"+label+":"+
                                    String(this===iterable)+":"+
                                    arguments.length);
                                iterator={};
                                Object.defineProperty(iterator,"next",{
                                    configurable:true,
                                    get:function(){
                                        log.push("getnext-"+label);
                                        return function(){
                                            log.push("next-"+label+":"+
                                                String(this===iterator)+":"+
                                                arguments.length);
                                            if(index===values.length){
                                                return {
                                                    done:true,
                                                    get value(){
                                                        log.push("badvalue-"+label);
                                                        return 0;
                                                    }
                                                };
                                            }
                                            return {
                                                done:false,
                                                value:values[index++]
                                            };
                                        };
                                    }
                                });
                                return iterator;
                            };
                        }
                    });
                    return iterable;
                }
                var sequence=Iterator.concat(
                    make("A",[10]),make("B",[20]));
                var eager=log.join(",");
                log.push("made");
                var first=sequence.next();
                var second=sequence.next();
                var done=sequence.next();
                return "eager="+eager+
                    "|values="+first.value+":"+first.done+","+
                        second.value+":"+second.done+","+
                        String(done.value)+":"+done.done+
                    "|log="+log.join(",");
            } catch(error) {
                return "case-error:"+
                    (error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "eager=getiter-A,getiter-B|",
            "values=10:false,20:false,undefined:true|",
            "log=getiter-A,getiter-B,made,",
            "open-A:true:0,getnext-A,next-A:true:0,next-A:true:0,",
            "open-B:true:0,getnext-B,next-B:true:0,next-B:true:0",
        ),
    },
    Case {
        group: "captured methods",
        description: "static this capture cache sentinel and primitive-open retry boundaries",
        source: r#"(function(){
            try {
                function errorName(thunk){
                    try{thunk();return "none"}
                    catch(error){return error&&error.name?error.name:String(error)}
                }
                var thisReads=0,poisonThis={};
                Object.defineProperty(poisonThis,Symbol.iterator,{
                    get:function(){thisReads++;throw "this-read"}
                });
                var oldCalls=0,newCalls=0;
                var capturedIterable={
                    [Symbol.iterator]:function(){
                        oldCalls++;
                        return {next:function(){return {done:false,value:21}}};
                    }
                };
                var captured=
                    Iterator.concat.call(poisonThis,capturedIterable);
                capturedIterable[Symbol.iterator]=
                    function(){newCalls++;throw "new-open"};
                var capturedStep=captured.next();
                var noncallableGets=0,noncallableIterator={};
                Object.defineProperty(noncallableIterator,"next",{
                    get:function(){noncallableGets++;return 1}
                });
                var noncallable=Iterator.concat({
                    [Symbol.iterator]:function(){return noncallableIterator}
                });
                var noncallableFirst=
                    errorName(function(){noncallable.next()});
                var noncallableSecond=
                    errorName(function(){noncallable.next()});
                var undefinedGets=0,undefinedIterator={};
                Object.defineProperty(undefinedIterator,"next",{
                    get:function(){
                        undefinedGets++;
                        if(undefinedGets===1)return undefined;
                        return function(){return {done:false,value:22}};
                    }
                });
                var undefinedSequence=Iterator.concat({
                    [Symbol.iterator]:function(){return undefinedIterator}
                });
                var undefinedFirst=
                    errorName(function(){undefinedSequence.next()});
                var undefinedStep=undefinedSequence.next();
                var openCalls=0;
                var primitiveOpen=function(){
                    openCalls++;
                    if(openCalls===1)return 1;
                    return {next:function(){return {done:false,value:23}}};
                };
                var primitiveIterable={[Symbol.iterator]:primitiveOpen};
                var primitiveSequence=Iterator.concat(primitiveIterable);
                primitiveIterable[Symbol.iterator]=
                    function(){throw "new-primitive-open"};
                var primitiveFirst=
                    errorName(function(){primitiveSequence.next()});
                var primitiveStep=primitiveSequence.next();
                return "this="+thisReads+
                    "|captured="+capturedStep.value+":"+
                        capturedStep.done+":"+oldCalls+":"+newCalls+
                    "|noncallable="+noncallableFirst+":"+
                        noncallableSecond+":"+noncallableGets+
                    "|undefined="+undefinedFirst+":"+
                        undefinedStep.value+":"+undefinedStep.done+":"+
                        undefinedGets+
                    "|primitive="+primitiveFirst+":"+
                        primitiveStep.value+":"+primitiveStep.done+":"+
                        openCalls;
            } catch(error) {
                return "case-error:"+
                    (error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "this=0|captured=21:false:1:0|",
            "noncallable=TypeError:TypeError:1|",
            "undefined=TypeError:22:false:2|",
            "primitive=TypeError:23:false:2",
        ),
    },
    Case {
        group: "abrupt retry",
        description: "open next getter call done and value failures retry without closing",
        source: r#"(function(){
            try {
                var closeCount=0,parts=[];
                function close(){
                    closeCount++;
                    return {};
                }
                function attempt(sequence){
                    try {
                        var result=sequence.next();
                        return "return:"+String(result.value)+":"+result.done;
                    } catch(error) {
                        return "throw:"+String(error);
                    }
                }
                function run(label,iterable,log){
                    var sequence=Iterator.concat(iterable);
                    parts.push(label+"="+attempt(sequence)+","+
                        attempt(sequence)+":"+log.join(","));
                }

                var openLog=[],openCount=0;
                run("open",{
                    [Symbol.iterator]:function(){
                        openLog.push("open"+(++openCount));
                        if(openCount===1)throw "open-error";
                        return {
                            next:function(){
                                openLog.push("next");
                                return {done:false,value:11};
                            },
                            return:close
                        };
                    }
                },openLog);

                var getLog=[],getCount=0,getIterator={return:close};
                Object.defineProperty(getIterator,"next",{
                    get:function(){
                        getLog.push("get"+(++getCount));
                        if(getCount===1)throw "get-error";
                        return function(){
                            getLog.push("next");
                            return {done:false,value:12};
                        };
                    }
                });
                run("get",{
                    [Symbol.iterator]:function(){return getIterator}
                },getLog);

                var callLog=[],callCount=0,callIterator={return:close};
                Object.defineProperty(callIterator,"next",{
                    get:function(){
                        callLog.push("get");
                        return function(){
                            callLog.push("next"+(++callCount));
                            if(callCount===1)throw "call-error";
                            return {done:false,value:13};
                        };
                    }
                });
                run("call",{
                    [Symbol.iterator]:function(){return callIterator}
                },callLog);

                var doneLog=[],doneCount=0;
                var doneIterator={
                    next:function(){
                        var current=++doneCount;
                        doneLog.push("next"+current);
                        return {
                            get done(){
                                doneLog.push("done"+current);
                                if(current===1)throw "done-error";
                                return false;
                            },
                            value:14
                        };
                    },
                    return:close
                };
                run("done",{
                    [Symbol.iterator]:function(){return doneIterator}
                },doneLog);

                var valueLog=[],valueCount=0;
                var valueIterator={
                    next:function(){
                        var current=++valueCount;
                        valueLog.push("next"+current);
                        return {
                            get done(){
                                valueLog.push("done"+current);
                                return false;
                            },
                            get value(){
                                valueLog.push("value"+current);
                                if(current===1)throw "value-error";
                                return 15;
                            }
                        };
                    },
                    return:close
                };
                run("value",{
                    [Symbol.iterator]:function(){return valueIterator}
                },valueLog);
                return parts.join("|")+"|closes="+closeCount;
            } catch(error) {
                return "case-error:"+
                    (error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "open=throw:open-error,return:11:false:open1,open2,next|",
            "get=throw:get-error,return:12:false:get1,get2,next|",
            "call=throw:call-error,return:13:false:get,next1,next2|",
            "done=throw:done-error,return:14:false:next1,done1,next2,done2|",
            "value=throw:value-error,return:15:false:",
            "next1,done1,value1,next2,done2,value2|closes=0",
        ),
    },
    Case {
        group: "return",
        description: "before-start preserve drain validation and passthrough semantics",
        source: r#"(function(){
            try {
                function resultText(result){
                    return String(result.value)+":"+result.done;
                }
                function completion(thunk){
                    try{return "return:"+String(thunk())}
                    catch(error){
                        return "throw:"+
                            (error&&error.name?error.name:String(error));
                    }
                }

                var beforeOpens=0;
                var before=Iterator.concat({
                    [Symbol.iterator]:function(){
                        beforeOpens++;
                        return {
                            next:function(){
                                return {done:false,value:1};
                            }
                        };
                    }
                });
                var beforeReturn=before.return("ignored");
                var beforeNext=before.next();

                var getterCount=0,getterIndex=0;
                var getterReceiver=false,getterArgc=-1,marker={};
                var getterIterator={
                    next:function(){
                        return {done:false,value:++getterIndex};
                    }
                };
                Object.defineProperty(getterIterator,"return",{
                    get:function(){
                        getterCount++;
                        if(getterCount===1)throw "return-get";
                        return function(){
                            getterReceiver=this===getterIterator;
                            getterArgc=arguments.length;
                            return marker;
                        };
                    }
                });
                var preserve=Iterator.concat({
                    [Symbol.iterator]:function(){return getterIterator}
                });
                var preserveFirst=preserve.next();
                var preserveError=completion(function(){
                    return preserve.return();
                });
                var preserveSecond=preserve.next();
                var preserveResult=preserve.return("ignored");
                var preserveDone=preserve.next();

                var drainSecondOpens=0;
                var drainIterator={
                    next:function(){return {done:false,value:7}},
                    return:function(){throw "return-call"}
                };
                var drain=Iterator.concat(
                    {
                        [Symbol.iterator]:function(){
                            return drainIterator;
                        }
                    },
                    {
                        [Symbol.iterator]:function(){
                            drainSecondOpens++;
                            return {
                                next:function(){return {done:true}}
                            };
                        }
                    }
                );
                drain.next();
                var drainError=completion(function(){
                    return drain.return();
                });
                var drainNext=drain.next();
                var drainAgain=drain.return();

                function invalid(value,present){
                    var iterator={
                        next:function(){
                            return {done:false,value:8};
                        }
                    };
                    if(present)iterator.return=value;
                    var sequence=Iterator.concat({
                        [Symbol.iterator]:function(){return iterator}
                    });
                    sequence.next();
                    var error=completion(function(){
                        return sequence.return();
                    });
                    return error+":"+resultText(sequence.next());
                }

                var primitiveIterator={
                    next:function(){return {done:false,value:9}},
                    return:function(){return 42}
                };
                var primitive=Iterator.concat({
                    [Symbol.iterator]:function(){
                        return primitiveIterator;
                    }
                });
                primitive.next();
                var primitiveResult=primitive.return("ignored");

                return "before="+String(beforeReturn)+":"+beforeOpens+":"+
                        resultText(beforeNext)+
                    "|getter="+resultText(preserveFirst)+":"+
                        preserveError+":"+resultText(preserveSecond)+":"+
                        (preserveResult===marker)+":"+getterReceiver+":"+
                        getterArgc+":"+resultText(preserveDone)+
                    "|drain="+drainError+":"+resultText(drainNext)+":"+
                        String(drainAgain)+":"+drainSecondOpens+
                    "|missing="+invalid(undefined,false)+
                    "|noncallable="+invalid(1,true)+
                    "|primitive="+typeof primitiveResult+":"+
                        primitiveResult+":"+resultText(primitive.next());
            } catch(error) {
                return "case-error:"+
                    (error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "before=undefined:0:undefined:true|",
            "getter=1:false:throw:return-get:2:false:true:true:0:undefined:true|",
            "drain=throw:return-call:undefined:true:undefined:0|",
            "missing=throw:TypeError:undefined:true|",
            "noncallable=throw:TypeError:undefined:true|",
            "primitive=number:42:undefined:true",
        ),
    },
    Case {
        group: "running guard",
        description: "all next and return reentry directions throw while outer work survives",
        source: r#"(function(){
            try {
                var log=[];
                function catchName(label,thunk){
                    try {
                        thunk();
                        log.push(label+":none");
                    } catch(error) {
                        log.push(label+":"+
                            (error&&error.name?error.name:String(error)));
                    }
                }
                function result(value){
                    return value.value+":"+value.done;
                }

                var nextNext;
                var nextNextIterable={
                    [Symbol.iterator]:function(){
                        catchName("nn",function(){nextNext.next()});
                        return {
                            next:function(){
                                return {done:false,value:1};
                            }
                        };
                    }
                };
                nextNext=Iterator.concat(nextNextIterable);
                var nextNextResult=nextNext.next();

                var nextReturn;
                var nextReturnIterator={
                    next:function(){
                        catchName("nr",function(){
                            nextReturn.return();
                        });
                        return {done:false,value:2};
                    }
                };
                nextReturn=Iterator.concat({
                    [Symbol.iterator]:function(){
                        return nextReturnIterator;
                    }
                });
                var nextReturnResult=nextReturn.next();

                var returnNext;
                var returnNextIterator={
                    next:function(){return {done:false,value:3}}
                };
                Object.defineProperty(returnNextIterator,"return",{
                    get:function(){
                        catchName("rn",function(){
                            returnNext.next();
                        });
                        return function(){return 30};
                    }
                });
                returnNext=Iterator.concat({
                    [Symbol.iterator]:function(){
                        return returnNextIterator;
                    }
                });
                returnNext.next();
                var returnNextResult=returnNext.return();

                var returnReturn;
                var returnReturnIterator={
                    next:function(){return {done:false,value:4}},
                    return:function(){
                        catchName("rr",function(){
                            returnReturn.return();
                        });
                        return 40;
                    }
                };
                returnReturn=Iterator.concat({
                    [Symbol.iterator]:function(){
                        return returnReturnIterator;
                    }
                });
                returnReturn.next();
                var returnReturnResult=returnReturn.return();

                return "results="+result(nextNextResult)+","+
                        result(nextReturnResult)+","+
                        returnNextResult+","+returnReturnResult+
                    "|log="+log.join(",");
            } catch(error) {
                return "case-error:"+
                    (error&&error.name?error.name:String(error));
            }
        })()"#,
        expected: concat!(
            "results=1:false,2:false,30,40|",
            "log=nn:TypeError,nr:TypeError,rn:TypeError,rr:TypeError",
        ),
    },
    Case {
        group: "raw IteratorNext stack",
        description: "the direct native inner step is absent from the visible backtrace",
        source: r#"(function(){
            try {
                Iterator.concat({
                    [Symbol.iterator]:function(){
                        return {
                            next:Object.getPrototypeOf(
                                [][Symbol.iterator]()).next
                        };
                    }
                }).next();
                return "no-error";
            } catch(error) {
                var stack=String(error.stack);
                var needle="    at next (native)";
                var count=0,index=0;
                while((index=stack.indexOf(needle,index))!==-1){
                    count++;
                    index+=needle.length;
                }
                return "TypeError="+
                    String(error&&error.name==="TypeError")+
                    "|native-next="+count;
            }
        })()"#,
        expected: "TypeError=true|native-next=1",
    },
];

#[test]
fn iterator_concat_matches_pinned_expectations() {
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
        "Iterator.concat pinned expectations failed in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn iterator_concat_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Iterator.concat oracle self-check: \
             set QJS_ORACLE to pinned upstream qjs"
        );
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
        "pinned QuickJS Iterator.concat vectors drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn iterator_concat_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Iterator.concat differential: \
             set QJS_ORACLE to pinned upstream qjs"
        );
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
        "Iterator.concat semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

#[test]
fn iterator_concat_cross_realm_graph_and_native_next_use_the_current_realm() {
    // The qjs CLI cannot bridge objects between two contexts. This directly
    // pins the corresponding QuickJS C path: the concat iterator and public
    // iterator-result wrappers use the concat intrinsic's defining context,
    // while JS_IteratorNext2 invokes an inner IteratorNext cproto with that
    // current context rather than switching to the inner method's realm. A
    // same-runtime, two-context probe against the pinned libquickjs reports:
    // `sequence-prototype=1:0|step-prototype=1|value-identity=1|native-error=1:0`.
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let iterator = eval_object(&mut defining, "Iterator", "defining Iterator");
    let concat = property_callable(&runtime, &mut defining, &iterator, "concat");
    let defining_concat_prototype = eval_object(
        &mut defining,
        "Object.getPrototypeOf(Iterator.concat())",
        "defining concat prototype",
    );
    let caller_concat_prototype = eval_object(
        &mut caller,
        "Object.getPrototypeOf(Iterator.concat())",
        "caller concat prototype",
    );
    let defining_object_prototype = defining.object_prototype().unwrap();
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

    caller
        .eval("globalThis.__qjoConcatMarker=Object()")
        .expect("install caller concat marker");
    let marker = eval_object(
        &mut caller,
        "globalThis.__qjoConcatMarker",
        "caller concat marker",
    );
    let iterable = eval_object(
        &mut caller,
        "[globalThis.__qjoConcatMarker]",
        "caller iterable",
    );
    let Value::Object(sequence) = caller
        .call(&concat, Value::Undefined, &[Value::Object(iterable)])
        .expect("cross-realm Iterator.concat call")
    else {
        panic!("cross-realm Iterator.concat did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&sequence).unwrap(),
        Some(defining_concat_prototype),
        "Iterator.concat used the caller realm's hidden concat prototype",
    );
    assert_ne!(
        runtime.get_prototype_of(&sequence).unwrap(),
        Some(caller_concat_prototype),
    );

    let next = property_callable(&runtime, &mut caller, &sequence, "next");
    let Value::Object(step) = caller
        .call(&next, Value::Object(sequence), &[])
        .expect("cross-realm concat next")
    else {
        panic!("cross-realm concat next did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&step).unwrap(),
        Some(defining_object_prototype),
        "concat next did not allocate its public result in the defining operation realm",
    );
    assert_eq!(
        object_property(&runtime, &mut caller, &step, "value"),
        Value::Object(marker),
        "concat next did not preserve the caller-realm yielded object",
    );
    assert_eq!(
        object_property(&runtime, &mut caller, &step, "done"),
        Value::Bool(false),
    );

    // This iterable returns an ordinary object whose `next` is the caller
    // realm's native Array Iterator next method. The receiver fails that
    // method's brand. A generic native call would create caller TypeError;
    // the raw IteratorNext path must create defining TypeError instead.
    let invalid_iterable = eval_object(
        &mut caller,
        r#"(function(){
            var nativeNext=
                Object.getPrototypeOf([][Symbol.iterator]()).next;
            return {
                [Symbol.iterator]:function(){
                    return {next:nativeNext};
                }
            };
        })()"#,
        "caller iterable with wrong-brand native next",
    );
    let Value::Object(invalid_sequence) = caller
        .call(
            &concat,
            Value::Undefined,
            &[Value::Object(invalid_iterable)],
        )
        .expect("construct wrong-brand cross-realm concat")
    else {
        panic!("wrong-brand cross-realm concat was not an object");
    };
    let invalid_next = property_callable(&runtime, &mut caller, &invalid_sequence, "next");
    assert!(matches!(
        caller.call(&invalid_next, Value::Object(invalid_sequence), &[],),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(
        &mut caller,
        "cross-realm concat native IteratorNext TypeError",
    );
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_type_error),
        "concat's raw native IteratorNext path switched to the inner method realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
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
  throw new TypeError("Iterator.concat observer returned a non-String");
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

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> Value {
    let key = runtime.intern_property_key(name).unwrap();
    context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read {name}: {error}"))
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
