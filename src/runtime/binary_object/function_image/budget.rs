//! Shared bounded accounting for whole BC5 function images.
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

/// Aggregate whole-image limits in addition to the per-value graph and
/// per-function envelope limits.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) struct FunctionImageLimits {
    graph: GraphLimits,
    envelope: FunctionEnvelopeLimits,
    max_functions: usize,
    max_whole_depth: usize,
    max_total_constant_pool_entries: usize,
    max_total_local_variables: usize,
    max_total_closure_variables: usize,
    max_total_code_bytes: usize,
    max_total_instructions: usize,
    max_total_atom_relocations: usize,
    max_total_debug_bytes: usize,
}

impl FunctionImageLimits {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub(in crate::runtime) const fn new(
        graph: GraphLimits,
        envelope: FunctionEnvelopeLimits,
        max_functions: usize,
        max_whole_depth: usize,
        max_total_constant_pool_entries: usize,
        max_total_local_variables: usize,
        max_total_closure_variables: usize,
        max_total_code_bytes: usize,
        max_total_instructions: usize,
        max_total_atom_relocations: usize,
        max_total_debug_bytes: usize,
    ) -> Self {
        Self {
            graph,
            envelope,
            max_functions,
            max_whole_depth,
            max_total_constant_pool_entries,
            max_total_local_variables,
            max_total_closure_variables,
            max_total_code_bytes,
            max_total_instructions,
            max_total_atom_relocations,
            max_total_debug_bytes,
        }
    }

    pub(super) const fn limit(self, kind: FunctionImageResourceKind) -> usize {
        match kind {
            FunctionImageResourceKind::Functions => self.max_functions,
            FunctionImageResourceKind::WholeDepth => self.max_whole_depth,
            FunctionImageResourceKind::TotalConstantPoolEntries => {
                self.max_total_constant_pool_entries
            }
            FunctionImageResourceKind::TotalLocalVariables => self.max_total_local_variables,
            FunctionImageResourceKind::TotalClosureVariables => self.max_total_closure_variables,
            FunctionImageResourceKind::TotalCodeBytes => self.max_total_code_bytes,
            FunctionImageResourceKind::TotalInstructions => self.max_total_instructions,
            FunctionImageResourceKind::TotalAtomRelocations => self.max_total_atom_relocations,
            FunctionImageResourceKind::TotalDebugBytes => self.max_total_debug_bytes,
        }
    }

    pub(super) fn check(
        self,
        kind: FunctionImageResourceKind,
        requested: usize,
    ) -> Result<(), FunctionImageBudgetError> {
        let limit = self.limit(kind);
        if requested > limit {
            return Err(FunctionImageBudgetError::ResourceLimit {
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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FunctionImageResourceKind {
    Functions,
    WholeDepth,
    TotalConstantPoolEntries,
    TotalLocalVariables,
    TotalClosureVariables,
    TotalCodeBytes,
    TotalInstructions,
    TotalAtomRelocations,
    TotalDebugBytes,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum FunctionImageBudgetError {
    ResourceLimit {
        kind: FunctionImageResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        kind: FunctionImageResourceKind,
    },
}

impl fmt::Display for FunctionImageBudgetError {
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

impl std::error::Error for FunctionImageBudgetError {}

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
        limits: FunctionImageLimits,
    ) -> Result<RemainingFunctionBudget, FunctionImageBudgetError> {
        Ok(RemainingFunctionBudget {
            constant_pool_entries: checked_remaining(
                self.constant_pool_entries,
                FunctionImageResourceKind::TotalConstantPoolEntries,
                limits,
            )?,
            local_variables: checked_remaining(
                self.local_variables,
                FunctionImageResourceKind::TotalLocalVariables,
                limits,
            )?,
            closure_variables: checked_remaining(
                self.closure_variables,
                FunctionImageResourceKind::TotalClosureVariables,
                limits,
            )?,
            code_bytes: checked_remaining(
                self.code_bytes,
                FunctionImageResourceKind::TotalCodeBytes,
                limits,
            )?,
            instructions: checked_remaining(
                self.instructions,
                FunctionImageResourceKind::TotalInstructions,
                limits,
            )?,
            atom_relocations: checked_remaining(
                self.atom_relocations,
                FunctionImageResourceKind::TotalAtomRelocations,
                limits,
            )?,
            debug_bytes: checked_remaining(
                self.debug_bytes,
                FunctionImageResourceKind::TotalDebugBytes,
                limits,
            )?,
        })
    }

    pub(super) fn checked_add(
        self,
        usage: FunctionUsage,
        limits: FunctionImageLimits,
    ) -> Result<Self, FunctionImageBudgetError> {
        Ok(Self {
            constant_pool_entries: checked_total(
                self.constant_pool_entries,
                usage.constant_pool_entries,
                FunctionImageResourceKind::TotalConstantPoolEntries,
                limits,
            )?,
            local_variables: checked_total(
                self.local_variables,
                usage.local_variables,
                FunctionImageResourceKind::TotalLocalVariables,
                limits,
            )?,
            closure_variables: checked_total(
                self.closure_variables,
                usage.closure_variables,
                FunctionImageResourceKind::TotalClosureVariables,
                limits,
            )?,
            code_bytes: checked_total(
                self.code_bytes,
                usage.code_bytes,
                FunctionImageResourceKind::TotalCodeBytes,
                limits,
            )?,
            instructions: checked_total(
                self.instructions,
                usage.instructions,
                FunctionImageResourceKind::TotalInstructions,
                limits,
            )?,
            atom_relocations: checked_total(
                self.atom_relocations,
                usage.atom_relocations,
                FunctionImageResourceKind::TotalAtomRelocations,
                limits,
            )?,
            debug_bytes: checked_total(
                self.debug_bytes,
                usage.debug_bytes,
                FunctionImageResourceKind::TotalDebugBytes,
                limits,
            )?,
        })
    }

    pub(super) fn aggregate_error_for_envelope(
        self,
        error: &FunctionEnvelopeError,
        remaining: RemainingFunctionBudget,
        limits: FunctionImageLimits,
    ) -> Option<FunctionImageBudgetError> {
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
                    FunctionImageResourceKind::TotalLocalVariables,
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
                    FunctionImageResourceKind::TotalClosureVariables,
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
                    FunctionImageResourceKind::TotalConstantPoolEntries,
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
                    FunctionImageResourceKind::TotalDebugBytes,
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
                    FunctionImageResourceKind::TotalCodeBytes,
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
                    FunctionImageResourceKind::TotalInstructions,
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
                    FunctionImageResourceKind::TotalAtomRelocations,
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
    kind: FunctionImageResourceKind,
    limits: FunctionImageLimits,
) -> Result<usize, FunctionImageBudgetError> {
    let requested = total
        .checked_add(additional)
        .ok_or(FunctionImageBudgetError::CountOverflow { kind })?;
    limits.check(kind, requested)?;
    Ok(requested)
}

fn checked_remaining(
    total: usize,
    kind: FunctionImageResourceKind,
    limits: FunctionImageLimits,
) -> Result<usize, FunctionImageBudgetError> {
    limits
        .limit(kind)
        .checked_sub(total)
        .ok_or(FunctionImageBudgetError::CountOverflow { kind })
}

fn aggregate_limit_error(
    total: usize,
    additional: usize,
    kind: FunctionImageResourceKind,
    limits: FunctionImageLimits,
) -> FunctionImageBudgetError {
    match total.checked_add(additional) {
        Some(requested) => FunctionImageBudgetError::ResourceLimit {
            kind,
            requested,
            limit: limits.limit(kind),
        },
        None => FunctionImageBudgetError::CountOverflow { kind },
    }
}
