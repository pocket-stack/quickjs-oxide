use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

// Pins QuickJS 2026-06-04 ObjectBindingPattern rest lowering. The excluded-key
// object is built before CopyDataProperties, and the rest target is a fresh
// ordinary object. Assignment and catch BindingPatterns have their own focused
// targets; non-simple parameters remain a separate compiler surface.

const DIRECT_CASES: &[(&str, &str)] = &[
    (
        "var let and const rest exclude fixed numeric and Symbol keys",
        r#"(function(){
            var excluded=Symbol('excluded'),kept=Symbol('kept'),proto={inherited:'I'};
            var source=Object.create(proto),hidden={value:'H',enumerable:false,configurable:true};
            source[2]='two';source.fixed='F';source.keep='K';source[excluded]='X';source[kept]='S';
            Object.defineProperty(source,'hidden',hidden);
            var {fixed,2:numeric,[excluded]:symbolic,...varRest}=source;
            let {keep,...letRest}=source;
            const {missing=42,...constRest}=source;
            var varSymbols=Object.getOwnPropertySymbols(varRest);
            var letSymbols=Object.getOwnPropertySymbols(letRest);
            var descriptor=Object.getOwnPropertyDescriptor(varRest,'keep');
            return fixed+':'+numeric+':'+symbolic+'|'+Object.getOwnPropertyNames(varRest).join(',')+
                ':'+varRest.keep+':'+(varSymbols.length===1&&varSymbols[0]===kept)+':'+varRest[kept]+'|'+
                Object.getOwnPropertyNames(letRest).join(',')+':'+
                (letSymbols.length===2&&letSymbols[0]===excluded&&letSymbols[1]===kept)+'|'+
                missing+':'+Object.getOwnPropertyNames(constRest).join(',')+'|'+
                descriptor.writable+':'+descriptor.enumerable+':'+descriptor.configurable+':'+
                (Object.getPrototypeOf(varRest)===Object.prototype)+':'+
                (varRest.inherited===undefined)+':'+(varRest.hidden===undefined);
        })()"#,
    ),
    (
        "rest recursively nests through object and array binding patterns",
        r#"(function(){
            var source={
                outer:{fixed:1,left:40,right:2},
                list:[3,{fixed:4,deep:41,extra:5}],
                top:42
            };
            let {
                outer:{fixed,...inner},
                list:[head,{fixed:otherFixed,...deep}],
                ...top
            }=source;
            return fixed+':'+Object.keys(inner).join(',')+':'+inner.left+':'+inner.right+'|'+
                head+':'+otherFixed+':'+Object.keys(deep).join(',')+':'+deep.deep+':'+deep.extra+'|'+
                Object.keys(top).join(',')+':'+top.top;
        })()"#,
    ),
    (
        "object rest boxes primitive binding sources after the nullish guard",
        r#"(function(){
            let {0:first,...stringRest}='abc';
            const {...numberRest}=17;
            var nullLog='';
            try{let {...missing}=null}catch(error){nullLog=error.name}
            return first+'|'+Object.keys(stringRest).join(',')+':'+stringRest[1]+stringRest[2]+'|'+
                Object.keys(numberRest).length+'|'+nullLog;
        })()"#,
    ),
];

const LOOP_CASES: &[(&str, &str)] = &[
    (
        "classic for accepts var let and const object-rest initializers",
        r#"(function(){
            var log='';
            for(var {x,...rest}={x:1,y:2};x<2;x++)log+='v'+rest.y;
            for(let {x,...rest}={x:2,y:3};x<3;x++)log+='l'+rest.y;
            for(const {x,...rest}={x:4,y:5};x===4;){log+='c'+rest.y;break}
            return log;
        })()"#,
    ),
    (
        "for-of var rest keeps the final fresh result object",
        r#"(function(){
            var log='';
            for(var {x,...rest} of [{x:1,y:40},{x:2,y:41}])log+=x+':'+rest.y+'|';
            return log+'last:'+x+':'+rest.y;
        })()"#,
    ),
    (
        "for-of lexical rest bindings receive fresh captured cells and objects",
        r#"(function(){
            var closures=[],rests=[];
            for(let {x,...rest} of [{x:1,y:40},{x:2,y:41}]){
                rests.push(rest);closures.push(function(){return x+':'+rest.y});rest.y+=1;
            }
            var symbol=Symbol('loop'),source={fixed:3};source[symbol]=42;
            var symbolic;
            for(const {fixed,...rest} of [source])symbolic=rest[symbol];
            return closures[0]()+'|'+closures[1]()+'|'+(rests[0]!==rests[1])+'|'+symbolic;
        })()"#,
    ),
    (
        "for-in rest copies the remaining UTF-16 index of each yielded key",
        r#"(function(){
            var log='';
            for(const {0:first,...rest} in {ab:1,cd:2})log+=first+rest[1]+'|';
            return log;
        })()"#,
    ),
];

const EXCLUSION_AND_ORDER_CASES: &[(&str, &str)] = &[
    (
        "computed exclusion performs ToPropertyKey once before Get and copy",
        r#"(function(){
            var log='',key={},source={};
            key[Symbol.toPrimitive]=function(hint){log+='key:'+hint+'|';return 'fixed'};
            source.__defineGetter__('fixed',function(){log+='get-fixed|';return 40});
            source.__defineGetter__('kept',function(){log+='get-kept|';return 2});
            let {[key]:value,...rest}=source;
            return value+rest.kept+'|'+Object.hasOwn(rest,'fixed')+'|'+log;
        })()"#,
    ),
    (
        "nested computed exclusion reuses its canonical property key",
        r#"(function(){
            var calls=0,key={};
            key[Symbol.toPrimitive]=function(){calls++;return 'x'};
            let {[key]:{value},...rest}={x:{value:40},kept:2};
            return calls+'|'+value+'|'+rest.kept;
        })()"#,
    ),
    (
        "object-rest prescan ignores regexp and template delimiters in computed keys",
        r#"(function(){
            let {[/[}]/.source]:a,[`${"}"}`]:b,...rest}={"[}]":1,"}":2,kept:3};
            return a+'|'+b+'|'+rest.kept;
        })()"#,
    ),
    (
        "fixed and Symbol exclusions skip a second getter during rest copy",
        r#"(function(){
            var log='',symbol=Symbol('excluded'),kept=Symbol('kept'),source={};
            source.__defineGetter__('fixed',function(){log+='fixed|';return 1});
            source.__defineGetter__(symbol,function(){log+='excluded-symbol|';return 2});
            source.__defineGetter__('other',function(){log+='other|';return 3});
            source.__defineGetter__(kept,function(){log+='kept-symbol|';return 4});
            const {fixed,[symbol]:symbolic,...rest}=source;
            var symbols=Object.getOwnPropertySymbols(rest);
            return fixed+':'+symbolic+':'+rest.other+':'+rest[kept]+'|'+log+'|'+
                Object.hasOwn(rest,'fixed')+':'+Object.hasOwn(rest,symbol)+':'+
                (symbols.length===1&&symbols[0]===kept);
        })()"#,
    ),
    (
        "ordinary rest copy snapshots enumerable own keys before any Get",
        r#"(function(){
            var source={},hidden={value:'hidden',writable:true,enumerable:false,configurable:true};
            source.__defineGetter__('a',function(){
                var b={value:'B',writable:true,enumerable:false,configurable:true};
                Object.defineProperty(source,'b',b);
                hidden.enumerable=true;Object.defineProperty(source,'hidden',hidden);
                source.late='late';return 'A';
            });
            source.b='before';Object.defineProperty(source,'hidden',hidden);
            let {...rest}=source;
            return Object.keys(rest).join(',')+'|'+rest.a+':'+rest.b+':'+
                (rest.hidden===undefined)+':'+(rest.late===undefined)+'|'+
                Object.getOwnPropertyDescriptor(source,'b').enumerable+':'+
                Object.getOwnPropertyDescriptor(source,'hidden').enumerable;
        })()"#,
    ),
    (
        "ordinary snapshot uses a live Get after an earlier getter deletes an own key",
        r#"(function(){
            var proto={later:'prototype'},source=Object.create(proto);
            source.__defineGetter__('first',function(){delete source.later;return 'first'});
            source.later='own';
            const {...rest}=source;
            return Object.keys(rest).join(',')+'|'+rest.first+':'+rest.later+':'+
                source.hasOwnProperty('later');
        })()"#,
    ),
    (
        "copied getters run in integer string then Symbol order after exclusions",
        r#"(function(){
            var log='',excluded=Symbol('excluded'),kept=Symbol('kept'),source={};
            source.__defineGetter__('10',function(){log+='10|';return 10});
            source.__defineGetter__('2',function(){log+='2|';return 2});
            source.__defineGetter__('a',function(){log+='a|';return 'A'});
            source.__defineGetter__(excluded,function(){log+='excluded|';return 'X'});
            source.__defineGetter__(kept,function(){log+='kept|';return 'K'});
            let {10:ten,[excluded]:symbolic,...rest}=source;
            return ten+':'+symbolic+'|'+Object.getOwnPropertyNames(rest).join(',')+':'+
                rest[2]+rest.a+rest[kept]+'|'+log;
        })()"#,
    ),
    (
        "object conversion precedes computed exclusion and no copy starts on failure",
        r#"(function(){
            var log='',key={};
            key[Symbol.toPrimitive]=function(){log+='key';return 'x'};
            try{let {[key]:value,...rest}=undefined}catch(error){return error.name+'|'+log}
        })()"#,
    ),
];

const REFERENCE_CASES: &[(&str, &str)] = &[
    (
        "sloppy with resolves a missing rest target before copy adds that name",
        r#"(function(){
            var value='outer',scope={},source={};
            source.__defineGetter__('x',function(){scope.value='late-scope';return 7});
            with(scope){var {...value}=source}
            return value.x+'|'+scope.value+'|'+Object.keys(value).join(',');
        })()"#,
    ),
    (
        "sloppy with retains an existing rest target reference after copy deletes it",
        r#"(function(){
            var value='outer',scope={value:'early-scope'},source={};
            source.__defineGetter__('x',function(){delete scope.value;return 8});
            with(scope){var {...value}=source}
            return value+'|'+scope.value.x+'|'+Object.keys(scope.value).join(',');
        })()"#,
    ),
];

const ABRUPT_CASES: &[(&str, &str)] = &[
    (
        "rest getter throw stops later copied getters and preserves the thrown value",
        r#"(function(){
            var log='',source={};
            source.__defineGetter__('a',function(){log+='a|';return 1});
            source.__defineGetter__('b',function(){log+='b|';throw 'copy-error'});
            source.__defineGetter__('c',function(){log+='c|';return 3});
            try{let {...rest}=source}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "rest copy fault closes for-of and keeps the pending fault over close",
        r#"(function(){
            var log='',done=false,source={};
            source.__defineGetter__('x',function(){log+='G|';throw 'copy-error'});
            var outer={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';if(done)return{done:true};done=true;return{value:source,done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            try{for(const {...rest} of outer){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "rest target Put fault follows copy then closes the outer iterator",
        r#"(function(){
            var log='',done=false,scope={},source={x:1};
            Object.defineProperty(scope,'value',{configurable:true,set:function(rest){log+='S:'+rest.x+'|';throw 'put-error'}});
            source.__defineGetter__('y',function(){log+='G|';return 2});
            var outer={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';if(done)return{done:true};done=true;return{value:source,done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            try{with(scope){for(var {...value} of outer){}}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
];

const PARSER_CASES: &[(&str, &str)] = &[
    (
        "object binding rest cannot have a trailing comma",
        "let {...rest,}={}",
    ),
    (
        "object binding rest must be the final property",
        "let {...rest,value}={}",
    ),
    (
        "object binding rest cannot carry an initializer",
        "let {...rest=defaultValue}={}",
    ),
    (
        "object binding rest requires a binding identifier",
        "let {...{value}}={}",
    ),
    (
        "object binding rest rejects a member target",
        "let {...rest.value}={}",
    ),
    (
        "strict object binding rest rejects eval",
        "(function(){'use strict';var {...eval}={}})()",
    ),
    (
        "lexical object rest participates in duplicate-binding errors",
        "let {value,...value}={}",
    ),
    (
        "object binding declarations still require an initializer",
        "let {...rest};",
    ),
    (
        "for-of lexical object rest cannot have a declaration initializer",
        "for(let {...rest}={} of []){}",
    ),
];

const SMOKE_SOURCE: &str = r#"(function(){
    var symbol=Symbol('keep'),source={fixed:1,other:40};source[symbol]=2;
    let {fixed,...rest}=source;
    return fixed+'|'+Object.keys(rest).join(',')+':'+rest.other+'|'+
        Object.getOwnPropertySymbols(rest).length+':'+rest[symbol];
})()"#;

#[test]
fn direct_object_rest_bindings_match_pinned_quickjs() {
    compare_cases("direct object-rest bindings", DIRECT_CASES);
}

#[test]
fn object_rest_loop_surfaces_match_pinned_quickjs() {
    compare_cases("object-rest loop surfaces", LOOP_CASES);
}

#[test]
fn object_rest_exclusion_and_copy_order_match_pinned_quickjs() {
    compare_cases(
        "object-rest exclusion and copy order",
        EXCLUSION_AND_ORDER_CASES,
    );
}

#[test]
fn sloppy_object_rest_reference_order_matches_pinned_quickjs() {
    compare_cases("sloppy object-rest reference order", REFERENCE_CASES);
}

#[test]
fn object_rest_abrupt_completion_matches_pinned_quickjs() {
    compare_cases("object-rest abrupt completion", ABRUPT_CASES);
}

#[test]
fn object_rest_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-rest parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in PARSER_CASES {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

#[test]
fn object_rest_smoke_runs_without_an_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        observe_rust_eval(&runtime, &mut context, SMOKE_SOURCE, "object-rest smoke"),
        "return|string|1|other:40|1:2",
    );
}

fn compare_cases(group: &str, cases: &[(&str, &str)]) {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP {group} differential: set QJS_ORACLE to upstream qjs");
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
            primitive_value_text(value)
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
                    primitive_value_text(value)
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
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
}

fn run_cli(program: &OsStr, source: &str, description: &str) -> Output {
    Command::new(program)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}

fn error_string_property(
    runtime: &Runtime,
    context: &mut Context,
    error: &quickjs_oxide::ObjectRef,
    name: &str,
    description: &str,
) -> String {
    let key = runtime
        .intern_property_key(name)
        .expect("Error property key");
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
        Value::Float(value) => value.to_string(),
        Value::BigInt(value) => value.to_string(),
        Value::String(value) => value.to_utf8_lossy(),
        Value::Object(_) => "<object>".to_owned(),
        Value::Symbol(_) => "<symbol>".to_owned(),
    }
}
