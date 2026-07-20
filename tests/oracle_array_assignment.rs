use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, ErrorKind, Runtime, RuntimeError, Value};

// Pins QuickJS 2026-06-04 ArrayAssignmentPattern lowering. Array binding
// declarations are covered separately: this target keeps AssignmentExpression
// result identity, Reference timing, IteratorClose, and synchronous for-in/of
// assignment heads on the assignment-specific path. Nested object assignment
// remains an explicit compiler frontier below.

const DIRECT_CASES: &[(&str, &str)] = &[
    (
        "assignment returns its unconsumed RHS while writing flat targets",
        r#"(function(){
            var identifier=0,holder={fixed:0,computed:0},key='computed';
            var source=[40,1,1],result;
            result=([identifier,holder.fixed,holder[key]]=source);
            return (result===source)+'|'+identifier+'|'+holder.fixed+'|'+holder.computed;
        })()"#,
    ),
    (
        "super fixed and computed properties are valid assignment targets",
        r#"(function(){
            var proto={
                set first(value){this.left=value},
                set second(value){this.right=value}
            };
            var home={
                __proto__:proto,
                run(values){
                    [super.first,super['second']]=values;
                    return this.left+'|'+this.right;
                }
            };
            return home.run([40,2]);
        })()"#,
    ),
    (
        "defaults use strict undefined and perform NamedEvaluation",
        r#"(function(){
            var zero,nil,missing,named,arrow,log='';
            [
                zero=(log+='zero|',9),
                nil=(log+='nil|',8),
                missing=(log+='missing|',40),
                named=function(){},
                arrow=()=>{}
            ]=[0,null,undefined];
            return zero+'|'+nil+'|'+missing+'|'+named.name+'|'+arrow.name+'|'+log;
        })()"#,
    ),
    (
        "empty elisions and rest share iterator assignment lowering",
        r#"(function(){
            var log='',empty={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';return{value:1,done:false}},
                return:function(){log+='C|';return{done:true}}
            };
            []=empty;
            var first,rest;
            [first,,...rest]=[1,2,3,4];
            return log+first+'|'+rest.join(',');
        })()"#,
    ),
    (
        "rest accepts a member target and exhausts the source iterator",
        r#"(function(){
            var holder={};
            [...holder.values]='ab';
            return Array.isArray(holder.values)+'|'+holder.values.join(',');
        })()"#,
    ),
    (
        "nested array patterns recurse through defaults elisions and rest",
        r#"(function(){
            var first,second,third,tail,result;
            result=([first,[second=40,,...tail],[[third]]]=[1,[undefined,9,2,3],[[4]]]);
            return (result[0]===1)+'|'+first+'|'+second+'|'+tail.join(',')+'|'+third;
        })()"#,
    ),
];

const REFERENCE_ORDER_CASES: &[(&str, &str)] = &[
    (
        "computed expression precedes IteratorNext but ToPropertyKey and Put follow it",
        r#"(function(){
            var log='',key={},target={},once=false;
            key[Symbol.toPrimitive]=function(hint){log+='K:'+hint+'|';return 'value'};
            Object.defineProperty(target,'value',{set:function(value){log+='P:'+value+'|'}});
            var iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';if(once)return{done:true};once=true;return{value:42,done:false}},
                return:function(){log+='C|';return{done:true}}
            };
            [target[(log+='E|',key)]]=iterator;
            return log;
        })()"#,
    ),
    (
        "with retains an existing property Reference across IteratorNext deletion",
        r#"(function(){
            var value='outer',scope={value:'scope'},once=false;
            var iterator={
                [Symbol.iterator]:function(){return this},
                next:function(){
                    if(once)return{done:true};once=true;delete scope.value;
                    return{value:40,done:false};
                },
                return:function(){return{done:true}}
            };
            with(scope){[value]=iterator}
            return value+'|'+scope.value;
        })()"#,
    ),
    (
        "with resolves a missing identifier before IteratorNext adds that property",
        r#"(function(){
            var value='outer',scope={},once=false;
            var iterator={
                [Symbol.iterator]:function(){return this},
                next:function(){
                    if(once)return{done:true};once=true;scope.value='late-scope';
                    return{value:41,done:false};
                },
                return:function(){return{done:true}}
            };
            with(scope){[value]=iterator}
            return value+'|'+scope.value;
        })()"#,
    ),
    (
        "nested target Reference is prepared after the outer step and before the inner step",
        r#"(function(){
            var value='outer',scope={},outerDone=false,innerDone=false;
            var inner={
                [Symbol.iterator]:function(){return this},
                next:function(){
                    if(innerDone)return{done:true};innerDone=true;delete scope.value;
                    return{value:42,done:false};
                },
                return:function(){return{done:true}}
            };
            var outer={
                [Symbol.iterator]:function(){return this},
                next:function(){
                    if(outerDone)return{done:true};outerDone=true;scope.value='inner-scope';
                    return{value:inner,done:false};
                },
                return:function(){return{done:true}}
            };
            with(scope){[[value]]=outer}
            return value+'|'+scope.value;
        })()"#,
    ),
];

const ITERATOR_CLOSE_CASES: &[(&str, &str)] = &[
    (
        "a successful short pattern closes its non-exhausted iterator",
        r#"(function(){
            var log='',value,iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';return{value:42,done:false}},
                return:function(){log+='C|';return{done:true}}
            };
            [value]=iterator;
            return value+'|'+log;
        })()"#,
    ),
    (
        "a close throw replaces an otherwise successful assignment",
        r#"(function(){
            var log='',value,iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';return{value:1,done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            try{[value]=iterator}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "a setter fault closes and keeps the pending Put failure over close",
        r#"(function(){
            var log='',target={},iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';return{value:1,done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            Object.defineProperty(target,'value',{set:function(){log+='P|';throw 'put-error'}});
            try{[target.value]=iterator}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "a computed ToPropertyKey fault closes and keeps its pending failure",
        r#"(function(){
            var log='',target={},key={},iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';return{value:1,done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            key[Symbol.toPrimitive]=function(){log+='K|';throw 'key-error'};
            try{[target[(log+='E|',key)]]=iterator}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "a default fault closes and keeps its pending failure over close",
        r#"(function(){
            var log='',value,iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';return{value:undefined,done:false}},
                return:function(){log+='C|';throw 'close-error'}
            };
            try{[value=(log+='D|',function(){throw 'default-error'})()]=iterator}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "IteratorNext failure does not call return",
        r#"(function(){
            var log='',value,iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';throw 'next-error'},
                return:function(){log+='C|';return{done:true}}
            };
            try{[value]=iterator}catch(error){return log+'caught:'+error}
        })()"#,
    ),
    (
        "rest exhausts naturally without calling return",
        r#"(function(){
            var log='',index=0,rest,iterator={
                [Symbol.iterator]:function(){log+='I|';return this},
                next:function(){log+='N|';index++;return index<3?{value:index,done:false}:{done:true}},
                return:function(){log+='C|';return{done:true}}
            };
            [...rest]=iterator;
            return rest.join(',')+'|'+log;
        })()"#,
    ),
    (
        "nested close runs inner then outer and preserves the inner failure",
        r#"(function(){
            var log='',value,outerDone=false,inner={
                [Symbol.iterator]:function(){log+='II|';return this},
                next:function(){log+='IN|';return{value:1,done:false}},
                return:function(){log+='IC|';throw 'inner-close'}
            },outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(outerDone)return{done:true};outerDone=true;return{value:inner,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            try{[[value]]=outer}catch(error){return log+'caught:'+error}
        })()"#,
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "direct IteratorNext fault points at its identifier target",
        "(function outer(){var x,it={[Symbol.iterator]:function(){return this},next:function nextFault(){throw new Error('next')}};[x]=it})()",
    ),
    (
        "computed target IteratorNext fault points at its member site",
        "(function outer(){var obj={},key='x',it={[Symbol.iterator]:function(){return this},next:function nextFault(){throw new Error('next')}};[obj[key]]=it})()",
    ),
    (
        "second-step rest fault points at the rest identifier",
        "(function outer(){var x,rest,it={i:0,[Symbol.iterator]:function(){return this},next:function restFault(){if(this.i++)throw new Error('next');return{value:1,done:false}}};[x,...rest]=it})()",
    ),
    (
        "later elision fault retains the preceding identifier site",
        "(function outer(){var x,it={i:0,[Symbol.iterator]:function(){return this},next:function elisionFault(){if(this.i++)throw new Error('next');return{value:1,done:false}}};[x,,]=it})()",
    ),
    (
        "nested rest drain fault retains the preceding outer target site",
        "(function outer(){var x,y,it={i:0,[Symbol.iterator]:function(){return this},next:function nestedRestFault(){if(this.i++)throw new Error('next');return{value:1,done:false}}};[x,...[y]]=it})()",
    ),
    (
        "later nested outer-step fault retains the preceding target site",
        "(function outer(){var a,x,it={i:0,[Symbol.iterator]:function(){return this},next:function laterNestedFault(){if(this.i++)throw new Error('next');return{value:1,done:false}}};[a,[x]]=it})()",
    ),
    (
        "leading nested outer-step fault points at the nested opener",
        "(function outer(){var x,it={[Symbol.iterator]:function(){return this},next:function leadingNestedFault(){throw new Error('next')}};[[x]]=it})()",
    ),
    (
        "post-elision nested outer-step fault retains the pattern opener",
        "(function outer(){var x,it={i:0,[Symbol.iterator]:function(){return this},next:function elidedNestedFault(){if(this.i++)throw new Error('next');return{value:1,done:false}}};[,[x]]=it})()",
    ),
    (
        "nested iterator acquisition fault retains the preceding outer target site",
        "(function outer(){var a,x,inner={[Symbol.iterator]:function innerAcquireFault(){throw new Error('acquire')}};[a,[x]]=[1,inner]})()",
    ),
    (
        "nested IteratorNext fault points at the inner identifier",
        "(function outer(){var x,inner={[Symbol.iterator]:function(){return this},next:function innerFault(){throw new Error('inner')}};[[x]]=[inner]})()",
    ),
    (
        "normal IteratorClose fault keeps the final identifier target",
        "(function outer(){var x,it={[Symbol.iterator]:function(){return this},next:function(){return{value:1,done:false}},return:function closeFault(){throw new Error('close')}};[x]=it})()",
    ),
];

const WRITE_CASES: &[(&str, &str)] = &[
    (
        "sloppy readonly Put is ignored while strict readonly Put throws",
        r#"(function(){
            var loose={};Object.defineProperty(loose,'value',{value:1,writable:false});
            [loose.value]=[2];
            var strictError=(function(){
                'use strict';
                var fixed={};Object.defineProperty(fixed,'value',{value:1,writable:false});
                try{[fixed.value]=[2]}catch(error){return error.name}
            })();
            return loose.value+'|'+strictError;
        })()"#,
    ),
    (
        "sloppy unresolvable Put creates a global while strict Put throws",
        r#"(function(){
            [__array_assignment_loose__]=[40];
            var loose=globalThis.__array_assignment_loose__;
            delete globalThis.__array_assignment_loose__;
            var strictError=(function(){
                'use strict';
                try{[__array_assignment_strict__]=[2]}catch(error){return error.name}
            })();
            return loose+'|'+strictError+'|'+typeof globalThis.__array_assignment_strict__;
        })()"#,
    ),
    (
        "const and later lexical assignment targets preserve TypeError and TDZ",
        r#"(function(){
            const fixed=1;var constError,tdzError;
            try{[fixed]=[2]}catch(error){constError=error.name}
            try{[later]=[3];let later}catch(error){tdzError=error.name}
            return fixed+'|'+constError+'|'+tdzError;
        })()"#,
    ),
];

const LOOP_CASES: &[(&str, &str)] = &[
    (
        "for-of accepts a leading Array literal fixed member target",
        r#"(function(){
            var receiver,value;
            Object.defineProperty(Array.prototype,'x',{
                configurable:true,
                set:function(next){receiver=this;value=next}
            });
            try{for([].x of [42]){}}
            finally{delete Array.prototype.x}
            return Array.isArray(receiver)+'|'+receiver.length+'|'+value;
        })()"#,
    ),
    (
        "for-of accepts a leading Object literal fixed member target",
        r#"(function(){
            var receiver,value;
            Object.defineProperty(Object.prototype,'x',{
                configurable:true,
                set:function(next){receiver=this;value=next}
            });
            try{for({}.x of [42]){}}
            finally{delete Object.prototype.x}
            return (Object.getPrototypeOf(receiver)===Object.prototype)+'|'+
                Object.keys(receiver).length+'|'+value;
        })()"#,
    ),
    (
        "for-of accepts a leading Array literal computed member target",
        r#"(function(){
            var calls=0,key={},receivers=[],values=[];
            key[Symbol.toPrimitive]=function(hint){calls++;return hint==='string'?'x':'wrong'};
            Object.defineProperty(Array.prototype,'x',{
                configurable:true,
                set:function(next){receivers.push(this);values.push(next)}
            });
            try{for([][key] of [40,2]){}}
            finally{delete Array.prototype.x}
            return calls+'|'+values.join(',')+'|'+(receivers[0]!==receivers[1])+'|'+
                Array.isArray(receivers[0])+':'+Array.isArray(receivers[1]);
        })()"#,
    ),
    (
        "for-of accepts a leading Object literal computed member target",
        r#"(function(){
            var calls=0,key={},receiver,value;
            key[Symbol.toPrimitive]=function(hint){calls++;return hint==='string'?'x':'wrong'};
            Object.defineProperty(Object.prototype,'x',{
                configurable:true,
                set:function(next){receiver=this;value=next}
            });
            try{for({}[key] of [41]){}}
            finally{delete Object.prototype.x}
            return calls+'|'+(Object.getPrototypeOf(receiver)===Object.prototype)+'|'+value;
        })()"#,
    ),
    (
        "for-in accepts a leading Array literal member target",
        r#"(function(){
            var receivers=[],values=[];
            Object.defineProperty(Array.prototype,'x',{
                configurable:true,
                set:function(next){receivers.push(this);values.push(next)}
            });
            try{for([].x in {a:1,b:2}){}}
            finally{delete Array.prototype.x}
            return values.join(',')+'|'+(receivers[0]!==receivers[1])+'|'+
                Array.isArray(receivers[0])+':'+Array.isArray(receivers[1]);
        })()"#,
    ),
    (
        "for-in accepts a leading Object literal member target",
        r#"(function(){
            var receivers=[],values=[];
            Object.defineProperty(Object.prototype,'x',{
                configurable:true,
                set:function(next){receivers.push(this);values.push(next)}
            });
            try{for({}.x in {a:1,b:2}){}}
            finally{delete Object.prototype.x}
            return values.join(',')+'|'+(receivers[0]!==receivers[1])+'|'+
                (Object.getPrototypeOf(receivers[0])===Object.prototype)+':'+
                (Object.getPrototypeOf(receivers[1])===Object.prototype);
        })()"#,
    ),
    (
        "for-of array assignment heads update existing identifiers",
        r#"(function(){
            var left,right,log='';
            for([left,right] of [[1,2],[3,4]])log+=left+':'+right+'|';
            return log+'last:'+left+':'+right;
        })()"#,
    ),
    (
        "for-in array assignment heads destructure each yielded string key",
        r#"(function(){
            var first,second,log='';
            for([first,second] in {ab:1,cd:2})log+=first+second+'|';
            return log+'last:'+first+second;
        })()"#,
    ),
    (
        "for-of array assignment heads support member targets and rest",
        r#"(function(){
            var target={};
            for([target.head,...target.tail] of [[1,2,3],[40,2]]){}
            return target.head+'|'+target.tail.join(',');
        })()"#,
    ),
    (
        "assignment failure closes inner then outer and preserves the pending fault",
        r#"(function(){
            var log='',target={},outerDone=false,inner={
                [Symbol.iterator]:function(){log+='II|';return this},
                next:function(){log+='IN|';return{value:1,done:false}},
                return:function(){log+='IC|';throw 'inner-close'}
            },outer={
                [Symbol.iterator]:function(){log+='OI|';return this},
                next:function(){log+='ON|';if(outerDone)return{done:true};outerDone=true;return{value:inner,done:false}},
                return:function(){log+='OC|';throw 'outer-close'}
            };
            Object.defineProperty(target,'value',{set:function(){log+='P|';throw 'put-error'}});
            try{for([target.value] of outer){}}
            catch(error){return log+'caught:'+error}
        })()"#,
    ),
];

const PARSER_CASES: &[(&str, &str)] = &[
    (
        "array assignment rest cannot have a trailing comma",
        "var value;[...value,]=[]",
    ),
    (
        "array assignment rest must be the final element",
        "var first,last;[...first,last]=[]",
    ),
    (
        "array assignment rest cannot carry an initializer",
        "var rest;[...rest=[]]=[]",
    ),
    ("array assignment rest requires a target", "var a;[...]=[];"),
    (
        "nested array rest default wins over its invalid leaf",
        "var a;[...[true&&a]=[]]=[];",
    ),
    (
        "nested object rest default wins over its invalid leaf",
        "var a;[...{p:true&&a}=[]]=[];",
    ),
    (
        "validated nested array rest default wins over its invalid leaf",
        "var a;({p:[...[true&&a]=[]]}={});",
    ),
    (
        "validated nested object rest default wins over its invalid leaf",
        "var a;({p:[...{q:true&&a}=[]]}={});",
    ),
    (
        "later invalid leaf wins over a direct object frontier",
        "var a,x;[{x},true&&a]=[];",
    ),
    (
        "later invalid leaf wins over a recursive object frontier",
        "var a,x;[[{x}],true&&a]=[];",
    ),
    (
        "later invalid for-of leaf wins over an object frontier",
        "var a,x;for([{x},true&&a] of []){}",
    ),
    (
        "rest-last error wins over a nested object frontier",
        "var a,x;[...{x},true&&a]=[];",
    ),
    (
        "invalid logical target reports at the following delimiter",
        "var a;[true&&a]=[];",
    ),
    (
        "invalid call target reports at the following delimiter",
        "var a;[a()]=[];",
    ),
    (
        "invalid new target reports at the following delimiter",
        "var a;[new a]=[];",
    ),
    (
        "array assignment pattern cannot be a compound target",
        "var value;[value]+=[1]",
    ),
    (
        "array assignment pattern cannot be a logical-and target",
        "var value;[value]&&=[1]",
    ),
    (
        "array assignment pattern cannot be a logical-or target",
        "var value;[value]||=[1]",
    ),
    (
        "array assignment pattern cannot be a nullish target",
        "var value;[value]??=[1]",
    ),
    (
        "strict array assignment rejects eval",
        "'use strict';var value;[eval]=[value]",
    ),
    (
        "invalid object assignment leaf wins over the implementation frontier",
        "var a;({p: true && a} = {});",
    ),
    (
        "later invalid object leaf wins over an earlier nested object frontier",
        "var a,x;({p:[{x}],q:true&&a}={});",
    ),
    (
        "strict object assignment shorthand preserves the QuickJS target diagnostic",
        "'use strict';var x;({eval}={});",
    ),
];

const SMOKE_SOURCE: &str = "(function(){var answer;var source=[42];var result=([answer]=source);return answer+'|'+(result===source)})()";

#[test]
fn direct_array_assignments_match_pinned_quickjs() {
    compare_cases("direct array assignments", DIRECT_CASES);
}

#[test]
fn array_assignment_reference_order_matches_pinned_quickjs() {
    compare_cases("array assignment Reference order", REFERENCE_ORDER_CASES);
}

#[test]
fn array_assignment_iterator_close_matches_pinned_quickjs() {
    compare_cases("array assignment IteratorClose", ITERATOR_CLOSE_CASES);
}

#[test]
fn array_assignment_iterator_origin_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP array-assignment stack differential: set QJS_ORACLE to upstream qjs");
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
fn array_assignment_write_failures_match_pinned_quickjs() {
    compare_cases("array assignment writes", WRITE_CASES);
}

#[test]
fn array_assignment_loop_heads_match_pinned_quickjs() {
    compare_cases("array assignment loop heads", LOOP_CASES);
}

#[test]
fn array_assignment_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP array-assignment parser differential: set QJS_ORACLE to upstream qjs");
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
fn nested_object_assignment_frontier_remains_explicit() {
    for source in [
        "(function(){var value;({value}={value:42});return value})()",
        "(function(){var value;[{value}]=[{value:42}];return value})()",
        "(function(){var x;({p:[{x}]}={p:[{x:42}]});return x})()",
        "(function(){var x;[[{x}]]=[[{x:42}]];return x})()",
        "(function(){var x;for([{x}] of [{x:42}]){}return x})()",
    ] {
        let error = quickjs_oxide::compiler::compile_script(source)
            .expect_err("object assignment unexpectedly compiled");
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{source}");
        assert_eq!(
            error.message(),
            "object destructuring assignment patterns are not implemented yet",
            "{source}",
        );
    }
}

#[test]
fn invalid_object_assignment_leaf_is_syntax_before_the_frontier() {
    for source in [
        "var a;({p: true && a} = {});",
        "var a,x;({p:[{x}],q:true&&a}={});",
        "'use strict';var x;({eval}={});",
    ] {
        let error = quickjs_oxide::compiler::compile_script(source)
            .expect_err("invalid object assignment unexpectedly compiled");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "invalid destructuring target", "{source}");
    }
}

#[test]
fn array_assignment_rest_diagnostics_have_quickjs_priority() {
    for (source, message) in [
        ("var a;[...]=[];", "missing binding pattern..."),
        (
            "var a;[...[true&&a]=[]]=[];",
            "rest element cannot have a default value",
        ),
        (
            "var a;[...{p:true&&a}=[]]=[];",
            "rest element cannot have a default value",
        ),
        (
            "var a;({p:[...[true&&a]=[]]}={});",
            "rest element cannot have a default value",
        ),
        (
            "var a;({p:[...{q:true&&a}=[]]}={});",
            "rest element cannot have a default value",
        ),
        (
            "var a,x;[...{x},true&&a]=[];",
            "rest element must be the last one",
        ),
    ] {
        let error = quickjs_oxide::compiler::compile_script(source)
            .expect_err("invalid rest assignment unexpectedly compiled");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), message, "{source}");
    }
}

#[test]
fn invalid_array_assignment_leaf_is_syntax_before_the_object_frontier() {
    for source in [
        "var a,x;[{x},true&&a]=[];",
        "var a,x;[[{x}],true&&a]=[];",
        "var a,x;for([{x},true&&a] of []){}",
        "var a;[true&&a]=[];",
        "var a;[a()]=[];",
        "var a;[new a]=[];",
    ] {
        let error = quickjs_oxide::compiler::compile_script(source)
            .expect_err("invalid array assignment unexpectedly compiled");
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert_eq!(error.message(), "invalid destructuring target", "{source}");
    }
}

#[test]
fn array_assignment_smoke_runs_without_an_oracle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        observe_rust_eval(
            &runtime,
            &mut context,
            SMOKE_SOURCE,
            "array assignment smoke"
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
