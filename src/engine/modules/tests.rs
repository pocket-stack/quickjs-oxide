use std::cell::{Cell, RefCell};

use super::*;
use crate::engine::heap::{PromiseData, PromiseState};

mod construction_tests;

mod raw_source_tests;

fn assert_eq_implemented<T: Eq>() {}

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
        _context: &mut crate::engine::api::Context,
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
        _context: &mut crate::engine::api::Context,
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

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
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
        _context: &mut crate::engine::api::Context,
        attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        if attributes.iter().all(|attribute| {
            attribute.key == JsString::from_static("type")
                && (attribute.value == JsString::from_static("json")
                    || attribute.value == JsString::from_static("json5"))
        }) {
            Ok(())
        } else {
            Err(ModuleLoaderError::new(
                "fixture JSON loader accepts only type: json or json5",
            ))
        }
    }

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
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
    fn normalize(
        &self,
        context: &mut Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.record("normalize", context);
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn check_attributes(
        &self,
        context: &mut Context,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        self.record("attributes", context);
        Ok(())
    }

    fn load(
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
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
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
        _context: &mut crate::engine::api::Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.normalizations
            .borrow_mut()
            .push((base_name.to_utf8_lossy(), specifier.to_utf8_lossy()));
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        self.sources
            .get(&normalized_name)
            .cloned()
            .map(crate::engine::api::ModuleLoadResult::SourceText)
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
        _context: &mut crate::engine::api::Context,
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
        _context: &mut crate::engine::api::Context,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        match self.failure(AbruptLoaderPhase::CheckAttributes) {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        self.loads
            .borrow_mut()
            .push(valid_fixture_module_name(normalized_name)?);
        if let Some(error) = self.failure(AbruptLoaderPhase::Load) {
            return Err(error);
        }
        Ok(crate::engine::api::ModuleLoadResult::SourceText(
            "export const answer = 42;".to_owned(),
        ))
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
        _context: &mut crate::engine::api::Context,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        if self.failing.get() {
            Err(ModuleLoaderError::exception(self.exception.clone()))
        } else {
            Ok(())
        }
    }

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        match normalized_name.as_str() {
            "pkg/dependency.js" => Ok(crate::engine::api::ModuleLoadResult::SourceText(
                "import { answer } from './leaf.js' with { type: 'javascript' }; export { answer };"
                    .to_owned())),
            "pkg/leaf.js" => Ok(crate::engine::api::ModuleLoadResult::SourceText("export const answer = 42;".to_owned())),
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
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        self.sources
            .borrow()
            .get(&normalized_name)
            .cloned()
            .map(crate::engine::api::ModuleLoadResult::SourceText)
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
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        let name = normalized_name.utf16_units().collect::<Vec<_>>();
        self.loads.borrow_mut().push(name.clone());
        self.sources
            .get(&name)
            .cloned()
            .map(crate::engine::api::ModuleLoadResult::SourceText)
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
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        self.loads.borrow_mut().push(normalized_name.clone());
        if !self.cleared.replace(true) {
            self.runtime.clear_module_loader();
        }
        self.sources
            .get(&normalized_name)
            .cloned()
            .map(crate::engine::api::ModuleLoadResult::SourceText)
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
        _context: &mut crate::engine::api::Context,
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

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        self.loads
            .borrow_mut()
            .push(valid_fixture_module_name(normalized_name)?);
        Err(ModuleLoaderError::new(
            "stale normalize loader unexpectedly handled load",
        ))
    }
}

type AttributeChecks = Rc<RefCell<Vec<Vec<(String, String)>>>>;

struct AttributeReplacingModuleLoader {
    runtime: Runtime,
    replacement: RefCell<Option<AttributeModuleLoader>>,
    replacement_registration: RefCell<Option<ModuleLoaderRegistration>>,
    checks: AttributeChecks,
}

impl fmt::Debug for AttributeReplacingModuleLoader {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AttributeReplacingModuleLoader")
    }
}

impl ModuleLoader for AttributeReplacingModuleLoader {
    fn check_attributes(
        &self,
        _context: &mut crate::engine::api::Context,
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

    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        _normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        Err(ModuleLoaderError::new(
            "stale attribute checker unexpectedly handled load",
        ))
    }
}

#[derive(Debug)]
struct PanickingModuleLoader;

impl ModuleLoader for PanickingModuleLoader {
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        _normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
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
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
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
        Ok(crate::engine::api::ModuleLoadResult::SourceText(format!(
            "import 'm{next}'; globalThis.__deepModuleRuns = (globalThis.__deepModuleRuns || 0) + 1;"
        )))
    }
}

#[derive(Debug)]
struct StarChainModuleLoader {
    module_count: usize,
}

impl ModuleLoader for StarChainModuleLoader {
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
        let normalized_name = valid_fixture_module_name(normalized_name)?;
        let index = normalized_name
            .strip_prefix('s')
            .and_then(|index| index.parse::<usize>().ok())
            .filter(|index| *index < self.module_count)
            .ok_or_else(|| ModuleLoaderError::new("invalid generated star module name"))?;
        if index + 1 == self.module_count {
            Ok(crate::engine::api::ModuleLoadResult::SourceText(
                "export const answer = 42;".to_owned(),
            ))
        } else {
            Ok(crate::engine::api::ModuleLoadResult::SourceText(format!(
                "export * from 's{}';",
                index + 1
            )))
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
    fn load(
        &self,
        _context: &mut crate::engine::api::Context,
        _normalized_name: &JsString,
        _attributes: &crate::engine::api::ModuleImportAttributes,
    ) -> Result<crate::engine::api::ModuleLoadResult, ModuleLoaderError> {
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
        if !runtime.execute_pending_job().unwrap().executed() {
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

mod identity;

mod loader_exceptions;

mod dynamic_import;

mod dynamic_import_attributes;

mod dynamic_import_cache;

mod loader_attributes;

mod json;

mod loader_lifecycle;

mod resolution_rollback;

mod live_bindings;

mod import_meta;

mod export_resolution;

mod namespace;

mod graph_evaluation;

mod top_level_await;

mod evaluation;

mod ownership;
