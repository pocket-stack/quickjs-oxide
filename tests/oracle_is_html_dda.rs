#![cfg(feature = "test262-host")]

use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const FIXTURE: &str = include_str!("fixtures/is_html_dda.js");
const QUICKJS_2026_06_04: &str = include_str!("fixtures/is_html_dda.quickjs-2026-06-04.txt");

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
        panic!("expected a string value");
    };
    value.to_utf8_lossy()
}

#[test]
fn is_html_dda_semantics_match_pinned_quickjs_transcript() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .install_test262_host()
        .expect("install Test262 host surface");

    let html_dda = eval(&mut context, "$262.IsHTMLDDA");
    assert!(!html_dda.to_boolean());

    eval(&mut context, FIXTURE);
    let transcript = text(eval(&mut context, "isHtmlDdaTranscript.join('\\n')"));
    assert_eq!(format!("{transcript}\n"), QUICKJS_2026_06_04);
}
