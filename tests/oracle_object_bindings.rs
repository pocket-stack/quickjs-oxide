use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

// This target pins the object-binding declaration path shared by direct
// declarations, classic for heads, and for-in/of heads in QuickJS 2026-06-04.
// Object-rest bindings use the same declaration path. Assignment and catch
// BindingPatterns have their own focused targets; parameters remain separate.

const DIRECT_CASES: &[(&str, &str)] = &[
    (
        "fixed shorthand string number and keyword property names",
        r#"(function(){
            var {fixed: a, shorthand, missing: defaulted=41}={fixed:1,shorthand:2};
            let {"text": text, 1: one}={text:3,1:4};
            const {if: keyword}={if:5};
            return a+'|'+shorthand+'|'+defaulted+'|'+text+'|'+one+'|'+keyword;
        })()"#,
    ),
    (
        "computed string number and Symbol keys",
        r#"(function(){
            var symbol=Symbol('object-binding'),source={text:40,1:1};
            source[symbol]=2;
            const {['text']: text,[1]: one,[symbol]: symbolic}=source;
            return text+one+symbolic;
        })()"#,
    ),
    (
        "defaults use strict undefined and anonymous functions receive names",
        r#"(function(){
            let {zero=9,nil=8,missing=7,named=function(){}}={zero:0,nil:null};
            return zero+'|'+nil+'|'+missing+'|'+named.name;
        })()"#,
    ),
    (
        "recursive object and array patterns share one source value",
        r#"(function(){
            let {left:{x=40}={},right:[y=41,{z=42}]=[]}={left:{},right:[undefined,{}]};
            const [{a},{b:[c]}]=[{a:1},{b:[2]}];
            return x+'|'+y+'|'+z+'|'+a+'|'+c;
        })()"#,
    ),
    (
        "nested pattern defaults run before recursive object conversion",
        r#"(function(){
            var log='';
            let {outer:{value=(log+='V',40)}=(log+='P',{})}={outer:undefined};
            return value+'|'+log;
        })()"#,
    ),
    (
        "object rest excludes fixed computed and Symbol keys",
        r#"(function(){
            var symbol=Symbol('excluded'),source={fixed:1,kept:2};source[symbol]=3;
            const {fixed,[symbol]:symbolic,...rest}=source;
            return fixed+'|'+symbolic+'|'+rest.kept+'|'+('fixed' in rest)+'|'+(symbol in rest);
        })()"#,
    ),
];

const SURFACE_CASES: &[(&str, &str)] = &[
    (
        "classic for var let and const object-binding heads",
        r#"(function(){
            var log='';
            for(var {x}={x:1};x<3;x++)log+='v'+x;
            for(let {y=3}={};y<5;y++)log+='l'+y;
            for(const {z}={z:5};z===5;) { log+='c'+z; break; }
            return log;
        })()"#,
    ),
    (
        "classic for computed key accepts in inside its NoIn initializer",
        r#"(function(){
            var result;
            for(let {[('x' in {x:1})?'x':'y']: value}={x:42};(result=value,false);){}
            return result;
        })()"#,
    ),
    (
        "for-of var binding keeps the final property value",
        r#"(function(){
            var log='';
            for(var {x=0} of [{x:1},{x:2},{}])log+=x;
            return log+'|'+x;
        })()"#,
    ),
    (
        "for-of lexical object bindings receive fresh captured cells",
        r#"(function(){
            var first,second,index=0;
            for(let {x=40} of [{x:1},{}]) {
                if(index++===0)first=function(){return x};
                else second=function(){return x};
            }
            return first()+'|'+second();
        })()"#,
    ),
    (
        "for-of const binding supports computed Symbol keys and nested arrays",
        r#"(function(){
            var key=Symbol('key'),source={};source[key]=[40,2];
            var result='';
            for(const {[key]:[left,right]} of [source])result=left+'|'+right;
            return result;
        })()"#,
    ),
    (
        "for-in object binding consumes each yielded string key",
        r#"(function(){
            var result='';
            for(const {0:first,1:second} in {ab:1,cd:2})result+=first+second;
            return result;
        })()"#,
    ),
];

const ORDER_CASES: &[(&str, &str)] = &[
    (
        "computed key ToPropertyKey runs once with string hint before get and default",
        r#"(function(){
            var log='',key={};
            key[Symbol.toPrimitive]=function(hint){log+='K:'+hint+'|';return 'x'};
            var source={};
            Object.defineProperty(source,'x',{get:function(){log+='G|';return undefined}});
            var {[key]: value=(log+='D|',42)}=source;
            return value+'|'+log;
        })()"#,
    ),
    (
        "object conversion precedes computed property evaluation",
        r#"(function(){
            var log='',key={};
            key[Symbol.toPrimitive]=function(){log+='K';return 'x'};
            try{let {[key]: value}=null}catch(error){return error.name+'|'+log}
        })()"#,
    ),
    (
        "properties evaluate key get and default from left to right",
        r#"(function(){
            var log='',first={},second={},source={};
            first[Symbol.toPrimitive]=function(hint){log+='K1:'+hint+'|';return 'a'};
            second[Symbol.toPrimitive]=function(hint){log+='K2:'+hint+'|';return 'b'};
            Object.defineProperty(source,'a',{get:function(){log+='Ga|';return 1}});
            Object.defineProperty(source,'b',{get:function(){log+='Gb|';return undefined}});
            Object.defineProperty(source,'z',{get:function(){log+='Gz|';return 3}});
            let {[first]: x=(log+='Da|',8),[second]: y=(log+='Db|',9),z}=source;
            return x+'|'+y+'|'+z+'|'+log;
        })()"#,
    ),
    (
        "fixed sloppy var prepares its with reference before the property getter",
        r#"(function(){
            var value='outer',scope={value:'scope'},source={};
            Object.defineProperty(source,'x',{get:function(){delete scope.value;return 7}});
            with(scope){var {x:value}=source}
            return value+'|'+scope.value;
        })()"#,
    ),
    (
        "computed sloppy var evaluates key then prepares with reference before get",
        r#"(function(){
            var value='outer',scope={},source={},key={};
            key[Symbol.toPrimitive]=function(hint){scope.value='scope';return 'x'};
            Object.defineProperty(source,'x',{get:function(){delete scope.value;return 8}});
            with(scope){var {[key]:value}=source}
            return value+'|'+scope.value;
        })()"#,
    ),
    (
        "nested var prepares its leaf reference after the outer get and before the inner get",
        r#"(function(){
            var value='outer',scope={},inner={},source={};
            Object.defineProperty(source,'outer',{get:function(){scope.value='scope';return inner}});
            Object.defineProperty(inner,'inner',{get:function(){delete scope.value;return 9}});
            with(scope){var {outer:{inner:value}}=source}
            return value+'|'+scope.value;
        })()"#,
    ),
];

const FOR_OF_FAULT_CASES: &[(&str, &str)] = &[
    (
        "computed key fault closes the outer iterator",
        r#"(function(){
            var log='',done=false,key={};
            key[Symbol.toPrimitive]=function(){log+='K|';throw 'key-error'};
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(done)return{done:true};done=true;return{value:{x:1},done:false}},
                return:function(){log+='OC|';return{done:true}}
            };
            try{for(const {[key]:value} of outer){}}
            catch(error){return log+'C:'+error}
        })()"#,
    ),
    (
        "property getter fault closes the outer iterator",
        r#"(function(){
            var log='',done=false,source={};
            Object.defineProperty(source,'x',{get:function(){log+='G|';throw 'get-error'}});
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(done)return{done:true};done=true;return{value:source,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            try{for(const {x} of outer){}}
            catch(error){return log+'C:'+error}
        })()"#,
    ),
    (
        "nested array close fault closes outer and keeps the inner fault",
        r#"(function(){
            var log='',outerDone=false,inner={
                [Symbol.iterator]:function(){log+='II|';return this},
                next:function(){log+='IN|';return{value:1,done:false}},
                return:function(){log+='IC|';throw 'inner-close'}
            },source={};
            Object.defineProperty(source,'x',{get:function(){log+='G|';return inner}});
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(outerDone)return{done:true};outerDone=true;return{value:source,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            try{for(const {x:[value]} of outer){}}
            catch(error){return log+'C:'+error}
        })()"#,
    ),
    (
        "nested array next fault skips inner close but closes outer",
        r#"(function(){
            var log='',outerDone=false,inner={
                [Symbol.iterator]:function(){log+='II|';return this},
                next:function(){log+='IN|';throw 'inner-next'},
                return:function(){log+='IC|';return{done:true}}
            },source={};
            Object.defineProperty(source,'x',{get:function(){log+='G|';return inner}});
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(outerDone)return{done:true};outerDone=true;return{value:source,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            try{for(const {x:[value]} of outer){}}
            catch(error){return log+'C:'+error}
        })()"#,
    ),
    (
        "nested array default fault closes inner then outer and keeps the default fault",
        r#"(function(){
            var log='',outerDone=false,inner={
                [Symbol.iterator]:function(){log+='II|';return this},
                next:function(){log+='IN|';return{value:undefined,done:false}},
                return:function(){log+='IC|';throw 'inner-close'}
            },source={};
            Object.defineProperty(source,'x',{get:function(){log+='G|';return inner}});
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(outerDone)return{done:true};outerDone=true;return{value:source,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            try{for(const {x:[value=(log+='D|',function(){throw 'default-error'})()]} of outer){}}
            catch(error){return log+'C:'+error}
        })()"#,
    ),
    (
        "nested object conversion fault closes the outer iterator",
        r#"(function(){
            var log='',done=false,source={};
            Object.defineProperty(source,'x',{get:function(){log+='G|';return null}});
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(done)return{done:true};done=true;return{value:source,done:false}},
                return:function(){log+='OC|';return{done:true}}
            };
            try{for(const {x:{value}} of outer){}}
            catch(error){return log+'C:'+error.name}
        })()"#,
    ),
    (
        "binding Put fault closes the outer iterator and keeps the Put fault",
        r#"(function(){
            var log='',done=false,scope={},source={};
            Object.defineProperty(scope,'value',{set:function(){log+='S|';throw 'put-error'}});
            Object.defineProperty(source,'x',{get:function(){log+='G|';return 1}});
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(done)return{done:true};done=true;return{value:source,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            try{with(scope){for(var {x:value} of outer){}}}
            catch(error){return log+'C:'+error}
        })()"#,
    ),
];

const PARSER_CASES: &[(&str, &str)] = &[
    ("direct object pattern requires an initializer", "var {x};"),
    (
        "reserved shorthand is not a binding target",
        "let {if}={if:1}",
    ),
    ("string property requires a target", "let {\"x\"}={x:1}"),
    ("computed property requires a target", "let {[x]}={}"),
    ("method syntax is not a binding property", "let {x(){}}={}"),
    (
        "shorthand cannot be followed by an object pattern without a colon",
        "let {a{b}}={a:{b:42}}",
    ),
    (
        "shorthand cannot be followed by an array pattern without a colon",
        "let {a[b]}={a:[42]}",
    ),
    (
        "shorthand cannot take a nested pattern default without a colon",
        "let {a{b}=defaultValue}={a:{b:42}}",
    ),
    (
        "string property cannot take a nested pattern without a colon",
        "let {\"a\"{b}}={}",
    ),
    (
        "keyword property cannot take a nested pattern without a colon",
        "let {if{b}}={}",
    ),
    (
        "computed property cannot take a nested pattern without a colon",
        "let {[\"a\"]{b}}={}",
    ),
    (
        "strict shorthand eval diagnoses after the property name",
        "'use strict';var {eval}={}",
    ),
    (
        "strict shorthand arguments diagnoses after the property name",
        "'use strict';var {arguments=1}={}",
    ),
    (
        "escaped reserved spelling remains an ordinary property name",
        "var {\\u0069f}={}",
    ),
    (
        "escaped reserved alias is an invalid binding target",
        "var {value:\\u0069f}={}",
    ),
    (
        "escaped reserved computed alias is an invalid binding target",
        "var {[0]:\\u0069f}={}",
    ),
    (
        "malformed object rest keeps syntax priority",
        "let {...rest,value}={}",
    ),
    (
        "object rest trailing comma wins before the lexical let target error",
        "let {...let,}={}",
    ),
    (
        "object rest lexical let uses the binding diagnostic",
        "let {...let}={}",
    ),
    (
        "invalid initializer wins after a valid object rest target",
        "let {...rest}=;",
    ),
    (
        "invalid for-of RHS wins after a valid object rest target",
        "for(let {...rest} of ){}",
    ),
    (
        "later duplicate declaration wins after a valid object rest binding",
        "let {...value}={},value",
    ),
];

#[test]
fn direct_object_binding_declarations_match_pinned_quickjs() {
    compare_cases("direct object-binding declarations", DIRECT_CASES);
}

#[test]
fn object_binding_loop_surfaces_match_pinned_quickjs() {
    compare_cases("object-binding loop surfaces", SURFACE_CASES);
}

#[test]
fn object_binding_evaluation_and_reference_order_matches_pinned_quickjs() {
    compare_cases("object-binding evaluation order", ORDER_CASES);
}

#[test]
fn for_of_object_binding_fault_cleanup_matches_pinned_quickjs() {
    compare_cases("for-of object-binding cleanup", FOR_OF_FAULT_CASES);
}

#[test]
fn object_binding_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-binding parser differential: set QJS_ORACLE to upstream qjs");
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

fn run_cli(program: &OsStr, source: &str, description: &str) -> std::process::Output {
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
