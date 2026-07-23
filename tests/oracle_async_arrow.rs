use std::ffi::OsStr;
use std::process::{Command, Output};

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

fn eval(context: &mut Context, source: &str) -> Value {
    context.eval(source).unwrap_or_else(|error| {
        if error == RuntimeError::Exception {
            panic!(
                "unexpected JavaScript exception: {:?}",
                context.take_exception()
            );
        }
        panic!("unexpected engine error: {error}");
    })
}

fn text(value: Value) -> String {
    let Value::String(value) = value else {
        panic!("expected a string");
    };
    value.to_utf8_lossy()
}

fn integer(value: Value) -> i32 {
    let Value::Int(value) = value else {
        panic!("expected an integer");
    };
    value
}

fn drain(runtime: &Runtime) -> usize {
    let mut count = 0;
    while runtime.is_job_pending() {
        assert!(runtime.execute_pending_job().unwrap());
        count += 1;
    }
    count
}

#[test]
fn async_arrow_shape_source_and_await_match_pinned_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var events = [];
var answer = 'pending';
var add = async (left, right) => {
    events.push('body');
    return await (left + right);
};
var exact = async ( /*a*/ value /*b*/ ) => /*c*/ await value;
var promise = add(19, 23);
events.push('after');
promise.then(function (value) {
    answer = value;
    events.push('then');
});
var constructError = 'none';
try {
    new add(1, 2);
} catch (error) {
    constructError = error.name;
}
[
    typeof add,
    add.name,
    add.length,
    Object.prototype.hasOwnProperty.call(add, 'prototype'),
    Object.prototype.toString.call(add),
    Object.getPrototypeOf(add).constructor.name,
    Object.getPrototypeOf(promise) === Promise.prototype,
    Function.prototype.toString.call(exact),
    constructError,
    events.join(','),
    answer
].join('|');
"#,
        )),
        concat!(
            "function|add|2|false|[object AsyncFunction]|AsyncFunction|true|",
            "async ( /*a*/ value /*b*/ ) => /*c*/ await value|",
            "TypeError|body,after|pending"
        )
    );
    assert_eq!(drain(&runtime), 2);
    assert_eq!(integer(eval(&mut context, "answer")), 42);
    assert_eq!(
        text(eval(&mut context, "events.join(',')")),
        "body,after,then"
    );
}

#[test]
fn async_arrow_keeps_lexical_this_arguments_and_new_target_across_await() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var lexicalResults = [];
function Outer(first) {
    var arrow = async second => {
        await 0;
        return [
            this.base,
            arguments[0],
            new.target === Outer,
            second
        ].join(',');
    };
    return arrow.call({ base: 1000 }, 42);
}
Outer.prototype.base = 40;
Outer.call({ base: 40 }, 1).then(function (value) {
    lexicalResults.push('call:' + value);
});
new Outer(1).then(function (value) {
    lexicalResults.push('construct:' + value);
});
"#,
    );
    assert_eq!(drain(&runtime), 4);
    assert_eq!(
        text(eval(&mut context, "lexicalResults.join('|')")),
        "call:40,1,false,42|construct:40,1,true,42"
    );
}

#[test]
fn async_arrow_keeps_lexical_super_and_receiver_across_await() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    eval(
        &mut context,
        r#"
var superResult = 'pending';
class Base {
    get answer() {
        return this.seed + 1;
    }
}
class Derived extends Base {
    constructor() {
        super();
        this.seed = 41;
    }
    read() {
        return (async () => {
            await 0;
            return super.answer;
        }).call({ seed: 1000 });
    }
}
new Derived().read().then(function (value) {
    superResult = value;
});
"#,
    );
    assert_eq!(drain(&runtime), 2);
    assert_eq!(integer(eval(&mut context, "superResult")), 42);
}

#[test]
fn async_arrow_parameter_abrupt_becomes_a_rejected_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        text(eval(
            &mut context,
            r#"
var parameterResult = 'pending';
var synchronous = 'none';
var fail = async (
    value = (() => { throw new RangeError('parameter'); })()
) => value;
try {
    var promise = fail();
} catch (error) {
    synchronous = error.name + ':' + error.message;
}
promise.then(undefined, function (error) {
    parameterResult = error.name + ':' + error.message;
});
[
    synchronous,
    Object.getPrototypeOf(promise) === Promise.prototype,
    parameterResult
].join('|');
"#,
        )),
        "none|true|pending"
    );
    assert_eq!(drain(&runtime), 1);
    assert_eq!(
        text(eval(&mut context, "parameterResult")),
        "RangeError:parameter"
    );
}

#[test]
fn async_arrow_contextual_tokens_match_pinned_quickjs() {
    const VALID: &[&str] = &[
        "var arrow = async await => 42; arrow().then(print);",
        r"var arrow = async aw\u0061it => 42; arrow().then(print);",
        r#"
function* outer() {
    return async (value = (yield) => yield) => value;
}
outer().next().value().then(function (value) {
    print(value(42));
});
"#,
        r#"
async function outer() {
    return (value = (await) => await) => value;
}
outer().then(function (arrow) {
    print(arrow()(42));
});
"#,
        r#"
class C {
    static {
        this.arrow = (value = (await) => await) => value;
    }
}
print(C.arrow()(42));
"#,
        r#"
async function outer() {
    return (value = (nested = async await => 42) => nested) => value;
}
outer().then(function (middle) {
    middle()()().then(print);
});
"#,
    ];
    const INVALID: &[&str] = &[
        "var arrow = async (await) => 1;",
        "var arrow = async (value = await 1) => value;",
        "async function outer() { return async await => 1; }",
        "function* outer() { return async yield => 1; }",
        "function* outer() { return (value = async yield => 1) => value; }",
        "async function outer() { return (value = async await => 1) => value; }",
        "class C { static { (value = async await => 1) => value; } }",
    ];

    for source in VALID {
        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
        assert!(
            oxide.status.success(),
            "quickjs-oxide rejected a pinned QuickJS async-arrow edge {source:?}: {}",
            String::from_utf8_lossy(&oxide.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&oxide.stdout), "42\n");
    }
    for source in INVALID {
        let oxide = run(env!("CARGO_BIN_EXE_qjs").as_ref(), source);
        assert!(
            !oxide.status.success(),
            "quickjs-oxide accepted an async-arrow edge rejected by pinned QuickJS: {source:?}"
        );
    }

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP async-arrow token differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    for source in VALID {
        let quickjs = run(&oracle, source);
        assert!(
            quickjs.status.success(),
            "pinned QuickJS rejected its recorded async-arrow edge {source:?}: {}",
            String::from_utf8_lossy(&quickjs.stderr)
        );
        assert_eq!(String::from_utf8_lossy(&quickjs.stdout), "42\n");
    }
    for source in INVALID {
        let quickjs = run(&oracle, source);
        assert!(
            !quickjs.status.success(),
            "pinned QuickJS accepted a recorded rejection edge {source:?}"
        );
    }
}

fn run(executable: &OsStr, source: &str) -> Output {
    Command::new(executable)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run {executable:?}: {error}"))
}
