use crate::value::fail_next_replacement_reservation_for_test;

use super::super::super::*;

#[test]
fn regexp_escape_is_strict_static_generic_and_non_constructible() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let Value::String(transcript) = context
        .eval(
            r#"
            (function () {
                var descriptor = Object.getOwnPropertyDescriptor(RegExp, "escape");
                var touched = 0;
                var inputError;
                try {
                    RegExp.escape({ toString: function () { touched++; return "x"; } });
                } catch (error) {
                    inputError = error.name + ":" + error.message;
                }
                var constructError;
                try {
                    new RegExp.escape("x");
                } catch (error) {
                    constructError = error.name;
                }
                return [
                    Object.getOwnPropertyNames(RegExp).join(","),
                    typeof RegExp.escape,
                    RegExp.escape.length,
                    RegExp.escape.name,
                    descriptor.writable,
                    descriptor.enumerable,
                    descriptor.configurable,
                    RegExp.escape.call(42, "a1_.-/"),
                    inputError,
                    touched,
                    constructError
                ].join("|");
            })()
            "#,
        )
        .expect("RegExp.escape surface probe")
    else {
        panic!("RegExp.escape surface probe did not return a String");
    };
    assert_eq!(
        transcript.to_utf8_lossy(),
        "length,name,escape,prototype|function|1|escape|true|false|true|\
         \\x611_\\.\\x2d\\/|TypeError:not a string|0|TypeError",
    );
}

#[test]
fn direct_replace_uses_a_second_buffer_while_generic_replace_keeps_the_outer_error() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval("RegExp.prototype.exec")
        .expect("materialize the standard exec slot");

    fail_next_replacement_reservation_for_test();
    let Value::String(result) = context
        .eval(r#"/a/[Symbol.replace]("a","X")"#)
        .expect("the direct matcher must discard the failed outer buffer")
    else {
        panic!("direct replacement did not return a String");
    };
    assert_eq!(result.to_utf8_lossy(), "X");

    fail_next_replacement_reservation_for_test();
    assert_eq!(
        context.eval(r#"/a/[Symbol.replace]("a",function(){return "X"})"#),
        Err(RuntimeError::Exception),
        "functional replacement must keep the generic outer buffer failure",
    );
    let Value::Object(error) = context
        .take_exception()
        .expect("take replacement buffer exception")
        .expect("replacement buffer exception was missing")
    else {
        panic!("replacement buffer exception was not an Error object");
    };
    let message = runtime.intern_property_key("message").unwrap();
    let Value::String(message) = context.get_property(&error, &message).unwrap() else {
        panic!("replacement buffer Error message was not a String");
    };
    assert_eq!(message.to_utf8_lossy(), "out of memory");
}
