use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray search test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn at_and_search_methods_match_typed_numeric_comparison_rules() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function completion(operation){
                try{operation();return "return"}
                catch(error){return error.name}
            }

            var bytes=new Uint8Array([10,20,10]);
            check("at first",bytes.at(0)===10);
            check("at last",bytes.at(-1)===10);
            check("at negative",bytes.at(-2)===20);
            check("at low oob",bytes.at(-4)===undefined);
            check("at high oob",bytes.at(3)===undefined);
            check("at truncates",bytes.at(1.9)===20);
            check("at infinity",bytes.at(Infinity)===undefined);
            check("indexOf",bytes.indexOf(10)===0);
            check("indexOf from",bytes.indexOf(10,1)===2);
            check("indexOf negative from",bytes.indexOf(10,-1)===2);
            check("lastIndexOf",bytes.lastIndexOf(10)===2);
            check("lastIndexOf from",bytes.lastIndexOf(10,1)===0);
            check("includes",bytes.includes(20)===true);
            check("includes miss",bytes.includes(21)===false);

            var floats=new Float32Array([NaN,-0,1]);
            check("includes NaN",floats.includes(NaN)===true);
            check("indexOf NaN",floats.indexOf(NaN)===-1);
            check("lastIndexOf NaN",floats.lastIndexOf(NaN)===-1);
            check("includes signed zero",floats.includes(0)===true);
            check("indexOf signed zero",floats.indexOf(0)===1);

            var bigints=new BigInt64Array([1n,-2n,1n]);
            check("bigint includes",bigints.includes(-2n)===true);
            check("bigint indexOf",bigints.indexOf(1n)===0);
            check("bigint lastIndexOf",bigints.lastIndexOf(1n)===2);
            check("bigint number differs",bigints.includes(1)===false);
            check("number bigint differs",bytes.includes(10n)===false);

            var coerced=false;
            var needle={valueOf:function(){coerced=true;return 10}};
            check("search value not coerced",bytes.indexOf(needle)===-1);
            check("search coercion flag",coerced===false);

            check("at brand",
                completion(function(){
                    Uint8Array.prototype.at.call({},0);
                })==="TypeError");
            check("indexOf brand",
                completion(function(){
                    Uint8Array.prototype.indexOf.call({},0);
                })==="TypeError");
            check("lastIndexOf brand",
                completion(function(){
                    Uint8Array.prototype.lastIndexOf.call({},0);
                })==="TypeError");
            check("includes brand",
                completion(function(){
                    Uint8Array.prototype.includes.call({},0);
                })==="TypeError");
            check("at not constructor",
                completion(function(){
                    new (Object.getPrototypeOf(Uint8Array.prototype).at)(0);
                })==="TypeError");

            var base=Object.getPrototypeOf(Uint8Array.prototype);
            check("filtered QuickJS own-key order",
                Reflect.ownKeys(base).map(function(key){
                    return typeof key==="symbol" ? key.toString() : key;
                }).join("|")===
                "length|at|buffer|byteLength|byteOffset|set|values|keys|"+
                "entries|copyWithin|every|some|forEach|map|filter|reduce|"+
                "reduceRight|fill|find|findIndex|findLast|findLastIndex|"+
                "reverse|slice|subarray|indexOf|lastIndexOf|includes|"+
                "constructor|toString|Symbol(Symbol.iterator)|"+
                "Symbol(Symbol.toStringTag)");
            for(var name of ["at","indexOf","lastIndexOf","includes"]){
                var descriptor=Object.getOwnPropertyDescriptor(base,name);
                check(name+" value",typeof descriptor.value==="function");
                check(name+" name",descriptor.value.name===name);
                check(name+" length",descriptor.value.length===1);
                check(name+" writable",descriptor.writable===true);
                check(name+" enumerable",descriptor.enumerable===false);
                check(name+" configurable",descriptor.configurable===true);
            }

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn index_coercion_observes_quickjs_resize_and_detach_boundaries() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function completion(operation){
                try{operation();return "return"}
                catch(error){return error.name}
            }

            var log="";
            var empty=new Uint8Array(0);
            var fromIndex={valueOf:function(){log+="F";return 0}};
            check("empty includes",empty.includes(0,fromIndex)===false);
            check("empty indexOf",empty.indexOf(0,fromIndex)===-1);
            check("empty lastIndexOf",empty.lastIndexOf(0,fromIndex)===-1);
            check("empty search skips fromIndex",log==="");
            var atIndex={valueOf:function(){log+="A";return 0}};
            check("empty at",empty.at(atIndex)===undefined);
            check("empty at coerces",log==="A");

            var buffer=new ArrayBuffer(4,{maxByteLength:8});
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            atIndex={
                valueOf:function(){
                    buffer.resize(2);
                    return -1;
                }
            };
            check("at negative uses initial length",
                tracking.at(atIndex)===undefined);

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            atIndex={
                valueOf:function(){
                    buffer.resize(4);
                    return 3;
                }
            };
            check("at positive uses grown live length",
                tracking.at(atIndex)===0);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            fromIndex={
                valueOf:function(){
                    buffer.resize(2);
                    return 0;
                }
            };
            check("includes missing tail is undefined",
                tracking.includes(undefined,fromIndex)===true);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            fromIndex={
                valueOf:function(){
                    buffer.resize(2);
                    return 0;
                }
            };
            check("indexOf ignores missing tail",
                tracking.indexOf(undefined,fromIndex)===-1);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2,3,4]);
            fromIndex={
                valueOf:function(){
                    buffer.resize(2);
                    return 3;
                }
            };
            check("lastIndexOf clips to live length",
                tracking.lastIndexOf(2,fromIndex)===1);

            buffer=new ArrayBuffer(2,{maxByteLength:8});
            tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            fromIndex={
                valueOf:function(){
                    buffer.resize(4);
                    return 2;
                }
            };
            check("search excludes grown tail",
                tracking.includes(0,fromIndex)===false);

            buffer=new ArrayBuffer(2);
            tracking=new Uint8Array(buffer);
            fromIndex={
                valueOf:function(){
                    buffer.transfer();
                    return 0;
                }
            };
            check("detached includes missing undefined",
                tracking.includes(undefined,fromIndex)===true);

            buffer=new ArrayBuffer(2);
            tracking=new Uint8Array(buffer);
            fromIndex={
                valueOf:function(){
                    buffer.transfer();
                    return 0;
                }
            };
            check("detached indexOf returns not found",
                tracking.indexOf(undefined,fromIndex)===-1);

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            log="";
            fromIndex={valueOf:function(){log+="F";return 0}};
            check("initial oob throws before coercion",
                completion(function(){fixed.includes(0,fromIndex)})
                    ==="TypeError");
            check("initial oob skipped coercion",log==="");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}
