//! Pure authentication of publication drafts; verification retains the exact owned input.

mod bindings;
mod children;
mod closures;
mod eval;
mod flow;
use eval::{verify_eval_environments, verify_eval_super_pseudo_bindings};
mod module_initializer_flow;
mod modules;
mod operands;
mod parameters;
mod roles;
use modules::{verify_module_link_entry, verify_unlinked_module_tables};
pub(in crate::engine::code) mod private_elements;
mod verified;
pub(crate) use verified::VerifiedFunction;

use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::code::bytecode_validation::{
    EvalEnvironmentPhaseContext, validate_class_initializer_bytecode_layout,
    validate_derived_constructor_bytecode_layout, validate_eval_environment_phase_layout,
    validate_parameter_initializer_scope_layout, validate_pattern_parameter_bytecode_layout,
};
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariable, ClosureVariableKind, ClosureVariableName, EvalCallerProfile,
    EvalCallerVariableTarget, EvalKind, EvalRootBinding,
};
use crate::engine::code::function::{UnlinkedConstant, UnlinkedFunction};
use crate::engine::code::module::{ModuleImportCollisionDeclaration, UnlinkedModule};
use crate::engine::value::JsString;
use std::collections::{HashMap, HashSet};

fn unlinked_closure_name<'a>(
    function: &'a UnlinkedFunction,
    descriptor: &ClosureVariable,
) -> Result<Option<&'a JsString>, RuntimeError> {
    match descriptor.name {
        ClosureVariableName::None => Ok(None),
        ClosureVariableName::Constant(index) => {
            let name = usize::try_from(index)
                .ok()
                .and_then(|index| function.constants().get(index))
                .and_then(UnlinkedConstant::as_primitive);
            let Some(crate::engine::value::PrimitiveValue::String(name)) = name else {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor referenced a non-string name constant",
                )));
            };
            Ok(Some(name))
        }
        ClosureVariableName::Atom(_) => Err(RuntimeError::Engine(Error::internal(
            "unlinked closure descriptor already contained a runtime atom",
        ))),
    }
}

const fn eval_variable_object_sentinel(kind: ClosureVariableKind) -> Option<&'static str> {
    match kind {
        ClosureVariableKind::EvalVariableObject => Some("<var>"),
        ClosureVariableKind::ArgEvalVariableObject => Some("<arg_var>"),
        ClosureVariableKind::Normal
        | ClosureVariableKind::ModuleImportView
        | ClosureVariableKind::FunctionName
        | ClosureVariableKind::GlobalFunction
        | ClosureVariableKind::WithObject
        | ClosureVariableKind::PrivateField
        | ClosureVariableKind::PrivateMethod
        | ClosureVariableKind::PrivateGetter
        | ClosureVariableKind::PrivateSetter
        | ClosureVariableKind::PrivateGetterSetter => None,
    }
}

/// Canonical compiler-authored pseudo-binding entry order. The unlinked
/// publisher repeats the heap boundary's structural check while additionally
/// authenticating the source-only sentinel names.
const fn pseudo_binding_entry(
    instruction: &crate::engine::code::bytecode::Instruction,
) -> Option<(u8, &'static str)> {
    match instruction {
        crate::engine::code::bytecode::Instruction::PushHomeObject => Some((1, "<home_object>")),
        crate::engine::code::bytecode::Instruction::PushActiveFunction => {
            Some((2, "<this_active_func>"))
        }
        crate::engine::code::bytecode::Instruction::PushNewTarget => Some((3, "<new.target>")),
        crate::engine::code::bytecode::Instruction::PushThis => Some((4, "<this>")),
        _ => None,
    }
}

fn eval_root_binding_is_derived_this(binding: &EvalRootBinding<JsString>) -> bool {
    binding.is_lexical
        && !binding.is_const
        && binding.kind == ClosureVariableKind::Normal
        && !binding.is_catch_parameter
        && binding.name.utf16_units().eq("<this>".encode_utf16())
}

fn eval_root_binding_is_super_pseudo(
    binding: &EvalRootBinding<JsString>,
    expected_name: &'static str,
) -> bool {
    !binding.is_lexical
        && !binding.is_const
        && binding.kind == ClosureVariableKind::Normal
        && !binding.is_catch_parameter
        && binding.name.utf16_units().eq(expected_name.encode_utf16())
}

#[derive(Clone, Copy)]
enum RootPublication<'a> {
    Script,
    TrustedOrdinaryLeaf,
    Module(&'a UnlinkedModule),
    Eval {
        kind: EvalKind,
        caller_strict: bool,
        expected_bindings: &'a [EvalRootBinding<JsString>],
        expected_profile: &'a EvalCallerProfile,
        expected_capabilities: EvalPublicationCapabilities,
    },
}

#[derive(Clone, Copy)]
pub(crate) struct EvalPublicationCapabilities {
    pub super_call_allowed: bool,
    pub super_allowed: bool,
    pub arguments_forbidden: bool,
}

pub(crate) fn verify_unlinked_tree(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    verify_unlinked_tree_with_root(function, RootPublication::Script)
}

/// Authenticate one detached ordinary callable translated from trusted BC5.
///
/// Unlike a Script root, this role may own arguments and ordinary locals. It
/// remains a standalone leaf: no module, eval, class, super, HomeObject, or
/// closure authority may be smuggled through the specialized publication
/// entry point.
pub(in crate::engine::code) fn verify_unlinked_ordinary_leaf(
    function: &UnlinkedFunction,
) -> Result<(), RuntimeError> {
    verify_unlinked_tree_with_root(function, RootPublication::TrustedOrdinaryLeaf)
}

/// Authenticate a compiler-owned module record and the distinct bytecode ABI
/// used to link and evaluate it. Ordinary script publication never accepts
/// this root shape.
pub(crate) fn verify_unlinked_module_tree(module: &UnlinkedModule) -> Result<(), RuntimeError> {
    verify_unlinked_module_tables(module)?;
    verify_module_link_entry(module)?;
    verify_unlinked_tree_with_root(module.function(), RootPublication::Module(module))
}

/// Authenticate a synthetic eval root against the exact caller bindings that
/// will instantiate its `EvalEnvironment` closure slots. This entry point is
/// deliberately separate from ordinary script publication: accepting these
/// sources without the live caller descriptor would make a forged bytecode
/// root indistinguishable from a compiler-produced direct eval.
#[cfg(test)]
pub(crate) fn verify_unlinked_eval_tree(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_super_call_allowed: bool,
    expected_super_allowed: bool,
) -> Result<(), RuntimeError> {
    let scope_count = expected_bindings
        .iter()
        .map(|binding| usize::from(binding.scope) + 1)
        .max()
        .unwrap_or(0);
    let mut scope_kinds =
        vec![crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot; scope_count];
    for binding in expected_bindings {
        if binding.kind == ClosureVariableKind::WithObject {
            scope_kinds[usize::from(binding.scope)] =
                crate::engine::code::function::metadata::EvalScopeKind::With;
        } else if binding.is_catch_parameter {
            scope_kinds[usize::from(binding.scope)] =
                crate::engine::code::function::metadata::EvalScopeKind::Catch;
        }
    }
    for (scope, scope_kind) in scope_kinds.iter_mut().enumerate() {
        let has_parameter_object = expected_bindings.iter().any(|binding| {
            usize::from(binding.scope) == scope
                && binding.kind == ClosureVariableKind::ArgEvalVariableObject
        });
        let has_body_object = expected_bindings.iter().any(|binding| {
            usize::from(binding.scope) == scope
                && binding.kind == ClosureVariableKind::EvalVariableObject
        });
        if has_parameter_object && !has_body_object {
            *scope_kind = crate::engine::code::function::metadata::EvalScopeKind::Parameter;
        }
    }
    let variable_target = if caller_strict {
        EvalCallerVariableTarget::StrictLocal
    } else {
        expected_bindings
            .iter()
            .position(|binding| {
                matches!(
                    binding.kind,
                    ClosureVariableKind::EvalVariableObject
                        | ClosureVariableKind::ArgEvalVariableObject
                )
            })
            .and_then(|index| u16::try_from(index).ok())
            .map(EvalCallerVariableTarget::ExternalBinding)
            .unwrap_or(EvalCallerVariableTarget::Global)
    };
    verify_unlinked_eval_tree_with_profile(
        function,
        kind,
        caller_strict,
        expected_bindings,
        &EvalCallerProfile {
            scope_kinds: scope_kinds.into_boxed_slice(),
            variable_target,
        },
        expected_super_call_allowed,
        expected_super_allowed,
    )
}

#[cfg(test)]
pub(crate) fn verify_unlinked_eval_tree_with_profile(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
    expected_super_call_allowed: bool,
    expected_super_allowed: bool,
) -> Result<(), RuntimeError> {
    verify_unlinked_eval_tree_with_profile_and_arguments(
        function,
        kind,
        caller_strict,
        expected_bindings,
        expected_profile,
        EvalPublicationCapabilities {
            super_call_allowed: expected_super_call_allowed,
            super_allowed: expected_super_allowed,
            arguments_forbidden: false,
        },
    )
}

pub(crate) fn verify_unlinked_eval_tree_with_profile_and_arguments(
    function: &UnlinkedFunction,
    kind: EvalKind,
    caller_strict: bool,
    expected_bindings: &[EvalRootBinding<JsString>],
    expected_profile: &EvalCallerProfile,
    expected_capabilities: EvalPublicationCapabilities,
) -> Result<(), RuntimeError> {
    if kind == EvalKind::None {
        return Err(RuntimeError::Engine(Error::internal(
            "eval publication requires a direct or indirect eval kind",
        )));
    }
    if kind == EvalKind::Indirect && !expected_bindings.is_empty() {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received caller bindings",
        )));
    }
    if kind == EvalKind::Indirect && caller_strict {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received caller strictness",
        )));
    }
    if expected_capabilities.super_call_allowed && !expected_capabilities.super_allowed {
        return Err(RuntimeError::Engine(Error::internal(
            "eval publication permits super() without SuperProperty",
        )));
    }
    if kind == EvalKind::Indirect
        && (expected_capabilities.super_call_allowed || expected_capabilities.super_allowed)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received a super capability",
        )));
    }
    if kind == EvalKind::Indirect
        && (!expected_profile.scope_kinds.is_empty()
            || expected_profile.variable_target != EvalCallerVariableTarget::Global)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "indirect eval publication received a caller scope profile",
        )));
    }
    for binding in expected_bindings {
        let Some(&scope_kind) = expected_profile.scope_kinds.get(usize::from(binding.scope)) else {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root binding disagrees with its caller scope profile",
            )));
        };
        if (binding.is_catch_parameter
            && scope_kind != crate::engine::code::function::metadata::EvalScopeKind::Catch)
            || (binding.kind == ClosureVariableKind::WithObject)
                != (scope_kind == crate::engine::code::function::metadata::EvalScopeKind::With)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root binding disagrees with its caller scope profile",
            )));
        }
        if binding.name.is_empty() {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root binding has an empty name",
            )));
        }
        if binding.is_catch_parameter
            && (!binding.is_lexical
                || binding.is_const
                || binding.kind != ClosureVariableKind::Normal)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root catch binding has invalid binding metadata",
            )));
        }
        if binding.kind.is_eval_variable_object() {
            let role_allowed = match scope_kind {
                crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot => true,
                crate::engine::code::function::metadata::EvalScopeKind::Parameter => {
                    binding.kind == ClosureVariableKind::ArgEvalVariableObject
                }
                _ => false,
            };
            if !role_allowed
                || binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || eval_variable_object_sentinel(binding.kind)
                    .is_none_or(|sentinel| binding.name.utf16_units().ne(sentinel.encode_utf16()))
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval root variable-object binding has invalid binding metadata",
                )));
            }
        }
        if binding.kind == ClosureVariableKind::WithObject
            && (binding.is_lexical
                || binding.is_const
                || binding.is_catch_parameter
                || binding.name.utf16_units().ne("<with>".encode_utf16()))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root with-object binding has invalid binding metadata",
            )));
        }
        if binding.kind == ClosureVariableKind::GlobalFunction {
            return Err(RuntimeError::Engine(Error::internal(
                "eval root imported a declaration-only global binding kind",
            )));
        }
    }
    if expected_profile
        .scope_kinds
        .iter()
        .enumerate()
        .any(|(scope, kind)| {
            *kind == crate::engine::code::function::metadata::EvalScopeKind::With
                && expected_bindings
                    .iter()
                    .filter(|binding| usize::from(binding.scope) == scope)
                    .count()
                    != 1
        })
    {
        return Err(RuntimeError::Engine(Error::internal(
            "eval root with scope does not contain exactly one object binding",
        )));
    }
    let has_variable_object = expected_bindings
        .iter()
        .any(|binding| binding.kind.is_eval_variable_object());
    match (caller_strict, expected_profile.variable_target) {
        (false, EvalCallerVariableTarget::Global) if !has_variable_object => {}
        (true, EvalCallerVariableTarget::StrictLocal) if kind == EvalKind::Direct => {}
        (false, EvalCallerVariableTarget::ExternalBinding(index))
            if expected_bindings
                .get(usize::from(index))
                .is_some_and(|binding| {
                    let target_role_matches = expected_profile
                        .scope_kinds
                        .get(usize::from(binding.scope))
                        .is_some_and(|scope_kind| match scope_kind {
                            crate::engine::code::function::metadata::EvalScopeKind::FunctionRoot => {
                                binding.kind == ClosureVariableKind::EvalVariableObject
                            }
                            crate::engine::code::function::metadata::EvalScopeKind::Parameter => {
                                binding.kind == ClosureVariableKind::ArgEvalVariableObject
                            }
                            _ => false,
                        });
                    target_role_matches
                        && !binding.is_lexical
                        && !binding.is_const
                        && !binding.is_catch_parameter
                }) => {}
        _ => {
            return Err(RuntimeError::Engine(Error::internal(
                "eval caller variable target is not authenticated",
            )));
        }
    }
    verify_unlinked_tree_with_root(
        function,
        RootPublication::Eval {
            kind,
            caller_strict,
            expected_bindings,
            expected_profile,
            expected_capabilities,
        },
    )
}

/// Provenance travels with a function through the iterative publication walk.
/// Each array is indexed by that function's closure descriptor index.
struct PublicationClosureOrigins {
    eval_bindings: Vec<Option<u16>>,
    function_names: Vec<Option<bool>>,
    derived_this: Vec<bool>,
    active_function: Vec<bool>,
    new_target: Vec<bool>,
}

struct PublicationWorkItem<'a> {
    function: &'a UnlinkedFunction,
    depth: usize,
    origins: PublicationClosureOrigins,
    id: usize,
}

fn verify_unlinked_tree_with_root(
    function: &UnlinkedFunction,
    root_publication: RootPublication<'_>,
) -> Result<(), RuntimeError> {
    let (synthetic_eval_tree, tree_expected_bindings, tree_expected_profile) =
        match root_publication {
            RootPublication::Script
            | RootPublication::TrustedOrdinaryLeaf
            | RootPublication::Module(_) => (false, &[][..], None),
            RootPublication::Eval {
                expected_bindings,
                expected_profile,
                ..
            } => (true, expected_bindings, Some(expected_profile)),
        };
    let root_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => Some(index),
            _ => None,
        })
        .collect::<Vec<_>>();
    // `expected_bindings` preserves the caller's inner-to-outer resolution
    // order. A same-shaped pseudo binding in an outer function segment is
    // shadowed and must not gain capability merely by matching the sentinel.
    let (root_derived_this_origin, root_active_function_origin, root_new_target_origin) =
        match root_publication {
            RootPublication::Eval {
                kind: EvalKind::Direct,
                expected_bindings,
                expected_capabilities,
                ..
            } if expected_capabilities.super_call_allowed => (
                expected_bindings
                    .iter()
                    .position(eval_root_binding_is_derived_this),
                expected_bindings.iter().position(|binding| {
                    eval_root_binding_is_super_pseudo(binding, "<this_active_func>")
                }),
                expected_bindings
                    .iter()
                    .position(|binding| eval_root_binding_is_super_pseudo(binding, "<new.target>")),
            ),
            _ => (None, None, None),
        };
    let root_derived_this_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => {
                root_derived_this_origin == Some(usize::from(index))
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    let root_active_function_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => {
                root_active_function_origin == Some(usize::from(index))
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    let root_new_target_origins = function
        .closure_variables()
        .iter()
        .map(|descriptor| match descriptor.source {
            ClosureSource::EvalEnvironment(index) => {
                root_new_target_origin == Some(usize::from(index))
            }
            _ => false,
        })
        .collect::<Vec<_>>();
    // Synthetic Eval roots authenticate imported descriptors directly and do
    // not expose the ordinary-function `add_eval_variables` metadata quirk.
    let root_function_name_origins = vec![None; function.closure_variables().len()];
    let mut next_function_id = 1_usize;
    let mut erased_function_name_slots = HashSet::<(usize, u16)>::new();
    let mut eval_consumed_erased_slots = HashSet::<(usize, u16)>::new();
    let mut erased_parent_by_child = HashMap::<(usize, u16), (usize, u16)>::new();
    let mut pending = vec![PublicationWorkItem {
        function,
        depth: 0,
        origins: PublicationClosureOrigins {
            eval_bindings: root_origins,
            function_names: root_function_name_origins,
            derived_this: root_derived_this_origins,
            active_function: root_active_function_origins,
            new_target: root_new_target_origins,
        },
        id: 0,
    }];
    while let Some(PublicationWorkItem {
        function,
        depth: function_depth,
        origins:
            PublicationClosureOrigins {
                eval_bindings: closure_origins,
                function_names: function_name_origins,
                derived_this: derived_this_origins,
                active_function: active_function_origins,
                new_target: new_target_origins,
            },
        id: function_id,
    }) = pending.pop()
    {
        let is_root = function_depth == 0;
        if is_root && function.metadata().class_initializer_kind.is_some() {
            return Err(RuntimeError::Engine(Error::internal(
                "class initializer bytecode escaped the class publication tree",
            )));
        }
        let arg_eval_variable_object_local = function
            .parameter_environment()
            .and_then(|layout| layout.arg_eval_variable_object_local);
        roles::verify(function, is_root, root_publication)?;
        let expected_eval_bindings = if is_root {
            match root_publication {
                RootPublication::Script
                | RootPublication::TrustedOrdinaryLeaf
                | RootPublication::Module(_) => None,
                RootPublication::Eval {
                    expected_bindings, ..
                } => Some(expected_bindings),
            }
        } else {
            None
        };
        let parameters::ParameterPublicationLayout {
            parameter_initializer_locals,
            parameter_body_pc,
            pattern_body_pc,
            parameter_initializer_capture_locals,
        } = parameters::verify(function, is_root)?;
        bindings::verify_metadata(function, is_root, arg_eval_variable_object_local)?;
        private_elements::verify_unlinked(function)?;
        let unnamed_arguments = function
            .argument_definitions()
            .iter()
            .map(|definition| definition.name.is_none())
            .collect::<Vec<_>>();
        let lexical_locals = function
            .local_definitions()
            .iter()
            .map(|definition| definition.is_lexical)
            .collect::<Vec<_>>();
        let const_locals = function
            .local_definitions()
            .iter()
            .map(|definition| definition.is_const)
            .collect::<Vec<_>>();
        validate_derived_constructor_bytecode_layout(
            function.metadata(),
            function.code(),
            &lexical_locals,
            &const_locals,
            function.closure_variables(),
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        validate_class_initializer_bytecode_layout(function.metadata(), function.code())
            .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        validate_parameter_initializer_scope_layout(
            function.metadata(),
            function.code(),
            parameter_body_pc.or(pattern_body_pc),
            &lexical_locals,
            &parameter_initializer_locals,
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        validate_pattern_parameter_bytecode_layout(
            function.metadata(),
            function.code(),
            &unnamed_arguments,
            &lexical_locals,
            &parameter_initializer_locals,
            function.parameter_environment(),
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;
        bindings::verify_definitions(function, arg_eval_variable_object_local)?;
        if function.closure_variables().len() != usize::from(function.metadata().closure_count) {
            return Err(RuntimeError::Engine(Error::internal(
                "function closure descriptor count does not match bytecode metadata",
            )));
        }
        if derived_this_origins.len() != function.closure_variables().len() {
            return Err(RuntimeError::Invariant(
                "derived-this provenance count disagrees with closure descriptors",
            ));
        }
        if active_function_origins.len() != function.closure_variables().len()
            || new_target_origins.len() != function.closure_variables().len()
        {
            return Err(RuntimeError::Invariant(
                "super-call provenance count disagrees with closure descriptors",
            ));
        }
        for instruction in function.code() {
            let crate::engine::code::bytecode::Instruction::InitializeDerivedVarRef(index) =
                instruction
            else {
                continue;
            };
            let descriptor = function
                .closure_variables()
                .get(usize::from(*index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "captured derived initializer is outside closure slots",
                    ))
                })?;
            let name = unlinked_closure_name(function, descriptor)?;
            if name.is_none_or(|name| name.utf16_units().ne("<this>".encode_utf16())) {
                return Err(RuntimeError::Engine(Error::internal(
                    "captured derived initializer lost its this-binding provenance",
                )));
            }
            if !derived_this_origins
                .get(usize::from(*index))
                .copied()
                .unwrap_or(false)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "captured derived initializer did not originate from derived this",
                )));
            }
        }
        for instruction in function.code() {
            let crate::engine::code::bytecode::Instruction::InitializeVarRef(index) = instruction
            else {
                continue;
            };
            if !is_root || !matches!(root_publication, RootPublication::Module(_)) {
                return Err(RuntimeError::Engine(Error::internal(
                    "module lexical initializer escaped a module root",
                )));
            }
            let descriptor = function
                .closure_variables()
                .get(usize::from(*index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "module lexical initializer is outside closure slots",
                    ))
                })?;
            if descriptor.source != ClosureSource::ModuleDeclaration
                || !descriptor.is_lexical
                || descriptor.kind != ClosureVariableKind::Normal
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "module lexical initializer targeted a non-declaration binding",
                )));
            }
        }
        for instruction in function.code() {
            let crate::engine::code::bytecode::Instruction::InitializeModuleImportCollision(index) =
                instruction
            else {
                continue;
            };
            let RootPublication::Module(module) = root_publication else {
                return Err(RuntimeError::Engine(Error::internal(
                    "module import collision initializer escaped a module root",
                )));
            };
            if !is_root
                || !module.import_collisions().iter().any(|collision| {
                    collision.closure_index == *index
                        && matches!(
                            collision.declaration,
                            ModuleImportCollisionDeclaration::Lexical
                                | ModuleImportCollisionDeclaration::Function
                        )
                })
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "module import collision initializer has no matching ledger entry",
                )));
            }
            let descriptor = function
                .closure_variables()
                .get(usize::from(*index))
                .ok_or_else(|| {
                    RuntimeError::Engine(Error::internal(
                        "module import collision initializer is outside closure slots",
                    ))
                })?;
            if descriptor.source != ClosureSource::ModuleImportCollision
                || !descriptor.is_lexical
                || !descriptor.is_const
                || !matches!(
                    descriptor.kind,
                    ClosureVariableKind::Normal | ClosureVariableKind::ModuleImportView
                )
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "module import collision initializer targeted a non-import binding",
                )));
            }
        }
        verify_unlinked_debug(function)?;
        if function.closure_variables().iter().any(|descriptor| {
            descriptor.is_const
                && !descriptor.is_lexical
                && descriptor.kind != ClosureVariableKind::FunctionName
        }) {
            return Err(RuntimeError::Engine(Error::internal(
                "a const closure descriptor must also be lexical",
            )));
        }
        if function_name_origins.len() != function.closure_variables().len() {
            return Err(RuntimeError::Invariant(
                "function-name provenance count disagrees with closure descriptors",
            ));
        }
        for (index, (descriptor, origin)) in function
            .closure_variables()
            .iter()
            .zip(&function_name_origins)
            .enumerate()
        {
            let Some(is_const) = origin else {
                continue;
            };
            if !function_name_view_matches_origin(*descriptor, *is_const) {
                return Err(RuntimeError::Engine(Error::internal(
                    "closure descriptor lost its ordinary FunctionName provenance",
                )));
            }
            if is_erased_function_name_view(*descriptor) {
                let index = u16::try_from(index).map_err(|_| {
                    RuntimeError::Engine(Error::internal(
                        "closure descriptor index exceeds bytecode range",
                    ))
                })?;
                erased_function_name_slots.insert((function_id, index));
            }
        }
        let declarations =
            closures::verify(function, is_root, root_publication, expected_eval_bindings)?;
        let flow::PublicationFlow {
            global_function_initializer_pcs,
            authenticated_active_function_local,
            authenticated_new_target_local,
            masked_lexical_initializer_targets,
            child_closure_pcs,
        } = flow::verify(
            function,
            declarations,
            &active_function_origins,
            &new_target_origins,
        )?;
        validate_eval_environment_phase_layout(
            function.eval_environments(),
            EvalEnvironmentPhaseContext {
                metadata: function.metadata(),
                code: function.code(),
                parameter_body_pc,
                pattern_body_pc,
                lexical_locals: &lexical_locals,
                parameter_initializer_locals: &parameter_initializer_locals,
                parameter_initializer_visible_locals: parameter_initializer_capture_locals
                    .as_deref(),
                parameter_environment: function.parameter_environment(),
            },
        )
        .map_err(|message| RuntimeError::Engine(Error::internal(message)))?;

        let mut captured_locals = vec![false; usize::from(function.metadata().local_count)];
        for (constant_index, child) in function
            .constants()
            .iter()
            .enumerate()
            .filter_map(|(index, constant)| constant.as_child().map(|child| (index, child)))
        {
            if child_closure_pcs[constant_index].is_empty() {
                continue;
            }
            for descriptor in child.closure_variables() {
                if let ClosureSource::ParentLocal(index) = descriptor.source {
                    if let Some(captured) = captured_locals.get_mut(usize::from(index)) {
                        *captured = true;
                    }
                }
            }
        }
        verify_eval_environments(
            function,
            function_depth,
            &mut captured_locals,
            synthetic_eval_tree,
            &closure_origins,
            tree_expected_bindings,
            tree_expected_profile,
        )?;
        for environment in function
            .eval_environments()
            .iter()
            // Ordinary direct eval also carries compiler-private `<this>` and
            // `new.target` spellings. They become derived-constructor
            // capabilities only when this exact call site inherited super()
            // authority; super-property-only object methods must not be
            // mistaken for derived constructors.
            .filter(|environment| environment.super_call_allowed)
        {
            verify_eval_super_pseudo_bindings(
                environment,
                function.metadata().derived_this_local,
                authenticated_active_function_local,
                authenticated_new_target_local,
                &derived_this_origins,
                &active_function_origins,
                &new_target_origins,
            )?;
        }
        for binding in function
            .eval_environments()
            .iter()
            .flat_map(|environment| environment.scopes.iter())
            .flat_map(|scope| scope.bindings.iter())
        {
            let crate::engine::code::function::metadata::EvalBindingSource::Closure(index) =
                binding.source
            else {
                continue;
            };
            if function_name_origins
                .get(usize::from(index))
                .is_some_and(Option::is_some)
                && function
                    .closure_variables()
                    .get(usize::from(index))
                    .is_some_and(|descriptor| is_erased_function_name_view(*descriptor))
                && !binding.is_lexical
                && !binding.is_const
                && binding.kind == ClosureVariableKind::Normal
            {
                eval_consumed_erased_slots.insert((function_id, index));
            }
        }
        for (index, (descriptor, origin)) in function
            .closure_variables()
            .iter()
            .zip(&function_name_origins)
            .enumerate()
        {
            if origin.is_none() || !is_erased_function_name_view(*descriptor) {
                continue;
            }
            let index = u16::try_from(index).map_err(|_| {
                RuntimeError::Engine(Error::internal(
                    "closure descriptor index exceeds bytecode range",
                ))
            })?;
            if eval_consumed_erased_slots.contains(&(function_id, index)) {
                // A function's own eval prepass runs before every child.
                continue;
            }
            let first_child_view = function
                .constants()
                .iter()
                .enumerate()
                .filter(|(constant_index, _)| !child_closure_pcs[*constant_index].is_empty())
                .filter_map(|(_, constant)| constant.as_child())
                .find_map(|child| {
                    child
                        .closure_variables()
                        .iter()
                        .find(|candidate| candidate.source == ClosureSource::ParentClosure(index))
                        .copied()
                });
            if first_child_view.is_none_or(|view| !is_erased_function_name_view(view)) {
                return Err(RuntimeError::Engine(Error::internal(
                    "erased FunctionName closure was not the first source request",
                )));
            }
        }

        operands::verify(
            function,
            &captured_locals,
            &global_function_initializer_pcs,
            &masked_lexical_initializer_targets,
        )?;
        children::verify_and_enqueue(
            children::ChildPublicationInputs {
                function,
                function_id,
                authenticated_active_function_local,
                authenticated_new_target_local,
                function_depth,
                child_closure_pcs: &child_closure_pcs,
                parameter_body_pc,
                pattern_body_pc,
                parameter_initializer_capture_locals: parameter_initializer_capture_locals
                    .as_deref(),
                function_name_origins: &function_name_origins,
                derived_this_origins: &derived_this_origins,
                active_function_origins: &active_function_origins,
                new_target_origins: &new_target_origins,
                closure_origins: &closure_origins,
            },
            children::ChildPublicationQueue {
                pending: &mut pending,
                next_function_id: &mut next_function_id,
                erased_parent_by_child: &mut erased_parent_by_child,
            },
        )?;
    }
    let mut authenticated_erased_slots = HashSet::new();
    let mut lineage = eval_consumed_erased_slots.into_iter().collect::<Vec<_>>();
    while let Some(slot) = lineage.pop() {
        if !authenticated_erased_slots.insert(slot) {
            continue;
        }
        if let Some(parent) = erased_parent_by_child.get(&slot).copied() {
            lineage.push(parent);
        }
    }
    if erased_function_name_slots
        .iter()
        .any(|slot| !authenticated_erased_slots.contains(slot))
    {
        return Err(RuntimeError::Engine(Error::internal(
            "erased FunctionName closure has no direct-eval lineage",
        )));
    }
    Ok(())
}

fn verify_unlinked_debug(function: &UnlinkedFunction) -> Result<(), RuntimeError> {
    let Some(debug) = function.debug() else {
        return Ok(());
    };
    let Some(table) = &debug.pc2line else {
        return Ok(());
    };
    if table.definition.line == u32::MAX || table.definition.column == u32::MAX {
        return Err(RuntimeError::Engine(Error::internal(
            "bytecode debug definition position cannot be represented one-based",
        )));
    }
    let mut previous_pc = None;
    for entry in &table.entries {
        if usize::try_from(entry.pc)
            .ok()
            .is_none_or(|pc| pc >= function.code().len())
        {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug PC is outside the instruction stream",
            )));
        }
        if previous_pc.is_some_and(|previous| entry.pc < previous) {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug PCs are not ordered",
            )));
        }
        if entry.position.line == u32::MAX || entry.position.column == u32::MAX {
            return Err(RuntimeError::Engine(Error::internal(
                "bytecode debug position cannot be represented one-based",
            )));
        }
        previous_pc = Some(entry.pc);
    }
    Ok(())
}

fn function_name_view_matches_origin(descriptor: ClosureVariable, origin_is_const: bool) -> bool {
    (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
        == (false, origin_is_const, ClosureVariableKind::FunctionName)
        || is_erased_function_name_view(descriptor)
}

fn is_erased_function_name_view(descriptor: ClosureVariable) -> bool {
    (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
        == (false, false, ClosureVariableKind::Normal)
}

fn verify_capture_flags(
    previous: &mut Option<(bool, bool, ClosureVariableKind)>,
    current: (bool, bool, ClosureVariableKind),
) -> Result<(), RuntimeError> {
    if previous.is_some_and(|previous| previous != current) {
        return Err(RuntimeError::Engine(Error::internal(
            "sibling closure descriptors disagree about one parent binding",
        )));
    }
    *previous = Some(current);
    Ok(())
}

#[cfg(test)]
mod tests;
