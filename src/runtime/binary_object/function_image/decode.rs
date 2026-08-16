//! Bounded whole-image decoder for QuickJS 2026-06-04 FunctionBytecode.
//!
//! This reader owns one atom table, one data-machine/object arena, one
//! function table, and one heterogeneous frame stack for the entire input.
//! The resulting image is structural only: no runtime heap object is allocated
//! and no native bytecode is admitted to execution.

use std::fmt;

use super::super::code::CodeImageParts;
use super::super::function_envelope::{
    FunctionEnvelope, FunctionEnvelopeError, FunctionEnvelopeParts,
    read_function_record_prefix_after_tag,
};
use super::super::graph::decode::{
    DataCompletion, DataFrame, DataMachine, DataMachineOutput, DataReadStep, DecodeError,
    MachineSource, PropertyDisposition,
};
use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits};
use super::atoms::{ImageAtomError, ImageAtomTable, ImageKey};
use super::budget::{
    FunctionImageBudgetError, FunctionImageLimits, FunctionImageResourceKind, FunctionTotals,
    FunctionUsage, RemainingFunctionBudget,
};
use super::model::{
    FunctionId, FunctionImage, FunctionRecord, ImageClosureVariable, ImageCode, ImageFunctionDebug,
    ImageFunctionEnvelope, ImageInstructionSpan, ImageLocalVariable, ImageRelocation, ImageValue,
};

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

impl From<FunctionImageBudgetError> for FunctionImageError {
    fn from(error: FunctionImageBudgetError) -> Self {
        match error {
            FunctionImageBudgetError::ResourceLimit {
                kind,
                requested,
                limit,
            } => Self::ResourceLimit {
                kind,
                requested,
                limit,
            },
            FunctionImageBudgetError::CountOverflow { kind } => Self::CountOverflow { kind },
        }
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
        DataMachine::<ImageValue, ImageKey>::new(limits.graph(), allow_object_references)?;
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

        let remaining = self.totals.remaining(self.limits)?;
        let envelope_limits = remaining.intersect(self.limits.envelope());
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
        let additional_debug_bytes = match envelope.debug() {
            Some(debug) => debug
                .pc2line()
                .len()
                .checked_add(debug.source().len())
                .ok_or(FunctionImageBudgetError::CountOverflow {
                    kind: FunctionImageResourceKind::TotalDebugBytes,
                })?,
            None => 0,
        };
        self.totals
            .checked_add(
                FunctionUsage::new(
                    constant_count,
                    envelope.locals().len(),
                    envelope.closures().len(),
                    envelope.code().as_bytes().len(),
                    envelope.code().instructions().len(),
                    envelope.code().atom_relocations().len(),
                    additional_debug_bytes,
                ),
                self.limits,
            )
            .map_err(Into::into)
    }

    fn map_prefix_error(
        &self,
        error: FunctionEnvelopeError,
        remaining: RemainingFunctionBudget,
    ) -> FunctionImageError {
        match self
            .totals
            .aggregate_error_for_envelope(&error, remaining, self.limits)
        {
            Some(error) => error.into(),
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
