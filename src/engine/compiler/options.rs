//! Source compilation request options; publication gets its own input contract.

use crate::engine::code::function::metadata::{
    ClosureVariableKind, EvalCallerProfile, EvalCallerVariableTarget, EvalKind, EvalRootBinding,
    EvalScopeKind,
};
use crate::engine::value::JsString;

/// Default filename used by the Rust convenience compile/eval APIs.
pub const DEFAULT_EVAL_FILENAME: &str = "<input>";

/// Named source compilation options.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompileOptions {
    pub filename: String,
}

impl CompileOptions {
    #[must_use]
    pub fn new(filename: impl Into<String>) -> Self {
        Self {
            filename: filename.into(),
        }
    }
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self::new(DEFAULT_EVAL_FILENAME)
    }
}

/// Internal compilation context for one synthetic eval root.
///
/// Direct eval imports the exact live caller bindings described by R1w.
/// Indirect eval has no external bindings and resolves against the defining
/// realm's global environment. `caller_strict` is ignored for indirect eval,
/// matching QuickJS's `JS_EVAL_TYPE_INDIRECT` path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvalCompileContext {
    pub kind: EvalKind,
    pub caller_strict: bool,
    pub bindings: Box<[EvalRootBinding<JsString>]>,
    pub caller_profile: EvalCallerProfile,
    pub super_call_allowed: bool,
    pub super_allowed: bool,
    pub arguments_forbidden: bool,
}

impl EvalCompileContext {
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn direct(caller_strict: bool, bindings: Vec<EvalRootBinding<JsString>>) -> Self {
        let scope_count = bindings
            .iter()
            .map(|binding| usize::from(binding.scope) + 1)
            .max()
            .unwrap_or(0);
        let mut scope_kinds = vec![EvalScopeKind::FunctionRoot; scope_count];
        for binding in &bindings {
            if binding.kind == ClosureVariableKind::WithObject {
                scope_kinds[usize::from(binding.scope)] = EvalScopeKind::With;
            } else if binding.is_catch_parameter {
                scope_kinds[usize::from(binding.scope)] = EvalScopeKind::Catch;
            }
        }
        for (scope, scope_kind) in scope_kinds.iter_mut().enumerate() {
            let has_parameter_object = bindings.iter().any(|binding| {
                usize::from(binding.scope) == scope
                    && binding.kind == ClosureVariableKind::ArgEvalVariableObject
            });
            let has_body_object = bindings.iter().any(|binding| {
                usize::from(binding.scope) == scope
                    && binding.kind == ClosureVariableKind::EvalVariableObject
            });
            if has_parameter_object && !has_body_object {
                *scope_kind = EvalScopeKind::Parameter;
            }
        }
        let variable_target = if caller_strict {
            EvalCallerVariableTarget::StrictLocal
        } else {
            bindings
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
        Self::direct_with_profile(
            caller_strict,
            bindings,
            EvalCallerProfile {
                scope_kinds: scope_kinds.into_boxed_slice(),
                variable_target,
            },
            false,
            false,
        )
    }

    pub fn direct_with_profile(
        caller_strict: bool,
        bindings: Vec<EvalRootBinding<JsString>>,
        caller_profile: EvalCallerProfile,
        super_call_allowed: bool,
        super_allowed: bool,
    ) -> Self {
        Self {
            kind: EvalKind::Direct,
            caller_strict,
            bindings: bindings.into_boxed_slice(),
            caller_profile,
            super_call_allowed,
            super_allowed,
            arguments_forbidden: false,
        }
    }

    pub fn direct_with_profile_and_arguments(
        caller_strict: bool,
        bindings: Vec<EvalRootBinding<JsString>>,
        caller_profile: EvalCallerProfile,
        super_call_allowed: bool,
        super_allowed: bool,
        arguments_forbidden: bool,
    ) -> Self {
        let mut context = Self::direct_with_profile(
            caller_strict,
            bindings,
            caller_profile,
            super_call_allowed,
            super_allowed,
        );
        context.arguments_forbidden = arguments_forbidden;
        context
    }

    pub fn indirect() -> Self {
        Self {
            kind: EvalKind::Indirect,
            caller_strict: false,
            bindings: Box::new([]),
            caller_profile: EvalCallerProfile {
                scope_kinds: Box::new([]),
                variable_target: EvalCallerVariableTarget::Global,
            },
            super_call_allowed: false,
            super_allowed: false,
            arguments_forbidden: false,
        }
    }
}
