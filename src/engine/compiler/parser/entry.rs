//! Root compilation and consuming parser completion.

use crate::engine::api::error::Error;
use crate::engine::code::function::metadata::EvalCallerProfile;
use crate::engine::code::function::metadata::EvalCallerVariableTarget;
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::code::function::metadata::EvalRootBinding;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::compiler::EvalCompileContext;
use crate::engine::compiler::ModuleCompileFailure;
use crate::engine::compiler::ModuleImportAttributeChecker;

use crate::engine::compiler::lexer::LexContext;
use crate::engine::compiler::lexer::Lexer;
use crate::engine::compiler::lexer::LexerOptions;
use crate::engine::compiler::model::ir::function::FunctionIrOptions;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::ir::function::FunctionSourceInfo;
use crate::engine::compiler::model::ir::function::FunctionTree;
use crate::engine::compiler::model::ir::function::SuperCapabilities;
use crate::engine::compiler::module;
use crate::engine::compiler::parser::builder::FunctionBuilder;
use crate::engine::compiler::parser::context::InMode;
use crate::engine::compiler::parser::context::ModuleDeclarationExport;
use crate::engine::compiler::parser::context::Parser;
use crate::engine::compiler::parser::context::RootCompileContext;
use crate::engine::compiler::parser::diagnostics::lex_error;
use crate::engine::compiler::validate_source_length;
use crate::engine::value::JsString;
use crate::source::SourceOffset;
use crate::source::text::SourceText;

impl<'source> Parser<'source> {
    pub(in crate::engine::compiler) fn parse(
        source: &'source str,
        filename: JsString,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(source, None, filename, RootCompileContext::Script, None)
            .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    pub(in crate::engine::compiler) fn parse_module(
        source: &'source str,
        filename: JsString,
        checker: Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<FunctionTree, ModuleCompileFailure> {
        Self::parse_root(source, None, filename, RootCompileContext::Module, checker)
    }

    pub(in crate::engine::compiler) fn parse_module_source(
        source: &'source SourceText,
        filename: JsString,
        checker: Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<FunctionTree, ModuleCompileFailure> {
        Self::parse_root(
            source.carrier(),
            Some(source),
            filename,
            RootCompileContext::Module,
            checker,
        )
    }

    pub(in crate::engine::compiler) fn parse_script_source(
        source: &'source SourceText,
        filename: JsString,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(
            source.carrier(),
            Some(source),
            filename,
            RootCompileContext::Script,
            None,
        )
        .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::engine::compiler) fn parse_eval(
        source: &'source str,
        filename: JsString,
        context: EvalCompileContext,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(
            source,
            None,
            filename,
            RootCompileContext::Eval(context),
            None,
        )
        .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    pub(in crate::engine::compiler) fn parse_eval_source(
        source: &'source SourceText,
        filename: JsString,
        context: EvalCompileContext,
    ) -> Result<FunctionTree, Error> {
        Self::parse_root(
            source.carrier(),
            Some(source),
            filename,
            RootCompileContext::Eval(context),
            None,
        )
        .map_err(ModuleCompileFailure::into_engine_without_checker)
    }

    pub(in crate::engine::compiler) fn parse_root(
        source: &'source str,
        source_text: Option<&'source SourceText>,
        filename: JsString,
        context: RootCompileContext,
        mut module_attribute_checker: Option<&mut dyn ModuleImportAttributeChecker>,
    ) -> Result<FunctionTree, ModuleCompileFailure> {
        #[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Parse,
        );
        validate_source_length(source.len())?;
        let is_module = matches!(&context, RootCompileContext::Module);
        let (
            root_kind,
            inherited_strict,
            external_bindings,
            caller_profile,
            super_capabilities,
            arguments_forbidden,
        ) = match context {
            RootCompileContext::Script => (
                FunctionKind::Script,
                false,
                Vec::<EvalRootBinding<JsString>>::new().into_boxed_slice(),
                EvalCallerProfile {
                    scope_kinds: Box::new([]),
                    variable_target: EvalCallerVariableTarget::Global,
                },
                SuperCapabilities::NONE,
                false,
            ),
            RootCompileContext::Module => (
                FunctionKind::Module,
                true,
                Vec::<EvalRootBinding<JsString>>::new().into_boxed_slice(),
                EvalCallerProfile {
                    scope_kinds: Box::new([]),
                    variable_target: EvalCallerVariableTarget::StrictLocal,
                },
                SuperCapabilities::NONE,
                false,
            ),
            RootCompileContext::Eval(context) => {
                if !matches!(context.kind, EvalKind::Direct | EvalKind::Indirect) {
                    return Err(
                        Error::internal("eval compiler received a non-eval root kind").into(),
                    );
                }
                if context.kind == EvalKind::Indirect
                    && (!context.bindings.is_empty()
                        || !context.caller_profile.scope_kinds.is_empty()
                        || context.caller_profile.variable_target
                            != EvalCallerVariableTarget::Global
                        || context.super_call_allowed
                        || context.super_allowed
                        || context.arguments_forbidden)
                {
                    return Err(Error::internal(
                        "indirect eval compiler received a caller environment",
                    )
                    .into());
                }
                let super_capabilities = SuperCapabilities {
                    super_call_allowed: context.super_call_allowed,
                    super_allowed: context.super_allowed,
                }
                .validated()
                .map_err(|_| {
                    Error::internal("eval compiler permits super() without SuperProperty")
                })?;
                (
                    FunctionKind::Eval(context.kind),
                    context.kind == EvalKind::Direct && context.caller_strict,
                    context.bindings,
                    context.caller_profile,
                    super_capabilities,
                    context.arguments_forbidden,
                )
            }
        };
        // QuickJS enables Annex B HTML comments for every Script and Eval
        // parse, independently of strict mode. Module parsing keeps them
        // disabled (`allow_html_comments = !is_module`).
        let lexer_options = LexerOptions {
            context: LexContext {
                strict: inherited_strict,
                module: is_module,
                ..LexContext::default()
            },
            allow_html_comments: !is_module,
        };
        let mut lexer = match source_text {
            Some(source) => Lexer::with_source_text(source, lexer_options),
            None => Lexer::with_options(source, lexer_options),
        };
        let first_token = lexer.next_token().map_err(lex_error)?;
        let source_span = first_token.span;
        let mut parser = Self {
            lexer,
            tokens: vec![first_token],
            cursor: 0,
            current_function: 0,
            in_mode: InMode::Allow,
            anonymous_function_definition: None,
            pending_unsupported: None,
            module: is_module.then(module::IrModule::default),
            module_declaration_export: ModuleDeclarationExport::None,
            module_declaration_export_target: None,
            functions: vec![FunctionBuilder::new(
                None,
                root_kind,
                FunctionSourceInfo {
                    span: source_span,
                    definition: SourceOffset::try_from_usize(0)
                        .map_err(|error| Error::internal(error.to_string()))?,
                    range: None,
                },
                FunctionIrOptions {
                    function_name: (!is_module).then(|| "<eval>".to_owned()),
                    private_name_binding: false,
                    class_constructor: false,
                    derived_class_constructor: false,
                    parameters: Vec::new(),
                    defined_argument_count: 0,
                    has_simple_parameter_list: true,
                    rest_parameter: None,
                    strict: inherited_strict,
                    super_capabilities,
                },
            )?],
        };
        if is_module {
            // QuickJS compiles every module root as an async function. The
            // separate module record bit records whether authored evaluation
            // can actually suspend; keeping the callable kind async here lets
            // top-level AwaitExpression and `for await` reuse the ordinary
            // async-function lowering and continuation machinery.
            parser.functions[0].execution_kind = BytecodeFunctionKind::Async;
            parser.functions[0].context.in_function_body = true;
            parser.functions[0].eval_caller_profile = caller_profile.clone();
        }
        if matches!(root_kind, FunctionKind::Eval(_)) {
            install_eval_external_bindings(
                &mut parser.functions[0],
                external_bindings,
                caller_profile,
                inherited_strict,
            )?;
        }
        let strict =
            inherited_strict || parser.directive_prologue_has_use_strict(0, inherited_strict)?;
        parser.relex_current_with_strict(strict)?;
        parser.functions[0].strict = strict;
        parser.functions[0].arguments_forbidden = arguments_forbidden;
        if is_module {
            parser.parse_module_body(&mut module_attribute_checker)?;
        } else {
            parser.parse_script_body()?;
        }
        #[cfg(feature = "profiling")]
        crate::engine::compiler::diagnostics::sample_ir_storage(
            crate::engine::api::profiling::CompilePhase::Parse,
            crate::engine::compiler::diagnostics::arena_bytes(&parser.functions),
            parser.functions.iter().map(|builder| &builder.ir),
        );
        Ok(FunctionTree {
            functions: parser
                .functions
                .into_iter()
                .map(FunctionBuilder::finish)
                .collect::<Result<_, _>>()?,
            source: source_text
                .cloned()
                .unwrap_or_else(|| SourceText::from_utf8(source)),
            filename,
            module: parser.module,
            pending_unsupported: parser.pending_unsupported,
        })
    }
}

use crate::engine::api::error::ErrorKind;
use crate::engine::code::function::metadata::ClosureSource;
use crate::engine::code::function::metadata::ClosureVariable;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::code::function::metadata::ClosureVariableName;
use crate::engine::code::function::metadata::EvalScopeKind;
use crate::engine::compiler::ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME;
use crate::engine::compiler::EVAL_VARIABLE_OBJECT_LOCAL_NAME;
use crate::engine::compiler::WITH_OBJECT_LOCAL_NAME;
use crate::engine::compiler::model::bindings::BindingStorage;
use crate::engine::compiler::model::bindings::binding_kind_from_closure_flags;
use crate::engine::compiler::model::ir::function::FunctionIr;
use crate::engine::compiler::resolution::ensure_string_constant;
use crate::engine::compiler::resolution::push_closure_variable;

pub(in crate::engine::compiler) fn install_eval_external_bindings(
    function: &mut FunctionIr,
    bindings: Box<[EvalRootBinding<JsString>]>,
    caller_profile: EvalCallerProfile,
    caller_strict: bool,
) -> Result<(), Error> {
    let FunctionKind::Eval(kind) = function.kind else {
        return Err(Error::internal(
            "eval caller bindings escaped a synthetic eval root",
        ));
    };
    if kind == EvalKind::Indirect && !bindings.is_empty() {
        return Err(Error::internal(
            "indirect eval root received external caller bindings",
        ));
    }
    if !function.closure_variables.is_empty() || !function.external_bindings.is_empty() {
        return Err(Error::internal(
            "eval caller bindings were installed more than once",
        ));
    }
    if bindings.iter().any(|binding| {
        let Some(&scope_kind) = caller_profile.scope_kinds.get(usize::from(binding.scope)) else {
            return true;
        };
        (binding.is_catch_parameter && scope_kind != EvalScopeKind::Catch)
            || (binding.kind == ClosureVariableKind::WithObject)
                != (scope_kind == EvalScopeKind::With)
    }) || caller_profile
        .scope_kinds
        .iter()
        .enumerate()
        .any(|(scope, kind)| {
            *kind == EvalScopeKind::With
                && bindings
                    .iter()
                    .filter(|binding| usize::from(binding.scope) == scope)
                    .count()
                    != 1
        })
    {
        return Err(Error::internal(
            "eval caller bindings disagree with their scope profile",
        ));
    }
    let has_variable_object = bindings.iter().any(|binding| {
        matches!(
            binding.kind,
            ClosureVariableKind::EvalVariableObject | ClosureVariableKind::ArgEvalVariableObject
        )
    });
    match (caller_strict, caller_profile.variable_target) {
        (false, EvalCallerVariableTarget::Global) if !has_variable_object => {}
        (true, EvalCallerVariableTarget::StrictLocal) if kind == EvalKind::Direct => {}
        (false, EvalCallerVariableTarget::ExternalBinding(index))
            if bindings.get(usize::from(index)).is_some_and(|binding| {
                matches!(
                    binding.kind,
                    ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                ) && !binding.is_lexical
                    && !binding.is_const
                    && !binding.is_catch_parameter
            }) => {}
        _ => {
            return Err(Error::internal(
                "eval caller variable target is not authenticated",
            ));
        }
    }

    for (index, binding) in bindings.iter().enumerate() {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        let name = String::from_utf16(&binding.name.utf16_units().collect::<Vec<_>>())
            .map_err(|_| Error::internal("eval caller binding name is not well formed"))?;
        let name = ensure_string_constant(function, &name)?;
        let descriptor = ClosureVariable {
            source: ClosureSource::EvalEnvironment(index),
            name: ClosureVariableName::Constant(name),
            is_lexical: binding.is_lexical,
            is_const: binding.is_const,
            kind: binding.kind,
        };
        let installed = push_closure_variable(function, descriptor)?;
        if installed != index {
            return Err(Error::internal(
                "eval caller closure indices are not contiguous",
            ));
        }
    }

    // Scope bindings are searched newest-first. Install outer-to-inner so the
    // innermost exact descriptor wins for duplicate names while every closure
    // slot remains available to the specialized publication verifier. The
    // `<var>` remains unspellable source metadata, but it must still have a
    // binding identity in the synthetic root.  QuickJS relays the same hidden
    // closure VarRef when eval source itself contains a direct eval; retaining
    // it here lets that later call authenticate the exact variable target.
    for (index, binding) in bindings.iter().enumerate().rev() {
        let index = u16::try_from(index)
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?;
        if binding.kind == ClosureVariableKind::EvalVariableObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.to_utf8_lossy() != EVAL_VARIABLE_OBJECT_LOCAL_NAME)
        {
            return Err(Error::internal(
                "eval variable object binding metadata is malformed",
            ));
        }
        if binding.kind == ClosureVariableKind::ArgEvalVariableObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.to_utf8_lossy() != ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME)
        {
            return Err(Error::internal(
                "argument eval variable object binding metadata is malformed",
            ));
        }
        if binding.kind == ClosureVariableKind::WithObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.to_utf8_lossy() != WITH_OBJECT_LOCAL_NAME)
        {
            return Err(Error::internal("with object binding metadata is malformed"));
        }
        let name = String::from_utf16(&binding.name.utf16_units().collect::<Vec<_>>())
            .map_err(|_| Error::internal("eval caller binding name is not well formed"))?;
        let kind =
            binding_kind_from_closure_flags(binding.kind, binding.is_lexical, binding.is_const)
                .ok_or_else(|| Error::internal("eval caller binding flags are inconsistent"))?;
        let installed = function.add_binding(
            function.var_scope,
            function.var_scope,
            name,
            BindingStorage::External(index),
            kind,
            None,
        );
        function.bindings[installed.0].is_catch_parameter = binding.is_catch_parameter;
    }
    function.external_bindings = bindings.into_vec();
    function.eval_caller_profile = caller_profile;
    Ok(())
}
