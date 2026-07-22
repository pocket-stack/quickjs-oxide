use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{CompileOptions, Runtime, RuntimeError, Value};

struct RuntimeCase {
    description: &'static str,
    source: &'static str,
    expected: &'static str,
}

// These vectors pin the class public-initialization boundaries observed from
// QuickJS 2026-06-04.  Test262 covers the broad language surface; this oracle
// keeps the implementation-sensitive ordering and parser-context edges small
// enough to diagnose directly.
const RUNTIME_CASES: &[RuntimeCase] = &[
    RuntimeCase {
        description: "computed keys, element ordering, and field descriptors",
        source: r#"
(function () {
    var order = [];
    var counts = {};
    function key(name) {
        counts[name] = (counts[name] || 0) + 1;
        order.push("key:" + name);
        return name;
    }

    class Ordered {
        [key("i1")] = (order.push("instance:i1"), 11);
        static [key("s1")] = (order.push("static:s1"), 21);
        static { order.push("block:1"); }
        [key("i2")] = (order.push("instance:i2"), 12);
        static [key("s2")] = (order.push("static:s2"), 22);
        static { order.push("block:2"); }
        constructor() { order.push("body"); }
    }

    var afterDefinition = order.join(",");
    var first = new Ordered();
    var second = new Ordered();

    var setterCalls = 0;
    class DescriptorBase {}
    Object.defineProperty(DescriptorBase.prototype, "value", {
        configurable: true,
        set: function () { setterCalls++; }
    });
    Object.defineProperty(DescriptorBase, "staticValue", {
        configurable: true,
        set: function () { setterCalls++; }
    });
    class DescriptorDerived extends DescriptorBase {
        value = 41;
        static staticValue = 42;
    }
    var descriptorInstance = new DescriptorDerived();
    var instanceDescriptor = Object.getOwnPropertyDescriptor(descriptorInstance, "value");
    var staticDescriptor = Object.getOwnPropertyDescriptor(DescriptorDerived, "staticValue");

    return [
        afterDefinition,
        order.join(","),
        [counts.i1, counts.s1, counts.i2, counts.s2].join(","),
        [first.i1, first.i2, second.i1, second.i2, Ordered.s1, Ordered.s2].join(","),
        setterCalls,
        [instanceDescriptor.value, instanceDescriptor.writable,
            instanceDescriptor.enumerable, instanceDescriptor.configurable].join(","),
        [staticDescriptor.value, staticDescriptor.writable,
            staticDescriptor.enumerable, staticDescriptor.configurable].join(",")
    ].join("|");
})()
"#,
        expected: concat!(
            "key:i1,key:s1,key:i2,key:s2,static:s1,block:1,static:s2,block:2|",
            "key:i1,key:s1,key:i2,key:s2,static:s1,block:1,static:s2,block:2,",
            "instance:i1,instance:i2,body,instance:i1,instance:i2,body|",
            "1,1,1,1|11,12,11,12,21,22|0|",
            "41,true,true,true|42,true,true,true",
        ),
    },
    RuntimeCase {
        description: "base, derived, and early-installed instance initializer timing",
        source: r#"
(function () {
    var baseOrder = [];
    class BaseTiming {
        field = (baseOrder.push("field"), 42);
        constructor(value = (baseOrder.push("parameter"), 1)) {
            baseOrder.push("body");
            this.value = value;
        }
    }
    var base = new BaseTiming();

    var replacement = { kind: "replacement" };
    var derivedOrder = [];
    class ReplacementBase {
        constructor() {
            derivedOrder.push("super");
            return replacement;
        }
    }
    class ReplacementDerived extends ReplacementBase {
        field = (derivedOrder.push("field"), 42);
        constructor() {
            derivedOrder.push("before");
            super();
            derivedOrder.push("after");
        }
    }
    var derived = new ReplacementDerived();

    var installOrder = [];
    class InstalledBeforeStatic {
        field = (installOrder.push("instance"), 42);
        static made = (installOrder.push("static"), new this());
        static { installOrder.push("block:" + this.made.field); }
    }

    return [
        baseOrder.join(","),
        base.field,
        derivedOrder.join(","),
        derived === replacement,
        derived.field,
        installOrder.join(",")
    ].join("|");
})()
"#,
        expected: concat!(
            "field,parameter,body|42|before,super,field,after|true|42|",
            "static,instance,block:42",
        ),
    },
    RuntimeCase {
        description: "anonymous class field names exist before nested static initialization",
        source: r#"
(function () {
    class Outer {
        fixed = class { static seen = this.name; };
        ["computed"] = class { static seen = this.name; };
        static sfixed = class { static seen = this.name; };
        static ["scomputed"] = class { static seen = this.name; };
    }
    var instance = new Outer();
    return [
        instance.fixed.name, instance.fixed.seen,
        instance.computed.name, instance.computed.seen,
        Outer.sfixed.name, Outer.sfixed.seen,
        Outer.scomputed.name, Outer.scomputed.seen
    ].join("|");
})()
"#,
        expected: "fixed|fixed|computed|computed|sfixed|sfixed|scomputed|scomputed",
    },
    RuntimeCase {
        description: "computed constructor and prototype field names remain grammar-valid",
        source: r#"
(function () {
    function errorName(source) {
        try { eval(source); return "none"; }
        catch (error) { return error.name; }
    }
    class ComputedNames {
        ["constructor"] = 11;
        ["prototype"] = 12;
        static ["constructor"] = 21;
    }
    var instance = new ComputedNames();
    return [
        instance.constructor,
        instance.prototype,
        ComputedNames.constructor,
        errorName("(class { static ['prototype'] = 22; })")
    ].join("|");
})()
"#,
        // The computed static `prototype` passes parsing, then DefineField
        // encounters the class constructor's non-configurable own property.
        expected: "11|12|21|TypeError",
    },
    RuntimeCase {
        description: "arguments context boundaries and QuickJS shorthand capture",
        source: r#"
(function () {
    function errorName(source) {
        try { eval(source); return "none"; }
        catch (error) { return error.name; }
    }

    var directEvalErrors = [
        errorName("new (class { field = eval('arguments'); })"),
        errorName("(class { static field = eval('arguments'); })"),
        errorName("(class { static { eval('arguments'); } })")
    ].join(",");

    class OrdinaryBoundary {
        field = (function () { return arguments.length; })(1, 2);
        static staticField = (function () { return arguments[0]; })(41);
        static {
            this.block = (function () { return arguments[0]; })(42);
        }
    }
    var ordinary = new OrdinaryBoundary();

    var shorthand = (function () {
        class CapturesOuterArguments {
            field = ({arguments});
            static staticField = ({arguments});
            static { this.block = ({arguments}); }
            evalField = eval("({arguments})");
        }
        var instance = new CapturesOuterArguments();
        return [
            instance.field.arguments[0],
            CapturesOuterArguments.staticField.arguments[0],
            CapturesOuterArguments.block.arguments[0],
            instance.evalField.arguments[0]
        ].join(",");
    })(43);

    var bindings = (function () {
        class ArgumentsBindings {
            field = (class arguments {});
            static staticField = (class arguments {});
            static { class arguments {} this.blockAccepted = true; }
        }
        var instance = new ArgumentsBindings();
        class EvalBinding {
            field = eval("(class arguments {})");
        }
        return [
            instance.field.name,
            ArgumentsBindings.staticField.name,
            ArgumentsBindings.blockAccepted,
            new EvalBinding().field.name
        ].join(",");
    })();

    return [
        directEvalErrors,
        [ordinary.field, OrdinaryBoundary.staticField, OrdinaryBoundary.block].join(","),
        shorthand,
        bindings
    ].join("|");
})()
"#,
        expected: concat!(
            "SyntaxError,SyntaxError,SyntaxError|2,41,42|",
            "43,43,43,43|arguments,arguments,true,arguments",
        ),
    },
    RuntimeCase {
        description: "static-block await restrictions stop at arrow bodies and functions",
        source: r#"
(function () {
    function errorName(source) {
        try { eval(source); return "none"; }
        catch (error) { return error.name; }
    }
    var directEvalError = errorName("(class { static { eval('await'); } })");

    class AwaitBoundaries {
        static {
            this.concise = () => await;
            this.block = () => { let await = 40; return await + 2; };
            this.ordinary = function (await) { return await; };
            this.nestedArrow = function () { return ((await) => await)(42); };
        }
    }
    return [
        typeof AwaitBoundaries.concise,
        AwaitBoundaries.block(),
        AwaitBoundaries.ordinary(42),
        AwaitBoundaries.nestedArrow(),
        directEvalError
    ].join("|");
})()
"#,
        expected: "function|42|42|42|ReferenceError",
    },
    RuntimeCase {
        description: "NamedEvaluation survives unresolved forward control-flow edges",
        source: r#"
(function () {
    var out = [];
    if (true) {
        let C = class { static seen = this.name; };
        out.push(C.name + ":" + C.seen);
    }
    try {
        let C = class { static seen = this.name; };
        out.push(C.name + ":" + C.seen);
    } catch (error) {
        out.push(error.name);
    }

    function direct(C = class { static seen = this.name; }) {
        return C.name + ":" + C.seen;
    }
    function array([C = class { static seen = this.name; }] = []) {
        return C.name + ":" + C.seen;
    }
    function object({C = class { static seen = this.name; }} = {}) {
        return C.name + ":" + C.seen;
    }
    out.push(direct(), array(), object());

    var C = "old";
    var caught;
    try {
        C = class { static { throw 42; } };
    } catch (error) {
        caught = error;
    }
    out.push(C + ":" + caught);
    return out.join("|");
})()
"#,
        expected: "C:C|C:C|C:C|C:C|C:C|old:42",
    },
    RuntimeCase {
        description: "authored loop backedges rebuild fresh class initialization state",
        source: r#"
(function () {
    var out = [];
    for (var index = 0; index < 2; index++) {
        class Fresh {
            field = (out.push("instance:" + index), index);
            static value = (out.push("static:" + index), index);
            static { out.push("block:" + this.value); }
        }
        out.push("value:" + new Fresh().field);
    }
    return out.join(",");
})()
"#,
        expected: concat!(
            "static:0,block:0,instance:0,value:0,",
            "static:1,block:1,instance:1,value:1",
        ),
    },
];

const SYNTAX_ERROR_CASES: &[(&str, &str)] = &[
    (
        "fixed instance constructor field",
        "class C { constructor; }",
    ),
    (
        "fixed quoted instance constructor field",
        "class C { 'constructor'; }",
    ),
    (
        "fixed static constructor field",
        "class C { static constructor; }",
    ),
    (
        "fixed quoted static constructor field",
        "class C { static 'constructor'; }",
    ),
    ("fixed instance prototype field", "class C { prototype; }"),
    (
        "fixed quoted instance prototype field",
        "class C { 'prototype'; }",
    ),
    (
        "fixed static prototype field",
        "class C { static prototype; }",
    ),
    (
        "fixed quoted static prototype field",
        "class C { static 'prototype'; }",
    ),
    (
        "instance field arguments reference",
        "class C { field = arguments; }",
    ),
    (
        "static field arguments reference",
        "class C { static field = arguments; }",
    ),
    (
        "static block arguments reference",
        "class C { static { arguments; } }",
    ),
    (
        "instance field arrow inherits arguments restriction",
        "class C { field = () => arguments; }",
    ),
    (
        "static field arrow inherits arguments restriction",
        "class C { static field = () => arguments; }",
    ),
    (
        "static block arrow inherits arguments restriction",
        "class C { static { (() => arguments); } }",
    ),
    (
        "class arguments binding does not legalize an authored reference",
        "class C { static { class arguments {} arguments; } }",
    ),
    (
        "static block await reference",
        "class C { static { await; } }",
    ),
    (
        "static block lexical await binding",
        "class C { static { let await; } }",
    ),
    (
        "static block var await binding",
        "class C { static { var await; } }",
    ),
    (
        "static block function await binding",
        "class C { static { function await() {} } }",
    ),
    (
        "static block class await binding",
        "class C { static { class await {} } }",
    ),
    (
        "static block arrow default await reference",
        "class C { static { ((value = await) => 0); } }",
    ),
    (
        "static block arrow await parameter",
        "class C { static { ((await) => 0); } }",
    ),
    (
        "static block arrow object-pattern await parameter",
        "class C { static { (({await}) => 0); } }",
    ),
    (
        "static block arrow array-pattern await parameter",
        "class C { static { (([await]) => 0); } }",
    ),
    (
        "static block arrow rest await parameter",
        "class C { static { ((...await) => 0); } }",
    ),
];

#[test]
fn class_public_initialization_runtime_vectors_are_stable() {
    for case in RUNTIME_CASES {
        assert_eq!(
            rust_string_observation(case.source, case.description),
            case.expected,
            "Rust class-public-init observation drifted for {}",
            case.description,
        );
    }
}

#[test]
fn class_public_initialization_early_errors_are_real_syntax_errors() {
    for &(description, source) in SYNTAX_ERROR_CASES {
        assert_compile_syntax_error(source, description);
    }
}

#[test]
fn class_public_initialization_vectors_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP class-public-init differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };

    for case in RUNTIME_CASES {
        let quickjs = oracle_observation(&oracle, case.source, case.description);
        assert_eq!(
            quickjs, case.expected,
            "pinned QuickJS vector drifted for {}",
            case.description,
        );
        assert_eq!(
            rust_string_observation(case.source, case.description),
            quickjs,
            "class-public-init differential drifted for {}",
            case.description,
        );
    }

    for &(description, source) in SYNTAX_ERROR_CASES {
        assert_eq!(
            oracle_observation(&oracle, source, description),
            "THROW=SyntaxError",
            "pinned QuickJS early-error vector drifted for {description}",
        );
    }
}

fn rust_string_observation(source: &str, description: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context.eval(source) {
        Ok(Value::String(value)) => value.to_utf8_lossy(),
        Ok(value) => {
            panic!("class-public-init vector for {description} returned non-string value {value:?}")
        }
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .unwrap_or_else(|error| panic!("take exception for {description}: {error}"))
                .unwrap_or_else(|| panic!("missing exception for {description}"));
            panic!(
                "class-public-init vector for {description} threw {}:{}",
                exception_string_property(&runtime, &mut context, &exception, "name", description),
                exception_string_property(
                    &runtime,
                    &mut context,
                    &exception,
                    "message",
                    description,
                ),
            );
        }
        Err(error) => panic!("engine failure for {description}: {error}"),
    }
}

fn assert_compile_syntax_error(source: &str, description: &str) {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    match context
        .compile_with_options_preserving_unsupported_diagnostics(source, &CompileOptions::default())
    {
        Err(RuntimeError::Exception) => {}
        Err(error) => panic!(
            "{description} was an engine/frontier diagnostic instead of SyntaxError: {error}"
        ),
        Ok(_) => panic!("{description} unexpectedly compiled: {source:?}"),
    }

    let exception = context
        .take_exception()
        .unwrap_or_else(|error| panic!("take exception for {description}: {error}"))
        .unwrap_or_else(|| panic!("missing exception for {description}"));
    assert_eq!(
        exception_string_property(&runtime, &mut context, &exception, "name", description),
        "SyntaxError",
        "wrong early-error kind for {description}: {source:?}",
    );
}

fn exception_string_property(
    runtime: &Runtime,
    context: &mut quickjs_oxide::Context,
    exception: &Value,
    property: &str,
    description: &str,
) -> String {
    let Value::Object(error) = exception else {
        panic!("exception for {description} was not an object: {exception:?}");
    };
    let key = runtime
        .intern_property_key(property)
        .unwrap_or_else(|failure| panic!("intern Error.{property} for {description}: {failure}"));
    let Value::String(value) = context
        .get_property(error, &key)
        .unwrap_or_else(|failure| panic!("read Error.{property} for {description}: {failure}"))
    else {
        panic!("Error.{property} for {description} was not a string");
    };
    value.to_utf8_lossy()
}

fn oracle_observation(oracle: &OsStr, source: &str, description: &str) -> String {
    let wrapper = r#"
try {
  var value = std.evalScript(scriptArgs[0]);
  print(String(value));
} catch (error) {
  if (error !== null && typeof error === "object")
    print("THROW=" + error.name);
  else
    print("THROW=" + typeof error);
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
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}
