use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DebugInfoMode, ErrorKind, JsString, Runtime, RuntimeError, Value,
};

// This probe deliberately isolates the non-String shell from source execution.
// It freezes the realm-local %eval% callable and identity semantics separately
// from the compiler and environment assertions below.
const ORACLE_PROBE: &str = r#"
(function () {
    function flags(descriptor) {
        return (descriptor.writable ? "1" : "0") +
               (descriptor.enumerable ? "1" : "0") +
               (descriptor.configurable ? "1" : "0");
    }
    function isConstructor(value) {
        try {
            Reflect.construct(function () {}, [], value);
            return true;
        } catch (_) {
            return false;
        }
    }

    var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "eval");
    var lengthDescriptor = Object.getOwnPropertyDescriptor(eval, "length");
    var nameDescriptor = Object.getOwnPropertyDescriptor(eval, "name");
    var observations = [
        "metadata=" + [
            typeof eval,
            eval.name,
            eval.length,
            Object.getPrototypeOf(eval) === Function.prototype,
            Object.prototype.hasOwnProperty.call(eval, "prototype"),
            eval.prototype === undefined,
            isConstructor(eval),
            Object.getOwnPropertyNames(eval).join(","),
            globalDescriptor.value === eval,
            flags(globalDescriptor),
            flags(lengthDescriptor),
            flags(nameDescriptor)
        ].join("|")
    ];

    var constructError = "none";
    try {
        new eval();
    } catch (error) {
        constructError = error.name;
    }
    observations.push("construct=" + constructError);

    var marker = {};
    var alias = eval;
    observations.push("calls=" + [
        eval(marker) === marker,
        (eval)(marker) === marker,
        ((eval))(marker) === marker,
        \u0065val(marker) === marker,
        (function (eval) { return eval(marker) === marker; })(eval),
        (0, eval)(marker) === marker,
        alias(marker) === marker,
        globalThis.eval(marker) === marker,
        eval.call(null, marker) === marker,
        eval.apply(null, [marker]) === marker,
        eval() === undefined
    ].join("|"));

    var coercions = 0;
    function poison() {
        coercions++;
        throw 99;
    }
    var ordinary = {};
    ordinary[Symbol.toPrimitive] = poison;
    ordinary.toString = poison;
    ordinary.valueOf = poison;
    var boxedString = new String("40 + 2");
    boxedString[Symbol.toPrimitive] = poison;
    boxedString.toString = poison;
    boxedString.valueOf = poison;
    var symbol = Symbol("eval source");
    observations.push("identity=" + [
        eval(ordinary) === ordinary,
        eval(boxedString) === boxedString,
        eval(symbol) === symbol,
        coercions
    ].join("|"));

    var held = eval;
    var deleted = delete globalThis.eval;
    var absent = typeof globalThis.eval === "undefined";
    var heldAfterDelete = held(ordinary) === ordinary;
    globalThis.eval = function replacement() { return 17; };
    var replacementVisible = globalThis.eval(ordinary) === 17;
    var heldAfterReplacement = held(ordinary) === ordinary;
    observations.push("mutation=" + [
        deleted,
        absent,
        heldAfterDelete,
        replacementVisible,
        heldAfterReplacement,
        coercions
    ].join("|"));

    return observations.join("\n");
})()
"#;

const EXPECTED_OBSERVATIONS: &[&str] = &[
    "metadata=function|eval|1|true|false|true|false|length,name|true|101|001|001",
    "construct=TypeError",
    "calls=true|true|true|true|true|true|true|true|true|true|true",
    "identity=true|true|true|0",
    "mutation=true|true|true|true|true|0",
];

// This probe freezes which syntactic forms receive a caller lexical environment
// and which forms take the ordinary, indirect call path.
const DIRECTNESS_ORACLE_PROBE: &str = r#"
(function () {
    globalThis.x = "G";
    var result = (function () {
        var x = "L";
        var original = eval;
        var alias = eval;
        var observations = [
            eval("x"),
            (eval)("x"),
            ((eval))("x"),
            \u0065val("x"),
            (function (eval) { return eval("x"); })(original),
            (0, eval)("x"),
            alias("x"),
            globalThis.eval("x"),
            eval.call(null, "x"),
            eval.apply(null, ["x"]),
            (true ? eval : eval)("x"),
            (eval = original)("x"),
            (function () {
                try { new eval("x"); return "none"; }
                catch (error) { return error.name; }
            })(),
            (function (eval) { return eval("x", 7); })(
                function replacement(source, extra) {
                    return "R:" + source + ":" + extra;
                }
            )
        ];
        globalThis.eval = function replacement(source, extra) {
            return "R:" + source + ":" + extra;
        };
        observations.push(eval("x", 8));
        globalThis.eval = original;
        observations.push(eval(...["x"]));
        observations.push(eval?.("x"));
        return observations.join("|");
    })();
    delete globalThis.x;
    return result;
})()
"#;

const EXPECTED_DIRECTNESS: &str = "L|L|L|L|L|G|G|G|G|G|G|G|TypeError|R:x:7|R:x:8|L|G";

// Keep the executable Rust slice inside syntax that the main parser already
// supports. Spread-call and optional-call parsing remain separate milestones;
// the full QuickJS contract above stays frozen by the oracle-only probe.
const R1X_DIRECTNESS_PROBE: &str = r#"
(function () {
    globalThis.x = "G";
    var result = (function () {
        var x = "L";
        var original = eval;
        var alias = eval;
        return [
            eval("x"),
            (eval)("x"),
            ((eval))("x"),
            \u0065val("x"),
            (function (eval) { return eval("x"); })(original),
            (0, eval)("x"),
            alias("x"),
            globalThis.eval("x"),
            eval.call(null, "x"),
            eval.apply(null, ["x"]),
            (true ? eval : eval)("x"),
            (eval = original)("x")
        ].join("|");
    })();
    delete globalThis.x;
    return result;
})()
"#;

const EXPECTED_R1X_DIRECTNESS: &str = "L|L|L|L|L|G|G|G|G|G|G|G";

// This remains oracle-only until String execution opens. It freezes the
// environment contract which the Oxide descriptor/materialization milestone
// now represents without pretending that source compilation is complete.
const ENVIRONMENT_ORACLE_PROBE: &str = r#"
(function () {
    var observations = [];
    function sloppy(argument) {
        var local = 2;
        {
            let block = 3;
            observations.push("direct=" +
                eval("[argument,local,block,this.tag,arguments[0]].join(',')"));
            eval("local=4; var added=5; let hidden=6");
        }
        observations.push("sloppy=" + [local, added, typeof hidden].join(","));
    }
    sloppy.call({ tag: "T" }, 1);

    function strict() {
        "use strict";
        var outer = 1;
        var within = eval(
            "outer=2; var onlyVar=3; let onlyLex=4; [onlyVar,onlyLex].join(',')"
        );
        observations.push(
            "strict=" + [within, outer, typeof onlyVar, typeof onlyLex].join("|")
        );
    }
    strict();

    function redeclaration() {
        let conflict = 1;
        globalThis.evalTouch = 0;
        try {
            eval("evalTouch=1; var conflict");
        } catch (error) {
            observations.push(
                "redeclare=" + [error.name, evalTouch, conflict].join(",")
            );
        }
        delete globalThis.evalTouch;
    }
    redeclaration();

    function C(argument) {
        observations.push(
            "special=" +
            eval("[new.target===C,arguments[0],this instanceof C].join(',')")
        );
    }
    new C(7);

    observations.push(
        "indirect=" +
        (0, eval)(
            "var indirectVar=8; let indirectLex=9; " +
            "[this===globalThis,indirectVar,typeof indirectLex].join(',')"
        )
    );
    observations.push(
        "indirectAfter=" +
        [globalThis.indirectVar, typeof indirectLex, delete globalThis.indirectVar].join(",")
    );
    observations.push(
        "indirectStrict=" +
        (0, eval)(
            "'use strict'; var strictVar=10; let strictLex=11; " +
            "[this===globalThis,strictVar,typeof strictLex].join(',')"
        )
    );
    observations.push(
        "indirectStrictAfter=" + [typeof strictVar, typeof strictLex].join(",")
    );
    return observations.join("\n");
})()
"#;

const EXPECTED_ENVIRONMENT: &[&str] = &[
    "direct=1,2,3,T,1",
    "sloppy=4,5,undefined",
    "strict=3,4|2|undefined|undefined",
    "redeclare=SyntaxError,0,1",
    "special=true,7,true",
    "indirect=true,8,number",
    "indirectAfter=8,undefined,true",
    "indirectStrict=true,10,number",
    "indirectStrictAfter=undefined,undefined",
];

// R1y freezes QuickJS's hidden per-activation `<var>` object rather than
// approximating novel sloppy direct-eval names as compiler-created locals.
const R1Y_FUNCTION_DECLARATION_PROBE: &str = r#"
(function () {
    var observations = [];
    function sloppy(parameter) {
        var existing = 1;
        function compiledBeforeEval() {
            return [fresh, existing, parameter].join(",");
        }
        var completion = eval(
            "var fresh=40; existing=2; parameter=3; fresh+2"
        );
        observations.push(
            "sloppy=" + [
                completion,
                compiledBeforeEval(),
                existing,
                parameter,
                delete fresh,
                typeof fresh
            ].join("|")
        );
    }
    sloppy(9);

    function retained() {
        eval("var answer=42");
        return function () { return answer; };
    }
    observations.push("closure=" + retained()());

    function repeated() {
        eval("var x=1");
        eval("var x");
        return typeof x + "|" + String(x);
    }
    observations.push("repeat=" + repeated());

    function functionThenVar() {
        return eval("function x(){}; var x; typeof x");
    }
    function varThenFunction() {
        return eval("var x; function x(){}; typeof x");
    }
    observations.push(
        "order=" + functionThenVar() + "|" + varThenFunction()
    );
    return observations.join("\n");
})()
"#;

const EXPECTED_R1Y_FUNCTION_DECLARATIONS: &[&str] = &[
    "sloppy=42|40,2,3|2|3|true|undefined",
    "closure=42",
    "repeat=undefined|undefined",
    "order=undefined|function",
];

const R1Y_EVAL_KIND_DECLARATION_PROBE: &str = r#"
(function () {
    var observations = [];
    function strictCaller() {
        "use strict";
        var value = eval(
            "var strictVar=40; function strictFn(){return strictVar+2} strictFn()"
        );
        return [value, typeof strictVar, typeof strictFn].join("|");
    }
    function strictSource() {
        var value = eval(
            "'use strict'; var strictVar=40; " +
            "function strictFn(){return strictVar+2} strictFn()"
        );
        return [value, typeof strictVar, typeof strictFn].join("|");
    }
    observations.push(
        "strict=" + strictCaller() + "|" + strictSource()
    );

    var indirect = (0, eval)(
        "var indirectVar=40; " +
        "function indirectFn(){return indirectVar+2} indirectFn()"
    );
    var varFlags = Object.getOwnPropertyDescriptor(globalThis, "indirectVar");
    var fnFlags = Object.getOwnPropertyDescriptor(globalThis, "indirectFn");
    observations.push(
        "indirect=" + [
            indirect,
            indirectFn(),
            varFlags.configurable,
            fnFlags.configurable,
            delete globalThis.indirectVar,
            delete globalThis.indirectFn
        ].join("|")
    );

    var indirectStrict = (0, eval)(
        "'use strict'; var strictVar=40; " +
        "function strictFn(){return strictVar+2} strictFn()"
    );
    observations.push(
        "indirectStrict=" + [
            indirectStrict,
            typeof strictVar,
            typeof strictFn
        ].join("|")
    );
    return observations.join("\n");
})()
"#;

const EXPECTED_R1Y_EVAL_KIND_DECLARATIONS: &[&str] = &[
    "strict=42|undefined|undefined|42|undefined|undefined",
    "indirect=42|42|true|true|true|true",
    "indirectStrict=42|undefined|undefined",
];

const R1Y_CALLER_BINDING_PROBE: &str = r#"
(function () {
    var observations = [];
    function caught() {
        try {
            throw 1;
        } catch (error) {
            eval("var error=42");
            return error;
        }
    }
    observations.push("catch=" + caught());

    function sourceCatch() {
        var sloppy = eval("try{throw 1}catch(e){var e=2;e}");
        var strict = eval(
            "'use strict';try{throw 1}catch(e){var e=2;e}"
        );
        return sloppy + "|" + strict;
    }
    observations.push("sourceCatch=" + sourceCatch());

    function outer() {
        let value = 1;
        var inner = (function () {
            eval("var value=2");
            return value;
        })();
        return inner + "|" + value;
    }
    observations.push("outer=" + outer());

    function conflict() {
        let value = 1;
        try {
            eval("var value=2");
            return "none";
        } catch (error) {
            return error.name + "|" + value;
        }
    }
    observations.push("conflict=" + conflict());

    function named(argument) {
        eval("var arguments=1; var named=2");
        return [
            arguments[0],
            named,
            eval("arguments"),
            eval("named"),
            eval("delete arguments"),
            eval("delete named"),
            arguments[0],
            typeof named
        ].join("|");
    }
    observations.push("special=" + named(7));
    return observations.join("\n");
})()
"#;

const EXPECTED_R1Y_CALLER_BINDINGS: &[&str] = &[
    "catch=42",
    "sourceCatch=2|2",
    "outer=2|1",
    "conflict=SyntaxError|1",
    "special=7|2|1|2|true|true|7|function",
];

const R1Y_DECLARATION_CONFLICT_PROBE: &str = r#"
var observations = [];
{
    let blockConflict = 1;
    try {
        eval(
            "function functionBeforeThrow(){}; " +
            "var createdBeforeThrow; var blockConflict"
        );
        observations.push("block=none");
    } catch (error) {
        observations.push(
            "block=" + [
                error.name,
                Object.prototype.hasOwnProperty.call(
                    globalThis,
                    "createdBeforeThrow"
                ),
                Object.prototype.hasOwnProperty.call(globalThis, "blockConflict"),
                Object.prototype.hasOwnProperty.call(
                    globalThis,
                    "functionBeforeThrow"
                ),
                String(globalThis.createdBeforeThrow),
                String(globalThis.blockConflict),
                typeof globalThis.functionBeforeThrow
            ].join("|")
        );
    }
}
delete globalThis.createdBeforeThrow;
delete globalThis.blockConflict;
delete globalThis.functionBeforeThrow;

let globalLocked = 1;
try {
    eval("var preflightMustNotExist; var globalLocked");
    observations.push("global=none");
} catch (error) {
    observations.push(
        "global=" + [
            error.name,
            Object.prototype.hasOwnProperty.call(
                globalThis,
                "preflightMustNotExist"
            ),
            Object.prototype.hasOwnProperty.call(globalThis, "globalLocked"),
            globalLocked
        ].join("|")
    );
}

var scriptVar = 1;
function scriptFn() {}
eval("var evalVar=1; function evalFn(){}");
observations.push(
    "flags=" + [
        Object.getOwnPropertyDescriptor(globalThis, "scriptVar").configurable,
        Object.getOwnPropertyDescriptor(globalThis, "scriptFn").configurable,
        Object.getOwnPropertyDescriptor(globalThis, "evalVar").configurable,
        Object.getOwnPropertyDescriptor(globalThis, "evalFn").configurable,
        delete globalThis.evalVar,
        delete globalThis.evalFn
    ].join("|")
);
observations.join("\n")
"#;

const EXPECTED_R1Y_DECLARATION_CONFLICTS: &[&str] = &[
    "block=SyntaxError|true|true|true|undefined|undefined|undefined",
    "global=SyntaxError|false|false|1",
    "flags=false|false|true|true|true|true",
];

const R1Y_LABELLED_FUNCTION_PROBE: &str = r#"
(function () {
    var observations = [];
    function direct() {
        var lexicalClosure;
        eval(
            "label: function local(){return 42;} " +
            "lexicalClosure=local"
        );
        return [
            lexicalClosure === local,
            lexicalClosure(),
            local()
        ].join("|");
    }
    observations.push("direct=" + direct());

    globalThis.labelledClosure = undefined;
    Object.defineProperty(globalThis, "labelled", {
        configurable: true,
        get: function () { return 9; },
        set: function (value) { labelledClosure = value; }
    });
    var inside = (0, eval)(
        "label: function labelled(){return 42;} " +
        "[typeof labelled,labelled(),labelledClosure===labelled].join('|')"
    );
    observations.push(
        "global=" + [
            inside,
            typeof labelled,
            labelled,
            labelledClosure(),
            Object.getOwnPropertyDescriptor(globalThis, "labelled").configurable
        ].join("|")
    );
    delete globalThis.labelled;
    delete globalThis.labelledClosure;
    return observations.join("\n");
})()
"#;

const EXPECTED_R1Y_LABELLED_FUNCTIONS: &[&str] = &[
    "direct=false|42|42",
    "global=function|42|false|number|9|42|true",
];

#[test]
fn eval_shell_matches_pinned_quickjs() {
    let rust = rust_observations();
    assert_eq!(rust, EXPECTED_OBSERVATIONS, "host-side eval shell drifted");

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval intrinsic differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_observations(&oracle),
        "eval intrinsic shell differed from pinned QuickJS"
    );
}

#[test]
fn pinned_quickjs_direct_eval_syntax_contract_is_frozen() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP direct eval syntax differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        oracle_value(&oracle, DIRECTNESS_ORACLE_PROBE),
        EXPECTED_DIRECTNESS,
        "pinned QuickJS direct/indirect eval classification drifted"
    );
}

#[test]
fn primitive_string_directness_matches_the_supported_quickjs_slice() {
    assert_eq!(
        rust_value(R1X_DIRECTNESS_PROBE),
        EXPECTED_R1X_DIRECTNESS,
        "Rust direct/indirect eval classification drifted",
    );
}

#[test]
fn pinned_quickjs_eval_environment_contract_is_frozen() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval environment differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        oracle_value(&oracle, ENVIRONMENT_ORACLE_PROBE)
            .lines()
            .collect::<Vec<_>>(),
        EXPECTED_ENVIRONMENT,
        "pinned QuickJS eval environment contract drifted"
    );
}

#[test]
fn eval_var_objects_and_source_order_match_pinned_quickjs() {
    let rust = rust_value(R1Y_FUNCTION_DECLARATION_PROBE);
    assert_eq!(
        rust.lines().collect::<Vec<_>>(),
        EXPECTED_R1Y_FUNCTION_DECLARATIONS,
        "Rust sloppy eval declaration environment drifted",
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval declaration differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_value(&oracle, R1Y_FUNCTION_DECLARATION_PROBE),
        "eval variable-object behavior differed from pinned QuickJS",
    );
}

#[test]
fn strict_and_indirect_eval_declarations_match_pinned_quickjs() {
    let rust = rust_value(R1Y_EVAL_KIND_DECLARATION_PROBE);
    assert_eq!(
        rust.lines().collect::<Vec<_>>(),
        EXPECTED_R1Y_EVAL_KIND_DECLARATIONS,
        "Rust strict/indirect eval declaration environment drifted",
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval-kind declaration differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_value(&oracle, R1Y_EVAL_KIND_DECLARATION_PROBE),
        "strict/indirect eval declarations differed from pinned QuickJS",
    );
}

#[test]
fn eval_declarations_respect_catch_lexicals_and_implicit_bindings() {
    let rust = rust_value(R1Y_CALLER_BINDING_PROBE);
    assert_eq!(
        rust.lines().collect::<Vec<_>>(),
        EXPECTED_R1Y_CALLER_BINDINGS,
        "Rust caller-binding precedence for eval declarations drifted",
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP eval caller-binding differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(
        rust,
        oracle_value(&oracle, R1Y_CALLER_BINDING_PROBE),
        "eval caller-binding precedence differed from pinned QuickJS",
    );
}

#[test]
fn eval_declaration_conflicts_and_global_flags_match_pinned_quickjs() {
    let rust = rust_value(R1Y_DECLARATION_CONFLICT_PROBE);
    assert_eq!(
        rust.lines().collect::<Vec<_>>(),
        EXPECTED_R1Y_DECLARATION_CONFLICTS,
        "Rust eval declaration preflight or global flags drifted",
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP eval declaration-conflict differential: set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    assert_eq!(
        rust,
        oracle_value(&oracle, R1Y_DECLARATION_CONFLICT_PROBE),
        "eval declaration conflicts differed from pinned QuickJS",
    );
}

#[test]
fn eval_labelled_functions_keep_lexical_and_annex_b_closures_distinct() {
    let rust = rust_value(R1Y_LABELLED_FUNCTION_PROBE);
    assert_eq!(
        rust.lines().collect::<Vec<_>>(),
        EXPECTED_R1Y_LABELLED_FUNCTIONS,
        "Rust eval labelled-function environments drifted",
    );

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!(
            "SKIP eval labelled-function differential: set QJS_ORACLE to pinned upstream qjs"
        );
        return;
    };
    assert_eq!(
        rust,
        oracle_value(&oracle, R1Y_LABELLED_FUNCTION_PROBE),
        "eval labelled-function environments differed from pinned QuickJS",
    );
}

#[test]
fn foreign_realm_eval_callable_preserves_non_string_identity() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let eval = global_eval(&runtime, &mut defining);

    assert_eq!(
        caller.call(&eval, Value::Undefined, &[]).unwrap(),
        Value::Undefined
    );

    let ordinary = caller.new_object().unwrap();
    let caller_global = caller.global_object().unwrap();
    let Value::Object(returned) = caller
        .call(
            &eval,
            Value::Object(caller_global),
            &[Value::Object(ordinary.clone())],
        )
        .unwrap()
    else {
        panic!("foreign eval did not return the ordinary object");
    };
    assert_eq!(returned, ordinary);

    let Value::Object(boxed_string) = caller.eval("new String('40 + 2')").unwrap() else {
        panic!("String construction did not return an object");
    };
    let Value::Object(returned) = caller
        .call(&eval, Value::Null, &[Value::Object(boxed_string.clone())])
        .unwrap()
    else {
        panic!("foreign eval did not return the String object");
    };
    assert_eq!(returned, boxed_string);

    let symbol = runtime
        .new_symbol(Some(JsString::try_from_utf8("foreign eval").unwrap()))
        .unwrap();
    let Value::Symbol(returned) = caller
        .call(&eval, Value::Bool(true), &[Value::Symbol(symbol.clone())])
        .unwrap()
    else {
        panic!("foreign eval did not return the Symbol");
    };
    assert_eq!(returned, symbol);
}

#[test]
fn syntactic_eval_replacements_take_the_complete_ordinary_call_path() {
    for (source, expected) in [
        (
            r#"
                eval = function replacement(source, extra) {
                    return source + ":" + extra + ":" + (this === globalThis);
                };
                eval("x", 7)
            "#,
            string_value("x:7:true"),
        ),
        (
            r#"
                (function (eval) {
                    return eval("x", 7);
                })(function replacement(source, extra) {
                    return source + ":" + extra + ":" + (this === globalThis);
                })
            "#,
            string_value("x:7:true"),
        ),
        (
            r#"
                eval = 1;
                try { eval("x"); "none"; } catch (error) { error.name; }
            "#,
            string_value("TypeError"),
        ),
        (
            r#"
                var trace = "";
                eval = function replacement(first, second) {
                    return trace + "|" + first + "|" + second;
                };
                eval((trace += "A", "x"), (trace += "B", 7))
            "#,
            string_value("AB|x|7"),
        ),
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(context.eval(source).unwrap(), expected, "source: {source}");
    }

    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context
            .eval(
                r#"
                    var trace = "";
                    var result = eval(42, (trace = "B"));
                    result + "|" + trace
                "#
            )
            .unwrap(),
        string_value("42|B"),
        "original direct eval must evaluate every argument but consume only the first"
    );
}

#[test]
fn primitive_string_eval_executes_indirect_and_direct_completion_values() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let eval = global_eval(&runtime, &mut context);

    assert_eq!(
        context
            .call(
                &eval,
                Value::Undefined,
                &[Value::String(JsString::try_from_utf8("40 + 2").unwrap())],
            )
            .unwrap(),
        Value::Int(42),
        "host Context::call did not execute primitive String eval",
    );
    assert_eq!(
        context.eval(r#"(0, eval)("40 + 2")"#).unwrap(),
        Value::Int(42)
    );
    assert_eq!(
        context.eval(r#"eval("(0, eval)('40 + 2')")"#).unwrap(),
        Value::Int(42),
        "eval source incorrectly rejected a nested indirect eval",
    );

    assert_eq!(
        context
            .eval(
                r#"
                    (function (argument) {
                        let local = 1;
                        var completion = eval("local += argument; local");
                        return completion + ":" + local;
                    })(41)
                "#,
            )
            .unwrap(),
        string_value("42:42"),
        "direct eval did not read and update the caller's live argument/local slots",
    );

    assert_eq!(
        context
            .eval(
                r#"
                    (function (argument) {
                        "use strict";
                        let local = 1;
                        return eval("local += argument; local");
                    })(41)
                "#,
            )
            .unwrap(),
        Value::Int(42),
        "direct eval did not inherit strict caller bindings",
    );
    assert_eq!(
        context
            .eval(r#"(function named() { return eval("named") === named; })()"#)
            .unwrap(),
        Value::Bool(true),
        "direct eval did not import the caller's private function-name binding",
    );
    assert_eq!(
        context
            .eval(
                r#"
                    (function named() {
                        return eval("(function () { return named.name; })");
                    })()()
                "#,
            )
            .unwrap(),
        string_value("named"),
        "eval child closure did not relay the caller's function-name binding",
    );
}

#[test]
fn eval_lexicals_are_ephemeral_but_returned_closures_retain_them() {
    for (description, source) in [
        (
            "direct",
            r#"
                (function () {
                    var closure = eval(
                        "let answer = 40; const increment = 2; " +
                        "(function () { return answer + increment; })"
                    );
                    return [closure(), typeof answer, typeof increment].join("|");
                })()
            "#,
        ),
        (
            "indirect",
            r#"
                var closure = (0, eval)(
                    "let answer = 40; const increment = 2; " +
                    "(function () { return answer + increment; })"
                );
                [closure(), typeof answer, typeof increment].join("|")
            "#,
        ),
    ] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context.eval(source).unwrap(),
            string_value("42|undefined|undefined"),
            "{description} eval lexical lifetime drifted",
        );
    }
}

#[test]
fn returned_eval_closure_retains_caller_lexical_in_every_debug_mode() {
    for debug_info in [
        DebugInfoMode::Full,
        DebugInfoMode::StripSource,
        DebugInfoMode::StripDebug,
    ] {
        let runtime = Runtime::new();
        runtime.set_debug_info_mode(debug_info);
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
                        (function () {
                            let answer = 42;
                            return eval("(function () { return answer; })");
                        })()()
                    "#,
                )
                .unwrap(),
            Value::Int(42),
            "eval external relay drifted in {debug_info:?}",
        );
    }
}

#[test]
fn eval_syntax_errors_are_catchable_and_direct_eval_inherits_strictness() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    assert_eq!(
        context
            .eval(
                r#"
                    try { (0, eval)("1 +"); "none"; }
                    catch (error) { error.name; }
                "#,
            )
            .unwrap(),
        string_value("SyntaxError"),
        "indirect eval parse errors must be JavaScript exceptions",
    );
    assert_eq!(
        context
            .eval(
                r#"
                    (function () {
                        "use strict";
                        try { eval("010"); return "none"; }
                        catch (error) { return error.name; }
                    })()
                "#,
            )
            .unwrap(),
        string_value("SyntaxError"),
        "direct eval did not inherit the caller's strict parse goal",
    );
    assert_eq!(
        context
            .eval(r#"(function () { return eval("010"); })()"#)
            .unwrap(),
        Value::Int(8),
        "sloppy direct eval unexpectedly inherited strict mode",
    );
    assert_eq!(
        context
            .eval(
                r#"
                    (function () {
                        "use strict";
                        try { eval("strictEvalLeak = 1"); return "none"; }
                        catch (error) {
                            return error.name + "|" + typeof strictEvalLeak;
                        }
                    })()
                "#,
            )
            .unwrap(),
        string_value("ReferenceError|undefined"),
        "direct eval lost inherited strict assignment semantics",
    );
}

#[test]
fn nested_direct_eval_stays_a_typed_frontier() {
    let source = r#"(function () { return eval("eval('40 + 2')"); })()"#;
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_unsupported(
        context.eval(source),
        source,
        "direct eval nested inside eval source is not implemented yet",
    );
    assert!(
        !context.has_exception(),
        "typed Unsupported leaked into the JavaScript exception slot: {source}",
    );
    assert_eq!(context.take_exception().unwrap(), None, "{source}");
}

#[test]
fn foreign_realm_primitive_string_eval_uses_its_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    defining.eval("globalThis.evalRealmMarker = 42").unwrap();
    caller.eval("globalThis.evalRealmMarker = 7").unwrap();

    let defining_array_prototype = eval_object(&mut defining, "Array.prototype");
    let caller_array_prototype = eval_object(&mut caller, "Array.prototype");
    let defining_syntax_error_prototype = eval_object(&mut defining, "SyntaxError.prototype");
    let eval = global_eval(&runtime, &mut defining);

    assert_eq!(
        caller
            .call(
                &eval,
                Value::Undefined,
                &[Value::String(
                    JsString::try_from_utf8("evalRealmMarker").unwrap(),
                )],
            )
            .unwrap(),
        Value::Int(42),
        "indirect eval resolved the caller realm instead of its defining realm",
    );
    let Value::Object(array) = caller
        .call(
            &eval,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("[]").unwrap())],
        )
        .unwrap()
    else {
        panic!("foreign primitive String eval did not return an Array object");
    };
    assert_eq!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(defining_array_prototype),
        "indirect eval allocated its result outside the defining realm",
    );
    assert_ne!(
        runtime.get_prototype_of(&array).unwrap(),
        Some(caller_array_prototype),
    );

    assert!(matches!(
        caller.call(
            &eval,
            Value::Undefined,
            &[Value::String(JsString::try_from_utf8("1 +").unwrap())],
        ),
        Err(RuntimeError::Exception),
    ));
    let Value::Object(error) = caller.take_exception().unwrap().unwrap() else {
        panic!("foreign eval SyntaxError was not an object");
    };
    assert_eq!(
        runtime.get_prototype_of(&error).unwrap(),
        Some(defining_syntax_error_prototype),
        "eval SyntaxError was allocated in the caller realm",
    );
}

fn rust_observations() -> Vec<String> {
    rust_value(ORACLE_PROBE)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn rust_value(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(value) = context.eval(source).unwrap() else {
        panic!("eval oracle probe did not return a String");
    };
    value.to_utf8_lossy()
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    oracle_value(oracle, ORACLE_PROBE)
        .lines()
        .map(str::to_owned)
        .collect()
}

fn oracle_value(oracle: &OsStr, source: &str) -> String {
    let wrapper = "print(std.evalScript(scriptArgs[0]));";
    let output = Command::new(oracle)
        .args(["--std", "-e", wrapper, source])
        .output()
        .expect("run pinned QuickJS eval intrinsic oracle");
    assert!(
        output.status.success(),
        "pinned QuickJS eval intrinsic oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("pinned QuickJS eval intrinsic oracle emitted non-UTF-8 output")
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

fn global_eval(runtime: &Runtime, context: &mut Context) -> CallableRef {
    let global = context.global_object().unwrap();
    let key = runtime.intern_property_key("eval").unwrap();
    let Value::Object(function) = context.get_property(&global, &key).unwrap() else {
        panic!("global eval was not an object");
    };
    runtime
        .as_callable(&function)
        .unwrap()
        .expect("global eval was not callable")
}

fn string_value(value: &str) -> Value {
    Value::String(JsString::try_from_utf8(value).unwrap())
}

fn eval_object(context: &mut Context, source: &str) -> quickjs_oxide::ObjectRef {
    let Value::Object(object) = context.eval(source).unwrap() else {
        panic!("{source} did not evaluate to an object");
    };
    object
}

fn assert_unsupported(result: Result<Value, RuntimeError>, boundary: &str, expected_message: &str) {
    let Err(RuntimeError::Engine(error)) = result else {
        panic!("eval frontier was not an engine error at {boundary}: {result:?}");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported, "{boundary}");
    assert_eq!(error.message(), expected_message, "{boundary}");
}
