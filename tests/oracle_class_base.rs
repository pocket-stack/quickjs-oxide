use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{ErrorKind, Runtime, Value};

// Pins the base-class portion of QuickJS 2026-06-04 `js_parse_class` and
// `OP_define_class`. Heritage is covered by the derived-class oracle and gate;
// fields, private elements, static blocks, and generator/async methods remain
// later feature slices.
const PROBE: &str = r#"
(function () {
    var out = [];
    function add(name, value) { out.push(name + "=" + value); }

    var defaults = 0;
    class C {
        constructor(value = (defaults++, 41)) { this.value = value; }
        get next() { return this.value + 1; }
        set next(value) { this.value = value - 1; }
        read() { return this.value; }
        static answer() { return 42; }
    }
    var callError;
    try { C(); }
    catch (error) { callError = error.name + ":" + error.message; }
    var instance = new C();
    instance.next = 43;
    add("constructor", [callError, defaults, instance.read(), instance.next,
        C.answer(), C.name, C.length, instance instanceof C].join("|"));

    var lateDefaults = 0;
    class LateDefault {
        constructor(first, second = (lateDefaults++, 42), third) {
            this.value = second;
        }
    }
    class RestConstructor {
        constructor(...values) { this.count = values.length; }
    }
    var lateCallError;
    try { LateDefault(1); }
    catch (error) { lateCallError = error.name; }
    var lateDefaultsAfterCall = lateDefaults;
    var lateInstance = new LateDefault(1);
    add("parameters", [lateCallError, lateDefaultsAfterCall, lateDefaults,
        LateDefault.length, lateInstance.value, RestConstructor.length,
        new RestConstructor(1, 2, 3).count].join("|"));

    class StringConstructor {
        "constructor"() { this.value = 42; }
    }
    class ComputedConstructor {
        ["constructor"]() { return 43; }
    }
    add("constructor-names", [new StringConstructor().value,
        new ComputedConstructor() instanceof ComputedConstructor,
        ComputedConstructor.prototype.constructor()].join("|"));

    var ctorPrototype = Object.getOwnPropertyDescriptor(C, "prototype");
    var prototypeCtor = Object.getOwnPropertyDescriptor(C.prototype, "constructor");
    var method = Object.getOwnPropertyDescriptor(C.prototype, "read");
    var accessor = Object.getOwnPropertyDescriptor(C.prototype, "next");
    add("descriptors", [ctorPrototype.writable, ctorPrototype.enumerable,
        ctorPrototype.configurable, prototypeCtor.writable,
        prototypeCtor.enumerable, prototypeCtor.configurable,
        method.writable, method.enumerable, method.configurable,
        accessor.enumerable, accessor.configurable].join("|"));
    add("method-names", [C.prototype.read.name, C.answer.name,
        accessor.get.name, accessor.set.name].join("|"));

    var order = "";
    function key(name) { order += name; return name; }
    class Computed {
        [key("i")]() { return 20; }
        static [key("s")]() { return 22; }
        get [key("g")]() { return 42; }
    }
    add("computed", [order, new Computed().i() + Computed.s(),
        Object.getOwnPropertyDescriptor(Computed.prototype, "g").get.name].join("|"));

    Object.prototype.baseClassValue = 20;
    Function.prototype.baseStaticValue = 22;
    class Home {
        read() { return super.baseClassValue + 22; }
        static read() { return super.baseStaticValue + 20; }
    }
    class ConstructorHome {
        constructor() { this.answer = super.baseClassValue + 22; }
    }
    add("super", [new Home().read(), Home.read(),
        new ConstructorHome().answer].join("|"));
    delete Object.prototype.baseClassValue;
    delete Function.prototype.baseStaticValue;

    var Anonymous = class {};
    var Named = class Inner {};
    add("names", [Anonymous.name, Named.name, typeof Inner].join("|"));
    add("special-names", (function () {
        class eval {}
        var NamedArguments = class arguments {};
        return [eval.name, NamedArguments.name].join("|");
    })());

    var Captured = class InnerCapture { self() { return InnerCapture; } };
    var capturedInstance = new Captured();
    var originalCaptured = Captured;
    Captured = null;
    var methodStrictError;
    class StrictMethod { run() { classMethodLeak = 1; } }
    try { new StrictMethod().run(); }
    catch (error) { methodStrictError = error.name; }
    var methodConstructError;
    try { new C.prototype.read(); }
    catch (error) { methodConstructError = error.name; }
    var computedPrototypeError;
    try { (class { static ["prototype"]() {} }); }
    catch (error) { computedPrototypeError = error.name; }
    add("methods", [capturedInstance.self() === originalCaptured,
        methodStrictError, typeof classMethodLeak, methodConstructError,
        computedPrototypeError].join("|"));
    add("source", [String(C).indexOf("class C {") === 0,
        String(C.prototype.read).indexOf("read()") === 0].join("|"));

    var declarationTdz;
    try { Before; }
    catch (error) { declarationTdz = error.name; }
    class Before {}
    var computedTdz;
    try { class During { [During]() {} } }
    catch (error) { computedTdz = error.name; }
    var computedAssignment;
    try { (class { [classBaseLeak = 1]() {} }); }
    catch (error) { computedAssignment = error.name; }
    var strictOuterError;
    try {
        (function () {
            "use strict";
            return class { [strictOuterClassLeak = 1]() {} };
        })();
    }
    catch (error) { strictOuterError = error.name; }
    var evalClassTdz;
    try { class EvalClass { [eval("EvalClass")]() {} } }
    catch (error) { evalClassTdz = error.name; }
    class SloppyEvalKey {
        [eval("with ({ value: 'eval' }) value")]() { return 42; }
    }
    var recovered;
    try { (class { [(function () { throw 1; })()]() {} }); }
    catch (_) { recoveredAfterClass = 42; recovered = recoveredAfterClass; }
    add("scope", [declarationTdz, computedTdz, computedAssignment,
        typeof classBaseLeak, strictOuterError, typeof strictOuterClassLeak,
        evalClassTdz, new SloppyEvalKey().eval(), recovered].join("|"));
    delete classBaseLeak;
    delete recoveredAfterClass;

    class PrimitiveReturn { constructor() { return 1; } }
    class ObjectReturn { constructor() { return { answer: 42 }; } }
    add("returns", [(new PrimitiveReturn()) instanceof PrimitiveReturn,
        new ObjectReturn().answer].join("|"));

    return out.join("\n");
})()
"#;

const EXPECTED: &str = concat!(
    "constructor=TypeError:class constructors must be invoked with 'new'|1|42|43|42|C|0|true\n",
    "parameters=TypeError|0|1|1|42|0|3\n",
    "constructor-names=42|true|43\n",
    "descriptors=false|false|false|true|false|true|true|false|true|false|true\n",
    "method-names=read|answer|get next|set next\n",
    "computed=isg|42|get g\n",
    "super=42|42|42\n",
    "names=Anonymous|Inner|undefined\n",
    "special-names=eval|arguments\n",
    "methods=true|ReferenceError|undefined|TypeError|TypeError\n",
    "source=true|true\n",
    "scope=ReferenceError|ReferenceError||number|ReferenceError|undefined|ReferenceError|42|42\n",
    "returns=true|42",
);

#[test]
fn base_class_observation_is_stable() {
    assert_eq!(rust_observation(), EXPECTED);
}

#[test]
fn base_class_observation_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP base-class differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert_eq!(rust_observation(), oracle_observation(&oracle));
}

#[test]
fn unsupported_class_families_remain_typed_frontiers() {
    for (source, expected) in [
        ("class C { field = 1; }", "class fields"),
        ("class C { #field; }", "private class elements"),
        ("class C { static {} }", "class static blocks"),
        ("class C { *method() {} }", "class generator methods"),
        ("class C { async method() {} }", "async class methods"),
        ("class C { get\nmethod() {} }", "class fields"),
    ] {
        let error = quickjs_oxide::compiler::compile_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Unsupported, "{source}");
        assert!(error.message().contains(expected), "{source}: {error}");
    }
}

#[test]
fn base_class_early_errors_are_rejected_during_compilation() {
    for (source, expected) in [
        (
            "class C { constructor() {} 'constructor'() {} }",
            "constructor appears more than once",
        ),
        ("class C { get constructor() {} }", "invalid method name"),
        (
            "class C { set 'constructor'(value) {} }",
            "invalid method name",
        ),
        ("class C { static prototype() {} }", "invalid method name"),
        ("class {}", "class statement requires a name"),
        (
            "if (true) class C {}",
            "class declarations can't appear in single-statement context",
        ),
    ] {
        let error = quickjs_oxide::compiler::compile_script(source).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::Syntax, "{source}");
        assert!(error.message().contains(expected), "{source}: {error}");
    }
}

#[test]
fn class_expression_restores_outer_lexing_and_asi_trivia() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(name) = context
        .eval("var AsiClass = class AsiClass {}\nAsiClass.name")
        .expect("class expression followed by ASI should compile and run")
    else {
        panic!("class ASI probe did not return a String");
    };
    assert_eq!(name.to_utf8_lossy(), "AsiClass");
}

fn rust_observation() -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let diagnostic_probe = [
        "try { ",
        PROBE,
        " } catch (error) { 'UNEXPECTED=' + error.name + ':' + error.message + '\\n' + error.stack }",
    ]
    .concat();
    let Value::String(value) = context
        .eval(&diagnostic_probe)
        .expect("Rust base-class probe failed")
    else {
        panic!("Rust base-class probe did not return a String");
    };
    value.to_utf8_lossy()
}

fn oracle_observation(oracle: &OsStr) -> String {
    let source = format!("print({PROBE});");
    let output = Command::new(oracle)
        .arg("--script")
        .arg("-e")
        .arg(source)
        .output()
        .expect("failed to execute pinned qjs");
    assert!(
        output.status.success(),
        "pinned qjs base-class probe failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("pinned qjs emitted non-UTF-8 output")
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}
