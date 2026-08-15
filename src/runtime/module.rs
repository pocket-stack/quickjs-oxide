//! Publication and execution for static ECMAScript modules.
//!
//! QuickJS publishes a `JSModuleDef` separately from the bytecode function it
//! drives. This slice keeps that ownership boundary across Context-local
//! caching, host resolution, live import cells, and iterative SCC
//! linking/evaluation. Static namespace objects and transitive exports are
//! included. Script-goal dynamic import shares the same loader, linker,
//! evaluator, namespace machinery, and top-level-await scheduling.

use super::*;
use crate::compiler::{
    CompileOptions, ModuleCompileFailure, ModuleImportAttributeChecker,
    compile_unlinked_module_with_name_and_attribute_checker,
};
use crate::heap::PromiseState;
use crate::module::{
    ModuleExportTarget, ModuleImportName, ModuleRequest, ModuleRequestIndex, UnlinkedModule,
};
pub use crate::module::{ModuleImportAttribute, ModuleImportAttributes};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// Failure reported by an embedder-provided module-host callback.
///
/// Message failures retain the Rust convenience API's native-error policy.
/// [`Self::exception`] instead models QuickJS `JS_Throw`: its JavaScript value
/// is propagated unchanged through static compilation or dynamic-import
/// rejection, including object and Symbol identity.
#[derive(Clone, Debug)]
pub struct ModuleLoaderError {
    kind: ModuleLoaderErrorKind,
}

#[derive(Clone, Debug)]
enum ModuleLoaderErrorKind {
    Message(String),
    Exception(Value),
}

impl ModuleLoaderError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            kind: ModuleLoaderErrorKind::Message(message.into()),
        }
    }

    /// Return an abrupt JavaScript completion carrying `exception` exactly.
    #[must_use]
    pub const fn exception(exception: Value) -> Self {
        Self {
            kind: ModuleLoaderErrorKind::Exception(exception),
        }
    }

    /// Return a stable human-readable description.
    ///
    /// For JavaScript-valued abrupt completions this returns
    /// `"JavaScript exception"`; use [`Self::exception_value`] to inspect the
    /// exact value or [`Self::message_text`] to distinguish both forms.
    #[must_use]
    pub fn message(&self) -> &str {
        match &self.kind {
            ModuleLoaderErrorKind::Message(message) => message,
            ModuleLoaderErrorKind::Exception(_) => "JavaScript exception",
        }
    }

    /// Return the host diagnostic text, if this is a message failure.
    #[must_use]
    pub fn message_text(&self) -> Option<&str> {
        match &self.kind {
            ModuleLoaderErrorKind::Message(message) => Some(message),
            ModuleLoaderErrorKind::Exception(_) => None,
        }
    }

    /// Borrow the exact JavaScript exception value, if one was supplied.
    #[must_use]
    pub const fn exception_value(&self) -> Option<&Value> {
        match &self.kind {
            ModuleLoaderErrorKind::Message(_) => None,
            ModuleLoaderErrorKind::Exception(exception) => Some(exception),
        }
    }
}

impl fmt::Display for ModuleLoaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl PartialEq for ModuleLoaderError {
    fn eq(&self, other: &Self) -> bool {
        match (&self.kind, &other.kind) {
            (ModuleLoaderErrorKind::Message(left), ModuleLoaderErrorKind::Message(right)) => {
                left == right
            }
            (ModuleLoaderErrorKind::Exception(left), ModuleLoaderErrorKind::Exception(right)) => {
                left.same_quickjs_representation(right)
            }
            _ => false,
        }
    }
}

// Equality is representation-based for exception values, including NaN
// payload bits, so it is reflexive and preserves the public pre-exception API
// bound.
impl Eq for ModuleLoaderError {}

impl StdError for ModuleLoaderError {}

impl From<&str> for ModuleLoaderError {
    fn from(message: &str) -> Self {
        Self::new(message)
    }
}

impl From<String> for ModuleLoaderError {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// Extensible result returned by the attributes-aware module loader boundary.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ModuleLoadResult {
    SourceText(String),
    /// A module compiled by the callback's initiating [`Context`].
    ///
    /// This is the safe Rust equivalent of QuickJS's loader returning the
    /// `JSModuleDef *` produced by a nested compile-only evaluation. The
    /// module must belong to the same Runtime and Context cache as the
    /// callback which consumes it.
    Compiled(ModuleBytecodeRef),
    /// Source text plus host-defined properties for that module's canonical
    /// `import.meta` object.
    ///
    /// Properties are installed as writable, enumerable, configurable data
    /// properties before the visible parse-in-progress definition completes
    /// atomically as executable source text. An empty vector still
    /// materializes `import.meta`, matching a host call to QuickJS
    /// `JS_GetImportMeta`.
    SourceTextWithImportMeta {
        source: String,
        properties: Vec<ModuleImportMetaProperty>,
    },
    /// Strict JSON source used to create a synthetic module with one
    /// `default` export. The host, not the engine, decides which requests are
    /// JSON; this variant deliberately carries no filename-extension policy.
    JsonText(String),
    /// QuickJS extended-JSON source used to create a synthetic module with one
    /// `default` export. This is the exact `JS_PARSE_JSON_EXT` grammar selected
    /// by QuickJS's file loader for `type: "json5"`; the host still owns all
    /// extension and import-attribute policy.
    Json5Text(String),
}

/// One host-defined data property for a source module's `import.meta` object.
///
/// The exact UTF-16 key and JavaScript value are preserved. Object and Symbol
/// values must belong to the Runtime invoking the loader.
#[derive(Clone, Debug, PartialEq)]
pub struct ModuleImportMetaProperty {
    key: JsString,
    value: Value,
}

impl ModuleImportMetaProperty {
    #[must_use]
    pub const fn new(key: JsString, value: Value) -> Self {
        Self { key, value }
    }

    #[must_use]
    pub const fn key(&self) -> &JsString {
        &self.key
    }

    #[must_use]
    pub const fn value(&self) -> &Value {
        &self.value
    }
}

/// Runtime-wide host boundary for module normalization and loading.
///
/// The loaded-module cache itself is Context-owned, matching QuickJS. The
/// loader is called synchronously during module compilation/resolution. Its
/// Context-aware hooks may re-enter the same Runtime and compile a module to
/// return as [`ModuleLoadResult::Compiled`], matching QuickJS's loader model.
pub trait ModuleLoader: fmt::Debug {
    /// Normalize `specifier` relative to `base_name` before cache lookup.
    ///
    /// These are ECMAScript Strings rather than Rust UTF-8 strings. Hosts
    /// which expose filesystem names can explicitly encode them with
    /// [`JsString::try_to_wtf8_bytes`] without aliasing lone surrogates. As at
    /// QuickJS's C-string callback boundary, text after an embedded NUL is not
    /// presented to either loader callback.
    fn normalize(
        &self,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        default_module_normalize_name(base_name, specifier)
            .map_err(|error| ModuleLoaderError::new(error.to_string()))
    }

    /// Context-aware normalization hook used by the runtime.
    ///
    /// Existing loaders remain source-compatible through this adapter. New
    /// hosts may override it to receive the exact initiating Context needed
    /// by synchronous nested module compilation.
    fn normalize_in_context(
        &self,
        _context: &mut Context,
        base_name: &JsString,
        specifier: &JsString,
    ) -> Result<JsString, ModuleLoaderError> {
        self.normalize(base_name, specifier)
    }

    /// Validate one request's attributes before normalization, cache lookup,
    /// or any following source text. Static syntax calls this only for a
    /// non-empty effective `with {}` object; dynamic import calls it whenever
    /// `options.with` is present, including an empty object, matching the two
    /// distinct QuickJS construction paths. For static syntax, the initiating
    /// Context cache already exposes the parse-in-progress module and the
    /// source-order request prefix through the current clause, so synchronous
    /// same-Context compilation and resolution can observe that identity.
    fn check_attributes(
        &self,
        _attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        Ok(())
    }

    /// Context-aware import-attribute hook used by the runtime.
    fn check_attributes_in_context(
        &self,
        _context: &mut Context,
        attributes: &[ModuleImportAttribute],
    ) -> Result<(), ModuleLoaderError> {
        self.check_attributes(attributes)
    }

    /// Legacy source-text loader entry point.
    ///
    /// Existing loaders can continue to implement only this method; the
    /// default [`Self::load_with_attributes`] adapter preserves their behavior.
    /// Attributes-aware loaders may instead override `load_with_attributes`.
    fn load(&self, _normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
        Err(ModuleLoaderError::new(
            "module loader does not implement source-text loading",
        ))
    }

    /// Load one cache-missing normalized module with the attributes from the
    /// request which selected it. Static authored `with {}` is collapsed
    /// through [`ModuleImportAttributes::effective`]; dynamic `options.with`
    /// retains [`ModuleImportAttributes::Present`] even when empty.
    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        self.load(normalized_name).map(ModuleLoadResult::SourceText)
    }

    /// Context-aware load hook used by the runtime.
    ///
    /// Override this method to return [`ModuleLoadResult::Compiled`] from a
    /// nested compile performed through `context`. The default preserves all
    /// legacy source-text and attributes-aware loaders.
    fn load_with_attributes_in_context(
        &self,
        _context: &mut Context,
        normalized_name: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        self.load_with_attributes(normalized_name, attributes)
    }
}

/// Host-owned lifetime token for an installed [`ModuleLoader`].
///
/// The Runtime keeps only a weak reference, matching QuickJS's host-owned
/// loader opaque and preventing `Runtime -> loader -> Runtime` reference
/// cycles. Keep this value alive for as long as module resolution should use
/// the loader. Dropping it disables the loader once no other registration
/// owns it. Dropping this token or calling [`Runtime::clear_module_loader`]
/// disables the next module-host callback, including a later callback in a
/// resolution already in flight. Each normalize, load, and attribute-check
/// invocation samples the then-current registration independently, matching
/// QuickJS's runtime callback and opaque lookup boundaries.
#[must_use = "the module loader is active only while its registration is retained"]
pub struct ModuleLoaderRegistration {
    _loader: Rc<dyn ModuleLoader>,
}

impl fmt::Debug for ModuleLoaderRegistration {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleLoaderRegistration")
            .finish_non_exhaustive()
    }
}

fn module_c_string_view(value: &JsString) -> Result<JsString, JsStringError> {
    let Some(nul) = value.utf16_units().position(|unit| unit == 0) else {
        return Ok(value.clone());
    };
    JsString::try_from_utf16(value.utf16_units().take(nul))
}

fn module_reference_error(prefix: &str, name: &JsString, suffix: &str) -> RuntimeError {
    let mut message = NativeErrorMessage::new();
    message.push_utf8(prefix);
    name.push_c_string_to(&mut message);
    message.push_utf8(suffix);
    Error::from_native_message(ErrorKind::Reference, message).into()
}

fn module_export_error_message(
    prefix: &str,
    export_name: &JsString,
    module_name: &JsString,
) -> NativeErrorMessage {
    let mut message = NativeErrorMessage::new();
    message.push_utf8(prefix);
    export_name.push_atom_get_str_to(&mut message);
    message.push_utf8("' in module '");
    module_name.push_atom_get_str_to(&mut message);
    message.push_utf8("'");
    message
}

fn default_module_normalize_name(
    base_name: &JsString,
    specifier: &JsString,
) -> Result<JsString, JsStringError> {
    const DOT: u16 = b'.' as u16;
    const SLASH: u16 = b'/' as u16;

    let base_name = module_c_string_view(base_name)?;
    let specifier = module_c_string_view(specifier)?;
    let specifier_units = specifier.utf16_units().collect::<Vec<_>>();
    if specifier_units.first().copied() != Some(DOT) {
        return Ok(specifier.clone());
    }

    let base_units = base_name.utf16_units().collect::<Vec<_>>();
    let mut base = base_units
        .iter()
        .rposition(|unit| *unit == SLASH)
        .map_or_else(Vec::new, |slash| base_units[..slash].to_vec());
    let mut rest_start = 0;
    loop {
        let rest = &specifier_units[rest_start..];
        if rest.starts_with(&[DOT, SLASH]) {
            rest_start += 2;
            continue;
        }
        if !rest.starts_with(&[DOT, DOT, SLASH]) {
            break;
        }
        if base.is_empty() {
            break;
        }
        let component_start = base
            .iter()
            .rposition(|unit| *unit == SLASH)
            .map_or(0, |slash| slash + 1);
        let component = &base[component_start..];
        if component == [DOT] || component == [DOT, DOT] {
            break;
        }
        base.truncate(component_start.saturating_sub(1));
        rest_start += 3;
    }
    if base.is_empty() {
        JsString::try_from_utf16(specifier_units[rest_start..].iter().copied())
    } else {
        base.push(SLASH);
        base.extend_from_slice(&specifier_units[rest_start..]);
        JsString::try_from_utf16(base)
    }
}

/// Opaque owning handle for one runtime-published ECMAScript module record.
///
/// Clones preserve module identity and therefore share link/evaluation state.
/// The defining Context cache remains rooted for as long as any handle
/// survives; that cache owns every raw edge of the module graph.
pub struct ModuleBytecodeRef {
    runtime: Runtime,
    raw: RawModuleRef,
    name: JsString,
}

impl Clone for ModuleBytecodeRef {
    fn clone(&self) -> Self {
        self.runtime
            .retain_context_handle(self.raw.cache)
            .expect("a live module handle must retain its defining cache");
        Self {
            runtime: self.runtime.clone(),
            raw: self.raw,
            name: self.name.clone(),
        }
    }
}

impl Drop for ModuleBytecodeRef {
    fn drop(&mut self) {
        self.runtime.release_context_handle(self.raw.cache);
    }
}

type ModuleRecord = RawModuleRecord;
type ModuleRecordBody = RawModuleRecordBody;
type PublishedModuleExport = RawPublishedModuleExport;
type PublishedModuleExportTarget = RawPublishedModuleExportTarget;
type ModuleResolutionState = RawModuleResolutionState;
type ModuleInstance = RawModuleInstance;
type ModuleNamespaceState = RawModuleNamespaceState;
type ModuleLinkStatus = RawModuleLinkStatus;
type ModuleEvaluationState = RawModuleEvaluationState;

#[derive(Clone, Copy)]
struct ModuleDfsEntry {
    index: usize,
    ancestor: usize,
}

struct ModuleLinkDfs {
    next_index: usize,
    stack: Vec<ModuleId>,
    entries: HashMap<ModuleId, ModuleDfsEntry>,
}

struct ModuleEvaluationDfs {
    next_index: usize,
    stack: Vec<ModuleId>,
    entries: HashMap<ModuleId, ModuleDfsEntry>,
    exception: Option<Value>,
}

struct ModuleResolveFrame {
    module: RawModuleRef,
    next_request: usize,
    dependencies: Vec<ModuleId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleExportResolveResultKind {
    NotFound,
    Circular,
    Ambiguous,
}

#[derive(Clone)]
enum ModuleResolvedBindingTarget {
    Local { closure_index: u16 },
    Namespace { export_index: usize },
}

#[derive(Clone)]
struct ModuleResolvedBinding {
    module: RawModuleRef,
    target: ModuleResolvedBindingTarget,
}

impl ModuleResolvedBinding {
    fn has_same_identity(&self, other: &Self) -> bool {
        if self.module != other.module {
            return false;
        }
        match (&self.target, &other.target) {
            (
                ModuleResolvedBindingTarget::Local {
                    closure_index: left,
                },
                ModuleResolvedBindingTarget::Local {
                    closure_index: right,
                },
            ) => left == right,
            (
                ModuleResolvedBindingTarget::Namespace { .. },
                ModuleResolvedBindingTarget::Namespace { .. },
            ) => true,
            (
                ModuleResolvedBindingTarget::Local { .. },
                ModuleResolvedBindingTarget::Namespace { .. },
            )
            | (
                ModuleResolvedBindingTarget::Namespace { .. },
                ModuleResolvedBindingTarget::Local { .. },
            ) => false,
        }
    }
}

enum ModuleExportResolveResult {
    Found(ModuleResolvedBinding),
    NotFound,
    Circular,
    Ambiguous,
}

impl ModuleExportResolveResult {
    const fn error_kind(&self) -> Option<ModuleExportResolveResultKind> {
        match self {
            Self::Found(_) => None,
            Self::NotFound => Some(ModuleExportResolveResultKind::NotFound),
            Self::Circular => Some(ModuleExportResolveResultKind::Circular),
            Self::Ambiguous => Some(ModuleExportResolveResultKind::Ambiguous),
        }
    }
}

enum ModuleExportResolveFrameState {
    Enter,
    AwaitIndirect,
    Stars {
        next_star: usize,
        found: Option<ModuleResolvedBinding>,
    },
}

struct ModuleExportResolveFrame {
    module: RawModuleRef,
    export_name: JsString,
    state: ModuleExportResolveFrameState,
}

struct ModuleExportNamesFrame {
    module: RawModuleRef,
    from_star: bool,
    entered: bool,
    next_star: usize,
}

struct ModuleDfsFrame {
    module: RawModuleRef,
    dependencies: Vec<RawModuleRef>,
    next_dependency: usize,
}

enum ModuleEvaluationVisit {
    Unevaluated,
    Evaluating,
    EvaluatingAsync,
    Evaluated,
    Errored(Value),
    Poisoned,
}

struct ModuleLoaderAttributeChecker<'a> {
    runtime: &'a Runtime,
    context: Context,
    parsing_module: RawModuleRef,
    runtime_failure: Option<RuntimeError>,
}

enum ModuleHostCallbackOutcome<T> {
    Completed(Result<T, ModuleLoaderError>),
    Throw(Value),
}

impl ModuleImportAttributeChecker for ModuleLoaderAttributeChecker<'_> {
    fn publish_request(&mut self, request: &ModuleRequest) -> Result<(), ModuleCompileFailure> {
        if self.runtime_failure.is_some() {
            return Err(Error::internal("module request publication failed").into());
        }
        if let Err(error) = self
            .runtime
            .append_parsing_module_request(self.parsing_module, request.clone())
        {
            self.runtime_failure = Some(error);
            return Err(Error::internal("module request publication failed").into());
        }
        Ok(())
    }

    fn check(&mut self, attributes: &[ModuleImportAttribute]) -> Result<(), ModuleCompileFailure> {
        if self.runtime_failure.is_some() {
            return Err(Error::internal("module attribute host callback failed").into());
        }
        let loader = self.runtime.current_module_loader();
        let Some(loader) = loader else {
            return Ok(());
        };
        let outcome = match self
            .runtime
            .invoke_module_host_callback(&mut self.context, |context| {
                loader.check_attributes_in_context(context, attributes)
            }) {
            Ok(outcome) => outcome,
            Err(error) => {
                self.runtime_failure = Some(error);
                return Err(Error::internal("module attribute host callback failed").into());
            }
        };
        match outcome {
            ModuleHostCallbackOutcome::Throw(exception) => {
                Err(ModuleCompileFailure::Throw(exception))
            }
            ModuleHostCallbackOutcome::Completed(Ok(())) => Ok(()),
            ModuleHostCallbackOutcome::Completed(Err(ModuleLoaderError {
                kind: ModuleLoaderErrorKind::Message(message),
            })) => Err(Error::new(ErrorKind::Type, message).into()),
            ModuleHostCallbackOutcome::Completed(Err(ModuleLoaderError {
                kind: ModuleLoaderErrorKind::Exception(exception),
            })) => Err(ModuleCompileFailure::Throw(exception)),
        }
    }
}

impl ModuleLinkDfs {
    fn new() -> Self {
        Self {
            next_index: 0,
            stack: Vec::new(),
            entries: HashMap::new(),
        }
    }
}

impl ModuleEvaluationDfs {
    fn new() -> Self {
        Self {
            next_index: 0,
            stack: Vec::new(),
            entries: HashMap::new(),
            exception: None,
        }
    }
}

enum ModuleCompilation {
    Published(RawModuleRef),
    Throw(Value),
}

impl ModuleBytecodeRef {
    /// Return the source/debug name attached to this module record.
    #[must_use]
    pub fn name(&self) -> &JsString {
        &self.name
    }

    /// Return whether this module was published by `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.runtime.is_same_runtime(runtime)
    }

    /// Return whether two handles name modules in the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.runtime.is_same_runtime(&other.runtime)
    }

    /// Stable identity of the runtime domain which published this module.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.runtime.domain_id()
    }
}

impl PartialEq for ModuleBytecodeRef {
    fn eq(&self, other: &Self) -> bool {
        self.runtime.is_same_runtime(&other.runtime) && self.raw == other.raw
    }
}

impl Eq for ModuleBytecodeRef {}

impl fmt::Debug for ModuleBytecodeRef {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ModuleBytecodeRef")
            .field("domain_id", &self.domain_id())
            .field("name", &self.name())
            .finish_non_exhaustive()
    }
}

impl Runtime {
    fn current_module_loader(&self) -> Option<Rc<dyn ModuleLoader>> {
        self.0
            .module_loader
            .borrow()
            .as_ref()
            .and_then(Weak::upgrade)
    }

    fn module_callback_context(&self, realm: ContextId) -> Result<Context, RuntimeError> {
        let id =
            self.0
                .state
                .borrow()
                .heap
                .context(realm)?
                .public_id
                .ok_or(RuntimeError::Invariant(
                    "module callback realm has no public Context identity",
                ))?;
        self.retain_context_handle(realm)?;
        Ok(Context {
            runtime: self.clone(),
            id,
            realm,
        })
    }

    fn invoke_module_host_callback<T>(
        &self,
        context: &mut Context,
        callback: impl FnOnce(&mut Context) -> Result<T, ModuleLoaderError>,
    ) -> Result<ModuleHostCallbackOutcome<T>, RuntimeError> {
        let Ok(_guard) = super::native_stack::ModuleHostCallbackGuard::enter(self) else {
            return Ok(ModuleHostCallbackOutcome::Throw(self.new_native_error(
                context.realm,
                NativeErrorKind::Internal,
                "stack overflow",
            )?));
        };
        Ok(ModuleHostCallbackOutcome::Completed(callback(context)))
    }

    fn propagate_module_host_throw<T>(
        &self,
        outcome: ModuleHostCallbackOutcome<T>,
    ) -> Result<Result<T, ModuleLoaderError>, RuntimeError> {
        match outcome {
            ModuleHostCallbackOutcome::Completed(result) => Ok(result),
            ModuleHostCallbackOutcome::Throw(exception) => {
                self.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    fn finish_module_loader_error(
        &self,
        error: ModuleLoaderError,
        message: impl FnOnce(&str) -> RuntimeError,
    ) -> Result<RuntimeError, RuntimeError> {
        match error.kind {
            ModuleLoaderErrorKind::Message(text) => Ok(message(&text)),
            ModuleLoaderErrorKind::Exception(exception) => {
                self.validate_value_domain(&exception, "module loader exception")?;
                self.set_pending_exception(exception)?;
                Ok(RuntimeError::Exception)
            }
        }
    }

    /// Install the runtime-wide module loader used by subsequent Context
    /// module resolution. Existing Context caches remain intact.
    pub fn set_module_loader<L>(&self, loader: L) -> ModuleLoaderRegistration
    where
        L: ModuleLoader + 'static,
    {
        let loader: Rc<dyn ModuleLoader> = Rc::new(loader);
        *self.0.module_loader.borrow_mut() = Some(Rc::downgrade(&loader));
        ModuleLoaderRegistration { _loader: loader }
    }

    /// Remove the runtime-wide module loader without clearing Context caches.
    pub fn clear_module_loader(&self) {
        self.0.module_loader.borrow_mut().take();
    }

    /// Dynamic-import's schedule-time attribute checker. The current loader
    /// is sampled here, independently from the later load job. Only the host
    /// callback is guarded: all preceding JavaScript property operations stay
    /// ordinarily re-entrant.
    pub(super) fn check_dynamic_import_attributes(
        &self,
        realm: ContextId,
        attributes: &[ModuleImportAttribute],
    ) -> Result<NativeConversion<()>, RuntimeError> {
        let loader = self.current_module_loader();
        let Some(loader) = loader else {
            return Ok(NativeConversion::Value(()));
        };
        let mut context = self.module_callback_context(realm)?;
        match self.invoke_module_host_callback(&mut context, |context| {
            loader.check_attributes_in_context(context, attributes)
        })? {
            ModuleHostCallbackOutcome::Throw(exception) => Ok(NativeConversion::Throw(exception)),
            ModuleHostCallbackOutcome::Completed(Ok(())) => Ok(NativeConversion::Value(())),
            ModuleHostCallbackOutcome::Completed(Err(ModuleLoaderError {
                kind: ModuleLoaderErrorKind::Message(message),
            })) => Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                &message,
            )?)),
            ModuleHostCallbackOutcome::Completed(Err(ModuleLoaderError {
                kind: ModuleLoaderErrorKind::Exception(exception),
            })) => {
                self.validate_value_domain(&exception, "module loader exception")?;
                Ok(NativeConversion::Throw(exception))
            }
        }
    }

    fn module_record(&self, module: RawModuleRef) -> Result<ModuleRecord, RuntimeError> {
        let state = self.0.state.borrow();
        if !state.heap.loaded_module_is_live(module)? {
            return Err(RuntimeError::AbortedModule);
        }
        Ok(state.heap.loaded_module(module)?)
    }

    /// Append QuickJS's initially empty `JSModuleDef` to this Context before
    /// the parser consumes any source token. The record owns no arena edge and
    /// is deliberately non-executable; source-order requests are the only
    /// fields which may grow before atomic completion.
    fn publish_parsing_module_record(
        &self,
        realm: ContextId,
        name: JsString,
    ) -> Result<RawModuleRef, RuntimeError> {
        self.publish_module_record(
            realm,
            ModuleRecord {
                name,
                body: ModuleRecordBody::Parsing,
                import_meta: None,
                declaration_order: Rc::from([]),
                link_initializers: Rc::from([]),
                import_collisions: Rc::from([]),
                requested_modules: Rc::new(Vec::new()),
                imports: Rc::from([]),
                exports: Rc::from([]),
                star_exports: Rc::from([]),
                resolution: ModuleResolutionState::Unresolved,
                instance: None,
                namespace: ModuleNamespaceState::Empty,
                link_status: ModuleLinkStatus::Unlinked,
                evaluation: ModuleEvaluationState::Unevaluated,
                has_top_level_await: false,
                evaluation_cycle_root: None,
                evaluation_promise: None,
                evaluation_resolve: None,
                evaluation_reject: None,
                pending_async_dependencies: 0,
                async_parent_modules: Vec::new(),
                async_evaluation_order: None,
                link_realm: None,
                compile_realm: realm,
            },
        )
    }

    fn append_parsing_module_request(
        &self,
        module: RawModuleRef,
        request: ModuleRequest,
    ) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .append_parsing_module_request(module, request)?;
        Ok(())
    }

    fn abort_parsing_module(&self, module: RawModuleRef) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state.heap.abort_parsing_loaded_module(module)?;
        state.apply_committed_cleanup(cleanup);
        Ok(())
    }

    /// Allocate and populate one fresh null-prototype import-meta object before
    /// the parse-in-progress record completes as executable source text.
    /// Keeping the public Values rooted until every descriptor has committed
    /// makes a failed host initialization leave no live executable cache entry;
    /// its append-only construction slot may remain as a rollback tombstone.
    fn new_module_import_meta(
        &self,
        properties: Vec<ModuleImportMetaProperty>,
    ) -> Result<ObjectRef, RuntimeError> {
        let meta = self.new_object(None)?;
        for property in properties {
            let key = self.intern_property_key_js_string(&property.key)?;
            let defined = self.define_own_property(
                &meta,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(property.value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )?;
            if !defined {
                return Err(RuntimeError::Invariant(
                    "fresh import.meta property definition was rejected",
                ));
            }
        }
        Ok(meta)
    }

    /// Rust counterpart of QuickJS `JS_GetImportMeta`: lazily allocate one
    /// canonical null-prototype object and publish it as a module-owned edge.
    fn get_or_create_module_import_meta(
        &self,
        module: RawModuleRef,
    ) -> Result<ObjectRef, RuntimeError> {
        if let Some(meta) = self.module_record(module)?.import_meta {
            return Ok(ObjectRef::from_borrowed_handle(self.clone(), meta)?);
        }

        let meta = self.new_object(None)?;
        self.mutate_module_record(module, |record| {
            if record.import_meta.is_some() {
                return Err(RuntimeError::Invariant(
                    "module import.meta was published during initialization",
                ));
            }
            record.import_meta = Some(meta.object_id());
            Ok(())
        })?;
        Ok(meta)
    }

    fn root_module(&self, raw: RawModuleRef) -> Result<ModuleBytecodeRef, RuntimeError> {
        let name = self.module_record(raw)?.name;
        self.retain_context_handle(raw.cache)?;
        Ok(ModuleBytecodeRef {
            runtime: self.clone(),
            raw,
            name,
        })
    }

    fn module_value_atoms(record: &ModuleRecord) -> Vec<Atom> {
        let mut atoms = Vec::with_capacity(2);
        if let ModuleRecordBody::Json {
            default_value: RawValue::Symbol(atom) | RawValue::Private(atom),
        } = &record.body
        {
            atoms.push(*atom);
        }
        if let ModuleEvaluationState::Errored(RawValue::Symbol(atom) | RawValue::Private(atom)) =
            &record.evaluation
        {
            atoms.push(*atom);
        }
        atoms
    }

    fn module_value_atom_delta(
        current: &ModuleRecord,
        replacement: &ModuleRecord,
    ) -> (Vec<Atom>, Vec<Atom>) {
        let mut old = Self::module_value_atoms(current);
        let new = Self::module_value_atoms(replacement);
        let mut added = Vec::with_capacity(new.len());
        for atom in new {
            if let Some(index) = old.iter().position(|candidate| *candidate == atom) {
                old.swap_remove(index);
            } else {
                added.push(atom);
            }
        }
        (added, old)
    }

    fn retain_module_atoms(
        state: &mut RuntimeState,
        atoms: Vec<Atom>,
    ) -> Result<Vec<Atom>, RuntimeError> {
        for (retained, &atom) in atoms.iter().enumerate() {
            if let Err(error) = state.atoms.retain(atom) {
                state.release_atoms(atoms[..retained].iter().copied())?;
                return Err(error.into());
            }
        }
        Ok(atoms)
    }

    fn publish_module_record(
        &self,
        cache: ContextId,
        record: ModuleRecord,
    ) -> Result<RawModuleRef, RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let atoms = Self::module_value_atoms(&record);
        let retained_atoms = Self::retain_module_atoms(&mut state, atoms)?;
        match state.heap.publish_loaded_module(cache, record) {
            Ok(module) => Ok(module),
            Err(error) => {
                state
                    .release_atoms(retained_atoms)
                    .expect("loaded-module atom rollback failed after rejected publication");
                Err(error.into())
            }
        }
    }

    fn replace_module_record(
        &self,
        module: RawModuleRef,
        replacement: ModuleRecord,
    ) -> Result<(), RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let current = state.heap.loaded_module(module)?;
        let (added_atoms, removed_atoms) = Self::module_value_atom_delta(&current, &replacement);
        state.preflight_atom_releases(&removed_atoms)?;
        let retained_atoms = Self::retain_module_atoms(&mut state, added_atoms)?;
        match state.heap.replace_loaded_module(module, replacement) {
            Ok(cleanup) => {
                debug_assert!(cleanup.atoms.starts_with(&removed_atoms));
                state.apply_committed_cleanup(cleanup);
                Ok(())
            }
            Err(error) => {
                state
                    .release_atoms(retained_atoms)
                    .expect("loaded-module atom rollback failed after rejected replacement");
                Err(error.into())
            }
        }
    }

    fn mutate_module_record<T>(
        &self,
        module: RawModuleRef,
        mutate: impl FnOnce(&mut ModuleRecord) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        let mut replacement = self.module_record(module)?;
        let result = mutate(&mut replacement)?;
        self.replace_module_record(module, replacement)?;
        Ok(result)
    }

    fn transition_module_record(
        &self,
        module: RawModuleRef,
        transition: RawModuleTransition,
    ) -> Result<(), RuntimeError> {
        self.0
            .state
            .borrow_mut()
            .heap
            .transition_loaded_module(module, transition)?;
        Ok(())
    }

    fn unpublish_failed_resolution(
        &self,
        cache: ContextId,
        seeds: impl IntoIterator<Item = ModuleId>,
    ) -> Result<(), RuntimeError> {
        let mut doomed = seeds.into_iter().collect::<HashSet<_>>();
        if doomed.is_empty() {
            return Err(RuntimeError::Invariant(
                "failed module resolution had no records to roll back",
            ));
        }
        loop {
            let records = self.0.state.borrow().heap.loaded_modules(cache)?;
            let mut changed = false;
            for (id, record) in records {
                if doomed.contains(&id) {
                    continue;
                }
                let depends_on_doomed = match &record.resolution {
                    ModuleResolutionState::Resolved(dependencies) => dependencies
                        .iter()
                        .any(|dependency| doomed.contains(dependency)),
                    ModuleResolutionState::Unresolved
                    | ModuleResolutionState::Resolving
                    | ModuleResolutionState::Failed => false,
                };
                if depends_on_doomed {
                    match &record.body {
                        ModuleRecordBody::SourceText { .. } | ModuleRecordBody::Json { .. } => {
                            doomed.insert(id);
                            changed = true;
                        }
                        ModuleRecordBody::Parsing => {
                            self.transition_module_record(
                                RawModuleRef { cache, module: id },
                                RawModuleTransition::FailResolution,
                            )?;
                            changed = true;
                        }
                        ModuleRecordBody::Aborted => {
                            return Err(RuntimeError::Invariant(
                                "failed-resolution closure reached an aborted identity",
                            ));
                        }
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut doomed = doomed.into_iter().collect::<Vec<_>>();
        doomed.sort_unstable_by_key(|id| std::cmp::Reverse(id.0));
        let mut state = self.0.state.borrow_mut();
        let removed_atoms = doomed
            .iter()
            .map(|id| {
                state
                    .heap
                    .loaded_module(RawModuleRef { cache, module: *id })
            })
            .collect::<Result<Vec<_>, _>>()?
            .iter()
            .flat_map(Self::module_value_atoms)
            .collect::<Vec<_>>();
        state.preflight_atom_releases(&removed_atoms)?;
        let cleanup = state.heap.unpublish_loaded_modules(cache, &doomed)?;
        debug_assert!(cleanup.atoms.starts_with(&removed_atoms));
        state.apply_committed_cleanup(cleanup);
        Ok(())
    }

    /// Compile and publish a static module without touching the runtime's
    /// pending-exception slot. The public Context boundary installs a thrown
    /// syntax exception exactly as the Script compilation path does.
    fn compile_module_record_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        name: &JsString,
        import_meta_properties: Option<Vec<ModuleImportMetaProperty>>,
    ) -> Result<ModuleCompilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let parsing_module = self.publish_parsing_module_record(realm, name.clone())?;
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.finish_parsing_module_compilation(
                realm,
                parsing_module,
                source,
                name,
                import_meta_properties,
            )
        }));
        match outcome {
            Ok(Ok(ModuleCompilation::Published(module))) if module == parsing_module => {
                Ok(ModuleCompilation::Published(module))
            }
            Ok(Ok(ModuleCompilation::Published(_))) => {
                self.abort_parsing_module(parsing_module)
                    .unwrap_or_else(|error| panic!("module construction rollback failed: {error}"));
                Err(RuntimeError::Invariant(
                    "module construction completed another cache identity",
                ))
            }
            Ok(result) => {
                self.abort_parsing_module(parsing_module)
                    .unwrap_or_else(|error| panic!("module construction rollback failed: {error}"));
                result
            }
            Err(payload) => {
                self.abort_parsing_module(parsing_module)
                    .unwrap_or_else(|error| panic!("module construction rollback failed: {error}"));
                resume_unwind(payload)
            }
        }
    }

    fn finish_parsing_module_compilation(
        &self,
        realm: ContextId,
        parsing_module: RawModuleRef,
        source: &str,
        name: &JsString,
        import_meta_properties: Option<Vec<ModuleImportMetaProperty>>,
    ) -> Result<ModuleCompilation, RuntimeError> {
        let debug_info = self.debug_info_mode();
        // QuickJS samples the runtime's attribute checker separately for
        // every authored `with` clause, so callbacks may replace or clear it
        // before the parser reaches the next clause.
        let mut checker = ModuleLoaderAttributeChecker {
            runtime: self,
            context: self.module_callback_context(realm)?,
            parsing_module,
            runtime_failure: None,
        };
        let compilation = compile_unlinked_module_with_name_and_attribute_checker(
            source,
            name.clone(),
            debug_info,
            Some(&mut checker),
        );
        if let Some(error) = checker.runtime_failure.take() {
            return Err(error);
        }
        let module = match compilation {
            Ok(module) => module,
            Err(ModuleCompileFailure::Throw(exception)) => {
                self.validate_value_domain(&exception, "module loader exception")?;
                return Ok(ModuleCompilation::Throw(exception));
            }
            Err(ModuleCompileFailure::Engine(error)) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                let explicit_location = if error.kind() == ErrorKind::Syntax {
                    if let Some(span) = error.span() {
                        let position = QuickJsSourceLocator::new(source)
                            .locate_byte_offset(span.start.byte_offset)
                            .map_err(|_| {
                                RuntimeError::Invariant(
                                    "syntax-error byte offset is invalid for its source",
                                )
                            })?;
                        Some(ExplicitBacktraceLocation {
                            filename: name.clone(),
                            position,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let exception = if error.kind() == ErrorKind::Syntax {
                    self.new_native_error_without_backtrace_from_error(realm, kind, &error)?
                } else {
                    self.new_native_error_from_error(realm, kind, &error)?
                };
                self.ensure_error_backtrace(&exception, false, explicit_location)?;
                return Ok(ModuleCompilation::Throw(exception));
            }
        };
        let import_meta = import_meta_properties
            .map(|properties| self.new_module_import_meta(properties))
            .transpose()?;
        self.complete_parsing_module(realm, parsing_module, module, import_meta.as_ref())
            .map(ModuleCompilation::Published)
    }

    /// Parse host-selected strict JSON and publish a genuine synthetic module
    /// record. This intentionally does not invoke the JavaScript compiler:
    /// JSON object construction, `__proto__`, duplicate keys, diagnostics,
    /// and realm identity must remain those of QuickJS's JSON parser.
    fn compile_json_module_record_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        name: &JsString,
    ) -> Result<ModuleCompilation, RuntimeError> {
        let source = JsString::try_from_utf8(source)?;
        let value = match self.parse_json_module_text(realm, &source, name)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(exception) => {
                return Ok(ModuleCompilation::Throw(exception));
            }
        };
        self.publish_json_module(realm, name.clone(), value)
            .map(ModuleCompilation::Published)
    }

    /// Parse host-selected QuickJS extended JSON and publish the same genuine
    /// synthetic module shape as strict JSON modules.
    fn compile_json5_module_record_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        name: &JsString,
    ) -> Result<ModuleCompilation, RuntimeError> {
        let source = JsString::try_from_utf8(source)?;
        let value = match self.parse_json5_module_text(realm, &source, name)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(exception) => {
                return Ok(ModuleCompilation::Throw(exception));
            }
        };
        self.publish_json_module(realm, name.clone(), value)
            .map(ModuleCompilation::Published)
    }

    fn compile_module_load_result_in_realm(
        &self,
        realm: ContextId,
        normalized_name: &JsString,
        loaded: ModuleLoadResult,
    ) -> Result<ModuleCompilation, RuntimeError> {
        match loaded {
            ModuleLoadResult::SourceText(source) => {
                self.compile_module_record_in_realm(realm, &source, normalized_name, None)
            }
            ModuleLoadResult::Compiled(module) => {
                if !module.belongs_to(self) {
                    return Err(RuntimeError::WrongRuntime("compiled module"));
                }
                if module.raw.cache != realm {
                    return Err(RuntimeError::WrongContext("compiled module"));
                }
                self.module_record(module.raw)?;
                Ok(ModuleCompilation::Published(module.raw))
            }
            ModuleLoadResult::SourceTextWithImportMeta { source, properties } => self
                .compile_module_record_in_realm(realm, &source, normalized_name, Some(properties)),
            ModuleLoadResult::JsonText(source) => {
                self.compile_json_module_record_in_realm(realm, &source, normalized_name)
            }
            ModuleLoadResult::Json5Text(source) => {
                self.compile_json5_module_record_in_realm(realm, &source, normalized_name)
            }
        }
    }

    fn compile_module_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
    ) -> Result<ModuleCompilation, RuntimeError> {
        let name = module_c_string_view(&JsString::try_from_utf8(filename)?)?;
        let compilation = self.compile_module_record_in_realm(realm, source, &name, None)?;
        let ModuleCompilation::Published(module) = compilation else {
            return Ok(compilation);
        };
        self.resolve_module_graph(realm, module)?;
        Ok(ModuleCompilation::Published(module))
    }

    fn resolve_module_graph(
        &self,
        realm: ContextId,
        module: RawModuleRef,
    ) -> Result<(), RuntimeError> {
        if module.cache != realm {
            return Err(RuntimeError::Invariant(
                "module resolution realm disagrees with its Context cache",
            ));
        }
        let record = self.module_record(module)?;
        match &record.resolution {
            ModuleResolutionState::Resolved(_)
            | ModuleResolutionState::Resolving
            | ModuleResolutionState::Failed => return Ok(()),
            ModuleResolutionState::Unresolved => {}
        }
        self.transition_module_record(module, RawModuleTransition::BeginResolution)?;
        let mut stack = vec![ModuleResolveFrame {
            module,
            next_request: 0,
            dependencies: Vec::with_capacity(record.requested_modules.len()),
        }];

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            while let Some(frame) = stack.last() {
                let frame_record = self.module_record(frame.module)?;
                if frame.next_request == frame_record.requested_modules.len() {
                    let completed = frame.module;
                    let dependencies = Rc::<[ModuleId]>::from(frame.dependencies.clone());
                    // Whole-record heap mutation is fallible. Keep the frame
                    // visible to rollback until the resolved state commits.
                    self.transition_module_record(
                        completed,
                        RawModuleTransition::FinishResolution(dependencies),
                    )?;
                    let popped = stack.pop().ok_or(RuntimeError::Invariant(
                        "module resolution stack unexpectedly became empty",
                    ))?;
                    if popped.module != completed {
                        return Err(RuntimeError::Invariant(
                            "module resolution stack changed during record publication",
                        ));
                    }
                    continue;
                }

                let (current, request) = {
                    let frame = stack.last_mut().ok_or(RuntimeError::Invariant(
                        "module resolution stack unexpectedly became empty",
                    ))?;
                    let current = frame.module;
                    let request = self
                        .module_record(current)?
                        .requested_modules
                        .get(frame.next_request)
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "module request index is outside its record",
                        ))?;
                    frame.next_request += 1;
                    (current, request)
                };
                let current_record = self.module_record(current)?;
                let base_name = module_c_string_view(&current_record.name)?;
                let specifier = module_c_string_view(&request.specifier)?;
                // QuickJS re-reads the normalize hook for every request and
                // does not retain it across the subsequent load callback.
                let normalized_name = {
                    let loader = self.current_module_loader();
                    if let Some(loader) = loader {
                        let mut context = self.module_callback_context(realm)?;
                        let outcome = self
                            .invoke_module_host_callback(&mut context, |context| {
                                loader.normalize_in_context(context, &base_name, &specifier)
                            })?;
                        match self.propagate_module_host_throw(outcome)? {
                            Ok(normalized) => normalized,
                            Err(error) => {
                                let error = self.finish_module_loader_error(error, |message| {
                                    module_reference_error(
                                        "could not normalize module '",
                                        &specifier,
                                        &format!("': {message}"),
                                    )
                                })?;
                                return Err(error);
                            }
                        }
                    } else {
                        default_module_normalize_name(&base_name, &specifier)?
                    }
                };
                let normalized_name = module_c_string_view(&normalized_name)?;
                let cached = self
                    .0
                    .state
                    .borrow()
                    .heap
                    .first_loaded_module(current.cache, &normalized_name)?;
                let dependency = if let Some(cached) = cached {
                    cached
                } else {
                    // Normalize may mutate the installed callbacks. Sample
                    // again for this load only, then release the host before
                    // compiling and walking the dependency's own requests.
                    let loaded = {
                        let loader = self.current_module_loader();
                        let Some(loader) = loader else {
                            return Err(module_reference_error(
                                "could not load module '",
                                &normalized_name,
                                "'",
                            ));
                        };
                        let mut context = self.module_callback_context(realm)?;
                        let attributes = if request.attributes.effective().is_some() {
                            &request.attributes
                        } else {
                            &ModuleImportAttributes::Absent
                        };
                        let outcome =
                            self.invoke_module_host_callback(&mut context, |context| {
                                loader.load_with_attributes_in_context(
                                    context,
                                    &normalized_name,
                                    attributes,
                                )
                            })?;
                        match self.propagate_module_host_throw(outcome)? {
                            Ok(loaded) => loaded,
                            Err(error) => {
                                let error = self.finish_module_loader_error(error, |message| {
                                    module_reference_error(
                                        "could not load module '",
                                        &normalized_name,
                                        &format!("': {message}"),
                                    )
                                })?;
                                return Err(error);
                            }
                        }
                    };
                    let compilation =
                        self.compile_module_load_result_in_realm(realm, &normalized_name, loaded)?;
                    match compilation {
                        ModuleCompilation::Published(dependency) => dependency,
                        ModuleCompilation::Throw(exception) => {
                            self.set_pending_exception(exception)?;
                            return Err(RuntimeError::Exception);
                        }
                    }
                };
                stack
                    .last_mut()
                    .ok_or(RuntimeError::Invariant(
                        "module resolution stack unexpectedly became empty",
                    ))?
                    .dependencies
                    .push(dependency.module);

                let dependency_record = self.module_record(dependency)?;
                let needs_resolution = matches!(
                    dependency_record.resolution,
                    ModuleResolutionState::Unresolved
                );
                if needs_resolution {
                    self.transition_module_record(
                        dependency,
                        RawModuleTransition::BeginResolution,
                    )?;
                    stack.push(ModuleResolveFrame {
                        module: dependency,
                        next_request: 0,
                        dependencies: Vec::with_capacity(dependency_record.requested_modules.len()),
                    });
                }
            }
            Ok(())
        }));

        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                if !stack.is_empty() {
                    self.rollback_module_resolution_stack(module, &stack);
                }
                resume_unwind(payload);
            }
        };

        if result.is_err() {
            self.rollback_module_resolution_stack(module, &stack);
        }
        match result {
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind()).ok_or(
                    RuntimeError::Invariant("module loader error lost native kind"),
                )?;
                let exception = self.new_native_error_from_error(realm, kind, &error)?;
                self.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            result => result,
        }
    }

    /// Resolve one dynamic-import root using the loader installed when the
    /// FIFO load job actually runs. Context cache lookup precedes loading, so
    /// a cache hit deliberately ignores the request's later attributes.
    pub(super) fn resolve_dynamic_import_module(
        &self,
        realm: ContextId,
        base_name: &JsString,
        specifier: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<RawModuleRef, RuntimeError> {
        let base_name = module_c_string_view(base_name)?;
        let specifier = module_c_string_view(specifier)?;
        let result = (|| {
            let normalized_name = {
                let loader = self.current_module_loader();
                if let Some(loader) = loader {
                    let mut context = self.module_callback_context(realm)?;
                    let outcome = self.invoke_module_host_callback(&mut context, |context| {
                        loader.normalize_in_context(context, &base_name, &specifier)
                    })?;
                    match self.propagate_module_host_throw(outcome)? {
                        Ok(normalized) => normalized,
                        Err(error) => {
                            let error = self.finish_module_loader_error(error, |message| {
                                module_reference_error(
                                    "could not normalize module '",
                                    &specifier,
                                    &format!("': {message}"),
                                )
                            })?;
                            return Err(error);
                        }
                    }
                } else {
                    default_module_normalize_name(&base_name, &specifier)?
                }
            };
            let normalized_name = module_c_string_view(&normalized_name)?;
            let cached = self
                .0
                .state
                .borrow()
                .heap
                .first_loaded_module(realm, &normalized_name)?;
            let module = if let Some(cached) = cached {
                cached
            } else {
                let loaded = {
                    let loader = self.current_module_loader();
                    let Some(loader) = loader else {
                        return Err(module_reference_error(
                            "could not load module '",
                            &normalized_name,
                            "'",
                        ));
                    };
                    let mut context = self.module_callback_context(realm)?;
                    let outcome = self.invoke_module_host_callback(&mut context, |context| {
                        loader.load_with_attributes_in_context(
                            context,
                            &normalized_name,
                            attributes,
                        )
                    })?;
                    match self.propagate_module_host_throw(outcome)? {
                        Ok(loaded) => loaded,
                        Err(error) => {
                            let error = self.finish_module_loader_error(error, |message| {
                                module_reference_error(
                                    "could not load module '",
                                    &normalized_name,
                                    &format!("': {message}"),
                                )
                            })?;
                            return Err(error);
                        }
                    }
                };
                let compilation =
                    self.compile_module_load_result_in_realm(realm, &normalized_name, loaded)?;
                match compilation {
                    ModuleCompilation::Published(module) => module,
                    ModuleCompilation::Throw(exception) => {
                        self.set_pending_exception(exception)?;
                        return Err(RuntimeError::Exception);
                    }
                }
            };
            self.resolve_module_graph(realm, module)?;
            Ok(module)
        })();
        match result {
            Err(RuntimeError::Engine(error))
                if NativeErrorKind::from_javascript_error(error.kind()).is_some() =>
            {
                let kind = NativeErrorKind::from_javascript_error(error.kind()).ok_or(
                    RuntimeError::Invariant("dynamic module loader error lost native kind"),
                )?;
                let exception = self.new_native_error_from_error(realm, kind, &error)?;
                self.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            result => result,
        }
    }

    fn rollback_module_resolution_stack(&self, module: RawModuleRef, stack: &[ModuleResolveFrame]) {
        let mut resolution_owned = Vec::new();
        for frame in stack {
            let record = self
                .module_record(frame.module)
                .unwrap_or_else(|error| panic!("module resolution rollback failed: {error}"));
            match &record.body {
                ModuleRecordBody::SourceText { .. } | ModuleRecordBody::Json { .. } => {
                    if matches!(record.resolution, ModuleResolutionState::Resolving) {
                        self.transition_module_record(
                            frame.module,
                            RawModuleTransition::ResetResolution,
                        )
                        .unwrap_or_else(|error| {
                            panic!("module resolution rollback failed: {error}")
                        });
                    }
                    resolution_owned.push(frame.module.module);
                }
                // A construction transaction exclusively owns the lifetime of
                // its Parsing identity. QuickJS keeps its one-shot resolution
                // latch set after a host failure; make that poisoned state
                // explicit without retaining partial raw dependency IDs.
                ModuleRecordBody::Parsing => {
                    if matches!(record.resolution, ModuleResolutionState::Resolving) {
                        self.transition_module_record(
                            frame.module,
                            RawModuleTransition::FailResolution,
                        )
                        .unwrap_or_else(|error| {
                            panic!("module resolution rollback failed: {error}")
                        });
                    }
                }
                ModuleRecordBody::Aborted => {
                    panic!("module resolution rollback reached an aborted identity");
                }
            }
        }
        if resolution_owned.is_empty() {
            return;
        }
        self.unpublish_failed_resolution(module.cache, resolution_owned)
            .unwrap_or_else(|error| panic!("module resolution rollback failed: {error}"));
    }

    fn complete_parsing_module(
        &self,
        realm: ContextId,
        parsing_module: RawModuleRef,
        module: UnlinkedModule,
        import_meta: Option<&ObjectRef>,
    ) -> Result<RawModuleRef, RuntimeError> {
        bytecode_publish::verify_unlinked_module_tree(&module)?;

        let parsing_record = self.module_record(parsing_module)?;
        if parsing_module.cache != realm
            || !matches!(&parsing_record.body, ModuleRecordBody::Parsing)
        {
            return Err(RuntimeError::Invariant(
                "module completion did not target its Parsing record",
            ));
        }

        let parts = module.into_parts();
        let function = self.publish_verified_unlinked_function(realm, parts.function)?;
        let exports = parts
            .exports
            .into_vec()
            .into_iter()
            .map(|export| PublishedModuleExport {
                export_name: export.export_name,
                target: match export.target {
                    ModuleExportTarget::Local { closure_index } => {
                        PublishedModuleExportTarget::SourceTextLocal { closure_index }
                    }
                    ModuleExportTarget::Indirect {
                        request,
                        import_name,
                    } => PublishedModuleExportTarget::Indirect {
                        request,
                        import_name,
                    },
                },
            })
            .collect::<Vec<_>>();
        let record = ModuleRecord {
            name: parts.name,
            body: ModuleRecordBody::SourceText {
                function: function.bytecode_id(),
            },
            import_meta: import_meta.map(ObjectRef::object_id),
            has_top_level_await: parts.has_top_level_await,
            declaration_order: Rc::from(parts.declaration_order),
            link_initializers: Rc::from(parts.link_initializers),
            import_collisions: Rc::from(parts.import_collisions),
            requested_modules: Rc::new(parts.requested_modules.into_vec()),
            imports: Rc::from(parts.imports),
            exports: Rc::from(exports),
            star_exports: Rc::from(parts.star_exports),
            // A checker may re-enter the resolver while the parser exposes
            // only its source-order request prefix. QuickJS latches that
            // result; completing the record must not silently resolve again.
            resolution: parsing_record.resolution,
            instance: None,
            namespace: ModuleNamespaceState::Empty,
            link_status: ModuleLinkStatus::Unlinked,
            evaluation: ModuleEvaluationState::Unevaluated,
            evaluation_cycle_root: None,
            evaluation_promise: None,
            evaluation_resolve: None,
            evaluation_reject: None,
            pending_async_dependencies: 0,
            async_parent_modules: Vec::new(),
            async_evaluation_order: None,
            link_realm: None,
            compile_realm: realm,
        };
        self.replace_module_record(parsing_module, record)?;
        drop(function);
        Ok(parsing_module)
    }

    fn publish_json_module(
        &self,
        realm: ContextId,
        name: JsString,
        default_value: Value,
    ) -> Result<RawModuleRef, RuntimeError> {
        self.validate_value_domain(&default_value, "JSON module value")?;
        let raw_default_value = self.raw_property_value(&default_value)?;
        let record = ModuleRecord {
            name,
            body: ModuleRecordBody::Json {
                default_value: raw_default_value,
            },
            import_meta: None,
            has_top_level_await: false,
            declaration_order: Rc::from([]),
            link_initializers: Rc::from([]),
            import_collisions: Rc::from([]),
            requested_modules: Rc::new(Vec::new()),
            imports: Rc::from([]),
            exports: Rc::from([PublishedModuleExport {
                export_name: JsString::from_static("default"),
                target: PublishedModuleExportTarget::SyntheticLocal { cell_index: 0 },
            }]),
            star_exports: Rc::from([]),
            resolution: ModuleResolutionState::Unresolved,
            instance: None,
            namespace: ModuleNamespaceState::Empty,
            link_status: ModuleLinkStatus::Unlinked,
            evaluation: ModuleEvaluationState::Unevaluated,
            evaluation_cycle_root: None,
            evaluation_promise: None,
            evaluation_resolve: None,
            evaluation_reject: None,
            pending_async_dependencies: 0,
            async_parent_modules: Vec::new(),
            async_evaluation_order: None,
            link_realm: None,
            compile_realm: realm,
        };
        let published = self.publish_module_record(realm, record)?;
        drop(default_value);
        Ok(published)
    }

    fn raw_module_dependencies(
        &self,
        module: RawModuleRef,
    ) -> Result<Vec<RawModuleRef>, RuntimeError> {
        let record = self.module_record(module)?;
        let ids = match &record.resolution {
            ModuleResolutionState::Resolved(ids) => ids.to_vec(),
            ModuleResolutionState::Failed => {
                return Err(RuntimeError::IncompleteModuleResolution);
            }
            ModuleResolutionState::Unresolved | ModuleResolutionState::Resolving => {
                return Err(RuntimeError::Invariant(
                    "module execution reached an unresolved graph",
                ));
            }
        };
        Ok(ids
            .into_iter()
            .map(|id| RawModuleRef {
                cache: module.cache,
                module: id,
            })
            .collect())
    }

    #[cfg(test)]
    fn module_dependencies(
        &self,
        module: &ModuleBytecodeRef,
    ) -> Result<Vec<ModuleBytecodeRef>, RuntimeError> {
        self.raw_module_dependencies(module.raw)?
            .into_iter()
            .map(|module| self.root_module(module))
            .collect()
    }

    fn module_dependency(
        &self,
        module: RawModuleRef,
        request: ModuleRequestIndex,
    ) -> Result<RawModuleRef, RuntimeError> {
        let record = self.module_record(module)?;
        let id = match &record.resolution {
            ModuleResolutionState::Resolved(ids) => {
                ids.get(request.0 as usize)
                    .copied()
                    .ok_or(RuntimeError::Invariant(
                        "module request is outside the resolved graph",
                    ))?
            }
            ModuleResolutionState::Failed => {
                return Err(RuntimeError::IncompleteModuleResolution);
            }
            ModuleResolutionState::Unresolved | ModuleResolutionState::Resolving => {
                return Err(RuntimeError::Invariant(
                    "module dependency lookup reached an unresolved graph",
                ));
            }
        };
        Ok(RawModuleRef {
            cache: module.cache,
            module: id,
        })
    }

    fn prepare_module_instance(
        &self,
        module: RawModuleRef,
        link_realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let mut pending = vec![module];
        while let Some(current) = pending.pop() {
            if self.module_record(current)?.instance.is_some() {
                continue;
            }
            self.prepare_single_module_instance(current, link_realm)?;
            let dependencies = self.raw_module_dependencies(current)?;
            pending.extend(dependencies.into_iter().rev());
        }
        Ok(())
    }

    /// Validate the complete resolved graph before publishing any module
    /// environment. QuickJS can retain a dangling `JSModuleDef *` when a
    /// checker resolves a parse-in-progress dependency which later fails; in
    /// this unsafe-free engine that identity becomes `Aborted` instead. The
    /// read-only preflight makes the resulting error deterministic and keeps
    /// a failed link attempt from leaving partial instances behind.
    fn preflight_module_graph_for_link(&self, module: RawModuleRef) -> Result<(), RuntimeError> {
        let mut pending = vec![module];
        let mut visited = HashSet::new();
        while let Some(current) = pending.pop() {
            if !visited.insert(current.module) {
                continue;
            }
            let record = self.module_record(current)?;
            match &record.body {
                ModuleRecordBody::SourceText { .. } | ModuleRecordBody::Json { .. } => {}
                ModuleRecordBody::Parsing => {
                    return Err(RuntimeError::IncompleteModuleResolution);
                }
                ModuleRecordBody::Aborted => return Err(RuntimeError::AbortedModule),
            }
            let dependencies = match &record.resolution {
                ModuleResolutionState::Resolved(dependencies) => dependencies,
                ModuleResolutionState::Resolving | ModuleResolutionState::Failed => {
                    return Err(RuntimeError::IncompleteModuleResolution);
                }
                ModuleResolutionState::Unresolved => {
                    return Err(RuntimeError::Invariant(
                        "module linking reached an unresolved graph",
                    ));
                }
            };
            if dependencies.len() != record.requested_modules.len() {
                return Err(RuntimeError::IncompleteModuleResolution);
            }
            pending.extend(dependencies.iter().rev().map(|dependency| RawModuleRef {
                cache: current.cache,
                module: *dependency,
            }));
        }
        Ok(())
    }

    fn prepare_single_module_instance(
        &self,
        module: RawModuleRef,
        link_realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let record = self.module_record(module)?;
        if record.instance.is_some() {
            return Ok(());
        }
        let descriptors = match &record.body {
            ModuleRecordBody::SourceText { function } => {
                let state = self.0.state.borrow();
                Some(
                    state
                        .heap
                        .function_bytecode(*function)?
                        .closure_variables
                        .clone(),
                )
            }
            ModuleRecordBody::Json { .. } => None,
            ModuleRecordBody::Parsing => {
                return Err(RuntimeError::Invariant(
                    "module instantiation reached a parse-in-progress record",
                ));
            }
            ModuleRecordBody::Aborted => return Err(RuntimeError::AbortedModule),
        };

        let mut slots = Vec::with_capacity(descriptors.as_ref().map_or(1, |items| items.len()));
        for descriptor in descriptors.iter().flat_map(|items| items.iter().copied()) {
            let ClosureVariableName::Atom(name) = descriptor.name else {
                return Err(RuntimeError::Invariant(
                    "published module closure descriptor has no atom",
                ));
            };
            let slot = match descriptor.source {
                ClosureSource::ModuleDeclaration => {
                    if descriptor.kind != ClosureVariableKind::Normal
                        || (descriptor.is_const && !descriptor.is_lexical)
                    {
                        return Err(RuntimeError::Invariant(
                            "published module declaration has invalid binding metadata",
                        ));
                    }
                    Some(self.new_uninitialized_captured_var_ref(
                        descriptor.is_lexical,
                        descriptor.is_const,
                        descriptor.kind,
                    )?)
                }
                ClosureSource::ModuleImport => {
                    if descriptor.kind != ClosureVariableKind::ModuleImportView
                        || !descriptor.is_lexical
                        || !descriptor.is_const
                    {
                        return Err(RuntimeError::Invariant(
                            "published module import has invalid binding metadata",
                        ));
                    }
                    None
                }
                ClosureSource::ModuleImportCollision => {
                    if !descriptor.is_lexical || !descriptor.is_const {
                        return Err(RuntimeError::Invariant(
                            "published module import collision has invalid binding metadata",
                        ));
                    }
                    match descriptor.kind {
                        ClosureVariableKind::ModuleImportView => None,
                        ClosureVariableKind::Normal => {
                            Some(self.new_uninitialized_captured_var_ref(
                                true,
                                true,
                                ClosureVariableKind::Normal,
                            )?)
                        }
                        _ => {
                            return Err(RuntimeError::Invariant(
                                "published module import collision has invalid binding kind",
                            ));
                        }
                    }
                }
                ClosureSource::ModuleImportMeta => {
                    if descriptor.kind != ClosureVariableKind::Normal
                        || !descriptor.is_lexical
                        || !descriptor.is_const
                    {
                        return Err(RuntimeError::Invariant(
                            "published import.meta binding has invalid metadata",
                        ));
                    }
                    let meta = self.get_or_create_module_import_meta(module)?;
                    Some(self.new_var_ref(
                        Value::Object(meta),
                        true,
                        true,
                        ClosureVariableKind::Normal,
                    )?)
                }
                ClosureSource::Global => {
                    if descriptor.kind != ClosureVariableKind::Normal
                        || descriptor.is_lexical
                        || descriptor.is_const
                    {
                        return Err(RuntimeError::Invariant(
                            "published module global has invalid binding metadata",
                        ));
                    }
                    Some(self.resolve_global_var(link_realm, name)?)
                }
                ClosureSource::ParentLocal(_)
                | ClosureSource::ParentArgument(_)
                | ClosureSource::ParentClosure(_)
                | ClosureSource::GlobalDeclaration
                | ClosureSource::ParentGlobal(_)
                | ClosureSource::EvalEnvironment(_) => {
                    return Err(RuntimeError::Invariant(
                        "module root closure descriptor used a non-root source",
                    ));
                }
            };
            slots.push(slot);
        }
        if descriptors.is_none() {
            // QuickJS `js_create_module_function` allocates one non-lexical
            // detached VarRef per local C/synthetic export. Its initial value
            // is `undefined`; the module initializer writes the JSON value at
            // evaluation time.
            slots.push(Some(self.new_var_ref(
                Value::Undefined,
                false,
                false,
                ClosureVariableKind::Normal,
            )?));
        }

        let latest = self.module_record(module)?;
        if latest.link_realm.is_some() {
            return Err(RuntimeError::Invariant(
                "uninstantiated module retained a link realm",
            ));
        }
        if latest.instance.is_some() {
            return Err(RuntimeError::Invariant(
                "module instance was published during preparation",
            ));
        }
        let raw_slots = slots
            .iter()
            .map(|slot| slot.as_ref().map(VarRefRoot::id))
            .collect();
        self.mutate_module_record(module, |record| {
            record.link_realm = Some(if link_realm == module.cache {
                RawModuleLinkRealm::Cache
            } else {
                RawModuleLinkRealm::Other(link_realm)
            });
            record.instance = Some(ModuleInstance {
                slots: raw_slots,
                callable: None,
            });
            Ok(())
        })?;
        drop(slots);
        Ok(())
    }

    fn throw_module_link_syntax_error<T>(
        &self,
        realm: ContextId,
        message: NativeErrorMessage,
    ) -> Result<T, RuntimeError> {
        let exception =
            self.new_native_error_from_message(realm, NativeErrorKind::Syntax, message)?;
        self.set_pending_exception(exception)?;
        Err(RuntimeError::Exception)
    }

    fn throw_module_export_resolution_error<T>(
        &self,
        realm: ContextId,
        kind: ModuleExportResolveResultKind,
        module: RawModuleRef,
        export_name: &JsString,
    ) -> Result<T, RuntimeError> {
        let module_name = self.module_record(module)?.name;
        let message = match kind {
            ModuleExportResolveResultKind::NotFound => {
                module_export_error_message("Could not find export '", export_name, &module_name)
            }
            ModuleExportResolveResultKind::Circular => module_export_error_message(
                "circular reference when looking for export '",
                export_name,
                &module_name,
            ),
            ModuleExportResolveResultKind::Ambiguous => {
                let mut message =
                    module_export_error_message("export '", export_name, &module_name);
                message.push_utf8(" is ambiguous");
                message
            }
        };
        self.throw_module_link_syntax_error(realm, message)
    }

    /// Resolve one exported name without consuming the native stack.
    ///
    /// The resolve set deliberately survives the complete operation instead
    /// of being popped with a DFS frame. This is the observable QuickJS
    /// behavior for a diamond containing a circular branch. Local exports
    /// retain their `(module, closure)` identity here; imported-local aliases
    /// are followed only when a caller asks for the live VarRef.
    #[allow(clippy::mutable_key_type)] // JsString hashes immutable contents; only its rope cache mutates.
    fn resolve_module_export(
        &self,
        module: RawModuleRef,
        export_name: &JsString,
    ) -> Result<ModuleExportResolveResult, RuntimeError> {
        enum Action {
            Continue,
            Complete(ModuleExportResolveResult),
            Push(RawModuleRef, JsString),
        }

        let mut resolve_set: HashSet<(ModuleId, JsString)> = HashSet::new();
        let mut stack = vec![ModuleExportResolveFrame {
            module,
            export_name: export_name.clone(),
            state: ModuleExportResolveFrameState::Enter,
        }];
        let mut completed = None;

        loop {
            if let Some(result) = completed.take() {
                let Some(parent) = stack.last_mut() else {
                    return Ok(result);
                };
                match &mut parent.state {
                    ModuleExportResolveFrameState::AwaitIndirect => {
                        stack.pop();
                        completed = Some(result);
                    }
                    ModuleExportResolveFrameState::Stars { found, .. } => match result {
                        ModuleExportResolveResult::Found(binding) => {
                            if found
                                .as_ref()
                                .is_some_and(|prior| !prior.has_same_identity(&binding))
                            {
                                stack.pop();
                                completed = Some(ModuleExportResolveResult::Ambiguous);
                            } else if found.is_none() {
                                *found = Some(binding);
                            }
                        }
                        ModuleExportResolveResult::Ambiguous => {
                            stack.pop();
                            completed = Some(ModuleExportResolveResult::Ambiguous);
                        }
                        ModuleExportResolveResult::NotFound
                        | ModuleExportResolveResult::Circular => {}
                    },
                    ModuleExportResolveFrameState::Enter => {
                        return Err(RuntimeError::Invariant(
                            "export resolver returned to an unentered parent frame",
                        ));
                    }
                }
                continue;
            }

            let action = {
                let frame = stack.last_mut().ok_or(RuntimeError::Invariant(
                    "export resolver call stack unexpectedly became empty",
                ))?;
                match &mut frame.state {
                    ModuleExportResolveFrameState::Enter => {
                        let record = self.module_record(frame.module)?;
                        if !resolve_set.insert((frame.module.module, frame.export_name.clone())) {
                            Action::Complete(ModuleExportResolveResult::Circular)
                        } else if let Some((export_index, target)) = record
                            .exports
                            .iter()
                            .enumerate()
                            .find(|(_, export)| export.export_name == frame.export_name)
                            .map(|(index, export)| (index, export.target.clone()))
                        {
                            match target {
                                PublishedModuleExportTarget::SourceTextLocal { closure_index } => {
                                    Action::Complete(ModuleExportResolveResult::Found(
                                        ModuleResolvedBinding {
                                            module: frame.module,
                                            target: ModuleResolvedBindingTarget::Local {
                                                closure_index,
                                            },
                                        },
                                    ))
                                }
                                PublishedModuleExportTarget::SyntheticLocal { cell_index } => {
                                    Action::Complete(ModuleExportResolveResult::Found(
                                        ModuleResolvedBinding {
                                            module: frame.module,
                                            target: ModuleResolvedBindingTarget::Local {
                                                closure_index: cell_index,
                                            },
                                        },
                                    ))
                                }
                                PublishedModuleExportTarget::Indirect {
                                    request,
                                    import_name: ModuleImportName::Namespace,
                                } => {
                                    let _ = request;
                                    Action::Complete(ModuleExportResolveResult::Found(
                                        ModuleResolvedBinding {
                                            module: frame.module,
                                            target: ModuleResolvedBindingTarget::Namespace {
                                                export_index,
                                            },
                                        },
                                    ))
                                }
                                PublishedModuleExportTarget::Indirect {
                                    request,
                                    import_name: ModuleImportName::Name(import_name),
                                } => {
                                    let dependency =
                                        self.module_dependency(frame.module, request)?;
                                    frame.state = ModuleExportResolveFrameState::AwaitIndirect;
                                    Action::Push(dependency, import_name)
                                }
                            }
                        } else if frame.export_name == JsString::from_static("default") {
                            Action::Complete(ModuleExportResolveResult::NotFound)
                        } else {
                            frame.state = ModuleExportResolveFrameState::Stars {
                                next_star: 0,
                                found: None,
                            };
                            Action::Continue
                        }
                    }
                    ModuleExportResolveFrameState::AwaitIndirect => {
                        return Err(RuntimeError::Invariant(
                            "export resolver indirect frame lost its child result",
                        ));
                    }
                    ModuleExportResolveFrameState::Stars { next_star, found } => {
                        let record = self.module_record(frame.module)?;
                        if let Some(star) = record.star_exports.get(*next_star) {
                            *next_star += 1;
                            let dependency = self.module_dependency(frame.module, star.request)?;
                            Action::Push(dependency, frame.export_name.clone())
                        } else {
                            Action::Complete(found.take().map_or(
                                ModuleExportResolveResult::NotFound,
                                ModuleExportResolveResult::Found,
                            ))
                        }
                    }
                }
            };

            match action {
                Action::Continue => {}
                Action::Complete(result) => {
                    stack.pop();
                    completed = Some(result);
                }
                Action::Push(module, export_name) => stack.push(ModuleExportResolveFrame {
                    module,
                    export_name,
                    state: ModuleExportResolveFrameState::Enter,
                }),
            }
        }
    }

    /// Collect the candidate names for a module namespace using the same
    /// global visited-set traversal as QuickJS `get_exported_names`.
    fn get_module_exported_names(
        &self,
        module: RawModuleRef,
    ) -> Result<Vec<JsString>, RuntimeError> {
        let default_name = JsString::from_static("default");
        let mut visited = HashSet::new();
        let mut seen_names = HashSet::new();
        let mut names = Vec::new();
        let mut stack = vec![ModuleExportNamesFrame {
            module,
            from_star: false,
            entered: false,
            next_star: 0,
        }];

        while let Some(frame) = stack.last_mut() {
            if !frame.entered {
                frame.entered = true;
                if !visited.insert(frame.module.module) {
                    stack.pop();
                    continue;
                }
                let record = self.module_record(frame.module)?;
                for export in record.exports.iter() {
                    if frame.from_star && export.export_name == default_name {
                        continue;
                    }
                    if seen_names.insert(export.export_name.utf16_units().collect::<Vec<_>>()) {
                        names.push(export.export_name.clone());
                    }
                }
                continue;
            }

            let record = self.module_record(frame.module)?;
            let Some(star) = record.star_exports.get(frame.next_star) else {
                stack.pop();
                continue;
            };
            frame.next_star += 1;
            let dependency = self.module_dependency(frame.module, star.request)?;
            stack.push(ModuleExportNamesFrame {
                module: dependency,
                from_star: true,
                entered: false,
                next_star: 0,
            });
        }
        Ok(names)
    }

    fn rollback_module_namespace_transaction(&self, created: &[RawModuleRef]) {
        let mut state = self.0.state.borrow_mut();
        let cleanup = state
            .heap
            .rollback_loaded_module_namespaces(created)
            .unwrap_or_else(|error| panic!("module namespace rollback failed: {error}"));
        state.apply_committed_cleanup(cleanup);
    }

    #[cfg(test)]
    pub(in crate::runtime) fn get_module_namespace(
        &self,
        module: &ModuleBytecodeRef,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        if !module.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        self.get_module_namespace_raw(module.raw, realm)
    }

    pub(super) fn get_module_namespace_raw(
        &self,
        module: RawModuleRef,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let mut created = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.build_module_namespace(module, realm, &mut created)
        }));
        match result {
            Ok(Ok(namespace)) => Ok(namespace),
            Ok(Err(error)) => {
                self.rollback_module_namespace_transaction(&created);
                Err(error)
            }
            Err(payload) => {
                self.rollback_module_namespace_transaction(&created);
                resume_unwind(payload)
            }
        }
    }

    fn build_module_namespace(
        &self,
        module: RawModuleRef,
        realm: ContextId,
        created: &mut Vec<RawModuleRef>,
    ) -> Result<ObjectRef, RuntimeError> {
        match self.module_record(module)?.namespace {
            ModuleNamespaceState::Building(object) => {
                if !created.contains(&module) {
                    return Err(RuntimeError::Invariant(
                        "module namespace cache retained a stale Building record",
                    ));
                }
                return Ok(ObjectRef::from_borrowed_handle(self.clone(), object)?);
            }
            ModuleNamespaceState::Ready(object) => {
                return Ok(ObjectRef::from_borrowed_handle(self.clone(), object)?);
            }
            ModuleNamespaceState::Empty => {}
        }

        let namespace = self.new_module_namespace_object()?;
        self.mutate_module_record(module, |record| {
            record.namespace = ModuleNamespaceState::Building(namespace.object_id());
            Ok(())
        })?;
        created.push(module);

        let mut names = self.get_module_exported_names(module)?;
        names.sort_by(|left, right| left.utf16_units().cmp(right.utf16_units()));
        for name in names {
            let binding = match self.resolve_module_export(module, &name)? {
                ModuleExportResolveResult::Found(binding) => binding,
                ModuleExportResolveResult::Ambiguous => continue,
                result => {
                    let kind = result.error_kind().ok_or(RuntimeError::Invariant(
                        "non-found module export resolution had no error kind",
                    ))?;
                    return self.throw_module_export_resolution_error(realm, kind, module, &name);
                }
            };
            let slot = self.materialize_module_resolved_binding_inner(
                binding, realm, module, &name, created,
            )?;
            let key = self.intern_property_key_js_string(&name)?;
            self.store_property_slot(
                &namespace,
                &key,
                PropertyFlags::data(true, true, false),
                PropertySlot::VarRef(slot.id()),
            )?;
        }

        let tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        self.store_property_slot(
            &namespace,
            &tag,
            PropertyFlags::data(false, false, false),
            PropertySlot::Data(RawValue::String(JsString::from_static("Module"))),
        )?;
        self.transition_module_record(
            module,
            RawModuleTransition::FinishNamespace(namespace.object_id()),
        )?;
        Ok(namespace)
    }

    fn materialize_module_resolved_binding(
        &self,
        binding: ModuleResolvedBinding,
        realm: ContextId,
        error_module: RawModuleRef,
        error_name: &JsString,
    ) -> Result<VarRefRoot, RuntimeError> {
        let mut created = Vec::new();
        let result = catch_unwind(AssertUnwindSafe(|| {
            self.materialize_module_resolved_binding_inner(
                binding,
                realm,
                error_module,
                error_name,
                &mut created,
            )
        }));
        match result {
            Ok(Ok(slot)) => Ok(slot),
            Ok(Err(error)) => {
                self.rollback_module_namespace_transaction(&created);
                Err(error)
            }
            Err(payload) => {
                self.rollback_module_namespace_transaction(&created);
                resume_unwind(payload)
            }
        }
    }

    fn materialize_module_resolved_binding_inner(
        &self,
        binding: ModuleResolvedBinding,
        realm: ContextId,
        error_module: RawModuleRef,
        error_name: &JsString,
        namespace_transaction: &mut Vec<RawModuleRef>,
    ) -> Result<VarRefRoot, RuntimeError> {
        let mut binding = binding;
        let mut local_aliases = HashSet::new();

        loop {
            match binding.target {
                ModuleResolvedBindingTarget::Namespace { export_index } => {
                    let record = self.module_record(binding.module)?;
                    let export =
                        record
                            .exports
                            .get(export_index)
                            .ok_or(RuntimeError::Invariant(
                                "resolved namespace export entry is outside the module table",
                            ))?;
                    let PublishedModuleExportTarget::Indirect {
                        request,
                        import_name: ModuleImportName::Namespace,
                    } = &export.target
                    else {
                        return Err(RuntimeError::Invariant(
                            "resolved namespace identity no longer names a namespace export",
                        ));
                    };
                    let target = self.module_dependency(binding.module, *request)?;
                    let namespace =
                        self.build_module_namespace(target, realm, namespace_transaction)?;
                    return self.new_var_ref(
                        Value::Object(namespace),
                        true,
                        true,
                        ClosureVariableKind::Normal,
                    );
                }
                ModuleResolvedBindingTarget::Local { closure_index } => {
                    if !local_aliases.insert((binding.module.module, closure_index)) {
                        return self.throw_module_export_resolution_error(
                            realm,
                            ModuleExportResolveResultKind::Circular,
                            error_module,
                            error_name,
                        );
                    }
                    let record = self.module_record(binding.module)?;
                    let function = match &record.body {
                        ModuleRecordBody::SourceText { function } => *function,
                        ModuleRecordBody::Json { .. } => {
                            if closure_index != 0 {
                                return Err(RuntimeError::Invariant(
                                    "synthetic module export cell is out of bounds",
                                ));
                            }
                            let slot = record
                                .instance
                                .as_ref()
                                .and_then(|instance| instance.slots.first())
                                .and_then(|slot| *slot)
                                .ok_or(RuntimeError::Invariant(
                                    "synthetic module export has no instantiated live cell",
                                ))?;
                            return Ok(VarRefRoot::from_borrowed_handle(self.clone(), slot)?);
                        }
                        ModuleRecordBody::Parsing => {
                            return Err(RuntimeError::Invariant(
                                "module export resolution reached a parse-in-progress record",
                            ));
                        }
                        ModuleRecordBody::Aborted => return Err(RuntimeError::AbortedModule),
                    };
                    let descriptor = {
                        let state = self.0.state.borrow();
                        state
                            .heap
                            .function_bytecode(function)?
                            .closure_variables
                            .get(usize::from(closure_index))
                            .copied()
                            .ok_or(RuntimeError::Invariant(
                                "resolved export closure is outside the module root",
                            ))?
                    };
                    match descriptor.source {
                        ClosureSource::ModuleDeclaration => {
                            if descriptor.kind != ClosureVariableKind::Normal {
                                return Err(RuntimeError::Invariant(
                                    "resolved module declaration export has invalid metadata",
                                ));
                            }
                            let slot = record
                                .instance
                                .as_ref()
                                .and_then(|instance| instance.slots.get(usize::from(closure_index)))
                                .and_then(|slot| *slot)
                                .ok_or(RuntimeError::Invariant(
                                    "resolved export has no instantiated live cell",
                                ))?;
                            return Ok(VarRefRoot::from_borrowed_handle(self.clone(), slot)?);
                        }
                        ClosureSource::ModuleImport => {
                            if descriptor.kind != ClosureVariableKind::ModuleImportView {
                                return Err(RuntimeError::Invariant(
                                    "resolved module import export has invalid metadata",
                                ));
                            }
                            let import = record
                                .imports
                                .iter()
                                .find(|import| import.closure_index == closure_index)
                                .cloned()
                                .ok_or(RuntimeError::Invariant(
                                    "exported module import has no import table entry",
                                ))?;
                            let dependency =
                                self.module_dependency(binding.module, import.request)?;
                            match import.import_name {
                                ModuleImportName::Namespace => {
                                    let namespace = self.build_module_namespace(
                                        dependency,
                                        realm,
                                        namespace_transaction,
                                    )?;
                                    return self.new_var_ref(
                                        Value::Object(namespace),
                                        true,
                                        true,
                                        ClosureVariableKind::Normal,
                                    );
                                }
                                ModuleImportName::Name(import_name) => {
                                    binding = match self
                                        .resolve_module_export(dependency, &import_name)?
                                    {
                                        ModuleExportResolveResult::Found(binding) => binding,
                                        result => {
                                            let kind = result.error_kind().ok_or(
                                                RuntimeError::Invariant(
                                                    "failed imported alias resolution had no error kind",
                                                ),
                                            )?;
                                            return self.throw_module_export_resolution_error(
                                                realm,
                                                kind,
                                                error_module,
                                                error_name,
                                            );
                                        }
                                    };
                                }
                            }
                        }
                        ClosureSource::ModuleImportCollision => match descriptor.kind {
                            ClosureVariableKind::Normal => {
                                let slot = record
                                    .instance
                                    .as_ref()
                                    .and_then(|instance| {
                                        instance.slots.get(usize::from(closure_index))
                                    })
                                    .and_then(|slot| *slot)
                                    .ok_or(RuntimeError::Invariant(
                                        "namespace import collision has no instantiated live cell",
                                    ))?;
                                return Ok(VarRefRoot::from_borrowed_handle(self.clone(), slot)?);
                            }
                            ClosureVariableKind::ModuleImportView => {
                                let import = record.imports
                                    .iter()
                                    .find(|import| import.closure_index == closure_index)
                                    .cloned()
                                    .ok_or(RuntimeError::Invariant(
                                        "exported module import collision has no import table entry",
                                    ))?;
                                let ModuleImportName::Name(import_name) = import.import_name else {
                                    return Err(RuntimeError::Invariant(
                                        "named import collision resolved to a namespace import",
                                    ));
                                };
                                let dependency =
                                    self.module_dependency(binding.module, import.request)?;
                                binding = match self
                                    .resolve_module_export(dependency, &import_name)?
                                {
                                    ModuleExportResolveResult::Found(binding) => binding,
                                    result => {
                                        let kind = result.error_kind().ok_or(
                                            RuntimeError::Invariant(
                                                "failed collision alias resolution had no error kind",
                                            ),
                                        )?;
                                        return self.throw_module_export_resolution_error(
                                            realm,
                                            kind,
                                            error_module,
                                            error_name,
                                        );
                                    }
                                };
                            }
                            _ => {
                                return Err(RuntimeError::Invariant(
                                    "module import collision export has invalid binding metadata",
                                ));
                            }
                        },
                        ClosureSource::ModuleImportMeta => {
                            return Err(RuntimeError::Invariant(
                                "import.meta binding escaped into module exports",
                            ));
                        }
                        ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentArgument(_)
                        | ClosureSource::ParentClosure(_)
                        | ClosureSource::GlobalDeclaration
                        | ClosureSource::Global
                        | ClosureSource::ParentGlobal(_)
                        | ClosureSource::EvalEnvironment(_) => {
                            return Err(RuntimeError::Invariant(
                                "local module export resolved to a non-module binding",
                            ));
                        }
                    }
                }
            }
        }
    }

    fn validate_module_indirect_exports(
        &self,
        module: RawModuleRef,
        dependencies: &[RawModuleRef],
        realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let record = self.module_record(module)?;
        for export in record.exports.iter() {
            let PublishedModuleExportTarget::Indirect {
                request,
                import_name: ModuleImportName::Name(import_name),
            } = &export.target
            else {
                continue;
            };
            let dependency =
                dependencies
                    .get(request.0 as usize)
                    .ok_or(RuntimeError::Invariant(
                        "indirect export request is outside the resolved graph",
                    ))?;
            let result = self.resolve_module_export(*dependency, import_name)?;
            if let Some(kind) = result.error_kind() {
                return self.throw_module_export_resolution_error(
                    realm,
                    kind,
                    module,
                    &export.export_name,
                );
            }
        }
        Ok(())
    }

    fn link_module_imports(
        &self,
        module: RawModuleRef,
        dependencies: &[RawModuleRef],
        realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let record = self.module_record(module)?;
        for import in record.imports.iter() {
            let dependency =
                dependencies
                    .get(import.request.0 as usize)
                    .ok_or(RuntimeError::Invariant(
                        "module import request is outside the resolved graph",
                    ))?;
            let slot = match &import.import_name {
                ModuleImportName::Namespace => {
                    let namespace = self.get_module_namespace_raw(*dependency, realm)?;
                    let slot = self
                        .module_record(module)?
                        .instance
                        .as_ref()
                        .and_then(|instance| instance.slots.get(usize::from(import.closure_index)))
                        .and_then(|slot| *slot)
                        .ok_or(RuntimeError::Invariant(
                            "namespace import has no preallocated declaration cell",
                        ))?;
                    let slot = VarRefRoot::from_borrowed_handle(self.clone(), slot)?;
                    self.write_var_ref(&slot, Value::Object(namespace))?;
                    continue;
                }
                ModuleImportName::Name(import_name) => {
                    let resolution = self.resolve_module_export(*dependency, import_name)?;
                    let binding = match resolution {
                        ModuleExportResolveResult::Found(binding) => binding,
                        result => {
                            let kind = result.error_kind().ok_or(RuntimeError::Invariant(
                                "failed module import resolution had no error kind",
                            ))?;
                            return self.throw_module_export_resolution_error(
                                realm,
                                kind,
                                *dependency,
                                import_name,
                            );
                        }
                    };
                    self.materialize_module_resolved_binding(
                        binding,
                        realm,
                        *dependency,
                        import_name,
                    )?
                }
            };
            self.mutate_module_record(module, |record| {
                let target = record
                    .instance
                    .as_mut()
                    .and_then(|instance| instance.slots.get_mut(usize::from(import.closure_index)))
                    .ok_or(RuntimeError::Invariant(
                        "module import closure is outside the instance",
                    ))?;
                *target = Some(slot.id());
                Ok(())
            })?;
            drop(slot);
        }
        Ok(())
    }

    fn create_module_callable(
        &self,
        module: RawModuleRef,
        realm: ContextId,
    ) -> Result<Option<CallableRef>, RuntimeError> {
        let record = self.module_record(module)?;
        let function = match &record.body {
            ModuleRecordBody::SourceText { function } => *function,
            ModuleRecordBody::Json { .. } => return Ok(None),
            ModuleRecordBody::Parsing => {
                return Err(RuntimeError::Invariant(
                    "module callable creation reached a parse-in-progress record",
                ));
            }
            ModuleRecordBody::Aborted => return Err(RuntimeError::AbortedModule),
        };
        if let Some(callable) = record
            .instance
            .as_ref()
            .and_then(|instance| instance.callable)
        {
            let callable = ObjectRef::from_borrowed_handle(self.clone(), callable)?;
            return Ok(Some(CallableRef::from_validated_object(callable)));
        }
        let slot_ids = record
            .instance
            .as_ref()
            .ok_or(RuntimeError::Invariant("module has no instance"))?
            .slots
            .iter()
            .map(|slot| {
                slot.ok_or(RuntimeError::Invariant(
                    "module callable retained an unresolved import slot",
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let slots = slot_ids
            .into_iter()
            .map(|slot| VarRefRoot::from_borrowed_handle(self.clone(), slot))
            .collect::<Result<Vec<_>, _>>()?;
        let function = FunctionBytecodeRef::from_borrowed_handle(self.clone(), function)?;
        let callable = self.new_bytecode_closure_with_slots(realm, &function, &slots)?;
        self.mutate_module_record(module, |record| {
            record
                .instance
                .as_mut()
                .ok_or(RuntimeError::Invariant("module instance disappeared"))?
                .callable = Some(callable.as_object().object_id());
            Ok(())
        })?;
        Ok(Some(callable))
    }

    fn enter_module_link_dfs(
        &self,
        module: RawModuleRef,
        dfs: &mut ModuleLinkDfs,
    ) -> Result<ModuleDfsFrame, RuntimeError> {
        if self.module_record(module)?.link_status != ModuleLinkStatus::Unlinked {
            return Err(RuntimeError::Invariant(
                "link DFS entered a module which was not unlinked",
            ));
        }
        self.transition_module_record(module, RawModuleTransition::BeginLink)?;
        let index = dfs.next_index;
        dfs.next_index = dfs
            .next_index
            .checked_add(1)
            .ok_or(RuntimeError::Invariant("module link DFS index overflow"))?;
        if dfs
            .entries
            .insert(
                module.module,
                ModuleDfsEntry {
                    index,
                    ancestor: index,
                },
            )
            .is_some()
        {
            return Err(RuntimeError::Invariant(
                "module entered the link DFS more than once",
            ));
        }
        dfs.stack.push(module.module);
        let dependencies = self.raw_module_dependencies(module)?;
        Ok(ModuleDfsFrame {
            module,
            dependencies,
            next_dependency: 0,
        })
    }

    fn link_module_dfs(
        &self,
        module: RawModuleRef,
        dfs: &mut ModuleLinkDfs,
    ) -> Result<(), RuntimeError> {
        match self.module_record(module)?.link_status {
            ModuleLinkStatus::Linked => return Ok(()),
            ModuleLinkStatus::Linking => {
                return Err(RuntimeError::Invariant(
                    "module linking was re-entered by the host",
                ));
            }
            ModuleLinkStatus::Poisoned => {
                return Err(RuntimeError::Invariant(
                    "module linking previously failed inside the engine",
                ));
            }
            ModuleLinkStatus::Unlinked => {}
        }
        let mut frames = vec![self.enter_module_link_dfs(module, dfs)?];

        while !frames.is_empty() {
            let dependency = {
                let frame = frames.last_mut().ok_or(RuntimeError::Invariant(
                    "module link call stack unexpectedly became empty",
                ))?;
                let dependency = frame.dependencies.get(frame.next_dependency).cloned();
                if dependency.is_some() {
                    frame.next_dependency += 1;
                }
                dependency
            };
            if let Some(dependency) = dependency {
                match self.module_record(dependency)?.link_status {
                    ModuleLinkStatus::Linked => {}
                    ModuleLinkStatus::Linking => {
                        let dependency_ancestor = dfs
                            .entries
                            .get(&dependency.module)
                            .map(|entry| entry.ancestor)
                            .ok_or(RuntimeError::Invariant(
                                "linking dependency has no DFS entry",
                            ))?;
                        let current_id = frames.last().map(|frame| frame.module.module).ok_or(
                            RuntimeError::Invariant(
                                "module link call stack unexpectedly became empty",
                            ),
                        )?;
                        let entry = dfs
                            .entries
                            .get_mut(&current_id)
                            .ok_or(RuntimeError::Invariant("linking module lost its DFS entry"))?;
                        entry.ancestor = entry.ancestor.min(dependency_ancestor);
                    }
                    ModuleLinkStatus::Unlinked => {
                        frames.push(self.enter_module_link_dfs(dependency, dfs)?);
                    }
                    ModuleLinkStatus::Poisoned => {
                        return Err(RuntimeError::Invariant(
                            "module linking previously failed inside the engine",
                        ));
                    }
                }
                continue;
            }

            let frame = frames.pop().ok_or(RuntimeError::Invariant(
                "module link call stack unexpectedly became empty",
            ))?;
            let realm = self
                .module_record(frame.module)?
                .link_realm
                .map(|realm| match realm {
                    RawModuleLinkRealm::Cache => frame.module.cache,
                    RawModuleLinkRealm::Other(realm) => realm,
                })
                .ok_or(RuntimeError::Invariant(
                    "instantiated module has no retained link realm",
                ))?;
            self.validate_module_indirect_exports(frame.module, &frame.dependencies, realm)?;
            self.link_module_imports(frame.module, &frame.dependencies, realm)?;
            let completion = match self.create_module_callable(frame.module, realm)? {
                Some(callable) => {
                    match self.call_internal(realm, &callable, Value::Bool(true), &[]) {
                        Ok(completion) => completion,
                        Err(error) => {
                            self.transition_module_record(
                                frame.module,
                                RawModuleTransition::PoisonLink,
                            )?;
                            return Err(error);
                        }
                    }
                }
                None => Completion::Return(Value::Undefined),
            };
            match completion {
                Completion::Return(Value::Undefined) => {
                    let entry = dfs
                        .entries
                        .get(&frame.module.module)
                        .copied()
                        .ok_or(RuntimeError::Invariant("linked module lost its DFS entry"))?;
                    if entry.index == entry.ancestor {
                        loop {
                            let member = *dfs.stack.last().ok_or(RuntimeError::Invariant(
                                "module link SCC stack underflow",
                            ))?;
                            let member = RawModuleRef {
                                cache: frame.module.cache,
                                module: member,
                            };
                            if self.module_record(member)?.link_status != ModuleLinkStatus::Linking
                            {
                                return Err(RuntimeError::Invariant(
                                    "module link SCC contained a non-linking member",
                                ));
                            }
                            self.transition_module_record(member, RawModuleTransition::FinishLink)?;
                            let popped = dfs.stack.pop().ok_or(RuntimeError::Invariant(
                                "module link SCC stack underflow after publication",
                            ))?;
                            if popped != member.module {
                                return Err(RuntimeError::Invariant(
                                    "module link SCC stack changed during record publication",
                                ));
                            }
                            if member.module == frame.module.module {
                                break;
                            }
                        }
                    }
                }
                Completion::Return(_) => {
                    self.transition_module_record(frame.module, RawModuleTransition::PoisonLink)?;
                    return Err(RuntimeError::Invariant(
                        "module link entry returned a non-undefined value",
                    ));
                }
                Completion::Throw(exception) => {
                    self.set_pending_exception(exception)?;
                    return Err(RuntimeError::Exception);
                }
            }

            if self.module_record(frame.module)?.link_status == ModuleLinkStatus::Linking {
                let dependency_ancestor = dfs
                    .entries
                    .get(&frame.module.module)
                    .map(|entry| entry.ancestor)
                    .ok_or(RuntimeError::Invariant(
                        "linking dependency has no DFS entry",
                    ))?;
                if let Some(parent) = frames.last() {
                    let entry = dfs
                        .entries
                        .get_mut(&parent.module.module)
                        .ok_or(RuntimeError::Invariant("linking module lost its DFS entry"))?;
                    entry.ancestor = entry.ancestor.min(dependency_ancestor);
                }
            }
        }
        if !dfs.stack.is_empty() {
            return Err(RuntimeError::Invariant(
                "successful module linking retained an SCC stack",
            ));
        }
        Ok(())
    }

    pub(super) fn link_module_graph(
        &self,
        module: RawModuleRef,
        initiating_realm: ContextId,
    ) -> Result<(), RuntimeError> {
        self.preflight_module_graph_for_link(module)?;
        self.prepare_module_instance(module, initiating_realm)?;
        let mut dfs = ModuleLinkDfs::new();
        let result = self.link_module_dfs(module, &mut dfs);
        if result.is_err() {
            for id in dfs.stack {
                let member = RawModuleRef {
                    cache: module.cache,
                    module: id,
                };
                if self.module_record(member)?.link_status == ModuleLinkStatus::Linking {
                    self.transition_module_record(member, RawModuleTransition::ResetLink)?;
                }
            }
        }
        result
    }

    fn enter_module_evaluation_dfs(
        &self,
        module: RawModuleRef,
        dfs: &mut ModuleEvaluationDfs,
    ) -> Result<ModuleDfsFrame, RuntimeError> {
        if !matches!(
            self.module_record(module)?.evaluation,
            ModuleEvaluationState::Unevaluated
        ) {
            return Err(RuntimeError::Invariant(
                "evaluation DFS entered a module which was not unevaluated",
            ));
        }
        self.transition_module_record(module, RawModuleTransition::BeginEvaluation)?;
        let index = dfs.next_index;
        dfs.next_index = dfs
            .next_index
            .checked_add(1)
            .ok_or(RuntimeError::Invariant(
                "module evaluation DFS index overflow",
            ))?;
        if dfs
            .entries
            .insert(
                module.module,
                ModuleDfsEntry {
                    index,
                    ancestor: index,
                },
            )
            .is_some()
        {
            return Err(RuntimeError::Invariant(
                "module entered the evaluation DFS more than once",
            ));
        }
        dfs.stack.push(module.module);
        Ok(ModuleDfsFrame {
            module,
            dependencies: self.raw_module_dependencies(module)?,
            next_dependency: 0,
        })
    }

    /// Execute a source-text module whose graph has no pending async work.
    /// Every module root is async bytecode in QuickJS, so even this synchronous
    /// path explicitly enters the async driver and inspects its returned
    /// Promise. A pending result here is an invariant: modules with authored
    /// TLA are started by `execute_async_module_body` instead.
    fn execute_source_text_module_body(
        &self,
        realm: ContextId,
        callable: &CallableRef,
    ) -> Result<Completion, RuntimeError> {
        let completion = self.call_internal(realm, callable, Value::Undefined, &[])?;
        let Completion::Return(Value::Object(promise)) = completion else {
            return match completion {
                Completion::Throw(_) => Err(RuntimeError::Invariant(
                    "async module callable threw instead of returning a Promise",
                )),
                Completion::Return(_) => Err(RuntimeError::Invariant(
                    "async module callable returned a non-Promise",
                )),
            };
        };
        let snapshot = self
            .0
            .state
            .borrow()
            .heap
            .promise_snapshot(promise.object_id())?;
        let result = self.root_raw_value(&snapshot.result)?;
        match snapshot.state {
            PromiseState::Fulfilled => Ok(Completion::Return(result)),
            PromiseState::Rejected => Ok(Completion::Throw(result)),
            PromiseState::Pending => Err(RuntimeError::Invariant(
                "synchronous module body retained a pending Promise",
            )),
        }
    }

    fn next_module_async_evaluation_order(&self) -> Result<u64, RuntimeError> {
        let mut state = self.0.state.borrow_mut();
        let order = state.next_module_async_evaluation_order;
        state.next_module_async_evaluation_order = order.checked_add(1).ok_or(
            RuntimeError::Invariant("module async evaluation order overflow"),
        )?;
        Ok(order)
    }

    fn module_callable(&self, module: RawModuleRef) -> Result<CallableRef, RuntimeError> {
        let callable = self
            .module_record(module)?
            .instance
            .as_ref()
            .and_then(|instance| instance.callable)
            .ok_or(RuntimeError::Invariant(
                "linked source-text module has no callable instance",
            ))?;
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_borrowed_handle(self.clone(), callable)?,
        ))
    }

    /// Start one authored TLA module through the existing AsyncFunction
    /// driver and attach QuickJS-style module completion reactions. The full
    /// private Promise-then path intentionally performs species lookup and
    /// allocates its discarded result capability.
    fn execute_async_module_body(
        &self,
        evaluation_realm: ContextId,
        module: RawModuleRef,
    ) -> Result<(), RuntimeError> {
        let callable = self.module_callable(module)?;
        let completion = self.call_internal(evaluation_realm, &callable, Value::Undefined, &[])?;
        let Completion::Return(Value::Object(promise)) = completion else {
            return Err(RuntimeError::Invariant(
                "async module callable did not return a Promise",
            ));
        };
        let make_handler = |kind| {
            self.new_internal_promise_function(
                evaluation_realm,
                NativeFunctionId::ModuleEvaluation(kind),
                1,
                0,
                InternalCallableData::ModuleEvaluation { module, kind },
            )
        };
        let fulfill = make_handler(ModuleEvaluationKind::Fulfill)?;
        let reject = make_handler(ModuleEvaluationKind::Reject)?;
        match self.attach_module_evaluation_handlers(
            evaluation_realm,
            &promise,
            &fulfill,
            &reject,
        )? {
            NativeConversion::Value(()) => Ok(()),
            NativeConversion::Throw(reason) => {
                // QuickJS discards this abrupt result and leaves the module
                // pending. Preserve its current exception slot for the host.
                self.set_pending_exception(reason)
            }
        }
    }

    fn execute_module_body_synchronously(
        &self,
        evaluation_realm: ContextId,
        module: RawModuleRef,
    ) -> Result<Completion, RuntimeError> {
        let record = self.module_record(module)?;
        match &record.body {
            ModuleRecordBody::SourceText { .. } => {
                let callable = self.module_callable(module)?;
                self.execute_source_text_module_body(evaluation_realm, &callable)
            }
            ModuleRecordBody::Json { default_value } => {
                let slot = record
                    .instance
                    .as_ref()
                    .and_then(|instance| instance.slots.first())
                    .and_then(|slot| *slot)
                    .ok_or(RuntimeError::Invariant(
                        "linked JSON module has no default live cell",
                    ))?;
                let slot = VarRefRoot::from_borrowed_handle(self.clone(), slot)?;
                let default_value = self.root_raw_value(default_value)?;
                self.write_var_ref(&slot, default_value)?;
                Ok(Completion::Return(Value::Undefined))
            }
            ModuleRecordBody::Parsing => Err(RuntimeError::Invariant(
                "module execution reached a parse-in-progress record",
            )),
            ModuleRecordBody::Aborted => Err(RuntimeError::AbortedModule),
        }
    }

    fn module_evaluation_settler(
        &self,
        module: RawModuleRef,
        kind: ModuleEvaluationKind,
    ) -> Result<Option<CallableRef>, RuntimeError> {
        let record = self.module_record(module)?;
        let target = match kind {
            ModuleEvaluationKind::Fulfill => record.evaluation_resolve,
            ModuleEvaluationKind::Reject => record.evaluation_reject,
        };
        target
            .map(|target| {
                let target = ObjectRef::from_borrowed_handle(self.clone(), target)?;
                self.as_callable(&target)?.ok_or(RuntimeError::Invariant(
                    "module evaluation resolving function lost its callable brand",
                ))
            })
            .transpose()
    }

    fn settle_module_evaluation_capability(
        &self,
        realm: ContextId,
        module: RawModuleRef,
        kind: ModuleEvaluationKind,
        value: Value,
    ) -> Result<(), RuntimeError> {
        let Some(target) = self.module_evaluation_settler(module, kind)? else {
            return Ok(());
        };
        let _ = self.call_internal(realm, &target, Value::Undefined, &[value])?;
        Ok(())
    }

    fn reject_async_module_ancestors(
        &self,
        realm: ContextId,
        module: RawModuleRef,
        reason: Value,
    ) -> Result<(), RuntimeError> {
        self.validate_value_domain(&reason, "async module rejection")?;
        let raw_reason = self.raw_property_value(&reason)?;
        let mut pending = vec![module.module];
        while let Some(module_id) = pending.pop() {
            let current = RawModuleRef {
                cache: module.cache,
                module: module_id,
            };
            let record = self.module_record(current)?;
            match record.evaluation {
                ModuleEvaluationState::Errored(_) => continue,
                ModuleEvaluationState::EvaluatingAsync => {}
                _ => {
                    return Err(RuntimeError::Invariant(
                        "async module rejection reached an inactive ancestor",
                    ));
                }
            }
            let parents = record.async_parent_modules;
            let mut state = self.0.state.borrow_mut();
            let retained_atoms = match raw_reason {
                RawValue::Symbol(atom) => Self::retain_module_atoms(&mut state, vec![atom])?,
                _ => Vec::new(),
            };
            if let Err(error) = state
                .heap
                .publish_loaded_module_async_error(current, raw_reason.clone())
            {
                state.release_atoms(retained_atoms)?;
                return Err(error.into());
            }
            drop(state);

            // QuickJS makes this module observably Errored, rejects its own
            // evaluation capability, and only then recursively visits parents.
            // Keep that per-node order so a reentrant host rejection tracker
            // cannot observe ancestors changing ahead of the reference engine.
            self.settle_module_evaluation_capability(
                realm,
                current,
                ModuleEvaluationKind::Reject,
                reason.clone(),
            )?;
            pending.extend(parents.iter().rev().copied());
        }
        Ok(())
    }

    fn gather_available_module_ancestors(
        &self,
        module: RawModuleRef,
    ) -> Result<Vec<RawModuleRef>, RuntimeError> {
        let mut ready = Vec::new();
        let mut ready_set = HashSet::new();
        let mut pending = vec![module];
        while let Some(completed) = pending.pop() {
            let parents = self.module_record(completed)?.async_parent_modules;
            for parent_id in parents {
                if ready_set.contains(&parent_id) {
                    continue;
                }
                let parent = RawModuleRef {
                    cache: module.cache,
                    module: parent_id,
                };
                let parent_record = self.module_record(parent)?;
                let cycle_root =
                    parent_record
                        .evaluation_cycle_root
                        .ok_or(RuntimeError::Invariant(
                            "async module parent has no cycle root",
                        ))?;
                if matches!(
                    self.module_record(RawModuleRef {
                        cache: module.cache,
                        module: cycle_root,
                    })?
                    .evaluation,
                    ModuleEvaluationState::Errored(_)
                ) {
                    continue;
                }
                let remaining = self
                    .0
                    .state
                    .borrow_mut()
                    .heap
                    .complete_loaded_module_async_dependency(parent)?;
                if remaining == 0 {
                    ready_set.insert(parent_id);
                    ready.push(parent);
                    if !parent_record.has_top_level_await {
                        pending.push(parent);
                    }
                }
            }
        }
        ready.sort_by_key(|module| {
            self.module_record(*module)
                .ok()
                .and_then(|record| record.async_evaluation_order)
                .unwrap_or(u64::MAX)
        });
        if ready.iter().any(|module| {
            self.module_record(*module)
                .ok()
                .and_then(|record| record.async_evaluation_order)
                .is_none()
        }) {
            return Err(RuntimeError::Invariant(
                "available async module ancestor has no ordering stamp",
            ));
        }
        Ok(ready)
    }

    fn fulfill_async_module(
        &self,
        realm: ContextId,
        module: RawModuleRef,
    ) -> Result<(), RuntimeError> {
        match self.module_record(module)?.evaluation {
            ModuleEvaluationState::Errored(_) => return Ok(()),
            ModuleEvaluationState::EvaluatingAsync => {}
            _ => {
                return Err(RuntimeError::Invariant(
                    "async module fulfillment reached an inactive module",
                ));
            }
        }
        self.transition_module_record(module, RawModuleTransition::FinishAsyncEvaluation)?;
        self.settle_module_evaluation_capability(
            realm,
            module,
            ModuleEvaluationKind::Fulfill,
            Value::Undefined,
        )?;

        for ancestor in self.gather_available_module_ancestors(module)? {
            let record = self.module_record(ancestor)?;
            if matches!(record.evaluation, ModuleEvaluationState::Errored(_)) {
                continue;
            }
            if record.has_top_level_await {
                self.execute_async_module_body(realm, ancestor)?;
                continue;
            }
            match self.execute_module_body_synchronously(realm, ancestor)? {
                Completion::Return(Value::Undefined) => {
                    self.transition_module_record(
                        ancestor,
                        RawModuleTransition::FinishAsyncEvaluation,
                    )?;
                    self.settle_module_evaluation_capability(
                        realm,
                        ancestor,
                        ModuleEvaluationKind::Fulfill,
                        Value::Undefined,
                    )?;
                }
                Completion::Throw(reason) => {
                    self.reject_async_module_ancestors(realm, ancestor, reason)?;
                }
                Completion::Return(_) => {
                    return Err(RuntimeError::Invariant(
                        "module evaluation returned a non-undefined value",
                    ));
                }
            }
        }
        Ok(())
    }

    pub(super) fn call_module_evaluation_callback(
        &self,
        realm: ContextId,
        target_kind: ModuleEvaluationKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "module evaluation callback received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "module evaluation callback argv was not padded",
            ))?;
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "module evaluation callback has no internal state",
            ))?;
        let InternalCallableData::ModuleEvaluation { module, kind } = internal else {
            return Err(RuntimeError::Invariant(
                "module evaluation callback has the wrong internal state",
            ));
        };
        if kind != target_kind {
            return Err(RuntimeError::Invariant(
                "module evaluation callback target disagrees with its capture",
            ));
        }
        match target_kind {
            ModuleEvaluationKind::Fulfill => self.fulfill_async_module(realm, module)?,
            ModuleEvaluationKind::Reject => {
                self.reject_async_module_ancestors(realm, module, argument)?;
            }
        }
        Ok(Completion::Return(Value::Undefined))
    }

    fn evaluate_module_dfs(
        &self,
        evaluation_realm: ContextId,
        module: RawModuleRef,
        dfs: &mut ModuleEvaluationDfs,
    ) -> Result<(), RuntimeError> {
        let initial_state = {
            let record = self.module_record(module)?;
            match &record.evaluation {
                ModuleEvaluationState::Unevaluated => ModuleEvaluationVisit::Unevaluated,
                ModuleEvaluationState::Evaluating => ModuleEvaluationVisit::Evaluating,
                ModuleEvaluationState::EvaluatingAsync => ModuleEvaluationVisit::EvaluatingAsync,
                ModuleEvaluationState::Evaluated => ModuleEvaluationVisit::Evaluated,
                ModuleEvaluationState::Errored(exception) => {
                    ModuleEvaluationVisit::Errored(self.root_raw_value(exception)?)
                }
                ModuleEvaluationState::Poisoned => ModuleEvaluationVisit::Poisoned,
            }
        };
        match initial_state {
            ModuleEvaluationVisit::EvaluatingAsync | ModuleEvaluationVisit::Evaluated => {
                return Ok(());
            }
            ModuleEvaluationVisit::Evaluating => {
                return Err(RuntimeError::Invariant(
                    "module evaluation was re-entered by the host",
                ));
            }
            ModuleEvaluationVisit::Errored(exception) => {
                if dfs.exception.replace(exception).is_some() {
                    return Err(RuntimeError::Invariant(
                        "module evaluation recorded more than one exception",
                    ));
                }
                return Err(RuntimeError::Exception);
            }
            ModuleEvaluationVisit::Poisoned => {
                return Err(RuntimeError::Invariant(
                    "module evaluation previously failed inside the engine",
                ));
            }
            ModuleEvaluationVisit::Unevaluated => {}
        }
        let mut frames = vec![self.enter_module_evaluation_dfs(module, dfs)?];

        while !frames.is_empty() {
            let dependency = {
                let frame = frames.last_mut().ok_or(RuntimeError::Invariant(
                    "module evaluation call stack unexpectedly became empty",
                ))?;
                let dependency = frame.dependencies.get(frame.next_dependency).cloned();
                if dependency.is_some() {
                    frame.next_dependency += 1;
                }
                dependency
            };
            if let Some(dependency) = dependency {
                let dependency_state = {
                    let record = self.module_record(dependency)?;
                    match &record.evaluation {
                        ModuleEvaluationState::Unevaluated => ModuleEvaluationVisit::Unevaluated,
                        ModuleEvaluationState::Evaluating => ModuleEvaluationVisit::Evaluating,
                        ModuleEvaluationState::EvaluatingAsync => {
                            ModuleEvaluationVisit::EvaluatingAsync
                        }
                        ModuleEvaluationState::Evaluated => ModuleEvaluationVisit::Evaluated,
                        ModuleEvaluationState::Errored(exception) => {
                            ModuleEvaluationVisit::Errored(self.root_raw_value(exception)?)
                        }
                        ModuleEvaluationState::Poisoned => ModuleEvaluationVisit::Poisoned,
                    }
                };
                let async_dependency = match dependency_state {
                    ModuleEvaluationVisit::Evaluated => {
                        let cycle_root = self
                            .module_record(dependency)?
                            .evaluation_cycle_root
                            .ok_or(RuntimeError::Invariant(
                                "completed dependency has no cycle root",
                            ))?;
                        Some(RawModuleRef {
                            cache: dependency.cache,
                            module: cycle_root,
                        })
                    }
                    ModuleEvaluationVisit::EvaluatingAsync => {
                        let cycle_root = self
                            .module_record(dependency)?
                            .evaluation_cycle_root
                            .ok_or(RuntimeError::Invariant(
                                "async dependency has no cycle root",
                            ))?;
                        Some(RawModuleRef {
                            cache: dependency.cache,
                            module: cycle_root,
                        })
                    }
                    ModuleEvaluationVisit::Evaluating => {
                        let dependency_ancestor = dfs
                            .entries
                            .get(&dependency.module)
                            .map(|entry| entry.ancestor)
                            .ok_or(RuntimeError::Invariant(
                                "evaluating dependency has no DFS entry",
                            ))?;
                        let current_id = frames.last().map(|frame| frame.module.module).ok_or(
                            RuntimeError::Invariant(
                                "module evaluation call stack unexpectedly became empty",
                            ),
                        )?;
                        let entry =
                            dfs.entries
                                .get_mut(&current_id)
                                .ok_or(RuntimeError::Invariant(
                                    "evaluating module lost its DFS entry",
                                ))?;
                        entry.ancestor = entry.ancestor.min(dependency_ancestor);
                        Some(dependency)
                    }
                    ModuleEvaluationVisit::Unevaluated => {
                        // Revisit this exact dependency after its child frame
                        // returns. InnerModuleEvaluation must then canonicalize
                        // its cycle root and register any async blocker; merely
                        // advancing past the edge loses that post-child phase.
                        let parent = frames.last_mut().ok_or(RuntimeError::Invariant(
                            "module evaluation call stack unexpectedly became empty",
                        ))?;
                        parent.next_dependency = parent.next_dependency.checked_sub(1).ok_or(
                            RuntimeError::Invariant(
                                "module dependency cursor underflow before child evaluation",
                            ),
                        )?;
                        frames.push(self.enter_module_evaluation_dfs(dependency, dfs)?);
                        continue;
                    }
                    ModuleEvaluationVisit::Errored(exception) => {
                        if dfs.exception.replace(exception).is_some() {
                            return Err(RuntimeError::Invariant(
                                "module evaluation recorded more than one exception",
                            ));
                        }
                        return Err(RuntimeError::Exception);
                    }
                    ModuleEvaluationVisit::Poisoned => {
                        return Err(RuntimeError::Invariant(
                            "module evaluation previously failed inside the engine",
                        ));
                    }
                };
                if let Some(async_dependency) = async_dependency {
                    let dependency_record = self.module_record(async_dependency)?;
                    if matches!(
                        dependency_record.evaluation,
                        ModuleEvaluationState::Errored(_)
                    ) {
                        let ModuleEvaluationState::Errored(exception) =
                            dependency_record.evaluation
                        else {
                            unreachable!();
                        };
                        let exception = self.root_raw_value(&exception)?;
                        if dfs.exception.replace(exception).is_some() {
                            return Err(RuntimeError::Invariant(
                                "module evaluation recorded more than one exception",
                            ));
                        }
                        return Err(RuntimeError::Exception);
                    }
                    if dependency_record.async_evaluation_order.is_some() {
                        let parent = frames.last().map(|frame| frame.module).ok_or(
                            RuntimeError::Invariant(
                                "module evaluation call stack unexpectedly became empty",
                            ),
                        )?;
                        self.0
                            .state
                            .borrow_mut()
                            .heap
                            .add_loaded_module_async_dependency(async_dependency, parent)?;
                    }
                }
                continue;
            }

            let frame = frames.pop().ok_or(RuntimeError::Invariant(
                "module evaluation call stack unexpectedly became empty",
            ))?;
            let record = self.module_record(frame.module)?;
            let completion = if record.pending_async_dependencies != 0 {
                let order = self.next_module_async_evaluation_order()?;
                self.transition_module_record(
                    frame.module,
                    RawModuleTransition::BeginAsyncEvaluation { order },
                )?;
                Completion::Return(Value::Undefined)
            } else if record.has_top_level_await {
                let order = self.next_module_async_evaluation_order()?;
                self.transition_module_record(
                    frame.module,
                    RawModuleTransition::BeginAsyncEvaluation { order },
                )?;
                self.execute_async_module_body(evaluation_realm, frame.module)?;
                Completion::Return(Value::Undefined)
            } else {
                self.execute_module_body_synchronously(evaluation_realm, frame.module)?
            };
            match completion {
                Completion::Return(Value::Undefined) => {
                    let entry = dfs.entries.get(&frame.module.module).copied().ok_or(
                        RuntimeError::Invariant("evaluated module lost its DFS entry"),
                    )?;
                    if entry.index == entry.ancestor {
                        loop {
                            let member = *dfs.stack.last().ok_or(RuntimeError::Invariant(
                                "module evaluation SCC stack underflow",
                            ))?;
                            let member = RawModuleRef {
                                cache: frame.module.cache,
                                module: member,
                            };
                            let is_evaluating = matches!(
                                self.module_record(member)?.evaluation,
                                ModuleEvaluationState::Evaluating
                            );
                            if !is_evaluating {
                                return Err(RuntimeError::Invariant(
                                    "module evaluation SCC contained a non-evaluating member",
                                ));
                            }
                            self.transition_module_record(
                                member,
                                RawModuleTransition::FinishEvaluation {
                                    cycle_root: frame.module.module,
                                },
                            )?;
                            let popped = dfs.stack.pop().ok_or(RuntimeError::Invariant(
                                "module evaluation SCC stack underflow after publication",
                            ))?;
                            if popped != member.module {
                                return Err(RuntimeError::Invariant(
                                    "module evaluation SCC stack changed during record publication",
                                ));
                            }
                            if member.module == frame.module.module {
                                break;
                            }
                        }
                    }
                }
                Completion::Return(_) => {
                    self.transition_module_record(
                        frame.module,
                        RawModuleTransition::PoisonEvaluation,
                    )?;
                    return Err(RuntimeError::Invariant(
                        "module evaluation returned a non-undefined value",
                    ));
                }
                Completion::Throw(exception) => {
                    if dfs.exception.replace(exception).is_some() {
                        return Err(RuntimeError::Invariant(
                            "module evaluation recorded more than one exception",
                        ));
                    }
                    return Err(RuntimeError::Exception);
                }
            }

            let still_evaluating = matches!(
                self.module_record(frame.module)?.evaluation,
                ModuleEvaluationState::Evaluating
            );
            if still_evaluating {
                let dependency_ancestor = dfs
                    .entries
                    .get(&frame.module.module)
                    .map(|entry| entry.ancestor)
                    .ok_or(RuntimeError::Invariant(
                        "evaluating dependency has no DFS entry",
                    ))?;
                if let Some(parent) = frames.last() {
                    let entry = dfs.entries.get_mut(&parent.module.module).ok_or(
                        RuntimeError::Invariant("evaluating module lost its DFS entry"),
                    )?;
                    entry.ancestor = entry.ancestor.min(dependency_ancestor);
                }
            }
        }
        Ok(())
    }

    fn evaluate_module_graph(
        &self,
        evaluation_realm: ContextId,
        module: RawModuleRef,
    ) -> Result<Value, RuntimeError> {
        let mut dfs = ModuleEvaluationDfs::new();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.evaluate_module_dfs(evaluation_realm, module, &mut dfs)
        }));
        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                self.poison_active_module_evaluations(module, &dfs.stack)
                    .unwrap_or_else(|error| {
                        panic!("module evaluation panic cleanup failed: {error}")
                    });
                resume_unwind(payload);
            }
        };
        match result {
            Ok(()) => {
                if !dfs.stack.is_empty() || dfs.exception.is_some() {
                    return Err(RuntimeError::Invariant(
                        "successful module evaluation retained DFS state",
                    ));
                }
                Ok(Value::Undefined)
            }
            Err(RuntimeError::Exception) => {
                let exception = dfs.exception.take().ok_or(RuntimeError::Invariant(
                    "module evaluation exception had no cached value",
                ))?;
                if let Err(error) = self.cache_module_evaluation_exception(
                    module.cache,
                    module.module,
                    &dfs.stack,
                    &exception,
                ) {
                    self.poison_active_module_evaluations(module, &dfs.stack)?;
                    return Err(error);
                }
                drop(exception);
                Err(RuntimeError::Exception)
            }
            Err(error) => {
                self.poison_active_module_evaluations(module, &dfs.stack)?;
                Err(error)
            }
        }
    }

    /// Return the Context-owned Promise for one module evaluation attempt.
    ///
    /// Pinned QuickJS publishes `m->promise` before executing authored module
    /// code.  Static execution and dynamic import both enter through this
    /// helper so a cache hit observes the same Promise identity, settlement,
    /// and rejection-tracker history regardless of which API evaluated the
    /// record first. Async SCCs retain the capability in their cycle root and
    /// settle it later from the module completion reaction jobs.
    pub(super) fn evaluate_module_promise(
        &self,
        requested_module: RawModuleRef,
        initiating_realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let requested_record = self.module_record(requested_module)?;
        let module =
            match requested_record.evaluation {
                ModuleEvaluationState::EvaluatingAsync
                | ModuleEvaluationState::Evaluated
                | ModuleEvaluationState::Errored(_) => RawModuleRef {
                    cache: requested_module.cache,
                    module: requested_record.evaluation_cycle_root.ok_or(
                        RuntimeError::Invariant("completed module evaluation has no cycle root"),
                    )?,
                },
                ModuleEvaluationState::Unevaluated => requested_module,
                ModuleEvaluationState::Evaluating => {
                    return Err(RuntimeError::Invariant(
                        "module evaluation Promise was requested during evaluation",
                    ));
                }
                ModuleEvaluationState::Poisoned => {
                    return Err(RuntimeError::Invariant(
                        "module evaluation previously failed inside the engine",
                    ));
                }
            };
        let record = self.module_record(module)?;
        if let Some(promise) = record.evaluation_promise {
            return match record.evaluation {
                ModuleEvaluationState::EvaluatingAsync
                | ModuleEvaluationState::Evaluated
                | ModuleEvaluationState::Errored(_) => {
                    ObjectRef::from_borrowed_handle(self.clone(), promise).map_err(Into::into)
                }
                ModuleEvaluationState::Unevaluated => Err(RuntimeError::Invariant(
                    "module retained an unsettled Promise before evaluation",
                )),
                ModuleEvaluationState::Evaluating => Err(RuntimeError::Invariant(
                    "module cycle-root Promise was requested during evaluation",
                )),
                ModuleEvaluationState::Poisoned => Err(RuntimeError::Invariant(
                    "module cycle-root evaluation previously failed inside the engine",
                )),
            };
        }
        if matches!(record.evaluation, ModuleEvaluationState::Evaluating) {
            return Err(RuntimeError::Invariant(
                "module cycle-root Promise was requested during evaluation",
            ));
        }
        if matches!(record.evaluation, ModuleEvaluationState::Poisoned) {
            return Err(RuntimeError::Invariant(
                "module cycle-root evaluation previously failed inside the engine",
            ));
        }
        if record.link_status != ModuleLinkStatus::Linked {
            return Err(RuntimeError::Invariant(
                "module evaluation Promise was requested before linking",
            ));
        }

        let capability = self.new_default_promise_capability(initiating_realm)?;
        let promise = capability.promise.clone();
        self.0
            .state
            .borrow_mut()
            .heap
            .publish_loaded_module_evaluation_capability(
                module,
                promise.object_id(),
                capability.resolve.as_object().object_id(),
                capability.reject.as_object().object_id(),
            )?;

        let settlement = match record.evaluation {
            ModuleEvaluationState::Unevaluated => {
                match self.evaluate_module_graph(initiating_realm, module) {
                    Ok(Value::Undefined) => match self.module_record(module)?.evaluation {
                        ModuleEvaluationState::EvaluatingAsync => return Ok(promise),
                        ModuleEvaluationState::Evaluated => Ok((true, Value::Undefined)),
                        ModuleEvaluationState::Errored(reason) => {
                            Ok((false, self.root_raw_value(&reason)?))
                        }
                        ModuleEvaluationState::Unevaluated | ModuleEvaluationState::Evaluating => {
                            Err(RuntimeError::Invariant(
                                "successful module evaluation retained an active root state",
                            ))
                        }
                        ModuleEvaluationState::Poisoned => Err(RuntimeError::Invariant(
                            "module evaluation poisoned after a successful graph traversal",
                        )),
                    },
                    Ok(_) => Err(RuntimeError::Invariant(
                        "module evaluation returned a non-undefined value",
                    )),
                    Err(RuntimeError::Exception) => {
                        let reason =
                            self.take_pending_exception()?
                                .ok_or(RuntimeError::Invariant(
                                    "module evaluation failed without a pending exception",
                                ))?;
                        Ok((false, reason))
                    }
                    Err(error) => Err(error),
                }
            }
            ModuleEvaluationState::Evaluated => Ok((true, Value::Undefined)),
            ModuleEvaluationState::Errored(reason) => Ok((false, self.root_raw_value(&reason)?)),
            ModuleEvaluationState::EvaluatingAsync => return Ok(promise),
            ModuleEvaluationState::Evaluating => Err(RuntimeError::Invariant(
                "module evaluation Promise was requested during evaluation",
            )),
            ModuleEvaluationState::Poisoned => Err(RuntimeError::Invariant(
                "module evaluation previously failed inside the engine",
            )),
        }?;
        let target = if settlement.0 {
            &capability.resolve
        } else {
            &capability.reject
        };
        match self.call_internal(
            initiating_realm,
            target,
            Value::Undefined,
            std::slice::from_ref(&settlement.1),
        )? {
            Completion::Return(_) => Ok(promise),
            Completion::Throw(_) => Err(RuntimeError::Invariant(
                "intrinsic module Promise resolving function threw",
            )),
        }
    }

    fn dynamic_import_settler(&self, object: ObjectId) -> Result<CallableRef, RuntimeError> {
        let object = ObjectRef::from_borrowed_handle(self.clone(), object)?;
        self.as_callable(&object)?.ok_or(RuntimeError::Invariant(
            "dynamic import resolving function lost its callable brand",
        ))
    }

    fn call_dynamic_import_settler(
        &self,
        realm: ContextId,
        target: ObjectId,
        value: Value,
    ) -> Result<Completion, RuntimeError> {
        let target = self.dynamic_import_settler(target)?;
        match self.call_internal(realm, &target, Value::Undefined, &[value])? {
            Completion::Return(_) => Ok(Completion::Return(Value::Undefined)),
            Completion::Throw(_) => Err(RuntimeError::Invariant(
                "intrinsic dynamic import resolving function threw",
            )),
        }
    }

    fn dynamic_import_error_reason(
        &self,
        realm: ContextId,
        error: RuntimeError,
    ) -> Result<Value, RuntimeError> {
        match error {
            RuntimeError::Exception => {
                self.take_pending_exception()?
                    .ok_or(RuntimeError::Invariant(
                        "dynamic import failure had no pending exception",
                    ))
            }
            RuntimeError::Engine(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                self.new_native_error_from_error(realm, kind, &error)
            }
            RuntimeError::AbortedModule => self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "module construction or resolution was rolled back",
            ),
            RuntimeError::IncompleteModuleResolution => self.new_native_error(
                realm,
                NativeErrorKind::Internal,
                "module resolution is incomplete and cannot be linked safely",
            ),
            error => Err(error),
        }
    }

    fn reject_dynamic_import_error(
        &self,
        realm: ContextId,
        reject: ObjectId,
        error: RuntimeError,
    ) -> Result<Completion, RuntimeError> {
        let reason = self.dynamic_import_error_reason(realm, error)?;
        self.call_dynamic_import_settler(realm, reject, reason)
    }

    pub(super) fn call_dynamic_import_handler(
        &self,
        realm: ContextId,
        target_kind: DynamicImportHandlerKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "dynamic import handler received a constructor invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "dynamic import handler argv was not padded",
            ))?;
        let active = self.active_function()?;
        let internal = self
            .0
            .state
            .borrow()
            .heap
            .native_internal_callable(active.object_id())?
            .ok_or(RuntimeError::Invariant(
                "dynamic import handler has no internal state",
            ))?;
        let InternalCallableData::DynamicImportHandler {
            module,
            resolve,
            reject,
            kind,
        } = internal
        else {
            return Err(RuntimeError::Invariant(
                "dynamic import handler has the wrong internal state",
            ));
        };
        if kind != target_kind || module.cache != realm {
            return Err(RuntimeError::Invariant(
                "dynamic import handler target disagrees with its capture",
            ));
        }

        match target_kind {
            DynamicImportHandlerKind::Reject => {
                self.call_dynamic_import_settler(realm, reject, argument)
            }
            DynamicImportHandlerKind::Fulfill => match self.get_module_namespace_raw(module, realm)
            {
                Ok(namespace) => {
                    self.call_dynamic_import_settler(realm, resolve, Value::Object(namespace))
                }
                Err(error) => self.reject_dynamic_import_error(realm, reject, error),
            },
        }
    }

    pub(super) fn execute_dynamic_import_load_job(
        &self,
        realm: ContextId,
        resolve: ObjectId,
        reject: ObjectId,
        base_name: Option<&JsString>,
        specifier: &JsString,
        attributes: &ModuleImportAttributes,
    ) -> Result<Completion, RuntimeError> {
        let Some(base_name) = base_name else {
            let reason = self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "no function filename for import()",
            )?;
            return self.call_dynamic_import_settler(realm, reject, reason);
        };
        let module =
            match self.resolve_dynamic_import_module(realm, base_name, specifier, attributes) {
                Ok(module) => module,
                Err(error) => return self.reject_dynamic_import_error(realm, reject, error),
            };
        if let Err(error) = self.link_module_graph(module, realm) {
            return self.reject_dynamic_import_error(realm, reject, error);
        }
        let evaluation_promise = match self.evaluate_module_promise(module, realm) {
            Ok(promise) => promise,
            Err(error) => return self.reject_dynamic_import_error(realm, reject, error),
        };
        match self.attach_dynamic_import_finish(
            realm,
            &evaluation_promise,
            module,
            resolve,
            reject,
        )? {
            NativeConversion::Value(()) => Ok(Completion::Return(Value::Undefined)),
            NativeConversion::Throw(value) => {
                // `JS_LoadModuleInternal` frees the abrupt `js_promise_then`
                // result and the surrounding load job still returns
                // undefined. The runtime's current exception remains set,
                // while the caller-facing import Promise deliberately stays
                // pending in this edge case.
                self.set_pending_exception(value)?;
                Ok(Completion::Return(Value::Undefined))
            }
        }
    }

    fn cache_module_evaluation_exception(
        &self,
        cache: ContextId,
        cycle_root: ModuleId,
        active: &[ModuleId],
        exception: &Value,
    ) -> Result<(), RuntimeError> {
        self.validate_value_domain(exception, "module evaluation exception")?;
        let raw = self.raw_property_value(exception)?;
        let mut evaluating = Vec::with_capacity(active.len());
        for &id in active {
            if matches!(
                self.module_record(RawModuleRef { cache, module: id })?
                    .evaluation,
                ModuleEvaluationState::Evaluating
            ) {
                evaluating.push(id);
            }
        }
        let mut state = self.0.state.borrow_mut();
        state.retain_raw_root(&raw)?;
        let retained_atoms = match &raw {
            RawValue::Symbol(atom) => {
                let count = evaluating.len();
                let atoms = vec![*atom; count];
                match Self::retain_module_atoms(&mut state, atoms) {
                    Ok(atoms) => atoms,
                    Err(error) => {
                        state.release_owned_raw_root_committed(raw);
                        return Err(error);
                    }
                }
            }
            _ => Vec::new(),
        };
        if let Err(error) =
            state
                .heap
                .publish_loaded_module_errors(cache, &evaluating, cycle_root, raw.clone())
        {
            state
                .release_atoms(retained_atoms)
                .expect("module evaluation error atom rollback failed");
            state.release_owned_raw_root_committed(raw);
            return Err(error.into());
        }
        // One extra owned occurrence was prepared with the cache batch, so
        // pending-exception publication is now an infallible raw move.
        let previous = state.pending_exception.replace(raw);
        if let Some(previous) = previous {
            state.release_owned_raw_root_committed(previous);
        }
        Ok(())
    }

    fn poison_active_module_evaluations(
        &self,
        module: RawModuleRef,
        active: &[ModuleId],
    ) -> Result<(), RuntimeError> {
        for id in active {
            let member = RawModuleRef {
                cache: module.cache,
                module: *id,
            };
            let is_evaluating = matches!(
                self.module_record(member)?.evaluation,
                ModuleEvaluationState::Evaluating
            );
            if is_evaluating {
                self.transition_module_record(member, RawModuleTransition::PoisonEvaluation)?;
            }
        }
        Ok(())
    }

    pub(super) fn execute_module(
        &self,
        initiating_realm: ContextId,
        module: &ModuleBytecodeRef,
    ) -> Result<Value, RuntimeError> {
        if !module.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        self.0.state.borrow().heap.context(initiating_realm)?;
        self.link_module_graph(module.raw, initiating_realm)?;
        let promise = self.evaluate_module_promise(module.raw, initiating_realm)?;
        Ok(Value::Object(promise))
    }
}

impl Context {
    /// Compile one static ECMAScript module and publish its opaque module
    /// record without linking or evaluating it.
    pub fn compile_module(&mut self, source: &str) -> Result<ModuleBytecodeRef, RuntimeError> {
        self.compile_module_with_options(source, &CompileOptions::default())
    }

    /// Compile one static module with an explicit debug/source filename.
    pub fn compile_module_with_filename(
        &mut self,
        source: &str,
        filename: &str,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        self.compile_module_with_options(source, &CompileOptions::new(filename))
    }

    /// Compile one static module with named compilation options.
    ///
    /// Implemented JavaScript early errors become pending exceptions. Grammar
    /// which is not implemented remains an engine [`ErrorKind::Unsupported`]
    /// diagnostic, including unsupported source loaded through the module
    /// graph.
    pub fn compile_module_with_options(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        match self
            .runtime
            .compile_module_in_realm(self.realm, source, &options.filename)?
        {
            ModuleCompilation::Published(module) => self.runtime.root_module(module),
            ModuleCompilation::Throw(exception) => {
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Return this module's canonical `import.meta` object, allocating it on
    /// first request without linking or evaluating the module.
    ///
    /// This mirrors QuickJS `JS_GetImportMeta`: repeated calls and authored
    /// `import.meta` expressions observe the same ordinary null-prototype
    /// object. Hosts may define their own properties on the returned object
    /// before calling [`Self::execute_module`].
    pub fn get_module_import_meta(
        &mut self,
        module: &ModuleBytecodeRef,
    ) -> Result<ObjectRef, RuntimeError> {
        if !module.belongs_to(&self.runtime) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        self.runtime.0.state.borrow().heap.context(self.realm)?;
        self.runtime.get_or_create_module_import_meta(module.raw)
    }

    /// Link and evaluate one runtime-published static module, returning the
    /// cycle root's cached evaluation Promise on every normal engine path.
    pub fn execute_module(&mut self, module: &ModuleBytecodeRef) -> Result<Value, RuntimeError> {
        self.runtime.execute_module(self.realm, module)
    }

    /// Instantiate and link a resolved module graph without evaluating any
    /// authored module body. This makes the ECMAScript resolution phase
    /// observable to conformance harnesses while [`Self::execute_module`]
    /// retains the ordinary combined link/evaluate convenience.
    pub fn link_module(&mut self, module: &ModuleBytecodeRef) -> Result<(), RuntimeError> {
        if !module.belongs_to(&self.runtime) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        self.runtime.0.state.borrow().heap.context(self.realm)?;
        self.runtime.link_module_graph(module.raw, self.realm)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/runtime_module/tests.rs"]
mod tests;
