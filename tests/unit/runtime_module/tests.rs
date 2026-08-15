use super::*;
use crate::heap::{PromiseData, PromiseState};

#[path = "construction_tests.rs"]
mod construction_tests;

fn assert_eq_implemented<T: Eq>() {}

#[test]
fn module_loader_error_keeps_eq_with_representation_exact_exceptions() {
    assert_eq_implemented::<ModuleLoaderError>();
    let nan = f64::from_bits(0x7ff8_0000_0000_0042);
    assert_eq!(
        ModuleLoaderError::exception(Value::Float(nan)),
        ModuleLoaderError::exception(Value::Float(nan))
    );
    assert_ne!(
        ModuleLoaderError::exception(Value::Float(nan)),
        ModuleLoaderError::exception(Value::Float(f64::NAN))
    );
    assert_ne!(
        ModuleLoaderError::new("JavaScript exception"),
        ModuleLoaderError::exception(Value::String(JsString::from_static("JavaScript exception")))
    );
}

#[test]
fn module_bytecode_and_compiled_load_results_compare_by_module_identity() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let first = context
        .compile_module_with_filename("export const value = 1;", "same.js")
        .unwrap();
    let second = context
        .compile_module_with_filename("export const value = 2;", "same.js")
        .unwrap();

    assert_eq!(first, first.clone());
    assert_ne!(first, second);
    assert_eq!(
        ModuleLoadResult::Compiled(first.clone()),
        ModuleLoadResult::Compiled(first)
    );
}

type SharedLoaderSources = Rc<RefCell<HashMap<String, String>>>;
type SharedLoaderLoads = Rc<RefCell<Vec<String>>>;
type SharedLoaderNormalizations = Rc<RefCell<Vec<(String, String)>>>;
type SharedUtf16LoaderLoads = Rc<RefCell<Vec<Vec<u16>>>>;
type SharedAttributeChecks = Rc<RefCell<Vec<Vec<(String, String)>>>>;
type SharedAttributeLoads = Rc<RefCell<Vec<RecordedAttributeLoad>>>;
type SharedModuleLoadResults = Rc<RefCell<HashMap<String, ModuleLoadResult>>>;
type SharedCallbackContexts = Rc<RefCell<Vec<(&'static str, u64, ContextId)>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
struct RecordedAttributeLoad {
    name: String,
    attributes: Option<Vec<(String, String)>>,
}

#[derive(Clone)]
struct AttributeLoaderControls {
    checks: SharedAttributeChecks,
    loads: SharedAttributeLoads,
    normalizations: SharedLoaderNormalizations,
    reject_checks: Rc<Cell<bool>>,
    fail_loads: Rc<Cell<bool>>,
}

struct AttributeModuleLoader {
    sources: SharedLoaderSources,
    controls: AttributeLoaderControls,
    clear_runtime_on_first_check: Option<Runtime>,
    cleared: Cell<bool>,
}

impl fmt::Debug for AttributeModuleLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttributeModuleLoader")
    }
}

impl AttributeModuleLoader {
    fn new(
        sources: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> (Self, AttributeLoaderControls) {
        let controls = AttributeLoaderControls {
            checks: Rc::new(RefCell::new(Vec::new())),
            loads: Rc::new(RefCell::new(Vec::new())),
            normalizations: Rc::new(RefCell::new(Vec::new())),
            reject_checks: Rc::new(Cell::new(false)),
            fail_loads: Rc::new(Cell::new(false)),
        };
        (
            Self {
                sources: Rc::new(RefCell::new(
                    sources
                        .into_iter()
                        .map(|(name, source)| (name.to_owned(), source.to_owned()))
                        .collect(),
                )),
                controls: controls.clone(),
                clear_runtime_on_first_check: None,
                cleared: Cell::new(false),
            },
            controls,
        )
    }
}

fn recorded_attribute_pairs(attributes: &[ModuleImportAttribute]) -> Vec<(String, String)> {
    attributes
        .iter()
        .map(|attribute| {
            (
                attribute.key.to_utf8_lossy(),
                attribute.value.to_utf8_lossy(),
            )
        })
        .collect()
}

impl ModuleLoader for AttributeModuleLoader {
    fn normalize(
        &self,
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

    fn check_attributes(
        &self,
        attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        self.controls
            .checks
            .borrow_mut()
            .push(recorded_attribute_pairs(attributes));
        if !self.cleared.replace(true)
            && let Some(runtime) = &self.clear_runtime_on_first_check
        {
            runtime.clear_module_loader();
        }
        if self.controls.reject_checks.get() {
            return Err(ModuleLoaderError::new("fixture rejected import attributes"));
        }
        Ok(())
    }

    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.controls
            .loads
            .borrow_mut()
            .push(RecordedAttributeLoad {
                name: normalized_name.clone(),
                attributes: attributes.syntactic().map(recorded_attribute_pairs),
            });
        if self.controls.fail_loads.get() {
            return Err(ModuleLoaderError::new("fixture loader2 failure"));
        }
        self.sources
            .borrow()
            .get(&normalized_name)
            .cloned()
            .map(ModuleLoadResult::SourceText)
            .ok_or_else(|| ModuleLoaderError::new("fixture module is missing"))
    }
}

#[derive(Debug)]
struct JsonModuleLoader {
    modules: SharedModuleLoadResults,
    loads: SharedAttributeLoads,
}

impl JsonModuleLoader {
    fn new(
        modules: impl IntoIterator<Item = (&'static str, ModuleLoadResult)>,
    ) -> (Self, SharedModuleLoadResults, SharedAttributeLoads) {
        let modules = Rc::new(RefCell::new(
            modules
                .into_iter()
                .map(|(name, result)| (name.to_owned(), result))
                .collect(),
        ));
        let loads = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                modules: modules.clone(),
                loads: loads.clone(),
            },
            modules,
            loads,
        )
    }
}

impl ModuleLoader for JsonModuleLoader {
    fn check_attributes(
        &self,
        attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        if attributes.iter().all(|attribute| {
            attribute.key == JsString::from_static("type")
                && attribute.value == JsString::from_static("json")
        }) {
            Ok(())
        } else {
            Err(ModuleLoaderError::new(
                "fixture JSON loader accepts only type: json",
            ))
        }
    }

    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(RecordedAttributeLoad {
            name: normalized_name.clone(),
            attributes: attributes.effective().map(recorded_attribute_pairs),
        });
        self.modules
            .borrow()
            .get(&normalized_name)
            .cloned()
            .ok_or_else(|| ModuleLoaderError::new("fixture module is missing"))
    }
}

fn valid_fixture_module_name(name: &JsString) -> Result<String, ModuleLoaderError> {
    String::from_utf16(&name.utf16_units().collect::<Vec<_>>())
        .map_err(|_| ModuleLoaderError::new("fixture module name is not valid UTF-16"))
}

#[derive(Debug)]
struct MapModuleLoader {
    sources: HashMap<String, String>,
    loads: SharedLoaderLoads,
    normalizations: SharedLoaderNormalizations,
}

#[derive(Debug)]
struct ContextRecordingModuleLoader {
    callbacks: SharedCallbackContexts,
}

impl ContextRecordingModuleLoader {
    fn record(&self, phase: &'static str, context: &Context) {
        self.callbacks
            .borrow_mut()
            .push((phase, context.id(), context.realm_id()));
    }
}

impl ModuleLoader for ContextRecordingModuleLoader {
    fn normalize_in_context(
        &self,
        context: &mut Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.record("normalize", context);
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn check_attributes_in_context(
        &self,
        context: &mut Context,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        self.record("attributes", context);
        Ok(())
    }

    fn load_with_attributes_in_context(
        &self,
        context: &mut Context,
        _normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        self.record("load", context);
        Ok(ModuleLoadResult::SourceText(
            "export const answer = 42;".to_owned(),
        ))
    }
}

#[derive(Debug)]
struct CompiledModuleLoader {
    module: ModuleBytecodeRef,
}

impl ModuleLoader for CompiledModuleLoader {
    fn load_with_attributes(
        &self,
        _normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        Ok(ModuleLoadResult::Compiled(self.module.clone()))
    }
}

impl MapModuleLoader {
    fn new(
        sources: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> (Self, SharedLoaderLoads, SharedLoaderNormalizations) {
        let loads = Rc::new(RefCell::new(Vec::new()));
        let normalizations = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                sources: sources
                    .into_iter()
                    .map(|(name, source)| (name.to_owned(), source.to_owned()))
                    .collect(),
                loads: loads.clone(),
                normalizations: normalizations.clone(),
            },
            loads,
            normalizations,
        )
    }
}

impl ModuleLoader for MapModuleLoader {
    fn normalize(
        &self,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.normalizations
            .borrow_mut()
            .push((base_name.to_utf8_lossy(), specifier.to_utf8_lossy()));
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        self.sources
            .get(&normalized_name)
            .cloned()
            .ok_or_else(|| ModuleLoaderError::new("fixture module is missing"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AbruptLoaderPhase {
    Normalize,
    CheckAttributes,
    Load,
}

#[derive(Debug)]
struct AbruptModuleLoader {
    phase: AbruptLoaderPhase,
    exception: Value,
    failing: Rc<Cell<bool>>,
    loads: SharedLoaderLoads,
}

impl AbruptModuleLoader {
    fn new(
        phase: AbruptLoaderPhase,
        exception: Value,
    ) -> (Self, Rc<Cell<bool>>, SharedLoaderLoads) {
        let failing = Rc::new(Cell::new(true));
        let loads = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                phase,
                exception,
                failing: failing.clone(),
                loads: loads.clone(),
            },
            failing,
            loads,
        )
    }

    fn failure(&self, phase: AbruptLoaderPhase) -> Option<ModuleLoaderError> {
        (self.failing.get() && self.phase == phase)
            .then(|| ModuleLoaderError::exception(self.exception.clone()))
    }
}

impl ModuleLoader for AbruptModuleLoader {
    fn normalize(
        &self,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        if let Some(error) = self.failure(AbruptLoaderPhase::Normalize) {
            return Err(error);
        }
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn check_attributes(
        &self,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        match self.failure(AbruptLoaderPhase::CheckAttributes) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        self.loads
            .borrow_mut()
            .push(valid_fixture_module_name(normalized_name)?);
        if let Some(error) = self.failure(AbruptLoaderPhase::Load) {
            return Err(error);
        }
        Ok("export const answer = 42;".to_owned())
    }
}

#[derive(Debug)]
struct DependencyAttributeAbruptLoader {
    exception: Value,
    failing: Rc<Cell<bool>>,
    loads: SharedLoaderLoads,
}

impl ModuleLoader for DependencyAttributeAbruptLoader {
    fn check_attributes(
        &self,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        if self.failing.get() {
            Err(ModuleLoaderError::exception(self.exception.clone()))
        } else {
            Ok(())
        }
    }

    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        match normalized_name.as_str() {
            "pkg/dependency.js" => Ok(
                "import { answer } from './leaf.js' with { type: 'javascript' }; export { answer };"
                    .to_owned(),
            ),
            "pkg/leaf.js" => Ok("export const answer = 42;".to_owned()),
            _ => Err(ModuleLoaderError::new("fixture module is missing")),
        }
    }
}

#[derive(Debug)]
struct MutableMapModuleLoader {
    sources: SharedLoaderSources,
    loads: SharedLoaderLoads,
}

impl MutableMapModuleLoader {
    fn new(
        sources: impl IntoIterator<Item = (&'static str, &'static str)>,
    ) -> (Self, SharedLoaderSources, SharedLoaderLoads) {
        let sources = Rc::new(RefCell::new(
            sources
                .into_iter()
                .map(|(name, source)| (name.to_owned(), source.to_owned()))
                .collect(),
        ));
        let loads = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                sources: sources.clone(),
                loads: loads.clone(),
            },
            sources,
            loads,
        )
    }
}

impl ModuleLoader for MutableMapModuleLoader {
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        self.sources
            .borrow()
            .get(&normalized_name)
            .cloned()
            .ok_or_else(|| ModuleLoaderError::new("fixture module is missing"))
    }
}

#[derive(Debug)]
struct Utf16RecordingModuleLoader {
    sources: HashMap<Vec<u16>, String>,
    loads: SharedUtf16LoaderLoads,
}

impl Utf16RecordingModuleLoader {
    fn new(
        sources: impl IntoIterator<Item = (Vec<u16>, &'static str)>,
    ) -> (Self, SharedUtf16LoaderLoads) {
        let loads = Rc::new(RefCell::new(Vec::new()));
        (
            Self {
                sources: sources
                    .into_iter()
                    .map(|(name, source)| (name, source.to_owned()))
                    .collect(),
                loads: loads.clone(),
            },
            loads,
        )
    }
}

impl ModuleLoader for Utf16RecordingModuleLoader {
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let name = normalized_name.utf16_units().collect::<Vec<_>>();
        self.loads.borrow_mut().push(name.clone());
        self.sources
            .get(&name)
            .cloned()
            .ok_or_else(|| ModuleLoaderError::new("UTF-16 fixture module is missing"))
    }
}

struct ClearingModuleLoader {
    runtime: Runtime,
    sources: HashMap<String, String>,
    loads: SharedLoaderLoads,
    cleared: Cell<bool>,
}

impl fmt::Debug for ClearingModuleLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ClearingModuleLoader")
    }
}

impl ModuleLoader for ClearingModuleLoader {
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        if !self.cleared.replace(true) {
            self.runtime.clear_module_loader();
        }
        self.sources
            .get(&normalized_name)
            .cloned()
            .ok_or_else(|| ModuleLoaderError::new("fixture module is missing"))
    }
}

struct NormalizeReplacingModuleLoader {
    runtime: Runtime,
    replacement: RefCell<Option<MapModuleLoader>>,
    replacement_registration: RefCell<Option<ModuleLoaderRegistration>>,
    normalizations: SharedLoaderNormalizations,
    loads: SharedLoaderLoads,
}

impl fmt::Debug for NormalizeReplacingModuleLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("NormalizeReplacingModuleLoader")
    }
}

impl ModuleLoader for NormalizeReplacingModuleLoader {
    fn normalize(
        &self,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.normalizations
            .borrow_mut()
            .push((base_name.to_utf8_lossy(), specifier.to_utf8_lossy()));
        if let Some(replacement) = self.replacement.borrow_mut().take() {
            self.replacement_registration
                .borrow_mut()
                .replace(self.runtime.set_module_loader(replacement));
        }
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        self.loads
            .borrow_mut()
            .push(valid_fixture_module_name(normalized_name)?);
        Err(ModuleLoaderError::new(
            "stale normalize loader unexpectedly handled load",
        ))
    }
}

struct AttributeReplacingModuleLoader {
    runtime: Runtime,
    replacement: RefCell<Option<AttributeModuleLoader>>,
    replacement_registration: RefCell<Option<ModuleLoaderRegistration>>,
    checks: Rc<RefCell<Vec<Vec<(String, String)>>>>,
}

impl fmt::Debug for AttributeReplacingModuleLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttributeReplacingModuleLoader")
    }
}

impl ModuleLoader for AttributeReplacingModuleLoader {
    fn check_attributes(
        &self,
        attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        self.checks
            .borrow_mut()
            .push(recorded_attribute_pairs(attributes));
        if let Some(replacement) = self.replacement.borrow_mut().take() {
            self.replacement_registration
                .borrow_mut()
                .replace(self.runtime.set_module_loader(replacement));
        }
        Ok(())
    }

    fn load(&self, _normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        Err(ModuleLoaderError::new(
            "stale attribute checker unexpectedly handled load",
        ))
    }
}

#[derive(Debug)]
struct PanickingModuleLoader;

impl ModuleLoader for PanickingModuleLoader {
    fn load(&self, _normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        panic!("intentional module loader panic")
    }
}

#[derive(Debug)]
struct PanickingClockHost;

impl HostServices for PanickingClockHost {
    fn now_millis(&self) -> i64 {
        panic!("intentional clock panic")
    }

    fn timezone_offset_minutes(&self, _epoch_millis: i64) -> i32 {
        0
    }

    fn random_seed(&self) -> u64 {
        1
    }
}

#[derive(Debug)]
struct CyclicChainModuleLoader {
    module_count: usize,
}

impl ModuleLoader for CyclicChainModuleLoader {
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        let index = normalized_name
            .strip_prefix('m')
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| *index < self.module_count)
            .ok_or_else(|| ModuleLoaderError::new("invalid generated module name"))?;
        let next = if index + 1 == self.module_count {
            0
        } else {
            index + 1
        };
        Ok(format!(
            "import 'm{next}'; globalThis.__deepModuleRuns = (globalThis.__deepModuleRuns || 0) + 1;"
        ))
    }
}

#[derive(Debug)]
struct StarChainModuleLoader {
    module_count: usize,
}

impl ModuleLoader for StarChainModuleLoader {
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        let index = normalized_name
            .strip_prefix('s')
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| *index < self.module_count)
            .ok_or_else(|| ModuleLoaderError::new("invalid generated star module name"))?;
        if index + 1 == self.module_count {
            Ok("export const answer = 42;".to_owned())
        } else {
            Ok(format!("export * from 's{}';", index + 1))
        }
    }
}

struct RuntimeHoldingLoader {
    _runtime: Runtime,
    drops: Rc<Cell<usize>>,
}

impl fmt::Debug for RuntimeHoldingLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("RuntimeHoldingLoader")
    }
}

impl ModuleLoader for RuntimeHoldingLoader {
    fn load(&self, _normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        Err(ModuleLoaderError::new(
            "lifetime probe loader is not callable",
        ))
    }
}

impl Drop for RuntimeHoldingLoader {
    fn drop(&mut self) {
        self.drops.set(self.drops.get() + 1);
    }
}

fn assert_script_true(context: &mut Context, source: &str) {
    assert_eq!(context.eval(source).unwrap(), Value::Bool(true));
}

fn eval_dynamic_import(context: &mut Context, source: &str, filename: &str) -> ObjectRef {
    let Value::Object(promise) = context.eval_with_filename(source, filename).unwrap() else {
        panic!("dynamic import did not return an object");
    };
    promise
}

fn promise_snapshot(runtime: &Runtime, promise: &ObjectRef) -> PromiseData {
    runtime
        .0
        .state
        .borrow()
        .heap
        .promise_snapshot(promise.object_id())
        .unwrap()
}

fn module_evaluation_promise(context: &mut Context, module: &ModuleBytecodeRef) -> ObjectRef {
    let Value::Object(promise) = context.execute_module(module).unwrap() else {
        panic!("module evaluation did not return a Promise");
    };
    promise
}

fn module_evaluation_snapshot(context: &mut Context, module: &ModuleBytecodeRef) -> PromiseData {
    let runtime = context.runtime().clone();
    let promise = module_evaluation_promise(context, module);
    promise_snapshot(&runtime, &promise)
}

fn drain_jobs(runtime: &Runtime) -> usize {
    let mut count = 0;
    loop {
        if !runtime.execute_pending_job().unwrap() {
            return count;
        }
        count += 1;
        assert!(count <= 128, "Promise jobs did not quiesce");
    }
}

fn take_error_message(runtime: &Runtime, context: &mut Context) -> JsString {
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("module failure did not produce an Error object");
    };
    let message_key = runtime.intern_property_key("message").unwrap();
    runtime
        .raw_string_property_for_diagnostics(&error, &message_key)
        .unwrap()
        .expect("module Error object has no string message")
}

fn assert_static_loader_exception(
    phase: AbruptLoaderPhase,
    make_exception: impl FnOnce(&Runtime) -> Value,
    source: &str,
) {
    let runtime = Runtime::new();
    let exception = make_exception(&runtime);
    let (loader, failing, loads) = AbruptModuleLoader::new(phase, exception.clone());
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(context.take_exception().unwrap(), Some(exception));
    assert!(!context.has_exception());

    failing.set(false);
    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__abruptRetry === 42");
    let expected_loads = usize::from(phase == AbruptLoaderPhase::Load) + 1;
    assert_eq!(loads.borrow().len(), expected_loads);
}

#[test]
fn module_loader_exception_values_are_not_wrapped_and_resolution_retries() {
    assert_static_loader_exception(
        AbruptLoaderPhase::Normalize,
        |runtime| Value::Object(runtime.new_object(None).unwrap()),
        "import { answer } from './dependency.js'; globalThis.__abruptRetry = answer;",
    );
    assert_static_loader_exception(
        AbruptLoaderPhase::CheckAttributes,
        |_| Value::Int(42),
        "import { answer } from './dependency.js' with { type: 'javascript' }; globalThis.__abruptRetry = answer;",
    );
    assert_static_loader_exception(
        AbruptLoaderPhase::Load,
        |runtime| {
            Value::Symbol(
                runtime
                    .new_symbol(Some(JsString::from_static("load-reason")))
                    .unwrap(),
            )
        },
        "import { answer } from './dependency.js'; globalThis.__abruptRetry = answer;",
    );
}

#[test]
fn dynamic_import_preserves_module_loader_exception_identity() {
    let runtime = Runtime::new();
    let reason = runtime.new_object(None).unwrap();
    let (loader, _, loads) =
        AbruptModuleLoader::new(AbruptLoaderPhase::Load, Value::Object(reason.clone()));
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let promise = eval_dynamic_import(&mut context, "import('./dependency.js')", "pkg/entry.js");

    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap());
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Object(reason)
    );
    assert_eq!(loads.borrow().as_slice(), ["pkg/dependency.js"]);
    assert!(!context.has_exception());
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_attribute_checker_preserves_exception_identity() {
    let runtime = Runtime::new();
    let reason = runtime.new_object(None).unwrap();
    let (loader, _, loads) = AbruptModuleLoader::new(
        AbruptLoaderPhase::CheckAttributes,
        Value::Object(reason.clone()),
    );
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let promise = eval_dynamic_import(
        &mut context,
        "import('./dependency.js', { with: { type: 'javascript' } })",
        "pkg/entry.js",
    );

    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Object(reason)
    );
    assert!(loads.borrow().is_empty());
    assert!(!context.has_exception());
    assert!(!runtime.is_job_pending());
}

#[test]
fn foreign_runtime_module_loader_exceptions_are_rejected_before_publication() {
    for phase in [
        AbruptLoaderPhase::Normalize,
        AbruptLoaderPhase::CheckAttributes,
        AbruptLoaderPhase::Load,
    ] {
        let runtime = Runtime::new();
        let foreign = Runtime::new().new_object(None).unwrap();
        let (loader, _, _) = AbruptModuleLoader::new(phase, Value::Object(foreign));
        let _registration = runtime.set_module_loader(loader);
        let mut context = runtime.new_context();
        let source = if phase == AbruptLoaderPhase::CheckAttributes {
            "import './dependency.js' with { type: 'javascript' };"
        } else {
            "import './dependency.js';"
        };

        assert!(matches!(
            context.compile_module_with_filename(source, "pkg/entry.js"),
            Err(RuntimeError::WrongRuntime("module loader exception"))
        ));
        assert!(!context.has_exception());
    }
}

#[test]
fn dependency_attribute_exception_rolls_back_the_resolution_graph_for_retry() {
    let runtime = Runtime::new();
    let reason = runtime.new_object(None).unwrap();
    let failing = Rc::new(Cell::new(true));
    let loads = Rc::new(RefCell::new(Vec::new()));
    let loader = DependencyAttributeAbruptLoader {
        exception: Value::Object(reason.clone()),
        failing: failing.clone(),
        loads: loads.clone(),
    };
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = "import './dependency.js'; globalThis.__dependencyAbruptRetry = 42;";

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        context.take_exception().unwrap(),
        Some(Value::Object(reason))
    );
    assert_eq!(loads.borrow().as_slice(), ["pkg/dependency.js"]);

    failing.set(false);
    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__dependencyAbruptRetry === 42");
    assert_eq!(
        loads.borrow().as_slice(),
        ["pkg/dependency.js", "pkg/dependency.js", "pkg/leaf.js"]
    );
}

#[test]
fn dynamic_import_load_and_finish_are_distinct_fifo_jobs_with_gc_roots() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "export const answer = 42; globalThis.__dynamicImportBodyRan = true;",
    )]);
    let _registration = runtime.set_module_loader(loader);

    let promise = eval_dynamic_import(&mut context, "import('./dependency.js')", "pkg/entry.js");
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    runtime.run_gc().unwrap();

    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(loads.borrow().as_slice(), ["pkg/dependency.js"]);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(
        runtime.is_job_pending(),
        "load did not enqueue the finish reaction"
    );
    assert_script_true(&mut context, "globalThis.__dynamicImportBodyRan === true");
    runtime.run_gc().unwrap();

    assert!(runtime.execute_pending_job().unwrap());
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    let Value::Object(namespace) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("dynamic import did not fulfill with a namespace object");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        runtime
            .get_property_in_realm(context.realm, &namespace, &answer)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_waits_for_a_pending_tla_evaluation_and_reuses_it() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .eval(
            r#"
            globalThis.__dynamicTlaLog = [];
            globalThis.__dynamicTlaGate = new Promise(function (resolve) {
                globalThis.__releaseDynamicTlaGate = resolve;
            });
            "#,
        )
        .unwrap();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/wait.js",
        r#"
        globalThis.__dynamicTlaLog.push("start");
        await globalThis.__dynamicTlaGate;
        globalThis.__dynamicTlaLog.push("end");
        export const answer = 42;
        "#,
    )]);
    let _registration = runtime.set_module_loader(loader);

    let first = eval_dynamic_import(
        &mut context,
        "globalThis.__firstWaitingImport = import('./wait.js'); __firstWaitingImport",
        "pkg/entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(loads.borrow().as_slice(), ["pkg/wait.js"]);
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Pending
    );
    assert_script_true(
        &mut context,
        "globalThis.__dynamicTlaLog.join(',') === 'start'",
    );
    assert!(
        !runtime.is_job_pending(),
        "an unresolved TLA gate left a runnable job"
    );

    let second = eval_dynamic_import(
        &mut context,
        "globalThis.__secondWaitingImport = import('./wait.js'); __secondWaitingImport",
        "pkg/entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(loads.borrow().as_slice(), ["pkg/wait.js"]);
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Pending
    );
    assert_eq!(
        promise_snapshot(&runtime, &second).state,
        PromiseState::Pending
    );
    assert!(
        !runtime.is_job_pending(),
        "a cached pending evaluation left a runnable job"
    );

    runtime.run_gc().unwrap();
    context
        .eval("globalThis.__releaseDynamicTlaGate()")
        .unwrap();
    assert!(drain_jobs(&runtime) > 0);

    let first = promise_snapshot(&runtime, &first);
    let second = promise_snapshot(&runtime, &second);
    assert_eq!(first.state, PromiseState::Fulfilled);
    assert_eq!(second.state, PromiseState::Fulfilled);
    let Value::Object(first_namespace) = runtime.root_raw_value(&first.result).unwrap() else {
        panic!("first dynamic import did not fulfill with a namespace object");
    };
    let Value::Object(second_namespace) = runtime.root_raw_value(&second.result).unwrap() else {
        panic!("second dynamic import did not fulfill with a namespace object");
    };
    assert_eq!(first_namespace.object_id(), second_namespace.object_id());
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        runtime
            .get_property_in_realm(context.realm, &first_namespace, &answer)
            .unwrap(),
        Completion::Return(Value::Int(42))
    );
    assert_script_true(
        &mut context,
        "globalThis.__dynamicTlaLog.join(',') === 'start,end'",
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_assimilates_a_namespace_then_export() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, _, _) = MapModuleLoader::new([(
        "thenable.js",
        "export function then(resolve) { resolve(42); }",
    )]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(&mut context, "import('thenable.js')", "entry.js");

    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(
        runtime.is_job_pending(),
        "namespace then was not assimilated"
    );
    assert!(runtime.execute_pending_job().unwrap());
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Int(42)
    );
}

#[test]
fn dynamic_import_internal_then_observes_species_and_ignored_capability() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, _, _) = MapModuleLoader::new([("species.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    context
        .eval(
            r#"
globalThis.__dynamicSpeciesLog = "";
Object.defineProperty(Promise, Symbol.species, {
configurable: true,
get: function () {
    __dynamicSpeciesLog += "species,";
    return function (executor) {
        __dynamicSpeciesLog += "constructor,";
        executor(
            function () { __dynamicSpeciesLog += "resolve,"; },
            function () { __dynamicSpeciesLog += "reject,"; }
        );
        return { ignored: true };
    };
}
});
"#,
        )
        .unwrap();
    let promise = eval_dynamic_import(&mut context, "import('species.js')", "entry.js");
    assert_script_true(&mut context, "__dynamicSpeciesLog === ''");

    assert!(runtime.execute_pending_job().unwrap());
    assert_script_true(
        &mut context,
        "__dynamicSpeciesLog === 'species,constructor,'",
    );
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_script_true(
        &mut context,
        "__dynamicSpeciesLog === 'species,constructor,resolve,'",
    );
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_discards_internal_then_species_abrupt_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, _, _) = MapModuleLoader::new([("species-throw.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    context
        .eval(
            r#"
globalThis.__dynamicSpeciesThrowLog = "";
Object.defineProperty(Promise, Symbol.species, {
configurable: true,
get: function () {
    __dynamicSpeciesThrowLog += "species,";
    throw 73;
}
});
"#,
        )
        .unwrap();
    let promise = eval_dynamic_import(&mut context, "import('species-throw.js')", "entry.js");

    assert!(runtime.execute_pending_job().unwrap());
    assert_script_true(&mut context, "__dynamicSpeciesThrowLog === 'species,'");
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(!runtime.is_job_pending());
    assert!(context.has_exception());
    assert_eq!(context.take_exception().unwrap(), Some(Value::Int(73)));
}

#[test]
fn dynamic_import_attributes_snapshot_descriptors_before_any_value_get() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, controls) =
        AttributeModuleLoader::new([("attributes.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(
        &mut context,
        r#"
globalThis.__dynamicAttributeLog = [];
var attributeSymbol = Symbol("ignored");
var attributeTarget = {};
Object.defineProperty(attributeTarget, "a", {
value: "A", enumerable: true, configurable: true
});
Object.defineProperty(attributeTarget, "b", {
value: "B", enumerable: true, configurable: true
});
Object.defineProperty(attributeTarget, attributeSymbol, {
value: "ignored", enumerable: true, configurable: true
});
var attributeProxy = new Proxy(attributeTarget, {
ownKeys: function (target) {
    __dynamicAttributeLog.push("ownKeys");
    return Reflect.ownKeys(target);
},
getOwnPropertyDescriptor: function (target, key) {
    __dynamicAttributeLog.push("descriptor:" + key);
    return Object.getOwnPropertyDescriptor(target, key);
},
get: function (target, key) {
    __dynamicAttributeLog.push("get:" + key);
    if (key === "a") {
        Object.defineProperty(target, "b", {
            value: "B", enumerable: false, configurable: true
        });
    }
    return target[key];
}
});
import("attributes.js", { with: attributeProxy })
"#,
        "entry.js",
    );
    assert_script_true(
        &mut context,
        "__dynamicAttributeLog.join(',') === 'ownKeys,descriptor:a,descriptor:b,get:a,get:b'",
    );
    assert_eq!(
        controls.checks.borrow().as_slice(),
        [vec![
            ("a".to_owned(), "A".to_owned()),
            ("b".to_owned(), "B".to_owned()),
        ]]
    );
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        controls.loads.borrow().as_slice(),
        [RecordedAttributeLoad {
            name: "attributes.js".to_owned(),
            attributes: Some(vec![
                ("a".to_owned(), "A".to_owned()),
                ("b".to_owned(), "B".to_owned()),
            ]),
        }]
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_empty_attributes_still_reach_checker_and_loader() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, controls) =
        AttributeModuleLoader::new([("empty-attributes.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(
        &mut context,
        "import('empty-attributes.js', { with: {} })",
        "entry.js",
    );

    assert_eq!(controls.checks.borrow().as_slice(), [Vec::new()]);
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        controls.loads.borrow().as_slice(),
        [RecordedAttributeLoad {
            name: "empty-attributes.js".to_owned(),
            attributes: Some(Vec::new()),
        }]
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_rejects_non_string_attribute_values_before_enqueue() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, controls) =
        AttributeModuleLoader::new([("bad-attributes.js", "export const ok = true;")]);
    let _registration = runtime.set_module_loader(loader);
    let promise = eval_dynamic_import(
        &mut context,
        "import('bad-attributes.js', { with: { type: 42 } })",
        "entry.js",
    );

    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Rejected
    );
    assert!(controls.checks.borrow().is_empty());
    assert!(controls.loads.borrow().is_empty());
    assert!(!runtime.is_job_pending());
}

#[test]
fn dynamic_import_load_job_samples_the_current_loader() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (first_loader, first_loads, _) =
        MapModuleLoader::new([("sampled.js", "export const source = 1;")]);
    let _first_registration = runtime.set_module_loader(first_loader);
    let promise = eval_dynamic_import(&mut context, "import('sampled.js')", "entry.js");

    let (second_loader, second_loads, _) =
        MapModuleLoader::new([("sampled.js", "export const source = 2;")]);
    let _second_registration = runtime.set_module_loader(second_loader);
    assert!(runtime.execute_pending_job().unwrap());
    assert!(first_loads.borrow().is_empty());
    assert_eq!(second_loads.borrow().as_slice(), ["sampled.js"]);
    assert!(runtime.execute_pending_job().unwrap());

    let snapshot = promise_snapshot(&runtime, &promise);
    let Value::Object(namespace) = runtime.root_raw_value(&snapshot.result).unwrap() else {
        panic!("sampled dynamic import did not return a namespace");
    };
    let source = runtime.intern_property_key("source").unwrap();
    assert_eq!(
        runtime
            .get_property_in_realm(context.realm, &namespace, &source)
            .unwrap(),
        Completion::Return(Value::Int(2))
    );
}

#[test]
fn dynamic_import_load_samples_replacement_installed_by_normalize() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
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
    let promise = eval_dynamic_import(&mut context, "import('./value.js')", "pkg/entry.js");

    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(initial_normalizations.borrow().len(), 1);
    assert!(initial_loads.borrow().is_empty());
    assert_eq!(replacement_loads.borrow().as_slice(), &["pkg/value.js"]);
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
}

#[test]
fn dynamic_import_resolution_failure_retries_the_acyclic_source_graph() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([("pkg/a.js", "import './missing.js';")]);
    let _loader_registration = runtime.set_module_loader(loader);

    for _ in 0..2 {
        let promise = eval_dynamic_import(&mut context, "import('./a.js')", "pkg/entry.js");
        assert!(runtime.execute_pending_job().unwrap());
        assert_eq!(
            promise_snapshot(&runtime, &promise).state,
            PromiseState::Rejected
        );
        assert!(!runtime.is_job_pending());
    }

    assert_eq!(
        loads.borrow().as_slice(),
        &["pkg/a.js", "pkg/missing.js", "pkg/a.js", "pkg/missing.js"]
    );
}

#[test]
fn dynamic_import_reuses_cycle_root_rejection_promise_and_tracker_history() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([
        ("cycle-a.js", "import 'cycle-b.js'; export const a = 1;"),
        ("cycle-b.js", "import 'cycle-a.js'; throw 42;"),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        captured.borrow_mut().push((
            event.is_handled(),
            event.promise().object_id(),
            event.reason().clone(),
        ));
    });

    let first = eval_dynamic_import(
        &mut context,
        "globalThis.__cycleFirst = import('cycle-a.js'); __cycleFirst.catch(function () {}); __cycleFirst",
        "entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap());
    {
        let events = events.borrow();
        assert_eq!(events.len(), 3);
        assert!(!events[0].0);
        assert!(!events[1].0);
        assert!(events[2].0);
        assert_ne!(events[0].1, events[1].1);
        assert_eq!(events[1].1, events[2].1);
        assert_eq!(events[0].2, Value::Int(42));
        assert_eq!(events[1].2, Value::Int(42));
        assert_eq!(events[2].2, Value::Int(42));
    }
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Rejected
    );
    assert!(
        runtime.execute_pending_job().unwrap(),
        "first catch reaction was missing"
    );
    assert!(!runtime.is_job_pending());

    let (cycle_a, cycle_b, root_promise) = {
        let state = runtime.0.state.borrow();
        let cycle_a = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("cycle-a.js"))
            .unwrap()
            .unwrap();
        let cycle_b = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("cycle-b.js"))
            .unwrap()
            .unwrap();
        let a = state.heap.loaded_module(cycle_a).unwrap();
        let b = state.heap.loaded_module(cycle_b).unwrap();
        assert_eq!(a.evaluation_cycle_root, Some(cycle_a.module));
        assert_eq!(b.evaluation_cycle_root, Some(cycle_a.module));
        assert!(b.evaluation_promise.is_none());
        (cycle_a, cycle_b, a.evaluation_promise.unwrap())
    };
    assert_ne!(cycle_a, cycle_b);

    let second = eval_dynamic_import(
        &mut context,
        "globalThis.__cycleSecond = import('cycle-b.js'); __cycleSecond.catch(function () {}); __cycleSecond",
        "entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &second).state,
        PromiseState::Rejected
    );
    assert!(!runtime.is_job_pending());
    assert_eq!(
        events.borrow().len(),
        3,
        "cached handled rejection retracked"
    );
    assert_eq!(loads.borrow().len(), 2, "cycle cache reloaded source text");
    assert_eq!(
        runtime.module_record(cycle_a).unwrap().evaluation_promise,
        Some(root_promise)
    );
    assert!(
        runtime
            .module_record(cycle_b)
            .unwrap()
            .evaluation_promise
            .is_none()
    );
}

#[test]
fn dynamic_import_successful_cycle_reuses_one_evaluation_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let (loader, loads, _) = MapModuleLoader::new([
        (
            "ok-cycle-a.js",
            "import 'ok-cycle-b.js'; export const a = 1;",
        ),
        (
            "ok-cycle-b.js",
            "import 'ok-cycle-a.js'; export const b = 2;",
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);

    let first = eval_dynamic_import(&mut context, "import('ok-cycle-a.js')", "entry.js");
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &first).state,
        PromiseState::Fulfilled
    );
    let (a, b, root_promise) = {
        let state = runtime.0.state.borrow();
        let a = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("ok-cycle-a.js"))
            .unwrap()
            .unwrap();
        let b = state
            .heap
            .first_loaded_module(context.realm, &JsString::from_static("ok-cycle-b.js"))
            .unwrap()
            .unwrap();
        let a_record = state.heap.loaded_module(a).unwrap();
        let b_record = state.heap.loaded_module(b).unwrap();
        assert_eq!(a_record.evaluation_cycle_root, Some(a.module));
        assert_eq!(b_record.evaluation_cycle_root, Some(a.module));
        assert!(b_record.evaluation_promise.is_none());
        (a, b, a_record.evaluation_promise.unwrap())
    };

    let second = eval_dynamic_import(&mut context, "import('ok-cycle-b.js')", "entry.js");
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &second).state,
        PromiseState::Fulfilled
    );
    assert_eq!(loads.borrow().len(), 2);
    assert_eq!(
        runtime.module_record(a).unwrap().evaluation_promise,
        Some(root_promise)
    );
    assert!(
        runtime
            .module_record(b)
            .unwrap()
            .evaluation_promise
            .is_none()
    );
}

#[test]
fn static_and_dynamic_entrypoints_share_the_cached_evaluation_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let static_module = context
        .compile_module_with_filename("export const value = 42;", "pkg/static.js")
        .unwrap();
    let static_result = module_evaluation_promise(&mut context, &static_module);
    let static_promise = runtime
        .module_record(static_module.raw)
        .unwrap()
        .evaluation_promise
        .unwrap();
    assert_eq!(static_result.object_id(), static_promise);

    let imported = eval_dynamic_import(&mut context, "import('./static.js')", "pkg/entry.js");
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &imported).state,
        PromiseState::Fulfilled
    );
    assert_eq!(
        runtime
            .module_record(static_module.raw)
            .unwrap()
            .evaluation_promise,
        Some(static_promise)
    );

    let (loader, _, _) =
        MapModuleLoader::new([("pkg/dynamic-first.js", "export const value = 7;")]);
    let _registration = runtime.set_module_loader(loader);
    let dynamic_first =
        eval_dynamic_import(&mut context, "import('./dynamic-first.js')", "pkg/entry.js");
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &dynamic_first).state,
        PromiseState::Fulfilled
    );
    let raw = runtime
        .0
        .state
        .borrow()
        .heap
        .first_loaded_module(
            context.realm,
            &JsString::from_static("pkg/dynamic-first.js"),
        )
        .unwrap()
        .unwrap();
    let dynamic_promise = runtime
        .module_record(raw)
        .unwrap()
        .evaluation_promise
        .unwrap();
    let handle = runtime.root_module(raw).unwrap();
    assert_eq!(
        module_evaluation_promise(&mut context, &handle).object_id(),
        dynamic_promise
    );
    assert_eq!(
        runtime.module_record(raw).unwrap().evaluation_promise,
        Some(dynamic_promise)
    );
}

#[test]
fn static_throw_then_cached_dynamic_import_preserves_both_promise_histories() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval("globalThis.__sharedModuleReason = {}; __sharedModuleReason")
        .unwrap();
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        captured.borrow_mut().push((
            event.is_handled(),
            event.promise().object_id(),
            event.reason().clone(),
        ));
    });

    let module = context
        .compile_module_with_filename(
            "throw globalThis.__sharedModuleReason;",
            "pkg/shared-throw.js",
        )
        .unwrap();
    let evaluation = module_evaluation_promise(&mut context, &module);
    let evaluation_snapshot = promise_snapshot(&runtime, &evaluation);
    assert_eq!(evaluation_snapshot.state, PromiseState::Rejected);
    assert_eq!(
        runtime.root_raw_value(&evaluation_snapshot.result).unwrap(),
        reason
    );
    {
        let events = events.borrow();
        assert_eq!(events.len(), 2);
        assert!(!events[0].0, "module-body Promise was already handled");
        assert!(!events[1].0, "evaluation Promise was already handled");
        assert_ne!(events[0].1, events[1].1);
        assert_eq!(events[0].2, reason);
        assert_eq!(events[1].2, reason);
    }
    assert_eq!(context.take_exception().unwrap(), None);
    assert_eq!(events.borrow().len(), 2);

    let imported = eval_dynamic_import(
        &mut context,
        "globalThis.__cachedThrowImport = import('./shared-throw.js'); __cachedThrowImport.catch(function () {}); __cachedThrowImport",
        "pkg/entry.js",
    );
    assert!(runtime.execute_pending_job().unwrap());
    {
        let events = events.borrow();
        assert_eq!(events.len(), 3);
        assert!(events[2].0);
        assert_eq!(events[2].1, events[1].1);
        assert_ne!(events[2].1, events[0].1);
        assert_eq!(events[2].2, reason);
    }
    assert!(runtime.execute_pending_job().unwrap());
    assert!(runtime.execute_pending_job().unwrap());
    assert_eq!(
        promise_snapshot(&runtime, &imported).state,
        PromiseState::Rejected
    );
    assert!(!runtime.is_job_pending());
    assert_eq!(events.borrow().len(), 3);
}

#[test]
fn default_module_normalizer_matches_quickjs_leading_dot_rules() {
    for (base, specifier, expected) in [
        ("pkg/entry.js", "bare", "bare"),
        ("pkg/entry.js", "./dep.js", "pkg/dep.js"),
        ("pkg/deep/entry.js", "../dep.js", "pkg/dep.js"),
        ("pkg/deep/entry.js", "../../dep.js", "dep.js"),
        ("entry.js", "../dep.js", "../dep.js"),
        ("pkg/entry.js", ".hidden", "pkg/.hidden"),
        ("./entry.js", "../dep.js", "./../dep.js"),
        ("../entry.js", "../dep.js", "../../dep.js"),
    ] {
        let base = JsString::try_from_utf8(base).unwrap();
        let specifier = JsString::try_from_utf8(specifier).unwrap();
        assert_eq!(
            default_module_normalize_name(&base, &specifier)
                .unwrap()
                .to_utf8_lossy(),
            expected
        );
    }
}

#[test]
fn import_attribute_states_preserve_syntax_and_fold_empty_for_hosts() {
    let absent = ModuleImportAttributes::Absent;
    let empty = ModuleImportAttributes::Present(Vec::new().into_boxed_slice());
    let present = ModuleImportAttributes::Present(
        vec![ModuleImportAttribute {
            key: JsString::from_static("type"),
            value: JsString::from_static("javascript"),
        }]
        .into_boxed_slice(),
    );

    assert!(absent.syntactic().is_none());
    assert!(absent.effective().is_none());
    assert_eq!(empty.syntactic(), Some([].as_slice()));
    assert!(empty.effective().is_none());
    assert_eq!(
        present.effective().map(recorded_attribute_pairs).unwrap(),
        vec![("type".to_owned(), "javascript".to_owned())]
    );
}

#[test]
fn loader2_observes_effective_attributes_only_on_cache_miss() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context
        .compile_module_with_filename("export const value = 39;", "pkg/cached.js")
        .unwrap();
    let (loader, controls) = AttributeModuleLoader::new([
        ("pkg/shared.js", "export const value = 0;"),
        ("pkg/absent.js", "export const value = 1;"),
        ("pkg/empty.js", "export const value = 1;"),
        ("pkg/present.js", "export const value = 1;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            r#"
            import { value as cached } from "./cached.js" with { cache: "hit" };
            import "./shared.js" with { flavor: "first" };
            import "./shared.js" with { flavor: "second" };
            import { value as absent } from "./absent.js";
            import { value as empty } from "./empty.js" with {};
            import { value as present } from "./present.js" with {
                first: "one",
                second: "two",
            };
            globalThis.__attributeLoader2 = cached + absent + empty + present;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeLoader2 === 42");
    assert_eq!(
        &*controls.checks.borrow(),
        &[
            vec![("cache".to_owned(), "hit".to_owned())],
            vec![("flavor".to_owned(), "first".to_owned())],
            vec![("flavor".to_owned(), "second".to_owned())],
            vec![
                ("first".to_owned(), "one".to_owned()),
                ("second".to_owned(), "two".to_owned()),
            ],
        ]
    );
    assert_eq!(
        &*controls.loads.borrow(),
        &[
            RecordedAttributeLoad {
                name: "pkg/shared.js".to_owned(),
                attributes: Some(vec![("flavor".to_owned(), "first".to_owned())]),
            },
            RecordedAttributeLoad {
                name: "pkg/absent.js".to_owned(),
                attributes: None,
            },
            RecordedAttributeLoad {
                name: "pkg/empty.js".to_owned(),
                attributes: None,
            },
            RecordedAttributeLoad {
                name: "pkg/present.js".to_owned(),
                attributes: Some(vec![
                    ("first".to_owned(), "one".to_owned()),
                    ("second".to_owned(), "two".to_owned()),
                ]),
            },
        ]
    );
    assert_eq!(controls.normalizations.borrow().len(), 6);
}

#[test]
fn attribute_check_precedes_following_syntax_and_all_resolution_callbacks() {
    let runtime = Runtime::new();
    let (loader, controls) =
        AttributeModuleLoader::new([("pkg/dependency.js", "export const value = 42;")]);
    controls.reject_checks.set(true);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(
            r#"import "./dependency.js" with { unsupported: "x" }; let = ;"#,
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("attribute check failure did not materialize a TypeError");
    };
    let name = runtime.intern_property_key("name").unwrap();
    let message = runtime.intern_property_key("message").unwrap();
    assert_eq!(
        context.get_property(&error, &name).unwrap(),
        Value::String(JsString::from_static("TypeError"))
    );
    assert_eq!(
        context.get_property(&error, &message).unwrap(),
        Value::String(JsString::from_static("fixture rejected import attributes"))
    );
    assert_eq!(
        &*controls.checks.borrow(),
        &[vec![("unsupported".to_owned(), "x".to_owned())]]
    );
    assert!(controls.normalizations.borrow().is_empty());
    assert!(controls.loads.borrow().is_empty());

    controls.reject_checks.set(false);
    let module = context
        .compile_module_with_filename(
            r#"
            import { value } from "./dependency.js" with { type: "javascript" };
            globalThis.__attributeCheckRetry = value;
            "#,
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeCheckRetry === 42");
    assert_eq!(controls.loads.borrow().len(), 1);
}

#[test]
fn dependency_attribute_check_failure_rolls_back_graph_for_retry() {
    let runtime = Runtime::new();
    let (loader, controls) = AttributeModuleLoader::new([
        (
            "pkg/a.js",
            r#"
            import { value } from "./dependency.js" with { type: "javascript" };
            export const answer = value + 1;
            "#,
        ),
        ("pkg/dependency.js", "export const value = 41;"),
    ]);
    controls.reject_checks.set(true);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(
            "import { answer } from './a.js'; export { answer };",
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(
        controls
            .loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/a.js"]
    );

    controls.reject_checks.set(false);
    let module = context
        .compile_module_with_filename(
            "import { answer } from './a.js'; globalThis.__attributeRollback = answer;",
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__attributeRollback === 42");
    assert_eq!(
        controls
            .loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/a.js", "pkg/a.js", "pkg/dependency.js"]
    );
    assert_eq!(controls.checks.borrow().len(), 2);
}

#[test]
fn loader2_failure_unpublishes_root_and_retries_with_same_attributes() {
    let runtime = Runtime::new();
    let (loader, controls) =
        AttributeModuleLoader::new([("pkg/dependency.js", "export const value = 42;")]);
    controls.fail_loads.set(true);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = r#"
        import { value } from "./dependency.js" with { type: "javascript" };
        globalThis.__loader2Retry = value;
    "#;

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("could not load module 'pkg/dependency.js': fixture loader2 failure")
    );
    controls.fail_loads.set(false);

    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__loader2Retry === 42");
    assert_eq!(controls.checks.borrow().len(), 2);
    assert_eq!(controls.loads.borrow().len(), 2);
    assert!(controls.loads.borrow().iter().all(
        |load| load.attributes == Some(vec![("type".to_owned(), "javascript".to_owned())])
    ));
}

#[test]
fn json_module_default_export_is_cached_by_normalized_name_and_keeps_json_semantics() {
    let runtime = Runtime::new();
    let (loader, _, loads) = JsonModuleLoader::new([
        (
            "pkg/value.json",
            ModuleLoadResult::JsonText(
                r#"{"answer":40,"nested":[2],"__proto__":{"polluted":true}}"#.to_owned(),
            ),
        ),
        (
            "pkg/indirect.js",
            ModuleLoadResult::SourceText(
                "export { default } from './value.json' with { type: 'json' };".to_owned(),
            ),
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import first from "./value.json" with { type: "json" };
            import { default as second } from "./value.json" with { type: "json" };
            import indirect from "./indirect.js";
            import * as namespace from "./value.json" with { type: "json" };
            const proto = Object.getOwnPropertyDescriptor(first, "__proto__");
            globalThis.__jsonModuleParity =
                first === second && second === indirect && namespace.default === first &&
                Reflect.ownKeys(namespace).length === 2 &&
                Object.keys(namespace).join(",") === "default" &&
                namespace[Symbol.toStringTag] === "Module" &&
                first.answer + first.nested[0] === 42 &&
                Object.getPrototypeOf(first) === Object.prototype &&
                Object.isExtensible(first) &&
                proto.value.polluted === true && proto.enumerable &&
                proto.writable && proto.configurable;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__jsonModuleParity === true");
    assert_eq!(
        loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/value.json", "pkg/indirect.js"]
    );
    assert_eq!(
        loads.borrow()[0].attributes,
        Some(vec![("type".to_owned(), "json".to_owned())])
    );
}

#[test]
fn json_module_live_cell_is_undefined_after_link_and_initialized_during_evaluation() {
    let runtime = Runtime::new();
    let (loader, _, _) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText(r#"{"answer":42}"#.to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import value from "./value.json" with { type: "json" };
            globalThis.__jsonEvaluationValue = value.answer;
            "#,
            "pkg/entry.js",
        )
        .unwrap();
    let dependency = runtime.module_dependencies(&module).unwrap().remove(0);

    context.link_module(&module).unwrap();
    let namespace = runtime
        .get_module_namespace(&dependency, context.realm)
        .unwrap();
    let default = runtime.intern_property_key("default").unwrap();
    assert_eq!(
        context.get_property(&namespace, &default).unwrap(),
        Value::Undefined
    );

    context.execute_module(&module).unwrap();
    let Value::Object(first) = context.get_property(&namespace, &default).unwrap() else {
        panic!("evaluated JSON module default was not the parsed object");
    };
    let answer = runtime.intern_property_key("answer").unwrap();
    assert_eq!(
        context.get_property(&first, &answer).unwrap(),
        Value::Int(42)
    );
    assert_script_true(&mut context, "__jsonEvaluationValue === 42");

    context.execute_module(&module).unwrap();
    assert_eq!(
        context.get_property(&namespace, &default).unwrap(),
        Value::Object(first)
    );
}

#[test]
fn json_module_named_import_fails_during_retryable_link() {
    let runtime = Runtime::new();
    let (loader, _, _) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText(r#"{"name":"not an export"}"#.to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { name } from "./value.json" with { type: "json" };
            globalThis.__jsonNamedImportBody = name;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert_eq!(
            take_error_message(&runtime, &mut context),
            JsString::from_static("Could not find export 'name' in module 'pkg/value.json'")
        );
    }
    assert_script_true(&mut context, "typeof __jsonNamedImportBody === 'undefined'");
}

#[test]
fn invalid_json_module_reports_fixture_location_and_rolls_back_for_retry() {
    let runtime = Runtime::new();
    let (loader, modules, loads) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText("{\n  notJson: 0\n}\n".to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let source = r#"
        import value from "./value.json" with { type: "json" };
        globalThis.__jsonRetry = value.answer;
    "#;

    assert!(matches!(
        context.compile_module_with_filename(source, "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
        panic!("invalid JSON module did not throw a SyntaxError");
    };
    for (name, expected) in [
        (
            "message",
            Value::String(JsString::from_static("expecting property name")),
        ),
        (
            "fileName",
            Value::String(JsString::from_static("pkg/value.json")),
        ),
        ("lineNumber", Value::Int(2)),
        ("columnNumber", Value::Int(3)),
    ] {
        let key = runtime.intern_property_key(name).unwrap();
        assert_eq!(context.get_property(&error, &key).unwrap(), expected);
    }

    modules.borrow_mut().insert(
        "pkg/value.json".to_owned(),
        ModuleLoadResult::JsonText(r#"{"answer":42}"#.to_owned()),
    );
    let module = context
        .compile_module_with_filename(source, "pkg/entry.js")
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__jsonRetry === 42");
    assert_eq!(
        loads
            .borrow()
            .iter()
            .map(|load| load.name.as_str())
            .collect::<Vec<_>>(),
        vec!["pkg/value.json", "pkg/value.json"]
    );
}

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

#[test]
fn failed_resolution_unpublishes_the_root_from_the_context_cache() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './missing.js';", "pkg/shared.js",),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(&*loads.borrow(), &["pkg/missing.js"]);

    context
        .compile_module_with_filename("export const value = 42;", "pkg/shared.js")
        .unwrap();
    let importer = context
        .compile_module_with_filename(
            "import { value } from './shared.js'; globalThis.__recoveredModule = value;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__recoveredModule === 42");
    assert_eq!(&*loads.borrow(), &["pkg/missing.js"]);
}

#[test]
fn failed_resolution_leaves_a_permanent_module_cache_tombstone() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './missing.js';", "pkg/failed.js"),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap();
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

    let replacement = context
        .compile_module_with_filename("export const ok = true;", "pkg/failed.js")
        .unwrap();
    assert_eq!(replacement.raw.module.0, 1);
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
}

#[test]
fn escaped_module_handle_reports_aborted_after_resolution_rollback() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([]);
    let _registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let name = JsString::from_static("pkg/aborted-entry.js");
    let ModuleCompilation::Published(raw) = runtime
        .compile_module_record_in_realm(context.realm, "import './missing.js';", &name, None)
        .unwrap()
    else {
        panic!("ordinary source unexpectedly threw during compilation");
    };
    let handle = runtime.root_module(raw).unwrap();

    assert!(matches!(
        runtime.resolve_module_graph(context.realm, raw),
        Err(RuntimeError::Exception)
    ));
    context.take_exception().unwrap();
    assert_eq!(handle.name(), &name);
    assert_eq!(handle, handle.clone());
    assert_eq!(
        context.get_module_import_meta(&handle),
        Err(RuntimeError::AbortedModule)
    );
    assert_eq!(
        context.link_module(&handle),
        Err(RuntimeError::AbortedModule)
    );
    assert_eq!(
        context.execute_module(&handle),
        Err(RuntimeError::AbortedModule)
    );
}

#[test]
fn failed_resolution_rolls_back_every_active_loaded_module() {
    let runtime = Runtime::new();
    let (loader, sources, loads) = MutableMapModuleLoader::new([
        ("pkg/a.js", "import './b.js'; export const a = 1;"),
        ("pkg/b.js", "import './missing.js'; export const b = 1;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './a.js';", "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(
        &*loads.borrow(),
        &["pkg/a.js", "pkg/b.js", "pkg/missing.js"]
    );

    sources
        .borrow_mut()
        .insert("pkg/b.js".to_owned(), "export const b = 42;".to_owned());
    let importer = context
        .compile_module_with_filename(
            "import { b } from './b.js'; globalThis.__activeRollback = b;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__activeRollback === 42");
    assert_eq!(
        &*loads.borrow(),
        &["pkg/a.js", "pkg/b.js", "pkg/missing.js", "pkg/b.js"]
    );
}

#[test]
fn failed_resolution_preserves_an_independently_completed_dependency() {
    let runtime = Runtime::new();
    let (loader, sources, loads) =
        MutableMapModuleLoader::new([("pkg/complete.js", "export const value = 42;")]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename(
            "import './complete.js'; import './missing.js';",
            "pkg/entry.js",
        ),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    sources.borrow_mut().insert(
        "pkg/complete.js".to_owned(),
        "export const value = 99;".to_owned(),
    );

    let importer = context
        .compile_module_with_filename(
            "import { value } from './complete.js'; globalThis.__completedCache = value;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__completedCache === 42");
    assert_eq!(&*loads.borrow(), &["pkg/complete.js", "pkg/missing.js"]);
}

#[test]
fn failed_resolution_unpublishes_cycle_members_that_reference_the_root() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([
        ("pkg/a.js", "export const a = 41;"),
        (
            "pkg/b.js",
            "import { a } from './a.js'; export const b = a + 1;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context
            .compile_module_with_filename("import './b.js'; import './missing.js';", "pkg/a.js",),
        Err(RuntimeError::Exception)
    ));
    assert!(matches!(
        context.take_exception().unwrap(),
        Some(Value::Object(_))
    ));
    assert_eq!(&*loads.borrow(), &["pkg/b.js", "pkg/missing.js"]);

    let importer = context
        .compile_module_with_filename(
            "import { b } from './b.js'; globalThis.__cycleRecovered = b;",
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&importer).unwrap();
    assert_script_true(&mut context, "__cycleRecovered === 42");
    assert_eq!(
        &*loads.borrow(),
        &["pkg/b.js", "pkg/missing.js", "pkg/b.js", "pkg/a.js"]
    );
}

#[test]
fn module_loader_cache_cycles_and_live_cells_follow_quickjs_order() {
    let runtime = Runtime::new();
    let (loader, loads, normalizations) = MapModuleLoader::new([
        (
            "pkg/a.js",
            r#"
            import { seen, read } from "./b.js";
            export { read };
            export let value = 1;
            export function bump() { value = 42; }
            globalThis.__aSeen = seen;
            globalThis.__aRead = read();
            "#,
        ),
        (
            "pkg/b.js",
            r#"
            import { value } from "./a.js";
            export var seen = 7;
            export function read() { return value; }
            globalThis.__bRuns = (globalThis.__bRuns || 0) + 1;
            "#,
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let entry = context
        .compile_module_with_filename(
            r#"
            import "./a.js";
            import { value, bump, read } from "./a.js";
            globalThis.__before = value;
            bump();
            globalThis.__after = value;
            globalThis.__afterViaCycle = read();
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(&*loads.borrow(), &["pkg/a.js", "pkg/b.js"]);
    assert_eq!(normalizations.borrow().len(), 4);
    let first = module_evaluation_promise(&mut context, &entry);
    assert_script_true(
        &mut context,
        r#"
        __aSeen === 7 && __aRead === 1 && __bRuns === 1 &&
        __before === 1 && __after === 42 && __afterViaCycle === 42
        "#,
    );
    let second = module_evaluation_promise(&mut context, &entry);
    assert_eq!(first.object_id(), second.object_id());
    assert_script_true(&mut context, "__bRuns === 1");
}

#[test]
fn default_import_clauses_share_the_exporters_live_cell() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([(
        "pkg/exporter.js",
        r#"
        export let value = 1;
        export { value as default };
        export function update() { value = 42; }
        "#,
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import onlyDefault from "./exporter.js";
            import defaultWithNamed, { update } from "./exporter.js";
            import defaultWithNamespace, * as namespace from "./exporter.js";
            globalThis.__defaultImportBefore =
                onlyDefault === 1 &&
                defaultWithNamed === 1 &&
                defaultWithNamespace === 1 &&
                namespace.default === 1;
            try {
                defaultWithNamed = 2;
            } catch (error) {
                globalThis.__defaultImportReadOnly = true;
            }
            update();
            globalThis.__defaultImportAfter =
                onlyDefault === 42 &&
                defaultWithNamed === 42 &&
                defaultWithNamespace === 42 &&
                namespace.default === 42;
            "#,
            "pkg/importer.js",
        )
        .unwrap();

    assert_eq!(&*loads.borrow(), &["pkg/exporter.js"]);
    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __defaultImportBefore === true &&
        __defaultImportReadOnly === true &&
        __defaultImportAfter === true
        "#,
    );
}

#[test]
fn default_function_declarations_are_hoisted_named_and_live_through_self_imports() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let anonymous = context
        .compile_module_with_filename(
            r#"
            import current from "./anonymous.js";
            const descriptor = Object.getOwnPropertyDescriptor(current, "name");
            globalThis.__anonymousDefault =
                current() === 23 && current.name === "default" &&
                descriptor.value === "default" &&
                descriptor.writable === false &&
                descriptor.enumerable === false &&
                descriptor.configurable === true;
            export default function () { return 23; }
            "#,
            "pkg/anonymous.js",
        )
        .unwrap();
    context.execute_module(&anonymous).unwrap();

    let named = context
        .compile_module_with_filename(
            r#"
            import current from "./named.js";
            export default function named() { return 23; }
            globalThis.__namedDefaultBefore =
                current === named && current() === 23 && current.name === "named";
            named = function replacement() { return 42; };
            globalThis.__namedDefaultAfter =
                current === named && current() === 42 && current.name === "replacement";
            "#,
            "pkg/named.js",
        )
        .unwrap();
    context.execute_module(&named).unwrap();

    assert_script_true(
        &mut context,
        r#"
        __anonymousDefault === true &&
        __namedDefaultBefore === true &&
        __namedDefaultAfter === true
        "#,
    );
}

#[test]
fn anonymous_default_generator_and_async_declarations_receive_the_default_name() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let generator = context
        .compile_module_with_filename(
            r#"
            import current from "./generator.js";
            globalThis.__defaultGenerator =
                current.name === "default" && current().next().value === 42;
            export default function* () { yield 42; }
            "#,
            "pkg/generator.js",
        )
        .unwrap();
    context.execute_module(&generator).unwrap();

    let async_function = context
        .compile_module_with_filename(
            r#"
            import current from "./async-function.js";
            globalThis.__defaultAsyncFunction = current.name === "default";
            export default async function () { return 42; }
            "#,
            "pkg/async-function.js",
        )
        .unwrap();
    context.execute_module(&async_function).unwrap();

    let async_generator = context
        .compile_module_with_filename(
            r#"
            import current from "./async-generator.js";
            globalThis.__defaultAsyncGenerator = current.name === "default";
            export default async function* () { yield 42; }
            "#,
            "pkg/async-generator.js",
        )
        .unwrap();
    context.execute_module(&async_generator).unwrap();

    assert_script_true(
        &mut context,
        r#"
        __defaultGenerator === true &&
        __defaultAsyncFunction === true &&
        __defaultAsyncGenerator === true
        "#,
    );
}

#[test]
fn default_class_declarations_keep_tdz_and_name_before_static_initializers() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let anonymous = context
        .compile_module_with_filename(
            r#"
            import Current from "./anonymous-class.js";
            try {
                typeof Current;
            } catch (error) {
                globalThis.__anonymousClassTdz = error instanceof ReferenceError;
            }
            export default class {
                static observedName = this.name;
            }
            globalThis.__anonymousClassName =
                Current.name === "default" && Current.observedName === "default";
            "#,
            "pkg/anonymous-class.js",
        )
        .unwrap();
    context.execute_module(&anonymous).unwrap();

    let named = context
        .compile_module_with_filename(
            r#"
            import Current from "./named-class.js";
            export default class Named {}
            globalThis.__namedClassBefore = Current === Named && Current.name === "Named";
            Named = 42;
            globalThis.__namedClassAfter = Current === 42;
            "#,
            "pkg/named-class.js",
        )
        .unwrap();
    context.execute_module(&named).unwrap();

    let static_name = context
        .compile_module_with_filename(
            r#"
            import Current from "./static-name-class.js";
            export default class { static name() { return "name method"; } }
            globalThis.__staticNameMethod = Current.name() === "name method";
            "#,
            "pkg/static-name-class.js",
        )
        .unwrap();
    context.execute_module(&static_name).unwrap();

    assert_script_true(
        &mut context,
        r#"
        __anonymousClassTdz === true &&
        __anonymousClassName === true &&
        __namedClassBefore === true &&
        __namedClassAfter === true &&
        __staticNameMethod === true
        "#,
    );
}

#[test]
fn imported_mutable_cell_has_an_immutable_importer_view() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/exporter.js",
        "export let value = 1; export function update() { value = 42; }",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { value, update } from "./exporter.js";
            try { value = 2; } catch (error) { globalThis.__importReadOnly = true; }
            try { eval("value = 3"); } catch (error) { globalThis.__evalImportReadOnly = true; }
            globalThis.__nestedImportRead = () => value;
            globalThis.__evalNestedImportRead = eval("() => value");
            globalThis.__importBeforeUpdate = value;
            update();
            globalThis.__importAfterUpdate = value;
            "#,
            "pkg/importer.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __importReadOnly === true && __evalImportReadOnly === true &&
        __importBeforeUpdate === 1 && __importAfterUpdate === 42 &&
        __nestedImportRead() === 42 && __evalNestedImportRead() === 42
        "#,
    );
}

#[test]
fn import_declaration_collisions_match_pinned_quickjs_single_slot_semantics() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/named-let.js", "export let value = 7;"),
        ("pkg/named-const.js", "export let value = 7;"),
        ("pkg/namespace.js", "export const value = 7;"),
        ("pkg/class.js", "export default 7;"),
        ("pkg/function.js", "export function value() { return 7; }"),
        ("pkg/default-expression.js", "export default null;"),
        ("pkg/default-var.js", "export default 7;"),
        ("pkg/destructure-array.js", "export let value = 7;"),
        ("pkg/destructure-object.js", "export let value = 7;"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { value as letValue, value as letAlias } from "./named-let.js";
            let letValue = 11;

            const constValue = 13;
            import { value as constValue, value as constAlias } from "./named-const.js";

            import * as namespaceValue from "./namespace.js";
            import * as namespaceAlias from "./namespace.js";
            let namespaceValue = 12;
            export { namespaceValue as collidedNamespace };
            import { collidedNamespace as namespaceExportAlias } from "./collision.js";

            import classValue from "./class.js";
            import classAlias from "./class.js";
            class classValue {}

            import { value as first, value as second } from "./function.js";
            { var first; }
            function second() { return 2; }
            function first() { return 1; }

            import defaultFunction from "./default-expression.js";
            import defaultFunctionAlias from "./default-expression.js";
            function defaultFunction() { return 42; }

            import defaultVar from "./default-var.js";
            var defaultVar;

            import {
                value as arrayValue,
                value as arrayAlias,
            } from "./destructure-array.js";
            let [arrayValue] = [17];

            import {
                value as objectValue,
                value as objectAlias,
            } from "./destructure-object.js";
            const { answer: objectValue } = { answer: 19 };

            let readonly = 0;
            try { letValue = 90; } catch (error) { readonly += error instanceof TypeError; }
            try { constValue = 91; } catch (error) { readonly += error instanceof TypeError; }
            try { namespaceValue = 92; } catch (error) { readonly += error instanceof TypeError; }
            try { classValue = 93; } catch (error) { readonly += error instanceof TypeError; }
            try { first = 94; } catch (error) { readonly += error instanceof TypeError; }
            try { defaultFunction = 95; } catch (error) { readonly += error instanceof TypeError; }
            try { defaultVar = 96; } catch (error) { readonly += error instanceof TypeError; }
            try { arrayValue = 97; } catch (error) { readonly += error instanceof TypeError; }
            try { objectValue = 98; } catch (error) { readonly += error instanceof TypeError; }

            globalThis.__importDeclarationCollision =
                letValue === 11 && letAlias === 11 &&
                constValue === 13 && constAlias === 13 &&
                namespaceValue === 12 && namespaceAlias.value === 7 &&
                namespaceExportAlias === 12 &&
                classValue === classAlias && classValue.name === "classValue" &&
                first() === 1 && second() === 1 &&
                defaultFunction === null && defaultFunctionAlias === null &&
                defaultVar === 7 &&
                arrayValue === 17 && arrayAlias === 17 &&
                objectValue === 19 && objectAlias === 19 &&
                readonly === 9;
            "#,
            "pkg/collision.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__importDeclarationCollision === true");

    let var_initializer = context
        .compile_module_with_filename(
            "import failed from './default-var.js'; var failed = 42;",
            "pkg/var-initializer-collision.js",
        )
        .unwrap();
    let snapshot = module_evaluation_snapshot(&mut context, &var_initializer);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert!(matches!(snapshot.result, RawValue::Object(_)));
}

#[test]
fn import_meta_is_cached_per_defining_module() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        r#"
            globalThis.__dependencyMeta = import.meta;
            export function readMeta() { return import.meta; }
        "#,
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
                import { readMeta } from "./dependency.js";
                const local = import.meta;
                function localRead() { return import.meta; }
                const before = Reflect.ownKeys(local).length === 0;
                local.answer = 42;
                const descriptor = Object.getOwnPropertyDescriptor(local, "answer");
                globalThis.__lateReadMeta = readMeta;
                globalThis.__importMetaParity =
                    before &&
                    Object.getPrototypeOf(local) === null &&
                    Object.isExtensible(local) &&
                    local === import.meta && local === localRead() &&
                    readMeta() === globalThis.__dependencyMeta &&
                    readMeta() !== local &&
                    descriptor.value === 42 && descriptor.writable &&
                    descriptor.enumerable && descriptor.configurable &&
                    delete local.answer && !("answer" in local) &&
                    typeof local.resolve === "undefined";
            "#,
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__importMetaParity === true");
    drop(module);
    runtime.run_gc().unwrap();
    assert_script_true(&mut context, "__lateReadMeta() === __dependencyMeta");
}

#[test]
fn host_gets_the_canonical_import_meta_before_linking() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
                globalThis.__hostMeta = import.meta;
                globalThis.__hostMetaAnswer = import.meta.answer;
            "#,
            "pkg/host-meta.js",
        )
        .unwrap();

    let first = context.get_module_import_meta(&module).unwrap();
    let second = context.get_module_import_meta(&module).unwrap();
    assert_eq!(first, second);
    assert_eq!(runtime.get_prototype_of(&first).unwrap(), None);
    assert!(runtime.is_extensible(&first).unwrap());

    let answer = runtime.intern_property_key("answer").unwrap();
    assert!(
        context
            .define_own_property(
                &first,
                &answer,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(Value::Int(42)),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )
            .unwrap()
    );

    // Ordinary host/user mutations remain valid before linking; every
    // following record replacement must keep accepting the same object.
    let prototype = context.new_object().unwrap();
    assert!(runtime.set_prototype_of(&first, Some(&prototype)).unwrap());
    runtime.prevent_extensions(&first).unwrap();

    context.execute_module(&module).unwrap();
    let global = context.global_object().unwrap();
    let observed = runtime.intern_property_key("__hostMeta").unwrap();
    assert_eq!(
        context.get_property(&global, &observed).unwrap(),
        Value::Object(first.clone())
    );
    assert_script_true(&mut context, "__hostMetaAnswer === 42");

    assert!(context.execute_module(&module).is_ok());
}

#[test]
fn module_record_owns_import_meta_through_gc_and_releases_cycles_with_its_cache() {
    let runtime = Runtime::new();
    let module = {
        let mut context = runtime.new_context();
        context.compile_module("export const answer = 42;").unwrap()
    };
    let mut host_context = runtime.new_context();
    let meta = host_context.get_module_import_meta(&module).unwrap();
    let self_key = runtime.intern_property_key("self").unwrap();
    assert!(
        host_context
            .set_property(&meta, &self_key, Value::Object(meta.clone()))
            .unwrap()
    );
    let meta_id = meta.object_id();
    drop(meta);
    runtime.run_gc().unwrap();
    assert!(runtime.0.state.borrow().heap.object(meta_id).is_ok());

    let observed = host_context.get_module_import_meta(&module).unwrap();
    assert_eq!(observed.object_id(), meta_id);
    drop(observed);
    drop(module);
    drop(host_context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
    assert_eq!(runtime.heap_counts().object_nodes, 0);
}

#[test]
fn loader_initializes_dependency_import_meta_before_source_completion() {
    let runtime = Runtime::new();
    let marker = runtime.new_object(None).unwrap();
    let dependency = ModuleLoadResult::SourceTextWithImportMeta {
        source: r#"
            globalThis.__dependencyMetaChecks = [
                import.meta.url,
                import.meta.main,
                import.meta.marker,
                Object.getOwnPropertyDescriptor(import.meta, "url")
            ];
            export const answer = 42;
        "#
        .to_owned(),
        properties: vec![
            ModuleImportMetaProperty::new(
                JsString::from_static("url"),
                Value::String(JsString::from_static("file:///pkg/dependency.js")),
            ),
            ModuleImportMetaProperty::new(JsString::from_static("main"), Value::Bool(false)),
            ModuleImportMetaProperty::new(
                JsString::from_static("marker"),
                Value::Object(marker.clone()),
            ),
        ],
    };
    let (loader, _, _) = JsonModuleLoader::new([("pkg/dependency.js", dependency)]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './dependency.js'; globalThis.__entryAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();
    context.execute_module(&module).unwrap();

    let marker_key = runtime
        .intern_property_key("__dependencyMetaChecks")
        .unwrap();
    let global = context.global_object().unwrap();
    let Value::Object(checks) = context.get_property(&global, &marker_key).unwrap() else {
        panic!("dependency import.meta checks were not published");
    };
    let zero = runtime.intern_property_key("0").unwrap();
    let one = runtime.intern_property_key("1").unwrap();
    let two = runtime.intern_property_key("2").unwrap();
    assert_eq!(
        context.get_property(&checks, &zero).unwrap(),
        Value::String(JsString::from_static("file:///pkg/dependency.js"))
    );
    assert_eq!(
        context.get_property(&checks, &one).unwrap(),
        Value::Bool(false)
    );
    assert_eq!(
        context.get_property(&checks, &two).unwrap(),
        Value::Object(marker)
    );
    assert_script_true(
        &mut context,
        r#"
            __entryAnswer === 42 &&
            __dependencyMetaChecks[3].writable &&
            __dependencyMetaChecks[3].enumerable &&
            __dependencyMetaChecks[3].configurable
        "#,
    );
}

#[test]
fn import_meta_host_values_must_belong_to_the_loading_runtime() {
    let runtime = Runtime::new();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let local = runtime.new_object(None).unwrap();
    let foreign = Runtime::new().new_object(None).unwrap();
    let result = ModuleLoadResult::SourceTextWithImportMeta {
        source: "export const answer = 42;".to_owned(),
        properties: vec![
            ModuleImportMetaProperty::new(
                JsString::from_static("local"),
                Value::Object(local.clone()),
            ),
            ModuleImportMetaProperty::new(JsString::from_static("foreign"), Value::Object(foreign)),
        ],
    };
    let (loader, _, _) = JsonModuleLoader::new([("pkg/dependency.js", result)]);
    let loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './dependency.js';", "pkg/entry.js",),
        Err(RuntimeError::WrongRuntime("descriptor value"))
    ));
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2,
        "entry and dependency construction tombstones must both remain append-only"
    );
    drop(loader_registration);
    drop(local);
    drop(context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
}

#[test]
fn failed_deep_resolution_releases_published_dependency_import_meta() {
    let runtime = Runtime::new();
    let baseline_objects = runtime.heap_counts().object_nodes;
    let marker = runtime.new_object(None).unwrap();
    let dependency = ModuleLoadResult::SourceTextWithImportMeta {
        source: "import './missing.js'; export const answer = 42;".to_owned(),
        properties: vec![ModuleImportMetaProperty::new(
            JsString::from_static("marker"),
            Value::Object(marker.clone()),
        )],
    };
    let (loader, _, _) = JsonModuleLoader::new([("pkg/dependency.js", dependency)]);
    let loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();

    assert!(matches!(
        context.compile_module_with_filename("import './dependency.js';", "pkg/entry.js"),
        Err(RuntimeError::Exception)
    ));
    assert!(context.take_exception().unwrap().is_some());
    assert_eq!(
        runtime
            .0
            .state
            .borrow()
            .heap
            .loaded_module_slot_count(context.realm)
            .unwrap(),
        2,
        "the failed entry and dependency remain only as cache tombstones"
    );
    drop(loader_registration);
    drop(marker);
    drop(context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().object_nodes, baseline_objects);
}

#[test]
fn missing_export_fails_during_retryable_link_before_module_bodies() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "globalThis.__missingDependencyRan = true; export const present = 1;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import { absent } from "./dependency.js";
            globalThis.__missingEntryRan = absent;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
    }
    assert_script_true(
        &mut context,
        "typeof __missingDependencyRan === 'undefined' && typeof __missingEntryRan === 'undefined'",
    );
}

#[test]
fn cyclic_link_failure_resets_every_active_scc_member() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            "import { b, absent } from './b.js'; export const a = b;",
        ),
        (
            "pkg/b.js",
            "import { a } from './a.js'; export const b = 2; globalThis.__cycleLinkBody = a;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename("import { a } from './a.js'; void a;", "pkg/entry.js")
        .unwrap();
    let a = runtime.module_dependencies(&module).unwrap().remove(0);
    let b = runtime.module_dependencies(&a).unwrap().remove(0);

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
        for member in [&module, &a, &b] {
            assert_eq!(
                runtime.module_record(member.raw).unwrap().link_status,
                ModuleLinkStatus::Unlinked
            );
        }
    }
    assert_script_true(&mut context, "typeof __cycleLinkBody === 'undefined'");
}

#[test]
fn exported_import_cycle_resolves_to_the_ultimate_live_cell() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "import { x } from './b.js'; export { x };"),
        (
            "pkg/b.js",
            "import { c } from './c.js'; export const x = 42; export const b = c;",
        ),
        (
            "pkg/c.js",
            "import { x } from './a.js'; export const c = x;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { b } from './b.js'; globalThis.__exportCycleBody = b;",
            "pkg/entry.js",
        )
        .unwrap();

    // Every import alias can be linked even though A's local export is an
    // imported binding whose own SCC member has not linked yet.
    context.link_module(&module).unwrap();
    let first = module_evaluation_promise(&mut context, &module);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert!(matches!(first_snapshot.result, RawValue::Object(_)));
    let second = module_evaluation_promise(&mut context, &module);
    assert_eq!(first.object_id(), second.object_id());
    // Evaluation still observes the specified TDZ: C reads B.x before B's
    // body initializes it. The exception is cached instead of becoming a
    // missing-cell invariant or native crash.
    assert_script_true(&mut context, "typeof __exportCycleBody === 'undefined'");
}

#[test]
fn circular_exported_import_alias_is_a_retryable_syntax_error() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "import { x } from './b.js'; export { x };"),
        ("pkg/b.js", "import { x } from './a.js'; export { x };"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { x } from './a.js'; globalThis.__circularAliasBody = x;",
            "pkg/entry.js",
        )
        .unwrap();

    for _ in 0..2 {
        assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
    }
    assert_script_true(&mut context, "typeof __circularAliasBody === 'undefined'");
}

#[test]
fn resolve_export_keeps_same_binding_diamonds_unambiguous() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/source.js", "export const answer = 42;"),
        ("pkg/left.js", "export { answer } from './source.js';"),
        ("pkg/right.js", "export { answer } from './source.js';"),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './barrel.js'; globalThis.__diamondAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__diamondAnswer === 42");
}

#[test]
fn namespace_exports_from_one_owner_share_quickjs_star_identity() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "export const a = 1;"),
        ("pkg/b.js", "export const b = 2;"),
        (
            "pkg/source.js",
            "export * as left from './a.js'; export * as right from './b.js';",
        ),
        ("pkg/left.js", "export { left as x } from './source.js';"),
        ("pkg/right.js", "export { right as x } from './source.js';"),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { x } from './barrel.js'; globalThis.__namespaceIdentity = x;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        "__namespaceIdentity.a === 1 && !('b' in __namespaceIdentity)",
    );
}

#[test]
fn resolve_export_reports_distinct_star_bindings_as_ambiguous() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/left.js", "export const answer = 1;"),
        ("pkg/right.js", "export const answer = 2;"),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './barrel.js'; void answer;",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("export 'answer' in module 'pkg/barrel.js' is ambiguous")
    );
}

#[test]
fn star_resolution_ignores_circular_and_not_found_branches() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/cycle-a.js",
            "export * from './cycle-b.js'; export const unrelated = 1;",
        ),
        (
            "pkg/cycle-b.js",
            "export * from './cycle-a.js'; export const other = 2;",
        ),
        ("pkg/empty.js", "export const absent = 3;"),
        ("pkg/source.js", "export const answer = 42;"),
        (
            "pkg/barrel.js",
            "export * from './cycle-a.js'; export * from './empty.js'; export * from './source.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './barrel.js'; globalThis.__starBranchAnswer = answer;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__starBranchAnswer === 42");
}

#[test]
fn module_namespace_omits_an_ambiguous_star_export() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/left.js",
            "export const answer = 1; export const left = 2;",
        ),
        (
            "pkg/right.js",
            "export const answer = 3; export const right = 4;",
        ),
        (
            "pkg/barrel.js",
            "export * from './left.js'; export * from './right.js';",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import * as ns from './barrel.js'; globalThis.__ambiguousNamespace = ns;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        "!('answer' in __ambiguousNamespace) && __ambiguousNamespace.left === 2 && __ambiguousNamespace.right === 4",
    );
}

#[test]
fn default_is_not_resolved_through_star_exports() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/source.js", "export default 42;"),
        ("pkg/barrel.js", "export * from './source.js';"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { default as answer } from './barrel.js'; void answer;",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("Could not find export 'default' in module 'pkg/barrel.js'")
    );
}

#[test]
fn indirect_export_preflight_blames_the_public_name_and_owner() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "globalThis.__indirectDependencyRan = true; export const present = 1;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "export { absent as publicName } from './dependency.js';",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static("Could not find export 'publicName' in module 'pkg/entry.js'")
    );
    assert_script_true(
        &mut context,
        "typeof __indirectDependencyRan === 'undefined'",
    );
}

#[test]
fn circular_indirect_exports_fail_without_native_recursion() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        ("pkg/a.js", "export { answer } from './b.js';"),
        ("pkg/b.js", "export { answer } from './a.js';"),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import { answer } from './a.js'; void answer;",
            "pkg/entry.js",
        )
        .unwrap();

    assert_eq!(context.link_module(&module), Err(RuntimeError::Exception));
    assert_eq!(
        take_error_message(&runtime, &mut context),
        JsString::from_static(
            "circular reference when looking for export 'answer' in module 'pkg/b.js'"
        )
    );
}

#[test]
fn namespace_cache_preserves_cycles_identity_and_live_cells() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            "export * as b from './b.js'; export let value = 1; export function bump() { value = 42; }",
        ),
        (
            "pkg/b.js",
            "export * as a from './a.js'; export const marker = 2;",
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            r#"
            import * as a from './a.js';
            import * as b from './b.js';
            globalThis.__namespaceA = a;
            globalThis.__namespaceB = b;
            globalThis.__namespaceBefore = a.value;
            a.bump();
            globalThis.__namespaceAfter = a.value;
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __namespaceA.b === __namespaceB &&
        __namespaceB.a === __namespaceA &&
        __namespaceBefore === 1 && __namespaceAfter === 42 &&
        Object.getPrototypeOf(__namespaceA) === null &&
        Object.isExtensible(__namespaceA) === false &&
        Reflect.ownKeys(__namespaceA).slice(0, 3).join(',') === 'b,bump,value' &&
        Reflect.ownKeys(__namespaceA)[3] === Symbol.toStringTag
        "#,
    );
}

#[test]
fn self_namespace_import_export_keeps_the_preallocated_cell() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/self.js",
        "import * as self from './self.js'; export { self }; export const answer = 42;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import * as ns from './self.js'; globalThis.__selfNamespace = ns;",
            "pkg/entry.js",
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        "__selfNamespace.self === __selfNamespace && __selfNamespace.answer === 42",
    );
}

#[test]
fn failed_namespace_build_rolls_back_its_placeholder_for_retry() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([("pkg/dependency.js", "export const present = 1;")]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "export { absent as publicName } from './dependency.js';",
            "pkg/entry.js",
        )
        .unwrap();
    runtime
        .prepare_module_instance(module.raw, context.realm)
        .unwrap();

    for _ in 0..2 {
        assert_eq!(
            runtime.get_module_namespace(&module, context.realm),
            Err(RuntimeError::Exception)
        );
        assert!(matches!(
            runtime.module_record(module.raw).unwrap().namespace,
            ModuleNamespaceState::Empty
        ));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
    }
}

#[test]
fn deep_star_resolution_uses_an_explicit_frame_stack() {
    const MODULE_COUNT: usize = 1_024;

    std::thread::Builder::new()
        .name("deep-star-module-graph".to_owned())
        .stack_size(256 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let _loader_registration = runtime.set_module_loader(StarChainModuleLoader {
                module_count: MODULE_COUNT,
            });
            let mut context = runtime.new_context();
            let module = context
                .compile_module_with_filename(
                    "import * as ns from 's0'; globalThis.__deepStarAnswer = ns.answer;",
                    "entry.js",
                )
                .unwrap();
            context.execute_module(&module).unwrap();
            assert_script_true(&mut context, "__deepStarAnswer === 42");
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn deep_cyclic_graph_uses_explicit_resolve_link_and_evaluation_stacks() {
    const MODULE_COUNT: usize = 1_024;

    std::thread::Builder::new()
        .name("deep-module-graph".to_owned())
        .stack_size(256 * 1024)
        .spawn(|| {
            let runtime = Runtime::new();
            let _loader_registration = runtime.set_module_loader(CyclicChainModuleLoader {
                module_count: MODULE_COUNT,
            });
            let mut context = runtime.new_context();
            let module = context
                .compile_module_with_filename(
                    "import 'm0'; globalThis.__deepModuleEntry = true;",
                    "entry.js",
                )
                .unwrap();
            context.link_module(&module).unwrap();
            context.execute_module(&module).unwrap();
            assert_script_true(
                &mut context,
                &format!("__deepModuleRuns === {MODULE_COUNT} && __deepModuleEntry === true"),
            );
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn dependency_evaluation_exception_is_cached_on_every_active_ancestor() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/abrupt.js",
        r#"
        globalThis.__abruptRuns = (globalThis.__abruptRuns || 0) + 1;
        throw 42;
        "#,
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import './abrupt.js'; globalThis.__ancestorRan = true;",
            "pkg/entry.js",
        )
        .unwrap();

    let first = module_evaluation_promise(&mut context, &module);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert_eq!(first_snapshot.result, RawValue::Int(42));
    let second = module_evaluation_promise(&mut context, &module);
    assert_eq!(first.object_id(), second.object_id());
    assert_script_true(
        &mut context,
        "__abruptRuns === 1 && typeof __ancestorRan === 'undefined'",
    );
}

#[test]
fn cyclic_evaluation_exception_is_cached_on_the_complete_active_scc() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            r#"
            import "./b.js";
            globalThis.__cycleARuns = (globalThis.__cycleARuns || 0) + 1;
            throw 42;
            "#,
        ),
        (
            "pkg/b.js",
            r#"
            import "./a.js";
            globalThis.__cycleBRuns = (globalThis.__cycleBRuns || 0) + 1;
            "#,
        ),
    ]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename(
            "import './a.js'; globalThis.__cycleEntryRan = true;",
            "pkg/entry.js",
        )
        .unwrap();
    let a = runtime.module_dependencies(&module).unwrap().remove(0);
    let b = runtime.module_dependencies(&a).unwrap().remove(0);

    let first = module_evaluation_promise(&mut context, &module);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert_eq!(first_snapshot.result, RawValue::Int(42));
    let second = module_evaluation_promise(&mut context, &module);
    assert_eq!(first.object_id(), second.object_id());
    for _ in 0..2 {
        for member in [&module, &a, &b] {
            assert!(matches!(
                runtime.module_record(member.raw).unwrap().evaluation,
                ModuleEvaluationState::Errored(RawValue::Int(42))
            ));
        }
    }
    assert_script_true(
        &mut context,
        "__cycleARuns === 1 && __cycleBRuns === 1 && typeof __cycleEntryRan === 'undefined'",
    );
}

#[test]
fn context_module_cache_is_oldest_first_and_loader_cache_is_per_context() {
    let runtime = Runtime::new();
    let (loader, loads, _) = MapModuleLoader::new([("pkg/loaded.js", "export const loaded = 42;")]);
    let _loader_registration = runtime.set_module_loader(loader);

    let mut first_context = runtime.new_context();
    first_context
        .compile_module_with_filename("export const value = 1;", "pkg/shared.js")
        .unwrap();
    first_context
        .compile_module_with_filename("export const value = 2;", "pkg/shared.js")
        .unwrap();
    let oldest = first_context
        .compile_module_with_filename(
            "import { value } from './shared.js'; globalThis.__oldest = value;",
            "pkg/oldest-entry.js",
        )
        .unwrap();
    first_context.execute_module(&oldest).unwrap();
    assert_script_true(&mut first_context, "__oldest === 1");

    let first_loaded = first_context
        .compile_module_with_filename(
            "import { loaded } from './loaded.js'; globalThis.__loaded = loaded;",
            "pkg/first-entry.js",
        )
        .unwrap();
    first_context.execute_module(&first_loaded).unwrap();

    let mut second_context = runtime.new_context();
    let second_loaded = second_context
        .compile_module_with_filename(
            "import { loaded } from './loaded.js'; globalThis.__loaded = loaded;",
            "pkg/second-entry.js",
        )
        .unwrap();
    second_context.execute_module(&second_loaded).unwrap();
    assert_eq!(&*loads.borrow(), &["pkg/loaded.js", "pkg/loaded.js"]);
    assert_script_true(&mut first_context, "__loaded === 42");
    assert_script_true(&mut second_context, "__loaded === 42");
}

#[test]
fn first_execute_context_owns_globals_for_the_complete_module_graph() {
    let runtime = Runtime::new();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/dependency.js",
        "globalThis.__graphDependencyRealm = __realmMarker; export const value = 42;",
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let mut compilation_context = runtime.new_context();
    compilation_context
        .eval("globalThis.__realmMarker = 1")
        .unwrap();
    let module = compilation_context
        .compile_module_with_filename(
            "import { value } from './dependency.js'; globalThis.__graphRootRealm = __realmMarker + value;",
            "pkg/entry.js",
        )
        .unwrap();

    let mut execution_context = runtime.new_context();
    execution_context
        .eval("globalThis.__realmMarker = 2")
        .unwrap();
    execution_context.execute_module(&module).unwrap();
    assert_script_true(
        &mut execution_context,
        "__graphDependencyRealm === 2 && __graphRootRealm === 44",
    );
    assert_script_true(
        &mut compilation_context,
        "typeof __graphDependencyRealm === 'undefined' && typeof __graphRootRealm === 'undefined'",
    );
}

#[test]
fn module_cells_use_the_link_context_while_bytecode_keeps_its_compile_realm() {
    let runtime = Runtime::new();
    let mut compilation_context = runtime.new_context();
    let compilation_object_prototype = compilation_context.eval("Object.prototype").unwrap();
    let compilation_function_prototype = compilation_context.eval("Function.prototype").unwrap();
    let compilation_array_prototype = compilation_context.eval("Array.prototype").unwrap();
    let compilation_type_error_prototype = compilation_context.eval("TypeError.prototype").unwrap();
    let module = compilation_context
        .compile_module(
            r#"
            globalThis.__moduleRealmObject = {};
            globalThis.__moduleRealmFunction = function () {};
            globalThis.__moduleRealmArray = [];
            try { null.value; } catch (error) { globalThis.__moduleRealmError = error; }
            "#,
        )
        .unwrap();

    let mut link_context = runtime.new_context();
    let link_object_prototype = link_context.eval("Object.prototype").unwrap();
    let link_function_prototype = link_context.eval("Function.prototype").unwrap();
    let link_array_prototype = link_context.eval("Array.prototype").unwrap();
    let link_type_error_prototype = link_context.eval("TypeError.prototype").unwrap();
    link_context.execute_module(&module).unwrap();
    let module_object_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmObject)")
        .unwrap();
    let module_function_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmFunction)")
        .unwrap();
    let module_array_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmArray)")
        .unwrap();
    let module_error_prototype = link_context
        .eval("Object.getPrototypeOf(__moduleRealmError)")
        .unwrap();

    // QuickJS creates the module closure and its global cells with the
    // linking Context, while the immutable function bytecode retains the
    // Context which compiled it. Object literals therefore use the latter
    // realm even though `globalThis` resolves through the former's cell.
    assert_eq!(module_object_prototype, compilation_object_prototype);
    assert_eq!(module_function_prototype, compilation_function_prototype);
    assert_eq!(module_array_prototype, compilation_array_prototype);
    assert_eq!(module_error_prototype, compilation_type_error_prototype);
    assert_ne!(module_object_prototype, link_object_prototype);
    assert_ne!(module_function_prototype, link_function_prototype);
    assert_ne!(module_array_prototype, link_array_prototype);
    assert_ne!(module_error_prototype, link_type_error_prototype);
    assert_script_true(
        &mut compilation_context,
        "typeof __moduleRealmObject === 'undefined'",
    );
}

#[test]
fn dependency_free_top_level_await_fulfills_the_evaluation_promise() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__tlaLog = []").unwrap();
    let module = context
        .compile_module(
            r#"
            globalThis.__tlaLog.push("start");
            const value = await 41;
            globalThis.__tlaLog.push("end:" + (value + 1));
            "#,
        )
        .unwrap();

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert_script_true(&mut context, "globalThis.__tlaLog.join(',') === 'start'");

    assert!(drain_jobs(&runtime) > 0);
    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    assert_eq!(
        runtime.root_raw_value(&snapshot.result).unwrap(),
        Value::Undefined
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaLog.join(',') === 'start,end:42'",
    );
    assert!(matches!(
        runtime.module_record(module.raw).unwrap().evaluation,
        ModuleEvaluationState::Evaluated
    ));

    let cached = module_evaluation_promise(&mut context, &module);
    assert_eq!(cached.object_id(), promise.object_id());
    assert!(!runtime.is_job_pending());
}

#[test]
fn async_dependency_does_not_block_a_sibling_but_delays_its_parent() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__tlaOrder = []").unwrap();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/async.js",
            r#"
            globalThis.__asyncDependencyDone = false;
            globalThis.__tlaOrder.push("async:start");
            await 0;
            globalThis.__asyncDependencyDone = true;
            globalThis.__tlaOrder.push("async:end");
            export const answer = 42;
            "#,
        ),
        (
            "pkg/sibling.js",
            r#"
            globalThis.__tlaOrder.push("sibling");
            export const sawAsyncEnd = globalThis.__asyncDependencyDone;
            "#,
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            r#"
            import { answer } from "./async.js";
            import { sawAsyncEnd } from "./sibling.js";
            globalThis.__tlaOrder.push("parent:" + answer + ":" + sawAsyncEnd);
            "#,
            "pkg/entry.js",
        )
        .unwrap();

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaOrder.join(',') === 'async:start,sibling'",
    );

    assert!(drain_jobs(&runtime) > 0);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaOrder.join(',') === 'async:start,sibling,async:end,parent:42:false'",
    );
    assert!(!runtime.is_job_pending());
}

#[test]
fn async_dependency_rejection_preserves_identity_and_skips_the_parent_body() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval("globalThis.__tlaReason = {}; globalThis.__tlaReason")
        .unwrap();
    let (loader, _, _) =
        MapModuleLoader::new([("pkg/reject.js", "await 0; throw globalThis.__tlaReason;")]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            "import './reject.js'; globalThis.__tlaParentRan = true;",
            "pkg/entry.js",
        )
        .unwrap();
    let dependency = runtime.module_dependencies(&module).unwrap().remove(0);

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert!(drain_jobs(&runtime) > 0);

    let snapshot = promise_snapshot(&runtime, &promise);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&snapshot.result).unwrap(), reason);
    assert_script_true(
        &mut context,
        "typeof globalThis.__tlaParentRan === 'undefined'",
    );
    for member in [&module, &dependency] {
        assert!(matches!(
            runtime.module_record(member.raw).unwrap().evaluation,
            ModuleEvaluationState::Errored(_)
        ));
    }

    let cached = module_evaluation_promise(&mut context, &module);
    assert_eq!(cached.object_id(), promise.object_id());
    let cached = promise_snapshot(&runtime, &cached);
    assert_eq!(cached.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&cached.result).unwrap(), reason);
    assert!(!runtime.is_job_pending());
}

#[test]
fn shared_async_dependency_rejects_evaluation_promises_in_forward_parent_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval(
            r#"
            globalThis.__sharedBranchReason = {};
            globalThis.__sharedBranchGate = new Promise(function (_, reject) {
                globalThis.__rejectSharedBranchGate = reject;
            });
            globalThis.__sharedBranchReason;
            "#,
        )
        .unwrap();
    let (loader, _, _) = MapModuleLoader::new([(
        "pkg/shared-branch.js",
        "await globalThis.__sharedBranchGate; export const value = 42;",
    )]);
    let _registration = runtime.set_module_loader(loader);
    let first = context
        .compile_module_with_filename(
            "import './shared-branch.js'; globalThis.__firstBranchRan = true;",
            "pkg/first-branch.js",
        )
        .unwrap();
    let second = context
        .compile_module_with_filename(
            "import './shared-branch.js'; globalThis.__secondBranchRan = true;",
            "pkg/second-branch.js",
        )
        .unwrap();
    let first_promise = module_evaluation_promise(&mut context, &first);
    let second_promise = module_evaluation_promise(&mut context, &second);
    assert_eq!(
        promise_snapshot(&runtime, &first_promise).state,
        PromiseState::Pending
    );
    assert_eq!(
        promise_snapshot(&runtime, &second_promise).state,
        PromiseState::Pending
    );
    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    let first_promise_id = first_promise.object_id();
    let second_raw = second.raw;
    let second_was_pending = Rc::new(Cell::new(false));
    let captured_second_was_pending = second_was_pending.clone();
    let observing_runtime = runtime.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        if !event.is_handled() && event.promise().object_id() == first_promise_id {
            captured_second_was_pending.set(matches!(
                observing_runtime
                    .module_record(second_raw)
                    .expect("reentrant rejection tracker lost the second parent")
                    .evaluation,
                ModuleEvaluationState::EvaluatingAsync
            ));
        }
        captured.borrow_mut().push((
            event.is_handled(),
            event.promise().object_id(),
            event.reason().clone(),
        ));
    });

    context
        .eval("globalThis.__rejectSharedBranchGate(globalThis.__sharedBranchReason)")
        .unwrap();
    assert!(drain_jobs(&runtime) > 0);

    assert_eq!(
        promise_snapshot(&runtime, &first_promise).state,
        PromiseState::Rejected
    );
    assert_eq!(
        promise_snapshot(&runtime, &second_promise).state,
        PromiseState::Rejected
    );
    assert_script_true(
        &mut context,
        "typeof globalThis.__firstBranchRan === 'undefined' && typeof globalThis.__secondBranchRan === 'undefined'",
    );
    assert_eq!(
        events.borrow().as_slice(),
        &[
            (false, first_promise.object_id(), reason.clone()),
            (false, second_promise.object_id(), reason),
        ]
    );
    assert!(
        second_was_pending.get(),
        "first rejection tracker callback observed the later parent already errored"
    );
    runtime.clear_host_promise_rejection_tracker();
    assert!(!runtime.is_job_pending());
}

#[test]
fn shared_tla_completion_executes_cross_linked_parents_in_callback_realm() {
    let runtime = Runtime::new();
    let mut first_context = runtime.new_context();
    first_context
        .eval(
            r#"
            globalThis.__crossRealmGate = new Promise(function (resolve) {
                globalThis.__releaseCrossRealmGate = resolve;
            });
            "#,
        )
        .unwrap();
    let dependency = first_context
        .compile_module_with_filename(
            "await globalThis.__crossRealmGate; export const value = 42;",
            "pkg/cross-realm-dependency.js",
        )
        .unwrap();
    let dependency_promise = module_evaluation_promise(&mut first_context, &dependency);
    assert_eq!(
        promise_snapshot(&runtime, &dependency_promise).state,
        PromiseState::Pending
    );

    let parent = first_context
        .compile_module_with_filename(
            "import './cross-realm-dependency.js'; throw 42;",
            "pkg/cross-realm-parent.js",
        )
        .unwrap();
    let async_parent = first_context
        .compile_module_with_filename(
            "import './cross-realm-dependency.js'; await 0;",
            "pkg/cross-realm-async-parent.js",
        )
        .unwrap();
    let first_realm = first_context.realm;
    let mut second_context = runtime.new_context();
    let parent_promise = module_evaluation_promise(&mut second_context, &parent);
    let async_parent_promise = module_evaluation_promise(&mut second_context, &async_parent);
    assert_eq!(
        promise_snapshot(&runtime, &parent_promise).state,
        PromiseState::Pending
    );
    assert_eq!(
        promise_snapshot(&runtime, &async_parent_promise).state,
        PromiseState::Pending
    );
    first_context
        .eval(
            r#"
            globalThis.__crossRealmSpecies = [];
            Object.defineProperty(Promise, Symbol.species, {
                configurable: true,
                get() {
                    globalThis.__crossRealmSpecies.push("A");
                    return Promise;
                },
            });
            "#,
        )
        .unwrap();
    second_context
        .eval(
            r#"
            globalThis.__crossRealmSpecies = [];
            Object.defineProperty(Promise, Symbol.species, {
                configurable: true,
                get() {
                    globalThis.__crossRealmSpecies.push("B");
                    return Promise;
                },
            });
            "#,
        )
        .unwrap();

    let events = Rc::new(RefCell::new(Vec::new()));
    let captured = events.clone();
    runtime.set_host_promise_rejection_tracker(move |event| {
        if !event.is_handled() {
            captured
                .borrow_mut()
                .push((event.context(), event.reason().clone()));
        }
    });
    first_context
        .eval("globalThis.__releaseCrossRealmGate()")
        .unwrap();
    assert!(drain_jobs(&runtime) > 0);

    assert_eq!(
        promise_snapshot(&runtime, &dependency_promise).state,
        PromiseState::Fulfilled
    );
    let parent_snapshot = promise_snapshot(&runtime, &parent_promise);
    assert_eq!(parent_snapshot.state, PromiseState::Rejected);
    assert_eq!(parent_snapshot.result, RawValue::Int(42));
    assert_eq!(
        promise_snapshot(&runtime, &async_parent_promise).state,
        PromiseState::Fulfilled
    );
    assert_eq!(
        events.borrow().as_slice(),
        &[(first_realm, Value::Int(42)), (first_realm, Value::Int(42)),]
    );
    assert_script_true(
        &mut first_context,
        "globalThis.__crossRealmSpecies.join(',') === 'A'",
    );
    assert_script_true(
        &mut second_context,
        "globalThis.__crossRealmSpecies.length === 0",
    );
    runtime.clear_host_promise_rejection_tracker();
    assert!(!runtime.is_job_pending());
}

#[test]
fn late_tla_fulfillment_does_not_overwrite_a_cached_sibling_rejection() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let reason = context
        .eval(
            r#"
            globalThis.__lateTlaLog = [];
            globalThis.__lateTlaReason = {};
            globalThis.__lateTlaGate = new Promise(function (resolve) {
                globalThis.__releaseLateTlaGate = resolve;
            });
            globalThis.__lateTlaReason;
            "#,
        )
        .unwrap();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/late-wait.js",
            r#"
            globalThis.__lateTlaLog.push("wait:start");
            await globalThis.__lateTlaGate;
            globalThis.__lateTlaLog.push("wait:end");
            "#,
        ),
        (
            "pkg/late-throw.js",
            r#"
            globalThis.__lateTlaLog.push("throw");
            throw globalThis.__lateTlaReason;
            "#,
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            r#"
            import "./late-wait.js";
            import "./late-throw.js";
            globalThis.__lateTlaParentRan = true;
            "#,
            "pkg/late-entry.js",
        )
        .unwrap();
    let dependencies = runtime.module_dependencies(&module).unwrap();
    let waiting = dependencies[0].clone();
    let throwing = dependencies[1].clone();

    let promise = module_evaluation_promise(&mut context, &module);
    let initial = promise_snapshot(&runtime, &promise);
    assert_eq!(initial.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&initial.result).unwrap(), reason);
    assert_script_true(
        &mut context,
        "globalThis.__lateTlaLog.join(',') === 'wait:start,throw' && typeof globalThis.__lateTlaParentRan === 'undefined'",
    );
    assert!(matches!(
        runtime.module_record(waiting.raw).unwrap().evaluation,
        ModuleEvaluationState::EvaluatingAsync
    ));
    for member in [&module, &throwing] {
        let ModuleEvaluationState::Errored(raw_reason) =
            runtime.module_record(member.raw).unwrap().evaluation
        else {
            panic!("synchronous module failure was not cached on its active ancestor");
        };
        assert_eq!(runtime.root_raw_value(&raw_reason).unwrap(), reason);
    }

    context.eval("globalThis.__releaseLateTlaGate()").unwrap();
    assert!(drain_jobs(&runtime) > 0);

    assert_script_true(
        &mut context,
        "globalThis.__lateTlaLog.join(',') === 'wait:start,throw,wait:end' && typeof globalThis.__lateTlaParentRan === 'undefined'",
    );
    assert!(matches!(
        runtime.module_record(waiting.raw).unwrap().evaluation,
        ModuleEvaluationState::Evaluated
    ));
    for member in [&module, &throwing] {
        let ModuleEvaluationState::Errored(raw_reason) =
            runtime.module_record(member.raw).unwrap().evaluation
        else {
            panic!("late TLA fulfillment changed the cached rejection state");
        };
        assert_eq!(runtime.root_raw_value(&raw_reason).unwrap(), reason);
    }
    let cached = module_evaluation_promise(&mut context, &module);
    assert_eq!(cached.object_id(), promise.object_id());
    let cached = promise_snapshot(&runtime, &cached);
    assert_eq!(cached.state, PromiseState::Rejected);
    assert_eq!(runtime.root_raw_value(&cached.result).unwrap(), reason);
    assert!(!runtime.is_job_pending());
}

#[test]
fn top_level_await_inside_a_cycle_unblocks_the_cycle_before_its_outer_parent() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__tlaCycleOrder = []").unwrap();
    let (loader, _, _) = MapModuleLoader::new([
        (
            "pkg/a.js",
            "import './b.js'; globalThis.__tlaCycleOrder.push('a');",
        ),
        (
            "pkg/b.js",
            r#"
            import "./a.js";
            globalThis.__tlaCycleOrder.push("b:start");
            await 0;
            globalThis.__tlaCycleOrder.push("b:end");
            "#,
        ),
    ]);
    let _registration = runtime.set_module_loader(loader);
    let module = context
        .compile_module_with_filename(
            "import './a.js'; globalThis.__tlaCycleOrder.push('entry');",
            "pkg/entry.js",
        )
        .unwrap();
    let a = runtime.module_dependencies(&module).unwrap().remove(0);
    let b = runtime.module_dependencies(&a).unwrap().remove(0);

    let promise = module_evaluation_promise(&mut context, &module);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Pending
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaCycleOrder.join(',') === 'b:start'",
    );
    for member in [&module, &a, &b] {
        assert!(matches!(
            runtime.module_record(member.raw).unwrap().evaluation,
            ModuleEvaluationState::EvaluatingAsync
        ));
    }
    assert_eq!(
        runtime
            .module_record(module.raw)
            .unwrap()
            .evaluation_cycle_root,
        Some(module.raw.module)
    );
    assert_eq!(
        runtime.module_record(a.raw).unwrap().evaluation_cycle_root,
        Some(a.raw.module)
    );
    assert_eq!(
        runtime.module_record(b.raw).unwrap().evaluation_cycle_root,
        Some(a.raw.module)
    );

    assert!(drain_jobs(&runtime) > 0);
    assert_eq!(
        promise_snapshot(&runtime, &promise).state,
        PromiseState::Fulfilled
    );
    assert_script_true(
        &mut context,
        "globalThis.__tlaCycleOrder.join(',') === 'b:start,b:end,a,entry'",
    );
    for member in [&module, &a, &b] {
        assert!(matches!(
            runtime.module_record(member.raw).unwrap().evaluation,
            ModuleEvaluationState::Evaluated
        ));
    }
    assert!(!runtime.is_job_pending());
}

#[test]
fn dependency_free_module_links_then_evaluates_with_module_semantics() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            r#"
            globalThis.__moduleThis = this;
            globalThis.__moduleVarBefore = value;
            globalThis.__moduleFunctionBefore = answer();
            var value = 7;
            function answer() { return 42; }
            let lexical = 9;
            globalThis.__moduleResult = value + lexical + answer();
            "#,
        )
        .unwrap();

    let snapshot = module_evaluation_snapshot(&mut context, &module);
    assert_eq!(snapshot.state, PromiseState::Fulfilled);
    assert_eq!(snapshot.result, RawValue::Undefined);
    assert_script_true(
        &mut context,
        r#"
        __moduleThis === undefined &&
        __moduleVarBefore === undefined &&
        __moduleFunctionBefore === 42 &&
        __moduleResult === 58 &&
        typeof value === "undefined" &&
        typeof lexical === "undefined" &&
        typeof answer === "undefined"
        "#,
    );
}

#[test]
fn module_identity_evaluates_once_and_caches_abrupt_completion() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    context.eval("globalThis.__moduleRuns = 0").unwrap();
    let once = context
        .compile_module("globalThis.__moduleRuns += 1")
        .unwrap();
    context.execute_module(&once).unwrap();
    context.execute_module(&once).unwrap();
    assert_script_true(&mut context, "__moduleRuns === 1");

    let abrupt = context.compile_module("throw 42").unwrap();
    let first = module_evaluation_promise(&mut context, &abrupt);
    let first_snapshot = promise_snapshot(&runtime, &first);
    assert_eq!(first_snapshot.state, PromiseState::Rejected);
    assert_eq!(first_snapshot.result, RawValue::Int(42));
    let second = module_evaluation_promise(&mut context, &abrupt);
    assert_eq!(first.object_id(), second.object_id());
}

#[test]
fn module_evaluation_caches_error_object_identity_across_contexts() {
    let runtime = Runtime::new();
    let module = {
        let mut compilation_context = runtime.new_context();
        compilation_context
            .compile_module("throw new Error('cached module error')")
            .unwrap()
    };

    let first_error_id = {
        let mut first_context = runtime.new_context();
        let snapshot = module_evaluation_snapshot(&mut first_context, &module);
        assert_eq!(snapshot.state, PromiseState::Rejected);
        let RawValue::Object(error) = snapshot.result else {
            panic!("module evaluation did not reject with an Error object");
        };
        error
    };
    runtime.run_gc().unwrap();

    let mut second_context = runtime.new_context();
    let snapshot = module_evaluation_snapshot(&mut second_context, &module);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    let RawValue::Object(second_error) = snapshot.result else {
        panic!("cached module evaluation did not retain an Error object");
    };
    assert_eq!(second_error, first_error_id);
}

#[test]
fn module_evaluation_cache_owns_symbol_atoms_until_the_cache_dies() {
    let runtime = Runtime::new();
    let baseline_atoms = runtime.test_atom_count();
    let module = {
        let mut compilation_context = runtime.new_context();
        compilation_context
            .compile_module("throw Symbol('cached module symbol')")
            .unwrap()
    };

    let first_symbol = {
        let mut first_context = runtime.new_context();
        let snapshot = module_evaluation_snapshot(&mut first_context, &module);
        assert_eq!(snapshot.state, PromiseState::Rejected);
        let RawValue::Symbol(symbol) = snapshot.result else {
            panic!("module evaluation did not reject with a Symbol");
        };
        symbol
    };
    runtime.run_gc().unwrap();

    let second_symbol = {
        let mut second_context = runtime.new_context();
        let snapshot = module_evaluation_snapshot(&mut second_context, &module);
        assert_eq!(snapshot.state, PromiseState::Rejected);
        let RawValue::Symbol(symbol) = snapshot.result else {
            panic!("cached module evaluation did not retain a Symbol");
        };
        symbol
    };
    assert_eq!(second_symbol, first_symbol);

    drop(module);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
    assert_eq!(runtime.test_atom_count(), baseline_atoms);
}

#[test]
fn direct_eval_uses_module_live_cells_without_leaking_eval_var() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            r#"
            let live = 1;
            eval("live = 42; var evalScoped = live + 1; globalThis.__evalScopedInside = evalScoped");
            globalThis.__moduleLiveAfterEval = live;
            globalThis.__evalScopedOutside = typeof evalScoped;
            "#,
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(
        &mut context,
        r#"
        __moduleLiveAfterEval === 42 &&
        __evalScopedInside === 43 &&
        __evalScopedOutside === "undefined" &&
        typeof live === "undefined" &&
        typeof evalScoped === "undefined"
        "#,
    );
}

#[test]
fn nested_var_preserves_quickjs_module_function_redeclaration_order() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module(
            r#"
            { var answer; }
            function answer() { return 1; }
            function answer() { return 42; }
            globalThis.__moduleRedeclaredAnswer = answer();
            "#,
        )
        .unwrap();

    context.execute_module(&module).unwrap();
    assert_script_true(&mut context, "__moduleRedeclaredAnswer === 42");
}

#[test]
fn module_handle_rejects_another_runtime() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context.compile_module("").unwrap();
    let mut other = Runtime::new().new_context();
    assert_eq!(
        other.execute_module(&module),
        Err(RuntimeError::WrongRuntime("module bytecode"))
    );
}

#[test]
fn first_execute_context_owns_module_global_resolution_and_evaluates_once() {
    let runtime = Runtime::new();
    let mut compilation_context = runtime.new_context();
    compilation_context
        .eval("globalThis.__realmMarker = 1")
        .unwrap();
    let module = compilation_context
        .compile_module(
            r#"
            globalThis.__moduleLinkMarker = __realmMarker;
            globalThis.__moduleLinkRuns = (globalThis.__moduleLinkRuns || 0) + 1;
            "#,
        )
        .unwrap();

    let mut first_execute_context = runtime.new_context();
    first_execute_context
        .eval("globalThis.__realmMarker = 2")
        .unwrap();
    let mut later_context = runtime.new_context();
    later_context.eval("globalThis.__realmMarker = 3").unwrap();

    let first = module_evaluation_promise(&mut first_execute_context, &module);
    assert_script_true(
        &mut first_execute_context,
        "__moduleLinkMarker === 2 && __moduleLinkRuns === 1",
    );
    assert_script_true(
        &mut compilation_context,
        "typeof __moduleLinkMarker === 'undefined' && typeof __moduleLinkRuns === 'undefined'",
    );
    assert_script_true(
        &mut later_context,
        "typeof __moduleLinkMarker === 'undefined' && typeof __moduleLinkRuns === 'undefined'",
    );

    let second = module_evaluation_promise(&mut later_context, &module);
    assert_eq!(first.object_id(), second.object_id());
    assert_script_true(
        &mut first_execute_context,
        "__moduleLinkMarker === 2 && __moduleLinkRuns === 1",
    );
    assert_script_true(
        &mut later_context,
        "typeof __moduleLinkMarker === 'undefined' && typeof __moduleLinkRuns === 'undefined'",
    );
}

#[test]
fn cloned_module_handle_roots_compilation_and_first_link_realms() {
    let runtime = Runtime::new();
    let module = {
        let mut context = runtime.new_context();
        context
            .compile_module("globalThis.__rootedModuleRealm = 42")
            .unwrap()
    };
    assert_eq!(runtime.heap_counts().context_nodes, 1);
    let surviving_handle = module.clone();
    drop(module);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    {
        let mut link_context = runtime.new_context();
        assert_eq!(runtime.heap_counts().context_nodes, 2);
        let snapshot = module_evaluation_snapshot(&mut link_context, &surviving_handle);
        assert_eq!(snapshot.state, PromiseState::Fulfilled);
        assert_eq!(snapshot.result, RawValue::Undefined);
        assert_script_true(&mut link_context, "__rootedModuleRealm === 42");
    }

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    drop(surviving_handle);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}

#[test]
fn cross_linked_module_caches_do_not_leak_a_context_cycle() {
    let runtime = Runtime::new();
    let mut first_context = runtime.new_context();
    let mut second_context = runtime.new_context();
    let first_module = first_context
        .compile_module("globalThis.__firstCrossCacheModule = 1")
        .unwrap();
    let second_module = second_context
        .compile_module("globalThis.__secondCrossCacheModule = 2")
        .unwrap();

    second_context.execute_module(&first_module).unwrap();
    first_context.execute_module(&second_module).unwrap();

    assert_eq!(
        runtime.module_record(first_module.raw).unwrap().link_realm,
        Some(RawModuleLinkRealm::Other(second_context.realm))
    );
    assert_eq!(
        runtime.module_record(second_module.raw).unwrap().link_realm,
        Some(RawModuleLinkRealm::Other(first_context.realm))
    );
    assert_eq!(runtime.heap_counts().context_nodes, 2);

    drop(first_module);
    drop(second_module);
    drop(first_context);
    drop(second_context);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}

#[test]
fn loaded_module_validator_rejects_internal_sentinels_and_cache_self_edges_atomically() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context.compile_module("export const answer = 42").unwrap();
    let raw = module.raw;

    assert!(matches!(
        runtime.mutate_module_record(raw, |record| {
            record.evaluation = ModuleEvaluationState::Errored(RawValue::Exception);
            Ok(())
        }),
        Err(RuntimeError::Heap(HeapError::Invariant(
            "loaded-module record contains an internal value sentinel"
        )))
    ));
    assert!(matches!(
        runtime.module_record(raw).unwrap().evaluation,
        ModuleEvaluationState::Unevaluated
    ));

    assert!(matches!(
        runtime.mutate_module_record(raw, |record| {
            record.instance = Some(ModuleInstance {
                slots: Vec::new(),
                callable: None,
            });
            record.link_realm = Some(RawModuleLinkRealm::Other(raw.cache));
            Ok(())
        }),
        Err(RuntimeError::Heap(HeapError::Invariant(
            "loaded-module cache realm escaped through an Other link edge"
        )))
    ));
    let record = runtime.module_record(raw).unwrap();
    assert!(record.instance.is_none());
    assert!(record.link_realm.is_none());
}

#[test]
fn json_module_handle_roots_its_parse_realm_across_context_gc() {
    let runtime = Runtime::new();
    let (loader, _, _) = JsonModuleLoader::new([(
        "pkg/value.json",
        ModuleLoadResult::JsonText(r#"{"answer":1}"#.to_owned()),
    )]);
    let _loader_registration = runtime.set_module_loader(loader);
    let module = {
        let mut compilation_context = runtime.new_context();
        compilation_context
            .eval("Object.prototype.__jsonParseRealm = 41")
            .unwrap();
        compilation_context
            .compile_module_with_filename(
                r#"
                import value from "./value.json" with { type: "json" };
                globalThis.__jsonParseRealm =
                    Object.getPrototypeOf(value).__jsonParseRealm + value.answer;
                globalThis.__jsonParsePrototype = Object.getPrototypeOf(value);
                "#,
                "pkg/entry.js",
            )
            .unwrap()
    };

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 1);

    {
        let mut execution_context = runtime.new_context();
        execution_context.execute_module(&module).unwrap();
        assert_script_true(
            &mut execution_context,
            "__jsonParseRealm === 42 && __jsonParsePrototype !== Object.prototype",
        );
    }

    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 2);
    drop(module);
    runtime.run_gc().unwrap();
    assert_eq!(runtime.heap_counts().context_nodes, 0);
}

#[test]
fn module_root_stack_frame_is_anonymous_and_retains_filename() {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let module = context
        .compile_module_with_filename("throw new Error(\"x\")", "module-stack.mjs")
        .unwrap();

    let snapshot = module_evaluation_snapshot(&mut context, &module);
    assert_eq!(snapshot.state, PromiseState::Rejected);
    let RawValue::Object(error) = snapshot.result else {
        panic!("module evaluation did not reject with an Error object");
    };
    let error = ObjectRef::from_borrowed_handle(runtime.clone(), error).unwrap();
    let stack_key = runtime.intern_property_key("stack").unwrap();
    assert_eq!(
        runtime
            .raw_string_property_for_diagnostics(&error, &stack_key)
            .unwrap(),
        Some(JsString::from_static(
            "    at <anonymous> (module-stack.mjs:1:16)\n"
        ))
    );
}
