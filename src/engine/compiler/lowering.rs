//! Lower resolved IR to verified bytecode and source debug information.
use crate::engine::compiler::model::bindings::{BindingKind, BindingStorage};
use crate::engine::compiler::model::ir::function::FunctionIr;
use crate::engine::compiler::model::ir::function::FunctionKind;
use crate::engine::compiler::model::ir::function::FunctionTree;
use crate::engine::compiler::model::ir::{
    CallArguments, IdentifierAccess, IdentifierReferenceAccess, IrConstant, IrOp, SpannedIrOp,
};
use crate::engine::compiler::model::scope::ScopeKind;

use crate::source::coordinates::QuickJsSourceIndex;

#[cfg(test)]
use super::DetachedBytecode;
use super::{ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME, EVAL_VARIABLE_OBJECT_LOCAL_NAME};
use crate::engine::api::error::Error;
use crate::engine::api::error::ErrorKind;
#[cfg(test)]
use crate::engine::atom::AtomTable;
use crate::engine::code::bytecode::DynamicEnvironmentSource;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::bytecode_validation::quickjs_copies_defined_argument_count;
use crate::engine::code::debug::DebugInfoMode;
use crate::engine::code::debug::Pc2LineEntry;
use crate::engine::code::debug::Pc2LineTable;
use crate::engine::code::function::UnlinkedConstant;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::UnlinkedFunctionDebug;
use crate::engine::code::function::UnlinkedVariableDefinition;
use crate::engine::code::function::metadata::ClosureSource;
use crate::engine::code::function::metadata::ClosureVariableKind;
use crate::engine::code::function::metadata::ClosureVariableName;
use crate::engine::code::function::metadata::ConstructorKind;
use crate::engine::code::function::metadata::EvalBindingSource;
use crate::engine::code::function::metadata::EvalKind;
use crate::engine::code::function::metadata::FunctionKind as BytecodeFunctionKind;
use crate::engine::code::function::metadata::FunctionMetadata;
use crate::engine::code::function::metadata::ParameterArgumentCell;
use crate::engine::code::function::metadata::ParameterBodyStorage;
use crate::engine::code::function::metadata::ParameterEnvironmentLayout;
use crate::engine::code::function::metadata::ParameterPatternCopy;
use crate::engine::compiler::flow::verify_lowered_max_stack;
use crate::engine::compiler::optimize::{
    apply_quickjs_late_throw_sites, fold_quickjs_constant_branches,
};
use crate::engine::compiler::relocation::relocate_lowered_instruction;
use crate::engine::value::JsString;
use crate::engine::value::JsStringError;
use crate::engine::value::PrimitiveValue as Value;
use crate::source::QuickJsSourceLocator;
use crate::source::SourceOffset;
use crate::source::text::SourceText;
#[cfg(test)]
use std::collections::HashMap;
use std::ops::Range;

#[derive(Debug, Default)]
struct ScopeLifecycle {
    tdz_locals: Vec<u16>,
    function_entries: Vec<ScopedFunctionEntry>,
    close_locals: Vec<u16>,
}

#[derive(Clone, Copy, Debug)]
struct ScopedFunctionEntry {
    constant: u32,
    local: u16,
}

fn captured_locals_by_function(functions: &[FunctionIr]) -> Result<Vec<Vec<bool>>, Error> {
    let mut captured = functions
        .iter()
        .map(|function| vec![false; function.locals.len()])
        .collect::<Vec<_>>();
    for (function_id, function) in functions.iter().enumerate().skip(1) {
        let parent = function
            .parent
            .ok_or_else(|| Error::internal("non-root function has no parent while lowering"))?
            .function;
        let parent_captured = captured
            .get_mut(parent)
            .ok_or_else(|| Error::internal("captured-local parent is out of bounds"))?;
        for descriptor in &function.closure_variables {
            let ClosureSource::ParentLocal(index) = descriptor.source else {
                continue;
            };
            let captured = parent_captured.get_mut(usize::from(index)).ok_or_else(|| {
                Error::internal("child closure captures an out-of-bounds parent local")
            })?;
            *captured = true;
        }
        if parent >= function_id {
            return Err(Error::internal(
                "function parent must precede its child while lowering",
            ));
        }
    }
    for (function_id, function) in functions.iter().enumerate() {
        let function_captured = captured
            .get_mut(function_id)
            .ok_or_else(|| Error::internal("eval captured-local function is out of bounds"))?;
        for binding in function
            .eval_environments
            .iter()
            .flat_map(|environment| environment.scopes.iter())
            .flat_map(|scope| scope.bindings.iter())
        {
            let EvalBindingSource::Local(index) = binding.source else {
                continue;
            };
            let captured = function_captured
                .get_mut(usize::from(index))
                .ok_or_else(|| Error::internal("eval captures an out-of-bounds local"))?;
            *captured = true;
        }
    }
    Ok(captured)
}

fn build_scope_lifecycles(
    function: &FunctionIr,
    captured_locals: &[bool],
) -> Result<Vec<ScopeLifecycle>, Error> {
    if captured_locals.len() != function.locals.len() {
        return Err(Error::internal(
            "captured-local metadata has the wrong length",
        ));
    }
    let mut scoped_constants = vec![None; function.bindings.len()];
    for scoped in &function.scoped_functions {
        let slot = scoped_constants
            .get_mut(scoped.binding.0)
            .ok_or_else(|| Error::internal("scoped function binding is out of bounds"))?;
        if slot.replace(scoped.constant).is_some() {
            return Err(Error::internal(
                "scoped function binding has more than one child record",
            ));
        }
    }
    function
        .scopes
        .iter()
        .map(|scope| {
            let mut lifecycle = ScopeLifecycle::default();
            // QuickJS links scope variables newest-first and expands both
            // enter and leave in that order after variable resolution.
            for &binding_id in scope.bindings.iter().rev() {
                let binding = function
                    .bindings
                    .get(binding_id.0)
                    .ok_or_else(|| Error::internal("scope binding is out of bounds"))?;
                let is_lexical = matches!(
                    binding.kind,
                    BindingKind::Lexical { .. }
                        | BindingKind::PrivateField { .. }
                        | BindingKind::PrivateMethod { .. }
                        | BindingKind::PrivateGetter { .. }
                        | BindingKind::PrivateSetter { .. }
                        | BindingKind::PrivateGetterSetter { .. }
                );
                let is_with_object = binding.kind == BindingKind::WithObject;
                if !is_lexical && !is_with_object {
                    continue;
                }
                let index = match binding.storage {
                    BindingStorage::Local(index) => index,
                    BindingStorage::External(_)
                    | BindingStorage::Module(_)
                    | BindingStorage::Global => continue,
                    BindingStorage::Argument(_) => {
                        return Err(Error::internal(
                            "scoped binding lifecycle referenced an argument",
                        ));
                    }
                };
                if is_lexical {
                    if let Some(constant) = scoped_constants[binding_id.0] {
                        lifecycle.function_entries.push(ScopedFunctionEntry {
                            constant,
                            local: index,
                        });
                    } else if function.synthetic_parameter_arguments_local != Some(index) {
                        lifecycle.tdz_locals.push(index);
                    }
                }
                if captured_locals[usize::from(index)] {
                    lifecycle.close_locals.push(index);
                }
            }
            Ok(lifecycle)
        })
        .collect()
}

#[cfg(test)]
pub(super) fn lower_detached_script(tree: FunctionTree) -> Result<DetachedBytecode<Value>, Error> {
    let mut functions = tree.functions;
    let function = functions
        .pop()
        .ok_or_else(|| Error::internal("compiler produced no script function"))?;
    if !function.closure_variables.is_empty() {
        return Err(Error::internal(
            "detached compiler cannot publish global-environment closure variables",
        ));
    }
    let captured_locals = vec![false; function.locals.len()];
    let scope_lifecycles = build_scope_lifecycles(&function, &captured_locals)?;
    let code = lower_ops(function.ops, &scope_lifecycles)?.code;
    let mut atom_strings = HashMap::<u32, Vec<JsString>>::new();
    let constants = function
        .constants
        .into_iter()
        .map(|constant| match constant {
            IrConstant::Primitive(value) => Ok(value),
            IrConstant::AtomString(value) => {
                if AtomTable::immediate_integer_atom(&value).is_some() {
                    Ok(Value::String(value))
                } else {
                    let strings = atom_strings.entry(value.content_hash()).or_default();
                    if let Some(canonical) = strings.iter().find(|string| *string == &value) {
                        Ok(Value::String(canonical.clone()))
                    } else {
                        strings.push(value.clone());
                        Ok(Value::String(value))
                    }
                }
            }
            IrConstant::RegExp { .. } => Err(Error::new(
                ErrorKind::Unsupported,
                "RegExp literals require runtime publication; use Context::compile or Context::eval",
            )),
            IrConstant::TemplateObject { .. } => Err(Error::new(
                ErrorKind::Unsupported,
                "tagged template objects require runtime publication; use Context::compile or Context::eval",
            )),
            IrConstant::Child(_) => Err(Error::internal(
                "detached compiler accepted a child-function constant",
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let max_stack = verify_lowered_max_stack(&code, constants.len())?;
    let bytecode = DetachedBytecode {
        code,
        constants,
        local_count: u16::try_from(function.locals.len())
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?,
        max_stack,
    };
    bytecode.verify()?;
    Ok(bytecode)
}

pub(super) fn lower_unlinked_tree(
    tree: FunctionTree,
    debug_info: DebugInfoMode,
) -> Result<UnlinkedFunction, Error> {
    #[cfg(feature = "profiling")]
    let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
        crate::engine::api::profiling::CompilePhase::Lowering,
    );

    let FunctionTree {
        functions: tree_functions,
        source,
        filename,
        module: _,
        pending_unsupported: _,
    } = tree;
    let source_index = if debug_info == DebugInfoMode::StripDebug {
        None
    } else {
        Some(
            QuickJsSourceLocator::from_bytes(source.raw_bytes())
                .index()
                .map_err(|error| Error::internal(error.to_string()))?,
        )
    };
    #[cfg(feature = "profiling")]
    crate::engine::compiler::diagnostics::sample_ir_storage(
        crate::engine::api::profiling::CompilePhase::Lowering,
        crate::engine::compiler::diagnostics::arena_bytes(&tree_functions),
        tree_functions.iter(),
    );
    let function_count = tree_functions.len();
    let captured_locals = captured_locals_by_function(&tree_functions)?;
    // A descendant eval descriptor names bindings owned by each ancestor and
    // by every intervening closure relay. Synthetic eval and module trees also
    // need names on their roots and every child relay: eval authenticates its
    // imported environment, while the module linker uses names for its own
    // declarations and unresolved globals. Those names are semantic linker
    // metadata, not optional debug labels, so StripDebug must retain them the
    // way QuickJS retains vardef names on these chains.
    let synthetic_semantic_tree = tree_functions.first().is_some_and(|function| {
        matches!(function.kind, FunctionKind::Eval(_) | FunctionKind::Module)
    });
    let mut retains_semantic_names = tree_functions
        .iter()
        .map(|function| synthetic_semantic_tree || !function.eval_environments.is_empty())
        .collect::<Vec<_>>();
    for function_id in (1..function_count).rev() {
        if !retains_semantic_names[function_id] {
            continue;
        }
        let parent = tree_functions[function_id]
            .parent
            .ok_or_else(|| Error::internal("eval-visible function has no parent"))?
            .function;
        let retained = retains_semantic_names
            .get_mut(parent)
            .ok_or_else(|| Error::internal("semantic-name parent is out of bounds"))?;
        *retained = true;
    }
    let mut functions = tree_functions.into_iter().map(Some).collect::<Vec<_>>();
    let mut lowered = (0..function_count).map(|_| None).collect::<Vec<_>>();

    for function_id in (0..function_count).rev() {
        let mut function = functions[function_id]
            .take()
            .ok_or_else(|| Error::internal("function IR was lowered more than once"))?;
        let retain_semantic_names = retains_semantic_names[function_id];
        if debug_info == DebugInfoMode::StripDebug && !retain_semantic_names {
            for descriptor in &mut function.closure_variables {
                if descriptor.is_lexical
                    && descriptor.kind == ClosureVariableKind::Normal
                    && !matches!(
                        descriptor.source,
                        ClosureSource::GlobalDeclaration
                            | ClosureSource::Global
                            | ClosureSource::ParentGlobal(_)
                    )
                {
                    descriptor.name = ClosureVariableName::None;
                }
            }
        }
        let argument_definitions = function
            .parameters
            .iter()
            .map(|name| {
                name.as_deref()
                    .map(JsString::try_from_utf8)
                    .transpose()
                    .map(UnlinkedVariableDefinition::ordinary)
                    .map_err(Error::from)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut local_definitions = (0..function.locals.len())
            .map(|_| UnlinkedVariableDefinition::ordinary(None))
            .collect::<Vec<_>>();
        for binding in &function.bindings {
            let BindingStorage::Local(index) = binding.storage else {
                continue;
            };
            let definition = local_definitions
                .get_mut(usize::from(index))
                .ok_or_else(|| Error::internal("local binding definition is out of bounds"))?;
            let storage_scope = function
                .scopes
                .get(binding.storage_scope.0)
                .ok_or_else(|| Error::internal("local binding scope is out of bounds"))?;
            let is_parameter_initializer = storage_scope.is_parameter_initializer
                && storage_scope.kind != ScopeKind::Parameter;
            let name = if debug_info == DebugInfoMode::StripDebug
                && !retain_semantic_names
                && matches!(binding.kind, BindingKind::Lexical { .. })
                && !function.parameter_locals.contains(&index)
            {
                None
            } else {
                Some(JsString::try_from_utf8(&binding.name)?)
            };
            *definition = match binding.kind {
                BindingKind::Lexical { is_const } => {
                    UnlinkedVariableDefinition::lexical(name, is_const)
                }
                BindingKind::Normal | BindingKind::FunctionName { .. } => {
                    UnlinkedVariableDefinition::ordinary(name)
                }
                BindingKind::EvalVariableObject => UnlinkedVariableDefinition {
                    name: Some(JsString::from_static(EVAL_VARIABLE_OBJECT_LOCAL_NAME)),
                    is_lexical: false,
                    is_const: false,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::EvalVariableObject,
                },
                BindingKind::ArgEvalVariableObject => UnlinkedVariableDefinition {
                    name: Some(JsString::from_static(ARG_EVAL_VARIABLE_OBJECT_LOCAL_NAME)),
                    is_lexical: false,
                    is_const: false,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::ArgEvalVariableObject,
                },
                BindingKind::WithObject => UnlinkedVariableDefinition::with_object(),
                BindingKind::PrivateField { .. } => UnlinkedVariableDefinition {
                    name,
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateField,
                },
                BindingKind::PrivateMethod { .. } => UnlinkedVariableDefinition {
                    name,
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateMethod,
                },
                BindingKind::PrivateGetter { .. } => UnlinkedVariableDefinition {
                    name,
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateGetter,
                },
                BindingKind::PrivateSetter { .. } => UnlinkedVariableDefinition {
                    name,
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateSetter,
                },
                BindingKind::PrivateGetterSetter { .. } => UnlinkedVariableDefinition {
                    name,
                    is_lexical: true,
                    is_const: true,
                    is_parameter_initializer: false,
                    kind: ClosureVariableKind::PrivateGetterSetter,
                },
            }
            .with_parameter_initializer(is_parameter_initializer);
        }
        let scope_lifecycles = build_scope_lifecycles(
            &function,
            captured_locals
                .get(function_id)
                .ok_or_else(|| Error::internal("captured-local function is out of bounds"))?,
        )?;
        let parameter_environment_parts = function
            .parameter_scope
            .map(|_| {
                let argument_cells = function
                    .parameter_argument_locals
                    .iter()
                    .enumerate()
                    .filter_map(|(argument, local)| {
                        local.map(|parameter_local| {
                            u16::try_from(argument).map(|argument| ParameterArgumentCell {
                                argument,
                                parameter_local,
                                body: ParameterBodyStorage::Argument(argument),
                            })
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?;
                let pattern_copies = function
                    .parameter_pattern_bindings
                    .iter()
                    .map(|binding| {
                        binding
                            .body_local
                            .map(|body_local| ParameterPatternCopy {
                                parameter_local: binding.parameter_local,
                                body_local,
                            })
                            .ok_or_else(|| {
                                Error::internal("parameter pattern body binding was not allocated")
                            })
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Ok::<_, Error>((argument_cells, pattern_copies))
            })
            .transpose()?;
        let has_pattern_parameters = function.pattern_parameter_initialization;
        let lowered_ops = lower_ops(function.ops, &scope_lifecycles)?;
        let parameter_initialization_end = lowered_ops.parameter_initialization_end;
        let parameter_pattern_end = has_pattern_parameters
            .then_some(parameter_initialization_end)
            .flatten();
        let parameter_environment = parameter_environment_parts
            .map(|(argument_cells, pattern_copies)| {
                Ok::<_, Error>(ParameterEnvironmentLayout {
                    initialization_end: parameter_initialization_end.ok_or_else(|| {
                        Error::internal("parameter environment has no initialization marker")
                    })?,
                    argument_cells: argument_cells.into_boxed_slice(),
                    pattern_copies: pattern_copies.into_boxed_slice(),
                    default_sources: function.parameter_default_sources.into_boxed_slice(),
                    synthetic_arguments_local: function.synthetic_parameter_arguments_local,
                    arg_eval_variable_object_local: function.arg_eval_variable_object_local,
                })
            })
            .transpose()?;
        let code = lowered_ops.code;
        let constant_count = function.constants.len();
        let constants = function
            .constants
            .into_iter()
            .map(|constant| match constant {
                IrConstant::AtomString(value) => Ok(UnlinkedConstant::atom_string(value)),
                IrConstant::Primitive(value) => unlinked_primitive(value),
                IrConstant::RegExp { pattern, program } => {
                    Ok(UnlinkedConstant::regexp(pattern, program))
                }
                IrConstant::TemplateObject { cooked, raw } => {
                    UnlinkedConstant::template_object(cooked, raw).map_err(|error| {
                        Error::internal(format!(
                            "compiler produced an invalid template object: {error}"
                        ))
                    })
                }
                IrConstant::Child(child) => lowered
                    .get_mut(child)
                    .and_then(Option::take)
                    .map(UnlinkedConstant::child)
                    .ok_or_else(|| Error::internal("child function was not lowered exactly once")),
            })
            .collect::<Result<Vec<_>, _>>()?;
        let max_stack = verify_lowered_max_stack(&code, constant_count)?;
        let metadata = FunctionMetadata {
            argument_count: u16::try_from(function.parameters.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?,
            // QuickJS only copies `fd->defined_arg_count` into its zeroed
            // bytecode record when `arg_count + var_count > 0`. A terminal
            // empty rest BindingPattern can increment the parser count while
            // owning neither a physical argument nor any QuickJS variable,
            // observably leaving Function.length at zero. Direct HomeObject,
            // `this`, and `new.target` reads allocate QuickJS pseudo variables
            // even though this backend can encode them without locals, so they
            // must also keep the authored count. Preserve that publication
            // quirk rather than normalizing it to the specification model.
            defined_argument_count: if function.rest_pattern_start == Some(0)
                && !quickjs_copies_defined_argument_count(
                    function.parameters.len(),
                    function.locals.len(),
                    &code,
                ) {
                0
            } else {
                u16::try_from(function.defined_argument_count)
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?
            },
            rest_parameter: function.rest_parameter,
            rest_pattern_start: function.rest_pattern_start,
            parameter_environment_local_count: u16::try_from(function.parameter_locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?,
            pattern_argument_count: u16::try_from(
                function
                    .parameters
                    .iter()
                    .filter(|name| name.is_none())
                    .count(),
            )
            .map_err(|_| Error::new(ErrorKind::JsInternal, "too many arguments"))?,
            parameter_pattern_end,
            local_count: u16::try_from(function.locals.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many local variables"))?,
            function_name_local: function.function_name_local,
            derived_this_local: function
                .derived_class_constructor
                .then_some(function.this_local)
                .flatten(),
            active_function_local: function
                .derived_class_constructor
                .then_some(function.active_function_local)
                .flatten(),
            eval_variable_object_local: function.eval_variable_object_local,
            needs_home_object: function.needs_home_object,
            closure_count: u16::try_from(function.closure_variables.len())
                .map_err(|_| Error::new(ErrorKind::JsInternal, "too many closure variables"))?,
            max_stack,
            strict: function.strict,
            strip_variable_debug: debug_info == DebugInfoMode::StripDebug
                && function.eval_environments.is_empty(),
            is_module: matches!(function.kind, FunctionKind::Module),
            super_call_allowed: function.super_call_allowed,
            super_allowed: function.super_allowed,
            arguments_forbidden: function.arguments_forbidden,
            eval_kind: match function.kind {
                FunctionKind::Eval(kind) => kind,
                FunctionKind::Script
                | FunctionKind::Module
                | FunctionKind::Ordinary
                | FunctionKind::Method
                | FunctionKind::Arrow => EvalKind::None,
            },
            function_kind: function.execution_kind,
            has_prototype: matches!(
                (function.kind, function.execution_kind),
                (FunctionKind::Ordinary, BytecodeFunctionKind::Normal)
                    | (
                        _,
                        BytecodeFunctionKind::Generator | BytecodeFunctionKind::AsyncGenerator
                    )
            ),
            constructor_kind: if function.execution_kind != BytecodeFunctionKind::Normal {
                ConstructorKind::None
            } else if function.derived_class_constructor {
                ConstructorKind::Derived
            } else if matches!(function.kind, FunctionKind::Ordinary) || function.class_constructor
            {
                ConstructorKind::Base
            } else {
                ConstructorKind::None
            },
            class_initializer_kind: function.class_initializer_kind,
            class_private_brand: function.class_private_brand,
        };
        let func_name = function
            .function_name
            .as_deref()
            .map(JsString::try_from_utf8)
            .transpose()?;
        let debug = match debug_info {
            DebugInfoMode::Full | DebugInfoMode::StripSource => Some(build_unlinked_debug(
                &source,
                source_index
                    .as_ref()
                    .ok_or_else(|| Error::internal("debug lowering has no source index"))?,
                filename.clone(),
                function.source.definition,
                if debug_info == DebugInfoMode::Full {
                    function.source.range
                } else {
                    None
                },
                &lowered_ops.pc_sites,
            )?),
            DebugInfoMode::StripDebug => None,
        };
        #[cfg(feature = "profiling")]
        crate::engine::api::profiling::record_lowered_function(&code, max_stack);
        let unlinked = UnlinkedFunction::new(
            code,
            constants,
            metadata,
            argument_definitions,
            local_definitions,
            function.closure_variables,
        )
        .with_parameter_environment(parameter_environment)
        .with_eval_environments(function.eval_environments)
        .with_name(func_name);
        lowered[function_id] = Some(match debug {
            Some(debug) => unlinked.with_debug(debug),
            None => unlinked,
        });
    }

    lowered[0]
        .take()
        .ok_or_else(|| Error::internal("root function was not lowered"))
}

struct LoweredOps {
    code: Vec<Instruction>,
    pc_sites: Vec<Option<SourceOffset>>,
    parameter_initialization_end: Option<u32>,
}

fn resolved_operation_len(operation: &IrOp) -> Result<usize, Error> {
    match operation {
        IrOp::Bytecode(_) | IrOp::PushConstant(_) | IrOp::MakeClosure(_) => Ok(1),
        IrOp::GlobalSet(_) | IrOp::CapturedLexicalSet(_) => Ok(2),
        _ => Err(Error::internal(
            "dynamic identifier retained a non-resolved fallback",
        )),
    }
}

fn dynamic_identifier_len(
    access: IdentifierAccess,
    sources: &[DynamicEnvironmentSource],
    fallback: &IrOp,
) -> Result<usize, Error> {
    let action_len = match access {
        IdentifierAccess::Get
        | IdentifierAccess::GetOrUndefined
        | IdentifierAccess::Put
        | IdentifierAccess::Delete => 1_usize,
        IdentifierAccess::Set => 2,
        IdentifierAccess::Initialize
        | IdentifierAccess::InitializeDerivedThis
        | IdentifierAccess::AnnexBPut => {
            return Err(Error::internal(
                "declaration-only access reached dynamic identifier lowering",
            ));
        }
    };
    sources
        .len()
        .checked_mul(action_len.saturating_add(3))
        .and_then(|length| length.checked_add(resolved_operation_len(fallback).ok()?))
        .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))
}

fn dynamic_identifier_reference_len(
    access: IdentifierReferenceAccess,
    sources: &[DynamicEnvironmentSource],
    late_sources: &[DynamicEnvironmentSource],
    fallback: &IrOp,
    syntactic_with: bool,
    fallback_readonly: bool,
) -> Result<usize, Error> {
    let global_reference = global_reference_index(access, late_sources, fallback);
    let fallback_access = match access {
        IdentifierReferenceAccess::Prepare | IdentifierReferenceAccess::Set => {
            IdentifierAccess::Set
        }
        IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => IdentifierAccess::Get,
        IdentifierReferenceAccess::PostPut => IdentifierAccess::Put,
    };
    let fallback_len = if syntactic_with {
        resolved_operation_len(fallback)?
    } else {
        dynamic_identifier_len(fallback_access, late_sources, fallback)?
    };
    match access {
        IdentifierReferenceAccess::Prepare => sources
            .len()
            .checked_mul(4)
            .and_then(|length| {
                length.checked_add(if syntactic_with && fallback_readonly {
                    2
                } else {
                    1
                })
            })
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow")),
        IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call => sources
            .len()
            .checked_mul(5)
            .and_then(|length| length.checked_add(1))
            .and_then(|length| length.checked_add(fallback_len))
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow")),
        IdentifierReferenceAccess::Set | IdentifierReferenceAccess::PostPut
            if global_reference.is_some() =>
        {
            Ok(6)
        }
        IdentifierReferenceAccess::Set | IdentifierReferenceAccess::PostPut => 11_usize
            .checked_add(fallback_len)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow")),
    }
}

/// QuickJS keeps an unresolved global assignment as `OP_make_var_ref` when an
/// authored `with` can intercept the name.  The runtime operation snapshots
/// the global reference before the RHS, including TDZ/readonly checks and the
/// missing-global sentinel.  Calls deliberately keep the ordinary
/// `undefined, GetVar` shape because `OP_scope_get_ref` is not an lvalue.
fn global_reference_index(
    access: IdentifierReferenceAccess,
    late_sources: &[DynamicEnvironmentSource],
    fallback: &IrOp,
) -> Option<u16> {
    // A GlobalSet fallback is not yet a prepared global Reference while an
    // imported eval/with environment remains to be tested. QuickJS leaves an
    // undefined sentinel on the stack in this case and runs scope_put_var
    // after the RHS; only a path with no late environment may use the compact
    // GlobalReference/PutRefValue pair.
    if !late_sources.is_empty() {
        return None;
    }
    match (access, fallback) {
        (
            IdentifierReferenceAccess::Prepare | IdentifierReferenceAccess::Set,
            IrOp::GlobalSet(index),
        )
        | (IdentifierReferenceAccess::Get, IrOp::Bytecode(Instruction::GetVar(index)))
        | (IdentifierReferenceAccess::PostPut, IrOp::Bytecode(Instruction::PutVar(index))) => {
            Some(*index)
        }
        _ => None,
    }
}

fn emit_resolved_operation(
    operation: IrOp,
    site: Option<SourceOffset>,
    code: &mut Vec<Instruction>,
    pc_sites: &mut Vec<Option<SourceOffset>>,
) -> Result<(), Error> {
    match operation {
        IrOp::Bytecode(instruction) => {
            if matches!(
                instruction,
                Instruction::Goto(_)
                    | Instruction::IfFalse(_)
                    | Instruction::IfTrue(_)
                    | Instruction::Catch(_)
                    | Instruction::Gosub(_)
            ) {
                return Err(Error::internal(
                    "dynamic identifier fallback retained a control-flow edge",
                ));
            }
            code.push(instruction);
            pc_sites.push(site);
        }
        IrOp::PushConstant(index) => {
            code.push(Instruction::PushConst(index));
            pc_sites.push(site);
        }
        IrOp::MakeClosure(index) => {
            code.push(Instruction::FClosure(index));
            pc_sites.push(site);
        }
        IrOp::GlobalSet(index) => {
            code.push(Instruction::Dup);
            pc_sites.push(site);
            code.push(Instruction::PutVar(index));
            pc_sites.push(None);
        }
        IrOp::CapturedLexicalSet(index) => {
            code.push(Instruction::Dup);
            pc_sites.push(site);
            code.push(Instruction::PutVarRefCheck(index));
            pc_sites.push(None);
        }
        _ => {
            return Err(Error::internal(
                "dynamic identifier retained a non-resolved fallback",
            ));
        }
    }
    Ok(())
}

fn emit_dynamic_identifier_operation(
    name: u32,
    access: IdentifierAccess,
    sources: &[DynamicEnvironmentSource],
    fallback: IrOp,
    site: Option<SourceOffset>,
    code: &mut Vec<Instruction>,
    pc_sites: &mut Vec<Option<SourceOffset>>,
) -> Result<(), Error> {
    let emitted = dynamic_identifier_len(access, sources, &fallback)?;
    let end = code
        .len()
        .checked_add(emitted)
        .and_then(|target| u32::try_from(target).ok())
        .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    let action_len = if access == IdentifierAccess::Set {
        2_usize
    } else {
        1
    };
    let mut first = true;
    for &source in sources {
        let next = code
            .len()
            .checked_add(action_len.saturating_add(3))
            .and_then(|target| u32::try_from(target).ok())
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
        code.push(Instruction::HasDynamicBinding { source, name });
        pc_sites.push(if first { site } else { None });
        first = false;
        code.push(Instruction::IfFalse(next));
        pc_sites.push(None);
        match access {
            IdentifierAccess::Get | IdentifierAccess::GetOrUndefined => {
                code.push(Instruction::GetDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Put => {
                code.push(Instruction::PutDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Set => {
                code.push(Instruction::Dup);
                pc_sites.push(None);
                code.push(Instruction::PutDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Delete => {
                code.push(Instruction::DeleteDynamicBinding { source, name });
                pc_sites.push(None);
            }
            IdentifierAccess::Initialize
            | IdentifierAccess::InitializeDerivedThis
            | IdentifierAccess::AnnexBPut => {
                return Err(Error::internal(
                    "declaration-only access reached dynamic identifier lowering",
                ));
            }
        }
        code.push(Instruction::Goto(end));
        pc_sites.push(None);
    }
    emit_resolved_operation(fallback, if first { site } else { None }, code, pc_sites)?;
    if u32::try_from(code.len()).ok() != Some(end) {
        return Err(Error::internal(
            "dynamic identifier lowering length changed",
        ));
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn emit_dynamic_identifier_reference(
    name: u32,
    access: IdentifierReferenceAccess,
    sources: &[DynamicEnvironmentSource],
    late_sources: &[DynamicEnvironmentSource],
    fallback: IrOp,
    syntactic_with: bool,
    fallback_readonly: bool,
    site: Option<SourceOffset>,
    code: &mut Vec<Instruction>,
    pc_sites: &mut Vec<Option<SourceOffset>>,
) -> Result<(), Error> {
    let emitted = dynamic_identifier_reference_len(
        access,
        sources,
        late_sources,
        &fallback,
        syntactic_with,
        fallback_readonly,
    )?;
    let start = code.len();
    let end = start
        .checked_add(emitted)
        .and_then(|target| u32::try_from(target).ok())
        .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    let global_reference = global_reference_index(access, late_sources, &fallback);
    let mut first = true;

    match access {
        IdentifierReferenceAccess::Prepare
        | IdentifierReferenceAccess::Get
        | IdentifierReferenceAccess::Call => {
            let action_len = if access == IdentifierReferenceAccess::Prepare {
                2_usize
            } else {
                3
            };
            for &source in sources {
                let next = code
                    .len()
                    .checked_add(action_len.saturating_add(2))
                    .and_then(|target| u32::try_from(target).ok())
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                code.push(Instruction::HasDynamicBinding { source, name });
                pc_sites.push(if first { site } else { None });
                first = false;
                code.push(Instruction::IfFalse(next));
                pc_sites.push(None);
                code.push(Instruction::DynamicEnvironmentObject(source));
                pc_sites.push(None);
                if matches!(
                    access,
                    IdentifierReferenceAccess::Get | IdentifierReferenceAccess::Call
                ) {
                    code.push(if access == IdentifierReferenceAccess::Call {
                        Instruction::GetRefValueUndef(name)
                    } else {
                        Instruction::GetRefValue(name)
                    });
                    pc_sites.push(None);
                }
                code.push(Instruction::Goto(end));
                pc_sites.push(None);
            }

            if access == IdentifierReferenceAccess::Prepare {
                if syntactic_with && fallback_readonly {
                    code.push(Instruction::Undefined);
                    pc_sites.push(if first { site } else { None });
                    code.push(Instruction::ThrowReadOnly(name));
                    pc_sites.push(None);
                } else if let Some(index) = global_reference {
                    code.push(Instruction::GlobalReference(index));
                    pc_sites.push(if first { site } else { None });
                } else {
                    code.push(Instruction::Undefined);
                    pc_sites.push(if first { site } else { None });
                }
            } else if access == IdentifierReferenceAccess::Get
                && syntactic_with
                && fallback_readonly
            {
                code.push(Instruction::Undefined);
                pc_sites.push(if first { site } else { None });
                code.push(Instruction::ThrowReadOnly(name));
                pc_sites.push(None);
            } else if access == IdentifierReferenceAccess::Get
                && let Some(index) = global_reference
            {
                code.push(Instruction::GlobalReference(index));
                pc_sites.push(if first { site } else { None });
                code.push(Instruction::GetRefValue(name));
                pc_sites.push(None);
            } else if syntactic_with {
                code.push(Instruction::Undefined);
                pc_sites.push(if first { site } else { None });
                emit_resolved_operation(fallback, None, code, pc_sites)?;
            } else {
                code.push(Instruction::Undefined);
                pc_sites.push(if first { site } else { None });
                emit_dynamic_identifier_operation(
                    name,
                    IdentifierAccess::Get,
                    late_sources,
                    fallback,
                    None,
                    code,
                    pc_sites,
                )?;
            }
        }
        IdentifierReferenceAccess::Set | IdentifierReferenceAccess::PostPut => {
            if access == IdentifierReferenceAccess::PostPut {
                code.push(Instruction::Perm3);
                pc_sites.push(site);
            } else {
                code.push(Instruction::Insert2);
                pc_sites.push(site);
                code.push(Instruction::Drop);
                pc_sites.push(None);
            }
            // Put the candidate base on top while retaining the result value
            // below it, then branch to the object-reference write when it is
            // not the static `undefined` sentinel.
            if access == IdentifierReferenceAccess::PostPut {
                code.push(Instruction::Insert2);
                pc_sites.push(None);
                code.push(Instruction::Drop);
                pc_sites.push(None);
            }
            if global_reference.is_some() {
                code.push(Instruction::Insert2);
                pc_sites.push(None);
                code.push(Instruction::Drop);
                pc_sites.push(None);
                if access == IdentifierReferenceAccess::Set {
                    code.push(Instruction::Insert2);
                    pc_sites.push(None);
                }
                code.push(Instruction::PutRefValue(name));
                pc_sites.push(None);
            } else {
                code.push(Instruction::Dup);
                pc_sites.push(None);
                code.push(Instruction::IsUndefinedOrNull);
                pc_sites.push(None);
                let dynamic_target_index = code.len();
                code.push(Instruction::IfFalse(u32::MAX));
                pc_sites.push(None);

                code.push(Instruction::Drop);
                pc_sites.push(None);
                if syntactic_with {
                    emit_resolved_operation(fallback, None, code, pc_sites)?;
                } else {
                    emit_dynamic_identifier_operation(
                        name,
                        if access == IdentifierReferenceAccess::Set {
                            IdentifierAccess::Set
                        } else {
                            IdentifierAccess::Put
                        },
                        late_sources,
                        fallback,
                        None,
                        code,
                        pc_sites,
                    )?;
                }
                code.push(Instruction::Goto(end));
                pc_sites.push(None);

                let dynamic_target = u32::try_from(code.len())
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                let Some(Instruction::IfFalse(target)) = code.get_mut(dynamic_target_index) else {
                    return Err(Error::internal("reference branch instruction disappeared"));
                };
                *target = dynamic_target;
                code.push(Instruction::Insert2);
                pc_sites.push(None);
                code.push(Instruction::Drop);
                pc_sites.push(None);
                if access == IdentifierReferenceAccess::Set {
                    code.push(Instruction::Insert2);
                    pc_sites.push(None);
                }
                code.push(Instruction::PutRefValue(name));
                pc_sites.push(None);
            }
        }
    }
    if u32::try_from(code.len()).ok() != Some(end) {
        return Err(Error::internal(
            "dynamic identifier Reference lowering length changed",
        ));
    }
    Ok(())
}

fn lower_ops(operations: Vec<SpannedIrOp>, scopes: &[ScopeLifecycle]) -> Result<LoweredOps, Error> {
    let mut offsets = Vec::with_capacity(operations.len() + 1);
    let mut code_len = 0_usize;
    for operation in &operations {
        offsets.push(code_len);
        let emitted = match &operation.op {
            IrOp::EnterScope(scope) => scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("scope entry is out of bounds"))?
                .tdz_locals
                .len()
                .checked_add(
                    scopes
                        .get(scope.0)
                        .ok_or_else(|| Error::internal("scope entry is out of bounds"))?
                        .function_entries
                        .len()
                        .saturating_mul(2),
                )
                .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            IrOp::PrepareCatchScope(scope) => {
                let lifecycle = scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("catch scope preparation is out of bounds"))?;
                if !lifecycle.function_entries.is_empty() {
                    return Err(Error::internal(
                        "catch parameter scope contains a function entry",
                    ));
                }
                lifecycle
                    .tdz_locals
                    .len()
                    .checked_mul(2)
                    .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?
            }
            IrOp::LeaveScope(scope) => scopes
                .get(scope.0)
                .ok_or_else(|| Error::internal("scope exit is out of bounds"))?
                .close_locals
                .len(),
            IrOp::GlobalSet(_) | IrOp::CapturedLexicalSet(_) => 2,
            IrOp::DynamicIdentifier {
                access,
                sources,
                fallback,
                ..
            } => dynamic_identifier_len(*access, sources, fallback)?,
            IrOp::DynamicIdentifierReference {
                access,
                sources,
                late_sources,
                fallback,
                syntactic_with,
                fallback_readonly,
                ..
            } => dynamic_identifier_reference_len(
                *access,
                sources,
                late_sources,
                fallback,
                *syntactic_with,
                *fallback_readonly,
            )?,
            _ => 1,
        };
        code_len = code_len
            .checked_add(emitted)
            .ok_or_else(|| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
    }
    offsets.push(code_len);

    let mut code = Vec::with_capacity(code_len);
    let mut pc_sites = Vec::with_capacity(code_len);
    let mut parameter_initialization_end = None;
    for operation in operations {
        let SpannedIrOp { op, pc_site } = operation;
        match op {
            IrOp::EnterScope(scope) => {
                let lifecycle = scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("scope entry is out of bounds"))?;
                // Reset every ordinary lexical lifetime before any block
                // closure captures it. This is observationally equivalent to
                // QuickJS's mixed newest-first expansion, while preserving the
                // runtime invariant that an initialized captured cell cannot
                // silently begin a new lifetime without CloseLocal.
                for &index in &lifecycle.tdz_locals {
                    code.push(Instruction::SetLocalUninitialized(index));
                    pc_sites.push(None);
                }
                for entry in &lifecycle.function_entries {
                    code.push(Instruction::FClosure(entry.constant));
                    pc_sites.push(None);
                    code.push(Instruction::InitializeLocal(entry.local));
                    pc_sites.push(None);
                }
            }
            IrOp::PrepareCatchScope(scope) => {
                let lifecycle = scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("catch scope preparation is out of bounds"))?;
                if !lifecycle.function_entries.is_empty() {
                    return Err(Error::internal(
                        "catch parameter scope contains a function entry",
                    ));
                }
                for &index in &lifecycle.tdz_locals {
                    code.push(Instruction::Undefined);
                    pc_sites.push(None);
                    code.push(Instruction::InitializeLocal(index));
                    pc_sites.push(None);
                }
            }
            IrOp::LeaveScope(scope) => {
                for &index in &scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("scope exit is out of bounds"))?
                    .close_locals
                {
                    code.push(Instruction::CloseLocal(index));
                    pc_sites.push(None);
                }
            }
            IrOp::ParameterInitializationEnd => {
                let pc = u32::try_from(code.len())
                    .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?;
                if parameter_initialization_end.replace(pc).is_some() {
                    return Err(Error::internal(
                        "function contains more than one parameter initialization marker",
                    ));
                }
                code.push(Instruction::Nop);
                pc_sites.push(None);
            }
            IrOp::Bytecode(mut instruction) => {
                relocate_lowered_instruction(&mut instruction, &offsets)?;
                code.push(instruction);
                pc_sites.push(pc_site);
            }
            IrOp::TemplateCall {
                argument_count,
                method,
            } => {
                // QuickJS `emit_u16` writes the low operand bits even when an
                // unreachable template has more than 65,535 arguments. The
                // reachability verifier still observes every push on a live
                // path and rejects its stack before this truncated call can
                // execute.
                code.push(if method {
                    Instruction::CallMethod(argument_count as u16)
                } else {
                    Instruction::Call(argument_count as u16)
                });
                pc_sites.push(pc_site);
            }
            IrOp::EvalCall {
                arguments,
                scope,
                environment,
            } => {
                // Retain the parser scope as an IR invariant until lowering,
                // but publish only its immutable linked descriptor ordinal.
                scopes
                    .get(scope.0)
                    .ok_or_else(|| Error::internal("eval call scope is out of bounds"))?;
                let environment = environment
                    .ok_or_else(|| Error::internal("eval call has no linked environment"))?;
                code.push(match arguments {
                    CallArguments::Fixed(argument_count) => Instruction::Eval {
                        argument_count,
                        environment,
                    },
                    CallArguments::Spread => Instruction::ApplyEval { environment },
                });
                pc_sites.push(pc_site);
            }
            IrOp::PushConstant(index) => {
                code.push(Instruction::PushConst(index));
                pc_sites.push(pc_site);
            }
            IrOp::MakeClosure(index) => {
                code.push(Instruction::FClosure(index));
                pc_sites.push(pc_site);
            }
            IrOp::GlobalSet(index) => {
                code.push(Instruction::Dup);
                pc_sites.push(pc_site);
                code.push(Instruction::PutVar(index));
                pc_sites.push(None);
            }
            IrOp::CapturedLexicalSet(index) => {
                code.push(Instruction::Dup);
                pc_sites.push(pc_site);
                code.push(Instruction::PutVarRefCheck(index));
                pc_sites.push(None);
            }
            IrOp::DynamicIdentifier {
                name,
                access,
                sources,
                fallback,
            } => {
                emit_dynamic_identifier_operation(
                    name,
                    access,
                    &sources,
                    *fallback,
                    pc_site,
                    &mut code,
                    &mut pc_sites,
                )?;
            }
            IrOp::DynamicIdentifierReference {
                name,
                access,
                sources,
                late_sources,
                fallback,
                syntactic_with,
                fallback_readonly,
            } => {
                emit_dynamic_identifier_reference(
                    name,
                    access,
                    &sources,
                    &late_sources,
                    *fallback,
                    syntactic_with,
                    fallback_readonly,
                    pc_site,
                    &mut code,
                    &mut pc_sites,
                )?;
            }
            IrOp::Identifier { .. }
            | IrOp::IdentifierReference { .. }
            | IrOp::ImportMeta { .. }
            | IrOp::PrivateField { .. } => {
                return Err(Error::internal(
                    "lexical operation reached bytecode lowering before resolution",
                ));
            }
        }
    }
    apply_quickjs_late_throw_sites(&code, &mut pc_sites)?;
    fold_quickjs_constant_branches(&mut code);
    Ok(LoweredOps {
        code,
        pc_sites,
        parameter_initialization_end,
    })
}

fn build_unlinked_debug(
    source: &SourceText,
    locator: &QuickJsSourceIndex<'_>,
    filename: JsString,
    definition: SourceOffset,
    source_range: Option<Range<SourceOffset>>,
    pc_sites: &[Option<SourceOffset>],
) -> Result<UnlinkedFunctionDebug, Error> {
    let carrier = source.carrier();
    let mut cursor = locator.cursor();
    let mut previous_site = definition;
    let definition = cursor
        .locate(definition)
        .map_err(|error| Error::internal(error.to_string()))?;
    let mut entries = Vec::new();
    let mut previous_position = Some(definition);
    for (pc, site) in pc_sites.iter().copied().enumerate() {
        let Some(site) = site else {
            continue;
        };
        // Repeated markers (including across unmarked instructions) were
        // already validated at this exact offset in the immutable source.
        if site == previous_site {
            continue;
        }
        let position = cursor
            .locate(site)
            .map_err(|error| Error::internal(error.to_string()))?;
        previous_site = site;
        if previous_position == Some(position) {
            continue;
        }
        entries.push(Pc2LineEntry {
            pc: u32::try_from(pc)
                .map_err(|_| Error::new(ErrorKind::JsInternal, "stack overflow"))?,
            position,
        });
        previous_position = Some(position);
    }

    let source = source_range
        .map(|range| {
            let start = range.start.as_usize();
            let end = range.end.as_usize();
            if start > end
                || end > carrier.len()
                || !carrier.is_char_boundary(start)
                || !carrier.is_char_boundary(end)
            {
                return Err(Error::internal("function source range is invalid"));
            }
            source
                .try_range_to_raw_bytes(start..end)
                .map_err(|_| Error::from(JsStringError::OutOfMemory))?
                .map(Vec::into_boxed_slice)
                .ok_or_else(|| Error::internal("function source range is invalid"))
        })
        .transpose()?;

    Ok(UnlinkedFunctionDebug {
        filename,
        pc2line: Some(Pc2LineTable::new(definition, entries)),
        source,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compiler::lexer::{Position, Span};
    use crate::engine::compiler::{
        FunctionIrOptions, FunctionSourceInfo, IrScope, ScopeId, SuperCapabilities,
        WITH_OBJECT_LOCAL_NAME, validate_scope_graph,
    };

    #[test]
    fn debug_source_cache_preserves_gaps_equal_coordinates_and_error_order() {
        use crate::source::{LineColumn, QuickJsSourceLocator};
        let offset = |value| SourceOffset::try_from_usize(value).unwrap();
        // Raw continuation bytes do not advance columns, so distinct offsets
        // 1 and 2 intentionally share a coordinate and must remain valid.
        let raw = [b'a', 0x80, b'b'];
        let source = SourceText::try_from_raw_bytes(&raw).unwrap();
        let locator = QuickJsSourceLocator::from_bytes(&raw).index().unwrap();
        let debug = build_unlinked_debug(
            &source,
            &locator,
            JsString::from_static("cache.js"),
            offset(0),
            None,
            &[
                Some(offset(0)),
                None,
                Some(offset(1)),
                None,
                Some(offset(1)),
                Some(offset(2)),
                Some(offset(3)),
                None,
                Some(offset(3)),
            ],
        )
        .unwrap();
        assert_eq!(
            debug.pc2line.unwrap().entries.as_ref(),
            &[
                Pc2LineEntry {
                    pc: 2,
                    position: LineColumn::new(0, 1)
                },
                Pc2LineEntry {
                    pc: 6,
                    position: LineColumn::new(0, 2)
                },
            ]
        );
        let source = SourceText::from_utf8("éx");
        let locator = QuickJsSourceLocator::new(source.carrier()).index().unwrap();
        // Repeated valid offsets cannot hide a following invalid new offset;
        // the first invalid site still wins over later sites and source ranges.
        let error = build_unlinked_debug(
            &source,
            &locator,
            JsString::from_static("cache.js"),
            offset(0),
            Some(offset(9)..offset(10)),
            &[
                Some(offset(0)),
                None,
                Some(offset(0)),
                Some(offset(1)),
                Some(offset(9)),
            ],
        )
        .unwrap_err();
        assert_eq!(error.message(), "source offset is not a UTF-8 boundary");
        let error = build_unlinked_debug(
            &source,
            &locator,
            JsString::from_static("cache.js"),
            offset(9),
            None,
            &[Some(offset(1))],
        )
        .unwrap_err();
        assert_eq!(error.message(), "source offset is out of bounds");
    }

    #[test]
    fn captured_with_object_has_close_lifetime_without_lexical_tdz() {
        let make_function = |strict| {
            let span = Span::new(Position::new(0, 1, 1), Position::new(0, 1, 1));
            let mut function = FunctionIr::new(
                None,
                FunctionKind::Ordinary,
                FunctionSourceInfo {
                    span,
                    definition: SourceOffset::try_from_usize(0).unwrap(),
                    range: None,
                },
                FunctionIrOptions {
                    function_name: None,
                    private_name_binding: false,
                    class_constructor: false,
                    derived_class_constructor: false,
                    parameters: Vec::new(),
                    defined_argument_count: 0,
                    has_simple_parameter_list: true,
                    rest_parameter: None,
                    strict,
                    super_capabilities: SuperCapabilities::NONE,
                },
            )
            .unwrap();
            let scope = ScopeId(function.scopes.len());
            function.scopes.push(IrScope {
                parent: Some(function.body_scope),
                kind: ScopeKind::With,
                is_parameter_initializer: false,
                bindings: Vec::new(),
                bindings_by_name: Default::default(),
            });
            function.locals.push(WITH_OBJECT_LOCAL_NAME.to_owned());
            function.add_binding(
                scope,
                scope,
                WITH_OBJECT_LOCAL_NAME.to_owned(),
                BindingStorage::Local(0),
                BindingKind::WithObject,
                None,
            );
            (function, scope)
        };

        let (function, scope) = make_function(false);
        let lifecycles = build_scope_lifecycles(&function, &[true]).unwrap();
        assert!(lifecycles[scope.0].tdz_locals.is_empty());
        assert!(lifecycles[scope.0].function_entries.is_empty());
        assert_eq!(lifecycles[scope.0].close_locals, [0]);

        let (strict, _) = make_function(true);
        let tree = FunctionTree {
            functions: vec![strict],
            source: "".into(),
            filename: JsString::from_static("<strict-with-metadata>"),
            module: None,
            pending_unsupported: None,
        };
        assert!(
            validate_scope_graph(&tree)
                .unwrap_err()
                .message()
                .contains("strict function retained a local with object")
        );
    }
}

pub(in crate::engine::compiler) fn unlinked_primitive(
    value: Value,
) -> Result<UnlinkedConstant, Error> {
    UnlinkedConstant::primitive(value).map_err(|error| {
        Error::internal(format!(
            "compiler emitted a runtime-bound constant into an unlinked function: {error}"
        ))
    })
}
