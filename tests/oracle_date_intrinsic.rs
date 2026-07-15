use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CallableRef, Context, ObjectRef, Runtime, RuntimeError, Value};

// Differential lock for the complete observable Date slice in pinned QuickJS
// 2026-06-04. Every JavaScript vector stays inside quickjs-oxide's currently
// implemented grammar; in particular it avoids classes, arrow functions,
// destructuring, regular expressions, and async/module syntax.

const PRELUDE: &str = r#"
function __bit(value){return value?"1":"0"}
function __bits(object,key){
    var descriptor=Object.getOwnPropertyDescriptor(object,key);
    if(descriptor===undefined)return "missing";
    return __bit(descriptor.writable)+__bit(descriptor.enumerable)+__bit(descriptor.configurable);
}
function __isConstructor(value){
    try{Reflect.construct(function(){},[],value);return true}catch(_error){return false}
}
function __show(value){
    if(value!==value)return "NaN";
    if(value===0)return 1/value===-Infinity?"-0":"+0";
    if(value===Infinity)return "+Infinity";
    if(value===-Infinity)return "-Infinity";
    return String(value);
}
function __error(thunk){
    try{return "return:"+String(thunk())}
    catch(error){
        if(error!==null&&typeof error==="object")return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
function __box(log,name,value){
    var object=Object();
    object.valueOf=function(){log.text+=name;return value};
    return object;
}
"#;

const GRAPH_CASES: &[(&str, &str)] = &[
    (
        "global constructor prototype keys aliases tags and descriptors",
        r#"(function(){
            var selected=["Number","Boolean","String","Math","Reflect","Symbol",
                "globalThis","BigInt","Date"];
            var globalKeys=Object.getOwnPropertyNames(globalThis),filtered=[],i;
            for(i=0;i<globalKeys.length;i++)
                if(selected.indexOf(globalKeys[i])>=0)filtered[filtered.length]=globalKeys[i];
            var symbols=Object.getOwnPropertySymbols(Date.prototype),symbolNames=[];
            for(i=0;i<symbols.length;i++)symbolNames[i]=String(symbols[i]);
            return [
                "global="+__bits(globalThis,"Date")+":"+
                    (Object.getOwnPropertyDescriptor(globalThis,"Date").value===Date),
                "global-order="+filtered.join(","),
                "constructor-keys="+Object.getOwnPropertyNames(Date).join(","),
                "prototype-keys="+Object.getOwnPropertyNames(Date.prototype).join(","),
                "prototype-symbols="+symbolNames.join(","),
                "links="+(Object.getPrototypeOf(Date)===Function.prototype)+":"+
                    (Object.getPrototypeOf(Date.prototype)===Object.prototype)+":"+
                    (Date.prototype.constructor===Date),
                "brands="+Object.prototype.toString.call(Date.prototype)+":"+
                    Object.prototype.toString.call(new Date(0)),
                "alias="+(Date.prototype.toGMTString===Date.prototype.toUTCString)+":"+
                    Date.prototype.toGMTString.name,
                "tags="+(Object.getOwnPropertyDescriptor(Date.prototype,Symbol.toStringTag)===undefined)+":"+
                    (symbols.length===1&&symbols[0]===Symbol.toPrimitive),
                "descriptors="+__bits(Date,"prototype")+":"+
                    __bits(Date.prototype,"constructor")+":"+
                    __bits(Date.prototype,"toString")+":"+
                    __bits(Date.prototype,Symbol.toPrimitive)
            ].join("\n");
        })()"#,
    ),
    (
        "all Date callables expose pinned names lengths flags and constructor bits",
        r#"(function(){
            var statics=[["now",0],["parse",1],["UTC",7]];
            var methods=[
                ["valueOf",0],["toString",0],["toUTCString",0],["toGMTString",0],
                ["toISOString",0],["toDateString",0],["toTimeString",0],
                ["toLocaleString",0],["toLocaleDateString",0],["toLocaleTimeString",0],
                ["getTimezoneOffset",0],["getTime",0],["getYear",0],["getFullYear",0],
                ["getUTCFullYear",0],["getMonth",0],["getUTCMonth",0],["getDate",0],
                ["getUTCDate",0],["getHours",0],["getUTCHours",0],["getMinutes",0],
                ["getUTCMinutes",0],["getSeconds",0],["getUTCSeconds",0],
                ["getMilliseconds",0],["getUTCMilliseconds",0],["getDay",0],["getUTCDay",0],
                ["setTime",1],["setMilliseconds",1],["setUTCMilliseconds",1],
                ["setSeconds",2],["setUTCSeconds",2],["setMinutes",3],["setUTCMinutes",3],
                ["setHours",4],["setUTCHours",4],["setDate",1],["setUTCDate",1],
                ["setMonth",2],["setUTCMonth",2],["setYear",1],
                ["setFullYear",3],["setUTCFullYear",3],["toJSON",1]
            ];
            function metadata(owner,entry){
                var key=entry[0],fn=owner[key];
                return key+":"+fn.name+":"+fn.length+":"+(fn.length===entry[1])+":"+
                    __isConstructor(fn)+":"+__bits(owner,key)+":"+
                    __bits(fn,"name")+":"+__bits(fn,"length")+":"+
                    Object.getOwnPropertyNames(fn).join(",");
            }
            var output=["Date:"+Date.name+":"+Date.length+":"+__isConstructor(Date)+":"+
                __bits(Date,"name")+":"+__bits(Date,"length")],i;
            for(i=0;i<statics.length;i++)output[output.length]=metadata(Date,statics[i]);
            for(i=0;i<methods.length;i++)output[output.length]=metadata(Date.prototype,methods[i]);
            var primitive=Date.prototype[Symbol.toPrimitive];
            output[output.length]="@@toPrimitive:"+primitive.name+":"+primitive.length+":"+
                __isConstructor(primitive)+":"+__bits(primitive,"name")+":"+
                __bits(primitive,"length")+":"+Object.getOwnPropertyNames(primitive).join(",");
            return output.join("|");
        })()"#,
    ),
    (
        "ordinary Date prototype fails branded value formatter getter and setter methods",
        r#"(function(){
            var names=["valueOf","getTime","toString","toISOString","getFullYear","setTime"],
                output=[],i;
            for(i=0;i<names.length;i++){
                var name=names[i];
                output[i]=name+":"+__error(function(){
                    return Date.prototype[name].call(Date.prototype,0);
                });
            }
            output[output.length]="ordinary="+
                Date.prototype[Symbol.toPrimitive].call(Date.prototype,"default");
            return output.join("|");
        })()"#,
    ),
];

const CONSTRUCTOR_CASES: &[(&str, &str)] = &[
    (
        "zero argument construct and function call have stable observable shape",
        r#"(function(){
            var constructed=new Date(),now=Date.now(),hit=false,bomb=Object();
            bomb[Symbol.toPrimitive]=function(){hit=true;throw "coerced"};
            var text=Date(bomb,bomb);
            return [typeof constructed.getTime(),Number.isFinite(constructed.getTime()),
                Object.getPrototypeOf(constructed)===Date.prototype,
                Math.abs(constructed.getTime()-now)<10000,
                typeof text,text.length>20,text.indexOf("GMT")>=0,hit].join(":");
        })()"#,
    ),
    (
        "one argument forms distinguish primitive conversion string parsing and genuine clone",
        r#"(function(){
            var log="",stringObject=Object(),numberObject=Object();
            stringObject[Symbol.toPrimitive]=function(hint){log+="s:"+hint+";";return "1970-01-01T00:00:01.234Z"};
            numberObject[Symbol.toPrimitive]=function(hint){log+="n:"+hint+";";return 42};
            var first=new Date(stringObject),second=new Date(numberObject),source=new Date(5678),hit=false;
            source[Symbol.toPrimitive]=function(){hit=true;throw "clone conversion"};
            source.valueOf=function(){hit=true;throw "clone valueOf"};
            var clone=new Date(source);
            return [first.getTime(),second.getTime(),clone.getTime(),hit,log,
                Object.getPrototypeOf(clone)===Date.prototype].join(":");
        })()"#,
    ),
    (
        "multi argument construction coerces seven fields left to right and ignores extras",
        r#"(function(){
            var log={text:""},ignored=Object();
            ignored.valueOf=function(){log.text+="X";throw "ignored"};
            var date=new Date(
                __box(log,"y",2000),__box(log,"m",1),__box(log,"d",29),
                __box(log,"h",23),__box(log,"i",58),__box(log,"s",57),
                __box(log,"x",123),ignored
            );
            return [date.getFullYear(),date.getMonth(),date.getDate(),date.getHours(),
                date.getMinutes(),date.getSeconds(),date.getMilliseconds(),log.text,
                Number.isFinite(date.getTime())].join(":");
        })()"#,
    ),
    (
        "multi argument conversion completes before newTarget prototype observation",
        r#"(function(){
            var log={text:""},custom={kind:"custom"};
            function NewTarget(){}
            Object.defineProperty(NewTarget,"prototype",{
                configurable:true,get:function(){log.text+="p";return custom}
            });
            var date=Reflect.construct(Date,[__box(log,"a",2001),__box(log,"b",2)],NewTarget);
            return [log.text,Object.getPrototypeOf(date)===custom,date.getFullYear(),date.getMonth()].join(":");
        })()"#,
    ),
    (
        "custom and primitive newTarget prototypes select custom and realm fallback prototypes",
        r#"(function(){
            var custom={marker:42};
            function Custom(){}Custom.prototype=custom;
            function Fallback(){}Fallback.prototype=17;
            var first=Reflect.construct(Date,[0],Custom),second=Reflect.construct(Date,[0],Fallback);
            return [Object.getPrototypeOf(first)===custom,first.marker,
                Object.getPrototypeOf(second)===Date.prototype,second.getTime()].join(":");
        })()"#,
    ),
];

const STATIC_CASES: &[(&str, &str)] = &[
    (
        "Date parse accepts pinned ISO legacy and explicit offset forms",
        r#"(function(){
            var values=[
                Date.parse("1970-01-01T00:00:00.000Z"),
                Date.parse("1970-01-01"),
                Date.parse("1970-01-01T00:00:00+08:00"),
                Date.parse("Thu, 01 Jan 1970 00:00:00 GMT"),
                Date.parse("not a date"),
                Date.parse("+275760-09-13T00:00:00.000-01:00")
            ],output=[],i;
            for(i=0;i<values.length;i++)output[i]=__show(values[i]);
            output[output.length]=__show(new Date(values[5]).getTime());
            return output.join("|");
        })()"#,
    ),
    (
        "Date UTC defaults legacy years limits and conversion order match",
        r#"(function(){
            var log={text:""},ignored=Object();
            ignored.valueOf=function(){log.text+="X";throw "ignored"};
            var ordered=Date.UTC(
                __box(log,"y",2000),__box(log,"m",1),__box(log,"d",29),
                __box(log,"h",23),__box(log,"i",58),__box(log,"s",57),
                __box(log,"x",123),ignored
            );
            var values=[Date.UTC(),Date.UTC(70),Date.UTC(99,11,31,23,59,59,999),
                Date.UTC(275760,8,13),Date.UTC(275760,8,13,0,0,0,1),ordered],output=[],i;
            for(i=0;i<values.length;i++)output[i]=__show(values[i]);
            output[output.length]=log.text;
            return output.join("|");
        })()"#,
    ),
    (
        "TimeClip endpoints signed zero and nonfinite inputs match",
        r#"(function(){
            var values=[new Date(8640000000000000).getTime(),new Date(-8640000000000000).getTime(),
                new Date(8640000000000001).getTime(),new Date(-8640000000000001).getTime(),
                new Date(-0).getTime(),new Date(-0.9).getTime(),new Date(Infinity).getTime(),
                new Date(NaN).getTime()],output=[],i;
            for(i=0;i<values.length;i++)output[i]=__show(values[i]);
            return output.join("|");
        })()"#,
    ),
    (
        "Date now and fixed local timezone offsets avoid wall clock equality assumptions",
        r#"(function(){
            var now=Date.now(),roundtrip=new Date(now).getTime(),fresh=new Date().getTime();
            var offsets=[new Date(0).getTimezoneOffset(),new Date(1609459200000).getTimezoneOffset(),
                new Date(1625097600000).getTimezoneOffset()];
            return [typeof now,now===Math.trunc(now),roundtrip===now,
                Math.abs(fresh-now)<10000,offsets.join(","),
                Date.parse("1970-01-01T00:00:00")===new Date(1970,0,1).getTime()].join("|");
        })()"#,
    ),
];

const STRING_GETTER_CASES: &[(&str, &str)] = &[
    (
        "all eight string methods format one fixed instant",
        r#"(function(){
            var date=new Date(Date.UTC(2018,0,2,23,4,6,927));
            var names=["toString","toUTCString","toISOString","toDateString","toTimeString",
                "toLocaleString","toLocaleDateString","toLocaleTimeString"],output=[],i;
            for(i=0;i<names.length;i++)output[i]=names[i]+"="+date[names[i]]();
            return output.join("|");
        })()"#,
    ),
    (
        "invalid dates stringify or throw at the pinned formatter boundary",
        r#"(function(){
            var date=new Date(NaN),names=["toString","toUTCString","toISOString","toDateString",
                "toTimeString","toLocaleString","toLocaleDateString","toLocaleTimeString"],
                output=[],i;
            for(i=0;i<names.length;i++){
                var name=names[i];
                output[i]=name+":"+__error(function(){return date[name]()});
            }
            return output.join("|");
        })()"#,
    ),
    (
        "all UTC local and legacy getters expose the pinned field vector",
        r#"(function(){
            var date=new Date(Date.UTC(2000,1,29,23,58,57,123));
            var names=["getYear","getFullYear","getUTCFullYear","getMonth","getUTCMonth",
                "getDate","getUTCDate","getHours","getUTCHours","getMinutes","getUTCMinutes",
                "getSeconds","getUTCSeconds","getMilliseconds","getUTCMilliseconds",
                "getDay","getUTCDay","getTimezoneOffset","getTime","valueOf"],output=[],i;
            for(i=0;i<names.length;i++)output[i]=names[i]+"="+__show(date[names[i]]());
            return output.join("|");
        })()"#,
    ),
];

const SETTER_CASES: &[(&str, &str)] = &[
    (
        "setter field windows coerce left to right and ignore extra arguments",
        r#"(function(){
            var date=new Date(Date.UTC(2000,0,2,3,4,5,6)),log={text:""},ignored=Object();
            ignored.valueOf=function(){log.text+="X";throw "ignored"};
            var result=date.setUTCHours(__box(log,"h",10),__box(log,"m",11),
                __box(log,"s",12),__box(log,"x",13),ignored);
            return [__show(result),date.toISOString(),log.text].join("|");
        })()"#,
    ),
    (
        "nonfinite setter input still converts later fields and invalidates the receiver",
        r#"(function(){
            var date=new Date(0),log={text:""};
            var result=date.setUTCMinutes(__box(log,"a",NaN),__box(log,"b",3),__box(log,"c",4));
            return [__show(result),__show(date.getTime()),log.text].join("|");
        })()"#,
    ),
    (
        "abrupt setter conversion leaves the original time value unchanged",
        r#"(function(){
            var date=new Date(Date.UTC(2001,2,4,5,6,7,8)),before=date.getTime(),log="",boom={token:"boom"};
            var first=Object(),second=Object();
            first.valueOf=function(){log+="a";return 20};
            second.valueOf=function(){log+="b";throw boom};
            var caught;
            try{date.setUTCSeconds(first,second)}catch(error){caught=error===boom}
            return [caught,log,date.getTime()===before,date.toISOString()].join("|");
        })()"#,
    ),
    (
        "invalid Date recovery distinguishes full year legacy year and other setters",
        r#"(function(){
            var utc=new Date(NaN),month=new Date(NaN),legacy=new Date(NaN),log={text:""};
            var utcResult=utc.setUTCFullYear(2000),monthResult=month.setUTCMonth(__box(log,"m",1));
            var legacyResult=legacy.setYear(99);
            return [__show(utcResult),utc.toISOString(),__show(monthResult),
                __show(month.getTime()),log.text,__show(legacyResult),legacy.getFullYear(),
                legacy.getMonth(),legacy.getDate()].join("|");
        })()"#,
    ),
    (
        "brand checks precede coercion and zero argument generic setters store NaN",
        r#"(function(){
            var hit=false,bomb=Object();
            bomb.valueOf=function(){hit=true;throw "coerced"};
            var branded=__error(function(){return Date.prototype.setTime.call({},bomb)});
            var date=new Date(0),result=date.setUTCSeconds();
            return [branded,hit,__show(result),__show(date.getTime())].join("|");
        })()"#,
    ),
    (
        "generic setters snapshot before conversion while setYear re-reads after conversion",
        r#"(function(){
            var generic=new Date(Date.UTC(2000,5,15,12)),year=new Date(Date.UTC(2000,5,15,12));
            var genericArg=Object(),yearArg=Object();
            genericArg.valueOf=function(){generic.setTime(Date.UTC(2010,0,1));return 2001};
            yearArg.valueOf=function(){year.setTime(Date.UTC(2010,0,1));return 2001};
            generic.setUTCFullYear(genericArg);year.setYear(yearArg);
            return [generic.toISOString(),year.toISOString()].join("|");
        })()"#,
    ),
];

const PRIMITIVE_JSON_CASES: &[(&str, &str)] = &[
    (
        "Date Symbol toPrimitive maps default string number and integer hints",
        r#"(function(){
            var method=Date.prototype[Symbol.toPrimitive],log="";
            function object(){
                var value=Object();
                value.toString=function(){log+="s";return "text"};
                value.valueOf=function(){log+="v";return 42};
                return value;
            }
            var first=method.call(object(),"default"),a=log;log="";
            var second=method.call(object(),"string"),b=log;log="";
            var third=method.call(object(),"number"),c=log;log="";
            var fourth=method.call(object(),"integer"),d=log;
            return [first,a,second,b,third,c,fourth,d].join("|");
        })()"#,
    ),
    (
        "Date Symbol toPrimitive rejects invalid hints and primitive receivers",
        r#"(function(){
            var method=Date.prototype[Symbol.toPrimitive];
            return [__error(function(){return method.call({},"invalid")}),
                __error(function(){return method.call({},undefined)}),
                __error(function(){return method.call(1,"number")})].join("|");
        })()"#,
    ),
    (
        "ordinary Date coercion uses string default and numeric number hints",
        r#"(function(){
            var date=new Date(0);
            return [String(date),__show(+date),date==String(date),date==0].join("|");
        })()"#,
    ),
    (
        "generic toJSON handles nonfinite values and invokes toISOString with exact receiver",
        r#"(function(){
            var log="",finite=Object(),infinite=Object();
            finite.valueOf=function(){log+="v";return 1};
            finite.toISOString=function(){log+="i:"+(this===finite)+":"+arguments.length;return "ok"};
            infinite.valueOf=function(){log+="n";return Infinity};
            infinite.toISOString=function(){log+="BAD";throw "unreachable"};
            var first=Date.prototype.toJSON.call(finite,"key");
            var second=Date.prototype.toJSON.call(infinite,"key");
            return [first,second,log,new Date(0).toJSON(),new Date(NaN).toJSON()].join("|");
        })()"#,
    ),
    (
        "generic toJSON preserves abrupt conversion getter and call failures",
        r#"(function(){
            var conversion=Object(),getter=Object(),noncallable=Object(),boom={kind:"boom"};
            conversion.valueOf=function(){throw boom};
            Object.defineProperty(getter,"toISOString",{get:function(){throw boom}});
            noncallable.valueOf=function(){return 1};noncallable.toISOString=17;
            function observe(value){
                try{Date.prototype.toJSON.call(value);return "return"}
                catch(error){return error===boom?"boom":error.name+":"+error.message}
            }
            return [observe(conversion),observe(getter),observe(noncallable),
                __error(function(){return Date.prototype.toJSON.call(null)})].join("|");
        })()"#,
    ),
];

#[test]
fn date_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Date oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("graph", GRAPH_CASES),
        ("constructor", CONSTRUCTOR_CASES),
        ("statics", STATIC_CASES),
        ("strings/getters", STRING_GETTER_CASES),
        ("setters", SETTER_CASES),
        ("primitive/JSON", PRIMITIVE_JSON_CASES),
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
fn date_graph_and_native_metadata_match_pinned_quickjs() {
    compare_cases("Date graph", GRAPH_CASES);
}

#[test]
fn date_constructor_forms_match_pinned_quickjs() {
    compare_cases("Date constructor", CONSTRUCTOR_CASES);
}

#[test]
fn date_statics_timeclip_and_timezone_match_pinned_quickjs() {
    compare_cases("Date statics", STATIC_CASES);
}

#[test]
fn date_string_methods_and_getters_match_pinned_quickjs() {
    compare_cases("Date strings/getters", STRING_GETTER_CASES);
}

#[test]
fn date_setter_order_recovery_and_mutation_match_pinned_quickjs() {
    compare_cases("Date setters", SETTER_CASES);
}

#[test]
fn date_to_primitive_and_to_json_match_pinned_quickjs() {
    compare_cases("Date primitive/JSON", PRIMITIVE_JSON_CASES);
}

#[test]
fn date_cross_realm_prototypes_fallback_and_errors_use_exact_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();

    let defining_constructor = date_constructor(&runtime, &mut defining);
    let defining_date_prototype =
        eval_object(&mut defining, "Date.prototype", "defining Date prototype");
    let caller_date_prototype = eval_object(&mut caller, "Date.prototype", "caller Date prototype");
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
    assert_ne!(defining_date_prototype, caller_date_prototype);
    assert_ne!(defining_type_error, caller_type_error);

    let foreign_date = expect_object(
        caller
            .construct(&defining_constructor, &[Value::Int(42)])
            .unwrap(),
        "foreign Date construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&foreign_date).unwrap(),
        Some(defining_date_prototype.clone()),
        "Date construction did not use the foreign constructor prototype",
    );

    let caller_new_target = eval_callable(
        &runtime,
        &mut caller,
        "(function(){function F(){}F.prototype=17;return F})()",
        "caller primitive-prototype newTarget",
    );
    let fallback_date = expect_object(
        caller
            .construct_with_new_target(&defining_constructor, &caller_new_target, &[Value::Int(7)])
            .unwrap(),
        "cross-realm Date fallback construction",
    );
    assert_eq!(
        runtime.get_prototype_of(&fallback_date).unwrap(),
        Some(caller_date_prototype),
        "primitive newTarget.prototype did not fall back to the newTarget realm",
    );

    let value_of = date_prototype_callable(&runtime, &mut defining, "valueOf");
    let caller_date = eval_object(&mut caller, "new Date(123)", "caller Date");
    assert_eq!(
        caller
            .call(&value_of, Value::Object(caller_date), &[])
            .unwrap()
            .as_number(),
        Some(123.0),
        "foreign Date valueOf rejected a genuine same-runtime Date",
    );

    let ordinary = eval_object(&mut caller, "({})", "caller ordinary object");
    assert_eq!(
        caller.call(&value_of, Value::Object(ordinary), &[]),
        Err(RuntimeError::Exception),
    );
    let native_error = take_exception_object(&mut caller, "foreign Date valueOf TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Date branded native error used the calling realm",
    );

    let to_json = date_prototype_callable(&runtime, &mut defining, "toJSON");
    let throwing = eval_object(
        &mut caller,
        r#"(function(){
            var value=Object();
            value.valueOf=function(){throw new TypeError("caller conversion")};
            return value;
        })()"#,
        "caller throwing toJSON receiver",
    );
    assert_eq!(
        caller.call(&to_json, Value::Object(throwing), &[]),
        Err(RuntimeError::Exception),
    );
    let user_error = take_exception_object(&mut caller, "caller toJSON conversion TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Date.toJSON replaced a caller-realm user error",
    );
}

#[test]
fn detached_date_and_native_method_keep_their_defining_realm_collectable() {
    let runtime = Runtime::new();
    let (date, value_of) = {
        let mut defining = runtime.new_context();
        let date = eval_object(&mut defining, "new Date(42)", "detached Date");
        let value_of = date_prototype_callable(&runtime, &mut defining, "valueOf");
        (date, value_of)
    };

    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "detached Date roots must retain their defining realm graph",
    );

    {
        let mut caller = runtime.new_context();
        assert_eq!(
            caller
                .call(&value_of, Value::Object(date.clone()), &[])
                .unwrap()
                .as_number(),
            Some(42.0),
            "detached Date and valueOf stopped working after their Context dropped",
        );
    }

    drop(value_of);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().context_nodes,
        1,
        "the detached Date must retain its prototype realm after valueOf drops",
    );

    drop(date);
    runtime.run_gc().unwrap();
    assert_eq!(
        runtime.heap_counts().live,
        0,
        "Date realms, prototypes, native functions, and instances must be collectable",
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

fn date_constructor(runtime: &Runtime, context: &mut Context) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, "Date")
}

fn date_prototype_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let prototype = eval_object(context, "Date.prototype", "Date prototype");
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
