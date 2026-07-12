use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{
    AccessorValue, CallableRef, Context, DescriptorField, JsString, ObjectRef,
    OrdinaryPropertyDescriptor, Runtime, RuntimeError, Value,
};

const VALUE_CASES: &[(&str, &str)] = &[
    (
        "custom iterator protocol",
        "(function(){function R(v,d){this.value=v;this.done=d};function I(){this.i=0};I.prototype.next=function(){this.i++;return new R(this.i,this.i>2)};function X(){};X.prototype[Symbol.iterator]=function(){return new I};var s='';for(var v of new X)s+=v;return s})()",
    ),
    (
        "iterator method receiver and next order",
        "(function(){function R(v,d){this.value=v;this.done=d};function I(x){this.x=x;this.i=0};I.prototype.next=function(){this.x.log+='n';this.i++;return new R(this.i,this.i>1)};function X(){this.log=''};X.prototype[Symbol.iterator]=function(){this.log+='i';return new I(this)};var x=new X,s='';for(var v of x)s+=v;return s+'|'+x.log})()",
    ),
    (
        "string iterator advances by Unicode code point",
        "(function(){var s='';for(var c of 'A\\uD83D\\uDCA9\\uD800B\\uDC00')s+=c.length+':'+c.charCodeAt(0)+'|';return s})()",
    ),
    (
        "string iterator exposes the shared iterator prototype contract",
        "(function(){var i='\\uD83D\\uDCA9'[Symbol.iterator](),a=i.next(),b=i.next();return (i[Symbol.iterator]()===i)+'|'+a.done+'|'+a.value.length+'|'+b.done+'|'+b.value})()",
    ),
    (
        "fixed member assignment target",
        "(function(){function B(){this.value=''}var b=new B,s='';for(b.value of 'xy')s+=b.value;return s+'|'+b.value})()",
    ),
    (
        "computed member assignment target",
        "(function(){function B(){this.value=''}var b=new B,k='value',s='';for(b[k] of 'xy')s+=b[k];return s+'|'+b.value})()",
    ),
    (
        "var binding keeps the final iterated value",
        "(function(){var s='';for(var value of 'ab')s+=value;return s+'|'+value})()",
    ),
    (
        "existing identifier assignment target",
        "(function(){var value='',s='';for(value of 'ab')s+=value;return s+'|'+value})()",
    ),
    (
        "let binding receives a fresh captured cell",
        "(function(){var f,g,i=0;for(let value of 'ab'){i++;if(i===1)f=function(){return value};else g=function(){return value}}return f()+'|'+g()+'|'+(f()!==g())})()",
    ),
    (
        "lexical head binding is in its temporal dead zone for the right operand",
        "(function(){let value='ab';try{for(let value of value)value}catch(e){return e.name+'|'+e.message}})()",
    ),
    (
        "const simple binding",
        "(function(){var s='';for(const value of 'ab')s+=value;return s})()",
    ),
    (
        "nested for-of values",
        "(function(){var s='';for(var a of 'ab')for(var b of '12')s+=a+b;return s})()",
    ),
    (
        "loop Script completion tracks the last entered body",
        "var forOfCompletion='';for(var forOfValue of 'ab'){forOfCompletion+=forOfValue;forOfCompletion}",
    ),
    (
        "zero-iteration Script completion remains undefined",
        "for(var x of ''){9}",
    ),
];

const CLOSE_CASES: &[(&str, &str)] = &[
    (
        "natural exhaustion does not call return",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.i=0;this.l=l};I.prototype.next=function(){this.i++;return new R(this.i,this.i>2)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,s='';for(var v of new X(l))s+=v;return s+'|'+l.s})()",
    ),
    (
        "break closes the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;for(var v of new X(l))break;return v+'|'+l.s})()",
    ),
    (
        "same-loop continue does not close the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.i=0;this.l=l};I.prototype.next=function(){this.i++;return new R(this.i,this.i>2)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,s='';for(var v of new X(l)){s+=v;continue}return s+'|'+l.s})()",
    ),
    (
        "same-loop labelled continue does not close the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.i=0;this.l=l};I.prototype.next=function(){this.i++;return new R(this.i,this.i>2)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,s='';loop:for(var v of new X(l)){s+=v;continue loop}return s+'|'+l.s})()",
    ),
    (
        "throw caught inside the loop does not close the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.i=0;this.l=l};I.prototype.next=function(){this.i++;return new R(this.i,this.i>2)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,s='';for(var v of new X(l)){try{throw v}catch(e){s+=e;continue}}return s+'|'+l.s})()",
    ),
    (
        "return expression is evaluated before iterator close",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(4,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;var value=(function(){for(var v of new X(l))return (l.s+='e',v)})();return value+'|'+l.s})()",
    ),
    (
        "body throw closes but close throw cannot replace the pending throw",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';throw 9};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for(var v of new X(l))throw 7}catch(e){return e+'|'+l.s}})()",
    ),
    (
        "body throw suppresses a primitive iterator-close result",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return 1};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for(var v of new X(l))throw 7}catch(e){return e+'|'+l.s}})()",
    ),
    (
        "non-callable return replaces break but not a pending throw",
        "(function(){function R(v,d){this.value=v;this.done=d};function I(){this.return=1};I.prototype.next=function(){return new R(1,false)};function X(){};X.prototype[Symbol.iterator]=function(){return new I};var normal,pending;try{for(var a of new X)break}catch(e){normal=e.name};try{for(var b of new X)throw 7}catch(e){pending=e};return normal+'|'+pending})()",
    ),
    (
        "break is replaced by a close throw",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';throw 9};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for(var v of new X(l))break}catch(e){return e+'|'+l.s}})()",
    ),
    (
        "return is replaced by a close throw",
        "(function(){function R(v,d){this.value=v;this.done=d};function I(){};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){throw 9};function X(){};X.prototype[Symbol.iterator]=function(){return new I};try{return (function(){for(var v of new X)return 5})()}catch(e){return e}})()",
    ),
    (
        "next throw does not call return",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){throw 8};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for(var v of new X(l))v}catch(e){return e+'|'+l.s}})()",
    ),
    (
        "assignment-target throw closes the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for((null).value of new X(l))0}catch(e){return e.name+'|'+l.s}})()",
    ),
    (
        "const write in the body closes the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for(const v of new X(l))v=2}catch(e){return e.name+'|'+l.s}})()",
    ),
    (
        "inner iterator closes before outer iterator on labelled break",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l,n){this.l=l;this.n=n};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+=this.n;return new R(0,true)};function X(l,n){this.l=l;this.n=n};X.prototype[Symbol.iterator]=function(){return new I(this.l,this.n)};var l=new L;outer:for(var a of new X(l,'o'))for(var b of new X(l,'i'))break outer;return l.s})()",
    ),
    (
        "labelled continue closes only the exited inner iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l,n,max){this.l=l;this.n=n;this.max=max;this.i=0};I.prototype.next=function(){this.i++;return new R(this.i,this.i>this.max)};I.prototype.return=function(){this.l.s+=this.n;return new R(0,true)};function X(l,n,max){this.l=l;this.n=n;this.max=max};X.prototype[Symbol.iterator]=function(){return new I(this.l,this.n,this.max)};var l=new L;outer:for(var a of new X(l,'o',2))for(var b of new X(l,'i',9))continue outer;return l.s})()",
    ),
    (
        "nested return closes iterators from inner to outer",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l,n){this.l=l;this.n=n};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+=this.n;return new R(0,true)};function X(l,n){this.l=l;this.n=n};X.prototype[Symbol.iterator]=function(){return new I(this.l,this.n)};var l=new L,value=(function(){for(var a of new X(l,'o'))for(var b of new X(l,'i'))return 5})();return value+'|'+l.s})()",
    ),
    (
        "switch break inside the body does not close the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l;this.i=0};I.prototype.next=function(){this.i++;return new R(this.i,this.i>1)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,s='';for(var v of new X(l)){switch(v){case 1:break}s+='b'}return s+'|'+l.s})()",
    ),
    (
        "inner block label break does not close the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l;this.i=0};I.prototype.next=function(){this.i++;return new R(this.i,this.i>1)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,s='';for(var v of new X(l)){inside:{break inside}s+='b'}return s+'|'+l.s})()",
    ),
    (
        "finally inside loop runs before iterator close",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;for(var v of new X(l)){try{break}finally{l.s+='f'}}return l.s})()",
    ),
    (
        "loop inside finally region closes before outer finally",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L;try{for(var v of new X(l))break}finally{l.s+='f'}return l.s})()",
    ),
    (
        "inner finally runs before iterator close on return",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,value=(function(){for(var v of new X(l)){try{return 5}finally{l.s+='f'}}})();return value+'|'+l.s})()",
    ),
    (
        "return inside finally discards its gosub address and closes the iterator",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,value=(function(){for(var v of new X(l)){try{return 4}finally{return 5}}})();return value+'|'+l.s})()",
    ),
    (
        "iterator closes before an outer finally on return",
        "(function(){function R(v,d){this.value=v;this.done=d};function L(){this.s=''};function I(l){this.l=l};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){this.l.s+='r';return new R(0,true)};function X(l){this.l=l};X.prototype[Symbol.iterator]=function(){return new I(this.l)};var l=new L,value=(function(){try{for(var v of new X(l))return 5}finally{l.s+='f'}})();return value+'|'+l.s})()",
    ),
];

const ERROR_CASES: &[(&str, &str)] = &[
    ("number is not iterable", "for(var value of 1)value"),
    (
        "non-callable iterator method",
        "function X(){}X.prototype[Symbol.iterator]=1;for(var value of new X)value",
    ),
    (
        "iterator method must return an object",
        "function X(){}X.prototype[Symbol.iterator]=function(){return 1};for(var value of new X)value",
    ),
    (
        "next method must be callable",
        "function I(){this.next=1}function X(){}X.prototype[Symbol.iterator]=function(){return new I};for(var value of new X)value",
    ),
    (
        "next must return an object",
        "function I(){}I.prototype.next=function(){return 1};function X(){}X.prototype[Symbol.iterator]=function(){return new I};for(var value of new X)value",
    ),
    (
        "close result must be an object on break",
        "function R(v,d){this.value=v;this.done=d};function I(){};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){return 1};function X(){};X.prototype[Symbol.iterator]=function(){return new I};for(var value of new X)break",
    ),
];

const SYNTAX_CASES: &[(&str, &str)] = &[
    (
        "var for-of declaration cannot have an initializer",
        "for(var value=1 of 'a')value",
    ),
    (
        "let for-of declaration cannot have an initializer",
        "for(let value=1 of 'a')value",
    ),
    (
        "const for-of declaration cannot have an initializer",
        "for(const value=1 of 'a')value",
    ),
    (
        "for-of declaration has one binding",
        "for(var first,second of 'a')first",
    ),
    ("for-of requires a right operand", "for(var value of)value"),
    (
        "async cannot be the bare for-of left expression",
        "for(async of 'a')async",
    ),
];

const STACK_CASES: &[(&str, &str)] = &[
    (
        "body fault closes the iterator without replacing its origin",
        "(function outer(){function R(v,d){this.value=v;this.done=d};function I(){};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){return new R(0,true)};function X(){};X.prototype[Symbol.iterator]=function(){return new I};for(var value of new X)(function body(){null.forOfBodyFault})()})()",
    ),
    (
        "close fault replaces break at the return method",
        "(function outer(){function R(v,d){this.value=v;this.done=d};function I(){};I.prototype.next=function(){return new R(1,false)};I.prototype.return=function(){(function close(){null.forOfCloseFault})()};function X(){};X.prototype[Symbol.iterator]=function(){return new I};for(var value of new X)break})()",
    ),
    (
        "next result type fault originates at loop advance",
        "(function outer(){function I(){};I.prototype.next=function next(){return 1};function X(){};X.prototype[Symbol.iterator]=function(){return new I};for(var value of new X)value})()",
    ),
];

const ACCESSOR_FIXTURE_BASE: &str = r#"
Function.forOfTrace='';
function ForOfResult(done,value,throwDone,throwValue){
    this.doneSlot=done;
    this.valueSlot=value;
    this.throwDone=throwDone;
    this.throwValue=throwValue;
}
function ForOfIterator(){this.i=0}
function ForOfCachedNext(){
    Function.forOfTrace+='c';
    this.i++;
    return new ForOfResult(this.i>this.limit,this.i,this.throwDone,this.throwValue);
}
function ForOfReturn(){
    Function.forOfTrace+='q';
    if(this.returnThrow)throw 10;
    if(this.returnPrimitive)return 1;
    return new ForOfResult(true,0,false,false);
}
"#;

const ACCESSOR_CASES: &[(&str, &str)] = &[
    (
        "done is read before value and a done result skips value",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;return i};var s='';for(var v of new X)s+=v;return s+'|'+Function.forOfTrace})()",
    ),
    (
        "done getter throw does not close the iterator",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;i.throwDone=true;return i};try{for(var v of new X)v}catch(e){return e+'|'+Function.forOfTrace}})()",
    ),
    (
        "value getter throw does not close the iterator",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;i.throwValue=true;return i};try{for(var v of new X)v}catch(e){return e+'|'+Function.forOfTrace}})()",
    ),
    (
        "next getter is cached before later prototype mutation",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;return i};var s='';for(var v of new X){s+=v;delete ForOfIteratorPrototype.next;ForOfIteratorPrototype.next=function(){throw 11}}return s+'|'+Function.forOfTrace})()",
    ),
    (
        "nullish return getters complete break normally",
        "(function(){function X(mode){this.mode=mode};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;i.returnNull=this.mode===1;i.returnUndefined=this.mode===2;return i};for(var a of new X(1))break;for(var b of new X(2))break;return Function.forOfTrace})()",
    ),
    (
        "return getter throw replaces break but not a pending throw",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;i.returnGetterThrow=true;return i};var normal,pending;try{for(var a of new X)break}catch(e){normal=e};try{for(var b of new X)throw 7}catch(e){pending=e};return normal+'|'+pending+'|'+Function.forOfTrace})()",
    ),
    (
        "non-callable return getter result replaces break but not a pending throw",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;i.returnNonCallable=true;return i};var normal,pending;try{for(var a of new X)break}catch(e){normal=e.name};try{for(var b of new X)throw 7}catch(e){pending=e};return normal+'|'+pending+'|'+Function.forOfTrace})()",
    ),
    (
        "primitive return call result replaces break but not a pending throw",
        "(function(){function X(){};X.prototype[Symbol.iterator]=function(){var i=new ForOfIterator;i.limit=1;i.returnPrimitive=true;return i};var normal,pending;try{for(var a of new X)break}catch(e){normal=e.name};try{for(var b of new X)throw 7}catch(e){pending=e};return normal+'|'+pending+'|'+Function.forOfTrace})()",
    ),
];

#[test]
fn for_of_values_match_pinned_quickjs() {
    compare_value_cases("for-of values", VALUE_CASES);
}

#[test]
fn iterator_close_values_match_pinned_quickjs() {
    compare_value_cases("IteratorClose", CLOSE_CASES);
}

#[test]
fn for_of_protocol_errors_match_pinned_quickjs() {
    compare_value_cases("for-of protocol errors", ERROR_CASES);
}

#[test]
fn for_of_accessor_protocol_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP for-of accessor differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let oracle_setup = oracle_accessor_setup();
    for &(description, source) in ACCESSOR_CASES {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        install_accessor_fixture(&runtime, &mut context);
        assert_eq!(
            observe_rust_eval(&runtime, &mut context, source, description),
            observe_oracle_with_setup(&oracle, &oracle_setup, source, description),
            "for-of accessor protocol drifted for {description}: {source:?}",
        );
    }
}

#[test]
fn for_of_parser_diagnostics_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP for-of parser differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in SYNTAX_CASES {
        compare_cli(&oracle, &[], source, description);
    }
}

#[test]
fn for_of_full_strip_source_and_strip_debug_stacks_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP for-of stack differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(description, source) in STACK_CASES {
        compare_cli(&oracle, &[], source, description);
        compare_cli(&oracle, &["--strip-source"], source, description);
        compare_cli(&oracle, &["-s"], source, description);
    }
}

#[test]
fn for_in_destructuring_and_for_await_boundaries_remain_explicit() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    for (source, expected) in [
        (
            "for(var key in Function)key",
            "for-in loops are not implemented yet",
        ),
        (
            "for(let [value] of 'a')value",
            "for-of destructuring bindings are not implemented yet",
        ),
        (
            "for await(var value of 'a')value",
            "for-await-of loops are not implemented yet",
        ),
        (
            "for(var value of /a;b/)value",
            "this literal form is not implemented yet",
        ),
    ] {
        let Err(RuntimeError::Exception) = context.compile(source) else {
            panic!("for-of boundary was not rejected explicitly: {source}");
        };
        let Value::Object(error) = context
            .take_exception()
            .expect("take for-of boundary exception")
            .expect("for-of boundary exception is present")
        else {
            panic!("for-of boundary did not materialize an Error object: {source}");
        };
        assert_eq!(
            error_string_property(&runtime, &mut context, &error, "message", source),
            expected,
            "{source}",
        );
    }
}

#[test]
fn for_of_cross_realm_regression() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let bytecode = defining
        .compile("(function(){var result='';for(var value of 'ab')result+=value;return result})()")
        .unwrap();
    assert_eq!(
        caller.execute(&bytecode).unwrap(),
        Value::String(JsString::try_from_utf8("ab").unwrap())
    );
}

fn install_accessor_fixture(runtime: &Runtime, context: &mut Context) {
    context
        .eval(ACCESSOR_FIXTURE_BASE)
        .expect("install for-of accessor fixture functions");
    let object_prototype = context.object_prototype().unwrap();
    let result_prototype = runtime.new_object(Some(&object_prototype)).unwrap();
    let iterator_prototype = runtime.new_object(Some(&object_prototype)).unwrap();

    let done_getter = function(
        runtime,
        context,
        "(function(){Function.forOfTrace+='d';if(this.throwDone)throw 7;return this.doneSlot})",
    );
    let value_getter = function(
        runtime,
        context,
        "(function(){Function.forOfTrace+='v';if(this.throwValue)throw 8;return this.valueSlot})",
    );
    let next_getter = function(
        runtime,
        context,
        "(function(){Function.forOfTrace+='n';return ForOfCachedNext})",
    );
    let return_getter = function(
        runtime,
        context,
        "(function(){Function.forOfTrace+='r';if(this.returnGetterThrow)throw 9;if(this.returnNull)return null;if(this.returnUndefined)return undefined;if(this.returnNonCallable)return 1;return ForOfReturn})",
    );
    define_accessor(runtime, context, &result_prototype, "done", done_getter);
    define_accessor(runtime, context, &result_prototype, "value", value_getter);
    define_accessor(runtime, context, &iterator_prototype, "next", next_getter);
    define_accessor(
        runtime,
        context,
        &iterator_prototype,
        "return",
        return_getter,
    );
    define_global(
        runtime,
        context,
        "ForOfResultPrototype",
        Value::Object(result_prototype),
    );
    define_global(
        runtime,
        context,
        "ForOfIteratorPrototype",
        Value::Object(iterator_prototype),
    );
    context
        .eval(
            "ForOfResult.prototype=ForOfResultPrototype;ForOfIterator.prototype=ForOfIteratorPrototype;0",
        )
        .expect("connect for-of accessor fixture prototypes");
}

fn oracle_accessor_setup() -> String {
    format!(
        r#"{ACCESSOR_FIXTURE_BASE}
var ForOfResultPrototype=Object.create(Object.prototype);
Object.defineProperty(ForOfResultPrototype,'done',{{configurable:true,get:function(){{Function.forOfTrace+='d';if(this.throwDone)throw 7;return this.doneSlot}}}});
Object.defineProperty(ForOfResultPrototype,'value',{{configurable:true,get:function(){{Function.forOfTrace+='v';if(this.throwValue)throw 8;return this.valueSlot}}}});
ForOfResult.prototype=ForOfResultPrototype;
var ForOfIteratorPrototype=Object.create(Object.prototype);
Object.defineProperty(ForOfIteratorPrototype,'next',{{configurable:true,get:function(){{Function.forOfTrace+='n';return ForOfCachedNext}}}});
Object.defineProperty(ForOfIteratorPrototype,'return',{{configurable:true,get:function(){{Function.forOfTrace+='r';if(this.returnGetterThrow)throw 9;if(this.returnNull)return null;if(this.returnUndefined)return undefined;if(this.returnNonCallable)return 1;return ForOfReturn}}}});
ForOfIterator.prototype=ForOfIteratorPrototype;
"#
    )
}

fn function(runtime: &Runtime, context: &mut Context, source: &str) -> CallableRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("for-of fixture did not produce a function: {source}");
    };
    runtime.as_callable(&object).unwrap().unwrap()
}

fn define_accessor(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
    getter: CallableRef,
) {
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                object,
                &key,
                &OrdinaryPropertyDescriptor {
                    get: DescriptorField::Present(AccessorValue::Callable(getter)),
                    set: DescriptorField::Present(AccessorValue::Undefined),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn define_global(runtime: &Runtime, context: &mut Context, name: &str, value: Value) {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        context
            .define_own_property(
                &global,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
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

fn observe_oracle_with_setup(
    oracle: &OsStr,
    setup: &str,
    source: &str,
    description: &str,
) -> String {
    let wrapper = r#"
std.evalScript(scriptArgs[0]);
try {
  var value = std.evalScript(scriptArgs[1]);
  print('return|' + typeof value + '|' + String(value));
} catch (error) {
  if (error !== null && typeof error === 'object')
    print('throw|object|' + error.name + '|' + error.message);
  else
    print('throw|' + typeof error + '|' + String(error));
}
"#;
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, setup, source])
        .output()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS accessor observer failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"));
    stdout.strip_suffix('\n').unwrap_or(&stdout).to_owned()
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
