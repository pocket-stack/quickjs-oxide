use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, CompleteOrdinaryPropertyDescriptor, Context, JsString, ObjectRef, PropertyKey,
    Runtime, RuntimeError, Value,
};

// Pins QuickJS 2026-06-04 `js_object_seal` and `js_object_isSealed` for
// Object.seal/freeze/isSealed/isFrozen. Proxy trap invariants, non-empty
// TypedArray freeze failures and module namespace objects are recorded as
// oracle-only or explicit boundaries; mapped and unmapped Arguments integrity
// semantics are locked by their dedicated differential.

const PRIMITIVE_CASES: &[(&str, &str)] = &[
    (
        "seal and freeze preserve every primitive without boxing",
        r#"(function(){
            var symbol=Symbol("integrity");
            return [
                Object.seal()===undefined,Object.freeze()===undefined,
                Object.seal(null)===null,Object.freeze(null)===null,
                Object.seal(false)===false,Object.freeze(true)===true,
                Object.seal(17)===17,Object.freeze(19)===19,
                Object.is(Object.seal(-0),-0),Object.is(Object.freeze(NaN),NaN),
                Object.seal("ab")==="ab",Object.freeze("cd")==="cd",
                Object.seal(23n)===23n,Object.freeze(29n)===29n,
                Object.seal(symbol)===symbol,Object.freeze(symbol)===symbol
            ].join(":");
        })()"#,
    ),
    (
        "integrity predicates report every primitive including missing arguments true",
        r#"(function(){
            var symbol=Symbol("query");
            return [
                Object.isSealed(),Object.isFrozen(),
                Object.isSealed(undefined),Object.isFrozen(undefined),
                Object.isSealed(null),Object.isFrozen(null),
                Object.isSealed(false),Object.isFrozen(true),
                Object.isSealed(0),Object.isFrozen(-0),
                Object.isSealed("x"),Object.isFrozen("x"),
                Object.isSealed(1n),Object.isFrozen(1n),
                Object.isSealed(symbol),Object.isFrozen(symbol)
            ].join(":");
        })()"#,
    ),
];

const DESCRIPTOR_CASES: &[(&str, &str)] = &[
    (
        "seal preserves data writability and accessor identity while clearing configurability",
        r#"(function(){
            function dataBits(descriptor){return descriptor.value+":"+descriptor.writable+":"+descriptor.enumerable+":"+descriptor.configurable}
            var calls=0,target=Object(),data=Object(),accessor=Object();
            function getter(){calls++;throw "getter"}function setter(value){calls+=value}
            data.value=17;data.writable=true;data.enumerable=true;data.configurable=true;
            accessor.get=getter;accessor.set=setter;accessor.enumerable=false;accessor.configurable=true;
            Object.defineProperty(target,"data",data);Object.defineProperty(target,"accessor",accessor);
            var returned=Object.seal(target),dataAfter=Object.getOwnPropertyDescriptor(target,"data");
            var accessorAfter=Object.getOwnPropertyDescriptor(target,"accessor");
            return (returned===target)+"|"+dataBits(dataAfter)+"|"+
                (accessorAfter.get===getter)+":"+(accessorAfter.set===setter)+":"+
                accessorAfter.enumerable+":"+accessorAfter.configurable+":"+calls+"|"+
                Object.isSealed(target)+":"+Object.isFrozen(target)+":"+Object.isExtensible(target);
        })()"#,
    ),
    (
        "freeze preserves values and accessor identity while making data non writable",
        r#"(function(){
            var calls=0,payload=Object(),target=Object(),data=Object(),locked=Object(),accessor=Object();
            function getter(){calls++;throw "getter"}function setter(value){calls+=value}
            data.value=payload;data.writable=true;data.enumerable=true;data.configurable=true;
            locked.value=31;locked.writable=false;locked.enumerable=false;locked.configurable=true;
            accessor.get=getter;accessor.set=setter;accessor.enumerable=true;accessor.configurable=true;
            Object.defineProperty(target,"data",data);Object.defineProperty(target,"locked",locked);
            Object.defineProperty(target,"accessor",accessor);
            var returned=Object.freeze(target),d=Object.getOwnPropertyDescriptor(target,"data");
            var l=Object.getOwnPropertyDescriptor(target,"locked"),a=Object.getOwnPropertyDescriptor(target,"accessor");
            return (returned===target)+"|"+(d.value===payload)+":"+d.writable+":"+d.enumerable+":"+d.configurable+"|"+
                l.value+":"+l.writable+":"+l.enumerable+":"+l.configurable+"|"+
                (a.get===getter)+":"+(a.set===setter)+":"+a.enumerable+":"+a.configurable+":"+calls+"|"+
                Object.isSealed(target)+":"+Object.isFrozen(target)+":"+Object.isExtensible(target);
        })()"#,
    ),
    (
        "nonenumerable and Symbol own keys are tightened while inherited properties are untouched",
        r#"(function(){
            var proto=Object(),target=Object.create(proto),hidden=Object(),symbol=Symbol("own");
            proto.inherited=1;target[symbol]=2;
            hidden.value=3;hidden.writable=true;hidden.enumerable=false;hidden.configurable=true;
            Object.defineProperty(target,"hidden",hidden);Object.freeze(target);
            var ownSymbol=Object.getOwnPropertyDescriptor(target,symbol);
            var ownHidden=Object.getOwnPropertyDescriptor(target,"hidden");
            var inherited=Object.getOwnPropertyDescriptor(proto,"inherited");
            return ownSymbol.value+":"+ownSymbol.writable+":"+ownSymbol.enumerable+":"+ownSymbol.configurable+"|"+
                ownHidden.value+":"+ownHidden.writable+":"+ownHidden.enumerable+":"+ownHidden.configurable+"|"+
                inherited.writable+":"+inherited.enumerable+":"+inherited.configurable+"|"+
                (Object.getOwnPropertyDescriptor(target,"inherited")===undefined);
        })()"#,
    ),
    (
        "Array and String exotic own properties receive their exact integrity flags",
        r#"(function(){
            function bits(object,key){var d=Object.getOwnPropertyDescriptor(object,key);return d.writable+":"+d.enumerable+":"+d.configurable}
            var sealed=[];sealed[0]="A";sealed[2]="C";sealed.extra="E";Object.seal(sealed);
            var frozen=[];frozen[0]="F";frozen.extra="X";Object.freeze(frozen);
            var string=Object("ab");string.extra="S";Object.freeze(string);
            return Object.getOwnPropertyNames(sealed).join(",")+"|"+
                bits(sealed,"0")+":"+bits(sealed,"2")+":"+bits(sealed,"length")+":"+bits(sealed,"extra")+"|"+
                bits(frozen,"0")+":"+bits(frozen,"length")+":"+bits(frozen,"extra")+"|"+
                Object.getOwnPropertyNames(string).join(",")+"|"+
                bits(string,"0")+":"+bits(string,"1")+":"+bits(string,"length")+":"+bits(string,"extra")+"|"+
                Object.isSealed(sealed)+":"+Object.isFrozen(sealed)+":"+
                Object.isSealed(frozen)+":"+Object.isFrozen(frozen)+":"+Object.isFrozen(string);
        })()"#,
    ),
];

const MUTATION_CASES: &[(&str, &str)] = &[
    (
        "seal then freeze is monotonic idempotent and preserves allowed writes",
        r#"(function(){
            var object=Object();object.value=1;
            var before=Object.isExtensible(object)+":"+Object.isSealed(object)+":"+Object.isFrozen(object);
            var firstSeal=Object.seal(object),secondSeal=Object.seal(object);
            object.value=2;object.added=3;var deleted=delete object.value;
            var afterSeal=Object.isExtensible(object)+":"+Object.isSealed(object)+":"+Object.isFrozen(object)+":"+
                object.value+":"+(object.added===undefined)+":"+deleted;
            var firstFreeze=Object.freeze(object),secondFreeze=Object.freeze(object);object.value=4;
            var descriptor=Object.getOwnPropertyDescriptor(object,"value");
            return before+"|"+(firstSeal===object)+":"+(secondSeal===object)+"|"+afterSeal+"|"+
                (firstFreeze===object)+":"+(secondFreeze===object)+"|"+
                object.value+":"+descriptor.writable+":"+descriptor.configurable+":"+
                Object.isSealed(object)+":"+Object.isFrozen(object);
        })()"#,
    ),
    (
        "preventExtensions alone exposes all four predicate combinations allowed for ordinary data",
        r#"(function(){
            function row(value){return Object.isSealed(value)+":"+Object.isFrozen(value)}
            var empty=Object();Object.preventExtensions(empty);
            var configurable=Object();configurable.x=1;Object.preventExtensions(configurable);
            var writable=Object(),wd=Object();wd.value=2;wd.writable=true;wd.enumerable=true;wd.configurable=false;
            Object.defineProperty(writable,"x",wd);Object.preventExtensions(writable);
            var frozen=Object(),fd=Object();fd.value=3;fd.writable=false;fd.enumerable=true;fd.configurable=false;
            Object.defineProperty(frozen,"x",fd);Object.preventExtensions(frozen);
            var accessor=Object(),ad=Object();ad.get=function(){throw "unused"};ad.enumerable=true;ad.configurable=false;
            Object.defineProperty(accessor,"x",ad);Object.preventExtensions(accessor);
            return row(empty)+"|"+row(configurable)+"|"+row(writable)+"|"+row(frozen)+"|"+row(accessor);
        })()"#,
    ),
    (
        "sealed Array elements remain writable but additions and deletion are rejected",
        r#"(function(){
            var array=[];array[0]="A";array[2]="C";Object.seal(array);
            array[0]="B";array[1]="new";array[3]="late";var removed=delete array[2];
            var beforeFreeze=array[0]+":"+(array[1]===undefined)+":"+(array[3]===undefined)+":"+removed+":"+array.length;
            Object.freeze(array);array[0]="D";
            return beforeFreeze+"|"+array[0]+":"+Object.getOwnPropertyDescriptor(array,"length").writable+":"+
                Object.isSealed(array)+":"+Object.isFrozen(array);
        })()"#,
    ),
    (
        "integrity operations and predicates never execute stored accessors",
        r#"(function(){
            var calls=0,sealed=Object(),frozen=Object();
            sealed.__defineGetter__("x",function(){calls++;throw "sealed getter"});
            frozen.__defineGetter__("x",function(){calls++;throw "frozen getter"});
            var a=Object.seal(sealed),b=Object.isSealed(sealed),c=Object.isFrozen(sealed);
            var d=Object.freeze(frozen),e=Object.isSealed(frozen),f=Object.isFrozen(frozen);
            return (a===sealed)+":"+b+":"+c+":"+(d===frozen)+":"+e+":"+f+":"+calls;
        })()"#,
    ),
];

// These are pinned-oracle-only because the current Rust runtime intentionally
// has none of the object families which can make an integrity operation fail
// after it has already changed observable state. Ordinary, Array and String
// transitions above are all valid descriptor tightenings and cannot produce a
// user-controlled mid-loop failure.
const EXOTIC_ORACLE_ONLY_CASES: &[(&str, &str)] = &[
    (
        "Proxy defineProperty failure leaves preventExtensions and earlier keys applied",
        r#"(function(){
            var log="",base=Object(),a=Object(),b=Object();
            a.value=1;a.writable=true;a.enumerable=true;a.configurable=true;
            b.value=2;b.writable=true;b.enumerable=true;b.configurable=true;
            Object.defineProperty(base,"a",a);Object.defineProperty(base,"b",b);
            var handler={
                preventExtensions:function(target){log+="prevent|";return Reflect.preventExtensions(target)},
                ownKeys:function(){log+="keys|";return ["a","b"]},
                defineProperty:function(target,key,descriptor){log+="define-"+key+"|";if(key==="b")return false;return Reflect.defineProperty(target,key,descriptor)}
            };
            var proxy=new Proxy(base,handler);
            try{Object.seal(proxy)}catch(error){
                return error.name+"|"+log+"|"+Object.isExtensible(base)+":"+
                    Object.getOwnPropertyDescriptor(base,"a").configurable+":"+
                    Object.getOwnPropertyDescriptor(base,"b").configurable;
            }
            return "missing";
        })()"#,
    ),
    (
        "freezing a nonempty TypedArray fails after making it nonextensible",
        r#"(function(){
            var value=new Uint8Array([7,8]);
            try{Object.freeze(value)}catch(error){
                var descriptor=Object.getOwnPropertyDescriptor(value,"0");
                return error.name+":"+Object.isExtensible(value)+":"+descriptor.value+":"+
                    descriptor.writable+":"+descriptor.configurable;
            }
            return "missing";
        })()"#,
    ),
    (
        "Proxy integrity predicate trap order can return before isExtensible",
        r#"(function(){
            var log="",base=Object();base.x=1;
            var proxy=new Proxy(base,{
                ownKeys:function(target){log+="keys|";return Reflect.ownKeys(target)},
                getOwnPropertyDescriptor:function(target,key){log+="descriptor-"+key+"|";return Reflect.getOwnPropertyDescriptor(target,key)},
                isExtensible:function(target){log+="extensible|";return Reflect.isExtensible(target)}
            });
            return Object.isFrozen(proxy)+"|"+log;
        })()"#,
    ),
];

const GRAPH_ORACLE: &str = r#"
function bit(value){return value?1:0}
function bits(descriptor){return 'D:'+bit(descriptor.writable)+bit(descriptor.enumerable)+bit(descriptor.configurable)}
function isConstructor(value){try{Reflect.construct(function(){},[],value);return true}catch(_){return false}}
function meta(name){
  var descriptor=Object.getOwnPropertyDescriptor(Object,name),value=descriptor.value;
  return value.name+':'+value.length+':' +(Object.getPrototypeOf(value)===Function.prototype)+':' +
    (typeof value==='function')+':'+isConstructor(value)+':' +Object.getOwnPropertyNames(value).join(',')+':'+bits(descriptor);
}
var selected=['length','name','prototype','create','getPrototypeOf','setPrototypeOf','defineProperty',
  'defineProperties','getOwnPropertyNames','getOwnPropertySymbols','groupBy','keys','values','entries',
  'isExtensible','preventExtensions','getOwnPropertyDescriptor','getOwnPropertyDescriptors','is','assign',
  'seal','freeze','isSealed','isFrozen'];
print('prefix='+Reflect.ownKeys(Object).filter(function(key){return selected.indexOf(key)>=0}).join(','));
['seal','freeze','isSealed','isFrozen'].forEach(function(name){print(name+'='+meta(name))});
print('identity='+(Object.seal===Object.seal)+':' +(Object.freeze===Object.freeze)+':' +
  (Object.isSealed===Object.isSealed)+':' +(Object.isFrozen===Object.isFrozen)+':' +
  (Object.seal!==Object.freeze)+':' +(Object.isSealed!==Object.isFrozen));
['seal','freeze','isSealed','isFrozen'].forEach(function(name){var fn=Object[name];
  print(name+'-props='+bits(Object.getOwnPropertyDescriptor(fn,'length'))+':' +bits(Object.getOwnPropertyDescriptor(fn,'name')));
});
"#;

const FRESH_DELETE_ORACLE: &str = r#"
var a=delete Object.seal,b=delete Object.freeze,c=delete Object.isSealed,d=delete Object.isFrozen;
print([a,b,c,d,'seal' in Object,'freeze' in Object,'isSealed' in Object,'isFrozen' in Object,
  typeof Object.seal,typeof Object.freeze,typeof Object.isSealed,typeof Object.isFrozen].join('|'));
"#;

#[test]
fn object_integrity_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object integrity oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("primitives", PRIMITIVE_CASES),
        ("descriptors", DESCRIPTOR_CASES),
        ("mutations", MUTATION_CASES),
        ("exotic boundaries", EXOTIC_ORACLE_ONLY_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
    assert_eq!(oracle_graph_observations(&oracle).len(), 10);
    assert_eq!(
        oracle_lines(
            &oracle,
            FRESH_DELETE_ORACLE,
            "Object integrity fresh delete",
        )
        .len(),
        1,
    );
}

#[test]
fn object_integrity_primitives_match_pinned_quickjs() {
    compare_cases("Object integrity primitives", PRIMITIVE_CASES);
}

#[test]
fn object_integrity_descriptors_match_pinned_quickjs() {
    compare_cases("Object integrity descriptors", DESCRIPTOR_CASES);
}

#[test]
fn object_integrity_mutation_and_idempotence_match_pinned_quickjs() {
    compare_cases("Object integrity mutations", MUTATION_CASES);
}

#[test]
fn object_integrity_graph_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object integrity graph: set QJS_ORACLE to upstream qjs");
        return;
    };
    assert_eq!(
        rust_graph_observations(),
        oracle_graph_observations(&oracle),
    );
}

#[test]
fn object_integrity_autoinit_can_be_deleted_before_materialization() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP Object integrity AutoInit delete: set QJS_ORACLE to upstream qjs");
        return;
    };
    let expected = oracle_lines(
        &oracle,
        FRESH_DELETE_ORACLE,
        "Object integrity fresh delete",
    );

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let object = global_callable(&runtime, &mut context, "Object");
    let mut values = Vec::new();
    for name in ["seal", "freeze", "isSealed", "isFrozen"] {
        let key = runtime.intern_property_key(name).unwrap();
        values.push(
            runtime
                .delete_property(object.as_object(), &key)
                .unwrap()
                .to_string(),
        );
    }
    for name in ["seal", "freeze", "isSealed", "isFrozen"] {
        let Value::Bool(present) = context.eval(&format!("'{name}' in Object")).unwrap() else {
            panic!("Object.{name} inherited-presence probe was not boolean");
        };
        values.push(present.to_string());
    }
    for name in ["seal", "freeze", "isSealed", "isFrozen"] {
        let key = runtime.intern_property_key(name).unwrap();
        let value = context.get_property(object.as_object(), &key).unwrap();
        values.push(value_type(&runtime, &value).to_owned());
    }
    assert_eq!(vec![values.join("|")], expected);
}

#[test]
fn object_integrity_cross_realm_identity_and_constructor_error_are_exact() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let defining_object = global_callable(&runtime, &mut defining, "Object");
    let seal = property_callable(&runtime, &mut defining, defining_object.as_object(), "seal");
    let freeze = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "freeze",
    );
    let is_sealed = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "isSealed",
    );
    let is_frozen = property_callable(
        &runtime,
        &mut defining,
        defining_object.as_object(),
        "isFrozen",
    );

    let object = eval_object(
        &mut caller,
        "(function(){var value=Object();value.x=1;return value})()",
    );
    assert_eq!(
        caller
            .call(&seal, Value::Undefined, &[Value::Object(object.clone())],)
            .unwrap(),
        Value::Object(object.clone()),
    );
    assert_eq!(
        caller
            .call(
                &is_sealed,
                Value::Undefined,
                &[Value::Object(object.clone())],
            )
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        caller
            .call(
                &is_frozen,
                Value::Undefined,
                &[Value::Object(object.clone())],
            )
            .unwrap(),
        Value::Bool(false),
    );
    assert_eq!(
        caller
            .call(&freeze, Value::Undefined, &[Value::Object(object.clone())],)
            .unwrap(),
        Value::Object(object.clone()),
    );
    assert_eq!(
        caller
            .call(&is_frozen, Value::Undefined, &[Value::Object(object)])
            .unwrap(),
        Value::Bool(true),
    );
    assert_eq!(
        caller
            .call(&freeze, Value::Null, &[Value::Float(-0.0)])
            .unwrap(),
        Value::Float(-0.0),
    );

    let caller_type_error = intrinsic_prototype(&runtime, &mut caller, "TypeError");
    assert_eq!(caller.construct(&seal, &[]), Err(RuntimeError::Exception));
    let error = take_exception_object(&mut caller);
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(caller_type_error),
        "non-constructor rejection must use the caller realm",
    );
}

#[test]
fn object_integrity_methods_are_per_realm_and_retain_then_release_their_realm() {
    let runtime = Runtime::new();
    let (seal, freeze, is_sealed, is_frozen) = {
        let mut first = runtime.new_context();
        let mut second = runtime.new_context();
        let first_object = global_callable(&runtime, &mut first, "Object");
        let second_object = global_callable(&runtime, &mut second, "Object");
        let first_seal = property_callable(&runtime, &mut first, first_object.as_object(), "seal");
        let first_seal_again =
            property_callable(&runtime, &mut first, first_object.as_object(), "seal");
        let first_freeze =
            property_callable(&runtime, &mut first, first_object.as_object(), "freeze");
        let first_is_sealed =
            property_callable(&runtime, &mut first, first_object.as_object(), "isSealed");
        let first_is_frozen =
            property_callable(&runtime, &mut first, first_object.as_object(), "isFrozen");
        let second_seal =
            property_callable(&runtime, &mut second, second_object.as_object(), "seal");
        assert_eq!(first_seal, first_seal_again);
        assert_ne!(first_seal, first_freeze);
        assert_ne!(first_is_sealed, first_is_frozen);
        assert_ne!(first_seal, second_seal);
        assert_eq!(
            runtime.get_prototype_of(first_seal.as_object()).unwrap(),
            Some(first.function_prototype().unwrap()),
        );
        drop(second_seal);
        (first_seal, first_freeze, first_is_sealed, first_is_frozen)
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(seal);
    drop(freeze);
    drop(is_sealed);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    drop(is_frozen);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

#[test]
fn object_integrity_records_current_exotic_object_gap() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval("typeof Proxy+'|'+typeof ArrayBuffer+'|'+typeof Uint8Array")
            .unwrap(),
        Value::String(JsString::try_from_utf8("undefined|undefined|undefined").unwrap()),
        "activate the exotic integrity vectors when these object families are published",
    );
    // Module namespace objects remain an additional ownKeys/DefineOwnProperty
    // integration point for the integrity surface.
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

fn rust_graph_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let function_prototype = context.function_prototype().unwrap();
    let object = global_callable(&runtime, &mut context, "Object");
    let selected = [
        "length",
        "name",
        "prototype",
        "create",
        "getPrototypeOf",
        "setPrototypeOf",
        "defineProperty",
        "defineProperties",
        "getOwnPropertyNames",
        "getOwnPropertySymbols",
        "groupBy",
        "keys",
        "values",
        "entries",
        "isExtensible",
        "preventExtensions",
        "getOwnPropertyDescriptor",
        "getOwnPropertyDescriptors",
        "is",
        "assign",
        "seal",
        "freeze",
        "isSealed",
        "isFrozen",
    ];
    let prefix = own_key_names(&runtime, object.as_object())
        .into_iter()
        .filter(|name| selected.contains(&name.as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let mut output = vec![format!("prefix={prefix}")];
    let mut methods = Vec::new();
    for name in ["seal", "freeze", "isSealed", "isFrozen"] {
        let key = runtime.intern_property_key(name).unwrap();
        let descriptor = data_descriptor(&runtime, object.as_object(), &key);
        let Value::Object(function) = descriptor.0 else {
            panic!("Object.{name} was not an object");
        };
        let callable = runtime.as_callable(&function).unwrap();
        methods.push(function.clone());
        output.push(format!(
            "{name}={}:{}:{}:{}:{}:{}:{}",
            string_property(&runtime, &mut context, &function, "name"),
            int_property(&runtime, &mut context, &function, "length"),
            runtime.get_prototype_of(&function).unwrap().as_ref() == Some(&function_prototype),
            callable.is_some(),
            runtime.is_constructor(&function).unwrap(),
            own_key_names(&runtime, &function).join(","),
            data_bits(descriptor.1, descriptor.2, descriptor.3),
        ));
    }
    let repeated = ["seal", "freeze", "isSealed", "isFrozen"].map(|name| {
        property_callable(&runtime, &mut context, object.as_object(), name)
            .as_object()
            .clone()
    });
    output.push(format!(
        "identity={}:{}:{}:{}:{}:{}",
        methods[0] == repeated[0],
        methods[1] == repeated[1],
        methods[2] == repeated[2],
        methods[3] == repeated[3],
        methods[0] != methods[1],
        methods[2] != methods[3],
    ));
    for (name, function) in ["seal", "freeze", "isSealed", "isFrozen"]
        .into_iter()
        .zip(methods)
    {
        let length = data_descriptor(
            &runtime,
            &function,
            &runtime.intern_property_key("length").unwrap(),
        );
        let function_name = data_descriptor(
            &runtime,
            &function,
            &runtime.intern_property_key("name").unwrap(),
        );
        output.push(format!(
            "{name}-props={}:{}",
            data_bits(length.1, length.2, length.3),
            data_bits(function_name.1, function_name.2, function_name.3),
        ));
    }
    output
}

fn oracle_graph_observations(oracle: &OsStr) -> Vec<String> {
    oracle_lines(oracle, GRAPH_ORACLE, "Object integrity graph")
}

fn oracle_lines(oracle: &OsStr, source: &str, description: &str) -> Vec<String> {
    let output = Command::new(oracle)
        .args(["--std", "-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS {description} failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS {description} output was not UTF-8: {error}"))
        .lines()
        .map(str::to_owned)
        .collect()
}

fn global_callable(runtime: &Runtime, context: &mut Context, name: &str) -> CallableRef {
    let global = context.global_object().unwrap();
    property_callable(runtime, context, &global, name)
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    owner: &ObjectRef,
    name: &str,
) -> CallableRef {
    let Value::Object(object) = context
        .get_property(owner, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&object)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn intrinsic_prototype(
    runtime: &Runtime,
    context: &mut Context,
    constructor_name: &str,
) -> ObjectRef {
    let constructor = global_callable(runtime, context, constructor_name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{constructor_name}.prototype was not an object");
    };
    prototype
}

fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source:?} did not evaluate to an object");
    };
    object
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        panic!("pending exception was not an object");
    };
    error
}

fn own_key_names(runtime: &Runtime, object: &ObjectRef) -> Vec<String> {
    runtime
        .own_property_keys(object)
        .unwrap()
        .into_iter()
        .map(|key| {
            runtime
                .property_key_to_js_string(&key)
                .unwrap()
                .to_utf8_lossy()
        })
        .collect()
}

fn data_descriptor(
    runtime: &Runtime,
    object: &ObjectRef,
    key: &PropertyKey,
) -> (Value, bool, bool, bool) {
    let CompleteOrdinaryPropertyDescriptor::Data {
        value,
        writable,
        enumerable,
        configurable,
    } = runtime
        .get_own_property(object, key)
        .unwrap()
        .expect("missing data descriptor")
    else {
        panic!("property was not a data descriptor");
    };
    (value, writable, enumerable, configurable)
}

fn string_property(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> String {
    let Value::String(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not a String property");
    };
    value.to_utf8_lossy()
}

fn int_property(runtime: &Runtime, context: &mut Context, object: &ObjectRef, name: &str) -> i32 {
    let Value::Int(value) = context
        .get_property(object, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("{name} was not an Int property");
    };
    value
}

fn data_bits(writable: bool, enumerable: bool, configurable: bool) -> String {
    format!(
        "D:{}{}{}",
        Number(writable),
        Number(enumerable),
        Number(configurable),
    )
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
