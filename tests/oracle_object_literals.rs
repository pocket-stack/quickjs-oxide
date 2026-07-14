use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{JsString, Runtime, RuntimeError, Value};

// Pins the data/computed/proto/spread lowering in QuickJS 2026-06-04. Method
// and accessor literal syntax deliberately remains a separate feature slice.
const PROBE: &str = r#"
(function(){
  var out = [];
  function add(name, value) { out.push(name + "=" + value); }

  var value = 3;
  var fixed = {2:"two", a:1, value, if:4, "a":5};
  var desc = Object.getOwnPropertyDescriptor(fixed, "a");
  add("fixed", fixed[2] + "|" + fixed.value + "|" + fixed.a + "|" +
      desc.writable + "|" + desc.enumerable + "|" + desc.configurable + "|" +
      Object.keys(fixed).join(","));

  var log = "";
  var key = {};
  Object.defineProperty(key, Symbol.toPrimitive, {
    value: function(hint) { log += "key(" + hint + ")"; return "computed"; }
  });
  function rhs() { log += "rhs"; return 7; }
  var computed = {[key]: rhs()};
  add("computed-order", log + "|" + computed.computed);

  var computedThrowLog = "";
  var throwingKey = {};
  Object.defineProperty(throwingKey, Symbol.toPrimitive, {
    value: function(){ computedThrowLog += "key"; throw "key-error"; }
  });
  try { ({[throwingKey]:(computedThrowLog += "rhs")}); }
  catch (error) { computedThrowLog += "|" + error; }
  add("computed-throw", computedThrowLog);

  var described = Symbol("named");
  var missing = Symbol();
  var empty = Symbol("");
  var named = {
    plain: function(){},
    [described]: function(){},
    [missing]: function(){},
    [empty]: function(){}
  };
  add("names", named.plain.name + "|" + named[described].name + "|" +
      named[missing].name + "|" + named[empty].name);

  var proto = {marker:9};
  var protoObject = {__proto__: proto};
  var protoNull = {__proto__: null};
  var protoPrimitive = {__proto__: 1};
  var __proto__ = 11;
  var protoShorthand = {__proto__};
  var protoComputed = {["__proto__"]: 12};
  add("proto", protoObject.marker + "|" +
      Object.hasOwn(protoObject, "__proto__") + "|" +
      (Object.getPrototypeOf(protoNull) === null) + "|" +
      (Object.getPrototypeOf(protoPrimitive) === Object.prototype) + "|" +
      protoShorthand.__proto__ + "|" + protoComputed.__proto__);

  var symbol = Symbol("spread");
  var sourceProto = {later:"from-prototype"};
  var source = Object.create(sourceProto);
  Object.defineProperty(source, "first", {
    enumerable: true,
    configurable: true,
    get: function(){ log += "get-first"; delete source.later; return "first"; }
  });
  source.later = "own-later";
  source[symbol] = "symbol";
  Object.defineProperty(source, "hidden", {value:"hidden", enumerable:false});
  log = "";
  var spread = {before:0, ...source, after:1};
  add("spread", spread.first + "|" + spread.later + "|" + spread[symbol] + "|" +
      Object.hasOwn(spread, "hidden") + "|" + log + "|" +
      Object.keys(spread).join(","));

  var setterHits = 0;
  Object.defineProperty(Object.prototype, "throughSetter", {
    configurable: true,
    set: function(){ setterHits++; }
  });
  var definedSpread = {...{throughSetter:42}};
  delete Object.prototype.throughSetter;
  add("spread-define", setterHits + "|" + Object.hasOwn(definedSpread, "throughSetter") +
      "|" + definedSpread.throughSetter);

  var spreadThrowLog = "";
  var throwingSource = {};
  Object.defineProperty(throwingSource, "a", {
    enumerable: true,
    get: function(){ spreadThrowLog += "a"; return 1; }
  });
  Object.defineProperty(throwingSource, "b", {
    enumerable: true,
    get: function(){ spreadThrowLog += "b"; throw "spread-error"; }
  });
  Object.defineProperty(throwingSource, "c", {
    enumerable: true,
    get: function(){ spreadThrowLog += "c"; return 3; }
  });
  try { ({...throwingSource}); }
  catch (error) { spreadThrowLog += "|" + error; }
  add("spread-throw", spreadThrowLog);

  var primitiveSpread = {..."ab", ...1, ...null, ...undefined};
  add("primitive-spread", Object.keys(primitiveSpread).join(","));

  return out.join("\n");
})()
"#;

#[test]
fn object_literal_observations_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP object-literal differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    assert_eq!(rust_observation(), oracle_observation(&oracle));
}

#[test]
fn object_literal_duplicate_proto_is_an_early_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert!(matches!(
        context.compile("({__proto__:1,__proto__:2})"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("duplicate ProtoSetter did not produce an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::try_from_utf8("duplicate __proto__ property name").unwrap())
    );
}

fn rust_observation() -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(value) = context
        .eval(PROBE)
        .expect("Rust object-literal probe failed")
    else {
        panic!("Rust object-literal probe did not return a String");
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
        "pinned qjs object-literal probe failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("pinned qjs emitted non-UTF-8 output")
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

#[test]
fn probe_expected_shape_is_stable_without_an_oracle() {
    assert_eq!(
        rust_observation(),
        [
            "fixed=two|3|5|true|true|true|2,a,value,if",
            "computed-order=key(string)rhs|7",
            "computed-throw=key|key-error",
            "names=plain|[named]||[]",
            "proto=9|false|true|true|11|12",
            "spread=first|from-prototype|symbol|false|get-first|before,first,later,after",
            "spread-define=0|true|42",
            "spread-throw=ab|spread-error",
            "primitive-spread=",
        ]
        .join("\n")
    );
}

#[test]
fn object_literal_result_uses_the_defining_realm() {
    let runtime = Runtime::new();
    let mut defining = runtime.new_context();
    let mut caller = runtime.new_context();
    let function = defining
        .eval("(function(){return {marker:1}})")
        .expect("object-literal factory compilation failed");
    let Value::Object(function) = function else {
        panic!("factory was not an Object");
    };
    let callable = runtime
        .as_callable(&function)
        .expect("callability lookup failed")
        .expect("factory was not callable");
    let Value::Object(result) = caller
        .call(&callable, Value::Undefined, &[])
        .expect("cross-realm object-literal call failed")
    else {
        panic!("factory result was not an Object");
    };
    assert_eq!(
        runtime.get_prototype_of(&result).unwrap(),
        Some(defining.object_prototype().unwrap())
    );
}
