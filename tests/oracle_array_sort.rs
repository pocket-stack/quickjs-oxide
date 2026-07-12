use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, ObjectRef, Runtime, RuntimeError,
    Value,
};

// This target pins QuickJS 2026-06-04's ordinary Array sort kernel and its
// change-by-copy wrapper. Stateful probes intentionally lock rqsort comparison
// order and QuickJS's write-back shortcuts, not merely the final permutation.

const VALIDATION_CASES: &[(&str, &str)] = &[
    (
        "sort validates comparefn before ToObject and length",
        r#"(function(){
            var source=Object(),log="";
            source.__defineGetter__("length",function(){log+="L";return 0});
            var first,second;
            try{Array.prototype.sort.call(source,1);first="missing"}
            catch(error){first=error.name+":"+error.message}
            try{Array.prototype.sort.call(null,1);second="missing"}
            catch(error){second=error.name+":"+error.message}
            return first+"|"+second+"|"+log;
        })()"#,
    ),
    (
        "toSorted validates comparefn before ToObject and length",
        r#"(function(){
            var source=Object(),log="";
            source.__defineGetter__("length",function(){log+="L";return 0});
            var first,second;
            try{Array.prototype.toSorted.call(source,false);first="missing"}
            catch(error){first=error.name+":"+error.message}
            try{Array.prototype.toSorted.call(undefined,false);second="missing"}
            catch(error){second=error.name+":"+error.message}
            return first+"|"+second+"|"+log;
        })()"#,
    ),
    (
        "omitted and explicit undefined select the default comparator",
        r#"(function(){
            var first=[2,10,1],second=[2,10,1];
            first.sort();second.sort(undefined);
            return first[0]+","+first[1]+","+first[2]+"|"+
                second[0]+","+second[1]+","+second[2];
        })()"#,
    ),
];

const DEFAULT_CASES: &[(&str, &str)] = &[
    (
        "default sort compares numeric-looking values lexicographically",
        r#"(function(){
            var source=[20,3,100,11],same=source.sort()===source;
            var copy=source.toSorted();
            return same+"|"+source[0]+","+source[1]+","+source[2]+","+source[3]+"|"+
                copy[0]+","+copy[1]+","+copy[2]+","+copy[3];
        })()"#,
    ),
    (
        "default comparison uses UTF16 code units including lone surrogates",
        r#"(function(){
            var source=["\uE000","\uD800","a","\uDC00","\uD7FF"];
            var result=source.toSorted(),codes="",i;
            for(i=0;i<result.length;i++)codes+=(i?",":"")+result[i].charCodeAt(0);
            return codes+"|"+source[0].charCodeAt(0)+"|"+result.length;
        })()"#,
    ),
    (
        "sort places defined values then undefined then holes",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=Array(6);source[0]=undefined;source[2]="b";source[4]="a";
            source.sort();
            return source.length+"|"+source[0]+"|"+source[1]+"|"+
                (source[2]===undefined)+"|"+own(source,2)+"|"+
                own(source,3)+own(source,4)+own(source,5);
        })()"#,
    ),
    (
        "toSorted densifies both undefined values and source holes",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=Array(6);source[0]=undefined;source[2]="b";source[4]="a";
            var result=source.toSorted(),bits="",i;
            for(i=0;i<result.length;i++)bits+=Number(own(result,i))+Number(result[i]===undefined);
            return result[0]+"|"+result[1]+"|"+bits+"|"+
                own(source,1)+own(source,3)+own(source,5);
        })()"#,
    ),
    (
        "default ToString is cached per slot but not skipped for the same object",
        r#"(function(){
            var value=Object(),log="";
            value.toString=function(){log+="T";return "x"};
            var source=[value,value];source.sort();
            return log+"|"+(source[0]===value)+"|"+(source[1]===value);
        })()"#,
    ),
];

const CUSTOM_CASES: &[(&str, &str)] = &[
    (
        "stable custom ties preserve original object order",
        r#"(function(){
            var a=Object(),b=Object(),c=Object(),d=Object();
            a.id="a";a.key=1;b.id="b";b.key=0;c.id="c";c.key=1;d.id="d";d.key=0;
            var source=[a,b,c,d],result=source.toSorted(function(x,y){return x.key-y.key});
            return result[0].id+result[1].id+result[2].id+result[3].id+"|"+
                source[0].id+source[1].id+source[2].id+source[3].id;
        })()"#,
    ),
    (
        "pinned rqsort comparator order for three values",
        r#"(function(){
            var source=[3,2,1],log="";
            source.sort(function(a,b){log+=a+":"+b+",";return a-b});
            return log+"|"+source[0]+source[1]+source[2];
        })()"#,
    ),
    (
        "pinned rqsort comparator order for seven values",
        r#"(function(){
            var source=[7,6,5,4,3,2,1],log="";
            source.sort(function(a,b){log+=a+":"+b+",";return a-b});
            return log+"|"+source[0]+source[1]+source[2]+source[3]+source[4]+source[5]+source[6];
        })()"#,
    ),
    (
        "raw-identical custom values bypass comparator calls",
        r#"(function(){
            var object=Object(),string="same",source=[object,object,1,1,string,string],log="";
            source.sort(function(a,b){
                function tag(value){return value===object?"object":typeof value+":"+(""+value)}
                log+=tag(a)+">"+tag(b)+",";return 0;
            });
            return log+"|"+(source[0]===object)+"|"+source[2]+"|"+source[4];
        })()"#,
    ),
    (
        "atomized literals share raw identity while tagged integers and dynamic strings do not",
        r#"(function(){
            function calls(a,b){var count=0;[a,b].sort(function(){count++;return 0});return count}
            function left(){return "same"}function right(){return "same"}
            function numeric(){return "1"}
            var a="sa",b="me";
            return calls("same","same")+"|"+calls(left(),right())+"|"+
                calls("1","1")+"|"+calls(numeric(),numeric())+"|"+
                calls("2147483648","2147483648")+"|"+calls(`same`,`same`)+"|"+
                calls(`1`,`1`)+"|"+calls(a+b,a+b);
        })()"#,
    ),
    (
        "custom result objects are converted with numeric hint",
        r#"(function(){
            var result=Object(),log="",source=[2,1];
            result.valueOf=function(){log+="N";return -1};
            result.toString=function(){log+="T";return "1"};
            source.sort(function(a,b){log+="C"+a+b;return result});
            return log+"|"+source[0]+","+source[1];
        })()"#,
    ),
    (
        "NaN comparator results are stable ties",
        r#"(function(){
            var source=[3,1,2],log="";
            source.sort(function(a,b){log+=a+":"+b+",";return 0/0});
            return source[0]+","+source[1]+","+source[2]+"|"+log;
        })()"#,
    ),
];

const ORDER_AND_PARTIAL_CASES: &[(&str, &str)] = &[
    (
        "collect compare defined Sets undefined Sets and Delete run in pinned order",
        r#"(function(){
            function own(object,key){return Object.prototype.hasOwnProperty.call(object,key)}
            var source=Object(),proto=Object(),descriptor=Object(),log="";
            Object.setPrototypeOf(source,proto);source.length=5;
            descriptor.get=function(){log+="G0";return "b"};
            descriptor.set=function(value){log+="S0"+value};descriptor.configurable=true;
            Object.defineProperty(source,"0",descriptor);
            descriptor=Object();descriptor.get=function(){log+="G1";return undefined};
            descriptor.set=function(value){log+="S1"+value};descriptor.configurable=true;
            Object.defineProperty(source,"1",descriptor);
            proto.__defineSetter__("2",function(value){log+="U2"+(value===undefined)});
            descriptor=Object();descriptor.get=function(){log+="G3";return "a"};
            descriptor.set=function(value){log+="S3"+(value===undefined)};descriptor.configurable=true;
            Object.defineProperty(source,"3",descriptor);
            Array.prototype.sort.call(source,function(a,b){log+="C";source[4]="made";return a<b?-1:1});
            return log+"|"+own(source,2)+"|"+own(source,4)+"|"+source.length;
        })()"#,
    ),
    (
        "unchanged original position skips Set while undefined does not",
        r#"(function(){
            var first=Object(),second=Object(),descriptor=Object(),log="";
            first.length=1;descriptor.get=function(){log+="G";return "a"};
            descriptor.set=function(value){log+="S"};descriptor.configurable=true;
            Object.defineProperty(first,"0",descriptor);Array.prototype.sort.call(first);
            descriptor=Object();second.length=1;
            descriptor.get=function(){log+="U";return undefined};
            descriptor.set=function(value){log+="W"+(value===undefined)};descriptor.configurable=true;
            Object.defineProperty(second,"0",descriptor);Array.prototype.sort.call(second);
            return log;
        })()"#,
    ),
    (
        "failed defined Set preserves an earlier moved value",
        r#"(function(){
            var source=Object(),descriptor=Object();source.length=3;source[0]="c";source[2]="b";
            descriptor.value="a";descriptor.writable=false;
            descriptor.enumerable=true;descriptor.configurable=false;
            Object.defineProperty(source,"1",descriptor);
            try{Array.prototype.sort.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source[0]+source[1]+source[2]}
        })()"#,
    ),
    (
        "failed undefined Set preserves completed defined writes",
        r#"(function(){
            var source=Object(),descriptor=Object();source.length=3;source[0]="b";source[1]="a";
            descriptor.value=undefined;descriptor.writable=false;
            descriptor.enumerable=true;descriptor.configurable=false;
            Object.defineProperty(source,"2",descriptor);
            try{Array.prototype.sort.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+source[0]+source[1]+"|"+(source[2]===undefined)}
        })()"#,
    ),
    (
        "failed Delete preserves defined and undefined writes",
        r#"(function(){
            var source=Object(),descriptor=Object(),installed=false;
            source.length=4;source[0]="b";source[1]="a";source[2]=undefined;
            try{
                Array.prototype.sort.call(source,function(a,b){
                    if(!installed){installed=true;descriptor.value="fixed";descriptor.writable=true;
                        descriptor.enumerable=true;descriptor.configurable=false;
                        Object.defineProperty(source,"3",descriptor)}
                    return a<b?-1:1;
                });return "missing";
            }catch(error){
                return error.name+"|"+error.message+"|"+source[0]+source[1]+"|"+
                    (source[2]===undefined)+"|"+source[3];
            }
        })()"#,
    ),
    (
        "getters and comparator mutate within a snapshotted length",
        r#"(function(){
            var source=Object(),descriptor=Object(),log="",written;
            source.length=3;source[2]="a";
            descriptor.get=function(){log+="G";source[1]="b";source[4]="outside";source.length=5;return "c"};
            descriptor.set=function(value){log+="S";written=value};descriptor.configurable=true;
            Object.defineProperty(source,"0",descriptor);
            Array.prototype.sort.call(source,function(a,b){log+="C";source[2]="changed";return a<b?-1:1});
            return log+"|"+source.length+"|"+written+"|"+source[1]+"|"+source[2]+"|"+source[4];
        })()"#,
    ),
];

const GENERIC_AND_LIMIT_CASES: &[(&str, &str)] = &[
    (
        "toSorted ignores constructor and species",
        r#"(function(){
            var source=[2,1],ctor=Object(),log="";
            ctor.__defineGetter__(Symbol.species,function(){log+="S";throw 71});
            source.constructor=ctor;
            var result=source.toSorted();
            var other=[4,3];other.__defineGetter__("constructor",function(){log+="C";throw 72});
            var second=other.toSorted();
            return result[0]+result[1]+"|"+second[0]+second[1]+"|"+log+"|"+Array.isArray(result);
        })()"#,
    ),
    (
        "primitive receivers are boxed or copied",
        r#"(function(){
            var boxed=Array.prototype.sort.call(7),copy=Array.prototype.toSorted.call(false);
            return Object.prototype.toString.call(boxed)+"|"+boxed.valueOf()+"|"+
                Array.isArray(copy)+"|"+copy.length;
        })()"#,
    ),
    (
        "String toSorted copies code units while sort rejects immutable writes",
        r#"(function(){
            var errorText;
            try{Array.prototype.sort.call("ba");errorText="missing"}
            catch(error){errorText=error.name+":"+error.message}
            var result=Array.prototype.toSorted.call("cba");
            return errorText+"|"+result[0]+result[1]+result[2]+"|"+result.length;
        })()"#,
    ),
    (
        "toSorted INT32 limit fails before indexed reads",
        r#"(function(){
            var source=Object(),log="";source.length=2147483648;
            source.__defineGetter__("0",function(){log+="G";throw 73});
            try{Array.prototype.toSorted.call(source);return "missing"}
            catch(error){return error.name+"|"+error.message+"|"+log}
        })()"#,
    ),
    (
        "sort accepts MAX_SAFE length and begins at full u64 index zero",
        r#"(function(){
            var source=Object(),log="";source.length=9007199254740991;
            source.__defineGetter__("0",function(){log+="G";throw 74});
            try{Array.prototype.sort.call(source);return "missing"}
            catch(error){return typeof error+"|"+(""+error)+"|"+log}
        })()"#,
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    (
        "Symbol comparator result",
        "[2,1].sort(function(){return Symbol('order')})",
    ),
    (
        "BigInt comparator result",
        "[2,1].sort(function(){return 1n})",
    ),
    (
        "default Symbol element ToString",
        "[Symbol('value'),'a'].sort()",
    ),
    (
        "arbitrary comparator throw",
        "[2,1].sort(function(){throw 'compare boom'})",
    ),
    (
        "source getter user throw before comparison",
        "(function(){var source=Object();source.length=2;source.__defineGetter__('0',function(){throw 81});return Array.prototype.sort.call(source,function(){throw 82})})()",
    ),
];

const RECURSION_CASES: &[(&str, &str)] = &[(
    "recursive comparator produces catchable stack overflow",
    r#"(function(){
        function compare(){[2,1].sort(compare);return 0}
        try{[2,1].sort(compare);return "missing"}
        catch(error){return error.name+"|"+error.message}
    })()"#,
)];

const GRAPH_ORACLE: &str = r#"
var implemented=['at','with','concat','every','some','forEach','map','filter','reduce','reduceRight','fill','find','findIndex','findLast','findLastIndex','indexOf','lastIndexOf','includes','join','toString','toLocaleString','pop','push','shift','unshift','reverse','toReversed','sort','toSorted','copyWithin','values','keys','entries'];
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
print('meta='+metadata('sort'));
print('meta='+metadata('toSorted'));
"#;

#[test]
fn array_sort_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array sort oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("validation", VALIDATION_CASES),
        ("default", DEFAULT_CASES),
        ("custom", CUSTOM_CASES),
        ("order/partial", ORDER_AND_PARTIAL_CASES),
        ("generic/limits", GENERIC_AND_LIMIT_CASES),
        ("errors", ERROR_CASES),
        ("recursion", RECURSION_CASES),
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
fn array_sort_comparefn_validation_order_matches_pinned_quickjs() {
    compare_value_cases("Array sort validation", VALIDATION_CASES);
}

#[test]
fn array_sort_default_utf16_holes_and_lazy_stringification_match_pinned_quickjs() {
    compare_value_cases("Array sort default comparison", DEFAULT_CASES);
}

#[test]
fn array_sort_stability_rqsort_order_raw_identity_and_to_number_match_pinned_quickjs() {
    compare_value_cases("Array sort custom comparison", CUSTOM_CASES);
}

#[test]
fn array_sort_collect_write_delete_and_partial_mutation_match_pinned_quickjs() {
    compare_value_cases("Array sort writeback order", ORDER_AND_PARTIAL_CASES);
}

#[test]
fn array_sort_species_generic_string_and_limits_match_pinned_quickjs() {
    compare_value_cases(
        "Array sort generic receivers/limits",
        GENERIC_AND_LIMIT_CASES,
    );
}

#[test]
fn array_sort_conversion_and_user_errors_match_pinned_quickjs() {
    compare_value_cases("Array sort errors", ERROR_CASES);
}

#[test]
fn array_sort_recursive_comparator_stack_overflow_is_catchable() {
    compare_value_cases("Array sort recursive comparator", RECURSION_CASES);
}

#[test]
fn array_sort_prototype_order_metadata_and_constructability_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Array sort graph differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
        "Array sort prototype order/metadata drifted",
    );
}

#[test]
fn array_sort_cross_realm_results_boxing_native_and_user_errors_match_quickjs() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_array_prototype = defining.array_prototype().unwrap();
    let caller_array_prototype = caller.array_prototype().unwrap();
    let defining_number_prototype = eval_object(
        &mut defining,
        "Number.prototype",
        "defining Number prototype",
    );
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
    let sort = property_callable(&runtime, &mut defining, &defining_array_prototype, "sort");
    let to_sorted = property_callable(
        &runtime,
        &mut defining,
        &defining_array_prototype,
        "toSorted",
    );

    let Value::Object(boxed) = caller
        .call(&sort, Value::Int(7), &[])
        .expect("cross-realm primitive Array.sort")
    else {
        panic!("cross-realm primitive Array.sort did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&boxed).unwrap(),
        Some(defining_number_prototype),
        "Array.sort boxed outside the method defining realm",
    );

    let receiver = eval_object(&mut caller, "[3,1,2]", "caller Array receiver");
    let Value::Object(result) = caller
        .call(&to_sorted, Value::Object(receiver), &[])
        .expect("cross-realm Array.toSorted")
    else {
        panic!("cross-realm Array.toSorted did not return an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining_array_prototype.clone()),
        "Array.toSorted result did not use the method defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(caller_array_prototype),
    );
    assert_eq!(int_property(&runtime, &mut caller, &result, "0"), 1);
    assert_eq!(int_property(&runtime, &mut caller, &result, "2"), 3);

    assert!(matches!(
        caller.call(&sort, Value::Null, &[Value::Int(1)]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.sort comparefn TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error.clone()),
        "Array.sort comparefn TypeError did not use the method defining realm",
    );

    let too_long = eval_object(
        &mut caller,
        "(function(){var source=Object();source.length=2147483648;return source})()",
        "caller oversized array-like",
    );
    assert!(matches!(
        caller.call(&to_sorted, Value::Object(too_long), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.toSorted RangeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_range_error),
        "Array.toSorted RangeError did not use the method defining realm",
    );

    let fixed_tail = eval_object(
        &mut caller,
        "(function(){var source=Object(),descriptor=Object();source.length=2;descriptor.value='tail';descriptor.writable=true;descriptor.enumerable=true;descriptor.configurable=false;Object.defineProperty(source,'1',descriptor);return source})()",
        "caller non-configurable sort tail",
    );
    assert!(matches!(
        caller.call(&sort, Value::Object(fixed_tail), &[]),
        Err(RuntimeError::Exception),
    ));
    let native_error = take_exception_object(&mut caller, "Array.sort Delete TypeError");
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(defining_type_error),
        "Array.sort Delete TypeError did not use the method defining realm",
    );

    let comparator = eval_callable(
        &runtime,
        &mut caller,
        "(function(){throw new TypeError('caller comparator')})",
        "caller throwing comparator",
    );
    let receiver = eval_object(&mut caller, "[2,1]", "caller comparator receiver");
    assert!(matches!(
        caller.call(
            &sort,
            Value::Object(receiver),
            &[Value::Object(comparator.as_object().clone())],
        ),
        Err(RuntimeError::Exception),
    ));
    let user_error = take_exception_object(&mut caller, "Array.sort user comparator TypeError");
    assert_eq!(
        runtime.get_prototype_of(&user_error).unwrap(),
        Some(caller_type_error),
        "Array.sort replaced a user comparator throw with a defining-realm error",
    );
}

#[test]
fn array_sort_atom_literal_identity_survives_publication_and_context_boundaries() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    assert_eq!(
        caller
            .eval(
                r#"(function(){
                    function calls(a,b){var count=0;[a,b].sort(function(){count++;return 0});return count}
                    function left(){return "same"}function right(){return "same"}
                    function numeric(){return "1"}
                    var a="sa",b="me";
                    return calls("same","same")+"|"+calls(left(),right())+"|"+
                        calls("1","1")+"|"+calls(numeric(),numeric())+"|"+
                        calls("2147483648","2147483648")+"|"+calls(`same`,`same`)+"|"+
                        calls(`1`,`1`)+"|"+calls(a+b,a+b);
                })()"#,
            )
            .unwrap(),
        Value::String(
            quickjs_oxide::JsString::try_from_utf8("0|0|1|0|0|0|1|1").unwrap(),
        ),
    );
    let array_prototype = defining.array_prototype().unwrap();
    let sort = property_callable(&runtime, &mut defining, &array_prototype, "sort");
    caller.eval("var sortCalls=0").unwrap();
    let comparator = eval_callable(
        &runtime,
        &mut caller,
        "(function(){sortCalls++;return 0})",
        "literal identity comparator",
    );
    let comparator = Value::Object(comparator.as_object().clone());

    let first = defining.eval("'shared across contexts'").unwrap();
    let second = caller.eval("'shared across contexts'").unwrap();
    let receiver = caller.new_array_from_values(vec![first, second]).unwrap();
    caller
        .call(
            &sort,
            Value::Object(receiver),
            std::slice::from_ref(&comparator),
        )
        .unwrap();
    assert_eq!(caller.eval("sortCalls").unwrap(), Value::Int(0));

    caller.eval("sortCalls=0").unwrap();
    let literal = caller.eval("'1'").unwrap();
    let property = runtime.intern_property_key("1").unwrap();
    let property = Value::String(runtime.property_key_to_js_string(&property).unwrap());
    let receiver = caller
        .new_array_from_values(vec![literal, property])
        .unwrap();
    caller
        .call(
            &sort,
            Value::Object(receiver),
            std::slice::from_ref(&comparator),
        )
        .unwrap();
    assert_eq!(caller.eval("sortCalls").unwrap(), Value::Int(1));

    caller.eval("sortCalls=0").unwrap();
    let literal = caller.eval("'shared property spelling'").unwrap();
    let property = runtime
        .intern_property_key("shared property spelling")
        .unwrap();
    let property = Value::String(runtime.property_key_to_js_string(&property).unwrap());
    let receiver = caller
        .new_array_from_values(vec![literal, property])
        .unwrap();
    caller
        .call(
            &sort,
            Value::Object(receiver),
            std::slice::from_ref(&comparator),
        )
        .unwrap();
    assert_eq!(caller.eval("sortCalls").unwrap(), Value::Int(0));

    caller.eval("sortCalls=0").unwrap();
    let first = defining.eval("'1'").unwrap();
    let second = caller.eval("'1'").unwrap();
    let receiver = caller.new_array_from_values(vec![first, second]).unwrap();
    caller
        .call(&sort, Value::Object(receiver), &[comparator])
        .unwrap();
    assert_eq!(caller.eval("sortCalls").unwrap(), Value::Int(1));
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
    for name in ["sort", "toSorted"] {
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
        .unwrap_or_else(|error| panic!("could not run QuickJS Array sort graph oracle: {error}"));
    assert!(
        output.status.success(),
        "QuickJS Array sort graph oracle failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS Array sort graph output was not UTF-8")
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
    let function = eval_object(context, source, description);
    runtime
        .as_callable(&function)
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
