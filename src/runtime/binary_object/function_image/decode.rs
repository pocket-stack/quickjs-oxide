//! Bounded whole-image decoder for QuickJS 2026-06-04 FunctionBytecode.
//!
//! This reader owns one atom table, one data-machine/object arena, one
//! function table, and one heterogeneous frame stack for the entire input.
//! The resulting image is structural only: no runtime heap object is allocated
//! and no native bytecode is admitted to execution.

use std::fmt;

use super::super::code::{CodeError, CodeImageParts, CodeResourceKind};
use super::super::function_envelope::{
    FunctionEnvelope, FunctionEnvelopeError, FunctionEnvelopeLimits, FunctionEnvelopeParts,
    FunctionResourceKind, read_function_record_prefix_after_tag,
};
use super::super::graph::decode::{
    DataCompletion, DataFrame, DataMachine, DataMachineOutput, DataReadStep, DecodeError,
    MachineSource, PropertyDisposition,
};
use super::super::graph::model::GraphLimits;
use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits};
use super::atoms::{ImageAtomError, ImageAtomTable, ImageKey};
use super::model::{
    FunctionId, FunctionImage, FunctionRecord, ImageClosureVariable, ImageCode, ImageFunctionDebug,
    ImageFunctionEnvelope, ImageInstructionSpan, ImageLocalVariable, ImageRelocation, ImageValue,
};

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

    const fn limit(self, kind: FunctionImageResourceKind) -> usize {
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

    fn check(
        self,
        kind: FunctionImageResourceKind,
        requested: usize,
    ) -> Result<(), FunctionImageError> {
        let limit = self.limit(kind);
        if requested > limit {
            return Err(FunctionImageError::ResourceLimit {
                kind,
                requested,
                limit,
            });
        }
        Ok(())
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
pub(in crate::runtime) enum FunctionImageError {
    Wire(WireError),
    Atom(ImageAtomError),
    Data(DecodeError<FunctionId>),
    Envelope(FunctionEnvelopeError),
    ResourceLimit {
        kind: FunctionImageResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        kind: FunctionImageResourceKind,
    },
    OffsetOverflow {
        offset: usize,
        addend: usize,
    },
    InvalidCompletionTarget,
    InvalidFunctionState {
        function_index: u32,
    },
    AllocationFailed,
}

impl fmt::Display for FunctionImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Atom(error) => fmt::Display::fmt(error, formatter),
            Self::Data(error) => fmt::Display::fmt(error, formatter),
            Self::Envelope(error) => fmt::Display::fmt(error, formatter),
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
            Self::OffsetOverflow { offset, addend } => write!(
                formatter,
                "whole-image offset {offset} plus {addend} bytes overflowed"
            ),
            Self::InvalidCompletionTarget => {
                formatter.write_str("invalid whole-image completion target")
            }
            Self::InvalidFunctionState { function_index } => write!(
                formatter,
                "function {function_index} has an invalid whole-image decoder state"
            ),
            Self::AllocationFailed => formatter.write_str("whole-image allocation failed"),
        }
    }
}

impl std::error::Error for FunctionImageError {}

impl From<WireError> for FunctionImageError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<ImageAtomError> for FunctionImageError {
    fn from(error: ImageAtomError) -> Self {
        Self::Atom(error)
    }
}

impl From<DecodeError<FunctionId>> for FunctionImageError {
    fn from(error: DecodeError<FunctionId>) -> Self {
        Self::Data(error)
    }
}

impl From<FunctionEnvelopeError> for FunctionImageError {
    fn from(error: FunctionEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

/// Decode one complete bytecode-mode BC5 image without making it executable.
pub(in crate::runtime) fn decode_function_image(
    input: &[u8],
    mode: ReaderMode,
    wire_limits: WireLimits,
    limits: FunctionImageLimits,
    allow_object_references: bool,
) -> Result<FunctionImage, FunctionImageError> {
    let mut cursor = WireCursor::new(input, mode, wire_limits)?;
    let atoms = ImageAtomTable::read(&mut cursor)?;
    let mut machine =
        DataMachine::<ImageValue, ImageKey>::new(limits.graph, allow_object_references)?;
    let source = machine.source();
    let mut functions = FunctionTable::new(source, limits);
    let mut frames = Vec::new();
    let mut data_depth = 0usize;
    let mut root = None;

    loop {
        if root.is_some() && frames.is_empty() {
            break;
        }

        // Ordinary-object keys are read by the parent before QuickJS enters
        // the recursive child call (whose first action is the stack check).
        // Keep that ordering for malformed-key versus local depth failures.
        let return_to = next_target(&mut cursor, &atoms, &frames)?;
        let depth = frames
            .len()
            .checked_add(1)
            .ok_or(FunctionImageError::CountOverflow {
                kind: FunctionImageResourceKind::WholeDepth,
            })?;
        limits.check(FunctionImageResourceKind::WholeDepth, depth)?;

        let tag_offset = cursor.position();
        let tag = cursor.read_tag()?;
        if tag == BcTag::FunctionBytecode {
            let frame = functions.begin_function(&mut cursor, &atoms, tag_offset)?;
            frames
                .try_reserve(1)
                .map_err(|_| FunctionImageError::AllocationFailed)?;
            frames.push(ActiveFrame::Function { frame, return_to });
        } else {
            match machine.read_value_after_tag(&mut cursor, tag, tag_offset, data_depth)? {
                DataReadStep::Complete(value) => {
                    deliver_completed(&machine, &mut frames, return_to, value, &mut root)?;
                }
                DataReadStep::Pending(frame) => {
                    frames
                        .try_reserve(1)
                        .map_err(|_| FunctionImageError::AllocationFailed)?;
                    frames.push(ActiveFrame::Data { frame, return_to });
                    data_depth =
                        data_depth
                            .checked_add(1)
                            .ok_or(FunctionImageError::CountOverflow {
                                kind: FunctionImageResourceKind::WholeDepth,
                            })?;
                }
            }
        }

        drain_completed(
            &mut machine,
            &mut functions,
            &mut frames,
            &mut data_depth,
            &mut root,
        )?;
    }

    // Strict-vs-compatible trailing-input behavior remains centralized in the
    // same cursor that read every prefix and constant-pool value.
    cursor.finish()?;

    let root = root.ok_or(FunctionImageError::InvalidCompletionTarget)?;
    let output = machine.finish_output()?;
    let root = output.unwrap_completion(root)?;
    let records = functions.finish(&output)?;
    let parts = output.into_parts();
    Ok(FunctionImage::new(
        source,
        atoms.into_dynamic_atoms(),
        parts.nodes,
        parts.ref_table,
        records,
        root,
    ))
}

#[derive(Clone, Copy)]
enum CompletionTarget {
    Root,
    DataParent {
        key: Option<PropertyDisposition<ImageKey>>,
    },
    FunctionConstant,
}

enum ActiveFrame {
    Data {
        frame: DataFrame<ImageValue, ImageKey>,
        return_to: CompletionTarget,
    },
    Function {
        frame: FunctionFrame,
        return_to: CompletionTarget,
    },
}

fn next_target(
    cursor: &mut WireCursor<'_>,
    atoms: &ImageAtomTable,
    frames: &[ActiveFrame],
) -> Result<CompletionTarget, FunctionImageError> {
    match frames.last() {
        None => Ok(CompletionTarget::Root),
        Some(ActiveFrame::Function { .. }) => Ok(CompletionTarget::FunctionConstant),
        Some(ActiveFrame::Data { frame, .. }) => {
            let key = frame
                .expects_property_key()
                .then(|| read_key(cursor, atoms))
                .transpose()?;
            Ok(CompletionTarget::DataParent { key })
        }
    }
}

fn read_key(
    cursor: &mut WireCursor<'_>,
    atoms: &ImageAtomTable,
) -> Result<PropertyDisposition<ImageKey>, FunctionImageError> {
    let offset = cursor.position();
    let raw = atoms.raw_space().decode_metadata_atom(cursor)?;
    Ok(
        match atoms.remap_key(atoms.raw_space(), raw, cursor.mode(), offset)? {
            Some(key) => PropertyDisposition::Define(key),
            None => PropertyDisposition::Ignore,
        },
    )
}

fn drain_completed(
    machine: &mut DataMachine<ImageValue, ImageKey>,
    functions: &mut FunctionTable,
    frames: &mut Vec<ActiveFrame>,
    data_depth: &mut usize,
    root: &mut Option<DataCompletion<ImageValue>>,
) -> Result<(), FunctionImageError> {
    loop {
        let complete = match frames.last() {
            Some(ActiveFrame::Data { frame, .. }) => frame.is_complete(),
            Some(ActiveFrame::Function { frame, .. }) => frame.is_complete(),
            None => false,
        };
        if !complete {
            return Ok(());
        }

        let active = frames
            .pop()
            .ok_or(FunctionImageError::InvalidCompletionTarget)?;
        let (return_to, value) = match active {
            ActiveFrame::Data { frame, return_to } => {
                *data_depth = data_depth
                    .checked_sub(1)
                    .ok_or(FunctionImageError::InvalidCompletionTarget)?;
                (return_to, machine.finish_frame(frame)?)
            }
            ActiveFrame::Function { frame, return_to } => {
                let function = functions.finish_frame(frame)?;
                let value = machine.wrap_opaque_value(ImageValue::from_function(function))?;
                (return_to, value)
            }
        };
        deliver_completed(machine, frames, return_to, value, root)?;
    }
}

fn deliver_completed(
    machine: &DataMachine<ImageValue, ImageKey>,
    frames: &mut [ActiveFrame],
    target: CompletionTarget,
    value: DataCompletion<ImageValue>,
    root: &mut Option<DataCompletion<ImageValue>>,
) -> Result<(), FunctionImageError> {
    match target {
        CompletionTarget::Root => {
            if root.replace(value).is_some() {
                return Err(FunctionImageError::InvalidCompletionTarget);
            }
        }
        CompletionTarget::DataParent { key } => {
            let Some(ActiveFrame::Data { frame, .. }) = frames.last_mut() else {
                return Err(FunctionImageError::InvalidCompletionTarget);
            };
            machine.attach_to_frame(frame, key, value)?;
        }
        CompletionTarget::FunctionConstant => {
            let Some(ActiveFrame::Function { frame, .. }) = frames.last_mut() else {
                return Err(FunctionImageError::InvalidCompletionTarget);
            };
            frame.push_constant(value)?;
        }
    }
    Ok(())
}

struct FunctionFrame {
    function: FunctionSlot,
    envelope: ImageFunctionEnvelope,
    expected_constants: usize,
    constants: Vec<DataCompletion<ImageValue>>,
}

impl FunctionFrame {
    fn push_constant(
        &mut self,
        value: DataCompletion<ImageValue>,
    ) -> Result<(), FunctionImageError> {
        if self.constants.len() >= self.expected_constants {
            return Err(FunctionImageError::InvalidCompletionTarget);
        }
        self.constants.push(value);
        Ok(())
    }

    fn is_complete(&self) -> bool {
        self.constants.len() == self.expected_constants
    }
}

struct PendingFunctionRecord {
    envelope: ImageFunctionEnvelope,
    constants: Vec<DataCompletion<ImageValue>>,
}

#[derive(Clone, Copy)]
struct FunctionSlot {
    source: MachineSource,
    index: u32,
}

/// Move-only proof that one same-source function slot was completely filled.
///
/// Fields and construction are private to this decoder. The semantic model can
/// consume the proof, but no sibling can turn an arbitrary local index into an
/// opaque ImageValue.
pub(super) struct AuthenticatedFunction {
    source: MachineSource,
    index: u32,
}

impl AuthenticatedFunction {
    fn new(source: MachineSource, index: u32) -> Self {
        Self { source, index }
    }

    pub(super) const fn source(&self) -> MachineSource {
        self.source
    }

    pub(super) const fn index(&self) -> u32 {
        self.index
    }
}

struct FunctionTable {
    source: MachineSource,
    limits: FunctionImageLimits,
    slots: Vec<Option<PendingFunctionRecord>>,
    totals: FunctionTotals,
}

#[derive(Default)]
struct FunctionTotals {
    constant_pool_entries: usize,
    local_variables: usize,
    closure_variables: usize,
    code_bytes: usize,
    instructions: usize,
    atom_relocations: usize,
    debug_bytes: usize,
}

#[derive(Clone, Copy)]
struct RemainingFunctionBudget {
    constant_pool_entries: usize,
    local_variables: usize,
    closure_variables: usize,
    code_bytes: usize,
    instructions: usize,
    atom_relocations: usize,
    debug_bytes: usize,
}

impl FunctionTable {
    fn new(source: MachineSource, limits: FunctionImageLimits) -> Self {
        Self {
            source,
            limits,
            slots: Vec::new(),
            totals: FunctionTotals::default(),
        }
    }

    fn begin_function(
        &mut self,
        cursor: &mut WireCursor<'_>,
        atoms: &ImageAtomTable,
        tag_offset: usize,
    ) -> Result<FunctionFrame, FunctionImageError> {
        let requested =
            self.slots
                .len()
                .checked_add(1)
                .ok_or(FunctionImageError::CountOverflow {
                    kind: FunctionImageResourceKind::Functions,
                })?;
        self.limits
            .check(FunctionImageResourceKind::Functions, requested)?;
        let index =
            u32::try_from(self.slots.len()).map_err(|_| FunctionImageError::CountOverflow {
                kind: FunctionImageResourceKind::Functions,
            })?;
        self.slots
            .try_reserve(1)
            .map_err(|_| FunctionImageError::AllocationFailed)?;
        self.slots.push(None);
        let function = FunctionSlot {
            source: self.source,
            index,
        };

        let remaining = self.remaining_budget()?;
        let envelope_limits = self
            .limits
            .envelope
            .intersect_counts(
                remaining.local_variables,
                remaining.closure_variables,
                remaining.constant_pool_entries,
                remaining.debug_bytes,
            )
            .intersect_code(
                remaining.code_bytes,
                remaining.instructions,
                remaining.atom_relocations,
            );
        let prefix =
            read_function_record_prefix_after_tag(cursor, atoms.raw_space(), envelope_limits)
                .map_err(|error| self.map_prefix_error(error, remaining))?;
        let (envelope, constant_count) = prefix.into_parts();
        let expected_constants = constant_count as usize;
        let totals = self.next_totals(&envelope, expected_constants)?;
        let envelope = self.relocate_envelope(atoms, envelope, tag_offset)?;
        self.totals = totals;

        let mut constants = Vec::new();
        constants
            .try_reserve_exact(expected_constants)
            .map_err(|_| FunctionImageError::AllocationFailed)?;
        Ok(FunctionFrame {
            function,
            envelope,
            expected_constants,
            constants,
        })
    }

    fn finish_frame(
        &mut self,
        frame: FunctionFrame,
    ) -> Result<AuthenticatedFunction, FunctionImageError> {
        if frame.function.source != self.source || !frame.is_complete() {
            return Err(FunctionImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        }
        let Some(slot) = self.slots.get_mut(frame.function.index as usize) else {
            return Err(FunctionImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        };
        if slot.is_some() {
            return Err(FunctionImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        }
        *slot = Some(PendingFunctionRecord {
            envelope: frame.envelope,
            constants: frame.constants,
        });
        Ok(AuthenticatedFunction::new(
            frame.function.source,
            frame.function.index,
        ))
    }

    fn finish(
        self,
        output: &DataMachineOutput<ImageValue, ImageKey>,
    ) -> Result<Box<[FunctionRecord]>, FunctionImageError> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(self.slots.len())
            .map_err(|_| FunctionImageError::AllocationFailed)?;
        for (index, slot) in self.slots.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| FunctionImageError::CountOverflow {
                kind: FunctionImageResourceKind::Functions,
            })?;
            let pending = slot.ok_or(FunctionImageError::InvalidFunctionState {
                function_index: index,
            })?;
            let mut constants = Vec::new();
            constants
                .try_reserve_exact(pending.constants.len())
                .map_err(|_| FunctionImageError::AllocationFailed)?;
            for completion in pending.constants {
                constants.push(output.unwrap_completion(completion)?);
            }
            records.push(FunctionRecord::new(
                pending.envelope,
                constants.into_boxed_slice(),
            ));
        }
        Ok(records.into_boxed_slice())
    }

    fn relocate_envelope(
        &self,
        atoms: &ImageAtomTable,
        envelope: FunctionEnvelope,
        diagnostic_offset: usize,
    ) -> Result<ImageFunctionEnvelope, FunctionImageError> {
        let FunctionEnvelopeParts {
            atom_space,
            flags,
            js_mode,
            name,
            argument_count,
            variable_count,
            defined_argument_count,
            stack_size,
            variable_reference_count,
            locals,
            closures,
            code,
            debug,
        } = envelope.into_parts();

        let name = atoms.remap_atom(atom_space, name, diagnostic_offset)?;
        let mut image_locals = Vec::new();
        image_locals
            .try_reserve_exact(locals.len())
            .map_err(|_| FunctionImageError::AllocationFailed)?;
        for local in locals {
            let (name, scope_next, variable_reference_index, flags) = local.into_parts();
            image_locals.push(ImageLocalVariable::new(
                atoms.remap_atom(atom_space, name, diagnostic_offset)?,
                scope_next,
                variable_reference_index,
                flags,
            ));
        }

        let mut image_closures = Vec::new();
        image_closures
            .try_reserve_exact(closures.len())
            .map_err(|_| FunctionImageError::AllocationFailed)?;
        for closure in closures {
            let (name, variable_index, flags) = closure.into_parts();
            image_closures.push(ImageClosureVariable::new(
                atoms.remap_atom(atom_space, name, diagnostic_offset)?,
                variable_index,
                flags,
            ));
        }

        let code = relocate_code(atoms, code.into_parts())?;
        let debug = match debug {
            Some(debug) => {
                let (filename, pc2line, source) = debug.into_parts();
                Some(ImageFunctionDebug::new(
                    atoms.remap_atom(atom_space, filename, diagnostic_offset)?,
                    pc2line,
                    source,
                ))
            }
            None => None,
        };

        Ok(ImageFunctionEnvelope::new(
            flags,
            js_mode,
            name,
            argument_count,
            variable_count,
            defined_argument_count,
            stack_size,
            variable_reference_count,
            image_locals.into_boxed_slice(),
            image_closures.into_boxed_slice(),
            code,
            debug,
        ))
    }

    fn next_totals(
        &self,
        envelope: &FunctionEnvelope,
        constant_count: usize,
    ) -> Result<FunctionTotals, FunctionImageError> {
        let constant_pool_entries = checked_total(
            self.totals.constant_pool_entries,
            constant_count,
            FunctionImageResourceKind::TotalConstantPoolEntries,
            self.limits,
        )?;
        let local_variables = checked_total(
            self.totals.local_variables,
            envelope.locals().len(),
            FunctionImageResourceKind::TotalLocalVariables,
            self.limits,
        )?;
        let closure_variables = checked_total(
            self.totals.closure_variables,
            envelope.closures().len(),
            FunctionImageResourceKind::TotalClosureVariables,
            self.limits,
        )?;
        let code_bytes = checked_total(
            self.totals.code_bytes,
            envelope.code().as_bytes().len(),
            FunctionImageResourceKind::TotalCodeBytes,
            self.limits,
        )?;
        let instructions = checked_total(
            self.totals.instructions,
            envelope.code().instructions().len(),
            FunctionImageResourceKind::TotalInstructions,
            self.limits,
        )?;
        let atom_relocations = checked_total(
            self.totals.atom_relocations,
            envelope.code().atom_relocations().len(),
            FunctionImageResourceKind::TotalAtomRelocations,
            self.limits,
        )?;
        let additional_debug_bytes = match envelope.debug() {
            Some(debug) => debug
                .pc2line()
                .len()
                .checked_add(debug.source().len())
                .ok_or(FunctionImageError::CountOverflow {
                    kind: FunctionImageResourceKind::TotalDebugBytes,
                })?,
            None => 0,
        };
        let debug_bytes = checked_total(
            self.totals.debug_bytes,
            additional_debug_bytes,
            FunctionImageResourceKind::TotalDebugBytes,
            self.limits,
        )?;
        Ok(FunctionTotals {
            constant_pool_entries,
            local_variables,
            closure_variables,
            code_bytes,
            instructions,
            atom_relocations,
            debug_bytes,
        })
    }

    fn remaining_budget(&self) -> Result<RemainingFunctionBudget, FunctionImageError> {
        Ok(RemainingFunctionBudget {
            constant_pool_entries: checked_remaining(
                self.totals.constant_pool_entries,
                FunctionImageResourceKind::TotalConstantPoolEntries,
                self.limits,
            )?,
            local_variables: checked_remaining(
                self.totals.local_variables,
                FunctionImageResourceKind::TotalLocalVariables,
                self.limits,
            )?,
            closure_variables: checked_remaining(
                self.totals.closure_variables,
                FunctionImageResourceKind::TotalClosureVariables,
                self.limits,
            )?,
            code_bytes: checked_remaining(
                self.totals.code_bytes,
                FunctionImageResourceKind::TotalCodeBytes,
                self.limits,
            )?,
            instructions: checked_remaining(
                self.totals.instructions,
                FunctionImageResourceKind::TotalInstructions,
                self.limits,
            )?,
            atom_relocations: checked_remaining(
                self.totals.atom_relocations,
                FunctionImageResourceKind::TotalAtomRelocations,
                self.limits,
            )?,
            debug_bytes: checked_remaining(
                self.totals.debug_bytes,
                FunctionImageResourceKind::TotalDebugBytes,
                self.limits,
            )?,
        })
    }

    fn map_prefix_error(
        &self,
        error: FunctionEnvelopeError,
        remaining: RemainingFunctionBudget,
    ) -> FunctionImageError {
        let aggregate = match &error {
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::LocalVariables,
                requested,
                ..
            } if remaining.local_variables
                < self
                    .limits
                    .envelope
                    .limit(FunctionResourceKind::LocalVariables)
                && *requested > remaining.local_variables =>
            {
                Some((
                    self.totals.local_variables,
                    *requested,
                    FunctionImageResourceKind::TotalLocalVariables,
                ))
            }
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::ClosureVariables,
                requested,
                ..
            } if remaining.closure_variables
                < self
                    .limits
                    .envelope
                    .limit(FunctionResourceKind::ClosureVariables)
                && *requested > remaining.closure_variables =>
            {
                Some((
                    self.totals.closure_variables,
                    *requested,
                    FunctionImageResourceKind::TotalClosureVariables,
                ))
            }
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::ConstantPoolEntries,
                requested,
                ..
            } if remaining.constant_pool_entries
                < self
                    .limits
                    .envelope
                    .limit(FunctionResourceKind::ConstantPoolEntries)
                && *requested > remaining.constant_pool_entries =>
            {
                Some((
                    self.totals.constant_pool_entries,
                    *requested,
                    FunctionImageResourceKind::TotalConstantPoolEntries,
                ))
            }
            FunctionEnvelopeError::ResourceLimit {
                kind: FunctionResourceKind::TotalDebugBytes,
                requested,
                ..
            } if remaining.debug_bytes
                < self
                    .limits
                    .envelope
                    .limit(FunctionResourceKind::TotalDebugBytes)
                && *requested > remaining.debug_bytes =>
            {
                Some((
                    self.totals.debug_bytes,
                    *requested,
                    FunctionImageResourceKind::TotalDebugBytes,
                ))
            }
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::Bytes,
                requested,
                ..
            }) if remaining.code_bytes
                < self.limits.envelope.code_limit(CodeResourceKind::Bytes)
                && *requested > remaining.code_bytes =>
            {
                Some((
                    self.totals.code_bytes,
                    *requested,
                    FunctionImageResourceKind::TotalCodeBytes,
                ))
            }
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::Instructions,
                requested,
                ..
            }) if remaining.instructions
                < self
                    .limits
                    .envelope
                    .code_limit(CodeResourceKind::Instructions)
                && *requested > remaining.instructions =>
            {
                Some((
                    self.totals.instructions,
                    *requested,
                    FunctionImageResourceKind::TotalInstructions,
                ))
            }
            FunctionEnvelopeError::Code(CodeError::ResourceLimit {
                kind: CodeResourceKind::AtomRelocations,
                requested,
                ..
            }) if remaining.atom_relocations
                < self
                    .limits
                    .envelope
                    .code_limit(CodeResourceKind::AtomRelocations)
                && *requested > remaining.atom_relocations =>
            {
                Some((
                    self.totals.atom_relocations,
                    *requested,
                    FunctionImageResourceKind::TotalAtomRelocations,
                ))
            }
            _ => None,
        };

        match aggregate {
            Some((total, requested, kind)) => {
                aggregate_limit_error(total, requested, kind, self.limits)
            }
            None => FunctionImageError::Envelope(error),
        }
    }
}

fn relocate_code(
    atoms: &ImageAtomTable,
    parts: CodeImageParts,
) -> Result<ImageCode, FunctionImageError> {
    let mut instructions = Vec::new();
    instructions
        .try_reserve_exact(parts.instructions.len())
        .map_err(|_| FunctionImageError::AllocationFailed)?;
    for instruction in parts.instructions {
        instructions.push(ImageInstructionSpan::new(
            instruction.offset(),
            instruction.opcode(),
        ));
    }

    let mut relocations = Vec::new();
    relocations
        .try_reserve_exact(parts.atom_relocations.len())
        .map_err(|_| FunctionImageError::AllocationFailed)?;
    for relocation in parts.atom_relocations {
        let offset = parts
            .payload_offset
            .checked_add(relocation.operand_offset() as usize)
            .ok_or(FunctionImageError::OffsetOverflow {
                offset: parts.payload_offset,
                addend: relocation.operand_offset() as usize,
            })?;
        relocations.push(ImageRelocation::new(
            relocation.operand_offset(),
            atoms.remap_atom(parts.atom_space, relocation.atom(), offset)?,
        ));
    }

    Ok(ImageCode::new(
        parts.bytes.into_boxed_slice(),
        instructions.into_boxed_slice(),
        relocations.into_boxed_slice(),
    ))
}

fn checked_total(
    total: usize,
    additional: usize,
    kind: FunctionImageResourceKind,
    limits: FunctionImageLimits,
) -> Result<usize, FunctionImageError> {
    let requested = total
        .checked_add(additional)
        .ok_or(FunctionImageError::CountOverflow { kind })?;
    limits.check(kind, requested)?;
    Ok(requested)
}

fn checked_remaining(
    total: usize,
    kind: FunctionImageResourceKind,
    limits: FunctionImageLimits,
) -> Result<usize, FunctionImageError> {
    limits
        .limit(kind)
        .checked_sub(total)
        .ok_or(FunctionImageError::CountOverflow { kind })
}

fn aggregate_limit_error(
    total: usize,
    additional: usize,
    kind: FunctionImageResourceKind,
    limits: FunctionImageLimits,
) -> FunctionImageError {
    match total.checked_add(additional) {
        Some(requested) => FunctionImageError::ResourceLimit {
            kind,
            requested,
            limit: limits.limit(kind),
        },
        None => FunctionImageError::CountOverflow { kind },
    }
}
