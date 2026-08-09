use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, Runtime,
    RuntimeError, Value,
};

// This target pins the allocating modes of QuickJS 2026-06-04's shared
// `js_array_every` kernel and its `JS_ArraySpeciesCreate` boundary.

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "map transforms present elements while preserving holes and length",
        r#"(function(){
            var source=[1,,3],log="";
            var result=source.map(function(value,index){log+=index;return value*2});
            return result.length+"|"+result[0]+"|"+(1 in result)+"|"+result[2]+"|"+
                (result!==source)+"|"+source[0]+"|"+source[2]+"|"+log;
        })()"#,
    ),
    (
        "filter compacts selected present source values",
        r#"(function(){
            var source=[1,,2,3,4],log="";
            var result=source.filter(function(value,index){log+=index;return value%2===1});
            return result.length+"|"+result[0]+"|"+result[1]+"|"+(2 in result)+"|"+
                (result!==source)+"|"+log;
        })()"#,
    ),
    (
        "filter uses ToBoolean but stores the original value",
        r#"(function(){
            var log="",truthy=Object();
            truthy.valueOf=function(){log+="v";throw 61};
            truthy.toString=function(){log+="s";throw 62};
            var result=[7].filter(function(){return truthy});
            return result.length+"|"+result[0]+"|"+log;
        })()"#,
    ),
    (
        "map stores callback objects without coercion",
        r#"(function(){
            var log="",marker=Object();
            marker.valueOf=function(){log+="v";throw 63};
            marker.toString=function(){log+="s";throw 64};
            var result=[1].map(function(){return marker});
            return (result[0]===marker)+"|"+log;
        })()"#,
    ),
    (
        "map callback undefined creates a present result property",
        r#"(function(){
            var result=[1].map(function(){});
            return result.length+"|"+(0 in result)+"|"+(result[0]===undefined);
        })()"#,
    ),
    (
        "empty inputs allocate empty results without callbacks",
        r#"(function(){
            var calls=0,callback=function(){calls++;return true};
            var mapped=[].map(callback),filtered=[].filter(callback);
            return mapped.length+"|"+filtered.length+"|"+calls+"|"+
                Array.isArray(mapped)+"|"+Array.isArray(filtered);
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "holes are skipped while inherited values become own result properties",
        r#"(function(){
            var proto=Object.create(Array.prototype),source=Array(4),log="";
            proto[1]=4;source[3]=8;Object.setPrototypeOf(source,proto);
            var mapped=source.map(function(value,index){log+="m"+index;return value+1});
            var filtered=source.filter(function(value,index){log+="f"+index;return true});
            return mapped.length+"|"+(0 in mapped)+"|"+mapped[1]+"|"+(2 in mapped)+"|"+
                mapped[3]+"|"+filtered.length+"|"+filtered[0]+"|"+filtered[1]+"|"+log;
        })()"#,
    ),
    (
        "length is snapshotted and later HasProperty observes mutation",
        r#"(function(){
            var source=[1,2,,4],log="";
            var result=source.map(function(value,index){
                log+=index+":"+value+",";
                if(index===0){delete source[1];source[2]=3;source[4]=5;source.length=5}
                return value*10;
            });
            return result.length+"|"+result[0]+"|"+(1 in result)+"|"+result[2]+"|"+
                result[3]+"|"+(4 in result)+"|"+log+"|"+source.length;
        })()"#,
    ),
    (
        "filter copies the value captured before callback mutation",
        r#"(function(){
            var source=[4];
            var result=source.filter(function(value,index,receiver){
                receiver[index]=9;return true;
            });
            return result[0]+"|"+source[0];
        })()"#,
    ),
    (
        "callback value index boxed receiver and strict thisArg are exact",
        r#"(function(){
            var marker=Object(),source="ab",log="";
            var mapped=Array.prototype.map.call(source,function(value,index,receiver){
                "use strict";
                log+="m"+(this===marker)+":"+value+":"+index+":"+
                    (typeof receiver)+":"+Object.prototype.toString.call(receiver)+",";
                return value;
            },marker);
            var filtered=Array.prototype.filter.call(source,function(value,index,receiver){
                "use strict";
                log+="f"+(this===undefined)+":"+(receiver!==source)+":"+index+",";
                return index===1;
            });
            return mapped[0]+mapped[1]+"|"+filtered[0]+"|"+log;
        })()"#,
    ),
    (
        "callback validation precedes constructor and species access",
        r#"(function(){
            var source=[1],log="";
            source.__defineGetter__("constructor",function(){log+="C";throw 71});
            try{source.map(1);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "species allocation precedes indexed access and callbacks",
        r#"(function(){
            var source=[1],ctor=Object(),descriptor=Object(),log="";
            function Species(length){log+="N"+length;return Object()}
            descriptor.get=function(){log+="S";return Species};
            Object.defineProperty(ctor,Symbol.species,descriptor);
            source.__defineGetter__("constructor",function(){log+="C";return ctor});
            source.__defineGetter__("0",function(){log+="G";return 5});
            var result=source.map(function(value){log+="F";return value+1});
            return result[0]+"|"+log;
        })()"#,
    ),
    (
        "a callback throw stops before later indexed access",
        r#"(function(){
            var source=[1,2],log="";
            source.__defineGetter__("1",function(){log+="G";throw 72});
            try{source.filter(function(){log+="F";throw 73});return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const SPECIES_CASES: &[(&str, &str)] = &[
    (
        "custom species receive map length or filter zero without a final length Set",
        r#"(function(){
            function Species(length){var result=Object();result.arg=length;return result}
            var ctor=Object();ctor[Symbol.species]=Species;
            var left=[1,,3],right=[1,2,3];left.constructor=ctor;right.constructor=ctor;
            var mapped=left.map(function(value){return value*2});
            var filtered=right.filter(function(value){return value>1});
            return mapped.arg+"|"+mapped[0]+"|"+(1 in mapped)+"|"+mapped[2]+"|"+
                ("length" in mapped)+"|"+filtered.arg+"|"+filtered[0]+"|"+filtered[1]+"|"+
                ("length" in filtered);
        })()"#,
    ),
    (
        "undefined constructor and null species fall back to base Arrays",
        r#"(function(){
            var left=[1,2],right=[1,2],ctor=Object();
            left.constructor=undefined;ctor[Symbol.species]=null;right.constructor=ctor;
            var mapped=left.map(function(value){return value});
            var filtered=right.filter(function(){return true});
            return Array.isArray(mapped)+"|"+mapped.length+"|"+
                Array.isArray(filtered)+"|"+filtered.length;
        })()"#,
    ),
    (
        "default species creation ignores a replaced global Array binding",
        r#"(function(){
            var intrinsic=Array,map=Array.prototype.map,source=[1];
            source.constructor=undefined;
            globalThis.Array=function(){throw 79};
            var result=map.call(source,function(value){return value+1});
            return intrinsic.isArray(result)+"|"+result.length+"|"+result[0];
        })()"#,
    ),
    (
        "generic receivers ignore their constructor property",
        r#"(function(){
            var source=Object(),log="";source[0]=3;source.length=1;
            source.__defineGetter__("constructor",function(){log+="C";throw 81});
            var mapped=Array.prototype.map.call(source,function(value){return value+1});
            var filtered=Array.prototype.filter.call(source,function(){return true});
            return Array.isArray(mapped)+"|"+mapped[0]+"|"+
                Array.isArray(filtered)+"|"+filtered[0]+"|"+log;
        })()"#,
    ),
    (
        "CreateDataProperty ignores an inherited result setter",
        r#"(function(){
            var hits=0,proto=Object(),ctor=Object();
            proto.__defineSetter__("0",function(){hits++});
            function Species(){return Object.create(proto)}
            ctor[Symbol.species]=Species;
            var source=[2];source.constructor=ctor;
            var result=source.map(function(value){return value*3});
            return result[0]+"|"+hits+"|"+Object.prototype.hasOwnProperty.call(result,"0");
        })()"#,
    ),
    (
        "a rejected result definition preserves callback order and stops traversal",
        r#"(function(){
            var log="",ctor=Object(),captured;
            function Species(){
                var result=Object(),descriptor=Object();captured=result;
                descriptor.value=9;descriptor.writable=false;
                descriptor.enumerable=true;descriptor.configurable=false;
                Object.defineProperty(result,"1",descriptor);return result;
            }
            ctor[Symbol.species]=Species;
            var source=[1,2,3];source.constructor=ctor;
            source.__defineGetter__("2",function(){log+="G";throw 82});
            try{source.map(function(value,index){log+="F"+index;return value});return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log+"|"+
                captured[0]+"|"+captured[1]}
        })()"#,
    ),
];

const GENERIC_CASES: &[(&str, &str)] = &[
    (
        "ordinary array-like results are defining-realm base Arrays",
        r#"(function(){
            var source=Object();source[0]="a";source[2]="c";source.length=3;
            var mapped=Array.prototype.map.call(source,function(value,index,receiver){
                return (receiver===source)+":"+index+":"+value;
            });
            var filtered=Array.prototype.filter.call(source,function(value){return value==="c"});
            return Array.isArray(mapped)+"|"+mapped.length+"|"+mapped[0]+"|"+
                (1 in mapped)+"|"+mapped[2]+"|"+filtered.length+"|"+filtered[0];
        })()"#,
    ),
    (
        "String receivers map and filter UTF-16 code units",
        r#"(function(){
            var source="A\uD83D\uDCA9Z",boxed=true,indices="";
            var mapped=Array.prototype.map.call(source,function(value,index,receiver){
                boxed=boxed&&typeof receiver==="object";indices+=index;
                return value.charCodeAt(0);
            });
            var filtered=Array.prototype.filter.call(source,function(value){
                return value.charCodeAt(0)===0xDCA9;
            });
            return mapped.length+"|"+mapped[0]+"|"+mapped[1]+"|"+mapped[2]+"|"+
                mapped[3]+"|"+filtered[0].charCodeAt(0)+"|"+boxed+"|"+indices;
        })()"#,
    ),
    (
        "zero-length primitive receivers allocate base Arrays without callbacks",
        r#"(function(){
            var calls=0,callback=function(){calls++;return true};
            var mapped=Array.prototype.map.call(7,callback);
            var filtered=Array.prototype.filter.call(false,callback);
            return Array.isArray(mapped)+"|"+mapped.length+"|"+
                Array.isArray(filtered)+"|"+filtered.length+"|"+calls;
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER map fails during base Array allocation",
        r#"(function(){
            var source=Object(),log="";source.length=9007199254740991;
            source.__defineGetter__("0",function(){log+="G";return 1});
            try{Array.prototype.map.call(source,function(){log+="F";return 1});return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "MAX_SAFE_INTEGER filter can abort at its first present index",
        r#"(function(){
            var source=Object(),log="";source.length=9007199254740991;source[0]="x";
            try{Array.prototype.filter.call(source,function(value,index,receiver){
                log+=(receiver===source)+":"+index+":"+value;throw 91;
            });return "missing"}
            catch(error){return typeof error+"|"+error+"|"+log}
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "null receiver",
        "Array.prototype.map.call(null,function(){return 0})",
    ),
    (
        "undefined receiver",
        "Array.prototype.filter.call(undefined,function(){return true})",
    ),
    ("missing map callback", "[].map()"),
    ("undefined filter callback", "[].filter(undefined)"),
    ("number callback", "[].map(1)"),
    ("symbol callback", "[].filter(Symbol('callback'))"),
    ("BigInt callback", "[].map(0n)"),
    (
        "Symbol length wins before callback validation",
        "(function(){var source=Object();source.length=Symbol('length');return Array.prototype.filter.call(source,1)})()",
    ),
    (
        "primitive constructor is not a function",
        "(function(){var source=[1];source.constructor=1;return source.map(function(value){return value})})()",
    ),
    (
        "null constructor is not a function",
        "(function(){var source=[1];source.constructor=null;return source.filter(function(){return true})})()",
    ),
    (
        "primitive species is not a function",
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=Symbol('species');source.constructor=ctor;return source.map(function(value){return value})})()",
    ),
    (
        "callable non-constructor species has the pinned constructor error",
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=Array.prototype.map;source.constructor=ctor;return source.filter(function(){return true})})()",
    ),
    (
        "plain object species is not a constructor",
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=Object();source.constructor=ctor;return source.map(function(value){return value})})()",
    ),
];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','copyWithin','values','keys','entries'];
var own=Reflect.ownKeys(Array.prototype),names=[];
for(var i=0;i<own.length;i++)
  if(implemented.indexOf(own[i])>=0)names[names.length]=own[i];
function bits(descriptor) {
  return 'D'+Number(descriptor.writable)+Number(descriptor.enumerable)+Number(descriptor.configurable);
}
function metadata(name) {
  var descriptor=Object.getOwnPropertyDescriptor(Array.prototype,name),fn=descriptor.value;
  var constructable;
  try { Reflect.construct(function(){},[],fn); constructable=true; }
  catch(error) { constructable=false; }
  return name+':'+fn.name+':'+fn.length+':'+bits(descriptor)+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'name'))+':'+
    bits(Object.getOwnPropertyDescriptor(fn,'length'))+':'+
    (typeof fn==='function')+':'+(Object.getPrototypeOf(fn)===Function.prototype)+':'+constructable;
}
print('keys='+names.join(','));
print('meta='+metadata('map'));
print('meta='+metadata('filter'));
"#;

#[test]
fn array_map_filter_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array map/filter oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("values", VALUE_CASES),
        ("order", ORDER_CASES),
        ("species", SPECIES_CASES),
        ("generic", GENERIC_CASES),
        ("errors", ERROR_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 3);
}

#[test]
fn array_map_filter_values_match_pinned_quickjs() {
    compare_value_cases("Array map/filter values", VALUE_CASES);
}

#[test]
fn array_map_filter_holes_order_and_abrupt_completion_match_pinned_quickjs() {
    compare_value_cases("Array map/filter observable order", ORDER_CASES);
}

#[test]
fn array_map_filter_species_semantics_match_pinned_quickjs() {
    compare_value_cases("Array map/filter species", SPECIES_CASES);
}

#[test]
fn array_map_filter_generic_receivers_match_pinned_quickjs() {
    compare_value_cases("Array map/filter generic receivers", GENERIC_CASES);
}

#[test]
fn array_map_filter_errors_match_pinned_quickjs() {
    compare_value_cases("Array map/filter errors", ERROR_CASES);
}

#[test]
fn array_map_filter_prototype_order_and_metadata_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array map/filter graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array map/filter prototype order/metadata drifted",
    );
}

#[test]
fn array_map_filter_species_boxing_results_and_errors_use_pinned_realms() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let defining_string_prototype = defining.string_prototype().unwrap();
    let defining_type_error = eval_object(
        &mut defining,
        "TypeError.prototype",
        "defining TypeError prototype",
    );
    let caller_object_prototype = caller.object_prototype().unwrap();
    let caller_type_error = eval_object(
        &mut caller,
        "TypeError.prototype",
        "caller TypeError prototype",
    );
    let map = property_callable(&runtime, &mut defining, &defining_array_prototype, "map");
    let filter = property_callable(&runtime, &mut defining, &defining_array_prototype, "filter");
    let identity = eval_callable(
        &runtime,
        &mut caller,
        "(function(value,index,receiver){globalThis.mapReceiver=receiver;return value})",
        "caller map identity callback",
    );

    let source = eval_object(&mut caller, "[1]", "caller Array");
    let Value::Object(mapped) = caller
        .call(
            &map,
            Value::Object(source.clone()),
            &[Value::Object(identity.as_object().clone())],
        )
        .expect("cross-realm default Array.map")
    else {
        panic!("cross-realm Array.map did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&mapped).unwrap(),
        Some(defining_array_prototype.clone()),
        "cross-realm default Array constructor was not replaced by the method realm Array",
    );

    let always = eval_callable(
        &runtime,
        &mut caller,
        "(function(){return true})",
        "caller filter callback",
    );
    let Value::Object(filtered) = caller
        .call(
            &filter,
            Value::Object(source),
            &[Value::Object(always.as_object().clone())],
        )
        .expect("cross-realm default Array.filter")
    else {
        panic!("cross-realm Array.filter did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&filtered).unwrap(),
        Some(defining_array_prototype),
        "cross-realm default Array.filter result used the caller realm",
    );

    caller
        .call(
            &map,
            Value::String(JsString::try_from_utf8("a").unwrap()),
            &[Value::Object(identity.as_object().clone())],
        )
        .expect("cross-realm primitive Array.map");
    let boxed = eval_object(&mut caller, "mapReceiver", "captured map receiver");
    assert_eq!(
        runtime.get_prototype_of(&boxed).unwrap(),
        Some(defining_string_prototype),
        "Array.map boxed its primitive receiver in the callback realm",
    );

    caller
        .eval("globalThis.callbackObject=Object()")
        .expect("install caller callback object");
    let object_callback = eval_callable(
        &runtime,
        &mut caller,
        "(function(){return callbackObject})",
        "caller object-producing callback",
    );
    let one = eval_object(&mut caller, "[1]", "caller callback source");
    let Value::Object(result) = caller
        .call(
            &map,
            Value::Object(one),
            &[Value::Object(object_callback.as_object().clone())],
        )
        .expect("cross-realm object-valued Array.map")
    else {
        panic!("object-valued Array.map did not return an object");
    };
    let zero = runtime.intern_property_key("0").unwrap();
    let Value::Object(callback_result) = caller.get_property(&result, &zero).unwrap() else {
        panic!("Array.map did not retain the callback object");
    };
    assert_eq!(
        runtime.get_prototype_of(&callback_result).unwrap(),
        Some(caller_object_prototype.clone()),
        "Array.map moved a callback result into the method defining realm",
    );
    let custom_source = eval_object(
        &mut caller,
        "(function(){var source=[1],ctor=Object();ctor[Symbol.species]=function(length){globalThis.crossSpeciesLength=length;return Object()};source.constructor=ctor;return source})()",
        "caller Array with custom species",
    );
    let Value::Object(custom_result) = caller
        .call(
            &map,
            Value::Object(custom_source),
            &[Value::Object(identity.as_object().clone())],
        )
        .expect("cross-realm custom species Array.map")
    else {
        panic!("custom species Array.map did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&custom_result).unwrap(),
        Some(caller_object_prototype),
        "Array.map moved a custom species result into the method realm",
    );
    assert_eq!(
        caller.eval("crossSpeciesLength").unwrap(),
        Value::Int(1),
        "Array.map passed the wrong length to cross-realm species",
    );

    let bad = eval_object(
        &mut caller,
        "(function(){var source=[1];source.constructor=1;return source})()",
        "caller Array with primitive constructor",
    );
    assert!(matches!(
        caller.call(
            &map,
            Value::Object(bad),
            &[Value::Object(identity.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.map species TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.map species TypeError did not use the method defining realm",
    );

    let throwing = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw new TypeError('caller callback')})",
        "caller throwing map callback",
    );
    let throwing_source = eval_object(&mut caller, "[1]", "caller throwing source");
    assert!(matches!(
        caller.call(
            &map,
            Value::Object(throwing_source),
            &[Value::Object(throwing.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.map callback TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.map replaced a callback throw with a defining-realm error",
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
    for name in ["map", "filter"] {
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
            panic!("could not run QuickJS Array map/filter graph oracle: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS Array map/filter graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array map/filter graph output was not UTF-8")
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
        .unwrap_or_else(|| panic!("Rust {description} was not callable"))
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
