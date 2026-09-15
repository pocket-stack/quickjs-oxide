//! Owned verification result. The draft cannot change between authentication
//! and publication, and callers cannot substitute a different function.

use super::{
    EvalPublicationCapabilities, verify_unlinked_eval_tree_with_profile_and_arguments,
    verify_unlinked_module_tree, verify_unlinked_ordinary_leaf, verify_unlinked_tree,
};
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::publication::EvalPublicationInput;
use crate::engine::code::module::UnlinkedModule;
use crate::engine::code::module::UnlinkedModuleParts;

pub(crate) struct VerifiedFunction(UnlinkedFunction);

impl VerifiedFunction {
    pub(crate) fn script(function: UnlinkedFunction) -> Result<Self, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Verify,
        );
        verify_unlinked_tree(&function)?;
        Ok(Self(function))
    }

    pub(in crate::engine::code) fn ordinary_leaf(
        function: UnlinkedFunction,
    ) -> Result<Self, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Verify,
        );
        verify_unlinked_ordinary_leaf(&function)?;
        Ok(Self(function))
    }

    pub(crate) fn eval(
        function: UnlinkedFunction,
        expected: EvalPublicationInput<'_>,
    ) -> Result<Self, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Verify,
        );
        verify_unlinked_eval_tree_with_profile_and_arguments(
            &function,
            expected.kind,
            expected.caller_strict,
            expected.bindings,
            expected.caller_profile,
            EvalPublicationCapabilities {
                super_call_allowed: expected.super_call_allowed,
                super_allowed: expected.super_allowed,
                arguments_forbidden: expected.arguments_forbidden,
            },
        )?;
        Ok(Self(function))
    }

    /// Keep module tables paired with the exact function they authenticated.
    /// The generic parts carrier changes only the function's ownership state.
    pub(crate) fn module(
        module: UnlinkedModule,
    ) -> Result<UnlinkedModuleParts<Self>, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Verify,
        );
        verify_unlinked_module_tree(&module)?;
        let UnlinkedModuleParts {
            name,
            function,
            has_top_level_await,
            declaration_order,
            link_initializers,
            import_collisions,
            requested_modules,
            imports,
            exports,
            star_exports,
        } = module.into_parts();
        Ok(UnlinkedModuleParts {
            name,
            function: Self(function),
            has_top_level_await,
            declaration_order,
            link_initializers,
            import_collisions,
            requested_modules,
            imports,
            exports,
            star_exports,
        })
    }

    pub(in crate::engine::code) fn into_function(self) -> UnlinkedFunction {
        self.0
    }
}
