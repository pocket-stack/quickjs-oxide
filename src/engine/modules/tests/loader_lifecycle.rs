use std::cell::{Cell, RefCell};

use super::*;

#[test]
fn attribute_check_samples_the_current_loader_for_each_clause() {
    let runtime = Runtime::new();
    let (mut loader, controls) = AttributeModuleLoader::new([
        ("pkg/first.js", "export const first = 20;"),
        ("pkg/second.js", "export const second = 22;"),
    ]);
    loader.clear_runtime_on_first_check = Some(runtime.clone());
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    assert!(matches!(
        context.compile_module_with_filename(
            r#"
            import { first } from "./first.js" with { type: "javascript" };
            import { second } from "./second.js" with { type: "javascript" };
            globalThis.__attributeLoaderSnapshot = first + second;
            "#,
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    // The first checker callback cleared the installed loader. QuickJS
    // re-reads the hook for the second clause, so A is not called twice;
    // resolution then fails before either dependency can load.
    assert_eq!(controls.checks.borrow().len(), 1);
    assert!(controls.loads.borrow().is_empty());
}

#[test]
fn attribute_check_replacement_is_visible_to_the_next_clause_and_resolution() {
    let runtime = Runtime::new();
    let (replacement, replacement_controls) = AttributeModuleLoader::new([
        ("pkg/first.js", "export const first = 20;"),
        ("pkg/second.js", "export const second = 22;"),
    ]);
    let initial_checks = Rc::new(RefCell::new(Vec::new()));
    let loader = AttributeReplacingModuleLoader {
        runtime: runtime.clone(),
        replacement: RefCell::new(Some(replacement)),
        replacement_registration: RefCell::new(None),
        checks: initial_checks.clone(),
    };
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { first } from "./first.js" with { phase: "initial" };
            import { second } from "./second.js" with { phase: "replacement" };
            globalThis.__attributeLoaderReplacement = first + second;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeLoaderReplacement === 42");
    assert_eq!(
        initial_checks.borrow().as_slice(),
        &[vec![("phase".to_owned(), "initial".to_owned())]]
    );
    assert_eq!(
        replacement_controls.checks.borrow().as_slice(),
        &[vec![("phase".to_owned(), "replacement".to_owned())]]
    );
    assert_eq!(replacement_controls.loads.borrow().len(), 2);
}

#[test]
fn loader_boundary_preserves_distinct_lone_surrogate_specifiers() {
    let runtime = Runtime::new();
    let (loader, loads) = Utf16RecordingModuleLoader::new([
        (vec![0xd800], "export const value = 40;"),
        (vec![0xd801], "export const value = 2;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { value as first } from "\ud800";
            import { value as second } from "\ud801";
            globalThis.__surrogateModuleNames = first + second;
            "#,
            "entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__surrogateModuleNames === 42");
    assert_eq!(&*loads.borrow(), &[vec![0xd800], vec![0xd801]]);
}

#[test]
fn loader_error_preserves_lone_surrogate_module_name() {
    let runtime = Runtime::new();
    let (loader, loads) = Utf16RecordingModuleLoader::new([]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(r#"import "\ud800";"#, "entry.js"),
        Err(RuntimeError::Exception)
    ));
    let message = take_error_message(&runtime, &mut context);
    let expected = "could not load module '"
        .encode_utf16()
        .chain([0xd800])
        .chain("': UTF-16 fixture module is missing".encode_utf16())
        .collect::<Vec<_>>();
    assert_eq!(message.utf16_units().collect::<Vec<_>>(), expected);
    assert_eq!(&*loads.borrow(), &[vec![0xd800]]);
}

#[test]
fn loader_boundary_retains_quickjs_c_string_nul_truncation() {
    let runtime = Runtime::new();
    let (loader, loads) = Utf16RecordingModuleLoader::new([(
        "pkg".encode_utf16().collect(),
        "export const value = 21;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { value as first } from "pkg\u0000first";
            import { value as second } from "pkg\u0000second";
            globalThis.__nulModuleNames = first + second;
            "#,
            "entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__nulModuleNames === 42");
    assert_eq!(
        &*loads.borrow(),
        &["pkg".encode_utf16().collect::<Vec<_>>()]
    );
}

#[test]
fn loader_registration_keeps_host_ownership_outside_the_runtime() {
    let runtime = Runtime::new();
    let drops = Rc::new(Cell::new(0));
    let registration = runtime.set_module_loader(RuntimeHoldingLoader {
        _runtime: runtime.clone(),
        drops: drops.clone(),
    });
    drop(runtime);
    assert_eq!(drops.get(), 0);
    drop(registration);
    assert_eq!(drops.get(), 1);
}

#[test]
fn nested_request_samples_loader_after_parent_load_clears_it() {
    let runtime = Runtime::new();
    let loads = Rc::new(RefCell::new(Vec::new()));
    let loader = ClearingModuleLoader {
        runtime: runtime.clone(),
        sources: [
            (
                "pkg/a.js".to_owned(),
                "import { value } from './b.js'; export const answer = value + 1;".to_owned(),
            ),
            ("pkg/b.js".to_owned(), "export const value = 41;".to_owned()),
        ]
        .into_iter()
        .collect(),
        loads: loads.clone(),
        cleared: Cell::new(false),
    };
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    assert!(matches!(
        context.compile_module_with_filename(
            "import { answer } from './a.js'; globalThis.__loaderSnapshot = answer;",
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(&*loads.borrow(), &["pkg/a.js"]);
}

#[test]
fn load_samples_replacement_installed_by_normalize() {
    let runtime = Runtime::new();
    let (replacement, replacement_loads, _) =
        MapModuleLoader::new([("pkg/value.js", "export const value = 42;")]);
    let initial_normalizations = Rc::new(RefCell::new(Vec::new()));
    let initial_loads = Rc::new(RefCell::new(Vec::new()));
    let loader = NormalizeReplacingModuleLoader {
        runtime: runtime.clone(),
        replacement: RefCell::new(Some(replacement)),
        replacement_registration: RefCell::new(None),
        normalizations: initial_normalizations.clone(),
        loads: initial_loads.clone(),
    };
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { value } from './value.js'; globalThis.__normalizeReplacement = value;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__normalizeReplacement === 42");
    assert_eq!(initial_normalizations.borrow().len(), 1);
    assert!(initial_loads.borrow().is_empty());
    assert_eq!(replacement_loads.borrow().as_slice(), &["pkg/value.js"]);
}

#[test]
fn loader_panic_rolls_back_the_active_resolution_transaction() {
    let runtime = Runtime::new();
    let panicking_registration = runtime.set_module_loader(PanickingModuleLoader);
    let mut context = runtime.new_context();
    let stack_top_sentinel = Some(0x5a5a_usize);
    runtime.0.host_stack_top.set(stack_top_sentinel);

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = context.compile_module_with_filename(
            "import { value } from './dependency.js'; export { value };",
            "pkg/shared.js",
        );
    }));
    assert!(panic.is_err());
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
    assert_eq!(runtime.0.host_stack_top.get(), stack_top_sentinel);
    drop(panicking_registration);
    runtime.clear_module_loader();

    context
        .compile_module_with_filename("export const value = 42;", "pkg/shared.js")
        .unwrap();
    let importer = context
        .compile_module_with_filename(
            "import { value } from './shared.js'; globalThis.__panicRollback = value;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__panicRollback === 42");
}

#[test]
fn host_panic_poisons_every_active_module_evaluation() {
    let runtime = Runtime::new_with_host_services(PanickingClockHost);
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            "globalThis.__beforeClockPanic = true; Date.now(); globalThis.__afterClockPanic = true;",
        )
        .unwrap();
    context.link_module(&module).unwrap();

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = context.execute_module(&module);
    }));
    assert!(panic.is_err());
    assert_eq!(
        context.execute_module(&module),
        Err(RuntimeError::Invariant(
            "module evaluation previously failed inside the engine"
        ))
    );
    assert_script_true(
        &mut context,
        "__beforeClockPanic === true && typeof __afterClockPanic === 'undefined'",
    );
}

#[test]
fn module_callbacks_receive_the_exact_initiating_context() {
    let runtime = Runtime::new();
    let callbacks = Rc::new(RefCell::new(Vec::new()));
    let _registration = runtime.set_module_loader(ContextRecordingModuleLoader {
        callbacks: callbacks.clone(),
    });
    let mut context = runtime.new_context();
    let expected_id = context.id();
    let expected_realm = context.realm_id();

    let module = context
        .compile_module_with_filename(
            "import { answer } from './dependency.js' with { type: 'javascript' }; globalThis.__callbackContextAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__callbackContextAnswer === 42");
    assert_eq!(
        callbacks.borrow().as_slice(),
        [
            ("attributes", expected_id, expected_realm),
            ("normalize", expected_id, expected_realm),
            ("load", expected_id, expected_realm),
        ]
    );
}

#[test]
fn loader_accepts_a_compiled_module_from_the_initiating_context() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let dependency = context
        .compile_module_with_filename("export const answer = 42;", "pkg/compiled-dependency.js")
        .unwrap();
    let _registration = runtime.set_module_loader(CompiledModuleLoader {
        module: dependency.clone(),
    });

    let entry = context
        .compile_module_with_filename(
            "import { answer } from './selected.js'; globalThis.__compiledLoaderAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&entry).unwrap();
    assert_script_true(&mut context, "__compiledLoaderAnswer === 42");
    assert_eq!(
        context.runtime().module_dependencies(&entry).unwrap(),
        [dependency]
    );
}

#[test]
fn compiled_loader_result_rejects_foreign_runtime_and_context() {
    let runtime = Runtime::new();
    let foreign_runtime = Runtime::new();
    let foreign_module = foreign_runtime
        .new_context()
        .compile_module("export const answer = 1;")
        .unwrap();
    let mut context = runtime.new_context();
    let _registration = runtime.set_module_loader(CompiledModuleLoader {
        module: foreign_module,
    });
    assert!(matches!(
        context.compile_module_with_filename("import './selected.js';", "pkg/entry.js"),
        Err(RuntimeError::WrongRuntime("compiled module"))
    ));

    drop(_registration);
    runtime.clear_module_loader();
    let other_module = runtime
        .new_context()
        .compile_module("export const answer = 2;")
        .unwrap();
    let _registration = runtime.set_module_loader(CompiledModuleLoader {
        module: other_module,
    });
    assert!(matches!(
        context.compile_module_with_filename("import './other.js';", "pkg/other-entry.js"),
        Err(RuntimeError::WrongContext("compiled module"))
    ));
}

#[test]
fn loader_dependency_with_top_level_await_evaluates_asynchronously() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "await 1; globalThis.__loadedTlaDependency = 42;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let module = context
        .compile_module_with_filename("import './dependency.js';", "pkg/entry.js")
        .unwrap();
    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(drain_jobs(&runtime) > 0);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
    assert_script_true(&mut context, "__loadedTlaDependency === 42");
    assert_eq!(&*loads.borrow(), &["pkg/dependency.js"]);
}
