use super::*;

type SharedReentryEvents = Rc<RefCell<Vec<(&'static str, usize, String, u64, ContextId)>>>;

struct ReentrantCompiledModuleLoader {
    depth: Rc<Cell<usize>>,
    maximum_load_depth: Rc<Cell<usize>>,
    events: SharedReentryEvents,
}

impl fmt::Debug for ReentrantCompiledModuleLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ReentrantCompiledModuleLoader")
    }
}

fn nested_compile_load_result(
    context: &mut Context,
    result: Result<ModuleBytecodeRef, RuntimeError>,
) -> Result<ModuleLoadResult, ModuleLoaderError> {
    match result {
        Ok(module) => Ok(ModuleLoadResult::Compiled(module)),
        Err(RuntimeError::Exception) => {
            let exception = context
                .take_exception()
                .map_err(|error| ModuleLoaderError::new(error.to_string()))?
                .ok_or_else(|| {
                    ModuleLoaderError::new("nested module compilation lost its exception")
                })?;
            Err(ModuleLoaderError::exception(exception))
        }
        Err(error) => Err(ModuleLoaderError::new(error.to_string())),
    }
}

impl ModuleLoader for ReentrantCompiledModuleLoader {
    fn normalize_in_context(
        &self,
        context: &mut Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.events.borrow_mut().push((
            "normalize",
            self.depth.get(),
            format!(
                "{}|{}",
                base_name.to_utf8_lossy(),
                specifier.to_utf8_lossy()
            ),
            context.id(),
            context.realm_id(),
        ));
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn load_with_attributes_in_context(
        &self,
        context: &mut Context,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        let depth = self.depth.get();
        self.maximum_load_depth
            .set(self.maximum_load_depth.get().max(depth));
        self.events.borrow_mut().push((
            "load",
            depth,
            normalized_name.clone(),
            context.id(),
            context.realm_id(),
        ));
        self.depth.set(depth + 1);
        let result = match normalized_name.as_str() {
            "outer.js" => context.compile_module_with_filename(
                "import './inner.js'; globalThis.reentryOrder.push('outer'); export const outer = 1;",
                "outer.js",
            ),
            "inner.js" => context.compile_module_with_filename(
                "globalThis.reentryOrder.push('inner'); export const inner = 1;",
                "inner.js",
            ),
            _ => {
                self.depth.set(depth);
                return Err(ModuleLoaderError::new("fixture module is missing"));
            }
        };
        self.depth.set(depth);
        nested_compile_load_result(context, result)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ParseCacheProbeMode {
    SameNameSuccess,
    SameNameFailure,
    PrefixSuccess,
    PrefixOuterFailure,
    PrefixLoadFailure,
    PrefixLoadFailureSwallowed,
    PrefixLoadPanic,
    PrefixCycleLoadFailure,
    PrefixCycleLoadFailureSwallowed,
    PrefixCycleLoadPanic,
    CheckerPanic,
}

#[derive(Clone)]
struct ParseCacheProbeControls {
    checks: Rc<Cell<usize>>,
    loads: SharedLoaderLoads,
    normalizations: SharedLoaderNormalizations,
    nested_module: Rc<RefCell<Option<ModuleBytecodeRef>>>,
    swallowed_failure: Rc<Cell<bool>>,
}

struct ParseCacheProbeLoader {
    mode: ParseCacheProbeMode,
    controls: ParseCacheProbeControls,
}

#[derive(Clone)]
struct ProvisionalImportMetaControls {
    checks: Rc<Cell<usize>>,
    marker_survived_checker_gc: Rc<Cell<bool>>,
}

struct ProvisionalImportMetaLoader {
    dependency: RefCell<Option<ModuleLoadResult>>,
    marker: ObjectId,
    controls: ProvisionalImportMetaControls,
}

impl fmt::Debug for ProvisionalImportMetaLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ProvisionalImportMetaLoader")
    }
}

impl ProvisionalImportMetaLoader {
    fn new(
        dependency: ModuleLoadResult,
        marker: ObjectId,
    ) -> (Self, ProvisionalImportMetaControls) {
        let controls = ProvisionalImportMetaControls {
            checks: Rc::new(Cell::new(0)),
            marker_survived_checker_gc: Rc::new(Cell::new(false)),
        };
        (
            Self {
                dependency: RefCell::new(Some(dependency)),
                marker,
                controls: controls.clone(),
            },
            controls,
        )
    }
}

impl ModuleLoader for ProvisionalImportMetaLoader {
    fn check_attributes_in_context(
        &self,
        context: &mut Context,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        self.controls.checks.set(self.controls.checks.get() + 1);
        context
            .runtime()
            .run_gc()
            .map_err(|error| ModuleLoaderError::new(error.to_string()))?;
        let alive = context
            .runtime()
            .0
            .state
            .borrow()
            .heap
            .object(self.marker)
            .is_ok();
        self.controls.marker_survived_checker_gc.set(alive);
        if alive {
            Ok(())
        } else {
            Err(ModuleLoaderError::new(
                "pending import.meta property was collected during parsing",
            ))
        }
    }

    fn load_with_attributes_in_context(
        &self,
        _context: &mut Context,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        match valid_fixture_module_name(normalized_name)?.as_str() {
            "dependency.js" => self
                .dependency
                .borrow_mut()
                .take()
                .ok_or_else(|| ModuleLoaderError::new("dependency was loaded twice")),
            "leaf.js" => Ok(ModuleLoadResult::SourceText(
                "export const leaf = 1;".to_owned(),
            )),
            name => Err(ModuleLoaderError::new(format!(
                "unexpected provisional import.meta load: {name}"
            ))),
        }
    }
}

impl fmt::Debug for ParseCacheProbeLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ParseCacheProbeLoader")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

impl ParseCacheProbeLoader {
    fn new(mode: ParseCacheProbeMode) -> (Self, ParseCacheProbeControls) {
        let controls = ParseCacheProbeControls {
            checks: Rc::new(Cell::new(0)),
            loads: Rc::new(RefCell::new(Vec::new())),
            normalizations: Rc::new(RefCell::new(Vec::new())),
            nested_module: Rc::new(RefCell::new(None)),
            swallowed_failure: Rc::new(Cell::new(false)),
        };
        (
            Self {
                mode,
                controls: controls.clone(),
            },
            controls,
        )
    }

    fn compile_nested(
        &self,
        context: &mut Context,
        source: &str,
        filename: &str,
    ) -> Result<ModuleBytecodeRef, ModuleLoaderError> {
        let result = context.compile_module_with_filename(source, filename);
        let ModuleLoadResult::Compiled(module) = nested_compile_load_result(context, result)?
        else {
            return Err(ModuleLoaderError::new(
                "nested parse-cache probe returned source text",
            ));
        };
        Ok(module)
    }
}

impl ModuleLoader for ParseCacheProbeLoader {
    fn normalize_in_context(
        &self,
        _context: &mut Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.controls
            .normalizations
            .borrow_mut()
            .push((base_name.to_utf8_lossy(), specifier.to_utf8_lossy()));
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn check_attributes_in_context(
        &self,
        context: &mut Context,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        let check = self.controls.checks.get();
        self.controls.checks.set(check + 1);
        if check != 0 {
            return Err(ModuleLoaderError::new(
                "parse-cache checker was entered recursively",
            ));
        }
        if self.mode == ParseCacheProbeMode::CheckerPanic {
            panic!("intentional parse-cache checker panic");
        }
        let (source, filename) = match self.mode {
            ParseCacheProbeMode::SameNameSuccess => ("export const marker = 99;", "same.js"),
            ParseCacheProbeMode::SameNameFailure => ("export const broken = ;", "same.js"),
            ParseCacheProbeMode::PrefixSuccess
            | ParseCacheProbeMode::PrefixOuterFailure
            | ParseCacheProbeMode::PrefixLoadFailure
            | ParseCacheProbeMode::PrefixLoadFailureSwallowed
            | ParseCacheProbeMode::PrefixLoadPanic => {
                ("import './outer.js'; export const probe = 1;", "probe.js")
            }
            ParseCacheProbeMode::PrefixCycleLoadFailure
            | ParseCacheProbeMode::PrefixCycleLoadFailureSwallowed
            | ParseCacheProbeMode::PrefixCycleLoadPanic => (
                "import './outer.js'; import './missing.js'; export const probe = 1;",
                "probe.js",
            ),
            ParseCacheProbeMode::CheckerPanic => unreachable!(),
        };
        if matches!(
            self.mode,
            ParseCacheProbeMode::PrefixLoadFailureSwallowed
                | ParseCacheProbeMode::PrefixCycleLoadFailureSwallowed
        ) {
            let result = context.compile_module_with_filename(source, filename);
            if result != Err(RuntimeError::Exception) {
                return Err(ModuleLoaderError::new(
                    "nested prefix failure did not produce an exception",
                ));
            }
            let exception = context
                .take_exception()
                .map_err(|error| ModuleLoaderError::new(error.to_string()))?;
            if exception.is_none() {
                return Err(ModuleLoaderError::new(
                    "nested prefix failure lost its exception",
                ));
            }
            context
                .runtime()
                .run_gc()
                .map_err(|error| ModuleLoaderError::new(error.to_string()))?;
            self.controls.swallowed_failure.set(true);
            return Ok(());
        }
        let module = self.compile_nested(context, source, filename)?;
        context
            .runtime()
            .run_gc()
            .map_err(|error| ModuleLoaderError::new(error.to_string()))?;
        *self.controls.nested_module.borrow_mut() = Some(module);
        Ok(())
    }

    fn load_with_attributes_in_context(
        &self,
        _context: &mut Context,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let name = valid_fixture_module_name(normalized_name)?;
        self.controls.loads.borrow_mut().push(name.clone());
        if name == "before.js" {
            return match self.mode {
                ParseCacheProbeMode::PrefixSuccess | ParseCacheProbeMode::PrefixOuterFailure => Ok(
                    ModuleLoadResult::SourceText("export const before = 1;".to_owned()),
                ),
                ParseCacheProbeMode::PrefixLoadFailure
                | ParseCacheProbeMode::PrefixLoadFailureSwallowed => Err(ModuleLoaderError::new(
                    "intentional parse-time prefix load failure",
                )),
                ParseCacheProbeMode::PrefixLoadPanic => {
                    panic!("intentional parse-time prefix load panic")
                }
                ParseCacheProbeMode::SameNameSuccess
                | ParseCacheProbeMode::SameNameFailure
                | ParseCacheProbeMode::PrefixCycleLoadFailure
                | ParseCacheProbeMode::PrefixCycleLoadFailureSwallowed
                | ParseCacheProbeMode::PrefixCycleLoadPanic
                | ParseCacheProbeMode::CheckerPanic => Err(ModuleLoaderError::new(format!(
                    "unexpected parse-cache load: {name}"
                ))),
            };
        }
        if name == "missing.js" {
            return match self.mode {
                ParseCacheProbeMode::PrefixCycleLoadFailure
                | ParseCacheProbeMode::PrefixCycleLoadFailureSwallowed => Err(
                    ModuleLoaderError::new("intentional parse-time cycle load failure"),
                ),
                ParseCacheProbeMode::PrefixCycleLoadPanic => {
                    panic!("intentional parse-time cycle load panic")
                }
                ParseCacheProbeMode::SameNameSuccess
                | ParseCacheProbeMode::SameNameFailure
                | ParseCacheProbeMode::PrefixSuccess
                | ParseCacheProbeMode::PrefixOuterFailure
                | ParseCacheProbeMode::PrefixLoadFailure
                | ParseCacheProbeMode::PrefixLoadFailureSwallowed
                | ParseCacheProbeMode::PrefixLoadPanic
                | ParseCacheProbeMode::CheckerPanic => Err(ModuleLoaderError::new(format!(
                    "unexpected parse-cache load: {name}"
                ))),
            };
        }
        Err(ModuleLoaderError::new(format!(
            "unexpected parse-cache load: {name}"
        )))
    }
}

#[derive(Debug)]
struct RecursiveContextModuleLoader {
    loads: Rc<Cell<usize>>,
    active: Rc<Cell<usize>>,
    maximum_active: Rc<Cell<usize>>,
}

#[derive(Debug)]
struct RecoveringNestedFailureModuleLoader {
    observed_nested_failure: Rc<Cell<bool>>,
    nested_missing_loads: Rc<Cell<usize>>,
}

impl ModuleLoader for RecoveringNestedFailureModuleLoader {
    fn load_with_attributes_in_context(
        &self,
        context: &mut Context,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        match valid_fixture_module_name(normalized_name)?.as_str() {
            "selected.js" => {
                let failed = context.compile_module_with_filename(
                    "import './nested-missing.js';",
                    "nested-failed.js",
                );
                if !matches!(failed, Err(RuntimeError::Exception)) {
                    return Err(ModuleLoaderError::new(
                        "nested failure did not produce a JavaScript exception",
                    ));
                }
                let exception = context
                    .take_exception()
                    .map_err(|error| ModuleLoaderError::new(error.to_string()))?;
                if exception.is_none() {
                    return Err(ModuleLoaderError::new(
                        "nested failure lost its JavaScript exception",
                    ));
                }
                self.observed_nested_failure.set(true);
                let result = context.compile_module_with_filename(
                    "export const answer = 42;",
                    "selected-fallback.js",
                );
                nested_compile_load_result(context, result)
            }
            "nested-missing.js" => {
                self.nested_missing_loads
                    .set(self.nested_missing_loads.get() + 1);
                Err(ModuleLoaderError::new("intentional nested load failure"))
            }
            _ => Err(ModuleLoaderError::new("fixture module is missing")),
        }
    }
}

impl ModuleLoader for RecursiveContextModuleLoader {
    fn load_with_attributes_in_context(
        &self,
        context: &mut Context,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        let next = self.loads.get() + 1;
        self.loads.set(next);
        let previous_depth = self.active.get();
        let depth = previous_depth + 1;
        self.active.set(depth);
        self.maximum_active
            .set(self.maximum_active.get().max(depth));
        let source = format!("import './overflow-{next}.js';");
        let result = context.compile_module_with_filename(&source, &normalized_name);
        self.active.set(previous_depth);
        nested_compile_load_result(context, result)
    }
}

fn assert_rejected_native_error(
    runtime: &Runtime,
    context: &mut Context,
    promise: &ObjectRef,
    expected_name: &'static str,
    expected_message: &'static str,
) {
    let snapshot = promise_snapshot(runtime, promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    let Value::Object(error) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("rejected Promise reason was not an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static(expected_name))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static(expected_message))
    );
}

#[test]
fn compiled_loader_reentry_matches_pinned_quickjs_order_and_context() {
    let runtime = Runtime::new();
    let depth = Rc::new(Cell::new(0));
    let maximum_load_depth = Rc::new(Cell::new(0));
    let events = Rc::new(RefCell::new(Vec::new()));
    let _loader_registration = runtime.set_module_loader(ReentrantCompiledModuleLoader {
        depth: depth.clone(),
        maximum_load_depth: maximum_load_depth.clone(),
        events: events.clone(),
    });
    let mut context = runtime.new_context();
    let expected_id = context.id();
    let expected_realm = context.realm_id();
    context.eval("globalThis.reentryOrder = [];").unwrap();
    let module = context
        .compile_module_with_filename(
            "import './outer.js'; globalThis.reentryOrder.push('entry');",
            "reentry-entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_eq!(depth.get(), 0);
    assert_eq!(maximum_load_depth.get(), 1);
    assert_eq!(
        events.borrow().as_slice(),
        [
            (
                "normalize",
                0,
                "reentry-entry.js|./outer.js".to_owned(),
                expected_id,
                expected_realm,
            ),
            (
                "load",
                0,
                "outer.js".to_owned(),
                expected_id,
                expected_realm,
            ),
            (
                "normalize",
                1,
                "outer.js|./inner.js".to_owned(),
                expected_id,
                expected_realm,
            ),
            (
                "load",
                1,
                "inner.js".to_owned(),
                expected_id,
                expected_realm,
            ),
        ]
    );
    assert_eq!(
        context.eval("JSON.stringify(reentryOrder)").unwrap(),
        Value::String(JsString::from_static("[\"inner\",\"outer\",\"entry\"]"))
    );
}

#[test]
fn parse_time_cache_publication_matches_the_same_name_quickjs_oracle() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::SameNameSuccess);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let outer = context
        .compile_module_with_filename(
            "import { marker as cachedMarker } from './same.js' with { type: 'probe' }; export const marker = 41; globalThis.__cachePublicationResult = cachedMarker + 1;",
            "same.js",
        )
        .unwrap();
    let nested = controls
        .nested_module
        .borrow()
        .as_ref()
        .expect("attribute checker did not retain its nested module")
        .clone();

    assert_ne!(outer, nested);
    assert_eq!(outer.raw.module.0, 0);
    assert_eq!(nested.raw.module.0, 1);
    assert_eq!(controls.checks.get(), 1);
    assert!(controls.loads.borrow().is_empty());
    assert_eq!(
        runtime.module_dependencies(&outer).unwrap(),
        [outer.clone()]
    );
    let Value::Object(promise) = context.execute_module(&outer).unwrap() else {
        panic!("module evaluation did not return a Promise");
    };
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
    assert_script_true(&mut context, "__cachePublicationResult === 42");
    context.link_module(&nested).unwrap();
    assert!(!context.has_exception());
}

#[test]
fn parse_time_cache_failure_rolls_back_both_same_name_constructions() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::SameNameFailure);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert_eq!(
        context.compile_module_with_filename(
            "import './same.js' with { type: 'probe' }; export const marker = 41;",
            "same.js",
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("nested syntax failure did not preserve its Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        context.get_property(&exception, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
    assert_eq!(controls.checks.get(), 1);
    assert!(controls.loads.borrow().is_empty());
    assert!(controls.nested_module.borrow().is_none());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2
    );

    let retry = context
        .compile_module_with_filename("globalThis.__parseCacheRetry = 42;", "same.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 2);
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__parseCacheRetry === 42");
    assert_eq!(controls.checks.get(), 1);
    assert!(!context.has_exception());
}

#[test]
fn parse_time_request_prefix_resolution_matches_the_quickjs_latch() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixSuccess);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let outer = context
        .compile_module_with_filename(
            "import './before.js' with { type: 'probe' }; import './after.js'; export const answer = 42;",
            "outer.js",
        )
        .unwrap();
    let probe = controls
        .nested_module
        .borrow()
        .as_ref()
        .expect("attribute checker did not retain its prefix probe")
        .clone();
    assert_ne!(outer, probe);
    assert_eq!(controls.checks.get(), 1);
    assert_eq!(
        controls.normalizations.borrow().as_slice(),
        [
            ("probe.js".to_owned(), "./outer.js".to_owned()),
            ("outer.js".to_owned(), "./before.js".to_owned()),
        ]
    );
    assert_eq!(controls.loads.borrow().as_slice(), ["before.js"]);
    let outer_dependencies = runtime.module_dependencies(&outer).unwrap();
    assert_eq!(outer_dependencies.len(), 1);
    assert_eq!(
        outer_dependencies[0].name(),
        &JsString::from_static("before.js")
    );
    assert_eq!(
        runtime.module_dependencies(&probe).unwrap(),
        [outer.clone()]
    );
    let record = runtime.module_record(outer.raw).unwrap();
    assert_eq!(record.requested_modules.len(), 2);
    assert!(matches!(
        record.resolution,
        ModuleResolutionState::Resolved(ref dependencies) if dependencies.len() == 1
    ));
    assert_eq!(
        context.link_module(&outer),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert!(runtime.module_record(outer.raw).unwrap().instance.is_none());
    runtime.run_gc().unwrap();
    assert!(!context.has_exception());
}

#[test]
fn link_preflight_classifies_reentrant_construction_states_as_incomplete() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();

    let parsing = runtime
        .publish_parsing_module_record(context.realm, JsString::from_static("still-parsing.js"))
        .unwrap();
    let parsing_handle = runtime.root_module(parsing).unwrap();
    assert_eq!(
        context.link_module(&parsing_handle),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert!(runtime.module_record(parsing).unwrap().instance.is_none());
    runtime.abort_parsing_module(parsing).unwrap();

    let ModuleCompilation::Published(resolving) = runtime
        .compile_module_record_in_realm(
            context.realm,
            "export const answer = 42;",
            &JsString::from_static("still-resolving.js"),
            None,
        )
        .unwrap()
    else {
        panic!("ordinary source unexpectedly threw during compilation");
    };
    runtime
        .transition_module_record(resolving, RawModuleTransition::BeginResolution)
        .unwrap();
    let resolving_handle = runtime.root_module(resolving).unwrap();
    assert_eq!(
        context.link_module(&resolving_handle),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert!(runtime.module_record(resolving).unwrap().instance.is_none());
    runtime
        .transition_module_record(resolving, RawModuleTransition::ResetResolution)
        .unwrap();
    assert!(!context.has_exception());
}

#[test]
fn nested_prefix_load_failure_preserves_the_exception_and_construction_owner() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixLoadFailure);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert_eq!(
        context.compile_module_with_filename(
            "import './before.js' with { type: 'probe' }; import './after.js';",
            "outer.js",
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static(
            "could not load module 'before.js': intentional parse-time prefix load failure"
        )
    );
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
    assert_eq!(controls.checks.get(), 1);
    assert_eq!(controls.loads.borrow().as_slice(), ["before.js"]);
    assert!(controls.nested_module.borrow().is_none());
    assert!(!controls.swallowed_failure.get());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2
    );
    runtime.run_gc().unwrap();

    let retry = context
        .compile_module_with_filename("globalThis.__prefixLoadRetry = 42;", "outer.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 2);
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__prefixLoadRetry === 42");
    assert!(!context.has_exception());
}

#[test]
fn swallowed_nested_prefix_failure_keeps_the_quickjs_one_shot_latch() {
    let runtime = Runtime::new();
    let (loader, controls) =
        ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixLoadFailureSwallowed);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let outer = context
        .compile_module_with_filename(
            "import './before.js' with { type: 'probe' }; import './after.js'; export const answer = 42;",
            "outer.js",
        )
        .unwrap();
    assert!(controls.swallowed_failure.get());
    assert_eq!(controls.checks.get(), 1);
    assert_eq!(controls.loads.borrow().as_slice(), ["before.js"]);
    assert_eq!(
        controls.normalizations.borrow().as_slice(),
        [
            ("probe.js".to_owned(), "./outer.js".to_owned()),
            ("outer.js".to_owned(), "./before.js".to_owned()),
        ]
    );
    assert!(controls.nested_module.borrow().is_none());
    assert!(matches!(
        runtime.module_record(outer.raw).unwrap().resolution,
        ModuleResolutionState::Failed
    ));
    assert_eq!(
        context.link_module(&outer),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert_eq!(
        context.execute_module(&outer),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert!(runtime.module_record(outer.raw).unwrap().instance.is_none());

    let promise = eval_dynamic_import(&mut context, "import('./outer.js')", "entry.js");
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_rejected_native_error(
        &runtime,
        &mut context,
        &promise,
        "InternalError",
        "module resolution is incomplete and cannot be linked safely",
    );
    assert!(!runtime.is_job_pending());
    assert_eq!(controls.loads.borrow().as_slice(), ["before.js"]);
    runtime.run_gc().unwrap();
    assert!(!context.has_exception());
}

#[test]
fn nested_prefix_load_panic_preserves_the_payload_and_recovers() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixLoadPanic);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = context.compile_module_with_filename(
            "import './before.js' with { type: 'probe' }; import './after.js';",
            "outer.js",
        );
    }))
    .expect_err("nested prefix loader panic did not escape");
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
    assert_eq!(message, Some("intentional parse-time prefix load panic"));
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
    assert_eq!(controls.checks.get(), 1);
    assert_eq!(controls.loads.borrow().as_slice(), ["before.js"]);
    assert!(controls.nested_module.borrow().is_none());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2
    );
    runtime.run_gc().unwrap();

    let retry = context
        .compile_module_with_filename("globalThis.__prefixPanicRetry = 42;", "outer.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 2);
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__prefixPanicRetry === 42");
    assert!(!context.has_exception());
}

#[test]
fn resolved_parsing_cycle_is_poisoned_before_failed_probe_rollback() {
    let runtime = Runtime::new();
    let (loader, controls) =
        ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixCycleLoadFailure);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert_eq!(
        context.compile_module_with_filename(
            "import './probe.js' with { type: 'probe' }; import './after.js';",
            "outer.js",
        ),
        Err(RuntimeError::Exception)
    );
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static(
            "could not load module 'missing.js': intentional parse-time cycle load failure"
        )
    );
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
    assert_eq!(controls.checks.get(), 1);
    assert_eq!(controls.loads.borrow().as_slice(), ["missing.js"]);
    assert_eq!(
        controls.normalizations.borrow().as_slice(),
        [
            ("probe.js".to_owned(), "./outer.js".to_owned()),
            ("outer.js".to_owned(), "./probe.js".to_owned()),
            ("probe.js".to_owned(), "./missing.js".to_owned()),
        ]
    );
    assert!(controls.nested_module.borrow().is_none());
    runtime.run_gc().unwrap();

    let retry = context
        .compile_module_with_filename("globalThis.__cycleFailureRetry = 42;", "outer.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 2);
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__cycleFailureRetry === 42");
    assert!(!context.has_exception());
}

#[test]
fn swallowed_cycle_failure_retains_failed_latch_without_dangling_probe() {
    let runtime = Runtime::new();
    let (loader, controls) =
        ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixCycleLoadFailureSwallowed);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let outer = context
        .compile_module_with_filename(
            "import './probe.js' with { type: 'probe' }; import './after.js'; export const answer = 42;",
            "outer.js",
        )
        .unwrap();
    assert!(controls.swallowed_failure.get());
    assert!(controls.nested_module.borrow().is_none());
    assert_eq!(controls.loads.borrow().as_slice(), ["missing.js"]);
    assert!(matches!(
        runtime.module_record(outer.raw).unwrap().resolution,
        ModuleResolutionState::Failed
    ));
    assert_eq!(
        context.link_module(&outer),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert_eq!(
        context.link_module(&outer),
        Err(RuntimeError::IncompleteModuleResolution)
    );
    assert_eq!(controls.loads.borrow().as_slice(), ["missing.js"]);
    assert!(runtime.module_record(outer.raw).unwrap().instance.is_none());
    runtime.run_gc().unwrap();
    assert!(!context.has_exception());
}

#[test]
fn resolved_parsing_cycle_rollback_preserves_the_original_panic() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixCycleLoadPanic);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = context.compile_module_with_filename(
            "import './probe.js' with { type: 'probe' }; import './after.js';",
            "outer.js",
        );
    }))
    .expect_err("cycle loader panic did not escape");
    let message = panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str));
    assert_eq!(message, Some("intentional parse-time cycle load panic"));
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
    assert_eq!(controls.loads.borrow().as_slice(), ["missing.js"]);
    assert!(controls.nested_module.borrow().is_none());
    runtime.run_gc().unwrap();

    let retry = context
        .compile_module_with_filename("globalThis.__cyclePanicRetry = 42;", "outer.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 2);
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__cyclePanicRetry === 42");
    assert!(!context.has_exception());
}

#[test]
fn referenced_failed_parsing_identity_is_aborted_without_quickjs_aba() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixOuterFailure);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert_eq!(
        context.compile_module_with_filename(
            "import './before.js' with { type: 'probe' }; let = ;",
            "outer.js",
        ),
        Err(RuntimeError::Exception)
    );
    let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
        panic!("outer syntax failure did not materialize an Error object");
    };
    let name = runtime.intern_property_key("name").unwrap();
    assert_eq!(
        context.get_property(&exception, &name).unwrap(),
        Value::String(JsString::from_static("SyntaxError"))
    );
    let probe = controls
        .nested_module
        .borrow()
        .as_ref()
        .expect("outer failure lost its escaped probe")
        .clone();
    let outer_raw = RawModuleRef {
        cache: context.realm,
        module: ModuleId(0),
    };
    assert!(matches!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module(outer_raw)
            .unwrap()
            .body,
        ModuleRecordBody::Aborted
    ));
    runtime.run_gc().unwrap();
    assert_eq!(
        context.link_module(&probe),
        Err(RuntimeError::AbortedModule)
    );
    assert_eq!(
        context.link_module(&probe),
        Err(RuntimeError::AbortedModule)
    );
    assert!(runtime.module_record(probe.raw).unwrap().instance.is_none());

    let imported = eval_dynamic_import(&mut context, "import('./probe.js')", "entry.js");
    assert!(runtime.execute_pending_job().unwrap());
    assert_rejected_native_error(
        &runtime,
        &mut context,
        &imported,
        "InternalError",
        "module construction or resolution was rolled back",
    );
    assert!(!runtime.is_job_pending());
    assert!(!context.has_exception());

    let retry = context
        .compile_module_with_filename("globalThis.__parseCacheSafeRetry = 42;", "outer.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 3);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("outer.js"))
            .unwrap(),
        Some(retry.raw)
    );
    assert_eq!(
        context.link_module(&probe),
        Err(RuntimeError::AbortedModule)
    );
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__parseCacheSafeRetry === 42");
    assert_eq!(controls.checks.get(), 1);
    assert_eq!(controls.loads.borrow().as_slice(), ["before.js"]);
    assert!(!context.has_exception());
}

#[test]
fn checker_panic_aborts_the_parsing_slot_and_reentry_depth_recovers() {
    let runtime = Runtime::new();
    let (loader, controls) = ParseCacheProbeLoader::new(ParseCacheProbeMode::CheckerPanic);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    let panic = catch_unwind(AssertUnwindSafe(|| {
        let _ = context.compile_module_with_filename(
            "import './dependency.js' with { type: 'probe' };",
            "panic.js",
        );
    }));
    assert!(panic.is_err());
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        1
    );
    let retry = context
        .compile_module_with_filename("globalThis.__parseCachePanicRetry = 42;", "panic.js")
        .unwrap();
    assert_eq!(retry.raw.module.0, 1);
    context.execute_module(&retry).unwrap();
    assert_script_true(&mut context, "__parseCachePanicRetry === 42");
    assert_eq!(controls.checks.get(), 1);
    assert!(!context.has_exception());
}

#[test]
fn recursive_context_loader_overflow_is_catchable_and_runtime_recovers() {
    std::thread::Builder::new()
        .name("module-loader-reentry-stack-proof".to_owned())
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let loads = Rc::new(Cell::new(0));
            let active = Rc::new(Cell::new(0));
            let maximum_active = Rc::new(Cell::new(0));
            let registration = runtime.set_module_loader(RecursiveContextModuleLoader {
                loads: loads.clone(),
                active: active.clone(),
                maximum_active: maximum_active.clone(),
            });
            let mut context = runtime.new_context();

            assert!(matches!(
                context.compile_module_with_filename(
                    "import './overflow-0.js';",
                    "overflow-entry.js",
                ),
                Err(RuntimeError::Exception)
            ));
            let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
                panic!("module-host overflow did not produce an Error object");
            };
            let name = runtime.intern_property_key("name").unwrap();
            let message = runtime.intern_property_key("message").unwrap();
            assert_eq!(
                context.get_property(&error, &name).unwrap(),
                Value::String(JsString::from_static("InternalError"))
            );
            assert_eq!(
                context.get_property(&error, &message).unwrap(),
                Value::String(JsString::from_static("stack overflow"))
            );
            assert!(loads.get() > 1);
            assert!(maximum_active.get() > 1);
            assert_eq!(active.get(), 0);
            assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
            assert!(!context.has_exception());

            drop(registration);
            runtime.clear_module_loader();
            let (recovery_loader, _, _) = MapModuleLoader::new([(
                "recovery.js",
                "export const answer = 42;",
            )]);
            let _recovery_registration = runtime.set_module_loader(recovery_loader);
            let recovered = context
                .compile_module_with_filename(
                    "import { answer } from './recovery.js'; globalThis.__moduleReentryRecovered = answer;",
                    "recovery-entry.js",
                )
                .unwrap();
            context.execute_module(&recovered).unwrap();
            assert_script_true(&mut context, "__moduleReentryRecovered === 42");
            assert_eq!(context.eval("6 * 7").unwrap(), Value::Int(42));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn failed_nested_compilation_does_not_rollback_suspended_outer_resolution() {
    let runtime = Runtime::new();
    let observed_nested_failure = Rc::new(Cell::new(false));
    let nested_missing_loads = Rc::new(Cell::new(0));
    let _registration = runtime.set_module_loader(RecoveringNestedFailureModuleLoader {
        observed_nested_failure: observed_nested_failure.clone(),
        nested_missing_loads: nested_missing_loads.clone(),
    });
    let mut context = runtime.new_context();

    let entry = context
        .compile_module_with_filename(
            "import { answer } from './selected.js'; globalThis.__nestedFailureRecovered = answer;",
            "nested-recovery-entry.js",
        )
        .unwrap();
    assert!(observed_nested_failure.get());
    assert_eq!(nested_missing_loads.get(), 1);
    assert!(!context.has_exception());
    context.execute_module(&entry).unwrap();
    assert_script_true(&mut context, "__nestedFailureRecovered === 42");
    assert_eq!(runtime.0.module_host_callback_depth.get(), 0);
}

#[test]
fn provisional_parse_gc_preserves_import_meta_properties_until_source_completion() {
    let runtime = Runtime::new();
    let marker = runtime.new_object(None).unwrap();
    let marker_id = marker.object_id();
    let dependency = ModuleLoadResult::SourceTextWithImportMeta {
        source: "import './leaf.js' with { type: 'probe' }; globalThis.__provisionalMetaMarker = import.meta.marker; export const answer = 42;".to_owned(),
        properties: vec![ModuleImportMetaProperty::new(
            JsString::from_static("marker"),
            Value::Object(marker.clone()),
        )],
    };
    let (loader, controls) = ProvisionalImportMetaLoader::new(dependency, marker_id);
    let registration = runtime.set_module_loader(loader);
    drop(marker);
    let mut context = runtime.new_context();

    let module = context
        .compile_module_with_filename(
            "import { answer } from './dependency.js'; globalThis.__provisionalMetaAnswer = answer;",
            "entry.js",
        )
        .unwrap();
    assert_eq!(controls.checks.get(), 1);
    assert!(controls.marker_survived_checker_gc.get());
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__provisionalMetaAnswer === 42");
    let global = context.global_object().unwrap();
    let key = runtime
        .intern_property_key("__provisionalMetaMarker")
        .unwrap();
    let Value::Object(observed) = context.get_property(&global, &key).unwrap() else {
        panic!("completed import.meta lost its provisional marker");
    };
    assert_eq!(observed.object_id(), marker_id);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.object(marker_id).is_ok());
    drop(registration);
}

#[test]
fn failed_provisional_parse_releases_uninstalled_import_meta_properties() {
    let runtime = Runtime::new();
    let marker = runtime.new_object(None).unwrap();
    let marker_id = marker.object_id();
    let dependency = ModuleLoadResult::SourceTextWithImportMeta {
        source: "import './leaf.js' with { type: 'probe' }; let = ;".to_owned(),
        properties: vec![ModuleImportMetaProperty::new(
            JsString::from_static("marker"),
            Value::Object(marker.clone()),
        )],
    };
    let (loader, controls) = ProvisionalImportMetaLoader::new(dependency, marker_id);
    let registration = runtime.set_module_loader(loader);
    drop(marker);
    let mut context = runtime.new_context();

    assert_eq!(
        context.compile_module_with_filename("import './dependency.js';", "entry.js"),
        Err(RuntimeError::Exception)
    );
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(controls.checks.get(), 1);
    assert!(controls.marker_survived_checker_gc.get());
    drop(context);
    drop(registration);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.object(marker_id).is_err());
    assert_eq!(runtime.heap_counts().context_nodes, 0);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}
