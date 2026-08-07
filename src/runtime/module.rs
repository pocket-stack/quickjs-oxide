//! Publication and execution for static ECMAScript modules.
//!
//! QuickJS publishes a `JSModuleDef` separately from the bytecode function it
//! drives. This slice keeps that ownership boundary across Context-local
//! caching, host resolution, live import cells, and iterative SCC
//! linking/evaluation. Namespace objects, transitive exports, and top-level
//! await remain explicit later frontiers.

use super::*;
use crate::compiler::{CompileOptions, compile_unlinked_module_with_name};
use crate::module::{
    ModuleExport, ModuleExportTarget, ModuleImport, ModuleLinkInitializer, ModuleRequest,
    ModuleStarExport, UnlinkedModule,
};
use std::collections::HashSet;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

/// Failure reported by an embedder-provided static-module loader.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModuleLoaderError {
    message: String,
}

impl ModuleLoaderError {
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    #[must_use]
    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for ModuleLoaderError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

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

/// Runtime-wide host boundary for source-text module normalization and load.
///
/// The loaded-module cache itself is Context-owned, matching QuickJS. The
/// loader is called synchronously during module compilation/resolution and
/// must not re-enter the same Runtime.
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

    /// Return UTF-8 ECMAScript source for one normalized module name.
    fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError>;
}

/// Host-owned lifetime token for an installed [`ModuleLoader`].
///
/// The Runtime keeps only a weak reference, matching QuickJS's host-owned
/// loader opaque and preventing `Runtime -> loader -> Runtime` reference
/// cycles. Keep this value alive for as long as module resolution should use
/// the loader. Dropping it disables the loader once no other registration
/// owns it. Dropping this token or calling [`Runtime::clear_module_loader`]
/// disables future graph transactions; a resolution already in flight keeps
/// its initial loader snapshot until it either commits or rolls back.
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

pub(super) struct ModuleGraph {
    records: RefCell<Vec<Option<Rc<ModuleRecord>>>>,
    first_by_name: RefCell<HashMap<JsString, usize>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ModuleId(usize);

impl ModuleGraph {
    pub(super) fn new() -> Self {
        Self {
            records: RefCell::new(Vec::new()),
            first_by_name: RefCell::new(HashMap::new()),
        }
    }

    fn publish(self: &Rc<Self>, record: ModuleRecord) -> ModuleBytecodeRef {
        let record = Rc::new(record);
        let index = {
            let mut records = self.records.borrow_mut();
            let index = records.len();
            records.push(Some(record.clone()));
            index
        };
        self.first_by_name
            .borrow_mut()
            .entry(record.name.clone())
            .or_insert(index);
        ModuleBytecodeRef {
            graph: self.clone(),
            id: ModuleId(index),
            record,
        }
    }

    fn first_named(self: &Rc<Self>, name: &JsString) -> Option<ModuleBytecodeRef> {
        let index = self.first_by_name.borrow().get(name).copied()?;
        let record = self.records.borrow().get(index)?.as_ref()?.clone();
        Some(ModuleBytecodeRef {
            graph: self.clone(),
            id: ModuleId(index),
            record,
        })
    }

    fn unpublish(&self, id: ModuleId) -> Result<(), RuntimeError> {
        let removed = {
            let mut records = self.records.borrow_mut();
            records
                .get_mut(id.0)
                .ok_or(RuntimeError::Invariant("module graph id is out of bounds"))?
                .take()
                .ok_or(RuntimeError::Invariant(
                    "module graph record was already unpublished",
                ))?
        };
        let name = removed.name.clone();
        if self.first_by_name.borrow().get(&name).copied() == Some(id.0) {
            let replacement =
                self.records
                    .borrow()
                    .iter()
                    .enumerate()
                    .find_map(|(index, record)| {
                        record
                            .as_ref()
                            .is_some_and(|record| record.name == name)
                            .then_some(index)
                    });
            let mut first_by_name = self.first_by_name.borrow_mut();
            if let Some(index) = replacement {
                first_by_name.insert(name, index);
            } else {
                first_by_name.remove(&name);
            }
        }
        // ModuleRecord drops release realm roots and may re-enter unrelated
        // Runtime state. Keep that destruction outside both graph borrows.
        drop(removed);
        Ok(())
    }

    fn unpublish_failed_resolution(
        &self,
        seeds: impl IntoIterator<Item = ModuleId>,
    ) -> Result<(), RuntimeError> {
        let mut doomed = seeds.into_iter().collect::<HashSet<_>>();
        if doomed.is_empty() {
            return Err(RuntimeError::Invariant(
                "failed module resolution had no records to roll back",
            ));
        }
        loop {
            let mut changed = false;
            {
                let records = self.records.borrow();
                for (index, record) in records.iter().enumerate() {
                    let id = ModuleId(index);
                    if doomed.contains(&id) {
                        continue;
                    }
                    let Some(record) = record else {
                        continue;
                    };
                    let depends_on_doomed = match &*record.resolution.borrow() {
                        ModuleResolutionState::Resolved(dependencies) => dependencies
                            .iter()
                            .any(|dependency| doomed.contains(dependency)),
                        ModuleResolutionState::Unresolved | ModuleResolutionState::Resolving => {
                            false
                        }
                    };
                    if depends_on_doomed {
                        doomed.insert(id);
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        let mut doomed = doomed.into_iter().collect::<Vec<_>>();
        doomed.sort_unstable_by_key(|id| std::cmp::Reverse(id.0));
        for id in doomed {
            self.unpublish(id)?;
        }
        Ok(())
    }

    fn record(&self, id: ModuleId) -> Result<Rc<ModuleRecord>, RuntimeError> {
        self.records
            .borrow()
            .get(id.0)
            .and_then(Option::as_ref)
            .cloned()
            .ok_or(RuntimeError::Invariant("module graph id is out of bounds"))
    }
}

/// Opaque owning handle for one runtime-published ECMAScript module record.
///
/// Clones preserve module identity and therefore share link/evaluation state.
/// The contained bytecode remains rooted for as long as any handle survives.
#[derive(Clone)]
pub struct ModuleBytecodeRef {
    graph: Rc<ModuleGraph>,
    id: ModuleId,
    record: Rc<ModuleRecord>,
}

struct ModuleRecord {
    name: JsString,
    function: FunctionBytecodeRef,
    // Retain the complete published record so the later graph linker can
    // extend this identity without changing the public compile boundary.
    _link_initializers: Box<[ModuleLinkInitializer]>,
    requested_modules: Box<[ModuleRequest]>,
    imports: Box<[ModuleImport]>,
    exports: Box<[ModuleExport]>,
    _star_exports: Box<[ModuleStarExport]>,
    resolution: RefCell<ModuleResolutionState>,
    instance: RefCell<Option<ModuleInstance>>,
    link_status: Cell<ModuleLinkStatus>,
    evaluation: RefCell<ModuleEvaluationState>,
    // QuickJS creates and caches the module function in the Context which
    // first executes the compiled module. Keep that link realm alive even
    // after its public Context handle is released and after evaluation has
    // discarded the cached callable from `state`.
    link_realm_root: RefCell<Option<ModuleRealmRoot>>,
    // Drop last, after cached callables and bytecode roots. A published module
    // retains its compilation realm through its bytecode and must not leave a
    // stale ContextId when the Context handle which compiled it is released.
    _realm_root: ModuleRealmRoot,
}

enum ModuleResolutionState {
    Unresolved,
    Resolving,
    Resolved(Box<[ModuleId]>),
}

struct ModuleRealmRoot {
    runtime: Runtime,
    realm: ContextId,
}

impl ModuleRealmRoot {
    fn retain(runtime: &Runtime, realm: ContextId) -> Result<Self, RuntimeError> {
        runtime.retain_context_handle(realm)?;
        Ok(Self {
            runtime: runtime.clone(),
            realm,
        })
    }
}

impl Drop for ModuleRealmRoot {
    fn drop(&mut self) {
        self.runtime.release_context_handle(self.realm);
    }
}

struct ModuleInstance {
    slots: Vec<Option<VarRefRoot>>,
    callable: Option<CallableRef>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ModuleLinkStatus {
    Unlinked,
    Linking,
    Linked,
    Poisoned,
}

enum ModuleEvaluationState {
    Unevaluated,
    Evaluating,
    Evaluated,
    Errored(Value),
    Poisoned,
}

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
    module: ModuleBytecodeRef,
    next_request: usize,
    dependencies: Vec<ModuleId>,
    loader: Option<Rc<dyn ModuleLoader>>,
}

struct ModuleDfsFrame {
    module: ModuleBytecodeRef,
    dependencies: Vec<ModuleBytecodeRef>,
    next_dependency: usize,
}

enum ModuleEvaluationVisit {
    Unevaluated,
    Evaluating,
    Evaluated,
    Errored(Value),
    Poisoned,
}

struct ModuleResolutionGuard<'a> {
    active: &'a Cell<bool>,
}

impl Drop for ModuleResolutionGuard<'_> {
    fn drop(&mut self) {
        self.active.set(false);
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
    Published(ModuleBytecodeRef),
    Throw(Value),
}

impl ModuleBytecodeRef {
    /// Return the source/debug name attached to this module record.
    #[must_use]
    pub fn name(&self) -> &JsString {
        &self.record.name
    }

    /// Return whether this module was published by `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.record.function.belongs_to(runtime)
    }

    /// Return whether two handles name modules in the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.record.function.is_same_runtime(&other.record.function)
    }

    /// Stable identity of the runtime domain which published this module.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.record.function.domain_id()
    }
}

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
    /// Install the runtime-wide source-text module loader used by subsequent
    /// Context module resolution. Existing Context caches remain intact.
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

    /// Compile and publish a static module without touching the runtime's
    /// pending-exception slot. The public Context boundary installs a thrown
    /// syntax exception exactly as the Script compilation path does.
    fn compile_module_record_in_realm(
        &self,
        realm: ContextId,
        graph: &Rc<ModuleGraph>,
        source: &str,
        name: &JsString,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<ModuleCompilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let module = match compile_unlinked_module_with_name(source, name.clone(), debug_info) {
            Ok(module) => module,
            Err(mut error) => {
                if error.kind() == ErrorKind::Unsupported && !preserve_unsupported_diagnostics {
                    let span = error.span();
                    error = Error::new(ErrorKind::Syntax, error.message().to_owned());
                    if let Some(span) = span {
                        error = error.with_span(span);
                    }
                }
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
        self.publish_unlinked_module(realm, graph, module)
            .map(ModuleCompilation::Published)
    }

    fn compile_module_in_realm(
        &self,
        realm: ContextId,
        graph: &Rc<ModuleGraph>,
        source: &str,
        filename: &str,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<ModuleCompilation, RuntimeError> {
        let name = module_c_string_view(&JsString::try_from_utf8(filename)?)?;
        let compilation = self.compile_module_record_in_realm(
            realm,
            graph,
            source,
            &name,
            preserve_unsupported_diagnostics,
        )?;
        let ModuleCompilation::Published(module) = compilation else {
            return Ok(compilation);
        };
        self.resolve_module_graph(realm, &module, preserve_unsupported_diagnostics)?;
        Ok(ModuleCompilation::Published(module))
    }

    fn resolve_module_graph(
        &self,
        realm: ContextId,
        module: &ModuleBytecodeRef,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<(), RuntimeError> {
        if !module.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        if self.0.module_resolution_active.replace(true) {
            module
                .graph
                .unpublish_failed_resolution(std::iter::once(module.id))?;
            return Err(RuntimeError::Invariant(
                "module loader re-entered source-text module resolution",
            ));
        }
        let _resolution_guard = ModuleResolutionGuard {
            active: &self.0.module_resolution_active,
        };
        match &*module.record.resolution.borrow() {
            ModuleResolutionState::Resolved(_) | ModuleResolutionState::Resolving => return Ok(()),
            ModuleResolutionState::Unresolved => {}
        }
        *module.record.resolution.borrow_mut() = ModuleResolutionState::Resolving;
        let loader = {
            self.0
                .module_loader
                .borrow()
                .as_ref()
                .and_then(Weak::upgrade)
        };
        let mut stack = vec![ModuleResolveFrame {
            module: module.clone(),
            next_request: 0,
            dependencies: Vec::with_capacity(module.record.requested_modules.len()),
            loader,
        }];

        let outcome = catch_unwind(AssertUnwindSafe(|| {
            while let Some(frame) = stack.last() {
                if frame.next_request == frame.module.record.requested_modules.len() {
                    let frame = stack.pop().ok_or(RuntimeError::Invariant(
                        "module resolution stack unexpectedly became empty",
                    ))?;
                    *frame.module.record.resolution.borrow_mut() =
                        ModuleResolutionState::Resolved(frame.dependencies.into_boxed_slice());
                    continue;
                }

                let (current, request, loader) = {
                    let frame = stack.last_mut().ok_or(RuntimeError::Invariant(
                        "module resolution stack unexpectedly became empty",
                    ))?;
                    let request = frame
                        .module
                        .record
                        .requested_modules
                        .get(frame.next_request)
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "module request index is outside its record",
                        ))?;
                    frame.next_request += 1;
                    (frame.module.clone(), request, frame.loader.clone())
                };
                let base_name = module_c_string_view(&current.record.name)?;
                let specifier = module_c_string_view(&request.specifier)?;
                let normalized_name = if let Some(loader) = &loader {
                    loader.normalize(&base_name, &specifier).map_err(|error| {
                        module_reference_error(
                            "could not normalize module '",
                            &specifier,
                            &format!("': {error}"),
                        )
                    })?
                } else {
                    default_module_normalize_name(&base_name, &specifier)?
                };
                let normalized_name = module_c_string_view(&normalized_name)?;
                let dependency = if let Some(cached) = current.graph.first_named(&normalized_name) {
                    cached
                } else {
                    let Some(loader) = &loader else {
                        return Err(module_reference_error(
                            "could not load module '",
                            &normalized_name,
                            "'",
                        ));
                    };
                    let source = loader.load(&normalized_name).map_err(|error| {
                        module_reference_error(
                            "could not load module '",
                            &normalized_name,
                            &format!("': {error}"),
                        )
                    })?;
                    match self.compile_module_record_in_realm(
                        realm,
                        &current.graph,
                        &source,
                        &normalized_name,
                        preserve_unsupported_diagnostics,
                    )? {
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
                    .push(dependency.id);

                let needs_resolution = {
                    let resolution = dependency.record.resolution.borrow();
                    matches!(&*resolution, ModuleResolutionState::Unresolved)
                };
                if needs_resolution {
                    *dependency.record.resolution.borrow_mut() = ModuleResolutionState::Resolving;
                    stack.push(ModuleResolveFrame {
                        module: dependency.clone(),
                        next_request: 0,
                        dependencies: Vec::with_capacity(dependency.record.requested_modules.len()),
                        loader,
                    });
                }
            }
            Ok(())
        }));

        let result = match outcome {
            Ok(result) => result,
            Err(payload) => {
                if !stack.is_empty() {
                    self.rollback_module_resolution_stack(module, &stack)
                        .unwrap_or_else(|error| {
                            panic!("module resolution panic rollback failed: {error}")
                        });
                }
                resume_unwind(payload);
            }
        };

        if result.is_err() {
            self.rollback_module_resolution_stack(module, &stack)?;
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

    fn rollback_module_resolution_stack(
        &self,
        module: &ModuleBytecodeRef,
        stack: &[ModuleResolveFrame],
    ) -> Result<(), RuntimeError> {
        for frame in stack {
            let is_resolving = {
                let resolution = frame.module.record.resolution.borrow();
                matches!(&*resolution, ModuleResolutionState::Resolving)
            };
            if is_resolving {
                *frame.module.record.resolution.borrow_mut() = ModuleResolutionState::Unresolved;
            }
        }
        module
            .graph
            .unpublish_failed_resolution(stack.iter().map(|frame| frame.module.id))
    }

    pub(super) fn publish_unlinked_module(
        &self,
        realm: ContextId,
        graph: &Rc<ModuleGraph>,
        module: UnlinkedModule,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        bytecode_publish::verify_unlinked_module_tree(&module)?;

        // Namespace and transitive-export machinery remains fail-closed until
        // the namespace-exotic/complete ResolveExport milestone. R3dy admits
        // side-effect edges and direct named imports only.
        let has_indirect_export = module
            .exports()
            .iter()
            .any(|export| matches!(export.target, ModuleExportTarget::Indirect { .. }));
        if module.imports().iter().any(|import| import.is_namespace)
            || !module.star_exports().is_empty()
            || has_indirect_export
        {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Unsupported,
                "module namespace and transitive export linking is not implemented",
            )));
        }

        let realm_root = ModuleRealmRoot::retain(self, realm)?;
        let parts = module.into_parts();
        let function = self.publish_verified_unlinked_function(realm, parts.function)?;
        Ok(graph.publish(ModuleRecord {
            name: parts.name,
            function,
            _link_initializers: parts.link_initializers,
            requested_modules: parts.requested_modules,
            imports: parts.imports,
            exports: parts.exports,
            _star_exports: parts.star_exports,
            resolution: RefCell::new(ModuleResolutionState::Unresolved),
            instance: RefCell::new(None),
            link_status: Cell::new(ModuleLinkStatus::Unlinked),
            evaluation: RefCell::new(ModuleEvaluationState::Unevaluated),
            link_realm_root: RefCell::new(None),
            _realm_root: realm_root,
        }))
    }

    fn module_dependencies(
        &self,
        module: &ModuleBytecodeRef,
    ) -> Result<Vec<ModuleBytecodeRef>, RuntimeError> {
        let ids = match &*module.record.resolution.borrow() {
            ModuleResolutionState::Resolved(ids) => ids.to_vec(),
            ModuleResolutionState::Unresolved | ModuleResolutionState::Resolving => {
                return Err(RuntimeError::Invariant(
                    "module execution reached an unresolved graph",
                ));
            }
        };
        ids.into_iter()
            .map(|id| {
                Ok(ModuleBytecodeRef {
                    graph: module.graph.clone(),
                    id,
                    record: module.graph.record(id)?,
                })
            })
            .collect()
    }

    fn prepare_module_instance(
        &self,
        module: &ModuleBytecodeRef,
        link_realm: ContextId,
    ) -> Result<(), RuntimeError> {
        let mut pending = vec![module.clone()];
        while let Some(current) = pending.pop() {
            if current.record.instance.borrow().is_some() {
                continue;
            }
            self.prepare_single_module_instance(&current, link_realm)?;
            let dependencies = self.module_dependencies(&current)?;
            pending.extend(dependencies.into_iter().rev());
        }
        Ok(())
    }

    fn prepare_single_module_instance(
        &self,
        module: &ModuleBytecodeRef,
        link_realm: ContextId,
    ) -> Result<(), RuntimeError> {
        if module.record.instance.borrow().is_some() {
            return Ok(());
        }
        if !module.record.function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        let descriptors = {
            let state = self.0.state.borrow();
            state
                .heap
                .function_bytecode(module.record.function.bytecode_id())?
                .closure_variables
                .clone()
        };

        let mut slots = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors.iter().copied() {
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

        if module.record.link_realm_root.borrow().is_some() {
            return Err(RuntimeError::Invariant(
                "uninstantiated module retained a link realm",
            ));
        }
        *module.record.link_realm_root.borrow_mut() =
            Some(ModuleRealmRoot::retain(self, link_realm)?);
        *module.record.instance.borrow_mut() = Some(ModuleInstance {
            slots,
            callable: None,
        });
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

    /// Resolve the ultimate declaration cell behind a local export.
    ///
    /// R3dy still rejects indirect and star export syntax at publication, but
    /// `export { importedName }` is represented as a local export of an import
    /// view. Following that authenticated view here avoids depending on SCC
    /// link order and gives circular alias chains a JavaScript SyntaxError
    /// instead of an absent-cell engine invariant.
    fn resolve_module_export_cell(
        &self,
        module: &ModuleBytecodeRef,
        export_name: &JsString,
        realm: ContextId,
    ) -> Result<VarRefRoot, RuntimeError> {
        let error_module_name = module.record.name.clone();
        let error_export_name = export_name.clone();
        let mut current = module.clone();
        let mut name = export_name.clone();
        let mut resolve_set = HashSet::new();

        loop {
            if !resolve_set.insert((current.id, name.utf16_units().collect::<Vec<_>>())) {
                return self.throw_module_link_syntax_error(
                    realm,
                    module_export_error_message(
                        "circular reference when looking for export '",
                        &error_export_name,
                        &error_module_name,
                    ),
                );
            }
            let Some(export) = current
                .record
                .exports
                .iter()
                .find(|export| export.export_name == name)
            else {
                return self.throw_module_link_syntax_error(
                    realm,
                    module_export_error_message(
                        "Could not find export '",
                        &error_export_name,
                        &error_module_name,
                    ),
                );
            };
            let ModuleExportTarget::Local { closure_index } = export.target else {
                return Err(RuntimeError::Invariant(
                    "indirect export escaped its publication frontier",
                ));
            };
            let descriptor = {
                let state = self.0.state.borrow();
                state
                    .heap
                    .function_bytecode(current.record.function.bytecode_id())?
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
                    return current
                        .record
                        .instance
                        .borrow()
                        .as_ref()
                        .and_then(|instance| instance.slots.get(usize::from(closure_index)))
                        .and_then(Option::as_ref)
                        .cloned()
                        .ok_or(RuntimeError::Invariant(
                            "resolved export has no instantiated live cell",
                        ));
                }
                ClosureSource::ModuleImport => {
                    if descriptor.kind != ClosureVariableKind::ModuleImportView {
                        return Err(RuntimeError::Invariant(
                            "resolved module import export has invalid metadata",
                        ));
                    }
                    let import = current
                        .record
                        .imports
                        .iter()
                        .find(|import| import.closure_index == closure_index)
                        .ok_or(RuntimeError::Invariant(
                            "exported module import has no import table entry",
                        ))?;
                    if import.is_namespace {
                        return Err(RuntimeError::Invariant(
                            "namespace import escaped its publication frontier",
                        ));
                    }
                    let request = import.request;
                    let import_name = import.import_name.clone();
                    let dependencies = self.module_dependencies(&current)?;
                    current = dependencies.get(request.0 as usize).cloned().ok_or(
                        RuntimeError::Invariant(
                            "exported module import request is outside the resolved graph",
                        ),
                    )?;
                    name = import_name;
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

    fn link_module_imports(
        &self,
        module: &ModuleBytecodeRef,
        dependencies: &[ModuleBytecodeRef],
        realm: ContextId,
    ) -> Result<(), RuntimeError> {
        for import in module.record.imports.iter() {
            if import.is_namespace {
                return Err(RuntimeError::Invariant(
                    "namespace import escaped its publication frontier",
                ));
            }
            let dependency =
                dependencies
                    .get(import.request.0 as usize)
                    .ok_or(RuntimeError::Invariant(
                        "module import request is outside the resolved graph",
                    ))?;
            let slot = self.resolve_module_export_cell(dependency, &import.import_name, realm)?;
            let mut instance = module.record.instance.borrow_mut();
            let target = instance
                .as_mut()
                .and_then(|instance| instance.slots.get_mut(usize::from(import.closure_index)))
                .ok_or(RuntimeError::Invariant(
                    "module import closure is outside the instance",
                ))?;
            *target = Some(slot);
        }
        Ok(())
    }

    fn create_module_callable(
        &self,
        module: &ModuleBytecodeRef,
        realm: ContextId,
    ) -> Result<CallableRef, RuntimeError> {
        if let Some(callable) = module
            .record
            .instance
            .borrow()
            .as_ref()
            .and_then(|instance| instance.callable.clone())
        {
            return Ok(callable);
        }
        let slots = module
            .record
            .instance
            .borrow()
            .as_ref()
            .ok_or(RuntimeError::Invariant("module has no instance"))?
            .slots
            .iter()
            .map(|slot| {
                slot.clone().ok_or(RuntimeError::Invariant(
                    "module callable retained an unresolved import slot",
                ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let callable =
            self.new_bytecode_closure_with_slots(realm, &module.record.function, &slots)?;
        module
            .record
            .instance
            .borrow_mut()
            .as_mut()
            .ok_or(RuntimeError::Invariant("module instance disappeared"))?
            .callable = Some(callable.clone());
        Ok(callable)
    }

    fn enter_module_link_dfs(
        &self,
        module: &ModuleBytecodeRef,
        dfs: &mut ModuleLinkDfs,
    ) -> Result<ModuleDfsFrame, RuntimeError> {
        if module.record.link_status.get() != ModuleLinkStatus::Unlinked {
            return Err(RuntimeError::Invariant(
                "link DFS entered a module which was not unlinked",
            ));
        }
        module.record.link_status.set(ModuleLinkStatus::Linking);
        let index = dfs.next_index;
        dfs.next_index = dfs
            .next_index
            .checked_add(1)
            .ok_or(RuntimeError::Invariant("module link DFS index overflow"))?;
        if dfs
            .entries
            .insert(
                module.id,
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
        dfs.stack.push(module.id);
        let dependencies = self.module_dependencies(module)?;
        Ok(ModuleDfsFrame {
            module: module.clone(),
            dependencies,
            next_dependency: 0,
        })
    }

    fn link_module_dfs(
        &self,
        module: &ModuleBytecodeRef,
        dfs: &mut ModuleLinkDfs,
    ) -> Result<(), RuntimeError> {
        match module.record.link_status.get() {
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
                match dependency.record.link_status.get() {
                    ModuleLinkStatus::Linked => {}
                    ModuleLinkStatus::Linking => {
                        let dependency_ancestor = dfs
                            .entries
                            .get(&dependency.id)
                            .map(|entry| entry.ancestor)
                            .ok_or(RuntimeError::Invariant(
                                "linking dependency has no DFS entry",
                            ))?;
                        let current_id = frames.last().map(|frame| frame.module.id).ok_or(
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
                        frames.push(self.enter_module_link_dfs(&dependency, dfs)?);
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
            let realm = frame
                .module
                .record
                .link_realm_root
                .borrow()
                .as_ref()
                .map(|root| root.realm)
                .ok_or(RuntimeError::Invariant(
                    "instantiated module has no retained link realm",
                ))?;
            self.link_module_imports(&frame.module, &frame.dependencies, realm)?;
            let callable = self.create_module_callable(&frame.module, realm)?;
            let completion = match self.call_internal(realm, &callable, Value::Bool(true), &[]) {
                Ok(completion) => completion,
                Err(error) => {
                    frame
                        .module
                        .record
                        .link_status
                        .set(ModuleLinkStatus::Poisoned);
                    return Err(error);
                }
            };
            match completion {
                Completion::Return(Value::Undefined) => {
                    let entry = dfs
                        .entries
                        .get(&frame.module.id)
                        .copied()
                        .ok_or(RuntimeError::Invariant("linked module lost its DFS entry"))?;
                    if entry.index == entry.ancestor {
                        loop {
                            let member = dfs.stack.pop().ok_or(RuntimeError::Invariant(
                                "module link SCC stack underflow",
                            ))?;
                            let record = frame.module.graph.record(member)?;
                            if record.link_status.get() != ModuleLinkStatus::Linking {
                                return Err(RuntimeError::Invariant(
                                    "module link SCC contained a non-linking member",
                                ));
                            }
                            record.link_status.set(ModuleLinkStatus::Linked);
                            if member == frame.module.id {
                                break;
                            }
                        }
                    }
                }
                Completion::Return(_) => {
                    frame
                        .module
                        .record
                        .link_status
                        .set(ModuleLinkStatus::Poisoned);
                    return Err(RuntimeError::Invariant(
                        "module link entry returned a non-undefined value",
                    ));
                }
                Completion::Throw(exception) => {
                    self.set_pending_exception(exception)?;
                    return Err(RuntimeError::Exception);
                }
            }

            if frame.module.record.link_status.get() == ModuleLinkStatus::Linking {
                let dependency_ancestor = dfs
                    .entries
                    .get(&frame.module.id)
                    .map(|entry| entry.ancestor)
                    .ok_or(RuntimeError::Invariant(
                        "linking dependency has no DFS entry",
                    ))?;
                if let Some(parent) = frames.last() {
                    let entry = dfs
                        .entries
                        .get_mut(&parent.module.id)
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

    fn link_module_graph(
        &self,
        module: &ModuleBytecodeRef,
        initiating_realm: ContextId,
    ) -> Result<(), RuntimeError> {
        self.prepare_module_instance(module, initiating_realm)?;
        let mut dfs = ModuleLinkDfs::new();
        let result = self.link_module_dfs(module, &mut dfs);
        if result.is_err() {
            for id in dfs.stack {
                let record = module.graph.record(id)?;
                if record.link_status.get() == ModuleLinkStatus::Linking {
                    record.link_status.set(ModuleLinkStatus::Unlinked);
                }
            }
        }
        result
    }

    fn enter_module_evaluation_dfs(
        &self,
        module: &ModuleBytecodeRef,
        dfs: &mut ModuleEvaluationDfs,
    ) -> Result<ModuleDfsFrame, RuntimeError> {
        if !matches!(
            &*module.record.evaluation.borrow(),
            ModuleEvaluationState::Unevaluated
        ) {
            return Err(RuntimeError::Invariant(
                "evaluation DFS entered a module which was not unevaluated",
            ));
        }
        *module.record.evaluation.borrow_mut() = ModuleEvaluationState::Evaluating;
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
                module.id,
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
        dfs.stack.push(module.id);
        Ok(ModuleDfsFrame {
            module: module.clone(),
            dependencies: self.module_dependencies(module)?,
            next_dependency: 0,
        })
    }

    fn evaluate_module_dfs(
        &self,
        module: &ModuleBytecodeRef,
        dfs: &mut ModuleEvaluationDfs,
    ) -> Result<(), RuntimeError> {
        let initial_state = {
            let evaluation = module.record.evaluation.borrow();
            match &*evaluation {
                ModuleEvaluationState::Unevaluated => ModuleEvaluationVisit::Unevaluated,
                ModuleEvaluationState::Evaluating => ModuleEvaluationVisit::Evaluating,
                ModuleEvaluationState::Evaluated => ModuleEvaluationVisit::Evaluated,
                ModuleEvaluationState::Errored(exception) => {
                    ModuleEvaluationVisit::Errored(exception.clone())
                }
                ModuleEvaluationState::Poisoned => ModuleEvaluationVisit::Poisoned,
            }
        };
        match initial_state {
            ModuleEvaluationVisit::Evaluated => return Ok(()),
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
                    let evaluation = dependency.record.evaluation.borrow();
                    match &*evaluation {
                        ModuleEvaluationState::Unevaluated => ModuleEvaluationVisit::Unevaluated,
                        ModuleEvaluationState::Evaluating => ModuleEvaluationVisit::Evaluating,
                        ModuleEvaluationState::Evaluated => ModuleEvaluationVisit::Evaluated,
                        ModuleEvaluationState::Errored(exception) => {
                            ModuleEvaluationVisit::Errored(exception.clone())
                        }
                        ModuleEvaluationState::Poisoned => ModuleEvaluationVisit::Poisoned,
                    }
                };
                match dependency_state {
                    ModuleEvaluationVisit::Evaluated => {}
                    ModuleEvaluationVisit::Evaluating => {
                        let dependency_ancestor = dfs
                            .entries
                            .get(&dependency.id)
                            .map(|entry| entry.ancestor)
                            .ok_or(RuntimeError::Invariant(
                                "evaluating dependency has no DFS entry",
                            ))?;
                        let current_id = frames.last().map(|frame| frame.module.id).ok_or(
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
                    }
                    ModuleEvaluationVisit::Unevaluated => {
                        frames.push(self.enter_module_evaluation_dfs(&dependency, dfs)?);
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
                }
                continue;
            }

            let frame = frames.pop().ok_or(RuntimeError::Invariant(
                "module evaluation call stack unexpectedly became empty",
            ))?;
            let (callable, realm) = {
                let instance = frame.module.record.instance.borrow();
                let callable = instance
                    .as_ref()
                    .and_then(|instance| instance.callable.clone())
                    .ok_or(RuntimeError::Invariant(
                        "linked module has no callable instance",
                    ))?;
                let realm = frame
                    .module
                    .record
                    .link_realm_root
                    .borrow()
                    .as_ref()
                    .map(|root| root.realm)
                    .ok_or(RuntimeError::Invariant(
                        "linked module has no retained realm",
                    ))?;
                (callable, realm)
            };
            let completion = self.call_internal(realm, &callable, Value::Undefined, &[])?;
            match completion {
                Completion::Return(Value::Undefined) => {
                    let entry = dfs.entries.get(&frame.module.id).copied().ok_or(
                        RuntimeError::Invariant("evaluated module lost its DFS entry"),
                    )?;
                    if entry.index == entry.ancestor {
                        loop {
                            let member = dfs.stack.pop().ok_or(RuntimeError::Invariant(
                                "module evaluation SCC stack underflow",
                            ))?;
                            let record = frame.module.graph.record(member)?;
                            let is_evaluating = {
                                let evaluation = record.evaluation.borrow();
                                matches!(&*evaluation, ModuleEvaluationState::Evaluating)
                            };
                            if !is_evaluating {
                                return Err(RuntimeError::Invariant(
                                    "module evaluation SCC contained a non-evaluating member",
                                ));
                            }
                            *record.evaluation.borrow_mut() = ModuleEvaluationState::Evaluated;
                            if member == frame.module.id {
                                break;
                            }
                        }
                    }
                }
                Completion::Return(_) => {
                    *frame.module.record.evaluation.borrow_mut() = ModuleEvaluationState::Poisoned;
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

            let still_evaluating = {
                let evaluation = frame.module.record.evaluation.borrow();
                matches!(&*evaluation, ModuleEvaluationState::Evaluating)
            };
            if still_evaluating {
                let dependency_ancestor = dfs
                    .entries
                    .get(&frame.module.id)
                    .map(|entry| entry.ancestor)
                    .ok_or(RuntimeError::Invariant(
                        "evaluating dependency has no DFS entry",
                    ))?;
                if let Some(parent) = frames.last() {
                    let entry =
                        dfs.entries
                            .get_mut(&parent.module.id)
                            .ok_or(RuntimeError::Invariant(
                                "evaluating module lost its DFS entry",
                            ))?;
                    entry.ancestor = entry.ancestor.min(dependency_ancestor);
                }
            }
        }
        Ok(())
    }

    fn evaluate_module_graph(&self, module: &ModuleBytecodeRef) -> Result<Value, RuntimeError> {
        let mut dfs = ModuleEvaluationDfs::new();
        let outcome = catch_unwind(AssertUnwindSafe(|| {
            self.evaluate_module_dfs(module, &mut dfs)
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
                for id in dfs.stack {
                    let record = module.graph.record(id)?;
                    if matches!(
                        &*record.evaluation.borrow(),
                        ModuleEvaluationState::Evaluating
                    ) {
                        *record.evaluation.borrow_mut() =
                            ModuleEvaluationState::Errored(exception.clone());
                    }
                }
                self.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
            Err(error) => {
                self.poison_active_module_evaluations(module, &dfs.stack)?;
                Err(error)
            }
        }
    }

    fn poison_active_module_evaluations(
        &self,
        module: &ModuleBytecodeRef,
        active: &[ModuleId],
    ) -> Result<(), RuntimeError> {
        for id in active {
            let record = module.graph.record(*id)?;
            let is_evaluating = {
                let evaluation = record.evaluation.borrow();
                matches!(&*evaluation, ModuleEvaluationState::Evaluating)
            };
            if is_evaluating {
                *record.evaluation.borrow_mut() = ModuleEvaluationState::Poisoned;
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
        self.link_module_graph(module, initiating_realm)?;
        self.evaluate_module_graph(module)
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
    pub fn compile_module_with_options(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        self.compile_module_with_options_internal(source, options, false)
    }

    /// Compile a module while retaining an implementation-frontier
    /// [`ErrorKind::Unsupported`] diagnostic for conformance harnesses.
    pub fn compile_module_with_options_preserving_unsupported_diagnostics(
        &mut self,
        source: &str,
        options: &CompileOptions,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        self.compile_module_with_options_internal(source, options, true)
    }

    fn compile_module_with_options_internal(
        &mut self,
        source: &str,
        options: &CompileOptions,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        match self.runtime.compile_module_in_realm(
            self.realm,
            &self.module_graph,
            source,
            &options.filename,
            preserve_unsupported_diagnostics,
        )? {
            ModuleCompilation::Published(module) => Ok(module),
            ModuleCompilation::Throw(exception) => {
                self.runtime.set_pending_exception(exception)?;
                Err(RuntimeError::Exception)
            }
        }
    }

    /// Link and evaluate one runtime-published static module. This first
    /// synchronous graph slice supports side-effect and direct named imports.
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
        self.runtime.link_module_graph(module, self.realm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    type SharedLoaderSources = Rc<RefCell<HashMap<String, String>>>;
    type SharedLoaderLoads = Rc<RefCell<Vec<String>>>;
    type SharedLoaderNormalizations = Rc<RefCell<Vec<(String, String)>>>;
    type SharedUtf16LoaderLoads = Rc<RefCell<Vec<Vec<u16>>>>;

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

    struct ReentrantModuleLoader {
        context: Rc<RefCell<Context>>,
        attempted: Cell<bool>,
        rejected: Rc<Cell<bool>>,
    }

    impl fmt::Debug for ReentrantModuleLoader {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("ReentrantModuleLoader")
        }
    }

    impl ModuleLoader for ReentrantModuleLoader {
        fn load(&self, normalized_name: &JsString) -> Result<String, ModuleLoaderError> {
            if !self.attempted.replace(true) {
                let result = self
                    .context
                    .borrow_mut()
                    .compile_module_with_filename("export const stale = 1;", "pkg/reentrant.js");
                self.rejected.set(matches!(
                    result,
                    Err(RuntimeError::Invariant(
                        "module loader re-entered source-text module resolution"
                    ))
                ));
            }
            match valid_fixture_module_name(normalized_name)?.as_str() {
                "pkg/dependency.js" => Ok("export const answer = 42;".to_owned()),
                _ => Err(ModuleLoaderError::new("fixture module is missing")),
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
    fn active_graph_keeps_its_initial_loader_snapshot_after_host_clear() {
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
        let module = context
            .compile_module_with_filename(
                "import { answer } from './a.js'; globalThis.__loaderSnapshot = answer;",
                "pkg/entry.js",
            )
            .unwrap();

        context.execute_module(&module).unwrap();
        assert_script_true(&mut context, "__loaderSnapshot === 42");
        assert_eq!(&*loads.borrow(), &["pkg/a.js", "pkg/b.js"]);

        assert!(matches!(
            context.compile_module_with_filename("import './fresh.js';", "pkg/next.js"),
            Err(RuntimeError::Exception)
        ));
        assert!(matches!(
            context.take_exception().unwrap(),
            Some(Value::Object(_))
        ));
        assert_eq!(&*loads.borrow(), &["pkg/a.js", "pkg/b.js"]);
    }

    #[test]
    fn loader_panic_rolls_back_the_active_resolution_transaction() {
        let runtime = Runtime::new();
        let panicking_registration = runtime.set_module_loader(PanickingModuleLoader);
        let mut context = runtime.new_context();

        let panic = catch_unwind(AssertUnwindSafe(|| {
            let _ = context.compile_module_with_filename(
                "import { value } from './dependency.js'; export { value };",
                "pkg/shared.js",
            );
        }));
        assert!(panic.is_err());
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
    fn loader_reentry_is_rejected_without_leaving_a_cached_module_record() {
        let runtime = Runtime::new();
        let rejected = Rc::new(Cell::new(false));
        let loader_context = Rc::new(RefCell::new(runtime.new_context()));
        let _loader_registration = runtime.set_module_loader(ReentrantModuleLoader {
            context: loader_context.clone(),
            attempted: Cell::new(false),
            rejected: rejected.clone(),
        });
        let mut context = runtime.new_context();
        let module = context
            .compile_module_with_filename(
                "import { answer } from './dependency.js'; globalThis.__reentryAnswer = answer;",
                "pkg/entry.js",
            )
            .unwrap();
        context.execute_module(&module).unwrap();
        assert!(rejected.get());
        assert_script_true(&mut context, "__reentryAnswer === 42");

        // The rejected nested compilation must not become the oldest record
        // for its name in the loader Context cache.
        loader_context
            .borrow_mut()
            .compile_module_with_filename("export const stale = 42;", "pkg/reentrant.js")
            .unwrap();
        let verification = loader_context
            .borrow_mut()
            .compile_module_with_filename(
                "import { stale } from './reentrant.js'; globalThis.__reentryRecovered = stale;",
                "pkg/verification.js",
            )
            .unwrap();
        loader_context
            .borrow_mut()
            .execute_module(&verification)
            .unwrap();
        assert_script_true(
            &mut loader_context.borrow_mut(),
            "__reentryRecovered === 42",
        );
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
        assert_eq!(context.execute_module(&entry).unwrap(), Value::Undefined);
        assert_script_true(
            &mut context,
            r#"
            __aSeen === 7 && __aRead === 1 && __bRuns === 1 &&
            __before === 1 && __after === 42 && __afterViaCycle === 42
            "#,
        );
        assert_eq!(context.execute_module(&entry).unwrap(), Value::Undefined);
        assert_script_true(&mut context, "__bRuns === 1");
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
                assert_eq!(member.record.link_status.get(), ModuleLinkStatus::Unlinked);
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
        for _ in 0..2 {
            assert_eq!(
                context.execute_module(&module),
                Err(RuntimeError::Exception)
            );
            assert!(matches!(
                context.take_exception().unwrap(),
                Some(Value::Object(_))
            ));
        }
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

        for _ in 0..2 {
            assert_eq!(
                context.execute_module(&module),
                Err(RuntimeError::Exception)
            );
            assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
        }
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

        for _ in 0..2 {
            assert_eq!(
                context.execute_module(&module),
                Err(RuntimeError::Exception)
            );
            assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
            for member in [&module, &a, &b] {
                assert!(matches!(
                    &*member.record.evaluation.borrow(),
                    ModuleEvaluationState::Errored(Value::Int(42))
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

        assert_eq!(context.execute_module(&module).unwrap(), Value::Undefined);
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
        assert_eq!(
            context.execute_module(&abrupt),
            Err(RuntimeError::Exception)
        );
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
        assert_eq!(
            context.execute_module(&abrupt),
            Err(RuntimeError::Exception)
        );
        assert_eq!(context.take_exception().unwrap(), Some(Value::Int(42)));
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

        assert_eq!(
            first_execute_context.execute_module(&module).unwrap(),
            Value::Undefined
        );
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

        assert_eq!(
            later_context.execute_module(&module).unwrap(),
            Value::Undefined
        );
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
    fn module_handle_roots_compilation_and_first_link_realms() {
        let runtime = Runtime::new();
        let module = {
            let mut context = runtime.new_context();
            context
                .compile_module("globalThis.__rootedModuleRealm = 42")
                .unwrap()
        };
        assert_eq!(runtime.heap_counts().context_nodes, 1);

        {
            let mut link_context = runtime.new_context();
            assert_eq!(runtime.heap_counts().context_nodes, 2);
            assert_eq!(
                link_context.execute_module(&module).unwrap(),
                Value::Undefined
            );
            assert_script_true(&mut link_context, "__rootedModuleRealm === 42");
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

        assert_eq!(
            context.execute_module(&module),
            Err(RuntimeError::Exception)
        );
        let Value::Object(error) = context.take_exception().unwrap().unwrap() else {
            panic!("module throw did not produce an Error object");
        };
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
