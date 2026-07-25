use super::*;

fn eval_string(context: &mut Context, source: &str) -> String {
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("DataView test did not return a String");
    };
    value.to_utf8_lossy()
}

fn assert_script(context: &mut Context, source: &str) {
    assert_eq!(eval_string(context, source), "ok");
}

#[test]
fn constructor_global_and_prototype_descriptors_match_the_public_surface() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }

            var globalDescriptor=Object.getOwnPropertyDescriptor(globalThis,"DataView");
            check("global value",globalDescriptor.value===DataView);
            check("global writable",globalDescriptor.writable===true);
            check("global enumerable",globalDescriptor.enumerable===false);
            check("global configurable",globalDescriptor.configurable===true);
            check("constructor name",DataView.name==="DataView");
            check("constructor length",DataView.length===1);

            var prototypeDescriptor=Object.getOwnPropertyDescriptor(DataView,"prototype");
            check("prototype value",prototypeDescriptor.value===DataView.prototype);
            check("prototype writable",prototypeDescriptor.writable===false);
            check("prototype enumerable",prototypeDescriptor.enumerable===false);
            check("prototype configurable",prototypeDescriptor.configurable===false);
            check("prototype parent",Object.getPrototypeOf(DataView.prototype)===Object.prototype);

            var constructorDescriptor=
                Object.getOwnPropertyDescriptor(DataView.prototype,"constructor");
            check("prototype constructor",constructorDescriptor.value===DataView);
            check("constructor writable",constructorDescriptor.writable===true);
            check("constructor enumerable",constructorDescriptor.enumerable===false);
            check("constructor configurable",constructorDescriptor.configurable===true);

            var tagDescriptor=
                Object.getOwnPropertyDescriptor(DataView.prototype,Symbol.toStringTag);
            check("tag value",tagDescriptor.value==="DataView");
            check("tag writable",tagDescriptor.writable===false);
            check("tag enumerable",tagDescriptor.enumerable===false);
            check("tag configurable",tagDescriptor.configurable===true);

            var accessors=["buffer","byteLength","byteOffset"];
            for(var i=0;i<accessors.length;i++){
                var accessor=Object.getOwnPropertyDescriptor(
                    DataView.prototype,accessors[i]
                );
                check(accessors[i]+" getter type",typeof accessor.get==="function");
                check(accessors[i]+" getter name",accessor.get.name==="get "+accessors[i]);
                check(accessors[i]+" getter length",accessor.get.length===0);
                check(accessors[i]+" setter",accessor.set===undefined);
                check(accessors[i]+" enumerable",accessor.enumerable===false);
                check(accessors[i]+" configurable",accessor.configurable===true);
            }

            var getters=[
                "getInt8","getUint8","getInt16","getUint16","getInt32","getUint32",
                "getBigInt64","getBigUint64","getFloat16","getFloat32","getFloat64"
            ];
            var setters=[
                "setInt8","setUint8","setInt16","setUint16","setInt32","setUint32",
                "setBigInt64","setBigUint64","setFloat16","setFloat32","setFloat64"
            ];
            for(var j=0;j<getters.length;j++){
                var getterDescriptor=
                    Object.getOwnPropertyDescriptor(DataView.prototype,getters[j]);
                check(getters[j]+" type",typeof getterDescriptor.value==="function");
                check(getters[j]+" name",getterDescriptor.value.name===getters[j]);
                check(getters[j]+" length",getterDescriptor.value.length===1);
                check(getters[j]+" writable",getterDescriptor.writable===true);
                check(getters[j]+" enumerable",getterDescriptor.enumerable===false);
                check(getters[j]+" configurable",getterDescriptor.configurable===true);

                var setterDescriptor=
                    Object.getOwnPropertyDescriptor(DataView.prototype,setters[j]);
                check(setters[j]+" type",typeof setterDescriptor.value==="function");
                check(setters[j]+" name",setterDescriptor.value.name===setters[j]);
                check(setters[j]+" length",setterDescriptor.value.length===2);
                check(setters[j]+" writable",setterDescriptor.writable===true);
                check(setters[j]+" enumerable",setterDescriptor.enumerable===false);
                check(setters[j]+" configurable",setterDescriptor.configurable===true);
            }

            var buffer=new ArrayBuffer(8);
            var view=new DataView(buffer,2,4);
            check("instance prototype",Object.getPrototypeOf(view)===DataView.prototype);
            check("instance constructor",view.constructor===DataView);
            check("instance tag",Object.prototype.toString.call(view)==="[object DataView]");
            check("instance buffer",view.buffer===buffer);
            check("instance byteLength",view.byteLength===4);
            check("instance byteOffset",view.byteOffset===2);

            var callError="none";
            try{DataView(buffer)}catch(error){callError=error.name}
            check("constructor requires new",callError==="TypeError");

            var methodConstructError="none";
            try{new DataView.prototype.getUint8()}
            catch(error){methodConstructError=error.name}
            check("methods are not constructors",methodConstructError==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn every_element_getter_and_setter_roundtrips_and_honors_endianness() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }

            var view=new DataView(new ArrayBuffer(8));
            function clear(){
                for(var i=0;i<8;i++) view.setUint8(i,0);
            }
            function expectBytes(label,expected){
                for(var i=0;i<expected.length;i++){
                    check(label+" byte "+i,view.getUint8(i)===expected[i]);
                }
            }
            function word(label,setter,getter,value,bigEndianBytes,littleEndianBytes){
                clear();
                check(label+" big return",view[setter](0,value,false)===undefined);
                expectBytes(label+" big",bigEndianBytes);
                check(label+" big roundtrip",view[getter](0,false)===value);

                clear();
                check(label+" little return",view[setter](0,value,true)===undefined);
                expectBytes(label+" little",littleEndianBytes);
                check(label+" little roundtrip",view[getter](0,true)===value);
            }

            clear();
            check("int8 return",view.setInt8(0,-2,true)===undefined);
            expectBytes("int8",[254]);
            check("int8 roundtrip",view.getInt8(0,true)===-2);

            clear();
            check("uint8 return",view.setUint8(0,254,true)===undefined);
            expectBytes("uint8",[254]);
            check("uint8 roundtrip",view.getUint8(0,true)===254);
            view.setUint8(0,258);
            check("uint8 conversion",view.getUint8(0)===2);

            word("int16","setInt16","getInt16",-2,
                [255,254],[254,255]);
            word("uint16","setUint16","getUint16",0xabcd,
                [0xab,0xcd],[0xcd,0xab]);
            word("int32","setInt32","getInt32",-2,
                [255,255,255,254],[254,255,255,255]);
            word("uint32","setUint32","getUint32",0x89abcdef,
                [0x89,0xab,0xcd,0xef],[0xef,0xcd,0xab,0x89]);
            word("bigInt64","setBigInt64","getBigInt64",-2n,
                [255,255,255,255,255,255,255,254],
                [254,255,255,255,255,255,255,255]);
            word("bigUint64","setBigUint64","getBigUint64",0x0102030405060708n,
                [1,2,3,4,5,6,7,8],[8,7,6,5,4,3,2,1]);
            word("float16","setFloat16","getFloat16",1.5,
                [0x3e,0],[0,0x3e]);
            word("float32","setFloat32","getFloat32",1.5,
                [0x3f,0xc0,0,0],[0,0,0xc0,0x3f]);
            word("float64","setFloat64","getFloat64",1.5,
                [0x3f,0xf8,0,0,0,0,0,0],[0,0,0,0,0,0,0xf8,0x3f]);

            clear();
            view.setUint16(0,0x1234);
            expectBytes("default big endian",[0x12,0x34]);
            check("default getter big endian",view.getUint16(0)===0x1234);
            check("opposite getter endian",view.getUint16(0,true)===0x3412);

            view.setFloat16(0,-0);
            check("float16 negative zero bytes",view.getUint16(0)===0x8000);
            check("float16 negative zero",1/view.getFloat16(0)===-Infinity);
            view.setFloat16(0,NaN);
            check("float16 canonical NaN bytes",view.getUint16(0)===0x7c01);
            check("float16 NaN",view.getFloat16(0)!==view.getFloat16(0));

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn array_buffer_is_view_recognizes_data_views_before_and_after_detach() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }

            var buffer=new ArrayBuffer(4);
            var view=new DataView(buffer);
            check("view",ArrayBuffer.isView(view)===true);
            check("buffer",ArrayBuffer.isView(buffer)===false);
            check("ordinary",ArrayBuffer.isView({})===false);
            check("prototype",ArrayBuffer.isView(DataView.prototype)===false);
            buffer.transfer();
            check("detached view",ArrayBuffer.isView(view)===true);
            check("detached buffer",ArrayBuffer.isView(buffer)===false);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn detached_views_keep_the_buffer_identity_but_reject_metadata_and_access() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }

            var buffer=new ArrayBuffer(8);
            var view=new DataView(buffer,2,4);
            view.setUint32(0,0x12345678);
            var moved=buffer.transfer();
            check("source detached",buffer.detached===true);
            check("target attached",moved.detached===false);
            check("buffer identity",view.buffer===buffer);
            check("isView",ArrayBuffer.isView(view)===true);
            check("byteLength error",
                errorName(function(){return view.byteLength})==="TypeError");
            check("byteOffset error",
                errorName(function(){return view.byteOffset})==="TypeError");
            check("read error",
                errorName(function(){return view.getUint8(0)})==="TypeError");
            check("write error",
                errorName(function(){return view.setUint8(0,1)})==="TypeError");
            check("constructor error",
                errorName(function(){return new DataView(buffer)})==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn fixed_and_tracking_views_follow_resizable_buffer_shrink_and_grow() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_script(
        &mut context,
        r#"(function(){
            var failures=[];
            function check(label,condition){
                if(!condition) failures.push(label);
            }
            function errorName(operation){
                try{operation();return "none"}
                catch(error){return error.name}
            }

            var buffer=new ArrayBuffer(8,{maxByteLength:16});
            var fixed=new DataView(buffer,2,4);
            var tracking=new DataView(buffer,2);
            var explicitUndefined=new DataView(buffer,2,undefined);
            fixed.setUint8(0,0x22);
            tracking.setUint8(2,0x44);

            check("initial fixed length",fixed.byteLength===4);
            check("initial fixed offset",fixed.byteOffset===2);
            check("initial tracking length",tracking.byteLength===6);
            check("initial tracking offset",tracking.byteOffset===2);
            check("undefined tracks",explicitUndefined.byteLength===6);

            buffer.resize(5);
            check("fixed length while oob",
                errorName(function(){return fixed.byteLength})==="TypeError");
            check("fixed offset while oob",
                errorName(function(){return fixed.byteOffset})==="TypeError");
            check("fixed valid declared index while oob",
                errorName(function(){return fixed.getUint8(0)})==="TypeError");
            check("fixed invalid declared index wins",
                errorName(function(){return fixed.getUint8(4)})==="RangeError");
            check("tracking shrinks",tracking.byteLength===3);
            check("tracking keeps offset",tracking.byteOffset===2);
            check("tracking retained byte",tracking.getUint8(2)===0x44);

            buffer.resize(2);
            check("tracking at end length",tracking.byteLength===0);
            check("tracking at end offset",tracking.byteOffset===2);
            check("tracking zero range",
                errorName(function(){return tracking.getUint8(0)})==="RangeError");

            buffer.resize(1);
            check("tracking length while offset oob",
                errorName(function(){return tracking.byteLength})==="TypeError");
            check("tracking offset while offset oob",
                errorName(function(){return tracking.byteOffset})==="TypeError");

            buffer.resize(8);
            check("fixed recovers length",fixed.byteLength===4);
            check("fixed recovers offset",fixed.byteOffset===2);
            check("tracking regrows",tracking.byteLength===6);
            check("undefined view regrows",explicitUndefined.byteLength===6);
            check("truncated bytes are zero",fixed.getUint8(0)===0);

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn constructor_coercion_and_new_target_reentrancy_preserve_error_order() {
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
                catch(error){
                    return error && error.name ? error.name : "throw:"+error;
                }
            }

            var log="";
            var buffer=new ArrayBuffer(8);
            var offset={
                valueOf:function(){log+="O";return 2}
            };
            var length={
                valueOf:function(){log+="L";return 4}
            };
            var view=new DataView(buffer,offset,length);
            check("ordinary coercion order",log==="OL");
            check("ordinary coercion result",view.byteOffset===2 && view.byteLength===4);

            log="";
            check("brand error",
                completion(function(){return new DataView({},offset,length)})==="TypeError");
            check("brand before arguments",log==="");

            var detached=new ArrayBuffer(4);
            detached.transfer();
            log="";
            offset={valueOf:function(){log+="O";return 0}};
            length={valueOf:function(){log+="L";return 1}};
            check("detached constructor",
                completion(function(){return new DataView(detached,offset,length)})==="TypeError");
            check("offset before detached and length after",log==="O");

            buffer=new ArrayBuffer(4);
            log="";
            offset={valueOf:function(){log+="O";return 5}};
            length={valueOf:function(){log+="L";return 0}};
            check("offset range",
                completion(function(){return new DataView(buffer,offset,length)})==="RangeError");
            check("offset range before length",log==="O");

            buffer=new ArrayBuffer(4);
            log="";
            offset={
                valueOf:function(){
                    log+="O";
                    buffer.transfer();
                    return 0;
                }
            };
            length={valueOf:function(){log+="L";return 1}};
            check("offset detaches",
                completion(function(){return new DataView(buffer,offset,length)})==="TypeError");
            check("detach before length",log==="O");

            buffer=new ArrayBuffer(8,{maxByteLength:16});
            log="";
            length={
                valueOf:function(){
                    log+="L";
                    buffer.resize(2);
                    return 4;
                }
            };
            check("length shrink revalidated",
                completion(function(){return new DataView(buffer,0,length)})==="RangeError");
            check("length was coerced",log==="L");

            buffer=new ArrayBuffer(8,{maxByteLength:16});
            log="";
            var shrinkingNewTarget=new Proxy(function(){},{
                get:function(target,key,receiver){
                    if(key==="prototype"){
                        log+="P";
                        buffer.resize(5);
                        return DataView.prototype;
                    }
                    return Reflect.get(target,key,receiver);
                }
            });
            check("prototype shrink revalidated",
                completion(function(){
                    return Reflect.construct(DataView,[buffer,2,4],shrinkingNewTarget);
                })==="RangeError");
            check("prototype shrink observed",log==="P");

            buffer=new ArrayBuffer(8);
            log="";
            var detachingNewTarget=new Proxy(function(){},{
                get:function(target,key,receiver){
                    if(key==="prototype"){
                        log+="P";
                        buffer.transfer();
                        return DataView.prototype;
                    }
                    return Reflect.get(target,key,receiver);
                }
            });
            check("prototype detach revalidated",
                completion(function(){
                    return Reflect.construct(DataView,[buffer,0,4],detachingNewTarget);
                })==="TypeError");
            check("prototype detach observed",log==="P");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}

#[test]
fn access_methods_coerce_in_spec_order_and_revalidate_after_reentry() {
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
                catch(error){
                    return error && error.name ? error.name : "throw:"+error;
                }
            }

            var log="";
            var index={valueOf:function(){log+="I";return 0}};
            var value={valueOf:function(){log+="V";return 7}};
            check("get brand",
                completion(function(){
                    return DataView.prototype.getUint8.call({},index);
                })==="TypeError");
            check("get brand before index",log==="");
            check("set brand",
                completion(function(){
                    return DataView.prototype.setUint8.call({},index,value);
                })==="TypeError");
            check("set brand before arguments",log==="");

            var buffer=new ArrayBuffer(1);
            var view=new DataView(buffer);
            log="";
            index={valueOf:function(){log+="I";return 2}};
            value={valueOf:function(){log+="V";return 7}};
            check("set range",
                completion(function(){return view.setUint8(index,value)})==="RangeError");
            check("set converts value before range",log==="IV");

            buffer=new ArrayBuffer(1);
            view=new DataView(buffer);
            log="";
            index={
                valueOf:function(){
                    log+="I";
                    buffer.transfer();
                    return 0;
                }
            };
            check("get reentrant detach",
                completion(function(){return view.getUint8(index)})==="TypeError");
            check("get index before detach check",log==="I");

            buffer=new ArrayBuffer(1);
            view=new DataView(buffer);
            log="";
            index={valueOf:function(){log+="I";return 0}};
            value={
                valueOf:function(){
                    log+="V";
                    buffer.transfer();
                    return 1;
                }
            };
            check("set reentrant detach",
                completion(function(){return view.setUint8(index,value)})==="TypeError");
            check("set index and value before detach check",log==="IV");

            buffer=new ArrayBuffer(1);
            view=new DataView(buffer);
            buffer.transfer();
            log="";
            index={valueOf:function(){log+="I";return 0}};
            value={valueOf:function(){log+="V";return 1}};
            check("set already detached",
                completion(function(){return view.setUint8(index,value)})==="TypeError");
            check("detached set still converts arguments",log==="IV");

            buffer=new ArrayBuffer(1);
            view=new DataView(buffer);
            log="";
            index={valueOf:function(){log+="I";return 9}};
            value={
                valueOf:function(){
                    log+="V";
                    throw "value";
                }
            };
            check("value throw wins range",
                completion(function(){return view.setUint8(index,value)})==="throw:value");
            check("value throw order",log==="IV");

            log="";
            index={valueOf:function(){log+="I";return 9}};
            value={valueOf:function(){log+="V";return 1n}};
            check("bigint conversion before range",
                completion(function(){return view.setBigInt64(index,value)})==="RangeError");
            check("bigint conversion order",log==="IV");
            check("bigint rejects number before range",
                completion(function(){return view.setBigInt64(9,1)})==="TypeError");
            check("number rejects bigint before range",
                completion(function(){return view.setUint8(9,1n)})==="TypeError");

            return failures.length===0 ? "ok" : failures.join(",");
        })()"#,
    );
}
