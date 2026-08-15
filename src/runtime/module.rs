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
mod tests {
    use super::*;
    use crate::heap::{PromiseData, PromiseState};

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
            ModuleLoaderError::exception(Value::String(JsString::from_static(
                "JavaScript exception"
            )))
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
                    ParseCacheProbeMode::PrefixSuccess
                    | ParseCacheProbeMode::PrefixOuterFailure => Ok(ModuleLoadResult::SourceText(
                        "export const before = 1;".to_owned(),
                    )),
                    ParseCacheProbeMode::PrefixLoadFailure
                    | ParseCacheProbeMode::PrefixLoadFailureSwallowed => Err(
                        ModuleLoaderError::new("intentional parse-time prefix load failure"),
                    ),
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

    fn module_evaluation_promise(context: &mut Context, module: &ModuleBytecodeRef) -> ObjectRef {
        let Value::Object(promise) = context.execute_module(module).unwrap() else {
            panic!("module evaluation did not return a Promise");
        };
        promise
    }

    fn module_evaluation_snapshot(
        context: &mut Context,
        module: &ModuleBytecodeRef,
    ) -> PromiseData {
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
        let promise =
            eval_dynamic_import(&mut context, "import('./dependency.js')", "pkg/entry.js");

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

        let promise =
            eval_dynamic_import(&mut context, "import('./dependency.js')", "pkg/entry.js");
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
        let Value::Object(second_namespace) = runtime.root_raw_value(&second.result).unwrap()
        else {
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
        let (loader, _, _) =
            MapModuleLoader::new([("species-throw.js", "export const ok = true;")]);
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
            JsString::from_static(
                "could not load module 'pkg/dependency.js': fixture loader2 failure"
            )
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
        let (loader, controls) =
            ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixCycleLoadPanic);
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
        let (loader, controls) =
            ParseCacheProbeLoader::new(ParseCacheProbeMode::PrefixOuterFailure);
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
            context.compile_module_with_filename(
                "import './b.js'; import './missing.js';",
                "pkg/a.js",
            ),
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
                ModuleImportMetaProperty::new(
                    JsString::from_static("foreign"),
                    Value::Object(foreign),
                ),
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
        let (loader, _, _) =
            MapModuleLoader::new([("pkg/dependency.js", "export const present = 1;")]);
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
        let (loader, loads, _) =
            MapModuleLoader::new([("pkg/loaded.js", "export const loaded = 42;")]);
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
        let compilation_function_prototype =
            compilation_context.eval("Function.prototype").unwrap();
        let compilation_array_prototype = compilation_context.eval("Array.prototype").unwrap();
        let compilation_type_error_prototype =
            compilation_context.eval("TypeError.prototype").unwrap();
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
}
