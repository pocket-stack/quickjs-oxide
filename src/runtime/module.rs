//! Publication and execution for static ECMAScript modules.
//!
//! QuickJS publishes a `JSModuleDef` separately from the bytecode function it
//! drives.  This first vertical slice keeps that ownership boundary even
//! though graph loading is deliberately fail-closed: a module without import
//! or re-export edges can be linked and evaluated, while dependency-bearing
//! records wait for the loader/cache milestone.

use super::*;
use crate::compiler::{CompileOptions, compile_unlinked_module_with_filename};
use crate::module::{
    ModuleExport, ModuleExportTarget, ModuleImport, ModuleLinkInitializer, ModuleRequest,
    ModuleStarExport, UnlinkedModule,
};

/// Opaque owning handle for one runtime-published ECMAScript module record.
///
/// Clones preserve module identity and therefore share link/evaluation state.
/// The contained bytecode remains rooted for as long as any handle survives.
#[derive(Clone)]
pub struct ModuleBytecodeRef(Rc<ModuleRecord>);

struct ModuleRecord {
    name: JsString,
    function: FunctionBytecodeRef,
    // Retain the complete published record so the later graph linker can
    // extend this identity without changing the public compile boundary.
    _link_initializers: Box<[ModuleLinkInitializer]>,
    _requested_modules: Box<[ModuleRequest]>,
    _imports: Box<[ModuleImport]>,
    _exports: Box<[ModuleExport]>,
    _star_exports: Box<[ModuleStarExport]>,
    state: RefCell<ModuleExecutionState>,
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

enum ModuleExecutionState {
    Unlinked,
    Linking,
    Linked(CallableRef),
    Evaluating,
    Evaluated,
    Errored(Value),
    Poisoned,
}

enum ModuleCompilation {
    Published(ModuleBytecodeRef),
    Throw(Value),
}

impl ModuleBytecodeRef {
    /// Return the source/debug name attached to this module record.
    #[must_use]
    pub fn name(&self) -> &JsString {
        &self.0.name
    }

    /// Return whether this module was published by `runtime`.
    #[must_use]
    pub fn belongs_to(&self, runtime: &Runtime) -> bool {
        self.0.function.belongs_to(runtime)
    }

    /// Return whether two handles name modules in the same runtime domain.
    #[must_use]
    pub fn is_same_runtime(&self, other: &Self) -> bool {
        self.0.function.is_same_runtime(&other.0.function)
    }

    /// Stable identity of the runtime domain which published this module.
    #[must_use]
    pub fn domain_id(&self) -> u64 {
        self.0.function.domain_id()
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
    /// Compile and publish a static module without touching the runtime's
    /// pending-exception slot. The public Context boundary installs a thrown
    /// syntax exception exactly as the Script compilation path does.
    fn compile_module_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
        preserve_unsupported_diagnostics: bool,
    ) -> Result<ModuleCompilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let module = match compile_unlinked_module_with_filename(source, filename, debug_info) {
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
                            filename: JsString::try_from_utf8(filename)?,
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
        Ok(ModuleCompilation::Published(
            self.publish_unlinked_module(realm, module)?,
        ))
    }

    pub(super) fn publish_unlinked_module(
        &self,
        realm: ContextId,
        module: UnlinkedModule,
    ) -> Result<ModuleBytecodeRef, RuntimeError> {
        bytecode_publish::verify_unlinked_module_tree(&module)?;

        // This milestone intentionally has no host loader. Reject the whole
        // graph shape before publishing bytecode or allocating module cells so
        // an unsupported dependency cannot leave partial runtime state.
        let has_indirect_export = module
            .exports()
            .iter()
            .any(|export| matches!(export.target, ModuleExportTarget::Indirect { .. }));
        if !module.requested_modules().is_empty()
            || !module.imports().is_empty()
            || !module.star_exports().is_empty()
            || has_indirect_export
        {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Unsupported,
                "module dependency loading is not implemented",
            )));
        }

        let realm_root = ModuleRealmRoot::retain(self, realm)?;
        let parts = module.into_parts();
        let function = self.publish_verified_unlinked_function(realm, parts.function)?;
        Ok(ModuleBytecodeRef(Rc::new(ModuleRecord {
            name: parts.name,
            function,
            _link_initializers: parts.link_initializers,
            _requested_modules: parts.requested_modules,
            _imports: parts.imports,
            _exports: parts.exports,
            _star_exports: parts.star_exports,
            state: RefCell::new(ModuleExecutionState::Unlinked),
            link_realm_root: RefCell::new(None),
            _realm_root: realm_root,
        })))
    }

    fn instantiate_module_callable(
        &self,
        module: &ModuleRecord,
        link_realm: ContextId,
    ) -> Result<CallableRef, RuntimeError> {
        if !module.function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("module bytecode"));
        }
        let descriptors = {
            let state = self.0.state.borrow();
            state
                .heap
                .function_bytecode(module.function.bytecode_id())?
                .closure_variables
                .clone()
        };

        // Authenticate the complete shape before allocating any module cell.
        for descriptor in descriptors.iter().copied() {
            if !matches!(descriptor.name, ClosureVariableName::Atom(_)) {
                return Err(RuntimeError::Invariant(
                    "published module closure descriptor has no atom",
                ));
            }
            match descriptor.source {
                ClosureSource::ModuleDeclaration => {
                    if descriptor.kind != ClosureVariableKind::Normal
                        || (descriptor.is_const && !descriptor.is_lexical)
                    {
                        return Err(RuntimeError::Invariant(
                            "published module declaration has invalid binding metadata",
                        ));
                    }
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
                }
                ClosureSource::ModuleImport => {
                    return Err(RuntimeError::Invariant(
                        "dependency-free module retained an import binding",
                    ));
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
            }
        }

        let mut slots = Vec::with_capacity(descriptors.len());
        for descriptor in descriptors.iter().copied() {
            let ClosureVariableName::Atom(name) = descriptor.name else {
                unreachable!("module descriptor names were preflighted");
            };
            let slot = match descriptor.source {
                ClosureSource::ModuleDeclaration => self.new_uninitialized_captured_var_ref(
                    descriptor.is_lexical,
                    descriptor.is_const,
                    descriptor.kind,
                )?,
                ClosureSource::Global => self.resolve_global_var(link_realm, name)?,
                ClosureSource::ModuleImport
                | ClosureSource::ParentLocal(_)
                | ClosureSource::ParentArgument(_)
                | ClosureSource::ParentClosure(_)
                | ClosureSource::GlobalDeclaration
                | ClosureSource::ParentGlobal(_)
                | ClosureSource::EvalEnvironment(_) => {
                    unreachable!("module descriptor sources were preflighted")
                }
            };
            slots.push(slot);
        }
        self.new_bytecode_closure_with_slots(link_realm, &module.function, &slots)
    }

    fn cache_module_exception(
        &self,
        module: &ModuleRecord,
        exception: Value,
    ) -> Result<Value, RuntimeError> {
        *module.state.borrow_mut() = ModuleExecutionState::Errored(exception.clone());
        self.set_pending_exception(exception)?;
        Err(RuntimeError::Exception)
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

        let linked = match &*module.0.state.borrow() {
            ModuleExecutionState::Evaluated => return Ok(Value::Undefined),
            ModuleExecutionState::Errored(exception) => {
                let exception = exception.clone();
                self.set_pending_exception(exception)?;
                return Err(RuntimeError::Exception);
            }
            ModuleExecutionState::Linked(callable) => {
                let link_realm = module
                    .0
                    .link_realm_root
                    .borrow()
                    .as_ref()
                    .map(|root| root.realm)
                    .ok_or(RuntimeError::Invariant(
                        "linked module has no retained link realm",
                    ))?;
                Some((callable.clone(), link_realm))
            }
            ModuleExecutionState::Unlinked => None,
            ModuleExecutionState::Linking | ModuleExecutionState::Evaluating => {
                return Err(RuntimeError::Invariant(
                    "dependency-free module execution re-entered itself",
                ));
            }
            ModuleExecutionState::Poisoned => {
                return Err(RuntimeError::Invariant(
                    "module execution previously failed inside the engine",
                ));
            }
        };

        let (callable, link_realm) = if let Some((callable, link_realm)) = linked {
            (callable, link_realm)
        } else {
            if module.0.link_realm_root.borrow().is_some() {
                return Err(RuntimeError::Invariant(
                    "unlinked module retained a link realm",
                ));
            }
            let callable = self.instantiate_module_callable(&module.0, initiating_realm)?;
            let link_realm_root = ModuleRealmRoot::retain(self, initiating_realm)?;
            *module.0.link_realm_root.borrow_mut() = Some(link_realm_root);
            *module.0.state.borrow_mut() = ModuleExecutionState::Linking;
            let completion =
                match self.call_internal(initiating_realm, &callable, Value::Bool(true), &[]) {
                    Ok(completion) => completion,
                    Err(error) => {
                        *module.0.state.borrow_mut() = ModuleExecutionState::Poisoned;
                        return Err(error);
                    }
                };
            match completion {
                Completion::Return(Value::Undefined) => {}
                Completion::Return(_) => {
                    *module.0.state.borrow_mut() = ModuleExecutionState::Poisoned;
                    return Err(RuntimeError::Invariant(
                        "module link entry returned a non-undefined value",
                    ));
                }
                Completion::Throw(exception) => {
                    return self.cache_module_exception(&module.0, exception);
                }
            }
            *module.0.state.borrow_mut() = ModuleExecutionState::Linked(callable.clone());
            (callable, initiating_realm)
        };

        *module.0.state.borrow_mut() = ModuleExecutionState::Evaluating;
        let completion = match self.call_internal(link_realm, &callable, Value::Undefined, &[]) {
            Ok(completion) => completion,
            Err(error) => {
                *module.0.state.borrow_mut() = ModuleExecutionState::Poisoned;
                return Err(error);
            }
        };
        match completion {
            Completion::Return(Value::Undefined) => {
                *module.0.state.borrow_mut() = ModuleExecutionState::Evaluated;
                Ok(Value::Undefined)
            }
            Completion::Return(_) => {
                *module.0.state.borrow_mut() = ModuleExecutionState::Poisoned;
                Err(RuntimeError::Invariant(
                    "module evaluation returned a non-undefined value",
                ))
            }
            Completion::Throw(exception) => self.cache_module_exception(&module.0, exception),
        }
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
    /// milestone accepts dependency-free synchronous modules only.
    pub fn execute_module(&mut self, module: &ModuleBytecodeRef) -> Result<Value, RuntimeError> {
        self.runtime.execute_module(self.realm, module)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_script_true(context: &mut Context, source: &str) {
        assert_eq!(context.eval(source).unwrap(), Value::Bool(true));
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
