use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    Context, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04's complete Proxy surface: all thirteen handler
// traps, null/undefined forwarding, revocation, callable/constructor caching,
// receiver/newTarget propagation, and the invariant checks in
// `js_proxy_*`. The `pinned quirks` group is deliberately not a statement of
// ideal ECMA-262 behavior. It records observable behavior in the selected
// QuickJS source, including its explicitly documented `proxy-missing-checks`.

const PRELUDE: &str = r#"
function __proxyValue(value) {
    if (value === undefined) return "undefined";
    if (value === null) return "null";
    if (typeof value === "symbol") return String(value);
    if (typeof value === "object" || typeof value === "function")
        return typeof value;
    if (typeof value === "number" && value === 0)
        return 1 / value < 0 ? "-0" : "+0";
    if (typeof value === "number" && value !== value) return "NaN";
    return String(value);
}
function __proxyCompletion(thunk) {
    try {
        return "return:" + __proxyValue(thunk());
    } catch (error) {
        if (error !== null &&
            (typeof error === "object" || typeof error === "function"))
            return "throw:" + error.name + ":" + error.message;
        return "throw:" + typeof error + ":" + String(error);
    }
}
function __proxyBits(descriptor) {
    if (descriptor === undefined) return "undefined";
    function bit(value) { return value ? 1 : 0; }
    var kind = Object.prototype.hasOwnProperty.call(descriptor, "value") ||
        Object.prototype.hasOwnProperty.call(descriptor, "writable") ? "d" : "a";
    var payload = kind === "d" ?
        __proxyValue(descriptor.value) + ":" + bit(descriptor.writable) :
        __proxyValue(descriptor.get) + ":" + __proxyValue(descriptor.set);
    return kind + ":" + payload + ":" + bit(descriptor.enumerable) +
        bit(descriptor.configurable);
}
function __proxyKey(key) {
    return typeof key === "symbol" ? String(key) : key;
}
function __proxyKeys(keys) {
    return keys.map(__proxyKey).join(",");
}
function __proxyIsConstructor(value) {
    try {
        Reflect.construct(function () {}, [], value);
        return true;
    } catch (_) {
        return false;
    }
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "Proxy constructor metadata global descriptor and method graph",
        r#"(function(){
            var globalDescriptor=Object.getOwnPropertyDescriptor(globalThis,"Proxy");
            var revocableDescriptor=Object.getOwnPropertyDescriptor(Proxy,"revocable");
            var nameDescriptor=Object.getOwnPropertyDescriptor(Proxy,"name");
            var lengthDescriptor=Object.getOwnPropertyDescriptor(Proxy,"length");
            return [
                typeof Proxy,Proxy.name,Proxy.length,__proxyIsConstructor(Proxy),
                __proxyCompletion(function(){return Proxy({}, {})}),
                Object.prototype.hasOwnProperty.call(Proxy,"prototype"),
                Object.getPrototypeOf(Proxy)===Function.prototype,
                __proxyKeys(Reflect.ownKeys(Proxy)),
                globalDescriptor.writable,globalDescriptor.enumerable,
                globalDescriptor.configurable,
                revocableDescriptor.value===Proxy.revocable,
                revocableDescriptor.writable,revocableDescriptor.enumerable,
                revocableDescriptor.configurable,
                nameDescriptor.writable,nameDescriptor.enumerable,nameDescriptor.configurable,
                lengthDescriptor.writable,lengthDescriptor.enumerable,lengthDescriptor.configurable
            ].join("|");
        })()"#,
    ),
    (
        "constructor requires object target and handler without coercion",
        r#"(function(){
            var log="",coercible={
                valueOf:function(){log+="v";return {}},
                toString:function(){log+="s";return "[object Object]"}
            };
            return [
                __proxyCompletion(function(){return new Proxy({}, {})}),
                __proxyCompletion(function(){return new Proxy(1, {})}),
                __proxyCompletion(function(){return new Proxy({}, null)}),
                __proxyCompletion(function(){return new Proxy(coercible, {})}),
                __proxyCompletion(function(){return new Proxy({}, coercible)}),
                log
            ].join("|");
        })()"#,
    ),
    (
        "revocable result and one-shot revoke function metadata",
        r#"(function(){
            var pair=Proxy.revocable({answer:42},{});
            var proxyDescriptor=Object.getOwnPropertyDescriptor(pair,"proxy");
            var revokeDescriptor=Object.getOwnPropertyDescriptor(pair,"revoke");
            return [
                Object.getPrototypeOf(pair)===Object.prototype,
                __proxyKeys(Reflect.ownKeys(pair)),
                pair.proxy===proxyDescriptor.value,pair.revoke===revokeDescriptor.value,
                proxyDescriptor.writable,proxyDescriptor.enumerable,proxyDescriptor.configurable,
                revokeDescriptor.writable,revokeDescriptor.enumerable,revokeDescriptor.configurable,
                typeof pair.revoke,pair.revoke.name,pair.revoke.length,
                __proxyIsConstructor(pair.revoke),
                Object.getPrototypeOf(pair.revoke)===Function.prototype,
                __proxyKeys(Object.getOwnPropertyNames(pair.revoke))
            ].join("|");
        })()"#,
    ),
];

const LIFECYCLE_CASES: &[(&str, &str)] = &[
    (
        "revocation is idempotent ignores this and arguments and severs the closure capture",
        r#"(function(){
            var pair=Proxy.revocable({answer:42},{}),proxy=pair.proxy,revoke=pair.revoke;
            var before=proxy.answer;
            var first=revoke.call({ignored:true},1,2);
            var second=revoke.call(null,3);
            return [
                before,first===undefined,second===undefined,
                __proxyCompletion(function(){return proxy.answer}),
                __proxyCompletion(function(){return revoke()}),
                typeof proxy,typeof revoke
            ].join("|");
        })()"#,
    ),
    (
        "every internal method rejects a revoked callable constructor proxy",
        r#"(function(){
            function Target(value){this.value=value;return value+1}
            var pair=Proxy.revocable(Target,{}),proxy=pair.proxy;
            pair.revoke();
            var operations=[
                function(){return Reflect.getPrototypeOf(proxy)},
                function(){return Reflect.setPrototypeOf(proxy,null)},
                function(){return Reflect.isExtensible(proxy)},
                function(){return Reflect.preventExtensions(proxy)},
                function(){return Reflect.has(proxy,"x")},
                function(){return Reflect.get(proxy,"x")},
                function(){return Reflect.set(proxy,"x",1)},
                function(){return Reflect.getOwnPropertyDescriptor(proxy,"x")},
                function(){return Reflect.defineProperty(proxy,"x",{value:1})},
                function(){return Reflect.deleteProperty(proxy,"x")},
                function(){return Reflect.ownKeys(proxy)},
                function(){return Reflect.apply(proxy,null,[1])},
                function(){return Reflect.construct(proxy,[1])}
            ];
            return operations.map(__proxyCompletion).join("|");
        })()"#,
    ),
    (
        "revoked operations do not read any handler trap getter",
        r#"(function(){
            var log="",handler={},names=[
                "getPrototypeOf","setPrototypeOf","isExtensible","preventExtensions",
                "has","get","set","getOwnPropertyDescriptor","defineProperty",
                "deleteProperty","ownKeys","apply","construct"
            ];
            names.forEach(function(name){
                Object.defineProperty(handler,name,{get:function(){log+=name+";";return undefined}});
            });
            var pair=Proxy.revocable(function(){},handler),proxy=pair.proxy;
            pair.revoke();
            var results=[
                __proxyCompletion(function(){return proxy.x}),
                __proxyCompletion(function(){return Reflect.ownKeys(proxy)}),
                __proxyCompletion(function(){return proxy()}),
                __proxyCompletion(function(){return new proxy()})
            ];
            return results.join("|")+"|log:"+log;
        })()"#,
    ),
    (
        "revocation preserves cached callable and constructable identity tests",
        r#"(function(){
            function Target(){}
            var pair=Proxy.revocable(Target,{}),proxy=pair.proxy;
            var before=[typeof proxy,__proxyIsConstructor(proxy)];
            pair.revoke();
            return before.join(":")+"|"+
                [typeof proxy,__proxyIsConstructor(proxy),
                 __proxyCompletion(function(){return proxy()}),
                 __proxyCompletion(function(){return new proxy()})].join(":");
        })()"#,
    ),
];

const FALLBACK_CASES: &[(&str, &str)] = &[
    (
        "null traps forward all eleven object internal methods",
        r#"(function(){
            var target={x:1},proto={inherited:7},handler={
                getPrototypeOf:null,setPrototypeOf:null,isExtensible:null,
                preventExtensions:null,has:null,get:null,set:null,
                getOwnPropertyDescriptor:null,defineProperty:null,
                deleteProperty:null,ownKeys:null
            };
            var proxy=new Proxy(target,handler),nextProto={next:8};
            var initialProto=Reflect.getPrototypeOf(proxy)===Object.prototype;
            var changed=Reflect.setPrototypeOf(proxy,nextProto);
            var inherited=proxy.next;
            var extensible=Reflect.isExtensible(proxy);
            var has=Reflect.has(proxy,"x"),got=Reflect.get(proxy,"x");
            var set=Reflect.set(proxy,"x",2);
            var defined=Reflect.defineProperty(proxy,"y",{
                value:3,writable:true,enumerable:true,configurable:true
            });
            var descriptor=Reflect.getOwnPropertyDescriptor(proxy,"y");
            var keys=__proxyKeys(Reflect.ownKeys(proxy));
            var deleted=Reflect.deleteProperty(proxy,"y");
            var prevented=Reflect.preventExtensions(proxy);
            return [
                initialProto,changed,inherited,extensible,has,got,set,target.x,
                defined,__proxyBits(descriptor),keys,deleted,!Object.hasOwn(target,"y"),
                prevented,Reflect.isExtensible(target)
            ].join("|");
        })()"#,
    ),
    (
        "undefined and null apply construct traps preserve this arguments and newTarget",
        r#"(function(){
            function Target(a,b){
                if(new.target){
                    this.sum=a+b;this.seen=new.target;return;
                }
                return this.base+a+b;
            }
            var proxy=new Proxy(Target,{apply:null,construct:undefined});
            function NewTarget(){}
            var proto={kind:"new-target"};NewTarget.prototype=proto;
            var called=Reflect.apply(proxy,{base:2},[19,21]);
            var made=Reflect.construct(proxy,[20,22],NewTarget);
            return [
                called,made.sum,made.seen===NewTarget,
                Object.getPrototypeOf(made)===proto,made.kind,
                typeof proxy,__proxyIsConstructor(proxy)
            ].join("|");
        })()"#,
    ),
    (
        "missing mutation traps preserve target rejection errors",
        r#"(function(){
            var frozen={};
            Object.defineProperty(frozen,"x",{
                value:1,writable:false,configurable:false
            });
            var setProxy=new Proxy(frozen,{});
            function strictSet(){"use strict";setProxy.x=2}
            var closed=Object.preventExtensions({});
            var defineProxy=new Proxy(closed,{});
            return [
                __proxyCompletion(strictSet),
                __proxyCompletion(function(){
                    return Object.defineProperty(defineProxy,"x",{value:1});
                })
            ].join("|");
        })()"#,
    ),
    (
        "empty-handler forwarding retains pinned finite stack overflow behavior",
        r#"(function(){
            function wrap(count){
                var value={x:42};
                for(var index=0;index<count;index++)
                    value=new Proxy(value,{});
                return value;
            }
            var admitted=wrap(1500),overflow=wrap(3000);
            return [
                admitted.x,
                __proxyCompletion(function(){return overflow.x}),
                ({x:43}).x
            ].join("|");
        })()"#,
    ),
    (
        "empty-handler forwarding preserves operation-specific pinned QuickJS stack behavior",
        r#"(function(){
            function wrap(value,count){
                var handler={};
                while(count--)
                    value=new Proxy(value,handler);
                return value;
            }
            var callable=wrap(function(){return 42},2000);
            var object=wrap({},2518);
            return [
                __proxyCompletion(function(){return callable()}),
                __proxyCompletion(function(){return Reflect.getPrototypeOf(object)})
            ].join("|");
        })()"#,
    ),
    (
        "Iterator toStringTag setter dispatches Proxy descriptor and define traps",
        r#"(function(){
            var log=[],target=Object.create(Iterator.prototype);
            var proxy=new Proxy(target,{
                getOwnPropertyDescriptor:function(seen,key){
                    log.push("gopd:"+String(key));
                    return Reflect.getOwnPropertyDescriptor(seen,key);
                },
                defineProperty:function(seen,key,descriptor){
                    log.push("define:"+String(key)+":"+descriptor.value);
                    return Reflect.defineProperty(seen,key,descriptor);
                }
            });
            proxy[Symbol.toStringTag]="X";
            return [log.join("|"),Object.prototype.toString.call(proxy)].join("|");
        })()"#,
    ),
    (
        "Function bind observes Proxy HasOwnProperty before length and name Get",
        r#"(function(){
            var log=[];
            function target(a,b){}
            var proxy=new Proxy(target,{
                getOwnPropertyDescriptor:function(seen,key){
                    log.push("gopd:"+key);
                    return Reflect.getOwnPropertyDescriptor(seen,key);
                },
                get:function(seen,key,receiver){
                    log.push("get:"+key);
                    return Reflect.get(seen,key,receiver);
                }
            });
            var bound=Function.prototype.bind.call(proxy,null,1);
            return [bound.length,bound.name,log.join("|")].join("|");
        })()"#,
    ),
];

const TRAP_CASES: &[(&str, &str)] = &[
    (
        "getPrototypeOf and setPrototypeOf receive handler target and requested prototype",
        r#"(function(){
            var log=[],target={},reported={reported:1},requested={requested:1},handler={
                getPrototypeOf:function(seen){
                    log.push("get:"+(this===handler)+":"+(seen===target));return reported;
                },
                setPrototypeOf:function(seen,proto){
                    log.push("set:"+(this===handler)+":"+(seen===target)+":"+(proto===requested));
                    return "truthy";
                }
            };
            var proxy=new Proxy(target,handler);
            return [
                Reflect.getPrototypeOf(proxy)===reported,
                Reflect.setPrototypeOf(proxy,requested),
                Object.getPrototypeOf(target)===Object.prototype,
                log.join(",")
            ].join("|");
        })()"#,
    ),
    (
        "isExtensible and preventExtensions receive the raw target and coerce trap results",
        r#"(function(){
            var log=[],target={},handler={
                isExtensible:function(seen){
                    log.push("is:"+(this===handler)+":"+(seen===target));
                    return Reflect.isExtensible(seen)?1:0;
                },
                preventExtensions:function(seen){
                    log.push("prevent:"+(this===handler)+":"+(seen===target));
                    Reflect.preventExtensions(seen);return {};
                }
            };
            var proxy=new Proxy(target,handler);
            return [
                Reflect.isExtensible(proxy),Reflect.preventExtensions(proxy),
                Reflect.isExtensible(proxy),log.join(",")
            ].join("|");
        })()"#,
    ),
    (
        "descriptor traps receive property keys and only the requested descriptor fields",
        r#"(function(){
            var log=[],target={a:1},handler={
                getOwnPropertyDescriptor:function(seen,key){
                    log.push("get:"+(this===handler)+":"+(seen===target)+":"+key);
                    return Reflect.getOwnPropertyDescriptor(seen,key);
                },
                defineProperty:function(seen,key,descriptor){
                    log.push("define:"+(this===handler)+":"+(seen===target)+":"+key+":"+
                        Reflect.ownKeys(descriptor).join(","));
                    return Reflect.defineProperty(seen,key,descriptor);
                }
            };
            var proxy=new Proxy(target,handler);
            var defined=Reflect.defineProperty(proxy,"b",{value:2,enumerable:true});
            var descriptor=Reflect.getOwnPropertyDescriptor(proxy,"b");
            return [defined,__proxyBits(descriptor),log.join("|")].join("|");
        })()"#,
    ),
    (
        "has get set and delete traps preserve key value and explicit receiver",
        r#"(function(){
            var log=[],target={x:20},receiver={marker:"receiver"},handler={
                has:function(seen,key){
                    log.push("has:"+(this===handler)+":"+(seen===target)+":"+key);
                    return Reflect.has(seen,key);
                },
                get:function(seen,key,seenReceiver){
                    log.push("get:"+(this===handler)+":"+(seen===target)+":"+key+":"+
                        (seenReceiver===receiver));
                    return Reflect.get(seen,key,seenReceiver)+2;
                },
                set:function(seen,key,value,seenReceiver){
                    log.push("set:"+(this===handler)+":"+(seen===target)+":"+key+":"+value+":"+
                        (seenReceiver===receiver));
                    seenReceiver[key]=value;return true;
                },
                deleteProperty:function(seen,key){
                    log.push("delete:"+(this===handler)+":"+(seen===target)+":"+key);
                    return Reflect.deleteProperty(seen,key);
                }
            };
            var proxy=new Proxy(target,handler);
            return [
                Reflect.has(proxy,"x"),Reflect.get(proxy,"x",receiver),
                Reflect.set(proxy,"y",42,receiver),receiver.y,
                Reflect.deleteProperty(proxy,"x"),Object.hasOwn(target,"x"),
                log.join("|")
            ].join("|");
        })()"#,
    ),
    (
        "ownKeys receives target and preserves arbitrary string symbol order",
        r#"(function(){
            var symbol=Symbol("s"),target={a:1},log="",handler={
                ownKeys:function(seen){
                    log+=(this===handler)+":"+(seen===target);
                    return [symbol,"z","a"];
                }
            };
            var proxy=new Proxy(target,handler);
            return __proxyKeys(Reflect.ownKeys(proxy))+"|"+log;
        })()"#,
    ),
    (
        "apply receives target thisArg and a fresh dense argument array",
        r#"(function(){
            function Target(){return "target"}
            var receiver={marker:7},captured,handler={
                apply:function(seen,thisArg,args){
                    captured=args;
                    return [
                        this===handler,seen===Target,thisArg===receiver,
                        Array.isArray(args),args.length,args[0],args[1],
                        Object.getPrototypeOf(args)===Array.prototype
                    ].join(":");
                }
            };
            var proxy=new Proxy(Target,handler),result=Reflect.apply(proxy,receiver,[20,22]);
            return result+"|"+(captured!==arguments);
        })()"#,
    ),
    (
        "construct receives target dense arguments and exact custom newTarget",
        r#"(function(){
            function Target(){}
            function NewTarget(){}
            var captured,firstCaptured,handler={
                construct:function(seen,args,newTarget){
                    captured=args;
                    if(args.length===2)firstCaptured=args;
                    return {
                        summary:[
                            this===handler,seen===Target,newTarget===NewTarget,
                            Array.isArray(args),args.length,args[0],args[1],
                            Object.getPrototypeOf(args)===Array.prototype
                        ].join(":")
                    };
                }
            };
            var proxy=new Proxy(Target,handler);
            var value=Reflect.construct(proxy,[20,22],NewTarget);
            var direct=new proxy(1);
            return value.summary+"|"+(firstCaptured.length===2)+":"+
                (firstCaptured!==captured)+"|"+
                __proxyValue(direct.summary)+":"+(direct.summary.indexOf("false")<0);
        })()"#,
    ),
    (
        "trap getter is read before invocation and a noncallable trap is rejected",
        r#"(function(){
            var log=[],target={x:42},handler={};
            Object.defineProperty(handler,"get",{get:function(){
                log.push("getter");
                return function(seen,key,receiver){
                    log.push("call:"+(this===handler)+":"+(seen===target)+":"+key+
                        ":"+(receiver===proxy));
                    return 42;
                };
            },configurable:true});
            var proxy=new Proxy(target,handler),first=proxy.x;
            Object.defineProperty(handler,"get",{value:1,configurable:true});
            var second=__proxyCompletion(function(){return proxy.x});
            return [first,second,log.join("|")].join("|");
        })()"#,
    ),
    (
        "false mutation traps feed Reflect booleans and throwing language or Object APIs",
        r#"(function(){
            var target={x:1},handler={
                set:function(){return false},
                defineProperty:function(){return false},
                deleteProperty:function(){return false}
            };
            var proxy=new Proxy(target,handler);
            function strictSet(){"use strict";proxy.x=2}
            return [
                __proxyCompletion(function(){return Reflect.set(proxy,"x",2)}),target.x,
                __proxyCompletion(strictSet),target.x,
                __proxyCompletion(function(){
                    return Reflect.defineProperty(proxy,"y",{value:2});
                }),
                __proxyCompletion(function(){
                    return Object.defineProperty(proxy,"y",{value:2});
                }),
                __proxyCompletion(function(){return Reflect.deleteProperty(proxy,"x")}),
                __proxyCompletion(function(){return delete proxy.x}),target.x
            ].join("|");
        })()"#,
    ),
    (
        "ownKeys consumes an array-like trap result length then indices in order",
        r#"(function(){
            var log=[],symbol=Symbol("s"),result={};
            Object.defineProperty(result,"length",{get:function(){log.push("length");return 2}});
            Object.defineProperty(result,"0",{get:function(){log.push("0");return "a"}});
            Object.defineProperty(result,"1",{get:function(){log.push("1");return symbol}});
            var proxy=new Proxy({a:1},{ownKeys:function(){log.push("trap");return result}});
            return __proxyKeys(Reflect.ownKeys(proxy))+"|"+log.join(",");
        })()"#,
    ),
];

const INVARIANT_CASES: &[(&str, &str)] = &[
    (
        "getPrototypeOf rejects primitives and mismatches on nonextensible targets",
        r#"(function(){
            var proto={},other={},target=Object.create(proto);
            var primitive=new Proxy(target,{getPrototypeOf:function(){return 1}});
            Object.preventExtensions(target);
            var mismatch=new Proxy(target,{getPrototypeOf:function(){return other}});
            var matching=new Proxy(target,{getPrototypeOf:function(){return proto}});
            return [
                __proxyCompletion(function(){return Reflect.getPrototypeOf(primitive)}),
                __proxyCompletion(function(){return Reflect.getPrototypeOf(mismatch)}),
                __proxyCompletion(function(){return Reflect.getPrototypeOf(matching)===proto})
            ].join("|");
        })()"#,
    ),
    (
        "setPrototypeOf false is observable and true cannot mismatch a nonextensible target",
        r#"(function(){
            var proto={},other={},target=Object.preventExtensions(Object.create(proto));
            var rejecting=new Proxy(target,{setPrototypeOf:function(){return 0}});
            var mismatch=new Proxy(target,{setPrototypeOf:function(){return true}});
            var matching=new Proxy(target,{setPrototypeOf:function(seen,next){return next===proto}});
            return [
                __proxyCompletion(function(){return Reflect.setPrototypeOf(rejecting,other)}),
                __proxyCompletion(function(){return Object.setPrototypeOf(rejecting,other)}),
                __proxyCompletion(function(){return Reflect.setPrototypeOf(mismatch,other)}),
                __proxyCompletion(function(){return Reflect.setPrototypeOf(matching,proto)})
            ].join("|");
        })()"#,
    ),
    (
        "isExtensible trap result must equal the target state in both directions",
        r#"(function(){
            var open={},closed=Object.preventExtensions({});
            var cases=[
                new Proxy(open,{isExtensible:function(){return false}}),
                new Proxy(closed,{isExtensible:function(){return true}}),
                new Proxy(open,{isExtensible:function(){return true}}),
                new Proxy(closed,{isExtensible:function(){return false}})
            ];
            return cases.map(function(proxy){
                return __proxyCompletion(function(){return Reflect.isExtensible(proxy)});
            }).join("|");
        })()"#,
    ),
    (
        "preventExtensions may report false but true requires a closed target",
        r#"(function(){
            var open1={},open2={},open3={};
            var falseTrap=new Proxy(open1,{preventExtensions:function(){return false}});
            var lying=new Proxy(open2,{preventExtensions:function(){return true}});
            var closing=new Proxy(open3,{preventExtensions:function(target){
                Object.preventExtensions(target);return true;
            }});
            return [
                __proxyCompletion(function(){return Reflect.preventExtensions(falseTrap)}),
                Object.isExtensible(open1),
                __proxyCompletion(function(){return Reflect.preventExtensions(lying)}),
                Object.isExtensible(open2),
                __proxyCompletion(function(){return Reflect.preventExtensions(closing)}),
                Object.isExtensible(open3)
            ].join("|");
        })()"#,
    ),
    (
        "has cannot hide nonconfigurable or nonextensible own properties",
        r#"(function(){
            var fixed={},closed={x:1},open={x:1};
            Object.defineProperty(fixed,"x",{value:1,configurable:false});
            Object.preventExtensions(closed);
            var fixedProxy=new Proxy(fixed,{has:function(){return false}});
            var closedProxy=new Proxy(closed,{has:function(){return false}});
            var openProxy=new Proxy(open,{has:function(){return false}});
            return [
                __proxyCompletion(function(){return Reflect.has(fixedProxy,"x")}),
                __proxyCompletion(function(){return Reflect.has(closedProxy,"x")}),
                __proxyCompletion(function(){return Reflect.has(openProxy,"x")}),
                Object.hasOwn(open,"x")
            ].join("|");
        })()"#,
    ),
    (
        "get preserves SameValue for frozen data including NaN and signed zero",
        r#"(function(){
            var one={},nan={},zero={};
            Object.defineProperty(one,"x",{value:1,writable:false,configurable:false});
            Object.defineProperty(nan,"x",{value:NaN,writable:false,configurable:false});
            Object.defineProperty(zero,"x",{value:-0,writable:false,configurable:false});
            return [
                __proxyCompletion(function(){return new Proxy(one,{get:function(){return 2}}).x}),
                __proxyCompletion(function(){return new Proxy(one,{get:function(){return 1}}).x}),
                __proxyCompletion(function(){return new Proxy(nan,{get:function(){return NaN}}).x}),
                __proxyCompletion(function(){return new Proxy(zero,{get:function(){return +0}}).x}),
                __proxyCompletion(function(){return new Proxy(zero,{get:function(){return -0}}).x})
            ].join("|");
        })()"#,
    ),
    (
        "get cannot invent a value for a frozen accessor without a getter",
        r#"(function(){
            var target={};
            Object.defineProperty(target,"x",{get:undefined,set:function(){},configurable:false});
            var lying=new Proxy(target,{get:function(){return 1}});
            var matching=new Proxy(target,{get:function(){return undefined}});
            return [
                __proxyCompletion(function(){return lying.x}),
                __proxyCompletion(function(){return matching.x})
            ].join("|");
        })()"#,
    ),
    (
        "set cannot report success for an incompatible frozen data write",
        r#"(function(){
            var target={};
            Object.defineProperty(target,"x",{value:1,writable:false,configurable:false});
            var proxy=new Proxy(target,{set:function(){return true}});
            return [
                __proxyCompletion(function(){return Reflect.set(proxy,"x",2)}),
                __proxyCompletion(function(){return Reflect.set(proxy,"x",1)}),
                target.x
            ].join("|");
        })()"#,
    ),
    (
        "set cannot report success for a frozen accessor without a setter",
        r#"(function(){
            var target={};
            Object.defineProperty(target,"x",{get:function(){return 1},set:undefined,configurable:false});
            var proxy=new Proxy(target,{set:function(){return true}});
            return __proxyCompletion(function(){return Reflect.set(proxy,"x",2)});
        })()"#,
    ),
    (
        "getOwnPropertyDescriptor requires object or undefined and cannot hide fixed properties",
        r#"(function(){
            var fixed={},closed={x:1},open={x:1};
            Object.defineProperty(fixed,"x",{value:1,configurable:false});
            Object.preventExtensions(closed);
            var primitive=new Proxy(open,{getOwnPropertyDescriptor:function(){return 1}});
            var hideFixed=new Proxy(fixed,{getOwnPropertyDescriptor:function(){return undefined}});
            var hideClosed=new Proxy(closed,{getOwnPropertyDescriptor:function(){return undefined}});
            var hideOpen=new Proxy(open,{getOwnPropertyDescriptor:function(){return undefined}});
            return [
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(primitive,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(hideFixed,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(hideClosed,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(hideOpen,"x")})
            ].join("|");
        })()"#,
    ),
    (
        "getOwnPropertyDescriptor cannot add to a closed target or invent fixed state",
        r#"(function(){
            var closed=Object.preventExtensions({}),open={},configurable={x:1};
            var descriptor={value:1,writable:true,enumerable:true,configurable:true};
            var addClosed=new Proxy(closed,{getOwnPropertyDescriptor:function(){return descriptor}});
            var addFixed=new Proxy(open,{getOwnPropertyDescriptor:function(){
                return {value:1,writable:true,enumerable:true,configurable:false};
            }});
            var tighten=new Proxy(configurable,{getOwnPropertyDescriptor:function(){
                return {value:1,writable:true,enumerable:true,configurable:false};
            }});
            return [
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(addClosed,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(addFixed,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(tighten,"x")})
            ].join("|");
        })()"#,
    ),
    (
        "getOwnPropertyDescriptor enforces frozen kind writable and flag compatibility",
        r#"(function(){
            var data={},accessor={},getter=function(){return 1};
            Object.defineProperty(data,"x",{value:1,writable:false,enumerable:false,configurable:false});
            Object.defineProperty(accessor,"x",{get:getter,enumerable:false,configurable:false});
            var writable=new Proxy(data,{getOwnPropertyDescriptor:function(){
                return {value:1,writable:true,enumerable:false,configurable:false};
            }});
            var enumerable=new Proxy(data,{getOwnPropertyDescriptor:function(){
                return {value:1,writable:false,enumerable:true,configurable:false};
            }});
            var kind=new Proxy(accessor,{getOwnPropertyDescriptor:function(){
                return {value:1,writable:false,enumerable:false,configurable:false};
            }});
            return [
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(writable,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(enumerable,"x")}),
                __proxyCompletion(function(){return Object.getOwnPropertyDescriptor(kind,"x")})
            ].join("|");
        })()"#,
    ),
    (
        "getOwnPropertyDescriptor completes omitted flags before publishing the result",
        r#"(function(){
            var target={};
            Object.defineProperty(target,"x",{
                value:42,writable:false,enumerable:false,configurable:false
            });
            var proxy=new Proxy(target,{getOwnPropertyDescriptor:function(){
                return {value:42};
            }});
            var descriptor=Object.getOwnPropertyDescriptor(proxy,"x");
            return [
                __proxyBits(descriptor),Reflect.ownKeys(descriptor).join(","),
                descriptor.value,descriptor.writable,
                descriptor.enumerable,descriptor.configurable
            ].join("|");
        })()"#,
    ),
    (
        "defineProperty cannot add to closed targets or fabricate nonconfigurable properties",
        r#"(function(){
            var closed=Object.preventExtensions({}),open={},configurable={x:1};
            var addClosed=new Proxy(closed,{defineProperty:function(){return true}});
            var addFixed=new Proxy(open,{defineProperty:function(){return true}});
            var tighten=new Proxy(configurable,{defineProperty:function(){return true}});
            return [
                __proxyCompletion(function(){
                    return Reflect.defineProperty(addClosed,"x",{value:1});
                }),
                __proxyCompletion(function(){
                    return Reflect.defineProperty(addFixed,"x",{value:1,configurable:false});
                }),
                __proxyCompletion(function(){
                    return Reflect.defineProperty(tighten,"x",{configurable:false});
                })
            ].join("|");
        })()"#,
    ),
    (
        "defineProperty enforces frozen data accessor identity and writable tightening",
        r#"(function(){
            var frozen={},accessor={},writable={};
            var getter=function(){return 1},other=function(){return 2};
            Object.defineProperty(frozen,"x",{value:1,writable:false,configurable:false});
            Object.defineProperty(accessor,"x",{get:getter,configurable:false});
            Object.defineProperty(writable,"x",{value:1,writable:true,configurable:false});
            var dataProxy=new Proxy(frozen,{defineProperty:function(){return true}});
            var accessorProxy=new Proxy(accessor,{defineProperty:function(){return true}});
            var writableProxy=new Proxy(writable,{defineProperty:function(){return true}});
            return [
                __proxyCompletion(function(){
                    return Reflect.defineProperty(dataProxy,"x",{value:2});
                }),
                __proxyCompletion(function(){
                    return Reflect.defineProperty(accessorProxy,"x",{get:other});
                }),
                __proxyCompletion(function(){
                    return Reflect.defineProperty(writableProxy,"x",{writable:false});
                }),
                __proxyCompletion(function(){
                    return Reflect.defineProperty(dataProxy,"x",{value:1});
                })
            ].join("|");
        })()"#,
    ),
    (
        "deleteProperty cannot hide nonconfigurable or nonextensible own properties",
        r#"(function(){
            var fixed={},closed={x:1},open={x:1};
            Object.defineProperty(fixed,"x",{value:1,configurable:false});
            Object.preventExtensions(closed);
            var fixedProxy=new Proxy(fixed,{deleteProperty:function(){return true}});
            var closedProxy=new Proxy(closed,{deleteProperty:function(){return true}});
            var openProxy=new Proxy(open,{deleteProperty:function(){return true}});
            var falseProxy=new Proxy(open,{deleteProperty:function(){return false}});
            return [
                __proxyCompletion(function(){return Reflect.deleteProperty(fixedProxy,"x")}),
                __proxyCompletion(function(){return Reflect.deleteProperty(closedProxy,"x")}),
                __proxyCompletion(function(){return Reflect.deleteProperty(openProxy,"x")}),
                Object.hasOwn(open,"x"),
                __proxyCompletion(function(){return Reflect.deleteProperty(falseProxy,"x")})
            ].join("|");
        })()"#,
    ),
    (
        "ownKeys accepts only strings and symbols and rejects duplicates",
        r#"(function(){
            var target={},symbol=Symbol("s"),cases=[
                new Proxy(target,{ownKeys:function(){return ["a",1]}}),
                new Proxy(target,{ownKeys:function(){return ["a","a"]}}),
                new Proxy(target,{ownKeys:function(){return [symbol,symbol]}}),
                new Proxy(target,{ownKeys:function(){return ["a",symbol]}})
            ];
            return cases.map(function(proxy){
                return __proxyCompletion(function(){return __proxyKeys(Reflect.ownKeys(proxy))});
            }).join("|");
        })()"#,
    ),
    (
        "ownKeys must include fixed keys and exactly match a nonextensible target",
        r#"(function(){
            var fixed={},closed={a:1},open={a:1};
            Object.defineProperty(fixed,"x",{value:1,configurable:false});
            Object.preventExtensions(closed);
            var missFixed=new Proxy(fixed,{ownKeys:function(){return []}});
            var missClosed=new Proxy(closed,{ownKeys:function(){return []}});
            var extraClosed=new Proxy(closed,{ownKeys:function(){return ["a","extra"]}});
            var extraOpen=new Proxy(open,{ownKeys:function(){return ["extra","a"]}});
            return [
                __proxyCompletion(function(){return Reflect.ownKeys(missFixed)}),
                __proxyCompletion(function(){return Reflect.ownKeys(missClosed)}),
                __proxyCompletion(function(){return Reflect.ownKeys(extraClosed)}),
                __proxyCompletion(function(){return __proxyKeys(Reflect.ownKeys(extraOpen))})
            ].join("|");
        })()"#,
    ),
    (
        "ownKeys preserves a large reverse exact result",
        r#"(function(){
            var count=4096,target={},keys=[];
            for(var index=0;index<count;index++){
                var key="reverse-"+index;
                keys.push(key);
                Object.defineProperty(target,key,{value:index,configurable:index%2===0});
            }
            Object.preventExtensions(target);
            var proxy=new Proxy(target,{ownKeys:function(){return keys.slice().reverse()}});
            var result=Reflect.ownKeys(proxy);
            return [result.length,result[0],result[result.length-1]].join("|");
        })()"#,
    ),
    (
        "construct trap must return an object and cannot make a nonconstructor constructable",
        r#"(function(){
            function Constructor(){}
            var primitive=new Proxy(Constructor,{construct:function(){return 1}});
            var nonConstructor=({method(){}}).method;
            var nonconstructor=new Proxy(nonConstructor,{construct:function(){return {made:true}}});
            return [
                __proxyCompletion(function(){return Reflect.construct(primitive,[])}),
                __proxyIsConstructor(nonconstructor),
                __proxyCompletion(function(){return Reflect.construct(nonconstructor,[])})
            ].join("|");
        })()"#,
    ),
];

const QUIRK_CASES: &[(&str, &str)] = &[
    (
        "pinned ownKeys rechecks revocation after the trap returns",
        r#"(function(){
            var pair,target={x:1};
            Object.defineProperty(target,"fixed",{value:2,configurable:false});
            pair=Proxy.revocable(target,{ownKeys:function(){
                pair.revoke();return ["x","fixed"];
            }});
            return __proxyCompletion(function(){return __proxyKeys(Reflect.ownKeys(pair.proxy))});
        })()"#,
    ),
    (
        "pinned getOwnPropertyDescriptor frozen checks compare flags but not value or getter identity",
        r#"(function(){
            var data={},accessor={},first=function(){return 1},second=function(){return 2};
            Object.defineProperty(data,"x",{
                value:1,writable:false,enumerable:false,configurable:false
            });
            Object.defineProperty(accessor,"x",{
                get:first,set:undefined,enumerable:false,configurable:false
            });
            var dataProxy=new Proxy(data,{getOwnPropertyDescriptor:function(){
                return {value:2,writable:false,enumerable:false,configurable:false};
            }});
            var accessorProxy=new Proxy(accessor,{getOwnPropertyDescriptor:function(){
                return {get:second,set:undefined,enumerable:false,configurable:false};
            }});
            var dataDescriptor=Object.getOwnPropertyDescriptor(dataProxy,"x");
            var accessorDescriptor=Object.getOwnPropertyDescriptor(accessorProxy,"x");
            return [
                dataDescriptor.value,dataDescriptor.value!==data.x,
                accessorDescriptor.get===second,accessorDescriptor.get!==first
            ].join("|");
        })()"#,
    ),
    (
        "pinned noncallable Proxy reads apply before reporting not a function",
        r#"(function(){
            var log="",handler={};
            Object.defineProperty(handler,"apply",{get:function(){
                log+="get;";
                return function(){log+="call;";return 42};
            }});
            var proxy=new Proxy({},handler);
            var result=__proxyCompletion(function(){return proxy()});
            return [typeof proxy,result,log].join("|");
        })()"#,
    ),
    (
        "pinned ownKeys reads every entry before reporting duplicates",
        r#"(function(){
            var log=[],result={length:3,0:"a",1:"a"};
            Object.defineProperty(result,"2",{get:function(){
                log.push("later");throw 42;
            }});
            var proxy=new Proxy({},{ownKeys:function(){return result}});
            return [
                __proxyCompletion(function(){return Reflect.ownKeys(proxy)}),
                log.join(",")
            ].join("|");
        })()"#,
    ),
    (
        "pinned trap dispatch enters a noncallable Proxy before rejecting it",
        r#"(function(){
            var log="",trapHandler={};
            Object.defineProperty(trapHandler,"apply",{get:function(){
                log+="get;";return function(){log+="call;";return 1};
            }});
            var trap=new Proxy({},trapHandler);
            var proxy=new Proxy({x:42},{get:trap});
            return [
                __proxyCompletion(function(){return proxy.x}),
                log
            ].join("|");
        })()"#,
    ),
    (
        "GetFunctionRealm reports revocation after an observable prototype Get",
        r#"(function(){
            var pair;
            pair=Proxy.revocable(function(){},{
                get:function(target,key,receiver){
                    if(key==="prototype"){pair.revoke();return null}
                    return Reflect.get(target,key,receiver);
                }
            });
            return __proxyCompletion(function(){
                return Reflect.construct(function(){},[],pair.proxy);
            });
        })()"#,
    ),
    (
        "pinned nested Proxy undefined descriptor trap bypasses target IsExtensible",
        r#"(function(){
            var log=[],base={x:1};
            var inner=new Proxy(base,{
                getOwnPropertyDescriptor:function(target,key){
                    log.push("inner-descriptor:"+key);
                    return Reflect.getOwnPropertyDescriptor(target,key);
                },
                isExtensible:function(target){
                    log.push("inner-extensible");
                    return Reflect.isExtensible(target);
                }
            });
            var outer=new Proxy(inner,{
                getOwnPropertyDescriptor:function(){
                    log.push("outer-descriptor");
                    return undefined;
                }
            });
            var descriptor=Object.getOwnPropertyDescriptor(outer,"x");
            return [descriptor===undefined,log.join("|")].join("|");
        })()"#,
    ),
    (
        "pinned descriptor conversion replaces an abrupt HasProperty with the following Get",
        r#"(function(){
            var first=true,log=[];
            var descriptor=new Proxy({
                value:42,writable:true,enumerable:true,configurable:true
            },{
                has:function(target,key){
                    log.push("has:"+key);
                    if(first){first=false;throw 17}
                    return Reflect.has(target,key);
                },
                get:function(target,key,receiver){
                    log.push("get:"+key);
                    return Reflect.get(target,key,receiver);
                }
            });
            var proxy=new Proxy({},{
                getOwnPropertyDescriptor:function(){return descriptor}
            });
            var result=Object.getOwnPropertyDescriptor(proxy,"x");
            return [__proxyBits(result),log.join("|")].join("|");
        })()"#,
    ),
];

const REENTRANCY_CASES: &[(&str, &str)] = &[
    (
        "a get trap can reenter the same Proxy after changing its recursion state",
        r#"(function(){
            var depth=0,log=[],target={x:41},handler={},proxy;
            handler.get=function(seen,key,receiver){
                log.push("get:"+depth+":"+(receiver===proxy));
                if(depth++===0)return Reflect.get(proxy,key,receiver)+1;
                return Reflect.get(seen,key,receiver);
            };
            proxy=new Proxy(target,handler);
            return [proxy.x,depth,log.join("|")].join("|");
        })()"#,
    ),
    (
        "ownKeys can reenter getOwnPropertyDescriptor on the same Proxy",
        r#"(function(){
            var log=[],target={x:42},handler={},proxy;
            handler.ownKeys=function(seen){
                log.push("keys");
                var descriptor=Reflect.getOwnPropertyDescriptor(proxy,"x");
                log.push("seen:"+descriptor.value);
                return Reflect.ownKeys(seen);
            };
            handler.getOwnPropertyDescriptor=function(seen,key){
                log.push("descriptor:"+key);
                return Reflect.getOwnPropertyDescriptor(seen,key);
            };
            proxy=new Proxy(target,handler);
            return __proxyKeys(Reflect.ownKeys(proxy))+"|"+log.join("|");
        })()"#,
    ),
    (
        "a trap may revoke itself while target and handler stay live on the native stack",
        r#"(function(){
            var pair,log=[],target={};
            Object.defineProperty(target,"x",{value:42,writable:false,configurable:false});
            pair=Proxy.revocable(target,{get:function(seen,key,receiver){
                log.push("trap:"+key+":"+(seen===target)+":"+(receiver===pair.proxy));
                pair.revoke();
                return 42;
            }});
            var first=pair.proxy.x;
            var second=__proxyCompletion(function(){return pair.proxy.x});
            return [first,second,log.join("|")].join("|");
        })()"#,
    ),
    (
        "nested ownKeys fallback rechecks the Proxy which supplied the trap",
        r#"(function(){
            var innerPair=Proxy.revocable({x:1},{
                ownKeys:function(){
                    innerPair.revoke();
                    return ["x"];
                }
            });
            var outer=new Proxy(innerPair.proxy,{});
            var first=__proxyCompletion(function(){return Reflect.ownKeys(outer)});

            var outerPair;
            var inner=new Proxy({x:1},{
                ownKeys:function(){
                    outerPair.revoke();
                    return ["x"];
                }
            });
            outerPair=Proxy.revocable(inner,{});
            var second=__proxyCompletion(function(){
                return __proxyKeys(Reflect.ownKeys(outerPair.proxy));
            });
            return [first,second].join("|");
        })()"#,
    ),
];

#[test]
fn proxy_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Proxy oracle self-check: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    for &(group, cases) in case_groups() {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }
}

#[test]
fn proxy_forwarding_and_generic_consumers_have_rust_only_regressions() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (description, expected) in [
        (
            "empty-handler forwarding retains pinned finite stack overflow behavior",
            "return|string|42|throw:InternalError:stack overflow|43",
        ),
        (
            "empty-handler forwarding preserves operation-specific pinned QuickJS stack behavior",
            "return|string|throw:InternalError:stack overflow|return:object",
        ),
        (
            "Iterator toStringTag setter dispatches Proxy descriptor and define traps",
            "return|string|gopd:Symbol(Symbol.toStringTag)|define:Symbol(Symbol.toStringTag):X|[object X]",
        ),
        (
            "Function bind observes Proxy HasOwnProperty before length and name Get",
            "return|string|1|bound target|gopd:length|get:length|get:name",
        ),
        (
            "pinned descriptor conversion replaces an abrupt HasProperty with the following Get",
            "return|string|d:42:1:11|has:enumerable|get:enumerable|has:configurable|get:configurable|has:value|get:value|has:writable|get:writable|has:get|has:set",
        ),
        (
            "nested ownKeys fallback rechecks the Proxy which supplied the trap",
            "return|string|throw:TypeError:revoked proxy|return:x",
        ),
    ] {
        let source = case_groups()
            .iter()
            .flat_map(|(_, cases)| cases.iter())
            .find_map(|&(candidate, source)| (candidate == description).then_some(source))
            .unwrap_or_else(|| panic!("missing Proxy fallback case: {description}"));
        assert_eq!(
            observe_rust(&runtime, &mut context, source, description),
            expected,
            "{description}",
        );
    }
}

#[test]
fn proxy_graph_lifecycle_and_fallback_match_pinned_quickjs() {
    compare_groups(&["graph", "lifecycle", "fallback"]);
}

#[test]
fn proxy_all_thirteen_traps_match_pinned_quickjs() {
    compare_groups(&["traps"]);
}

#[test]
fn proxy_invariants_match_pinned_quickjs() {
    compare_groups(&["invariants"]);
}

#[test]
fn proxy_pinned_quirks_match_quickjs_2026_06_04() {
    compare_groups(&["pinned quirks"]);
}

#[test]
fn proxy_reentrancy_matches_pinned_quickjs() {
    compare_groups(&["reentrancy"]);
}

#[test]
fn proxy_native_and_user_errors_use_their_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
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

    let invariant_proxy = eval_object(
        &mut defining,
        r#"(function(){
            var target={x:1};Object.preventExtensions(target);
            return new Proxy(target,{has:function(){return false}});
        })()"#,
        "foreign invariant Proxy",
    );
    define_global(
        &runtime,
        &mut caller,
        "foreignProxy",
        Value::Object(invariant_proxy),
    );
    assert_eq!(
        caller.eval("'x' in foreignProxy"),
        Err(RuntimeError::Exception),
    );
    let invariant_error = take_exception_object(&mut caller, "foreign Proxy invariant TypeError");
    assert_eq!(
        runtime.get_prototype_of(&invariant_error).unwrap(),
        Some(caller_type_error.clone()),
        "Proxy's internal invariant TypeError did not use the active caller realm",
    );

    let revoked_proxy = eval_object(
        &mut defining,
        r#"(function(){
            var pair=Proxy.revocable(function(){},{});
            pair.revoke();
            return pair.proxy;
        })()"#,
        "foreign revoked Proxy",
    );
    define_global(
        &runtime,
        &mut caller,
        "foreignProxy",
        Value::Object(revoked_proxy),
    );
    assert_eq!(caller.eval("foreignProxy.x"), Err(RuntimeError::Exception));
    let revoked_error = take_exception_object(&mut caller, "foreign revoked Proxy TypeError");
    assert_eq!(
        runtime.get_prototype_of(&revoked_error).unwrap(),
        Some(caller_type_error.clone()),
        "revoked Proxy TypeError did not use the active caller realm",
    );

    let throwing_proxy = eval_object(
        &mut defining,
        r#"(new Proxy({x:1},{get:function(){throw new TypeError("defining trap")}}))"#,
        "foreign throwing Proxy",
    );
    define_global(
        &runtime,
        &mut caller,
        "foreignProxy",
        Value::Object(throwing_proxy),
    );
    assert_eq!(caller.eval("foreignProxy.x"), Err(RuntimeError::Exception));
    let user_error = take_exception_object(&mut caller, "foreign Proxy user TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(defining_type_error),
        "Proxy replaced a defining-realm user trap throw with a caller-realm error",
    );

    let bad_construct_proxy = eval_object(
        &mut defining,
        "(new Proxy(function(){},{construct:function(){return 1}}))",
        "foreign primitive-returning construct Proxy",
    );
    define_global(
        &runtime,
        &mut caller,
        "foreignProxy",
        Value::Object(bad_construct_proxy),
    );
    assert_eq!(
        caller.eval("new foreignProxy()"),
        Err(RuntimeError::Exception)
    );
    let construct_error = take_exception_object(&mut caller, "foreign Proxy construct TypeError");
    assert_eq!(
        runtime.get_prototype_of(&construct_error).unwrap(),
        Some(caller_type_error),
        "Proxy's construct-result TypeError did not use the active caller realm",
    );
}

#[test]
fn proxy_hidden_edges_survive_gc_and_revocation_is_safe_across_collection() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let record = eval_object(
        &mut context,
        r#"(function(){
            var target={answer:40},handler={
                get:function(seen,key,receiver){return Reflect.get(seen,key,receiver)+2}
            };
            target.handler=handler;handler.target=target;
            return Proxy.revocable(target,handler);
        })()"#,
        "cyclic revocable Proxy record",
    );
    let proxy = object_property(&runtime, &mut context, &record, "proxy");
    let revoke_object = object_property(&runtime, &mut context, &record, "revoke");
    let revoke = runtime
        .as_callable(&revoke_object)
        .unwrap()
        .expect("Proxy revoke object was not callable");
    drop(record);

    runtime.run_gc().expect("collect while Proxy is live");
    assert_eq!(
        context
            .get_property(&proxy, &runtime.intern_property_key("answer").unwrap())
            .expect("Proxy target and handler did not survive collection"),
        Value::Int(42),
    );

    context
        .call(&revoke, Value::Undefined, &[])
        .expect("revoke after collection");
    drop(revoke);
    drop(revoke_object);
    runtime
        .run_gc()
        .expect("collect after one-shot revoke capture");
    assert!(matches!(
        context.get_property(&proxy, &runtime.intern_property_key("answer").unwrap()),
        Err(RuntimeError::Exception),
    ));
    let error = take_exception_object(&mut context, "revoked Proxy after collection");
    assert_eq!(
        string_property(&runtime, &mut context, &error, "name"),
        "TypeError",
    );
}

fn case_groups() -> &'static [(&'static str, &'static [(&'static str, &'static str)])] {
    &[
        ("graph", GRAPH_CASES),
        ("lifecycle", LIFECYCLE_CASES),
        ("fallback", FALLBACK_CASES),
        ("traps", TRAP_CASES),
        ("invariants", INVARIANT_CASES),
        ("pinned quirks", QUIRK_CASES),
        ("reentrancy", REENTRANCY_CASES),
    ]
}

fn compare_groups(groups: &[&str]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP Proxy differential for {}: set QJS_ORACLE to pinned upstream qjs",
            groups.join(", "),
        );
        return;
    };
    let mut failures = Vec::new();
    for &(group, cases) in case_groups()
        .iter()
        .filter(|(group, _)| groups.contains(group))
    {
        for &(description, source) in cases {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            let oxide = observe_rust(&runtime, &mut context, source, description);
            let quickjs = observe_oracle(&oracle, source, description);
            if oxide != quickjs {
                failures.push(format!(
                    "{group} / {description}\nsource: {source:?}\noxide: {oxide:?}\nquickjs: {quickjs:?}",
                ));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "Proxy semantics drifted in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn observed_source(source: &str) -> String {
    format!("{PRELUDE}\n{source}")
}

fn observe_rust(
    runtime: &Runtime,
    context: &mut Context,
    source: &str,
    description: &str,
) -> String {
    match context.eval(&observed_source(source)) {
        Ok(value) => format!(
            "return|{}|{}",
            value_type(runtime, &value),
            primitive_text(value),
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
                    primitive_text(value),
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
  if (error !== null &&
      (typeof error === 'object' || typeof error === 'function'))
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;
    let source = observed_source(source);
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

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn eval_object(context: &mut Context, source: &str, description: &str) -> ObjectRef {
    let Value::Object(object) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("Rust rejected {description} ({source:?}): {error}"))
    else {
        panic!("{description} was not an object");
    };
    object
}

fn object_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> ObjectRef {
    let Value::Object(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap_or_else(|error| panic!("read object property {name}: {error}"))
    else {
        panic!("{name} was not an object");
    };
    value
}

fn take_exception_object(context: &mut Context, description: &str) -> ObjectRef {
    let Some(Value::Object(error)) = context
        .take_exception()
        .unwrap_or_else(|failure| panic!("take {description}: {failure}"))
    else {
        panic!("{description} was missing or was not an object");
    };
    error
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap_or_else(|error| panic!("read {name}: {error}"))
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
            if runtime
                .as_callable(object)
                .expect("inspect callable")
                .is_some()
            {
                "function"
            } else {
                "object"
            }
        }
        Value::Symbol(_) => "symbol",
    }
}

fn primitive_text(value: Value) -> String {
    match value {
        Value::Undefined => "undefined".to_owned(),
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Int(value) => value.to_string(),
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
