use std::ffi::OsStr;
use std::process::Command;

use quickjs_oxide::{
    CallableRef, Context, DescriptorField, JsString, ObjectRef, OrdinaryPropertyDescriptor,
    Runtime, RuntimeError, Value,
};

const ORACLE_SETUP: &str = r#"
var savedConcat = String.prototype.concat;
var seed = "x".repeat(8193);
var near = seed;
for (var i = 0; i < 16; i++)
    near = near + near;
var ordinary = "A".repeat(8193) + "\ud83d" + "\ude80" + "z".repeat(513);
var peer = "A".repeat(8192) + ("A\ud83d\ude80" + "z".repeat(513));
var boundaryPeer = "A".repeat(8193) + "\ud83d" + "\ude81" + "z".repeat(513);
var prefixPeer = ordinary + "q";
var markers = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
var deep = "";
for (var j = 0; j < 70; j++)
    deep = deep + markers.charAt(j % markers.length).repeat(8193);
var power = "m";
var powers = [power];
for (var k = 1; k < 30; k++) {
    power = power + power;
    powers.push(power);
}
var exactMax = "";
for (var n = powers.length - 1; n >= 0; n--)
    exactMax = exactMax + powers[n];
"#;

// The 536,936,448-code-unit rope is never printed or coerced to a flat
// string. Only cached length, random indices and the overflow error cross the
// process boundary. The ordinary 8 KiB ropes cover content operations and the
// property-key linearization boundary without making the oracle memory-heavy.
const PROBE: &str = r#"
(function () {
    var caught = function (thunk) {
        try {
            thunk();
            return "ok";
        } catch (error) {
            return "throw:" + error.name + ":" + error.message;
        }
    };

    var out = "limit=" + near.length + "|" + near[0] + "|" +
              near[near.length - 1] + "|" +
              caught(function () { return near + near; });

    var equal = ordinary === peer;
    var less = ordinary < peer;
    var greater = peer < ordinary;
    var codePoint = ordinary.codePointAt(8193);
    var holder = {};
    holder[ordinary] = 17;
    out += "\nordinary=" + ordinary.length + "|" + ordinary[0] + "|" +
           ordinary[8192] + "|" + ordinary[8193].charCodeAt(0) + "|" +
           ordinary[8194].charCodeAt(0) + "|" +
           ordinary[ordinary.length - 1] + "|" + codePoint + "|" +
           equal + "|" + less + "|" + greater + "|" + holder[peer] + "|" +
           (ordinary < boundaryPeer) + "|" + (boundaryPeer < ordinary) + "|" +
           (ordinary < prefixPeer) + "|" + (prefixPeer < ordinary);

    var viaConcat = savedConcat.call("L", ordinary, "R");
    out += "\nconcat=" + viaConcat.length + "|" + viaConcat[0] + "|" +
           viaConcat[1] + "|" + viaConcat[viaConcat.length - 1] + "|" +
           viaConcat.codePointAt(8194);

    out += "\nrebalance=" + deep.length + "|" + deep[0] + "|" +
           deep[8192] + "|" + deep[8193] + "|" + deep[30 * 8193] + "|" +
           deep[59 * 8193] + "|" + deep[60 * 8193] + "|" +
           deep[61 * 8193] + "|" + deep[deep.length - 1];

    var log = "";
    var later = {
        toString: function () {
            log += "later,";
            return "later";
        }
    };
    var overflow = caught(function () {
        return savedConcat.call("", near, near, later);
    });
    out += "\nconcat-overflow=" + overflow + "|" + log + "|" + near.length;
    out += "\nmax=" + exactMax.length + "|" + (exactMax + "").length + "|" +
           caught(function () { return exactMax + "x"; });
    return out;
})()
"#;

// The live parser does not implement JavaScript try/catch yet. Rust therefore
// evaluates the successful observations together, then observes the two
// exceptions through Context's host completion API and assembles the same four
// lines as the upstream probe.
const RUST_SUCCESS_PROBE: &str = r#"
(function () {
    var out = "limit=" + near.length + "|" + near[0] + "|" +
              near[near.length - 1];

    var equal = ordinary === peer;
    var less = ordinary < peer;
    var greater = peer < ordinary;
    var codePoint = ordinary.codePointAt(8193);
    holder[ordinary] = 17;
    out += "\nordinary=" + ordinary.length + "|" + ordinary[0] + "|" +
           ordinary[8192] + "|" + ordinary[8193].charCodeAt(0) + "|" +
           ordinary[8194].charCodeAt(0) + "|" +
           ordinary[ordinary.length - 1] + "|" + codePoint + "|" +
           equal + "|" + less + "|" + greater + "|" + holder[peer] + "|" +
           (ordinary < boundaryPeer) + "|" + (boundaryPeer < ordinary) + "|" +
           (ordinary < prefixPeer) + "|" + (prefixPeer < ordinary);

    var viaConcat = savedConcat.call("L", ordinary, "R");
    out += "\nconcat=" + viaConcat.length + "|" + viaConcat[0] + "|" +
           viaConcat[1] + "|" + viaConcat[viaConcat.length - 1] + "|" +
           viaConcat.codePointAt(8194);
    out += "\nrebalance=" + deep.length + "|" + deep[0] + "|" +
           deep[8192] + "|" + deep[8193] + "|" + deep[30 * 8193] + "|" +
           deep[59 * 8193] + "|" + deep[60 * 8193] + "|" +
           deep[61 * 8193] + "|" + deep[deep.length - 1];
    return out;
})()
"#;

#[test]
fn string_rope_matches_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String rope differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let rust = rust_observations();
    let upstream = oracle_observations(&oracle);
    assert_eq!(rust.len(), 6, "Rust String rope probe breadth changed");
    assert_eq!(
        upstream.len(),
        6,
        "QuickJS String rope probe breadth changed"
    );
    assert_eq!(
        rust, upstream,
        "String rope behavior differed from pinned QuickJS"
    );
}

#[test]
fn string_rope_overflow_uses_vm_and_native_defining_realms() {
    let runtime = Runtime::new();
    let mut first = runtime.new_context();
    let mut second = runtime.new_context();
    let first_string = first.string_prototype().unwrap();
    let concat = property_callable(&runtime, &mut first, &first_string, "concat");
    let first_internal_error = intrinsic_prototype(&runtime, &mut first, "InternalError");
    let second_internal_error = intrinsic_prototype(&runtime, &mut second, "InternalError");
    assert_ne!(first_internal_error, second_internal_error);

    let near = near_limit_rope();
    let second_global = second.global_object().unwrap();
    define_data(
        &runtime,
        &second_global,
        "nearLimitString",
        Value::String(near.clone()),
    );
    assert_eq!(
        second.eval("nearLimitString + nearLimitString"),
        Err(RuntimeError::Exception)
    );
    let vm_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&vm_error).unwrap(),
        Some(second_internal_error)
    );
    assert_internal_string_too_long(&runtime, &mut second, &vm_error);

    assert_eq!(
        second.call(
            &concat,
            Value::String(JsString::try_from_utf8("").unwrap()),
            &[Value::String(near.clone()), Value::String(near)],
        ),
        Err(RuntimeError::Exception)
    );
    let native_error = take_exception_object(&mut second);
    assert_eq!(
        runtime.get_prototype_of(&native_error).unwrap(),
        Some(first_internal_error)
    );
    assert_internal_string_too_long(&runtime, &mut second, &native_error);
}

#[test]
fn string_rope_dag_does_not_keep_its_realm_graph_alive() {
    let runtime = Runtime::new();
    let rope = near_limit_rope();
    let method = {
        let mut context = runtime.new_context();
        let global = context.global_object().unwrap();
        define_data(&runtime, &global, "rootedRope", Value::String(rope.clone()));
        let prototype = context.string_prototype().unwrap();
        property_callable(&runtime, &mut context, &prototype, "concat")
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    assert_eq!(rope.len(), 536_936_448);
    assert_eq!(rope.code_unit_at(0), Some(u16::from(b'x')));
    assert_eq!(rope.code_unit_at(rope.len() - 1), Some(u16::from(b'x')));

    drop(method);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
    drop(rope);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().live, 0);
}

fn rust_observations() -> Vec<String> {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let global = context.global_object().unwrap();
    let prototype = context.string_prototype().unwrap();
    let concat = property_callable(&runtime, &mut context, &prototype, "concat");
    let (ordinary, peer, boundary_peer, prefix_peer) = ordinary_ropes();
    let near = near_limit_rope();
    let deep = rebalanced_marker_rope();
    let exact_max = exact_max_rope();

    define_data(
        &runtime,
        &global,
        "savedConcat",
        Value::Object(concat.as_object().clone()),
    );
    define_data(&runtime, &global, "near", Value::String(near.clone()));
    define_data(&runtime, &global, "ordinary", Value::String(ordinary));
    define_data(&runtime, &global, "peer", Value::String(peer));
    define_data(
        &runtime,
        &global,
        "boundaryPeer",
        Value::String(boundary_peer),
    );
    define_data(&runtime, &global, "prefixPeer", Value::String(prefix_peer));
    define_data(&runtime, &global, "deep", Value::String(deep));
    define_data(
        &runtime,
        &global,
        "exactMax",
        Value::String(exact_max.clone()),
    );
    let holder = context.new_object().unwrap();
    define_data(&runtime, &global, "holder", Value::Object(holder));

    let result = match context.eval(RUST_SUCCESS_PROBE) {
        Ok(value) => value,
        Err(RuntimeError::Exception) => {
            let error = take_exception_object(&mut context);
            let name = error_string(&runtime, &mut context, &error, "name");
            let message = error_string(&runtime, &mut context, &error, "message");
            panic!(
                "Rust String rope probe threw {}: {}",
                name.to_utf8_lossy(),
                message.to_utf8_lossy()
            );
        }
        Err(error) => panic!("Rust String rope probe failed outside JavaScript: {error}"),
    };
    let Value::String(output) = result else {
        panic!("String rope probe did not return a String");
    };
    let mut lines = output
        .to_utf8_lossy()
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    assert_eq!(lines.len(), 4, "Rust successful rope probe breadth changed");

    assert_eq!(context.eval("near + near"), Err(RuntimeError::Exception));
    let vm_overflow = take_error_observation(&runtime, &mut context);
    lines[0].push('|');
    lines[0].push_str(&vm_overflow);

    define_data(
        &runtime,
        &global,
        "ropeLog",
        Value::String(JsString::try_from_utf8("").unwrap()),
    );
    let later = context.new_object().unwrap();
    let later_to_string = context
        .eval("(function(){ropeLog=ropeLog+'later,';return 'later';})")
        .unwrap();
    define_data(&runtime, &later, "toString", later_to_string);
    assert_eq!(
        context.call(
            &concat,
            Value::String(JsString::try_from_utf8("").unwrap()),
            &[
                Value::String(near.clone()),
                Value::String(near.clone()),
                Value::Object(later),
            ],
        ),
        Err(RuntimeError::Exception)
    );
    let native_overflow = take_error_observation(&runtime, &mut context);
    let Value::String(log) = context
        .get_property(&global, &runtime.intern_property_key("ropeLog").unwrap())
        .unwrap()
    else {
        panic!("rope conversion log was not a String");
    };
    lines.push(format!(
        "concat-overflow={native_overflow}|{}|{}",
        log.to_utf8_lossy(),
        near.len()
    ));
    let Value::Int(exact_length) = context.eval("exactMax.length").unwrap() else {
        panic!("exact MAX String length was not an Int");
    };
    let Value::Int(exact_plus_empty_length) = context.eval("(exactMax + '').length").unwrap()
    else {
        panic!("exact MAX String plus empty length was not an Int");
    };
    let exact_overflow = observe_source_exception(&runtime, &mut context, "exactMax + 'x'");
    assert_eq!(usize::try_from(exact_length).unwrap(), exact_max.len());
    lines.push(format!(
        "max={exact_length}|{exact_plus_empty_length}|{exact_overflow}"
    ));
    lines
}

fn near_limit_rope() -> JsString {
    let mut value = JsString::try_from_utf8(&"x".repeat(8193)).unwrap();
    for _ in 0..16 {
        value = value.try_concat(&value).unwrap();
    }
    assert_eq!(value.len(), 536_936_448);
    value
}

fn rebalanced_marker_rope() -> JsString {
    let mut value = JsString::try_from_utf8("").unwrap();
    for index in 0..70 {
        let marker = char::from(b'A' + u8::try_from(index % 26).unwrap());
        let chunk = JsString::try_from_utf8(&marker.to_string().repeat(8193)).unwrap();
        value = value.try_concat(&chunk).unwrap();
    }
    assert_eq!(value.len(), 573_510);
    value
}

fn exact_max_rope() -> JsString {
    let mut powers = Vec::with_capacity(30);
    let mut power = JsString::try_from_utf8("m").unwrap();
    powers.push(power.clone());
    for _ in 1..30 {
        power = power.try_concat(&power).unwrap();
        powers.push(power.clone());
    }
    let mut value = JsString::try_from_utf8("").unwrap();
    for power in powers.into_iter().rev() {
        value = value.try_concat(&power).unwrap();
    }
    assert_eq!(value.len(), JsString::MAX_LEN);
    value
}

fn ordinary_ropes() -> (JsString, JsString, JsString, JsString) {
    let high = JsString::try_from_utf16([0xd83d]).unwrap();
    let low = JsString::try_from_utf16([0xde80]).unwrap();
    let z = JsString::try_from_utf8(&"z".repeat(513)).unwrap();
    let ordinary = JsString::try_from_utf8(&"A".repeat(8193))
        .unwrap()
        .try_concat(&high)
        .unwrap()
        .try_concat(&low)
        .unwrap()
        .try_concat(&z)
        .unwrap();
    let peer_tail = JsString::try_from_utf16([0x41, 0xd83d, 0xde80])
        .unwrap()
        .try_concat(&z)
        .unwrap();
    let peer = JsString::try_from_utf8(&"A".repeat(8192))
        .unwrap()
        .try_concat(&peer_tail)
        .unwrap();
    let boundary_peer = JsString::try_from_utf8(&"A".repeat(8193))
        .unwrap()
        .try_concat(&high)
        .unwrap()
        .try_concat(&JsString::try_from_utf16([0xde81]).unwrap())
        .unwrap()
        .try_concat(&z)
        .unwrap();
    let prefix_peer = ordinary
        .try_concat(&JsString::try_from_utf8("q").unwrap())
        .unwrap();
    assert_eq!(ordinary.len(), 8708);
    assert_eq!(ordinary, peer);
    (ordinary, peer, boundary_peer, prefix_peer)
}

fn define_data(runtime: &Runtime, object: &ObjectRef, name: &str, value: Value) {
    let key = runtime.intern_property_key(name).unwrap();
    assert!(
        runtime
            .define_own_property(
                object,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(false),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );
}

fn property_callable(
    runtime: &Runtime,
    context: &mut Context,
    object: &ObjectRef,
    name: &str,
) -> CallableRef {
    let key = runtime.intern_property_key(name).unwrap();
    let Value::Object(value) = context.get_property(object, &key).unwrap() else {
        panic!("{name} was not an object");
    };
    runtime
        .as_callable(&value)
        .unwrap()
        .unwrap_or_else(|| panic!("{name} was not callable"))
}

fn intrinsic_prototype(runtime: &Runtime, context: &mut Context, name: &str) -> ObjectRef {
    let global = context.global_object().unwrap();
    let constructor = property_callable(runtime, context, &global, name);
    let Value::Object(prototype) = context
        .get_property(
            constructor.as_object(),
            &runtime.intern_property_key("prototype").unwrap(),
        )
        .unwrap()
    else {
        panic!("{name}.prototype was not an object");
    };
    prototype
}

fn take_exception_object(context: &mut Context) -> ObjectRef {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("operation did not throw an Error object");
    };
    error
}

fn assert_internal_string_too_long(runtime: &Runtime, context: &mut Context, error: &ObjectRef) {
    assert_eq!(
        error_string(runtime, context, error, "name"),
        JsString::try_from_utf8("InternalError").unwrap()
    );
    assert_eq!(
        error_string(runtime, context, error, "message"),
        JsString::try_from_utf8("string too long").unwrap()
    );
}

fn take_error_observation(runtime: &Runtime, context: &mut Context) -> String {
    let error = take_exception_object(context);
    let name = error_string(runtime, context, &error, "name");
    let message = error_string(runtime, context, &error, "message");
    format!("throw:{}:{}", name.to_utf8_lossy(), message.to_utf8_lossy())
}

fn observe_source_exception(runtime: &Runtime, context: &mut Context, source: &str) -> String {
    assert_eq!(context.eval(source), Err(RuntimeError::Exception));
    take_error_observation(runtime, context)
}

fn error_string(
    runtime: &Runtime,
    context: &mut Context,
    error: &ObjectRef,
    name: &str,
) -> JsString {
    let Value::String(value) = context
        .get_property(error, &runtime.intern_property_key(name).unwrap())
        .unwrap()
    else {
        panic!("Error.{name} was not a String");
    };
    value
}

fn oracle_observations(oracle: &OsStr) -> Vec<String> {
    let source = format!("{ORACLE_SETUP}\nprint({PROBE});");
    let output = Command::new(oracle)
        .arg("-e")
        .arg(source)
        .output()
        .expect("run QuickJS String rope oracle");
    assert!(
        output.status.success(),
        "QuickJS String rope oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS String rope oracle emitted non-UTF-8 output")
        .lines()
        .map(str::to_owned)
        .collect()
}
