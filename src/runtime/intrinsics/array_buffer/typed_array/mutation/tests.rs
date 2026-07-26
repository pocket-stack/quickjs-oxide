use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("TypedArray mutation test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn copy_within_fill_and_reverse_mutate_live_words_in_place() {
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
            var bytes=new Uint8Array([0,1,2,3,4]);
            check("copyWithin returns receiver",
                bytes.copyWithin(1,0,4)===bytes);
            check("copyWithin overlap",
                Array.prototype.join.call(bytes,",")==="0,0,1,2,3");
            check("fill returns receiver",bytes.fill(258,2,4)===bytes);
            check("fill converts once",
                Array.prototype.join.call(bytes,",")==="0,0,2,2,3");
            check("reverse returns receiver",bytes.reverse()===bytes);
            check("reverse bytes",
                Array.prototype.join.call(bytes,",")==="3,2,2,0,0");

            var words=new Uint16Array([1,2,3,4]);
            words.copyWithin(0,2);
            check("copyWithin word width",
                Array.prototype.join.call(words,",")==="3,4,3,4");
            words.fill(65538,-2);
            check("fill word width",
                Array.prototype.join.call(words,",")==="3,4,2,2");
            words.reverse();
            check("reverse word width",
                Array.prototype.join.call(words,",")==="2,2,4,3");

            var bigints=new BigInt64Array([1n,2n,3n]);
            bigints.fill(-2n,1);
            bigints.reverse();
            check("bigint mutation",
                bigints[0]===-2n && bigints[1]===-2n && bigints[2]===1n);
            check("bigint fill rejects number",
                completion(function(){bigints.fill(1)})==="TypeError");

            var rawBuffer=new ArrayBuffer(8);
            var rawView=new DataView(rawBuffer);
            rawView.setUint32(0,0x7fc01234,true);
            rawView.setUint32(4,0x80000000,true);
            var floats=new Float32Array(rawBuffer);
            floats.reverse();
            check("reverse preserves raw float words",
                rawView.getUint32(0,true)===0x80000000 &&
                rawView.getUint32(4,true)===0x7fc01234);
            floats.copyWithin(0,1);
            check("copyWithin preserves raw float words",
                rawView.getUint32(0,true)===0x7fc01234);

            var windowBuffer=new ArrayBuffer(8);
            var windowBytes=new Uint8Array(windowBuffer);
            windowBytes.set([9,1,2,3,4,8]);
            var window=new Uint8Array(windowBuffer,1,4);
            window.reverse();
            check("view mutation stays in window",
                Array.prototype.join.call(windowBytes,",")
                    ==="9,4,3,2,1,8,0,0");

            check("copyWithin brand",
                completion(function(){
                    Uint8Array.prototype.copyWithin.call({},0,0);
                })==="TypeError");
            check("fill brand",
                completion(function(){
                    Uint8Array.prototype.fill.call({},0);
                })==="TypeError");
            check("reverse brand",
                completion(function(){
                    Uint8Array.prototype.reverse.call({});
                })==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn mutation_coercion_revalidates_detach_and_resizable_bounds_like_quickjs() {
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
            function errorText(operation){
                try{operation();return "return"}
                catch(error){return error.name+":"+error.message}
            }
            function values(array){
                return Array.prototype.join.call(array,",");
            }

            var log="";
            var buffer=new ArrayBuffer(6,{maxByteLength:8});
            var array=new Uint8Array(buffer);
            array.set([0,1,2,3,4,5]);
            var target={valueOf:function(){log+="T";return 1}};
            var source={
                valueOf:function(){
                    log+="S";
                    buffer.resize(4);
                    return 0;
                }
            };
            var end={valueOf:function(){log+="E";return 6}};
            check("copyWithin resized return",
                array.copyWithin(target,source,end)===array);
            check("copyWithin coercion order",log==="TSE");
            check("copyWithin caps to live space",values(array)==="0,0,1,2");

            log="";
            buffer=new ArrayBuffer(2);
            array=new Uint8Array(buffer);
            target={
                valueOf:function(){
                    log+="T";
                    buffer.transfer();
                    return 0;
                }
            };
            source={valueOf:function(){log+="S";return 0}};
            end={valueOf:function(){log+="E";return 1}};
            check("copyWithin detach after all coercions",
                completion(function(){
                    array.copyWithin(target,source,end);
                })==="TypeError");
            check("copyWithin detached coercion order",log==="TSE");

            log="";
            buffer=new ArrayBuffer(6,{maxByteLength:8});
            array=new Uint8Array(buffer);
            array.set([1,2,3,4,5,6]);
            var value={
                valueOf:function(){
                    log+="V";
                    buffer.resize(4);
                    return 9;
                }
            };
            var start={valueOf:function(){log+="S";return 1}};
            end={valueOf:function(){log+="E";return 6}};
            check("fill resized return",array.fill(value,start,end)===array);
            check("fill coercion order",log==="VSE");
            check("fill caps end to live length",values(array)==="1,9,9,9");

            log="";
            buffer=new ArrayBuffer(2);
            array=new Uint8Array(buffer);
            value={
                valueOf:function(){
                    log+="V";
                    buffer.transfer();
                    return 7;
                }
            };
            start={valueOf:function(){log+="S";return 0}};
            end={valueOf:function(){log+="E";return 1}};
            check("fill detach after all coercions",
                completion(function(){array.fill(value,start,end)})
                    ==="TypeError");
            check("fill detached coercion order",log==="VSE");

            log="";
            array=new Uint8Array(0);
            target={valueOf:function(){log+="T";return 0}};
            source={valueOf:function(){log+="S";return 0}};
            end={valueOf:function(){log+="E";return 0}};
            array.copyWithin(target,source,end);
            check("zero copyWithin still coerces bounds",log==="TSE");
            log="";
            value={valueOf:function(){log+="V";return 1}};
            start={valueOf:function(){log+="S";return 0}};
            end={valueOf:function(){log+="E";return 0}};
            array.fill(value,start,end);
            check("zero fill still coerces value and bounds",log==="VSE");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            var fixed=new Uint8Array(buffer,0,4);
            buffer.resize(2);
            check("reverse rejects oob fixed view",
                completion(function(){fixed.reverse()})==="TypeError");
            check("reverse oob exact text",
                errorText(function(){fixed.reverse()})===
                "TypeError:ArrayBuffer is detached or resized");
            var tracking=new Uint8Array(buffer);
            tracking.set([1,2]);
            tracking.reverse();
            check("reverse uses live tracking length",
                values(tracking)==="2,1");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            fixed=new Uint8Array(buffer,0,4);
            fixed.set([0,1,2,3]);
            target={
                valueOf:function(){
                    buffer.resize(2);
                    return 0;
                }
            };
            source={valueOf:function(){return 2}};
            end={
                valueOf:function(){
                    buffer.resize(4);
                    return 4;
                }
            };
            fixed.copyWithin(target,source,end);
            check("copyWithin fixed view recovers before revalidation",
                values(fixed)==="0,0,0,0");

            buffer=new ArrayBuffer(4,{maxByteLength:8});
            fixed=new Uint8Array(buffer,0,4);
            fixed.set([0,1,2,3]);
            value={
                valueOf:function(){
                    buffer.resize(2);
                    return 9;
                }
            };
            start={valueOf:function(){return 1}};
            end={
                valueOf:function(){
                    buffer.resize(4);
                    return 4;
                }
            };
            fixed.fill(value,start,end);
            check("fill fixed view recovers before revalidation",
                values(fixed)==="0,9,9,9");

            buffer=new ArrayBuffer(8,{maxByteLength:12});
            var partialWords=new Uint16Array(buffer);
            partialWords.set([1,2,3,4]);
            target={
                valueOf:function(){
                    buffer.resize(5);
                    return 0;
                }
            };
            partialWords.copyWithin(target,1,4);
            check("copyWithin floors partial tracking element",
                partialWords.length===2 && values(partialWords)==="2,2");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}
