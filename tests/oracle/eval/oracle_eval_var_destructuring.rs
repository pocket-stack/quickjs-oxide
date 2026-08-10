//! Differential coverage for sloppy direct-eval `var` BindingPatterns.
//!
//! QuickJS keeps imported eval-variable objects as late scope candidates for
//! these References. The ordinary cases must update the predeclared eval
//! property, while a getter/default/iterator that deletes that property must
//! still retarget the later write to the global fallback.

use std::ffi::OsStr;
use std::process::{Command, Output};

const MATRIX_SOURCE: &str = r#"
function emit(label, value, cleanup) {
    print(label + "=" + value);
    for (var name of cleanup) delete globalThis[name];
}

emit("array-normal", (function () {
    return eval(`var [y]=[2];Object.prototype.hasOwnProperty.call(globalThis,"y")+"|"+typeof globalThis.y+"|"+String(globalThis.y)+"|"+typeof y+"|"+String(y)`);
})(), ["y"]);

emit("object-normal", (function () {
    return eval(`var {a:y}={get a(){return 2}};Object.prototype.hasOwnProperty.call(globalThis,"y")+"|"+typeof globalThis.y+"|"+String(globalThis.y)+"|"+typeof y+"|"+String(y)`);
})(), ["y"]);

emit("object-names", (function () {
    return eval(`var {fn=function(){},gen=function*(){},af=async function(){},arrow=()=>{},aa=async()=>{},cls=class{}}={};[fn,gen,af,arrow,aa,cls].map(function(value){return value.name}).join(",")`);
})(), ["fn", "gen", "af", "arrow", "aa", "cls"]);

emit("array-names", (function () {
    return eval(`var [fn=function(){},gen=function*(){},af=async function(){},arrow=()=>{},aa=async()=>{},cls=class{}]=[];[fn,gen,af,arrow,aa,cls].map(function(value){return value.name}).join(",")`);
})(), ["fn", "gen", "af", "arrow", "aa", "cls"]);

emit("existing", (function () {
    var x = 1;
    eval(`var {x=23,fn=function(){}}={}`);
    return x + "|" + fn.name;
})(), ["fn"]);

emit("duplicate-eval", (function () {
    eval(`var {d=function(){}}={}`);
    var first = d.name;
    eval(`var [d=function(){}]=[]`);
    return first + ">" + d.name;
})(), ["d"]);

emit("object-order", (function () {
    var log = [];
    var src = {
        get a() { log.push("get:a"); return undefined; },
        get b() { log.push("get:b"); return 42; }
    };
    function key(value) { log.push("key:" + value); return value; }
    function init(value) { log.push("init:" + value); return value === "a" ? 31 : 32; }
    var result = eval(`var {[key("a")]:oa=init("a"),[key("b")]:ob=init("b")}=src;String(oa)+","+String(ob)`);
    return log.join(">") + "|" + result;
})(), ["oa", "ob"]);

emit("array-order", (function () {
    var log = [];
    var step = 0;
    var iterable = {
        [Symbol.iterator]: function () {
            log.push("iter");
            return {
                next: function () {
                    var current = step++;
                    log.push("next:" + current);
                    return {
                        get done() { log.push("done:" + current); return false; },
                        get value() { log.push("value:" + current); return current === 0 ? undefined : 52; }
                    };
                },
                return: function () { log.push("return"); return {done:true}; }
            };
        }
    };
    function initArray() { log.push("init"); return 51; }
    var result = eval(`var [ia=initArray(),ib]=iterable;String(ia)+","+String(ib)`);
    return log.join(">") + "|" + result;
})(), ["ia", "ib"]);

emit("array-late-next", (function () {
    return eval(`var i=0;var [y]={ [Symbol.iterator](){return {next(){if(i++===0){delete y;return {value:2,done:false}}return {done:true}}}}};var own=Object.prototype.hasOwnProperty.call(globalThis,"y");var g=globalThis.y;delete globalThis.y;var after;try{after=String(y)}catch(e){after=e.name}own+"|"+g+"|"+after`);
})(), ["y"]);

emit("object-late-getter", (function () {
    return eval(`var {a:y}={get a(){delete y;return 2}};var own=Object.prototype.hasOwnProperty.call(globalThis,"y");var g=globalThis.y;delete globalThis.y;var after;try{after=String(y)}catch(e){after=e.name}own+"|"+g+"|"+after`);
})(), ["y"]);

emit("array-late-default", (function () {
    return eval(`var [y=(delete y,2)]=[undefined];var own=Object.prototype.hasOwnProperty.call(globalThis,"y");var g=globalThis.y;delete globalThis.y;var after;try{after=String(y)}catch(e){after=e.name}own+"|"+g+"|"+after`);
})(), ["y"]);

emit("object-rest-late-getter", (function () {
    return eval(`var {...y}={get a(){delete y;return 2}};var own=Object.prototype.hasOwnProperty.call(globalThis,"y");var g=globalThis.y;delete globalThis.y;var after;try{after=String(y)}catch(e){after=e.name}own+"|"+g.a+"|"+after`);
})(), ["y"]);
"#;

const EXPECTED_STDOUT: &str = concat!(
    "array-normal=false|undefined|undefined|number|2\n",
    "object-normal=false|undefined|undefined|number|2\n",
    "object-names=fn,gen,af,arrow,aa,cls\n",
    "array-names=fn,gen,af,arrow,aa,cls\n",
    "existing=23|fn\n",
    "duplicate-eval=d>d\n",
    "object-order=key:a>get:a>init:a>key:b>get:b|31,42\n",
    "array-order=iter>next:0>done:0>value:0>init>next:1>done:1>value:1>return|51,52\n",
    "array-late-next=true|2|ReferenceError\n",
    "object-late-getter=true|2|ReferenceError\n",
    "array-late-default=true|2|ReferenceError\n",
    "object-rest-late-getter=true|2|ReferenceError\n",
);

#[test]
fn sloppy_eval_var_destructuring_matches_pinned_quickjs() {
    let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref());
    assert_success("quickjs-oxide", &oxide);

    if let Some(oracle) = std::env::var_os("QJS_ORACLE") {
        let quickjs = run(&oracle);
        assert_success("pinned QuickJS", &quickjs);
        assert_eq!(
            oxide.stdout, quickjs.stdout,
            "sloppy eval-var destructuring output differed from pinned QuickJS"
        );
    } else {
        eprintln!("SKIP sloppy eval-var differential: set QJS_ORACLE to pinned upstream qjs");
    }
}

fn assert_success(engine: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected the sloppy eval-var destructuring matrix: {}\nsource:\n{MATRIX_SOURCE}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        EXPECTED_STDOUT,
        "{engine} output drifted for the sloppy eval-var destructuring matrix"
    );
}

fn run(executable: &OsStr) -> Output {
    Command::new(executable)
        .args(["-e", MATRIX_SOURCE])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}
