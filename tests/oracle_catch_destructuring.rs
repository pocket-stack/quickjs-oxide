use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

// Pins QuickJS 2026-06-04's CatchParameter BindingPattern path. QuickJS
// lowers catch patterns through the declaration destructuring machinery, but
// simple catch identifiers retain their separate JS_VAR_CATCH behavior.

const DIRECT_CASES: &[(&str, &str)] = &[
    (
        "direct object and array catch bindings",
        r#"(function(){
            var objectValue,arrayValue;
            try{throw {left:40,right:2}}catch({left,right}){objectValue=left+right}
            try{throw [6,7]}catch([first,second]){arrayValue=first*second}
            return objectValue+'|'+arrayValue;
        })()"#,
    ),
    (
        "recursive object and array patterns share defaults and elisions",
        r#"(function(){
            try{throw {left:[0,{value:undefined}],right:{items:[41,0,42]}}}
            catch({left:[,{value=40}],right:{items:[first,,last]}}){
                return value+'|'+first+'|'+last;
            }
        })()"#,
    ),
    (
        "leaf defaults distinguish undefined from null and zero",
        r#"(function(){
            try{throw {missing:undefined,nil:null,zero:0}}
            catch({missing=40,nil=41,zero=42}){
                return missing+'|'+nil+'|'+zero;
            }
        })()"#,
    ),
    (
        "whole object and array catch patterns accept defaults",
        r#"(function(){
            var objectResult,arrayResult;
            try{throw undefined}catch({value}={value:40}){objectResult=value}
            try{throw undefined}catch([value]=[42]){arrayResult=value}
            return objectResult+'|'+arrayResult;
        })()"#,
    ),
    (
        "object catch binding boxes strings after its nullish guard",
        r#"(function(){
            var boxed;
            try{throw 'abc'}catch({0:first,length}){boxed=first+'|'+length}
            try{try{throw null}catch({missing}){}}
            catch(error){return boxed+'|'+error.name}
        })()"#,
    ),
    (
        "array catch binding rejects a non iterable object",
        r#"(function(){
            try{try{throw {value:42}}catch([value]){}}
            catch(error){return error.name+'|'+error.message}
        })()"#,
    ),
];

const REST_CASES: &[(&str, &str)] = &[
    (
        "object rest excludes fixed computed and Symbol keys",
        r#"(function(){
            var excluded=Symbol('excluded'),kept=Symbol('kept'),source={fixed:1,other:40};
            source[excluded]=2;source[kept]=42;
            try{throw source}
            catch({fixed,[excluded]:symbolic,...rest}){
                var symbols=Object.getOwnPropertySymbols(rest);
                return fixed+'|'+symbolic+'|'+rest.other+'|'+rest[kept]+'|'+
                    Object.hasOwn(rest,'fixed')+'|'+Object.hasOwn(rest,excluded)+'|'+
                    (symbols.length===1&&symbols[0]===kept)+'|'+
                    (Object.getPrototypeOf(rest)===Object.prototype);
            }
        })()"#,
    ),
    (
        "object rest copies enumerable own properties and skips inherited and hidden keys",
        r#"(function(){
            var source=Object.create({inherited:1});
            source.fixed=2;source.kept=40;
            Object.defineProperty(source,'hidden',{value:3,enumerable:false});
            try{throw source}catch({fixed,...rest}){
                return fixed+'|'+rest.kept+'|'+Object.keys(rest).join(',')+'|'+
                    ('inherited' in rest)+'|'+('hidden' in rest);
            }
        })()"#,
    ),
    (
        "array rest drains the iterator without an early close",
        r#"(function(){
            var log='',index=0,iterator={
                [Symbol.iterator]:function(){log+='open|';return this},
                next:function(){log+='next'+index+'|';return index<3?
                    {value:40+index++,done:false}:{done:true}},
                return:function(){log+='close|';return {done:true}}
            };
            try{throw iterator}catch([first,...rest]){
                return first+'|'+rest.join(',')+'|'+log;
            }
        })()"#,
    ),
    (
        "rest recursively composes object and array catch patterns",
        r#"(function(){
            try{throw {outer:{fixed:1,left:40,right:2},items:[3,{skip:4,deep:42}]}}
            catch({outer:{fixed,...objectRest},items:[head,{skip,...deepRest}]}){
                return fixed+'|'+objectRest.left+'|'+objectRest.right+'|'+
                    head+'|'+skip+'|'+deepRest.deep;
            }
        })()"#,
    ),
];

const ORDER_AND_NAME_CASES: &[(&str, &str)] = &[
    (
        "computed key converts once before Get and default",
        r#"(function(){
            var log='',key={},source={};
            key[Symbol.toPrimitive]=function(hint){log+='key:'+hint+'|';return 'value'};
            Object.defineProperty(source,'value',{get:function(){log+='get|';return undefined}});
            try{throw source}catch({[key]:value=(log+='default|',42)}){
                return value+'|'+log;
            }
        })()"#,
    ),
    (
        "object conversion precedes computed key evaluation",
        r#"(function(){
            var log='',key={};
            key[Symbol.toPrimitive]=function(){log+='key|';return 'value'};
            try{try{throw undefined}catch({[key]:value}){}}
            catch(error){return error.name+'|'+log}
        })()"#,
    ),
    (
        "computed properties evaluate key Get and default from left to right",
        r#"(function(){
            var log='',first={},second={},source={};
            first[Symbol.toPrimitive]=function(){log+='key1|';return 'a'};
            second[Symbol.toPrimitive]=function(){log+='key2|';return 'b'};
            Object.defineProperty(source,'a',{get:function(){log+='get1|';return 40}});
            Object.defineProperty(source,'b',{get:function(){log+='get2|';return undefined}});
            try{throw source}catch({[first]:a=(log+='default1|',1),[second]:b=(log+='default2|',2)}){
                return a+'|'+b+'|'+log;
            }
        })()"#,
    ),
    (
        "computed exclusions are reused by object rest",
        r#"(function(){
            var calls=0,key={},source={fixed:40,kept:2};
            key[Symbol.toPrimitive]=function(){calls++;return 'fixed'};
            try{throw source}catch({[key]:value,...rest}){
                return calls+'|'+value+'|'+rest.kept+'|'+Object.hasOwn(rest,'fixed');
            }
        })()"#,
    ),
    (
        "anonymous functions in object and array leaf defaults receive names",
        r#"(function(){
            var objectName,arrayName,nestedName;
            try{throw {}}
            catch({objectFn=function(){},nested:{nestedFn=function(){}}={}}){
                objectName=objectFn.name;nestedName=nestedFn.name;
            }
            try{throw []}catch([arrayFn=function(){}]){arrayName=arrayFn.name}
            return objectName+'|'+arrayName+'|'+nestedName;
        })()"#,
    ),
];

const SCOPE_AND_EVAL_CASES: &[(&str, &str)] = &[
    (
        "catch pattern bindings are mutable lexical cells",
        "(function(){try{throw {value:40}}catch({value}){value+=2;return value}})()",
    ),
    (
        "pattern initializer closure keeps the parameter cell across body shadowing",
        r#"(function(){
            try{throw {read:undefined,value:7}}
            catch({read=function(){return value},value}){
                let value=9;
                let bodyRead=function(){return value};
                return read.name+'|'+read()+'|'+bodyRead();
            }
        })()"#,
    ),
    (
        "pattern catch permits a same name body lexical unlike simple catch",
        "(function(){try{throw {value:1}}catch({value}){let value=42;return value}})()",
    ),
    (
        "reentered pattern catch creates fresh captured cells",
        r#"(function(){
            var first,second,index=0;
            while(index<2){
                try{throw {value:++index}}
                catch({value}){
                    if(index===1)first=function(){return value};
                    else second=function(){return value};
                }
            }
            return first()+'|'+second()+'|'+(first()!==second());
        })()"#,
    ),
    (
        "direct eval reads and writes a pattern binding",
        "(function(){try{throw {value:40}}catch({value}){eval('value+=2');return eval('value')}})()",
    ),
    (
        "direct eval var reuses a simple catch binding",
        "(function(){try{throw 1}catch(value){eval('var value=42');return value}})()",
    ),
    (
        "direct eval var cannot redeclare a pattern catch binding",
        r#"(function(){
            try{throw {value:1}}
            catch({value}){
                try{eval('var value=2');return 'missing'}
                catch(error){return error.name+'|'+error.message}
            }
        })()"#,
    ),
    (
        "simple catch var reuses its binding",
        "(function(){try{throw 1}catch(value){var value=42;return value}})()",
    ),
];

const ITERATOR_CASES: &[(&str, &str)] = &[
    (
        "one element array pattern closes an unfinished iterator",
        r#"(function(){
            var log='',iterator={
                [Symbol.iterator]:function(){log+='open|';return this},
                next:function(){log+='next|';return {value:40,done:false}},
                return:function(){log+='close|';return {done:true}}
            };
            try{throw iterator}catch([value]){return value+'|'+log}
        })()"#,
    ),
    (
        "empty array pattern opens then closes without calling next",
        r#"(function(){
            var log='',iterator={
                [Symbol.iterator]:function(){log+='open|';return this},
                next:function(){log+='next|';return {value:40,done:false}},
                return:function(){log+='close|';return {done:true}}
            };
            try{throw iterator}catch([]){return log}
        })()"#,
    ),
    (
        "default fault closes the iterator and wins over close fault",
        r#"(function(){
            var log='',iterator={
                [Symbol.iterator]:function(){log+='open|';return this},
                next:function(){log+='next|';return {value:undefined,done:false}},
                return:function(){log+='close|';throw 'close-error'}
            };
            try{try{throw iterator}
                catch([value=(log+='default|',function(){throw 'default-error'}())]){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "nested iterator faults close inner then outer and keep the original fault",
        r#"(function(){
            var log='',inner={
                [Symbol.iterator]:function(){log+='inner-open|';return this},
                next:function(){log+='inner-next|';return {value:undefined,done:false}},
                return:function(){log+='inner-close|';throw 'inner-close-error'}
            },outer={
                [Symbol.iterator]:function(){log+='outer-open|';return this},
                next:function(){log+='outer-next|';return {value:inner,done:false}},
                return:function(){log+='outer-close|';throw 'outer-close-error'}
            };
            try{try{throw outer}
                catch([[value=(log+='default|',function(){throw 'default-error'}())]]){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "iterator next fault does not call return",
        r#"(function(){
            var log='',iterator={
                [Symbol.iterator]:function(){log+='open|';return this},
                next:function(){log+='next|';throw 'next-error'},
                return:function(){log+='close|';return {done:true}}
            };
            try{try{throw iterator}catch([value]){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
];

// These deliberately pin observable QuickJS 2026-06-04 behavior, including
// behavior that differs from ECMA-262. They are exact rather than merely
// differential so an updated oracle cannot silently redefine this milestone.
const PINNED_QUIRK_CASES: &[(&str, &str, &str)] = &[
    (
        "object catch pattern skips TDZ initialization",
        "(function(){try{throw {}}catch({first=second,second}){return String(first)+'|'+String(second)}})()",
        "return|string|undefined|undefined",
    ),
    (
        "array catch pattern skips TDZ initialization",
        "(function(){try{throw []}catch([first=second,second]){return String(first)+'|'+String(second)}})()",
        "return|string|undefined|undefined",
    ),
    (
        "computed key observes a later uninitialized binding as undefined",
        "(function(){try{throw {undefined:9}}catch({[later]:value,later}){return value+'|'+String(later)}})()",
        "return|string|9|undefined",
    ),
    (
        "whole catch patterns accept top level defaults",
        "(function(){var a,b;try{throw undefined}catch({value}={value:40}){a=value}try{throw undefined}catch([value]=[42]){b=value}return a+'|'+b})()",
        "return|string|40|42",
    ),
    (
        "abrupt catch initialization closes its iterator but skips its own finally",
        r#"(function(){
            var log='',iterator={
                [Symbol.iterator]:function(){log+='open|';return this},
                next:function(){log+='next|';return {value:undefined,done:false}},
                return:function(){log+='close|';return {done:true}}
            };
            try{
                try{throw iterator}
                catch([value=(log+='default|',null.value)]){log+='body|'}
                finally{log+='finally|'}
            }catch(error){return log+error.name}
        })()"#,
        "return|string|open|next|default|close|TypeError",
    ),
];

const DIAGNOSTIC_CASES: &[(&str, &str)] = &[
    (
        "duplicate object catch names are lexical redefinitions",
        "try{}catch({value,value}){}",
    ),
    (
        "duplicate nested array and object catch names are lexical redefinitions",
        "try{}catch([value,{value}]){}",
    ),
    (
        "pattern catch binding conflicts with var",
        "try{}catch({value}){var value}",
    ),
    (
        "simple catch binding conflicts with same body let",
        "try{}catch(value){let value}",
    ),
    (
        "strict object catch pattern rejects eval as a target",
        "'use strict';try{}catch({value:eval}){}",
    ),
    (
        "strict simple catch binding rejects eval with its own diagnostic",
        "'use strict';try{}catch(eval){}",
    ),
    (
        "sloppy object catch pattern still rejects lexical let",
        "try{}catch({value:let}){}",
    ),
    (
        "array catch rest rejects a default",
        "try{}catch([...rest=[]]){}",
    ),
    (
        "array catch rest must be last",
        "try{}catch([...rest,last]){}",
    ),
    (
        "object catch rest must be last",
        "try{}catch({...rest,last}){}",
    ),
    (
        "object catch pattern rejects a non binding leaf",
        "try{}catch({value:true}){}",
    ),
    (
        "catch pattern cannot be parenthesized",
        "try{}catch(({value})){}",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "object catch conversion fault uses the throw site",
        "(function catchObjectBinding(){\n  try {\n    throw null;\n  } catch ({ value }) {\n  }\n})()",
    ),
    (
        "computed catch key fault keeps its user function origin",
        "(function catchComputedBinding(){\n  var key = {};\n  key[Symbol.toPrimitive] = function catchKey(){ throw new Error(\"computed\"); };\n  try {\n    throw {};\n  } catch ({ [key]: value }) {\n  }\n})()",
    ),
    (
        "array catch default fault closes at its initializer origin",
        "(function catchArrayBinding(){\n  var iterable = {\n    [Symbol.iterator]: function(){ return this; },\n    next: function(){ return { value: undefined, done: false }; },\n    return: function(){ return { done: true }; }\n  };\n  try {\n    throw iterable;\n  } catch ([value = (function catchDefault(){ null.value; })()]) {\n  }\n})()",
    ),
];

#[test]
fn catch_destructuring_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP catch-destructuring oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(group, cases) in &[
        ("direct/default", DIRECT_CASES),
        ("rest", REST_CASES),
        ("order/name", ORDER_AND_NAME_CASES),
        ("scope/eval", SCOPE_AND_EVAL_CASES),
        ("iterator", ITERATOR_CASES),
    ] {
        for &(description, source) in cases {
            let observation = observe_oracle(&oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector did not produce a completion for {description}: {observation:?}",
            );
        }
    }

    for &(description, source, expected) in PINNED_QUIRK_CASES {
        assert_eq!(
            observe_oracle(&oracle, source, description),
            expected,
            "pinned QuickJS catch quirk changed for {description}",
        );
    }
}

#[test]
fn catch_destructuring_direct_nested_and_defaults_match_pinned_quickjs() {
    compare_value_cases("catch direct/nested/default bindings", DIRECT_CASES);
}

#[test]
fn catch_destructuring_rest_matches_pinned_quickjs() {
    compare_value_cases("catch rest bindings", REST_CASES);
}

#[test]
fn catch_destructuring_computed_order_and_names_match_pinned_quickjs() {
    compare_value_cases("catch computed/order/NamedEvaluation", ORDER_AND_NAME_CASES);
}

#[test]
fn catch_destructuring_scope_var_and_eval_match_pinned_quickjs() {
    compare_value_cases("catch scope/var/eval", SCOPE_AND_EVAL_CASES);
}

#[test]
fn catch_destructuring_iterator_close_matches_pinned_quickjs() {
    compare_value_cases("catch IteratorClose", ITERATOR_CASES);
}

#[test]
fn catch_destructuring_pinned_quirks_have_exact_results() {
    let oracle = std::env::var_os("QJS_ORACLE");
    for &(description, source, expected) in PINNED_QUIRK_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            expected,
            "Rust engine drifted from the pinned catch quirk for {description}",
        );
        if let Some(oracle) = oracle.as_deref() {
            assert_eq!(
                observe_oracle(oracle, source, description),
                expected,
                "QuickJS oracle drifted from the pinned catch quirk for {description}",
            );
        }
    }
}

#[test]
fn catch_destructuring_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP catch-destructuring diagnostics: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in DIAGNOSTIC_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn catch_destructuring_exact_cli_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP catch-destructuring stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["--strip-source"], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

fn compare_value_cases(group: &str, cases: &[(&str, &str)]) {
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

fn compare_cli(oracle: &OsStr, options: &[&str], source: &str, description: &str) {
    let rust = run_cli(
        env!("CARGO_BIN_EXE_qjs").as_ref(),
        options,
        source,
        description,
    );
    let quickjs = run_cli(oracle, options, source, description);
    assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
    assert_eq!(rust.stdout, quickjs.stdout, "{description}");
    assert_eq!(rust.stderr, quickjs.stderr, "{description}");
}

fn run_cli(program: &OsStr, options: &[&str], source: &str, description: &str) -> Output {
    Command::new(program)
        .args(options)
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
