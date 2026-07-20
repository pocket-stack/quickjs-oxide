use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

// Pins QuickJS 2026-06-04 ObjectAssignmentPattern lowering. Object binding
// declarations are covered separately: this target keeps AssignmentExpression
// identity, full Reference timing, object-rest copying, object/array recursion,
// and synchronous for-in/of assignment heads on the assignment-specific path.

const DIRECT_CASES: &[(&str, &str)] = &[
    (
        "assignment returns its unconsumed RHS while writing every leaf form",
        r#"(function(){
            var shorthand=0,holder={fixed:0,computed:0},key='dynamic';
            var source={shorthand:40,fixed:1,dynamic:1},result;
            result=({shorthand,fixed:holder.fixed,[key]:holder.computed}=source);
            return (result===source)+'|'+shorthand+'|'+holder.fixed+'|'+holder.computed;
        })()"#,
    ),
    (
        "fixed numeric and computed symbol source keys are supported",
        r#"(function(){
            var symbol=Symbol('value'),numeric,symbolic;
            ({1:numeric,[symbol]:symbolic}={1:40,[symbol]:2});
            return numeric+'|'+symbolic;
        })()"#,
    ),
    (
        "primitive sources are converted with ToObject",
        r#"(function(){
            var first,length,missing;
            ({0:first,length,missing=40}='ab');
            return first+'|'+length+'|'+missing;
        })()"#,
    ),
    (
        "null and undefined fail before any target Reference is evaluated",
        r#"(function(){
            var calls=0,target={};
            function receiver(){calls++;return target}
            var nullError,undefinedError;
            try{({p:receiver().p}=null)}catch(error){nullError=error.name}
            try{({p:receiver().p}=undefined)}catch(error){undefinedError=error.name}
            return nullError+'|'+undefinedError+'|'+calls;
        })()"#,
    ),
    (
        "defaults use strict undefined and perform NamedEvaluation",
        r#"(function(){
            var zero,nil,missing,named,arrow,log='';
            ({
                zero=(log+='zero|',9),
                nil=(log+='nil|',8),
                missing=(log+='missing|',40),
                named=function(){},
                arrow=()=>{}
            }={zero:0,nil:null});
            return zero+'|'+nil+'|'+missing+'|'+named.name+'|'+arrow.name+'|'+log;
        })()"#,
    ),
];

const REFERENCE_ORDER_CASES: &[(&str, &str)] = &[
    (
        "a fixed leaf prepares its member Reference before Get and puts after default",
        r#"(function(){
            var log='',target={},source={};
            function receiver(){log+='R|';return target}
            Object.defineProperty(source,'p',{get:function(){log+='G|';return undefined}});
            Object.defineProperty(target,'value',{set:function(value){log+='P:'+value+'|'}});
            ({p:receiver().value=(log+='D|',42)}=source);
            return log;
        })()"#,
    ),
    (
        "a computed target evaluates base and key before Get but coerces key after Get",
        r#"(function(){
            var log='',target={},source={},key={};
            function receiver(){log+='R|';return target}
            function keyExpression(){log+='E|';return key}
            key[Symbol.toPrimitive]=function(hint){log+='K:'+hint+'|';return 'value'};
            Object.defineProperty(source,'p',{get:function(){log+='G|';return 42}});
            Object.defineProperty(target,'value',{set:function(value){log+='P:'+value+'|'}});
            ({p:receiver()[keyExpression()]}=source);
            return log;
        })()"#,
    ),
    (
        "a computed source key is coerced before the target Reference and source Get",
        r#"(function(){
            var log='',target={},source={},key={};
            function receiver(){log+='R|';return target}
            key[Symbol.toPrimitive]=function(hint){log+='K:'+hint+'|';return 'p'};
            Object.defineProperty(source,'p',{get:function(){log+='G|';return 42}});
            Object.defineProperty(target,'value',{set:function(value){log+='P:'+value+'|'}});
            ({[(log+='E|',key)]:receiver().value}=source);
            return log;
        })()"#,
    ),
    (
        "with retains an existing identifier Reference across source deletion",
        r#"(function(){
            var value='outer',scope={value:'scope'},source={};
            Object.defineProperty(source,'p',{get:function(){delete scope.value;return 40}});
            with(scope){({p:value}=source)}
            return value+'|'+scope.value;
        })()"#,
    ),
    (
        "with resolves a missing identifier before the source getter adds it",
        r#"(function(){
            var value='outer',scope={},source={};
            Object.defineProperty(source,'p',{get:function(){scope.value='late';return 41}});
            with(scope){({p:value}=source)}
            return value+'|'+scope.value;
        })()"#,
    ),
    (
        "super fixed and computed leaves retain depth-three References",
        r#"(function(){
            var log='',key={};
            key[Symbol.toPrimitive]=function(hint){log+='K:'+hint+'|';return 'second'};
            var proto={
                set first(value){log+='P1:'+value+'|';this.left=value},
                set second(value){log+='P2:'+value+'|';this.right=value}
            };
            var home={
                __proto__:proto,
                run(source){
                    ({a:super.first,b:super[(log+='E|',key)]}=source);
                    return this.left+'|'+this.right+'|'+log;
                }
            };
            var source={a:40};
            Object.defineProperty(source,'b',{get:function(){log+='G|';return 2}});
            return home.run(source);
        })()"#,
    ),
    (
        "a nested array reads the outer property before preparing its inner Reference",
        r#"(function(){
            var log='',target={},source={};
            function receiver(){log+='R|';return target}
            Object.defineProperty(source,'p',{get:function(){log+='G|';return [42]}});
            Object.defineProperty(target,'value',{set:function(value){log+='P:'+value+'|'}});
            ({p:[receiver().value]}=source);
            return log;
        })()"#,
    ),
    (
        "a nested object reads each outer property before that level prepares a leaf Reference",
        r#"(function(){
            var log='',target={},inner={},source={};
            function receiver(){log+='R|';return target}
            Object.defineProperty(source,'p',{get:function(){log+='G|';return inner}});
            Object.defineProperty(inner,'q',{get:function(){log+='H|';return 42}});
            Object.defineProperty(target,'value',{set:function(value){log+='P:'+value+'|'}});
            ({p:{q:receiver().value}}=source);
            return log;
        })()"#,
    ),
];

const NESTED_CASES: &[(&str, &str)] = &[
    (
        "object patterns recurse into arrays objects defaults elisions and rest",
        r#"(function(){
            var first,second,third,tail,last;
            ({a:[first,{b:second=40},,...tail],c:{d:[third],e:last=5}}=
              {a:[1,{b:undefined},9,2,3],c:{d:[4]}});
            return first+'|'+second+'|'+tail.join(',')+'|'+third+'|'+last;
        })()"#,
    ),
    (
        "array patterns recurse back into object assignments",
        r#"(function(){
            var first,second,rest;
            [{a:first},{b:[second],...rest}]=[{a:40},{b:[2],c:3,d:4}];
            return first+'|'+second+'|'+Object.keys(rest).join(',')+'|'+rest.c+'|'+rest.d;
        })()"#,
    ),
    (
        "nested object defaults run before recursively converting the replacement",
        r#"(function(){
            var log='',value;
            function fallback(){log+='D|';return{q:42}}
            var source={};
            Object.defineProperty(source,'p',{get:function(){log+='G|';return undefined}});
            ({p:{q:value}=fallback()}=source);
            return value+'|'+log;
        })()"#,
    ),
    (
        "nested array defaults run before acquiring and closing their iterator",
        r#"(function(){
            var log='',value,done=false;
            var iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';if(done)return{done:true};done=true;return{value:42,done:false}},
                return:function(){log+='C|';return{done:true}}
            };
            ({p:[value]=(log+='D|',iterator)}={});
            return value+'|'+log;
        })()"#,
    ),
    (
        "nested object rest can itself be reached through an array pattern",
        r#"(function(){
            var head,rest;
            [{head,...rest}]=[{head:40,a:1,b:2}];
            return head+'|'+Object.keys(rest).join(',')+'|'+rest.a+'|'+rest.b;
        })()"#,
    ),
    (
        "an object rest target may be another destructuring pattern through array rest",
        r#"(function(){
            var first,others;
            ({items:[first,...{length:others}]}={items:[40,1,2,3]});
            return first+'|'+others;
        })()"#,
    ),
];

const REST_CASES: &[(&str, &str)] = &[
    (
        "rest excludes fixed computed and symbol keys after one key coercion",
        r#"(function(){
            var calls=0,key={},symbol=Symbol('symbol'),fixed,dynamic,symbolic,rest;
            key[Symbol.toPrimitive]=function(hint){calls++;return hint==='string'?'dynamic':'wrong'};
            var source={fixed:40,dynamic:1,keep:2,[symbol]:3};
            ({fixed,[key]:dynamic,[symbol]:symbolic,...rest}=source);
            return calls+'|'+fixed+'|'+dynamic+'|'+symbolic+'|'+
                Object.keys(rest).join(',')+'|'+rest.keep+'|'+
                Object.getOwnPropertySymbols(rest).length;
        })()"#,
    ),
    (
        "rest canonicalizes duplicate numeric and string exclusions",
        r#"(function(){
            var first,second,rest;
            ({1:first,['1']:second,...rest}={1:40,keep:2});
            return first+'|'+second+'|'+Object.keys(rest).join(',')+'|'+rest.keep;
        })()"#,
    ),
    (
        "rest copies only enumerable own properties and materializes getter values",
        r#"(function(){
            var calls=0,source=Object.create({inherited:1}),rest;
            Object.defineProperty(source,'copied',{enumerable:true,get:function(){calls++;return 42}});
            Object.defineProperty(source,'hidden',{enumerable:false,value:2});
            ({...rest}=source);
            var descriptor=Object.getOwnPropertyDescriptor(rest,'copied');
            return calls+'|'+rest.copied+'|'+('hidden' in rest)+'|'+('inherited' in rest)+'|'+
                descriptor.writable+'|'+descriptor.enumerable+'|'+descriptor.configurable;
        })()"#,
    ),
    (
        "a computed rest target evaluates its Reference before copy and coerces after copy",
        r#"(function(){
            var log='',target={},source={},key={};
            function receiver(){log+='R|';return target}
            function keyExpression(){log+='E|';return key}
            key[Symbol.toPrimitive]=function(hint){log+='K:'+hint+'|';return 'rest'};
            Object.defineProperty(source,'a',{enumerable:true,get:function(){log+='G|';return 42}});
            Object.defineProperty(target,'rest',{set:function(value){log+='P:'+value.a+'|'}});
            ({...receiver()[keyExpression()]}=source);
            return log;
        })()"#,
    ),
    (
        "a rest Reference failure happens before source getters are copied",
        r#"(function(){
            var log='',source={};
            Object.defineProperty(source,'a',{enumerable:true,get:function(){log+='G|';return 1}});
            function receiver(){log+='R|';throw 'reference-error'}
            try{({...receiver().rest}=source)}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "a copy failure prevents target-key coercion and Put",
        r#"(function(){
            var log='',target={},source={},key={};
            key[Symbol.toPrimitive]=function(){log+='K|';return 'rest'};
            Object.defineProperty(source,'a',{enumerable:true,get:function(){log+='G|';throw 'copy-error'}});
            Object.defineProperty(target,'rest',{set:function(){log+='P|'}});
            try{({...target[key]}=source)}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "rest accepts fixed member and super assignment targets",
        r#"(function(){
            var holder={};
            ({a:holder.a,...holder.rest}={a:40,b:1});
            var proto={set rest(value){this.saved=value}};
            var home={__proto__:proto,run(source){({...super.rest}=source);return this.saved}};
            var saved=home.run({c:2,d:3});
            return holder.a+'|'+holder.rest.b+'|'+saved.c+'|'+saved.d;
        })()"#,
    ),
    (
        "rest preserves the original assignment result identity",
        r#"(function(){
            var rest,source={a:40,b:2},result;
            result=({a:rest,...rest}=source);
            return (result===source)+'|'+rest.b;
        })()"#,
    ),
];

const LOOP_CASES: &[(&str, &str)] = &[
    (
        "for-of object assignment heads update identifiers across iterations",
        r#"(function(){
            var left,right,log='';
            for({left,right} of [{left:1,right:2},{left:40,right:2}])log+=left+':'+right+'|';
            return log+'last:'+left+':'+right;
        })()"#,
    ),
    (
        "for-in object assignment heads destructure each yielded string key",
        r#"(function(){
            var first,second,log='';
            for({0:first,1:second} in {ab:1,cd:2})log+=first+second+'|';
            return log+'last:'+first+second;
        })()"#,
    ),
    (
        "for-of object heads support member targets nested arrays and rest",
        r#"(function(){
            var target={};
            for({head:target.head,items:[target.first],...target.rest} of
                [{head:1,items:[2],extra:3},{head:40,items:[2],extra:4}]){}
            return target.head+'|'+target.first+'|'+target.rest.extra;
        })()"#,
    ),
    (
        "for-of prepares a with Reference independently for each yielded object",
        r#"(function(){
            var value='outer',scope={value:0},log='';
            with(scope){for({p:value} of [{p:40},{p:2}])log+=value+'|'}
            return log+value+'|'+scope.value;
        })()"#,
    ),
];

const ITERATOR_CLOSE_CASES: &[(&str, &str)] = &[
    (
        "an object-head Put failure closes the outer iterator and keeps the pending fault",
        r#"(function(){
            var log='',target={},done=false,iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';if(done)return{done:true};done=true;return{value:{p:1},done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            Object.defineProperty(target,'value',{set:function(){log+='P|';throw 'put-error'}});
            try{for({p:target.value} of iterator){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "nested assignment failure closes the inner then outer iterator",
        r#"(function(){
            var log='',target={},outerDone=false,inner={
                [Symbol.iterator]:function(){log+='II|';return this},
                next:function(){log+='IN|';return{value:1,done:false}},
                return:function(){log+='IC|';throw 'inner-close'}
            },outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(outerDone)return{done:true};outerDone=true;return{value:{p:inner},done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            Object.defineProperty(target,'value',{set:function(){log+='P|';throw 'put-error'}});
            try{for({p:[target.value]} of outer){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "successful nested short patterns close only their inner iterators",
        r#"(function(){
            var log='',value,outerIndex=0;
            function inner(next){var done=false;return{
                [Symbol.iterator]:function(){log+='I'+next+'|';return this},
                next:function(){log+='N'+next+'|';if(done)return{done:true};done=true;return{value:next,done:false}},
                return:function(){log+='C'+next+'|';return{done:true}}
            }}
            var outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){outerIndex++;return outerIndex<3?{value:{p:inner(outerIndex)},done:false}:{done:true}},
                return:function(){log+='OC|';return{done:true}}
            };
            for({p:[value]} of outer)log+='V'+value+'|';
            return log;
        })()"#,
    ),
    (
        "a source getter failure in an object head closes the outer iterator",
        r#"(function(){
            var log='',value,done=false,source={},iterator;
            Object.defineProperty(source,'p',{get:function(){log+='G|';throw 'get-error'}});
            iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';if(done)return{done:true};done=true;return{value:source,done:false}},
                return:function(){log+='C|';return{done:true}}
            };
            try{for({p:value} of iterator){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "fixed leaf getter fault points at its assignment target",
        "(function outer(){var value,source={};Object.defineProperty(source,'p',{get:function fixedGetter(){throw new Error('get')}});({p:value}=source)})()",
    ),
    (
        "computed leaf getter fault points at its assignment target",
        "(function outer(){var value,key='p',source={};Object.defineProperty(source,'p',{get:function computedGetter(){throw new Error('get')}});({[key]:value}=source)})()",
    ),
    (
        "rest copy getter fault points at its rest target",
        "(function outer(){var rest,source={};Object.defineProperty(source,'p',{enumerable:true,get:function restGetter(){throw new Error('copy')}});({...rest}=source)})()",
    ),
    (
        "computed target ToPropertyKey fault points at its member target",
        "(function outer(){var target={},key={};key[Symbol.toPrimitive]=function targetKey(){throw new Error('key')};({p:target[key]}={p:1})})()",
    ),
    (
        "computed target Put fault points at its member target",
        "(function outer(){var target={},key='value';Object.defineProperty(target,key,{set:function targetPut(){throw new Error('put')}});({p:target[key]}={p:1})})()",
    ),
];

const PARSER_CASES: &[(&str, &str)] = &[
    (
        "object assignment rest cannot have a trailing comma",
        "var rest;({...rest,}={})",
    ),
    (
        "object assignment rest must be the final property",
        "var rest,value;({...rest,value}={})",
    ),
    (
        "object assignment rest cannot carry an initializer",
        "var rest;({...rest={}}={})",
    ),
    ("object assignment rest requires a target", "({...}={})"),
    (
        "invalid logical leaf reports at its following delimiter",
        "var value;({p:true&&value}={})",
    ),
    (
        "invalid call leaf reports at its following delimiter",
        "var value;({p:value()}={})",
    ),
    (
        "invalid new leaf reports at its following delimiter",
        "var Value;({p:new Value}={})",
    ),
    (
        "invalid nested object leaf is diagnosed recursively",
        "var value;({p:{q:true&&value}}={})",
    ),
    (
        "invalid nested array rest keeps its own trailing-comma diagnostic",
        "var rest;({p:[...rest,]}={})",
    ),
    (
        "invalid nested object rest keeps its rest-last diagnostic",
        "var rest,value;({p:{...rest,value}}={})",
    ),
    (
        "object assignment pattern cannot be a compound target",
        "var value;({value}+={value:1})",
    ),
    (
        "object assignment pattern cannot be a logical-and target",
        "var value;({value}&&={value:1})",
    ),
    (
        "strict object assignment rejects eval shorthand",
        "'use strict';var value;({eval}={value:1})",
    ),
    (
        "strict object assignment rejects arguments alias targets",
        "'use strict';var value;({p:arguments}={p:1})",
    ),
];

const SMOKE_SOURCE: &str = "(function(){var answer;var source={answer:42};var result=({answer}=source);return answer+'|'+(result===source)})()";

#[test]
fn direct_object_assignments_match_pinned_quickjs() {
    compare_cases("direct object assignments", DIRECT_CASES);
}

#[test]
fn object_assignment_reference_order_matches_pinned_quickjs() {
    compare_cases("object assignment Reference order", REFERENCE_ORDER_CASES);
}

#[test]
fn nested_object_and_array_assignments_match_pinned_quickjs() {
    compare_cases("nested object and array assignments", NESTED_CASES);
}

#[test]
fn object_assignment_rest_matches_pinned_quickjs() {
    compare_cases("object assignment rest", REST_CASES);
}

#[test]
fn object_assignment_loop_heads_match_pinned_quickjs() {
    compare_cases("object assignment loop heads", LOOP_CASES);
}

#[test]
fn object_assignment_iterator_close_matches_pinned_quickjs() {
    compare_cases("object assignment IteratorClose", ITERATOR_CLOSE_CASES);
}

#[test]
fn object_assignment_operation_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-assignment stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in STACK_CASES {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        assert_eq!(rust.status.code(), quickjs.status.code(), "{description}");
        assert_eq!(rust.stdout, quickjs.stdout, "{description}");
        assert_eq!(rust.stderr, quickjs.stderr, "{description}");
    }
}

#[test]
fn object_assignment_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-assignment parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let mut failures = Vec::new();
    for &(description, source) in PARSER_CASES {
        let rust = run_cli(env!("CARGO_BIN_EXE_qjs").as_ref(), source, description);
        let quickjs = run_cli(&oracle, source, description);
        if rust.status.code() != quickjs.status.code()
            || rust.stdout != quickjs.stdout
            || rust.stderr != quickjs.stderr
        {
            eprintln!(
                "MISMATCH {description}\nRust: status={:?}\n{}Oracle: status={:?}\n{}",
                rust.status.code(),
                String::from_utf8_lossy(&rust.stderr),
                quickjs.status.code(),
                String::from_utf8_lossy(&quickjs.stderr),
            );
            failures.push(description);
        }
    }
    assert!(failures.is_empty(), "parser mismatches: {failures:?}");
}

#[test]
fn object_assignment_smoke_runs_without_an_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        observe_rust_eval(
            &runtime,
            &mut context,
            SMOKE_SOURCE,
            "object assignment smoke"
        ),
        "return|string|42|true",
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
