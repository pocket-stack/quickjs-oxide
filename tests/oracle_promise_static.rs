use quickjs_oxide::{Context, Runtime, RuntimeError, Value};

const FIXTURE: &str = include_str!("fixtures/r3n_promise_static.js");
const EXPECTED: &str = include_str!("fixtures/r3n_promise_static.quickjs-2026-06-04.txt");

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

#[test]
fn promise_try_with_resolvers_and_race_match_pinned_quickjs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    eval(&mut context, FIXTURE);
    runtime.run_gc().unwrap();
    while runtime.is_job_pending() {
        assert!(runtime.execute_pending_job().unwrap());
        runtime.run_gc().unwrap();
    }

    let Value::String(transcript) = eval(&mut context, "r3nTranscript.join('\\n') + '\\n'") else {
        panic!("R3n Promise static transcript was not a string");
    };
    assert_eq!(transcript.to_utf8_lossy(), EXPECTED);
}
