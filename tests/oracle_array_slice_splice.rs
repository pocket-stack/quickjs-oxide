use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This target pins QuickJS 2026-06-04 `js_array_slice`, its magic-selected
// mutating `splice` branch, and the adjacent dense `js_array_toSpliced` path.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "slice clamps positive and negative bounds without changing its source",
        r#"(function(){
            var source=[0,1,2,3,4],all=source.slice(),middle=source.slice(1,-1);
            var tail=source.slice(-2,99),empty=source.slice(4,1);
            return all.join("")+"|"+middle.join("")+"|"+tail.join("")+"|"+
                empty.length+"|"+source.join("");
        })()"#,
    ),
    (
        "slice and splice preserve holes while copying inherited values as own data",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            Array.prototype[1]="proto";
            var first=Array(4);first[0]="a";first[3]="d";
            var sliced=first.slice(0,4);
            var second=Array(4);second[0]="a";second[3]="d";
            var removed=second.splice(0,3);
            var result=sliced.length+"|"+sliced[0]+"|"+sliced[1]+"|"+own(sliced,1)+"|"+
                own(sliced,2)+"|"+removed.length+"|"+removed[1]+"|"+own(removed,1)+"|"+
                own(removed,2)+"|"+second.length+"|"+second[0];
            delete Array.prototype[1];return result;
        })()"#,
    ),
    (
        "splice shrink grow and equal replacement return deleted values and mutate in place",
        r#"(function(){
            var shrink=[0,1,2,3,4],a=shrink.splice(1,3,"x");
            var grow=[0,1,2,3],b=grow.splice(1,1,"x","y","z");
            var equal=[0,1,2],c=equal.splice(1,1,"x");
            return a.join(",")+":"+shrink.join(",")+"|"+
                b.join(",")+":"+grow.join(",")+"|"+
                c.join(",")+":"+equal.join(",");
        })()"#,
    ),
    (
        "splice actual argc distinguishes omitted start and deleteCount",
        r#"(function(){
            var a=[1,2],r0=a.splice();
            var b=[1,2],r1=b.splice(undefined);
            var c=[1,2],r2=c.splice(1);
            var d=[1,2],r3=d.splice(1,undefined,9);
            return r0.length+":"+a.join(",")+"|"+r1.join(",")+":"+b.length+"|"+
                r2.join(",")+":"+c.join(",")+"|"+r3.length+":"+d.join(",");
        })()"#,
    ),
    (
        "toSpliced actual argc mirrors the pinned omitted deleteCount branches",
        r#"(function(){
            var source=[1,2],a=source.toSpliced(),b=source.toSpliced(undefined);
            var c=source.toSpliced(1),d=source.toSpliced(1,undefined,9);
            return a.join(",")+"|"+b.length+"|"+c.join(",")+"|"+d.join(",")+"|"+
                source.join(",");
        })()"#,
    ),
    (
        "toSpliced densifies holes and inherited values without changing its source",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            Array.prototype[2]="proto";
            var source=Array(5);source[0]="a";source[4]="e";
            var result=source.toSpliced(1,1,"x",undefined);
            var text=result.length+"|"+result[0]+"|"+(result[1]===undefined)+"|"+
                result[2]+"|"+result[3]+"|"+(result[4]===undefined)+"|"+result[5]+"|"+
                own(result,0)+own(result,1)+own(result,2)+own(result,3)+own(result,4)+own(result,5)+"|"+
                source.length+"|"+own(source,1)+own(source,2);
            delete Array.prototype[2];return text;
        })()"#,
    ),
    (
        "all three methods preserve object identity for copied and inserted values",
        r#"(function(){
            var a=Object(),b=Object(),source=[a,b];
            var sliced=source.slice(0,1),removed=source.splice(1,1,a);
            var copied=source.toSpliced(1,1,b);
            return (sliced[0]===a)+"|"+(removed[0]===b)+"|"+(source[1]===a)+"|"+
                (copied[0]===a)+"|"+(copied[1]===b);
        })()"#,
    ),
    (
        "NaN infinities fractions strings null and undefined use pinned bound conversion",
        r#"(function(){
            var source=[0,1,2,3];
            var a=source.slice(NaN,Infinity),b=source.slice(-Infinity,2.9);
            var c=source.slice("2",-1),d=source.slice(1,undefined),e=source.slice(1,null);
            var f=source.toSpliced(-2.9,Infinity,"x");
            return a.join("")+"|"+b.join("")+"|"+c.join("")+"|"+d.join("")+"|"+
                e.length+"|"+f.join(",");
        })()"#,
    ),
];

const ORDER_AND_SPECIES_CASES: &[(&str, &str)] = &[
    (
        "slice converts bounds before constructor species and indexed reads",
        r#"(function(){
            var log="",source=[0,1,2,3],ctor=Object(),start=Object(),end=Object();
            start.valueOf=function(){log+="S";return 1};
            end.valueOf=function(){log+="E";return 3};
            function Species(length){
                log+="A"+length;var result=Object();
                result.__defineSetter__("length",function(value){log+="F"+value});return result;
            }
            ctor.__defineGetter__(Symbol.species,function(){log+="P";return Species});
            source.__defineGetter__("constructor",function(){log+="C";return ctor});
            source.__defineGetter__("1",function(){log+="G1";return "b"});
            source.__defineGetter__("2",function(){log+="G2";return "c"});
            var result=source.slice(start,end);
            return log+"|"+result[0]+result[1]+"|"+("length" in result);
        })()"#,
    ),
    (
        "splice finishes the removed result before moving and inserting on source",
        r#"(function(){
            var log="",source=[0,1,2],ctor=Object(),one,two;
            function Species(length){
                log+="N"+length;var result=Object();
                result.__defineSetter__("length",function(value){log+="L"+value});return result;
            }
            ctor[Symbol.species]=Species;source.constructor=ctor;
            source.__defineGetter__("1",function(){log+="g1";return 1});
            source.__defineSetter__("1",function(value){log+="s1"+value;one=value});
            source.__defineGetter__("2",function(){log+="g2";return 2});
            source.__defineSetter__("2",function(value){log+="s2"+value;two=value});
            var result=source.splice(1,1,"x","y");
            return log+"|"+result[0]+"|"+one+"|"+two+"|"+source.length;
        })()"#,
    ),
    (
        "species result uses CreateDataProperty and leaves preseeded hole properties intact",
        r#"(function(){
            var setterHits=0,lengthLog="",proto=Object(),ctor=Object();
            proto.__defineSetter__("0",function(){setterHits++});
            proto.__defineSetter__("length",function(value){lengthLog+="L"+value});
            function Species(){var result=Object.create(proto);result[1]="seed";return result}
            ctor[Symbol.species]=Species;
            var source=[4,,6];source.constructor=ctor;var result=source.slice();
            return result[0]+"|"+result[1]+"|"+result[2]+"|"+setterHits+"|"+lengthLog+"|"+
                Object.prototype.hasOwnProperty.call(result,"0");
        })()"#,
    ),
    (
        "generic slice ignores a throwing constructor while custom species sees exact empty count",
        r#"(function(){
            var generic=Object(),log="";generic.length=1;generic[0]="x";
            generic.__defineGetter__("constructor",function(){log+="bad";throw 71});
            var first=Array.prototype.slice.call(generic);
            var source=[1,2],ctor=Object();
            function Species(length){log+="N"+length;return Object()}
            ctor[Symbol.species]=Species;source.constructor=ctor;
            var second=source.slice(2,1);
            return first[0]+"|"+first.length+"|"+log+"|"+second.length;
        })()"#,
    ),
    (
        "a failed removed-result length Set prevents splice source mutation",
        r#"(function(){
            var captured,ctor=Object(),source=[1,2,3],descriptor=Object();
            function Species(){
                captured=Object();descriptor.value=0;descriptor.writable=false;
                descriptor.enumerable=false;descriptor.configurable=false;
                Object.defineProperty(captured,"length",descriptor);return captured;
            }
            ctor[Symbol.species]=Species;source.constructor=ctor;
            try{source.splice(1,1,"x");return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+captured[0]+"|"+
                source.length+"|"+source.join(",")}
        })()"#,
    ),
    (
        "species may alias the splice result to its source",
        r#"(function(){
            var source=[1,2,3],ctor=Object();function Species(){return source}
            ctor[Symbol.species]=Species;source.constructor=ctor;
            var result=source.splice(1,1,9);
            return (result===source)+"|"+source.length+"|"+source[0]+"|"+source[1]+"|"+
                (2 in source);
        })()"#,
    ),
    (
        "species construction mutation is observed by later slice reads",
        r#"(function(){
            var source=[1,2,3],ctor=Object();
            function Species(){source[1]=8;return Object()}
            ctor[Symbol.species]=Species;source.constructor=ctor;
            var result=source.slice(0,3);return result[0]+","+result[1]+","+result[2];
        })()"#,
    ),
    (
        "species may alias a slice result to its source",
        r#"(function(){
            var source=[1,2,3],ctor=Object();function Species(){return source}
            ctor[Symbol.species]=Species;source.constructor=ctor;
            var result=source.slice(1,3);
            return (result===source)+"|"+source.length+"|"+source[0]+"|"+source[1]+"|"+
                (2 in source);
        })()"#,
    ),
    (
        "a rejected result definition leaves the completed slice prefix externally observable",
        r#"(function(){
            var captured,ctor=Object(),descriptor=Object();
            function Species(){
                captured=Object();descriptor.value="fixed";descriptor.writable=false;
                descriptor.enumerable=true;descriptor.configurable=false;
                Object.defineProperty(captured,"1",descriptor);return captured;
            }
            ctor[Symbol.species]=Species;var source=[1,2,3];source.constructor=ctor;
            try{source.slice();return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+captured[0]+"|"+
                captured[1]+"|"+("length" in captured)}
        })()"#,
    ),
    (
        "constructor species lookup and constructor validation preserve abrupt order",
        r#"(function(){
            function run(mode){
                var source=[1],ctor=Object();
                if(mode===0)source.__defineGetter__("constructor",function(){throw 81});
                if(mode===1){ctor.__defineGetter__(Symbol.species,function(){throw 82});source.constructor=ctor}
                if(mode===2){ctor[Symbol.species]=1;source.constructor=ctor}
                try{source.slice();return "missing"}
                catch(error){return typeof error+":"+(typeof error==="object"?error.name+":"+error.message:error)}
            }
            return run(0)+"|"+run(1)+"|"+run(2);
        })()"#,
    ),
];

const MOVE_AND_PARTIAL_CASES: &[(&str, &str)] = &[
    (
        "splice shrink moves its tail forward before deleting and inserting",
        r#"(function(){
            var source=Object(),log="",one,two;source.length=5;source[0]="a";
            source.__defineSetter__("1",function(value){log+="s1";one=value});
            source.__defineSetter__("2",function(value){log+="s2";two=value});
            source.__defineGetter__("3",function(){log+="g3";return "d"});
            source.__defineGetter__("4",function(){log+="g4";return "e"});
            var removed=Array.prototype.splice.call(source,1,2);
            return log+"|"+one+two+"|"+removed.length+"|"+source.length;
        })()"#,
    ),
    (
        "splice growth moves its tail backward before ascending item Sets",
        r#"(function(){
            var source=Object(),log="";source.length=4;source[0]="a";source[1]="b";
            source.__defineGetter__("2",function(){log+="g2";return "c"});
            source.__defineSetter__("2",function(value){log+="s2"+value});
            source.__defineGetter__("3",function(){log+="g3";return "d"});
            source.__defineSetter__("3",function(value){log+="s3"+value});
            source.__defineSetter__("4",function(value){log+="s4"+value});
            source.__defineSetter__("5",function(value){log+="s5"+value});
            Array.prototype.splice.call(source,1,1,"x","y","z");
            return log+"|"+source[1]+"|"+source.length;
        })()"#,
    ),
    (
        "failed second shrink target Set preserves the completed first move",
        r#"(function(){
            var source=Object(),descriptor=Object(),first;source.length=5;
            source[3]="d";source[4]="e";
            source.__defineSetter__("1",function(value){first=value});
            descriptor.value="fixed";descriptor.writable=false;descriptor.enumerable=true;
            descriptor.configurable=false;Object.defineProperty(source,"2",descriptor);
            try{Array.prototype.splice.call(source,1,2);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+first+"|"+source[2]+"|"+
                source[3]+"|"+source[4]+"|"+source.length}
        })()"#,
    ),
    (
        "failed descending tail Delete preserves the move and earlier higher Delete",
        r#"(function(){
            var source=["a","b","c","d","e"],descriptor=Object();
            descriptor.value="d";descriptor.writable=true;descriptor.enumerable=true;
            descriptor.configurable=false;Object.defineProperty(source,"3",descriptor);
            try{source.splice(1,3,"x");return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source.length+"|"+
                source[1]+"|"+source[2]+"|"+source[3]+"|"+(4 in source)}
        })()"#,
    ),
    (
        "failed later insertion preserves tail growth and earlier inserted item",
        r#"(function(){
            var source=["a","b","c"],descriptor=Object();
            descriptor.value="c";descriptor.writable=false;descriptor.enumerable=true;
            descriptor.configurable=false;Object.defineProperty(source,"2",descriptor);
            try{source.splice(1,1,"x","y");return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source.length+"|"+
                source[1]+"|"+source[2]+"|"+source[3]}
        })()"#,
    ),
    (
        "failed final source length Set retains moves deletions and inserted items",
        r#"(function(){
            var source=Object(),log="";source[0]="a";source[1]="b";source[2]="c";
            source.__defineGetter__("length",function(){return 3});
            source.__defineSetter__("length",function(value){log+="L"+value;throw 77});
            try{Array.prototype.splice.call(source,1,1,"x","y");return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log+"|"+source[1]+"|"+
                source[2]+"|"+source[3]}
        })()"#,
    ),
    (
        "genuine Array Uint32 overflow keeps the ordinary high property",
        r#"(function(){
            var source=Array(4294967295);
            try{source.splice(4294967295,0,"x");return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source.length+"|"+
                source["4294967295"]+"|"+Object.prototype.hasOwnProperty.call(source,"4294967295")}
        })()"#,
    ),
    (
        "failed later grow source Get preserves the completed high move and grown length",
        r#"(function(){
            var source=["a","b","c","d"];
            source.__defineGetter__("2",function(){throw 92});
            try{source.splice(1,1,"x","y","z");return "missing"}
            catch(error){return typeof error+"|"+error+"|"+source.length+"|"+source[1]+"|"+
                source[5]}
        })()"#,
    ),
    (
        "splice always Sets source length even when argc requests no mutation",
        r#"(function(){
            var source=[1,2],descriptor=Object();descriptor.writable=false;
            Object.defineProperty(source,"length",descriptor);
            try{source.splice();return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source.length+"|"+
                source[0]+source[1]}
        })()"#,
    ),
];

const TO_SPLICED_ORDER_CASES: &[(&str, &str)] = &[
    (
        "toSpliced reads only the retained prefix and suffix in ascending order",
        r#"(function(){
            var source=Object(),log="";source.length=5;
            source.__defineGetter__("0",function(){log+="g0";source[4]="changed";return "a"});
            source.__defineGetter__("1",function(){log+="bad1";throw 81});
            source.__defineGetter__("2",function(){log+="bad2";throw 82});
            source.__defineGetter__("3",function(){log+="g3";return "d"});source[4]="e";
            var result=Array.prototype.toSpliced.call(source,1,2,"x");
            return log+"|"+result.join(",")+"|"+source[4];
        })()"#,
    ),
    (
        "toSpliced ignores constructor species and a replaced global Array",
        r#"(function(){
            var intrinsic=Array,source=[1,2,3],log="";
            source.__defineGetter__("constructor",function(){log+="C";throw 91});
            globalThis.Array=function(){throw 92};
            var result=intrinsic.prototype.toSpliced.call(source,1,1,8);
            return intrinsic.isArray(result)+"|"+result.join(",")+"|"+log;
        })()"#,
    ),
    (
        "toSpliced prefix mutation changes a later suffix query",
        r#"(function(){
            var source=Object(),log="";source.length=4;
            source.__defineGetter__("0",function(){log+="0";source[3]="new";return "a"});
            source[1]="b";source[2]="c";source[3]="old";
            var result=Array.prototype.toSpliced.call(source,1,2,"x");
            return log+"|"+result[0]+result[1]+result[2];
        })()"#,
    ),
    (
        "toSpliced signed-31-bit length failure precedes indexed reads",
        r#"(function(){
            var source=Object(),log="";source.length=2147483648;
            source.__defineGetter__("0",function(){log+="G";return 1});
            try{Array.prototype.toSpliced.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "toSpliced may delete a MAX_SAFE range into an empty dense result without reads",
        r#"(function(){
            var source=Object(),log="";source.length=9007199254740991;
            source.__defineGetter__("0",function(){log+="G";throw 93});
            var result=Array.prototype.toSpliced.call(source,0,9007199254740991);
            return result.length+"|"+log+"|"+Array.isArray(result);
        })()"#,
    ),
    (
        "toSpliced dense result definitions bypass Array prototype numeric setters",
        r#"(function(){
            var hits=0;Array.prototype.__defineSetter__("0",function(){hits++});
            var source=Array(1),result=source.toSpliced();
            var text=hits+"|"+Object.prototype.hasOwnProperty.call(result,"0")+"|"+
                (result[0]===undefined);
            delete Array.prototype[0];return text;
        })()"#,
    ),
];

const GENERIC_LIMIT_AND_ERROR_CASES: &[(&str, &str)] = &[
    (
        "generic objects and primitive strings use the pinned boxing and UTF-16 paths",
        r#"(function(){
            var source=Object();source.length=3;source[0]="a";source[2]="c";
            var a=Array.prototype.slice.call(source,0,3);
            var b=Array.prototype.toSpliced.call("A\uD83D\uDCA9Z",1,2,"x");
            return a.length+"|"+a[0]+"|"+(1 in a)+"|"+a[2]+"|"+
                b.length+"|"+b[0]+b[1]+b[2];
        })()"#,
    ),
    (
        "splice on a String boxes and builds its result before immutable source mutation fails",
        r#"(function(){
            try{Array.prototype.splice.call("ab",0,1);return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
    (
        "64-bit property keys work without traversing the full MAX_SAFE range",
        r#"(function(){
            var key="9007199254740990";
            var a=Object();a.length=9007199254740991;a[key]="S";
            var sliced=Array.prototype.slice.call(a,9007199254740990);
            var b=Object();b.length=9007199254740991;b[key]="P";
            var removed=Array.prototype.splice.call(b,9007199254740990,1);
            var c=Object();c.length=9007199254740991;c[key]="T";
            var copied=Array.prototype.toSpliced.call(c,0,9007199254740990);
            return sliced.length+":"+sliced[0]+"|"+removed.length+":"+removed[0]+":"+
                b.length+":"+(key in b)+"|"+copied.length+":"+copied[0];
        })()"#,
    ),
    (
        "slice and splice allow a sparse 2^31 result before the first getter throws",
        r#"(function(){
            function run(method){
                var source=Object();source.length=2147483648;
                source.__defineGetter__("0",function(){throw 61});
                try{method.call(source,0,2147483648);return "missing"}
                catch(error){return typeof error+":"+error}
            }
            return run(Array.prototype.slice)+"|"+run(Array.prototype.splice);
        })()"#,
    ),
    (
        "default Array species rejects a 2^32 result before indexed reads",
        r#"(function(){
            function run(method){
                var source=Object(),log="";source.length=4294967296;
                source.__defineGetter__("0",function(){log+="G";return 1});
                try{method.call(source,0,4294967296);return "missing"}
                catch(error){return error.name+":"+error.message+":"+log}
            }
            return run(Array.prototype.slice)+"|"+run(Array.prototype.splice);
        })()"#,
    ),
    (
        "splice and toSpliced distinguish their MAX_SAFE TypeError messages",
        r#"(function(){
            function run(method){
                var source=Object(),log="";source.length=9007199254740991;
                source.__defineGetter__("0",function(){log+="G";return 1});
                try{method.call(source,0,0,1);return "missing"}
                catch(error){return error.name+":"+error.message+":"+log}
            }
            return run(Array.prototype.splice)+"|"+run(Array.prototype.toSpliced);
        })()"#,
    ),
    (
        "null receivers and Symbol or BigInt conversions preserve pinned native errors",
        r#"(function(){
            function run(source){try{return source()}catch(error){return error.name+":"+error.message}}
            return run(function(){return Array.prototype.slice.call(null)})+"|"+
                run(function(){return [1].slice(Symbol("s"))})+"|"+
                run(function(){return [1].splice(0,1n)})+"|"+
                run(function(){return [1].toSpliced(Symbol("s"))});
        })()"#,
    ),
    (
        "user conversion and indexed getter throws remain arbitrary values",
        r#"(function(){
            function run(source){try{source();return "missing"}catch(error){return typeof error+":"+error}}
            var bound=Object();bound.valueOf=function(){throw 71};
            var source=Object();source.length=1;source.__defineGetter__("0",function(){throw 72});
            return run(function(){return [1].slice(bound)})+"|"+
                run(function(){return [1].splice(0,bound)})+"|"+
                run(function(){return Array.prototype.toSpliced.call(source)});
        })()"#,
    ),
    (
        "recursive slice getter produces a catchable stack overflow",
        r#"(function(){
            var source=Object();source.length=1;
            source.__defineGetter__("0",function(){
                return Array.prototype.slice.call(source,0,1)[0];
            });
            try{Array.prototype.slice.call(source,0,1);return "missing"}
            catch(error){return error.name+"|"+error.message}
        })()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','concat','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','join','toString','toLocaleString','pop','push','shift','unshift','reverse','toReversed','sort','toSorted','slice','splice','toSpliced','copyWithin','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor){return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable)}
function metadata(name){
  var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,name),fn=descriptor.value,constructable;
  try{Reflect.construct(function(){},[],fn);constructable=true}catch(error){constructable=false}
  return name+':'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
    (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable;
}
print('keys='+names.join(','));
print('meta='+metadata('slice'));
print('meta='+metadata('splice'));
print('meta='+metadata('toSpliced'));
"#;

#[test]
fn array_slice_splice_basic_rust_smoke() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var source=[1,2,3,4],slice=source.slice(1,3);
                    var removed=source.splice(1,2,8,9,10),copy=source.toSpliced(1,1,7);
                    return slice.join(",")+"|"+removed.join(",")+"|"+
                        source.join(",")+"|"+copy.join(",");
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("2,3|2,3|1,8,9,10,4|1,7,9,10,4").unwrap()),
    );
}

#[test]
fn array_slice_recursive_getter_stack_overflow_is_catchable_without_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"(function(){
                    var source=Object();source.length=1;
                    source.__defineGetter__("0",function(){
                        return Array.prototype.slice.call(source,0,1)[0];
                    });
                    try{Array.prototype.slice.call(source,0,1);return "missing"}
                    catch(error){return error.name+"|"+error.message}
                })()"#,
            )
            .unwrap(),
        Value::String(JsString::try_from_utf8("InternalError|stack overflow").unwrap()),
    );
}

#[test]
fn array_slice_splice_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array slice/splice oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order/species", ORDER_AND_SPECIES_CASES),
        ("move/partial", MOVE_AND_PARTIAL_CASES),
        ("toSpliced order", TO_SPLICED_ORDER_CASES),
        ("generic/limits/errors", GENERIC_LIMIT_AND_ERROR_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 4);
}

#[test]
fn array_slice_splice_values_holes_and_argc_match_pinned_quickjs() {
    compare_value_cases("Array slice/splice values", VALUE_CASES);
}

#[test]
fn array_slice_splice_conversion_species_and_result_order_match_pinned_quickjs() {
    compare_value_cases("Array slice/splice species order", ORDER_AND_SPECIES_CASES);
}

#[test]
fn array_splice_move_direction_and_partial_mutation_match_pinned_quickjs() {
    compare_value_cases("Array splice move/partial", MOVE_AND_PARTIAL_CASES);
}

#[test]
fn array_to_spliced_dense_query_order_and_limits_match_pinned_quickjs() {
    compare_value_cases("Array.toSpliced query order", TO_SPLICED_ORDER_CASES);
}

#[test]
fn array_slice_splice_generic_limits_and_errors_match_pinned_quickjs() {
    compare_value_cases(
        "Array slice/splice generic/limits/errors",
        GENERIC_LIMIT_AND_ERROR_CASES,
    );
}

#[test]
fn array_slice_splice_prototype_order_metadata_and_constructability_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array slice/splice graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array slice/splice prototype order/metadata drifted",
    );
}

#[test]
fn array_slice_splice_results_native_errors_and_user_throws_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let defining_range_error = eval_object(
        &mut defining,
        "RangeError.prototype",
        "defining RangeError prototype",
    );
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let slice = property_callable(&runtime, &mut defining, &defining_array_prototype, "slice");
    let splice = property_callable(&runtime, &mut defining, &defining_array_prototype, "splice");
    let to_spliced = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "toSpliced",
    );

    let receiver = eval_object(&mut caller, "[1,2,3]", "caller slice receiver");
    let Value::Object(result) = caller
        .call(
            &slice,
            Value::Object(receiver),
            &[Value::Int(1), Value::Int(3)],
        )
        .expect("cross-realm Array.slice")
    else {
        panic!("cross-realm Array.slice did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "foreign intrinsic Array constructor was not replaced by the method realm",
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 2);

    let receiver = eval_object(&mut caller, "[1,2,3]", "caller splice receiver");
    let Value::Object(removed) = caller
        .call(
            &splice,
            Value::Object(receiver.clone()),
            &[Value::Int(1), Value::Int(1), Value::Int(9)],
        )
        .expect("cross-realm Array.splice")
    else {
        panic!("cross-realm Array.splice did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&removed).unwrap(),
        Some(defining_array_prototype.clone()),
        "Array.splice removed result did not use the method defining realm",
    );
    assert_eq!(int_property(&runtime, &mut caller, &removed, "0"), 2);
    assert_eq!(int_property(&runtime, &mut caller, &receiver, "1"), 9);

    let receiver = eval_object(&mut caller, "[1,2,3]", "caller toSpliced receiver");
    let Value::Object(result) = caller
        .call(
            &to_spliced,
            Value::Object(receiver),
            &[Value::Int(1), Value::Int(1), Value::Int(8)],
        )
        .expect("cross-realm Array.toSpliced")
    else {
        panic!("cross-realm Array.toSpliced did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "Array.toSpliced result did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype.clone()),
    );

    let custom_species = eval_object(
        &mut caller,
        "(function(){var source=[1,2],ctor=Object();function Species(length){return [length]}ctor[Symbol.species]=Species;source.constructor=ctor;return source})()",
        "caller custom-species source",
    );
    let Value::Object(custom_result) = caller
        .call(
            &slice,
            Value::Object(custom_species),
            &[Value::Int(0), Value::Int(1)],
        )
        .expect("cross-realm custom Array species")
    else {
        panic!("custom Array species did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_result).unwrap(),
        Some(caller_array_prototype),
        "custom foreign species result lost its constructor realm",
    );

    let too_long = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=2147483648;return source})()",
        "caller oversized toSpliced receiver",
    );
    assert!(matches!(
        caller.call(&to_spliced, Value::Object(too_long), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.toSpliced RangeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_range_error),
        "Array.toSpliced RangeError did not use the method defining realm",
    );

    let overflow = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=9007199254740991;return source})()",
        "caller splice overflow receiver",
    );
    assert!(matches!(
        caller.call(
            &splice,
            Value::Object(overflow),
            &[Value::Int(0), Value::Int(0), Value::Int(1)],
        ),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.splice TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.splice TypeError did not use the method defining realm",
    );

    let throwing_receiver = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=1;source.__defineGetter__('0',function(){throw new TypeError('caller getter')});return source})()",
        "caller throwing slice receiver",
    );
    assert!(matches!(
        caller.call(
            &slice,
            Value::Object(throwing_receiver),
            &[Value::Int(0), Value::Int(1)],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.slice user getter TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.slice replaced a user getter throw with a defining-realm error",
    );
}

fn compare_value_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group} differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in cases {
        let expected = observe_oracle(&oracle, source, description);
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            expected,
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
                    error_string_property(runtime, context, &error, "name", description),
                    error_string_property(runtime, context, &error, "message", description),
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
  var value = std.evalScript(scriptArgs[0]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
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

fn rust_graph_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let array_prototype = context.array_prototype().unwrap();
    let function_prototype = context.function_prototype().unwrap();
    let implemented = [
        "at",
        "with",
        "concat",
        "every",
        "some",
        "forEach",
        "map",
        "filter",
        "reduce",
        "reduceRight",
        "fill",
        "find",
        "findIndex",
        "findLast",
        "findLastIndex",
        "indexOf",
        "lastIndexOf",
        "includes",
        "join",
        "toString",
        "toLocaleString",
        "pop",
        "push",
        "shift",
        "unshift",
        "reverse",
        "toReversed",
        "sort",
        "toSorted",
        "slice",
        "splice",
        "toSpliced",
        "copyWithin",
        "values",
        "keys",
        "entries",
    ];
    let names = runtime
        .own_property_keys(&array_prototype)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .filter(|name| implemented.contains(&name.as_str()))
        .collect::<Vec<_>>();
    let mut observations = vec![format!("keys={}", names.join(","))];
    for name in ["slice", "splice", "toSpliced"] {
        observations.push(format!(
            "meta={}",
            method_metadata(
                &runtime,
                &mut context,
                &array_prototype,
                &function_prototype,
                name,
            )
        ));
    }
    observations
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", GRAPH_ORACLE])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS Array slice/splice graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array slice/splice graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array slice/splice graph output was not UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn method_metadata(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    function_prototype: &ObjectRef,
    name: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let descriptor = runtime
        .get_own_property(owner, &key)
        .unwrap()
        .unwrap_or_else(|| panic!("missing Array.prototype.{name}"));
    let CompleteOrdinaryPropertyDescriptor::Data {
        value: Value::Object(function),
        writable,
        enumerable,
        configurable,
    } = &descriptor
    else {
        panic!("Array.prototype.{name} was not a function data property");
    };
    let callable = runtime
        .as_callable(function)
        .unwrap()
        .unwrap_or_else(|| panic!("Array.prototype.{name} was not callable"));
    let function_name = context
        .get_property(function, &runtime.intern_property_key("name").unwrap())
        .unwrap();
    let function_length = context
        .get_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap();
    let name_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("name").unwrap())
        .unwrap()
        .unwrap_or_else(|| panic!("Array.{name} name descriptor was missing"));
    let length_descriptor = runtime
        .get_own_property(function, &runtime.intern_property_key("length").unwrap())
        .unwrap()
        .unwrap_or_else(|| panic!("Array.{name} length descriptor was missing"));
    format!(
        "{name}:{}:{}:D{}{}{}:{}:{}:{}:{}:{}",
        primitive_value_text(function_name),
        primitive_value_text(function_length),
        Number(*writable),
        Number(*enumerable),
        Number(*configurable),
        data_descriptor_bits(&name_descriptor),
        data_descriptor_bits(&length_descriptor),
        true,
        runtime.get_prototype_of(function).unwrap().as_ref() == Some(function_prototype),
        runtime.is_constructor(callable.as_object()).unwrap(),
    )
}

fn data_descriptor_bits(descriptor: &CompleteOrdinaryPropertyDescriptor) -> String {
    let CompleteOrdinaryPropertyDescriptor::Data {
        writable,
        enumerable,
        configurable,
        ..
    } = descriptor
    else {
        panic!("expected a data descriptor");
    };
    format!(
        "D{}{}{}",
        Number(*writable),
        Number(*enumerable),
        Number(*configurable),
    )
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

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Int(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an Int property");
    };
    value
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{name} for {description}: {failure}"))
    else {
        panic!("Error.{name} was not a string for {description}");
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

struct Number(bool);

impl std::fmt::Display for Number {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(if self.0 { "1" } else { "0" })
    }
}
