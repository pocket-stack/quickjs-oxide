use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, JsString, ObjectRef, Runtime, RuntimeError, Value};

// Pins the complete QuickJS 2026-06-04 Reflect intrinsic. In particular, the
// construct-order vectors intentionally follow `js_reflect_construct` rather
// than deriving an order from the prose specification: an explicit newTarget
// is checked first, the array-like argument list is then materialized, and the
// target constructor check happens in `JS_CallConstructor2` afterwards.

const METHODS: &[(&str, usize)] = &[
    ("apply", 3),
    ("construct", 2),
    ("defineProperty", 3),
    ("deleteProperty", 2),
    ("get", 2),
    ("getOwnPropertyDescriptor", 2),
    ("getPrototypeOf", 1),
    ("has", 2),
    ("isExtensible", 1),
    ("ownKeys", 1),
    ("preventExtensions", 1),
    ("set", 3),
    ("setPrototypeOf", 2),
];

const GRAPH_CASES: &[(&str, &str)] = &[(
    "global graph key order descriptors metadata identities and tag",
    r#"(function(){
        function bit(value){return value?1:0}
        function bits(descriptor){
            return "D:"+bit(descriptor.writable)+bit(descriptor.enumerable)+bit(descriptor.configurable);
        }
        function isConstructor(value){
            try{Reflect.construct(function(){},[],value);return true}catch(_){return false}
        }
        function keyText(key){return typeof key==="symbol"?String(key):key}
        var names=["apply","construct","defineProperty","deleteProperty","get",
            "getOwnPropertyDescriptor","getPrototypeOf","has","isExtensible","ownKeys",
            "preventExtensions","set","setPrototypeOf"];
        var lengths=[3,2,3,2,2,2,1,2,1,1,1,3,2];
        var descriptor=Object.getOwnPropertyDescriptor(globalThis,"Reflect");
        var implemented=["String","Math","Reflect","Symbol","globalThis"];
        var metadata=[];
        for(var i=0;i<names.length;i++){
            var name=names[i],own=Object.getOwnPropertyDescriptor(Reflect,name),fn=own.value;
            var fnName=Object.getOwnPropertyDescriptor(fn,"name");
            var fnLength=Object.getOwnPropertyDescriptor(fn,"length");
            metadata[metadata.length]=name+":"+fn.name+":"+fn.length+":"+
                (fn.length===lengths[i])+":"+(Object.getPrototypeOf(fn)===Function.prototype)+":"+
                (typeof fn==="function")+":"+isConstructor(fn)+":"+bits(own)+":"+
                bits(fnName)+":"+bits(fnLength)+":"+Object.getOwnPropertyNames(fn).join(",");
        }
        var tag=Object.getOwnPropertyDescriptor(Reflect,Symbol.toStringTag);
        return [
            "global="+(descriptor.value===Reflect)+":"+bits(descriptor)+":"+
                (Object.getPrototypeOf(Reflect)===Object.prototype)+":"+Object.isExtensible(Reflect)+":"+
                Object.prototype.toString.call(Reflect),
            "global-order="+Reflect.ownKeys(globalThis).filter(function(key){
                return implemented.indexOf(key)>=0;
            }).join(","),
            "keys="+Reflect.ownKeys(Reflect).map(keyText).join(","),
            "methods="+metadata.join("|"),
            "identity="+(Reflect===Reflect)+":"+(Reflect.get===Reflect.get)+":"+
                (Reflect.get!==Reflect.set)+":"+(Reflect.construct!==Reflect.apply),
            "tag="+tag.value+":"+bits(tag)
        ].join("\n");
    })()"#,
)];

const AUTO_INIT_CASES: &[(&str, &str)] = &[
    (
        "fresh global AutoInit can be deleted without materializing or recovering",
        r#"(function(){
            var keys=Object.getOwnPropertyNames(globalThis),present=keys.indexOf("Reflect")>=0;
            var deleted=delete globalThis.Reflect;
            return [present,deleted,"Reflect" in globalThis,typeof Reflect,
                Object.getOwnPropertyDescriptor(globalThis,"Reflect")===undefined].join(":");
        })()"#,
    ),
    (
        "descriptor lookup materializes one stable configurable global object",
        r#"(function(){
            var first=Object.getOwnPropertyDescriptor(globalThis,"Reflect"),value=first.value;
            var second=Object.getOwnPropertyDescriptor(globalThis,"Reflect");
            return [(value===Reflect),(second.value===value),(Reflect===Reflect),
                first.writable,first.enumerable,first.configurable,
                Object.getPrototypeOf(value)===Object.prototype].join(":");
        })()"#,
    ),
    (
        "method mutation and deletion persist and deleting the global does not resurrect it",
        r#"(function(){
            var saved=Reflect,originalGet=saved.get,marker=function marker(){return 17};
            saved.get=marker;
            var changed=saved.get===marker;
            var methodDelete=delete saved.construct;
            var methodMissing=!("construct" in saved)&&typeof saved.construct==="undefined";
            var globalDelete=delete globalThis.Reflect;
            var globalMissing=!("Reflect" in globalThis)&&typeof Reflect==="undefined";
            var detached=[saved.get===marker,saved.apply===saved.apply,
                !("construct" in saved),originalGet!==saved.get].join(":");
            globalThis.Reflect=saved;
            return [changed,methodDelete,methodMissing,globalDelete,globalMissing,detached,
                globalThis.Reflect===saved].join("|");
        })()"#,
    ),
    (
        "defineProperty can replace a fresh global AutoInit and its flags stay exact",
        r#"(function(){
            Object.defineProperty(globalThis,"Reflect",{
                value:17,writable:false,enumerable:true,configurable:true
            });
            var descriptor=Object.getOwnPropertyDescriptor(globalThis,"Reflect");
            return [Reflect,descriptor.value,descriptor.writable,descriptor.enumerable,
                descriptor.configurable,delete globalThis.Reflect,typeof Reflect].join(":");
        })()"#,
    ),
];

const APPLY_CONSTRUCT_CASES: &[(&str, &str)] = &[
    (
        "apply preserves this and materializes inherited sparse array-like arguments",
        r#"(function(){
            function target(a,b,c){
                return [this.marker,arguments.length,a,b,c,2 in arguments].join(":");
            }
            var prototype=Object();prototype[1]="B";
            var list=Object.create(prototype);list.length=3;list[0]="A";
            return Reflect.apply(target,{marker:"this"},list);
        })()"#,
    ),
    (
        "apply reads length then indexes left to right and preserves abrupt getters",
        r#"(function(){
            var log="",list=Object(),boom={token:"boom"};
            Object.defineProperty(list,"length",{get:function(){log+="l";return 3}});
            Object.defineProperty(list,"0",{get:function(){log+="0";return 20}});
            Object.defineProperty(list,"1",{get:function(){log+="1";return 22}});
            Object.defineProperty(list,"2",{get:function(){log+="2";throw boom}});
            var caught;
            try{Reflect.apply(function(){log+="b"},null,list)}catch(error){caught=error}
            list=Object();log+="|";
            Object.defineProperty(list,"length",{get:function(){log+="L";return 2}});
            Object.defineProperty(list,"0",{get:function(){log+="A";return 20}});
            Object.defineProperty(list,"1",{get:function(){log+="B";return 22}});
            var result=Reflect.apply(function(a,b){log+="C";return a+b},null,list);
            return [caught===boom,result,log].join(":");
        })()"#,
    ),
    (
        "apply validates callable target before touching argsList and rejects primitive lists",
        r#"(function(){
            var log="",poison=Object();
            Object.defineProperty(poison,"length",{get:function(){log+="length";throw "poison"}});
            var invalidTarget,primitiveList,missingList;
            try{Reflect.apply({},null,poison)}catch(error){invalidTarget=error.name}
            var afterTarget=log;
            try{Reflect.apply(function(){},null,"ab")}catch(error){primitiveList=error.name}
            try{Reflect.apply(function(){},null)}catch(error){missingList=error.name}
            return [invalidTarget,afterTarget,primitiveList,missingList,log].join(":");
        })()"#,
    ),
    (
        "construct supplies new.target and supports subclass-style custom prototypes",
        r#"(function(){
            function Base(a,b){this.sum=a+b;this.seen=new.target}
            function Derived(){}
            var prototype={kind:"derived"};Derived.prototype=prototype;
            var value=Reflect.construct(Base,[20,22],Derived);
            return [value.sum,value.seen===Derived,Object.getPrototypeOf(value)===prototype,
                value instanceof Derived,value instanceof Base,value.kind].join(":");
        })()"#,
    ),
    (
        "construct honors object returns and ignores primitive constructor returns",
        r#"(function(){
            var override={answer:42};
            function ReturnsObject(){this.local=1;return override}
            function ReturnsPrimitive(){this.answer=42;return 17}
            var first=Reflect.construct(ReturnsObject,[]),second=Reflect.construct(ReturnsPrimitive,[]);
            return [first===override,first.local,second.answer,
                Object.getPrototypeOf(second)===ReturnsPrimitive.prototype].join(":");
        })()"#,
    ),
    (
        "construct pins QuickJS newTarget argsList and target validation order",
        r#"(function(){
            var log="",poison=Object(),argumentError={kind:"args"};
            Object.defineProperty(poison,"length",{get:function(){log+="L";throw argumentError}});
            var bothInvalid,newTargetFirst,argsBeforeTarget,targetAfterList;
            try{Reflect.construct({},poison,null)}catch(error){bothInvalid=error.name}
            var afterBoth=log;
            try{Reflect.construct(function(){},poison,{})}catch(error){newTargetFirst=error.name}
            var afterNewTarget=log;
            try{Reflect.construct({},poison)}catch(error){argsBeforeTarget=error===argumentError}
            var afterArgs=log;
            try{Reflect.construct({},[])}catch(error){targetAfterList=error.name}
            return [bothInvalid,afterBoth,newTargetFirst,afterNewTarget,argsBeforeTarget,
                afterArgs,targetAfterList].join(":");
        })()"#,
    ),
    (
        "construct extracts arguments before newTarget prototype and constructor body",
        r#"(function(){
            var log="",list=Object(),prototype={realm:"custom"};
            Object.defineProperty(list,"length",{get:function(){log+="l";return 2}});
            Object.defineProperty(list,"0",{get:function(){log+="0";return 20}});
            Object.defineProperty(list,"1",{get:function(){log+="1";return 22}});
            function Target(a,b){log+="b";this.answer=a+b}
            var NewTarget=Target.bind(null);
            Object.defineProperty(NewTarget,"prototype",{
                get:function(){log+="p";return prototype},configurable:true
            });
            var value=Reflect.construct(Target,list,NewTarget);
            return [value.answer,Object.getPrototypeOf(value)===prototype,log].join(":");
        })()"#,
    ),
    (
        "construct rejects primitive lists and defaults newTarget to target",
        r#"(function(){
            function Target(a){this.value=a;this.same=new.target===Target}
            var primitive,missing;
            try{Reflect.construct(Target,"x")}catch(error){primitive=error.name}
            try{Reflect.construct(Target)}catch(error){missing=error.name}
            var value=Reflect.construct(Target,{0:42,length:1});
            return [primitive,missing,value.value,value.same,
                Object.getPrototypeOf(value)===Target.prototype].join(":");
        })()"#,
    ),
];

const PROPERTY_CASES: &[(&str, &str)] = &[
    (
        "defineProperty converts key then descriptor fields and returns booleans",
        r#"(function(){
            var log="",target=Object(),key=Object(),descriptor=Object();
            key[Symbol.toPrimitive]=function(hint){log+="k"+hint+",";return "answer"};
            Object.defineProperty(descriptor,"enumerable",{get:function(){log+="e,";return true}});
            Object.defineProperty(descriptor,"configurable",{get:function(){log+="c,";return true}});
            Object.defineProperty(descriptor,"value",{get:function(){log+="v,";return 42}});
            Object.defineProperty(descriptor,"writable",{get:function(){log+="w,";return false}});
            var created=Reflect.defineProperty(target,key,descriptor),own=Object.getOwnPropertyDescriptor(target,"answer");
            Object.preventExtensions(target);
            var newFailed=Reflect.defineProperty(target,"late",{value:1});
            var incompatible=Reflect.defineProperty(target,"answer",{writable:true});
            return [created,log,own.value,own.writable,own.enumerable,own.configurable,
                newFailed,incompatible,target.hasOwnProperty("late")].join(":");
        })()"#,
    ),
    (
        "defineProperty validates target before key and descriptor and preserves abrupt conversion",
        r#"(function(){
            var log="",key=Object(),descriptor=Object(),boom={kind:"descriptor"};
            key[Symbol.toPrimitive]=function(){log+="k";throw "key"};
            Object.defineProperty(descriptor,"enumerable",{get:function(){log+="d";throw boom}});
            var invalid,keyThrow,descriptorThrow,target=Object();
            try{Reflect.defineProperty(null,key,descriptor)}catch(error){invalid=error.name}
            var afterInvalid=log;
            try{Reflect.defineProperty(target,key,descriptor)}catch(error){keyThrow=error}
            key[Symbol.toPrimitive]=function(){log+="K";return "x"};
            try{Reflect.defineProperty(target,key,descriptor)}catch(error){descriptorThrow=error===boom}
            return [invalid,afterInvalid,keyThrow,descriptorThrow,log].join(":");
        })()"#,
    ),
    (
        "deleteProperty handles strings symbols missing and nonconfigurable properties",
        r#"(function(){
            var target=Object(),symbol=Symbol("delete");target.open=1;target[symbol]=2;
            Object.defineProperty(target,"fixed",{value:3,configurable:false});
            var a=Reflect.deleteProperty(target,"open"),b=Reflect.deleteProperty(target,symbol);
            var c=Reflect.deleteProperty(target,"missing"),d=Reflect.deleteProperty(target,"fixed");
            return [a,b,c,d,target.hasOwnProperty("open"),target.hasOwnProperty(symbol),
                target.fixed].join(":");
        })()"#,
    ),
    (
        "get and has use property keys prototypes accessors and explicit receivers",
        r#"(function(){
            var log="",symbol=Symbol("key"),prototype=Object(),target=Object.create(prototype);
            prototype.inherited=20;target[symbol]=22;
            Object.defineProperty(prototype,"access",{get:function(){log+=this.marker;return this.value}});
            var receiver={marker:"r",value:42};
            var key=Object();key[Symbol.toPrimitive]=function(hint){log+=hint;return "inherited"};
            return [Reflect.get(target,"inherited"),Reflect.get(target,symbol),
                Reflect.get(target,"access",receiver),Reflect.get(target,key),
                Reflect.has(target,"inherited"),Reflect.has(target,symbol),
                Reflect.has(target,"missing"),log].join(":");
        })()"#,
    ),
    (
        "getOwnPropertyDescriptor returns fresh exact descriptors for string and symbol keys",
        r#"(function(){
            var target=Object(),payload=Object(),symbol=Symbol("own"),calls=0;
            Object.defineProperty(target,"data",{value:payload,writable:false,enumerable:true,configurable:false});
            var getter=function(){calls++;return 7};
            Object.defineProperty(target,symbol,{get:getter,set:undefined,enumerable:false,configurable:true});
            var first=Reflect.getOwnPropertyDescriptor(target,"data"),second=Reflect.getOwnPropertyDescriptor(target,"data");
            var access=Reflect.getOwnPropertyDescriptor(target,symbol);
            return [Object.getOwnPropertyNames(first).join(","),first.value===payload,first.writable,
                first.enumerable,first.configurable,first!==second,
                Object.getOwnPropertyNames(access).join(","),access.get===getter,
                access.set===undefined,access.enumerable,access.configurable,calls,
                Reflect.getOwnPropertyDescriptor(target,"missing")===undefined].join(":");
        })()"#,
    ),
    (
        "getPrototypeOf and setPrototypeOf preserve null and report failed changes",
        r#"(function(){
            var first=Object(),second=Object(),target=Object.create(first);
            var changed=Reflect.setPrototypeOf(target,second),after=Reflect.getPrototypeOf(target)===second;
            var toNull=Reflect.setPrototypeOf(target,null),nullProto=Reflect.getPrototypeOf(target)===null;
            var restored=Reflect.setPrototypeOf(target,first);Object.preventExtensions(target);
            var same=Reflect.setPrototypeOf(target,first),failed=Reflect.setPrototypeOf(target,second);
            var badPrototype;
            try{Reflect.setPrototypeOf(target,17)}catch(error){badPrototype=error.name}
            return [changed,after,toNull,nullProto,restored,same,failed,
                Reflect.getPrototypeOf(target)===first,badPrototype].join(":");
        })()"#,
    ),
    (
        "isExtensible and preventExtensions require objects and return booleans",
        r#"(function(){
            var target=Object(),before=Reflect.isExtensible(target),prevented=Reflect.preventExtensions(target);
            var after=Reflect.isExtensible(target),again=Reflect.preventExtensions(target);
            var isPrimitive,preventPrimitive;
            try{Reflect.isExtensible(1)}catch(error){isPrimitive=error.name}
            try{Reflect.preventExtensions(null)}catch(error){preventPrimitive=error.name}
            return [before,prevented,after,again,isPrimitive,preventPrimitive].join(":");
        })()"#,
    ),
    (
        "ownKeys returns canonical own string and symbol order including nonenumerables",
        r#"(function(){
            var first=Symbol("first"),second=Symbol("second"),prototype={inherited:1};
            var target=Object.create(prototype);target.z=1;target[10]=2;target[first]=3;target[2]=4;target.a=5;
            Object.defineProperty(target,"hidden",{value:6,enumerable:false});target[second]=7;
            var keys=Reflect.ownKeys(target),out=[];
            for(var i=0;i<keys.length;i++)out[i]=typeof keys[i]==="symbol"?
                (keys[i]===first?"first":"second"):keys[i];
            return [out.join(","),Array.isArray(keys),Object.getPrototypeOf(keys)===Array.prototype,
                keys.indexOf("inherited")<0].join(":");
        })()"#,
    ),
    (
        "set uses receiver accessors symbols and returns false for rejected writes",
        r#"(function(){
            var log="",symbol=Symbol("set"),prototype=Object(),target=Object.create(prototype);
            var receiver={marker:"R"};
            Object.defineProperty(prototype,"access",{set:function(value){log+=this.marker+value}});
            Object.defineProperty(prototype,"readonly",{value:1,writable:false});
            Object.defineProperty(prototype,"data",{value:2,writable:true});
            var accessor=Reflect.set(target,"access",42,receiver);
            var inheritedData=Reflect.set(target,"data",20,receiver);
            var readOnly=Reflect.set(target,"readonly",9,receiver);
            var symbolSet=Reflect.set(target,symbol,22);
            Object.preventExtensions(target);
            var late=Reflect.set(target,"late",1);
            return [accessor,log,inheritedData,receiver.data,target.hasOwnProperty("data"),
                readOnly,receiver.hasOwnProperty("readonly"),symbolSet,target[symbol],late,
                target.hasOwnProperty("late")].join(":");
        })()"#,
    ),
    (
        "keyed methods validate target before converting property keys",
        r#"(function(){
            var names=["deleteProperty","get","getOwnPropertyDescriptor","has","set"],out=[],log="";
            for(var i=0;i<names.length;i++){
                var key=Object();key[Symbol.toPrimitive]=function(){log+="k";throw "key"};
                try{Reflect[names[i]](null,key,1)}catch(error){out[out.length]=error.name+":"+log}
            }
            return out.join("|");
        })()"#,
    ),
];

#[test]
fn reflect_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Reflect oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("AutoInit", AUTO_INIT_CASES),
        ("apply/construct", APPLY_CONSTRUCT_CASES),
        ("properties", PROPERTY_CASES),
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
fn reflect_graph_and_native_metadata_match_pinned_quickjs() {
    compare_cases("Reflect graph", GRAPH_CASES);
}

#[test]
fn reflect_autoinit_mutation_and_deletion_match_pinned_quickjs() {
    compare_cases("Reflect AutoInit", AUTO_INIT_CASES);
}

#[test]
fn reflect_apply_and_construct_match_pinned_quickjs() {
    compare_cases("Reflect apply/construct", APPLY_CONSTRUCT_CASES);
}

#[test]
fn reflect_property_operations_match_pinned_quickjs() {
    compare_cases("Reflect property operations", PROPERTY_CASES);
}

#[test]
fn reflect_exposes_all_thirteen_nonconstructable_methods() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reflect = reflect_object(&runtime, &mut context);
    for &(name, expected_length) in METHODS {
        let callable = property_callable(&runtime, &mut context, &reflect, name);
        assert_eq!(
            number_property(&runtime, &mut context, callable.as_object(), "length"),
            expected_length as f64,
            "Reflect.{name}.length drifted",
        );
        assert_eq!(
            string_property(&runtime, &mut context, callable.as_object(), "name"),
            name,
            "Reflect.{name}.name drifted",
        );
        assert!(
            !runtime.is_constructor(callable.as_object()).unwrap(),
            "Reflect.{name} unexpectedly carried the constructor bit",
        );
        assert_eq!(
            context.construct(&callable, &[]),
            Err(RuntimeError::Exception)
        );
        take_exception_object(&mut context, &format!("new Reflect.{name}"));
    }
}

#[test]
fn reflect_cross_realm_results_native_errors_and_user_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let reflect = reflect_object(&runtime, &mut defining);
    let descriptor_method = property_callable(
        &runtime,
        &mut defining,
        &reflect,
        "getOwnPropertyDescriptor",
    );
    let own_keys = property_callable(&runtime, &mut defining, &reflect, "ownKeys");
    let get = property_callable(&runtime, &mut defining, &reflect, "get");

    let defining_object_prototype = defining.object_prototype().unwrap();
    let defining_array_prototype = eval_object(&mut defining, "Array.prototype", "Array prototype");
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

    let target = eval_object(&mut caller, "({answer:42})", "caller target");
    let descriptor = expect_object(
        caller
            .call(
                &descriptor_method,
                Value::Undefined,
                &[
                    Value::Object(target.clone()),
                    Value::String(JsString::try_from_utf8("answer").unwrap()),
                ],
            )
            .unwrap(),
        "cross-realm Reflect.getOwnPropertyDescriptor result",
    );
    assert_eq!(
        runtime.get_prototype_of(&descriptor).unwrap(),
        Some(defining_object_prototype),
        "descriptor object used the calling realm",
    );

    let keys = expect_object(
        caller
            .call(
                &own_keys,
                Value::Undefined,
                &[Value::Object(target.clone())],
            )
            .unwrap(),
        "cross-realm Reflect.ownKeys result",
    );
    assert_eq!(
        runtime.get_prototype_of(&keys).unwrap(),
        Some(defining_array_prototype),
        "ownKeys Array used the calling realm",
    );

    assert_eq!(
        caller.call(
            &get,
            Value::Undefined,
            &[
                Value::Int(1),
                Value::String(JsString::try_from_utf8("x").unwrap()),
            ],
        ),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "cross-realm Reflect.get TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "native target validation error used the calling realm",
    );

    let throwing_key = eval_object(
        &mut caller,
        r#"(function(){
            var key=Object();
            key[Symbol.toPrimitive]=function(){throw new TypeError("caller key")};
            return key;
        })()"#,
        "caller throwing property key",
    );
    assert_eq!(
        caller.call(
            &get,
            Value::Undefined,
            &[Value::Object(target), Value::Object(throwing_key)],
        ),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller property-key TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error.clone()),
        "Reflect.get replaced a user-thrown caller-realm error",
    );

    assert_eq!(caller.construct(&get, &[]), Err(RuntimeError::Exception));
    let constructor_error = take_exception_object(&mut caller, "new foreign Reflect.get");
    assert_eq!(
        runtime.get_prototype_of(&constructor_error).unwrap(),
        Some(caller_type_error),
        "generic non-constructor rejection did not use the caller realm",
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
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle(&oracle, source, description),
            "{group} drifted for {description}: {source:?}",
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
        Err(error) => panic!("Rust engine failure for {description} ({source:?}): {error}"),
    }
}

fn observe_oracle(oracle: &OsStr, source: &str, description: &str) -> String {
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

fn reflect_object(runtime: &Runtime, context: &mut Context) -> ObjectRef {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("Reflect").unwrap();
    let Value::Object(reflect) = context.get_property(&global, &key).unwrap() else {
        panic!("global Reflect was not an object");
    };
    reflect
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

fn number_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> f64 {
    let key = runtime.intern_property_key(name).unwrap();
    context
        .get_property(object, &key)
        .unwrap_or_else(|error| panic!("read numeric property {name}: {error}"))
        .as_number()
        .unwrap_or_else(|| panic!("{name} was not numeric"))
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
