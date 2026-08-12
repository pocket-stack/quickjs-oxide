use std::cell::RefCell;
use std::rc::Rc;

use quickjs_oxide::{
    ErrorKind, JsString, ModuleLoader, ModuleLoaderError, Runtime, RuntimeError, Value,
};

mod support;

use support::compile_syntax_error;

const IMPORT_SOURCE: &str = "import('dependency')";

#[derive(Debug)]
struct RecordingModuleLoader {
    loads: Rc<RefCell<Vec<String>>>,
}

impl ModuleLoader for RecordingModuleLoader {
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = normalized_name.to_utf8_lossy();
        self.loads.borrow_mut().push(normalized_name.clone());
        if normalized_name == "dependency" {
            Ok("globalThis.__dynamicImportBodyRuns += 1; export const answer = 42;".to_owned())
        } else {
            Err(ModuleLoaderError::new(format!(
                "unexpected test module: {normalized_name}"
            )))
        }
    }
}

fn assert_unsupported<T>(
    label: &str,
    result: Result<T, RuntimeError>,
    context: &mut quickjs_oxide::Context,
    expected_message: &str,
) {
    let Err(RuntimeError::Engine(error)) = result else {
        panic!("{label} did not retain the Unsupported engine diagnostic");
    };
    assert_eq!(error.kind(), ErrorKind::Unsupported);
    assert_eq!(error.message(), expected_message);
    assert!(context.take_exception().unwrap().is_none());
}

fn assert_dynamic_import_jobs(source: &str, install_test262_host: bool) {
    let runtime = Runtime::new();
    let loads = Rc::new(RefCell::new(Vec::new()));
    let _registration = runtime.set_module_loader(RecordingModuleLoader {
        loads: loads.clone(),
    });
    let mut context = runtime.new_context();
    if install_test262_host {
        #[cfg(feature = "test262-host")]
        context.install_test262_host().unwrap();
        #[cfg(not(feature = "test262-host"))]
        panic!("the Test262 host is unavailable without test262-host");
    }
    context
        .eval(
            "globalThis.__dynamicImportBodyRuns = 0; \
             globalThis.__dynamicImportResult = 'pending';",
        )
        .unwrap();

    let Value::Object(_) = context
        .eval(source)
        .unwrap_or_else(|error| panic!("dynamic-import entrypoint rejected valid syntax: {error}"))
    else {
        panic!("dynamic-import entrypoint did not return a Promise object");
    };
    assert_eq!(
        context
            .eval("globalThis.__dynamicImportPromise instanceof Promise")
            .unwrap(),
        Value::Bool(true)
    );
    assert_eq!(
        context.eval("globalThis.__dynamicImportResult").unwrap(),
        Value::String(JsString::try_from_utf8("pending").unwrap())
    );
    assert!(loads.borrow().is_empty(), "module load ran synchronously");
    assert!(
        runtime.is_job_pending(),
        "ImportCall did not enqueue a load job"
    );

    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(loads.borrow().as_slice(), ["dependency"]);
    assert_eq!(
        context.eval("globalThis.__dynamicImportBodyRuns").unwrap(),
        Value::Int(1)
    );
    assert_eq!(
        context.eval("globalThis.__dynamicImportResult").unwrap(),
        Value::String(JsString::try_from_utf8("pending").unwrap()),
        "the user reaction ran in the module-load job"
    );

    let mut executed_jobs = 1usize;
    while runtime.execute_pending_job().unwrap() {
        executed_jobs += 1;
        assert!(executed_jobs <= 8, "dynamic-import jobs did not quiesce");
    }
    assert!(
        executed_jobs >= 3,
        "dynamic import collapsed load, finish, and user reaction jobs"
    );
    assert_eq!(
        context.eval("globalThis.__dynamicImportResult").unwrap(),
        Value::Int(42)
    );
    assert!(!runtime.is_job_pending());
    assert!(context.take_exception().unwrap().is_none());
}

#[test]
fn public_entrypoints_execute_dynamic_import_promise_jobs() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    context
        .compile(IMPORT_SOURCE)
        .expect("Context::compile rejected a valid ImportCall");
    assert!(
        !runtime.is_job_pending(),
        "compilation scheduled an import job"
    );
    assert!(context.take_exception().unwrap().is_none());

    assert_dynamic_import_jobs(
        "globalThis.__dynamicImportPromise = \
         import('dependency').then(function(namespace) { \
           globalThis.__dynamicImportResult = namespace.answer; \
           return namespace.answer; \
         }); \
         globalThis.__dynamicImportPromise",
        false,
    );

    assert_dynamic_import_jobs(
        r#"globalThis.__dynamicImportPromise = Function(
             "return import('dependency').then(function(namespace) { globalThis.__dynamicImportResult = namespace.answer; return namespace.answer; });"
           )();
           globalThis.__dynamicImportPromise"#,
        false,
    );
}

#[test]
fn genuine_synchronous_module_frontiers_remain_unsupported() {
    for source in ["await 1;", "for await (const value of []) {}"] {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let result = context.compile_module(source);
        assert_unsupported(
            "Context::compile_module",
            result,
            &mut context,
            "top-level await is not implemented in this synchronous module slice",
        );
    }
}

#[cfg(feature = "test262-host")]
#[test]
fn conformance_eval_script_executes_dynamic_import_promise_jobs() {
    assert_dynamic_import_jobs(
        r#"globalThis.__dynamicImportPromise = $262.evalScript(
             "import('dependency').then(function(namespace) { globalThis.__dynamicImportResult = namespace.answer; return namespace.answer; })"
           );
           globalThis.__dynamicImportPromise"#,
        true,
    );
}

#[test]
fn context_compiles_and_executes_catch_destructuring_bindings() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let bytecode = context
        .compile("try { throw {value: 42}; } catch ({value}) { value }")
        .unwrap();
    assert_eq!(context.execute(&bytecode).unwrap(), Value::Int(42));
}

#[test]
fn object_rest_bindings_preserve_lexical_conflict_diagnostics() {
    for source in [
        "let {value, ...value} = {value: 1};",
        "const {value, ...value} = {value: 1};",
        "let value; var {...value} = {};",
    ] {
        assert_eq!(
            compile_syntax_error(source),
            "invalid redefinition of lexical identifier",
            "{source}"
        );
    }
}

#[test]
fn object_rest_bindings_preserve_later_source_error_priority() {
    for (source, expected) in [
        ("let {...rest} = ;", "unexpected token in expression: ';'"),
        (
            "for (let {...rest} of ) {}",
            "unexpected token in expression: ')'",
        ),
        (
            "let {...value} = {}, value;",
            "invalid redefinition of lexical identifier",
        ),
        (
            "let {...value} = {}; let value;",
            "invalid redefinition of lexical identifier",
        ),
        (
            "function f(){ var {...value} = {}; let value; }",
            "invalid redefinition of a variable",
        ),
    ] {
        assert_eq!(compile_syntax_error(source), expected, "{source}");
    }
}
