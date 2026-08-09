#![cfg(feature = "test262-host")]

use quickjs_oxide::heap::{NativeCProto, NativeFunctionId};
use quickjs_oxide::{Context, PendingJobOutcome, Runtime, RuntimeError, Value};

const FIXTURE: &str = include_str!("fixtures/create_realm_host.js");
const QUICKJS_2026_06_04: &str = include_str!("fixtures/create_realm_host.quickjs-2026-06-04.txt");

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
fn test262_realm_helpers_are_defining_realm_generic_functions() {
    for target in [
        NativeFunctionId::Test262EvalScript,
        NativeFunctionId::Test262CreateRealm,
        NativeFunctionId::Test262IsHtmlDda,
    ] {
        assert_eq!(target.descriptor().cproto, NativeCProto::Generic);
        assert!(!target.descriptor().cproto.default_is_constructor());
        assert!(!target.uses_calling_realm());
    }
}

#[test]
fn test262_create_realm_and_eval_script_match_pinned_quickjs_transcript() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let installed = context
        .install_test262_host()
        .expect("install Test262 host surface");
    let global_262 = eval(&mut context, "globalThis.$262");
    assert!(matches!(global_262, Value::Object(object) if object == installed));

    eval(&mut context, FIXTURE);
    let transcript = text(eval(&mut context, "createRealmTranscript.join('\\n')"));
    assert_eq!(format!("{transcript}\n"), QUICKJS_2026_06_04);

    assert!(
        runtime.is_job_pending(),
        "evalScript must not drain a child realm's Promise jobs"
    );
    let first = runtime
        .execute_pending_job_with_context()
        .expect("execute child realm Promise job");
    let PendingJobOutcome::Executed {
        context: Some(job_realm),
    } = first
    else {
        panic!("child realm job did not report its surviving realm: {first:?}");
    };
    assert_ne!(job_realm, context.realm_id());
    let mut remaining_jobs = 0usize;
    while runtime.is_job_pending() {
        remaining_jobs += 1;
        assert!(
            remaining_jobs <= 64,
            "child realm Promise jobs did not settle within 64 executions"
        );
        runtime
            .execute_pending_job_with_context()
            .expect("drain remaining child realm Promise jobs");
    }
    assert_eq!(
        eval(&mut context, "createRealmChild.global.realmJobValue"),
        Value::Int(42)
    );
}

#[test]
fn returned_child_host_retains_and_then_releases_its_realm_cycle() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let parent_262 = context
        .install_test262_host()
        .expect("install parent Test262 host surface");
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    let child_262 = eval(&mut context, "$262.createRealm()");
    let Value::Object(child_262) = child_262 else {
        panic!("createRealm did not return the child $262 object");
    };
    assert_eq!(
        runtime.heap_counts().context_nodes,
        2,
        "dropping createRealm's temporary Context must leave the returned realm alive"
    );
    runtime.run_gc().expect("collect while child is exported");
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    let eval_script_key = runtime
        .intern_property_key("evalScript")
        .expect("intern evalScript key");
    let Value::Object(eval_script_object) = context
        .get_property(&child_262, &eval_script_key)
        .expect("read child evalScript")
    else {
        panic!("child evalScript was not an object");
    };
    let eval_script = runtime
        .as_callable(&eval_script_object)
        .expect("validate child evalScript")
        .expect("child evalScript was not callable");
    drop(eval_script_object);
    drop(child_262);
    runtime
        .run_gc()
        .expect("collect while only child callable is exported");
    assert_eq!(runtime.heap_counts().context_nodes, 2);
    assert_eq!(
        context
            .call(
                &eval_script,
                Value::Undefined,
                &[Value::String(
                    quickjs_oxide::JsString::try_from_utf8("40 + 2")
                        .expect("build evalScript source"),
                )],
            )
            .expect("call retained child evalScript from parent realm"),
        Value::Int(42)
    );

    drop(eval_script);
    runtime
        .run_gc()
        .expect("collect unreachable child realm cycle");
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    let child_262 = eval(&mut context, "$262.createRealm()");
    let Value::Object(child_262) = child_262 else {
        panic!("second createRealm did not return the child $262 object");
    };
    let is_html_dda_key = runtime
        .intern_property_key("IsHTMLDDA")
        .expect("intern IsHTMLDDA key");
    let Value::Object(is_html_dda_object) = context
        .get_property(&child_262, &is_html_dda_key)
        .expect("read child IsHTMLDDA")
    else {
        panic!("child IsHTMLDDA was not an object");
    };
    let is_html_dda = runtime
        .as_callable(&is_html_dda_object)
        .expect("validate child IsHTMLDDA")
        .expect("child IsHTMLDDA was not callable");
    drop(is_html_dda_object);
    drop(child_262);
    runtime
        .run_gc()
        .expect("collect while only child IsHTMLDDA is exported");
    assert_eq!(runtime.heap_counts().context_nodes, 2);
    assert_eq!(
        context
            .call(&is_html_dda, Value::Undefined, &[])
            .expect("call retained child IsHTMLDDA from parent realm"),
        Value::Null
    );
    drop(is_html_dda);
    runtime
        .run_gc()
        .expect("collect IsHTMLDDA-retained child realm cycle");
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    drop(parent_262);
    drop(context);
    runtime.run_gc().expect("collect parent realm cycle");
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}
