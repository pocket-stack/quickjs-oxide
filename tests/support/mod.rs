use quickjs_oxide::{Runtime, RuntimeError, Value};

pub fn compile_syntax_error(source: &str) -> String {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    assert_eq!(
        context.compile(source).unwrap_err(),
        RuntimeError::Exception,
        "source did not fail with a JavaScript exception: {source:?}",
    );
    let Value::Object(error) = context
        .take_exception()
        .unwrap()
        .unwrap_or_else(|| panic!("compile exception was missing: {source:?}"))
    else {
        panic!("compile exception was not an Error object: {source:?}");
    };
    assert!(runtime.is_error_object(&error).unwrap());

    let name_key = runtime.intern_property_key("name").unwrap();
    let Value::String(name) = context.get_property(&error, &name_key).unwrap() else {
        panic!("compile exception name was not a string: {source:?}");
    };
    assert_eq!(name.to_utf8_lossy(), "SyntaxError", "{source}");

    let message_key = runtime.intern_property_key("message").unwrap();
    let Value::String(message) = context.get_property(&error, &message_key).unwrap() else {
        panic!("compile exception message was not a string: {source:?}");
    };
    message.to_utf8_lossy()
}
