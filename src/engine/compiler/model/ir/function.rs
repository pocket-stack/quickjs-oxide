//! Function artifacts shared by completed parsing, resolution and lowering.

use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
use crate::engine::code::function::metadata::ClassInitializerKind;
use crate::engine::code::function::metadata::ClosureVariable;
use crate::engine::code::function::metadata::EvalCallerProfile;
use crate::engine::code::function::metadata::EvalCallerVariableTarget;
use crate::engine::code::function::metadata::EvalEnvironment;
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::code::function::metadata::EvalRootBinding;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::code::function::metadata::ParameterDefaultSource;
use crate::engine::compiler::EVAL_RET_LOCAL_NAME;
use crate::engine::compiler::MAX_LOCAL_VARIABLES;
use crate::engine::compiler::lexer::Span;
use crate::engine::compiler::model::bindings::BindingId;
use crate::engine::compiler::model::bindings::BindingKind;
use crate::engine::compiler::model::bindings::BindingStorage;
use crate::engine::compiler::model::bindings::IrBinding;
use crate::engine::compiler::model::bindings::IrEvalDeclaration;
use crate::engine::compiler::model::bindings::IrGlobalDeclaration;
use crate::engine::compiler::model::bindings::IrHoistedFunction;
use crate::engine::compiler::model::bindings::IrProgramAnnexFunction;
use crate::engine::compiler::model::bindings::IrScopedFunction;
use crate::engine::compiler::model::bindings::ResolvedBinding;
use crate::engine::compiler::model::bindings::SyntheticLocal;
use crate::engine::compiler::model::bindings::SyntheticLocalKind;
use crate::engine::compiler::model::ir::FunctionId;
use crate::engine::compiler::model::ir::IrConstant;
use crate::engine::compiler::model::ir::IrOp;
use crate::engine::compiler::model::ir::SpannedIrOp;
use crate::engine::compiler::model::scope::IrScope;
use crate::engine::compiler::model::scope::ScopeId;
use crate::engine::compiler::model::scope::ScopeKind;
use crate::engine::compiler::module;
use crate::engine::compiler::pseudo_binding::ACTIVE_FUNCTION_LOCAL_NAME;
use crate::engine::compiler::pseudo_binding::THIS_LOCAL_NAME;
use crate::engine::value::JsString;
use crate::engine::value::PrimitiveValue as Value;
use crate::source::SourceOffset;
use crate::source::text::SourceText;
use std::collections::HashMap;
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) struct ParentLink {
    pub(in crate::engine::compiler) function: FunctionId,
    pub(in crate::engine::compiler) definition_scope: ScopeId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::engine::compiler) enum FunctionKind {
    Script,
    Module,
    Eval(EvalKind),
    Ordinary,
    /// Compiler-only object-literal concise method. Like an ordinary function
    /// it owns `this`, `arguments`, and `new.target`, but publication lowers it
    /// as a non-constructor with no `prototype` property.
    Method,
    /// Compiler-only parse/binding kind. QuickJS publishes synchronous arrow
    /// bytecode as a normal function with no prototype or constructor bit.
    Arrow,
}

#[derive(Debug)]
pub(in crate::engine::compiler) struct IrParameterPatternBinding {
    pub(in crate::engine::compiler) name: String,
    pub(in crate::engine::compiler) parameter_local: u16,
    pub(in crate::engine::compiler) body_local: Option<u16>,
    pub(in crate::engine::compiler) declaration_span: Span,
}

#[derive(Debug)]
pub(in crate::engine::compiler) struct FunctionIr {
    /// Completed body boundary, retained for suspension metadata validation.
    pub(in crate::engine::compiler) body_parsed: bool,
    /// Parent function plus the scope which was current at this function's
    /// definition. This is QuickJS `parent` + `parent_scope_level` as one
    /// invariant-preserving typed link.
    pub(in crate::engine::compiler) parent: Option<ParentLink>,
    pub(in crate::engine::compiler) kind: FunctionKind,
    /// Callable execution semantics are independent from the grammar role.
    /// A class or object generator remains a concise `Method` for bindings
    /// and HomeObject purposes while publishing generator bytecode.
    pub(in crate::engine::compiler) execution_kind: BytecodeFunctionKind,
    /// Base/derived class constructors share the compiler's concise-method
    /// binding model but publish constructor bytecode without an ordinary
    /// function's eagerly visible `.prototype` shape. `DefineClass` owns that
    /// descriptor, while `CheckCtor` enforces construct-only invocation.
    pub(in crate::engine::compiler) class_constructor: bool,
    /// Whether this class constructor uses the derived [[Construct]]
    /// protocol. Keeping this separate from ordinary constructability lets
    /// arrows and direct eval inherit `super()` authority without pretending
    /// to be constructors themselves.
    pub(in crate::engine::compiler) derived_class_constructor: bool,
    /// Synthetic QuickJS class-element program role. This is assigned only by
    /// class lowering after the ordinary method-shaped FunctionIr is created.
    pub(in crate::engine::compiler) class_initializer_kind: Option<ClassInitializerKind>,
    /// Whether this authenticated instance/static aggregate installs the
    /// private-method brand for its class side before executing element code.
    pub(in crate::engine::compiler) class_private_brand: bool,
    /// QuickJS parser authority copied independently from HomeObject storage.
    pub(in crate::engine::compiler) super_call_allowed: bool,
    pub(in crate::engine::compiler) super_allowed: bool,
    pub(in crate::engine::compiler) arguments_forbidden: bool,
    pub(in crate::engine::compiler) source: FunctionSourceInfo,
    /// Intrinsic function name, independent of contextual `SetName` inference
    /// for anonymous definitions.
    pub(in crate::engine::compiler) function_name: Option<String>,
    /// Whether a named expression may lazily create QuickJS's private
    /// `JS_VAR_FUNCTION_NAME` self binding. Declarations carry an intrinsic
    /// name but resolve recursion through their authored environment.
    pub(in crate::engine::compiler) private_name_binding: bool,
    /// Lazily allocated private self-binding local.
    pub(in crate::engine::compiler) function_name_local: Option<u16>,
    /// Root local initialized by the typed arguments-object entry prologue.
    ///
    /// Like QuickJS's `arguments_var_idx`, this is selected only when source
    /// resolution (or a function-scoped `var`/function declaration) needs the
    /// implicit binding. A named physical `arguments` parameter suppresses it;
    /// a BindingPattern BoundName does not, because QuickJS reserves an
    /// anonymous argument slot and initializes the arguments object first.
    pub(in crate::engine::compiler) arguments_local: Option<u16>,
    /// Lazily materialized QuickJS pseudo variables captured by descendant
    /// arrows or exposed to direct eval. Arrow frames never own these locals;
    /// only concise methods can own the HomeObject cell.
    pub(in crate::engine::compiler) home_object_local: Option<u16>,
    /// QuickJS's hidden `this_active_func`, captured by arrows/direct eval so
    /// `super()` dynamically reads the active constructor's [[Prototype]].
    pub(in crate::engine::compiler) active_function_local: Option<u16>,
    pub(in crate::engine::compiler) this_local: Option<u16>,
    pub(in crate::engine::compiler) new_target_local: Option<u16>,
    /// Hidden null-prototype variable object for sloppy authored function code
    /// containing syntactic direct eval. Its identity is explicit rather than
    /// inferred from local allocation order.
    pub(in crate::engine::compiler) eval_variable_object_local: Option<u16>,
    /// Hidden `<arg_var>` object used by sloppy direct eval in a parentless
    /// Parameter Environment. Unlike authored parameter cells, this slot is
    /// rooted for the full activation and is projected into both parameter
    /// and body eval descriptors.
    pub(in crate::engine::compiler) arg_eval_variable_object_local: Option<u16>,
    /// Sloppy Parameter Environment alias of the ordinary function's
    /// unmapped arguments object. QuickJS skips this cell's ordinary TDZ reset
    /// and initializes it together with the body arguments binding.
    pub(in crate::engine::compiler) synthetic_parameter_arguments_local: Option<u16>,
    /// A lazily allocated HomeObject pseudo local requires the published
    /// method function to retain its object literal as HomeObject. Descendant
    /// arrows relay the local without carrying this metadata themselves.
    pub(in crate::engine::compiler) needs_home_object: bool,
    /// Physical call-frame argument slots. Destructuring parameters use an
    /// unnamed slot, matching QuickJS's `JS_ATOM_NULL` argument descriptor;
    /// their individual BoundNames live in root locals instead.
    pub(in crate::engine::compiler) parameters: Vec<Option<String>>,
    /// Every authored BoundName in formal-list order, including leaves of a
    /// BindingPattern. This is the authority for duplicate-parameter policy,
    /// `arguments` shadowing, and Annex B parameter-name checks.
    pub(in crate::engine::compiler) parameter_names: Vec<String>,
    /// QuickJS `defined_arg_count`, exposed as the function's public `length`.
    /// An identifier rest parameter owns a physical argument slot but is not
    /// included in this count.
    pub(in crate::engine::compiler) defined_argument_count: usize,
    /// QuickJS `has_simple_parameter_list`. Besides early-error policy, this
    /// selects mapped versus unmapped `arguments` for sloppy functions.
    pub(in crate::engine::compiler) has_simple_parameter_list: bool,
    /// Physical argument slot overwritten by the entry-time `OP_rest` result.
    pub(in crate::engine::compiler) rest_parameter: Option<u16>,
    /// First actual argument collected for a terminal `...BindingPattern`.
    /// Unlike an identifier rest parameter this does not reserve a physical
    /// frame slot; the fresh Array is consumed directly by destructuring.
    pub(in crate::engine::compiler) rest_pattern_start: Option<u16>,
    /// Independent declarative scope used by identifier default parameters.
    /// QuickJS calls this its argument scope; keeping the identity explicit
    /// lets resolution enforce the body-variable visibility barrier.
    pub(in crate::engine::compiler) parameter_scope: Option<ScopeId>,
    /// Every initializer-visible mutable cell owned by `parameter_scope`, in
    /// FormalParameters BoundName order. Identifier formals contribute one
    /// cell while a BindingPattern contributes one cell per leaf.
    pub(in crate::engine::compiler) parameter_locals: Vec<u16>,
    /// Exact whole-list pre-scan reservation for authored parameter cells.
    /// Reserving this leading local prefix before parsing any initializer
    /// prevents nested class/function compilation from interleaving scratch
    /// locals with the heap-visible Parameter Environment ABI.
    pub(in crate::engine::compiler) parameter_local_reservation_count: Option<usize>,
    /// Parameter-scope cell selected by each physical named argument. An
    /// anonymous BindingPattern slot has no direct cell because destructuring
    /// initializes its individual BoundNames instead.
    pub(in crate::engine::compiler) parameter_argument_locals: Vec<Option<u16>>,
    /// Parameter-scope BindingPattern leaves which must be copied into fresh
    /// FunctionRoot variables after every parameter expression has run.
    pub(in crate::engine::compiler) parameter_pattern_bindings: Vec<IrParameterPatternBinding>,
    /// Top-level formal initializers in source order. Pattern-leaf defaults
    /// create the argument scope but do not cut Function.length, so they are
    /// intentionally absent from this list.
    pub(in crate::engine::compiler) parameter_default_sources: Vec<ParameterDefaultSource>,
    /// At least one BindingPattern is initialized before the authored body.
    /// Without a Parameter Environment it runs in FunctionRoot; with one it
    /// runs in the parentless parameter scope and is copied out at the end.
    pub(in crate::engine::compiler) pattern_parameter_initialization: bool,
    pub(in crate::engine::compiler) locals: Vec<String>,
    pub(in crate::engine::compiler) scopes: Vec<IrScope>,
    pub(in crate::engine::compiler) bindings: Vec<IrBinding>,
    pub(in crate::engine::compiler) global_declarations: Vec<IrGlobalDeclaration>,
    /// Last direct function declaration attached to each ordinary
    /// function-scoped argument/local binding.
    pub(in crate::engine::compiler) hoisted_functions: Vec<IrHoistedFunction>,
    /// Source-ordered declaration records for sloppy direct eval targeting a
    /// caller function's variable environment.
    pub(in crate::engine::compiler) eval_declarations: Vec<IrEvalDeclaration>,
    pub(in crate::engine::compiler) eval_declarations_installed: bool,
    /// First caller lexical name which conflicts with an eval `var`/function.
    /// The eval still compiles so global declaration instantiation can run
    /// before this typed SyntaxError is thrown at bytecode entry.
    pub(in crate::engine::compiler) eval_redeclaration: Option<String>,
    pub(in crate::engine::compiler) function_hoists_installed: bool,
    /// Phase marker for the final hidden-frame entry prefix. Unlike ordinary
    /// body hoists this also applies to scripts and eval roots, so it cannot
    /// be inferred from `function_hoists_installed`.
    pub(in crate::engine::compiler) pseudo_binding_prologues_installed: bool,
    /// Scoped lexical function slots, including one slot per sloppy same-scope
    /// duplicate as in QuickJS `JS_VAR_FUNCTION_DECL`.
    pub(in crate::engine::compiler) scoped_functions: Vec<IrScopedFunction>,
    /// ProgramBody's labelled-function exception has authored closure writes
    /// but no lexical scope-entry slot.
    pub(in crate::engine::compiler) program_annex_functions: Vec<IrProgramAnnexFunction>,
    pub(in crate::engine::compiler) var_scope: ScopeId,
    pub(in crate::engine::compiler) body_scope: ScopeId,
    /// QuickJS `eval_ret_idx`: the script-only hidden completion local.
    /// Keeping the typed slot separate from its unspellable debug name avoids
    /// confusing it with future source bindings or other synthetic locals.
    pub(in crate::engine::compiler) eval_ret_local: Option<u16>,
    /// Every local which deliberately has no source binding identity. This is
    /// validated separately from authored locals before publication.
    pub(in crate::engine::compiler) synthetic_locals: Vec<SyntheticLocal>,
    pub(in crate::engine::compiler) ops: Vec<SpannedIrOp>,
    pub(in crate::engine::compiler) constants: Vec<IrConstant>,
    /// First primitive string occurrence; constant ordinals remain append-only.
    pub(in crate::engine::compiler) string_constants: HashMap<JsString, u32>,
    pub(in crate::engine::compiler) closure_variables: Vec<ClosureVariable>,
    /// Exact flattened caller bindings imported by a synthetic direct-eval
    /// root. Entries retain their original R1w descriptor indices even though
    /// bindings are inserted into the synthetic root in outer-to-inner order
    /// so ordinary reverse lookup selects the innermost duplicate name.
    pub(in crate::engine::compiler) external_bindings: Vec<EvalRootBinding<JsString>>,
    /// Exact imported caller scope topology and variable target.  The root's
    /// flat external binding vector remains the closure-prefix ABI, while this
    /// profile reconstructs the original ordered suffix for nested eval.
    pub(in crate::engine::compiler) eval_caller_profile: EvalCallerProfile,
    /// Immutable QuickJS-shaped scope chains linked for syntactic direct-eval
    /// call sites. Multiple calls from the same parser scope share one entry.
    pub(in crate::engine::compiler) eval_environments: Vec<EvalEnvironment<JsString>>,
    pub(in crate::engine::compiler) strict: bool,
}

#[derive(Clone, Debug)]
pub(in crate::engine::compiler) struct FunctionSourceInfo {
    pub(in crate::engine::compiler) span: Span,
    pub(in crate::engine::compiler) definition: SourceOffset,
    pub(in crate::engine::compiler) range: Option<Range<SourceOffset>>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in crate::engine::compiler) struct SuperCapabilities {
    pub(in crate::engine::compiler) super_call_allowed: bool,
    pub(in crate::engine::compiler) super_allowed: bool,
}

impl SuperCapabilities {
    pub(in crate::engine::compiler) const NONE: Self = Self {
        super_call_allowed: false,
        super_allowed: false,
    };
    pub(in crate::engine::compiler) const PROPERTY: Self = Self {
        super_call_allowed: false,
        super_allowed: true,
    };
    pub(in crate::engine::compiler) const CALL_AND_PROPERTY: Self = Self {
        super_call_allowed: true,
        super_allowed: true,
    };

    pub(in crate::engine::compiler) fn validated(self) -> Result<Self, Error> {
        if self.super_call_allowed && !self.super_allowed {
            return Err(Error::internal(
                "function permits super() without SuperProperty",
            ));
        }
        Ok(self)
    }
}

#[derive(Clone, Debug)]
pub(in crate::engine::compiler) struct FunctionIrOptions {
    pub(in crate::engine::compiler) function_name: Option<String>,
    pub(in crate::engine::compiler) private_name_binding: bool,
    pub(in crate::engine::compiler) class_constructor: bool,
    pub(in crate::engine::compiler) derived_class_constructor: bool,
    pub(in crate::engine::compiler) parameters: Vec<Option<String>>,
    pub(in crate::engine::compiler) defined_argument_count: usize,
    pub(in crate::engine::compiler) has_simple_parameter_list: bool,
    pub(in crate::engine::compiler) rest_parameter: Option<u16>,
    pub(in crate::engine::compiler) strict: bool,
    pub(in crate::engine::compiler) super_capabilities: SuperCapabilities,
}

impl FunctionIr {
    pub(in crate::engine::compiler) fn new(
        parent: Option<ParentLink>,
        kind: FunctionKind,
        source: FunctionSourceInfo,
        options: FunctionIrOptions,
    ) -> Result<Self, Error> {
        let super_capabilities = options.super_capabilities.validated()?;
        if options.derived_class_constructor
            && (!options.class_constructor
                || kind != FunctionKind::Method
                || super_capabilities != SuperCapabilities::CALL_AND_PROPERTY)
        {
            return Err(Error::internal("derived constructor metadata is malformed"));
        }
        let parameter_count = options.parameters.len();
        if options.defined_argument_count > options.parameters.len()
            || (options.has_simple_parameter_list
                && (options.defined_argument_count != options.parameters.len()
                    || options.rest_parameter.is_some()
                    || options.parameters.iter().any(Option::is_none)))
            || options.rest_parameter.is_some_and(|rest| {
                usize::from(rest) + 1 != options.parameters.len()
                    || options.defined_argument_count != usize::from(rest)
                    || options.has_simple_parameter_list
                    || options.parameters[usize::from(rest)].is_none()
            })
        {
            return Err(Error::internal("formal parameter metadata is malformed"));
        }
        let (locals, eval_ret_local, synthetic_locals) =
            if matches!(kind, FunctionKind::Script | FunctionKind::Eval(_)) {
                (
                    vec![EVAL_RET_LOCAL_NAME.to_owned()],
                    Some(0),
                    vec![SyntheticLocal {
                        index: 0,
                        kind: SyntheticLocalKind::EvalCompletion,
                    }],
                )
            } else {
                (Vec::new(), None, Vec::new())
            };
        // QuickJS reserves scope zero for arguments/function-scoped storage,
        // then pushes the authored body scope. Named-expression self storage
        // is a lazy local in the root, not a synthetic lexical parent scope.
        let function_root = ScopeId(0);
        let body = ScopeId(1);
        let scopes = vec![
            IrScope {
                parent: None,
                kind: ScopeKind::FunctionRoot,
                is_parameter_initializer: false,
                bindings: Vec::new(),
                bindings_by_name: Default::default(),
            },
            IrScope {
                parent: Some(function_root),
                kind: if matches!(
                    kind,
                    FunctionKind::Script | FunctionKind::Module | FunctionKind::Eval(_)
                ) {
                    ScopeKind::ProgramBody
                } else {
                    ScopeKind::FunctionBody
                },
                is_parameter_initializer: false,
                bindings: Vec::new(),
                bindings_by_name: Default::default(),
            },
        ];
        let var_scope = function_root;
        let ops = if matches!(
            kind,
            FunctionKind::Ordinary
                | FunctionKind::Method
                | FunctionKind::Arrow
                | FunctionKind::Eval(_)
        ) {
            vec![SpannedIrOp {
                op: IrOp::EnterScope(body),
                pc_site: None,
            }]
        } else {
            Vec::new()
        };
        let mut function = Self {
            body_parsed: false,
            parent,
            kind,
            execution_kind: BytecodeFunctionKind::Normal,
            class_constructor: options.class_constructor,
            derived_class_constructor: options.derived_class_constructor,
            class_initializer_kind: None,
            class_private_brand: false,
            super_call_allowed: super_capabilities.super_call_allowed,
            super_allowed: super_capabilities.super_allowed,
            arguments_forbidden: false,
            source,
            function_name: options.function_name,
            private_name_binding: options.private_name_binding,
            function_name_local: None,
            arguments_local: None,
            home_object_local: None,
            active_function_local: None,
            this_local: None,
            new_target_local: None,
            eval_variable_object_local: None,
            arg_eval_variable_object_local: None,
            synthetic_parameter_arguments_local: None,
            needs_home_object: false,
            parameters: options.parameters,
            parameter_names: Vec::new(),
            defined_argument_count: options.defined_argument_count,
            has_simple_parameter_list: options.has_simple_parameter_list,
            rest_parameter: options.rest_parameter,
            rest_pattern_start: None,
            parameter_scope: None,
            parameter_locals: Vec::new(),
            parameter_local_reservation_count: None,
            parameter_argument_locals: vec![None; parameter_count],
            parameter_pattern_bindings: Vec::new(),
            parameter_default_sources: Vec::new(),
            pattern_parameter_initialization: false,
            locals,
            scopes,
            bindings: Vec::new(),
            global_declarations: Vec::new(),
            hoisted_functions: Vec::new(),
            eval_declarations: Vec::new(),
            eval_declarations_installed: false,
            eval_redeclaration: None,
            function_hoists_installed: false,
            pseudo_binding_prologues_installed: false,
            scoped_functions: Vec::new(),
            program_annex_functions: Vec::new(),
            var_scope,
            body_scope: body,
            eval_ret_local,
            synthetic_locals,
            ops,
            constants: Vec::new(),
            string_constants: HashMap::new(),
            closure_variables: Vec::new(),
            external_bindings: Vec::new(),
            eval_caller_profile: EvalCallerProfile {
                scope_kinds: Box::new([]),
                variable_target: EvalCallerVariableTarget::Global,
            },
            eval_environments: Vec::new(),
            strict: options.strict,
        };
        for (index, name) in function.parameters.clone().into_iter().enumerate() {
            let Some(name) = name else {
                continue;
            };
            let index = u16::try_from(index)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
            function.parameter_names.push(name.clone());
            function.add_binding(
                function.var_scope,
                function.var_scope,
                name,
                BindingStorage::Argument(index),
                BindingKind::Normal,
                None,
            );
        }
        Ok(function)
    }

    /// Allocate the derived constructor's hidden cells after formal parsing.
    /// Parameter-environment cells must remain the leading locals, but
    /// unresolved `super()`/`this` operations in parameter initializers do not
    /// need physical operands until the later identifier-linking pass.
    pub(in crate::engine::compiler) fn allocate_derived_constructor_pseudo_bindings(
        &mut self,
    ) -> Result<(), Error> {
        if !self.derived_class_constructor
            || !self.class_constructor
            || self.kind != FunctionKind::Method
            || self.active_function_local.is_some()
            || self.this_local.is_some()
        {
            return Err(Error::internal(
                "derived constructor pseudo bindings were allocated in an invalid phase",
            ));
        }
        if self.locals.len().saturating_add(2) > MAX_LOCAL_VARIABLES {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "too many local variables",
            ));
        }
        let active_function = u16::try_from(self.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        self.locals.push(ACTIVE_FUNCTION_LOCAL_NAME.to_owned());
        self.active_function_local = Some(active_function);
        self.add_binding(
            self.var_scope,
            self.var_scope,
            ACTIVE_FUNCTION_LOCAL_NAME.to_owned(),
            BindingStorage::Local(active_function),
            BindingKind::Normal,
            None,
        );

        let this = u16::try_from(self.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        self.locals.push(THIS_LOCAL_NAME.to_owned());
        self.this_local = Some(this);
        self.add_binding(
            self.var_scope,
            self.var_scope,
            THIS_LOCAL_NAME.to_owned(),
            BindingStorage::Local(this),
            BindingKind::Lexical { is_const: false },
            None,
        );
        Ok(())
    }

    /// Preserve every authored constant and its ordinal. Only name-constant
    /// reuse consults the derived first-occurrence index.
    pub(in crate::engine::compiler) fn append_constant(
        &mut self,
        constant: IrConstant,
    ) -> Result<u32, Error> {
        let index = u32::try_from(self.constants.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "out of memory"))?;
        if let IrConstant::Primitive(Value::String(value)) = &constant {
            self.string_constants.entry(value.clone()).or_insert(index);
        }
        self.constants.push(constant);
        Ok(index)
    }

    pub(in crate::engine::compiler) fn add_binding(
        &mut self,
        storage_scope: ScopeId,
        declaration_scope: ScopeId,
        name: String,
        storage: BindingStorage,
        kind: BindingKind,
        declaration_span: Option<Span>,
    ) -> BindingId {
        let binding = BindingId(self.bindings.len());
        self.bindings.push(IrBinding {
            name,
            storage_scope,
            declaration_scope,
            storage,
            kind,
            is_scoped_function: false,
            is_scoped_generator: false,
            is_catch_parameter: false,
            declaration_span,
        });
        self.scopes[storage_scope.0].bindings.push(binding);
        self.scopes[storage_scope.0]
            .bindings_by_name
            .insert(self.bindings[binding.0].name.clone(), binding);
        binding
    }

    pub(in crate::engine::compiler) fn add_synthetic_local(
        &mut self,
        kind: SyntheticLocalKind,
    ) -> Result<u16, Error> {
        if self.locals.len() >= MAX_LOCAL_VARIABLES {
            return Err(Error::new(
                ErrorKind::JsInternal,
                "too many local variables",
            ));
        }
        let index = u16::try_from(self.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?;
        self.locals.push(kind.name().to_owned());
        self.synthetic_locals.push(SyntheticLocal { index, kind });
        Ok(index)
    }

    pub(in crate::engine::compiler) fn binding_in_scope(
        &self,
        scope: ScopeId,
        name: &str,
    ) -> Option<&IrBinding> {
        self.binding_id_in_scope(scope, name)
            .map(|binding| &self.bindings[binding.0])
    }

    pub(in crate::engine::compiler) fn binding_id_in_scope(
        &self,
        scope: ScopeId,
        name: &str,
    ) -> Option<BindingId> {
        self.scopes[scope.0].binding_named(name)
    }

    /// Rare late function-name insertion changes order after normal appends.
    /// Rebuild once there, rather than burdening every name lookup with a scan.
    pub(in crate::engine::compiler) fn rebuild_scope_name_index(&mut self, scope: ScopeId) {
        let scope = &mut self.scopes[scope.0];
        scope.bindings_by_name.clear();
        for &binding in &scope.bindings {
            scope
                .bindings_by_name
                .insert(self.bindings[binding.0].name.clone(), binding);
        }
    }

    pub(in crate::engine::compiler) fn binding_id_from_scope(
        &self,
        mut scope: ScopeId,
        name: &str,
    ) -> Option<(ScopeId, BindingId)> {
        loop {
            if let Some(binding) = self.binding_id_in_scope(scope, name) {
                return Some((scope, binding));
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    pub(in crate::engine::compiler) fn first_global_declaration_is_normal(
        &self,
        name: &str,
    ) -> bool {
        self.global_declarations
            .iter()
            .find(|declaration| declaration.name == name)
            .is_some_and(|declaration| !declaration.is_lexical)
    }

    pub(in crate::engine::compiler) fn binding_from_scope(
        &self,
        mut scope: ScopeId,
        name: &str,
    ) -> Option<ResolvedBinding> {
        loop {
            if let Some(binding) = self.binding_in_scope(scope, name) {
                return Some(ResolvedBinding {
                    storage: binding.storage,
                    kind: binding.kind,
                });
            }
            scope = self.scopes[scope.0].parent?;
        }
    }

    pub(in crate::engine::compiler) fn scope_is_within(
        &self,
        mut scope: ScopeId,
        ancestor: ScopeId,
    ) -> bool {
        loop {
            if scope == ancestor {
                return true;
            }
            let Some(parent) = self.scopes[scope.0].parent else {
                return false;
            };
            scope = parent;
        }
    }
}

#[derive(Debug)]
pub(in crate::engine::compiler) struct FunctionTree {
    pub(in crate::engine::compiler) functions: Vec<FunctionIr>,
    pub(in crate::engine::compiler) source: SourceText,
    pub(in crate::engine::compiler) filename: JsString,
    pub(in crate::engine::compiler) module: Option<module::IrModule>,
    pub(in crate::engine::compiler) pending_unsupported: Option<Error>,
}
