use crate::value::fail_next_replacement_reservation_for_test;

use super::super::super::*;

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
