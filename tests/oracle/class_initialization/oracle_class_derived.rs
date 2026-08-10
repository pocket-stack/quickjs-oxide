use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{Runtime, Value};

// Pins the derived-class portion of QuickJS 2026-06-04 `js_parse_class`,
// `OP_define_class`, `OP_init_ctor`, and the derived-constructor return
// protocol. Keep observations structural: error names are stable semantics,
// while implementation-specific diagnostic wording is deliberately omitted.
const PROBE: &str = r#"
(function () {
    var out = [];
    function add(name, values) { out.push(name + "=" + values.join("|")); }

    var heritageLog = [];
    function Parent() {}
    var Heritage = Parent.bind(null);
    Object.defineProperty(Heritage, "prototype", {
        configurable: true,
        get: function () {
            heritageLog.push("prototype");
            return Parent.prototype;
        }
    });
    class Wired extends (heritageLog.push("heritage"), Heritage) {
        [(heritageLog.push("element"), "method")]() { return 42; }
    }
    class NullBase extends null {}
    var nullConstructError;
    try { new NullBase(); }
    catch (error) { nullConstructError = error.name; }
    add("heritage", [
        heritageLog.join(","),
        Object.getPrototypeOf(Wired) === Heritage,
        Object.getPrototypeOf(Wired.prototype) === Parent.prototype,
        Wired.prototype.constructor === Wired,
        new Wired().method(),
        Object.getPrototypeOf(NullBase) === Function.prototype,
        Object.getPrototypeOf(NullBase.prototype) === null,
        nullConstructError
    ]);

    function ForwardBase(first, second) {
        this.sum = first + second;
        this.count = arguments.length;
    }
    class ForwardDerived extends ForwardBase {}
    var iteratorCalls = 0;
    var originalArrayIterator = Array.prototype[Symbol.iterator];
    Array.prototype[Symbol.iterator] = function () {
        iteratorCalls++;
        throw new Error("default constructor iterated argv");
    };
    var forwarded;
    try { forwarded = new ForwardDerived(20, 22); }
    finally { Array.prototype[Symbol.iterator] = originalArrayIterator; }
    add("default", [forwarded.sum, forwarded.count, iteratorCalls]);

    var explicitLog = [];
    class ExplicitBase {
        constructor(first, second) {
            explicitLog.push("base");
            this.sum = first + second;
        }
    }
    class FixedDerived extends ExplicitBase {
        constructor() {
            explicitLog.push("fixed");
            super((explicitLog.push("a"), 20), (explicitLog.push("b"), 22));
            explicitLog.push("done");
        }
    }
    var fixed = new FixedDerived();
    var spreadValues = [20, 22];
    spreadValues[Symbol.iterator] = function () {
        explicitLog.push("iterator");
        return originalArrayIterator.call(this);
    };
    class SpreadDerived extends ExplicitBase {
        constructor() {
            explicitLog.push("spread");
            super(...spreadValues);
            explicitLog.push("done");
        }
    }
    var spread = new SpreadDerived();
    add("explicit", [explicitLog.join(","), fixed.sum, spread.sum]);

    var baseCalls = 0;
    class OnceBase {
        constructor() {
            baseCalls++;
            this.call = baseCalls;
        }
    }
    class OnceDerived extends OnceBase {
        constructor() {
            var before;
            try { this.call; }
            catch (error) { before = error.name; }
            super();
            var second;
            try { super(); }
            catch (error) { second = error.name; }
            this.errors = before + "," + second;
        }
    }
    var once = new OnceDerived();
    add("this", [once.errors, baseCalls, once.call]);

    class ReturnBase { constructor() { this.answer = 42; } }
    class ReturnObject extends ReturnBase { constructor() { return { answer: 42 }; } }
    class ReturnUndefined extends ReturnBase { constructor() { super(); return undefined; } }
    class ReturnMissing extends ReturnBase { constructor() {} }
    class ReturnPrimitive extends ReturnBase { constructor() { return 1; } }
    var missingError;
    var primitiveError;
    try { new ReturnMissing(); }
    catch (error) { missingError = error.name; }
    try { new ReturnPrimitive(); }
    catch (error) { primitiveError = error.name; }
    add("returns", [
        new ReturnObject().answer,
        new ReturnUndefined().answer,
        missingError,
        primitiveError
    ]);

    class TargetBase {
        constructor() { this.targetName = new.target.name; }
    }
    class TargetDerived extends TargetBase { constructor() { super(); } }
    function AlternateTarget() {}
    var ordinaryTarget = new TargetDerived();
    var reflectedTarget = Reflect.construct(TargetDerived, [], AlternateTarget);
    add("new-target", [
        ordinaryTarget.targetName,
        reflectedTarget.targetName,
        Object.getPrototypeOf(reflectedTarget) === AlternateTarget.prototype
    ]);

    var liveLog = [];
    class FirstBase { constructor() { liveLog.push("first"); } }
    class SecondBase { constructor() { liveLog.push("second"); } }
    class LiveDerived extends FirstBase {
        constructor() {
            super((Object.setPrototypeOf(LiveDerived, SecondBase), 0));
            try { super(); }
            catch (error) { this.secondError = error.name; }
        }
    }
    var live = new LiveDerived();
    add("live-super", [liveLog.join(","), live.secondError]);

    class RelayBase { constructor(value) { this.value = value; } }
    class ArrowDerived extends RelayBase {
        constructor() { (() => super(42))(); }
    }
    class EvalDerived extends RelayBase {
        constructor() { eval("super(42)"); }
    }
    class NestedEvalDerived extends RelayBase {
        constructor() { eval("eval('super(42)')"); }
    }
    class ParameterDerived extends RelayBase {
        constructor(value = super(42)) {}
    }
    add("relay", [
        new ArrowDerived().value,
        new EvalDerived().value,
        new NestedEvalDerived().value,
        new ParameterDerived().value
    ]);

    return out.join("\n");
})()
"#;

const EXPECTED: &str = concat!(
    "heritage=heritage,prototype,element|true|true|true|42|true|true|TypeError\n",
    "default=42|2|0\n",
    "explicit=fixed,a,b,base,done,spread,iterator,base,done|42|42\n",
    "this=ReferenceError,ReferenceError|2|1\n",
    "returns=42|42|ReferenceError|TypeError\n",
    "new-target=TargetDerived|AlternateTarget|true\n",
    "live-super=first,second|ReferenceError\n",
    "relay=42|42|42|42",
);

#[test]
fn derived_class_observation_is_stable() {
    assert_eq!(rust_observation(), EXPECTED);
}

#[test]
fn derived_class_observation_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP derived-class differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let rust = rust_observation();
    let quickjs = oracle_observation(&oracle);
    assert_eq!(quickjs, EXPECTED, "pinned QuickJS observation drifted");
    assert_eq!(rust, quickjs);
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
        .expect("Rust derived-class probe failed")
    else {
        panic!("Rust derived-class probe did not return a String");
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
        "pinned qjs derived-class probe failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout)
        .expect("pinned qjs emitted non-UTF-8 output")
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}
