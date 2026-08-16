//! Shared bounded accounting for whole BC5 bytecode images.
//!
//! Decode and encode deliberately use the same totals, remaining-budget
//! intersection, and error-attribution rules. This keeps a stricter write
//! policy from reporting a whole-image limit where the equivalent reader
//! would report the narrower per-function limit (or vice versa).

use std::fmt;

use super::super::code::{CodeError, CodeResourceKind};
use super::super::function_envelope::{
    FunctionEnvelopeError, FunctionEnvelopeLimits, FunctionResourceKind,
};
use super::super::graph::model::GraphLimits;

/// Per-module limits for each variable-length BC5 Module table.
///
/// There is intentionally no `Default`: every bytecode-image admission policy
/// must choose these four caps explicitly.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct ModuleLimits {
    max_requests: usize,
    max_exports: usize,
    max_star_exports: usize,
    max_imports: usize,
}

impl ModuleLimits {
    #[must_use]
    pub(in crate::runtime) const fn new(
        max_requests: usize,
        max_exports: usize,
        max_star_exports: usize,
        max_imports: usize,
    ) -> Self {
        Self {
            max_requests,
            max_exports,
            max_star_exports,
            max_imports,
        }
    }

    /// Intersect one record's limits with the remaining aggregate image
    /// budget. The returned limits can be shared by decoder and encoder.
    #[must_use]
    pub(super) const fn intersect(
        self,
        max_requests: usize,
        max_exports: usize,
        max_star_exports: usize,
        max_imports: usize,
    ) -> Self {
        Self {
            max_requests: minimum(self.max_requests, max_requests),
            max_exports: minimum(self.max_exports, max_exports),
            max_star_exports: minimum(self.max_star_exports, max_star_exports),
            max_imports: minimum(self.max_imports, max_imports),
        }
    }

    pub(super) const fn limit(self, kind: ModuleResourceKind) -> usize {
        match kind {
            ModuleResourceKind::Requests => self.max_requests,
            ModuleResourceKind::Exports => self.max_exports,
            ModuleResourceKind::StarExports => self.max_star_exports,
            ModuleResourceKind::Imports => self.max_imports,
        }
    }

    pub(super) fn check(
        self,
        kind: ModuleResourceKind,
        requested: usize,
    ) -> Result<(), ModuleBudgetError> {
        let limit = self.limit(kind);
        if requested > limit {
            return Err(ModuleBudgetError::ResourceLimit {
                kind,
                requested,
                limit,
            });
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ModuleResourceKind {
    Requests,
    Exports,
    StarExports,
    Imports,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ModuleBudgetError {
    ResourceLimit {
        kind: ModuleResourceKind,
        requested: usize,
        limit: usize,
    },
}

impl fmt::Display for ModuleBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimit {
                kind,
                requested,
                limit,
            } => write!(
                formatter,
                "{kind:?} per-module resource limit exceeded: requested {requested}, limit {limit}"
            ),
        }
    }
}

impl std::error::Error for ModuleBudgetError {}

/// Aggregate whole-image limits in addition to the per-value graph and
/// per-record function and module limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct BytecodeImageLimits {
    graph: GraphLimits,
    envelope: FunctionEnvelopeLimits,
    module: ModuleLimits,
    max_functions: usize,
    max_modules: usize,
    max_whole_depth: usize,
    max_total_constant_pool_entries: usize,
    max_total_local_variables: usize,
    max_total_closure_variables: usize,
    max_total_code_bytes: usize,
    max_total_instructions: usize,
    max_total_atom_relocations: usize,
    max_total_debug_bytes: usize,
    max_total_module_requests: usize,
    max_total_module_exports: usize,
    max_total_module_star_exports: usize,
    max_total_module_imports: usize,
}

impl BytecodeImageLimits {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub(in crate::runtime) const fn new(
        graph: GraphLimits,
        envelope: FunctionEnvelopeLimits,
        module: ModuleLimits,
        max_functions: usize,
        max_modules: usize,
        max_whole_depth: usize,
        max_total_constant_pool_entries: usize,
        max_total_local_variables: usize,
        max_total_closure_variables: usize,
        max_total_code_bytes: usize,
        max_total_instructions: usize,
        max_total_atom_relocations: usize,
        max_total_debug_bytes: usize,
        max_total_module_requests: usize,
        max_total_module_exports: usize,
        max_total_module_star_exports: usize,
        max_total_module_imports: usize,
    ) -> Self {
        Self {
            graph,
            envelope,
            module,
            max_functions,
            max_modules,
            max_whole_depth,
            max_total_constant_pool_entries,
            max_total_local_variables,
            max_total_closure_variables,
            max_total_code_bytes,
            max_total_instructions,
            max_total_atom_relocations,
            max_total_debug_bytes,
            max_total_module_requests,
            max_total_module_exports,
            max_total_module_star_exports,
            max_total_module_imports,
        }
    }

    pub(super) const fn limit(self, kind: BytecodeImageResourceKind) -> usize {
        match kind {
            BytecodeImageResourceKind::Functions => self.max_functions,
            BytecodeImageResourceKind::Modules => self.max_modules,
            BytecodeImageResourceKind::WholeDepth => self.max_whole_depth,
            BytecodeImageResourceKind::TotalConstantPoolEntries => {
                self.max_total_constant_pool_entries
            }
            BytecodeImageResourceKind::TotalLocalVariables => self.max_total_local_variables,
            BytecodeImageResourceKind::TotalClosureVariables => self.max_total_closure_variables,
            BytecodeImageResourceKind::TotalCodeBytes => self.max_total_code_bytes,
            BytecodeImageResourceKind::TotalInstructions => self.max_total_instructions,
            BytecodeImageResourceKind::TotalAtomRelocations => self.max_total_atom_relocations,
            BytecodeImageResourceKind::TotalDebugBytes => self.max_total_debug_bytes,
            BytecodeImageResourceKind::TotalModuleRequests => self.max_total_module_requests,
            BytecodeImageResourceKind::TotalModuleExports => self.max_total_module_exports,
            BytecodeImageResourceKind::TotalModuleStarExports => self.max_total_module_star_exports,
            BytecodeImageResourceKind::TotalModuleImports => self.max_total_module_imports,
        }
    }

    pub(super) fn check(
        self,
        kind: BytecodeImageResourceKind,
        requested: usize,
    ) -> Result<(), BytecodeImageBudgetError> {
        let limit = self.limit(kind);
        if requested > limit {
            return Err(BytecodeImageBudgetError::ResourceLimit {
                kind,
                requested,
                limit,
            });
        }
        Ok(())
    }

    #[must_use]
    pub(super) const fn graph(self) -> GraphLimits {
        self.graph
    }

    #[must_use]
    pub(super) const fn envelope(self) -> FunctionEnvelopeLimits {
        self.envelope
    }

    #[must_use]
    pub(super) const fn module(self) -> ModuleLimits {
        self.module
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BytecodeImageResourceKind {
    Functions,
    Modules,
    WholeDepth,
    TotalConstantPoolEntries,
    TotalLocalVariables,
    TotalClosureVariables,
    TotalCodeBytes,
    TotalInstructions,
    TotalAtomRelocations,
    TotalDebugBytes,
    TotalModuleRequests,
    TotalModuleExports,
    TotalModuleStarExports,
    TotalModuleImports,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BytecodeImageBudgetError {
    ResourceLimit {
        kind: BytecodeImageResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        kind: BytecodeImageResourceKind,
    },
}

impl fmt::Display for BytecodeImageBudgetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ResourceLimit {
                kind,
                requested,
                limit,
            } => write!(
                formatter,
                "{kind:?} whole-image resource limit exceeded: requested {requested}, limit {limit}"
            ),
            Self::CountOverflow { kind } => {
                write!(formatter, "{kind:?} whole-image resource count overflowed")
            }
        }
    }
}

impl std::error::Error for BytecodeImageBudgetError {}

/// Aggregate module-table usage already committed to one image.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct ModuleTotals {
    requests: usize,
    exports: usize,
    star_exports: usize,
    imports: usize,
}

/// A staged contribution from one or more tables in a module record.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ModuleUsage {
    requests: usize,
    exports: usize,
    star_exports: usize,
    imports: usize,
}

impl ModuleUsage {
    #[must_use]
    pub(super) const fn new(
        requests: usize,
        exports: usize,
        star_exports: usize,
        imports: usize,
    ) -> Self {
        Self {
            requests,
            exports,
            star_exports,
            imports,
        }
    }
}

/// Remaining aggregate room projected into one record's four table counts.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct RemainingModuleBudget {
    requests: usize,
    exports: usize,
    star_exports: usize,
    imports: usize,
}

impl RemainingModuleBudget {
    #[must_use]
    pub(super) const fn intersect(self, limits: ModuleLimits) -> ModuleLimits {
        limits.intersect(self.requests, self.exports, self.star_exports, self.imports)
    }
}

impl ModuleTotals {
    /// Compute the aggregate room available before the next table is inspected.
    ///
    /// A subtraction failure means an earlier accounting invariant was
    /// violated; it is reported instead of wrapping `usize`.
    pub(super) fn remaining(
        self,
        limits: BytecodeImageLimits,
    ) -> Result<RemainingModuleBudget, BytecodeImageBudgetError> {
        Ok(RemainingModuleBudget {
            requests: checked_remaining(
                self.requests,
                BytecodeImageResourceKind::TotalModuleRequests,
                limits,
            )?,
            exports: checked_remaining(
                self.exports,
                BytecodeImageResourceKind::TotalModuleExports,
                limits,
            )?,
            star_exports: checked_remaining(
                self.star_exports,
                BytecodeImageResourceKind::TotalModuleStarExports,
                limits,
            )?,
            imports: checked_remaining(
                self.imports,
                BytecodeImageResourceKind::TotalModuleImports,
                limits,
            )?,
        })
    }

    /// Commit one staged table contribution with checked aggregate arithmetic.
    pub(super) fn checked_add(
        self,
        usage: ModuleUsage,
        limits: BytecodeImageLimits,
    ) -> Result<Self, BytecodeImageBudgetError> {
        Ok(Self {
            requests: checked_total(
                self.requests,
                usage.requests,
                BytecodeImageResourceKind::TotalModuleRequests,
                limits,
            )?,
            exports: checked_total(
                self.exports,
                usage.exports,
                BytecodeImageResourceKind::TotalModuleExports,
                limits,
            )?,
            star_exports: checked_total(
                self.star_exports,
                usage.star_exports,
                BytecodeImageResourceKind::TotalModuleStarExports,
                limits,
            )?,
            imports: checked_total(
                self.imports,
                usage.imports,
                BytecodeImageResourceKind::TotalModuleImports,
                limits,
            )?,
        })
    }

    /// Reattribute an effective per-record failure to the aggregate whose
    /// remaining budget caused it. This exactly mirrors function accounting:
    /// a genuine narrower per-module cap remains a [`ModuleBudgetError`].
    pub(super) fn aggregate_error_for_module(
        self,
        error: &ModuleBudgetError,
        remaining: RemainingModuleBudget,
        limits: BytecodeImageLimits,
    ) -> Option<BytecodeImageBudgetError> {
        let (total, requested, kind) = match error {
            ModuleBudgetError::ResourceLimit {
                kind: ModuleResourceKind::Requests,
                requested,
                ..
            } if remaining.requests < limits.module().limit(ModuleResourceKind::Requests)
                && *requested > remaining.requests =>
            {
                (
                    self.requests,
                    *requested,
                    BytecodeImageResourceKind::TotalModuleRequests,
                )
            }
            ModuleBudgetError::ResourceLimit {
                kind: ModuleResourceKind::Exports,
                requested,
                ..
            } if remaining.exports < limits.module().limit(ModuleResourceKind::Exports)
                && *requested > remaining.exports =>
            {
                (
                    self.exports,
                    *requested,
                    BytecodeImageResourceKind::TotalModuleExports,
                )
            }
            ModuleBudgetError::ResourceLimit {
                kind: ModuleResourceKind::StarExports,
                requested,
                ..
            } if remaining.star_exports
                < limits.module().limit(ModuleResourceKind::StarExports)
                && *requested > remaining.star_exports =>
            {
                (
                    self.star_exports,
                    *requested,
                    BytecodeImageResourceKind::TotalModuleStarExports,
                )
            }
            ModuleBudgetError::ResourceLimit {
                kind: ModuleResourceKind::Imports,
                requested,
                ..
            } if remaining.imports < limits.module().limit(ModuleResourceKind::Imports)
                && *requested > remaining.imports =>
            {
                (
                    self.imports,
                    *requested,
                    BytecodeImageResourceKind::TotalModuleImports,
                )
            }
            _ => return None,
        };
        Some(aggregate_limit_error(total, requested, kind, limits))
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct FunctionTotals {
    constant_pool_entries: usize,
    local_variables: usize,
    closure_variables: usize,
    code_bytes: usize,
    instructions: usize,
    atom_relocations: usize,
    debug_bytes: usize,
}

#[derive(Clone, Copy)]
pub(super) struct FunctionUsage {
    constant_pool_entries: usize,
    local_variables: usize,
    closure_variables: usize,
    code_bytes: usize,
    instructions: usize,
    atom_relocations: usize,
    debug_bytes: usize,
}

impl FunctionUsage {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub(super) const fn new(
        constant_pool_entries: usize,
        local_variables: usize,
        closure_variables: usize,
        code_bytes: usize,
        instructions: usize,
        atom_relocations: usize,
        debug_bytes: usize,
    ) -> Self {
        Self {
            constant_pool_entries,
            local_variables,
            closure_variables,
            code_bytes,
            instructions,
            atom_relocations,
            debug_bytes,
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct RemainingFunctionBudget {
    constant_pool_entries: usize,
    local_variables: usize,
    closure_variables: usize,
    code_bytes: usize,
    instructions: usize,
    atom_relocations: usize,
    debug_bytes: usize,
}

impl RemainingFunctionBudget {
    #[must_use]
    pub(super) const fn intersect(self, limits: FunctionEnvelopeLimits) -> FunctionEnvelopeLimits {
        limits
            .intersect_counts(
                self.local_variables,
                self.closure_variables,
                self.constant_pool_entries,
                self.debug_bytes,
            )
            .intersect_code(self.code_bytes, self.instructions, self.atom_relocations)
    }
}

impl FunctionTotals {
    pub(super) fn remaining(
        self,
        limits: BytecodeImageLimits,
    ) -> Result<RemainingFunctionBudget, BytecodeImageBudgetError> {
        Ok(RemainingFunctionBudget {
            constant_pool_entries: checked_remaining(
                self.constant_pool_entries,
                BytecodeImageResourceKind::TotalConstantPoolEntries,
                limits,
            )?,
            local_variables: checked_remaining(
                self.local_variables,
                BytecodeImageResourceKind::TotalLocalVariables,
                limits,
            )?,
            closure_variables: checked_remaining(
                self.closure_variables,
                BytecodeImageResourceKind::TotalClosureVariables,
                limits,
            )?,
            code_bytes: checked_remaining(
                self.code_bytes,
                BytecodeImageResourceKind::TotalCodeBytes,
                limits,
            )?,
            instructions: checked_remaining(
                self.instructions,
                BytecodeImageResourceKind::TotalInstructions,
                limits,
            )?,
            atom_relocations: checked_remaining(
                self.atom_relocations,
                BytecodeImageResourceKind::TotalAtomRelocations,
                limits,
            )?,
            debug_bytes: checked_remaining(
                self.debug_bytes,
                BytecodeImageResourceKind::TotalDebugBytes,
                limits,
            )?,
        })
    }

    pub(super) fn checked_add(
        self,
        usage: FunctionUsage,
        limits: BytecodeImageLimits,
    ) -> Result<Self, BytecodeImageBudgetError> {
        Ok(Self {
            constant_pool_entries: checked_total(
                self.constant_pool_entries,
                usage.constant_pool_entries,
                BytecodeImageResourceKind::TotalConstantPoolEntries,
                limits,
            )?,
            local_variables: checked_total(
                self.local_variables,
                usage.local_variables,
                BytecodeImageResourceKind::TotalLocalVariables,
                limits,
            )?,
            closure_variables: checked_total(
                self.closure_variables,
                usage.closure_variables,
                BytecodeImageResourceKind::TotalClosureVariables,
                limits,
            )?,
            code_bytes: checked_total(
                self.code_bytes,
                usage.code_bytes,
                BytecodeImageResourceKind::TotalCodeBytes,
                limits,
            )?,
            instructions: checked_total(
                self.instructions,
                usage.instructions,
                BytecodeImageResourceKind::TotalInstructions,
                limits,
            )?,
            atom_relocations: checked_total(
                self.atom_relocations,
                usage.atom_relocations,
                BytecodeImageResourceKind::TotalAtomRelocations,
                limits,
            )?,
            debug_bytes: checked_total(
                self.debug_bytes,
                usage.debug_bytes,
                BytecodeImageResourceKind::TotalDebugBytes,
                limits,
            )?,
        })
    }

    pub(super) fn aggregate_error_for_envelope(
        self,
        error: &FunctionEnvelopeError,
        remaining: RemainingFunctionBudget,
        limits: BytecodeImageLimits,
    ) -> Option<BytecodeImageBudgetError> {
        let (total, requested, kind) = match error {
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::LocalVariables,
                requested,
                ..
            } if remaining.local_variables
                < limits
                    .envelope()
                    .limit(FunctionResourceKind::LocalVariables)
                && *requested > remaining.local_variables =>
            {
                (
                    self.local_variables,
                    *requested,
                    BytecodeImageResourceKind::TotalLocalVariables,
                )
            }
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::ClosureVariables,
                requested,
                ..
            } if remaining.closure_variables
                < limits
                    .envelope()
                    .limit(FunctionResourceKind::ClosureVariables)
                && *requested > remaining.closure_variables =>
            {
                (
                    self.closure_variables,
                    *requested,
                    BytecodeImageResourceKind::TotalClosureVariables,
                )
            }
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::ConstantPoolEntries,
                requested,
                ..
            } if remaining.constant_pool_entries
                < limits
                    .envelope()
                    .limit(FunctionResourceKind::ConstantPoolEntries)
                && *requested > remaining.constant_pool_entries =>
            {
                (
                    self.constant_pool_entries,
                    *requested,
                    BytecodeImageResourceKind::TotalConstantPoolEntries,
                )
            }
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::TotalDebugBytes,
                requested,
                ..
            } if remaining.debug_bytes
                < limits
                    .envelope()
                    .limit(FunctionResourceKind::TotalDebugBytes)
                && *requested > remaining.debug_bytes =>
            {
                (
                    self.debug_bytes,
                    *requested,
                    BytecodeImageResourceKind::TotalDebugBytes,
                )
            }
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested,
                ..
            }) if remaining.code_bytes < limits.envelope().code_limit(CodeResourceKind::Bytes)
                && *requested > remaining.code_bytes =>
            {
                (
                    self.code_bytes,
                    *requested,
                    BytecodeImageResourceKind::TotalCodeBytes,
                )
            }
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::Instructions,
                requested,
                ..
            }) if remaining.instructions
                < limits.envelope().code_limit(CodeResourceKind::Instructions)
                && *requested > remaining.instructions =>
            {
                (
                    self.instructions,
                    *requested,
                    BytecodeImageResourceKind::TotalInstructions,
                )
            }
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::AtomRelocations,
                requested,
                ..
            }) if remaining.atom_relocations
                < limits
                    .envelope()
                    .code_limit(CodeResourceKind::AtomRelocations)
                && *requested > remaining.atom_relocations =>
            {
                (
                    self.atom_relocations,
                    *requested,
                    BytecodeImageResourceKind::TotalAtomRelocations,
                )
            }
            _ => return None,
        };
        Some(aggregate_limit_error(total, requested, kind, limits))
    }
}

fn checked_total(
    total: usize,
    additional: usize,
    kind: BytecodeImageResourceKind,
    limits: BytecodeImageLimits,
) -> Result<usize, BytecodeImageBudgetError> {
    let requested = total
        .checked_add(additional)
        .ok_or(BytecodeImageBudgetError::CountOverflow { kind })?;
    limits.check(kind, requested)?;
    Ok(requested)
}

fn checked_remaining(
    total: usize,
    kind: BytecodeImageResourceKind,
    limits: BytecodeImageLimits,
) -> Result<usize, BytecodeImageBudgetError> {
    limits
        .limit(kind)
        .checked_sub(total)
        .ok_or(BytecodeImageBudgetError::CountOverflow { kind })
}

fn aggregate_limit_error(
    total: usize,
    additional: usize,
    kind: BytecodeImageResourceKind,
    limits: BytecodeImageLimits,
) -> BytecodeImageBudgetError {
    match total.checked_add(additional) {
        Some(requested) => BytecodeImageBudgetError::ResourceLimit {
            kind,
            requested,
            limit: limits.limit(kind),
        },
        None => BytecodeImageBudgetError::CountOverflow { kind },
    }
}

const fn minimum(left: usize, right: usize) -> usize {
    if left < right { left } else { right }
}
