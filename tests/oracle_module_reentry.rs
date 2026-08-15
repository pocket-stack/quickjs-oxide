use std::cell::{Cell, RefCell};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::rc::Rc;

use quickjs_oxide::{
    Context, ContextId, JsString, ModuleImportAttributes, ModuleLoadResult, ModuleLoader,
    ModuleLoaderError, PromiseState, Runtime, Value,
};

const MODULE_GLOBAL_SHADOW_TRANSCRIPT: &str =
    include_str!("fixtures/module-global-shadow/quickjs-2026-06-04.txt");

#[derive(Debug, PartialEq, Eq)]
struct CallbackEvent {
    phase: &'static str,
    depth: usize,
    subject: String,
    context_id: u64,
    realm_id: ContextId,
}

#[derive(Debug)]
struct DynamicReentryLoader {
    depth: Rc<Cell<usize>>,
    events: Rc<RefCell<Vec<CallbackEvent>>>,
}

impl DynamicReentryLoader {
    fn record(&self, phase: &'static str, subject: String, context: &Context) {
        self.events.borrow_mut().push(CallbackEvent {
            phase,
            depth: self.depth.get(),
            subject,
            context_id: context.id(),
            realm_id: context.realm_id(),
        });
    }

    fn compile_loaded_module(
        &self,
        context: &mut Context,
        source: &str,
        filename: &str,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let previous_depth = self.depth.get();
        self.depth.set(previous_depth + 1);
        let result = context.compile_module_with_filename(source, filename);
        self.depth.set(previous_depth);
        result.map(ModuleLoadResult::Compiled).map_err(|error| {
            ModuleLoaderError::new(format!("nested module compilation failed: {error}"))
        })
    }
}

impl ModuleLoader for DynamicReentryLoader {
    fn normalize_in_context(
        &self,
        context: &mut Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        let base_name = base_name.to_utf8_lossy();
        let specifier = specifier.to_utf8_lossy();
        self.record("normalize", format!("{base_name}|{specifier}"), context);
        match (base_name.as_str(), specifier.as_str()) {
            ("dynamic-reentry-entry.js", "./outer.js") => {
                Ok(JsString::try_from_utf8("outer.js").unwrap())
            }
            ("outer.js", "./inner.js") => Ok(JsString::try_from_utf8("inner.js").unwrap()),
            _ => Err(ModuleLoaderError::new("unexpected module normalization")),
        }
    }

    fn load_with_attributes_in_context(
        &self,
        context: &mut Context,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = normalized_name.to_utf8_lossy();
        self.record("load", normalized_name.clone(), context);
        match normalized_name.as_str() {
            "outer.js" => self.compile_loaded_module(
                context,
                "import { value } from './inner.js'; \
                 globalThis.__dynamicReentryOrder.push('outer'); \
                 export const answer = value + 1;",
                "outer.js",
            ),
            "inner.js" => self.compile_loaded_module(
                context,
                "globalThis.__dynamicReentryOrder.push('inner'); export const value = 41;",
                "inner.js",
            ),
            _ => Err(ModuleLoaderError::new("unexpected module load")),
        }
    }
}

#[test]
fn dynamic_import_accepts_reentrant_compiled_modules_from_the_initiating_context() {
    let runtime = Runtime::new();
    let depth = Rc::new(Cell::new(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let _registration = runtime.set_module_loader(DynamicReentryLoader {
        depth: depth.clone(),
        events: events.clone(),
    });
    let mut context = runtime.new_context();
    let expected_context_id = context.id();
    let expected_realm_id = context.realm_id();

    let Value::Object(promise) = context
        .eval_with_filename(
            "globalThis.__dynamicReentryOrder = []; \
             globalThis.__dynamicReentryResult = 'pending'; \
             import('./outer.js').then(function (namespace) { \
               globalThis.__dynamicReentryOrder.push('entry'); \
               globalThis.__dynamicReentryResult = namespace.answer; \
               return namespace.answer; \
             });",
            "dynamic-reentry-entry.js",
        )
        .unwrap()
    else {
        panic!("dynamic import did not return a Promise");
    };

    assert!(events.borrow().is_empty(), "dynamic load ran synchronously");
    let mut executed_jobs = 0;
    while runtime.execute_pending_job().unwrap() {
        executed_jobs += 1;
        assert!(executed_jobs <= 8, "dynamic import jobs did not quiesce");
    }

    assert_eq!(depth.get(), 0);
    assert_eq!(
        events.borrow().as_slice(),
        [
            CallbackEvent {
                phase: "normalize",
                depth: 0,
                subject: "dynamic-reentry-entry.js|./outer.js".to_owned(),
                context_id: expected_context_id,
                realm_id: expected_realm_id,
            },
            CallbackEvent {
                phase: "load",
                depth: 0,
                subject: "outer.js".to_owned(),
                context_id: expected_context_id,
                realm_id: expected_realm_id,
            },
            CallbackEvent {
                phase: "normalize",
                depth: 1,
                subject: "outer.js|./inner.js".to_owned(),
                context_id: expected_context_id,
                realm_id: expected_realm_id,
            },
            CallbackEvent {
                phase: "load",
                depth: 1,
                subject: "inner.js".to_owned(),
                context_id: expected_context_id,
                realm_id: expected_realm_id,
            },
        ]
    );
    assert_eq!(
        context
            .eval("JSON.stringify(__dynamicReentryOrder)")
            .unwrap(),
        Value::String(JsString::try_from_utf8("[\"inner\",\"outer\",\"entry\"]").unwrap())
    );
    assert_eq!(
        context.eval("__dynamicReentryResult").unwrap(),
        Value::Int(42)
    );
    let snapshot = runtime
        .promise_snapshot(&promise)
        .unwrap()
        .expect("dynamic import result was not a Promise");
    assert_eq!(snapshot.state(), PromiseState::Fulfilled);
    assert_eq!(snapshot.result(), &Value::Int(42));
    assert!(!runtime.is_job_pending());
    assert_eq!(context.take_exception().unwrap(), None);
}

#[test]
fn module_global_binding_survives_nested_and_sibling_lexical_shadowing() {
    let fixture = module_global_shadow_fixture_dir();
    let oxide =
        run_module_global_shadow_file(env!("CARGO_BIN_EXE_qjs").as_ref(), &fixture, "entry.mjs");
    assert_module_global_shadow_success("quickjs-oxide", "entry.mjs", &oxide);
    assert_eq!(
        String::from_utf8_lossy(&oxide.stdout),
        MODULE_GLOBAL_SHADOW_TRANSCRIPT,
        "quickjs-oxide confused the exported module cell with a nested or sibling lexical",
    );

    for invalid in ["same-scope.mjs", "overlapping-scope.mjs"] {
        let rejected =
            run_module_global_shadow_file(env!("CARGO_BIN_EXE_qjs").as_ref(), &fixture, invalid);
        assert_module_global_shadow_rejection("quickjs-oxide", invalid, &rejected);
    }

    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP module-global shadow differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    let quickjs = run_module_global_shadow_file(&oracle, &fixture, "entry.mjs");
    assert_module_global_shadow_success("pinned QuickJS", "entry.mjs", &quickjs);
    assert_eq!(
        String::from_utf8_lossy(&quickjs.stdout),
        MODULE_GLOBAL_SHADOW_TRANSCRIPT,
        "pinned QuickJS transcript differs from the frozen module-global shadow fixture",
    );
    assert_eq!(
        oxide.stdout, quickjs.stdout,
        "quickjs-oxide module-global shadow transcript differs from pinned QuickJS",
    );

    for invalid in ["same-scope.mjs", "overlapping-scope.mjs"] {
        let rejected = run_module_global_shadow_file(&oracle, &fixture, invalid);
        assert_module_global_shadow_rejection("pinned QuickJS", invalid, &rejected);
    }
}

fn module_global_shadow_fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/module-global-shadow")
}

fn run_module_global_shadow_file(executable: &OsStr, fixture: &Path, filename: &str) -> Output {
    Command::new(executable)
        .args(["--module", filename])
        .current_dir(fixture)
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run {executable:?} for module-global shadow {filename}: {error}")
        })
}

fn assert_module_global_shadow_success(engine: &str, filename: &str, output: &Output) {
    assert!(
        output.status.success(),
        "{engine} rejected module-global shadow fixture {filename}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    assert!(
        output.stderr.is_empty(),
        "{engine} wrote stderr for module-global shadow fixture {filename}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}

fn assert_module_global_shadow_rejection(engine: &str, filename: &str, output: &Output) {
    assert!(
        !output.status.success(),
        "{engine} accepted conflicting module declarations in {filename}",
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("SyntaxError"),
        "{engine} did not report SyntaxError for {filename}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
}
