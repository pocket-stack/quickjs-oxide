//! Bounded whole-image decoder for QuickJS 2026-06-04 bytecode images.
//!
//! This reader owns one atom table, one data-machine/object arena, one
//! function table, one module table, and one heterogeneous frame stack for the
//! entire input.
//! The resulting image is structural only: no runtime heap object is allocated
//! and no native bytecode is admitted to execution.

use super::super::function_envelope::FunctionEnvelopeError;
use super::super::graph::decode::{
    DataCompletion, DataCursor, DataFrame, DataMachine, DataReadStep, DecodeError, MachineSource,
    PropertyDisposition,
};
use super::super::graph::sab_transport::SabArchiveError;
use super::super::read_cursor::CheckedReadCursor;
use super::super::wire::{BcTag, ReaderMode, WireCursor, WireError, WireLimits};
use super::atoms::{ImageAtom, ImageAtomError, ImageAtomTable, ImageKey};
use super::budget::{
    BytecodeImageBudgetError, BytecodeImageLimits, BytecodeImageResourceKind, ModuleBudgetError,
    ModuleResourceKind,
};
use super::model::{BytecodeImage, ImageOpaque, ImageValue};
use std::fmt;

mod function;
mod module;

use function::{FunctionFrame, FunctionTable};
use module::{ModuleFrame, ModuleTable};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum ModuleField {
    LocalExportVariable,
    IndirectExportRequest,
    StarExportRequest,
    ImportVariable,
    ImportRequest,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum BytecodeImageError {
    Wire(WireError),
    Atom(ImageAtomError),
    Data(DecodeError<ImageOpaque>),
    Envelope(FunctionEnvelopeError),
    Module(ModuleBudgetError),
    ResourceLimit {
        kind: BytecodeImageResourceKind,
        requested: usize,
        limit: usize,
    },
    CountOverflow {
        kind: BytecodeImageResourceKind,
    },
    OffsetOverflow {
        offset: usize,
        addend: usize,
    },
    InvalidCompletionTarget,
    InvalidFunctionState {
        function_index: u32,
    },
    ModuleCountOutOfRange {
        kind: ModuleResourceKind,
        offset: usize,
        count: u32,
        maximum: u32,
    },
    ModuleFieldOutOfRange {
        field: ModuleField,
        offset: usize,
        value: u32,
        maximum: u32,
    },
    InvalidModuleState {
        module_index: u32,
    },
    AllocationFailed,
}

impl fmt::Display for BytecodeImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wire(error) => fmt::Display::fmt(error, formatter),
            Self::Atom(error) => fmt::Display::fmt(error, formatter),
            Self::Data(error) => fmt::Display::fmt(error, formatter),
            Self::Envelope(error) => fmt::Display::fmt(error, formatter),
            Self::Module(error) => fmt::Display::fmt(error, formatter),
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
            Self::ModuleCountOutOfRange {
                kind,
                offset,
                count,
                maximum,
            } => write!(
                formatter,
                "{kind:?} module count {count} at byte {offset} exceeds QuickJS positive-int maximum {maximum}"
            ),
            Self::ModuleFieldOutOfRange {
                field,
                offset,
                value,
                maximum,
            } => write!(
                formatter,
                "{field:?} module field {value} at byte {offset} exceeds QuickJS positive-int maximum {maximum}"
            ),
            Self::InvalidModuleState { module_index } => write!(
                formatter,
                "module {module_index} has an invalid whole-image decoder state"
            ),
            Self::AllocationFailed => formatter.write_str("whole-image allocation failed"),
        }
    }
}

impl std::error::Error for BytecodeImageError {}

impl From<WireError> for BytecodeImageError {
    fn from(error: WireError) -> Self {
        Self::Wire(error)
    }
}

impl From<ImageAtomError> for BytecodeImageError {
    fn from(error: ImageAtomError) -> Self {
        Self::Atom(error)
    }
}

impl From<DecodeError<ImageOpaque>> for BytecodeImageError {
    fn from(error: DecodeError<ImageOpaque>) -> Self {
        Self::Data(error)
    }
}

impl From<FunctionEnvelopeError> for BytecodeImageError {
    fn from(error: FunctionEnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

impl From<ModuleBudgetError> for BytecodeImageError {
    fn from(error: ModuleBudgetError) -> Self {
        Self::Module(error)
    }
}

impl From<BytecodeImageBudgetError> for BytecodeImageError {
    fn from(error: BytecodeImageBudgetError) -> Self {
        match error {
            BytecodeImageBudgetError::ResourceLimit {
                kind,
                requested,
                limit,
            } => Self::ResourceLimit {
                kind,
                requested,
                limit,
            },
            BytecodeImageBudgetError::CountOverflow { kind } => Self::CountOverflow { kind },
        }
    }
}

impl From<SabArchiveError> for BytecodeImageError {
    fn from(error: SabArchiveError) -> Self {
        match error {
            SabArchiveError::Wire(error) => Self::Wire(error),
            SabArchiveError::Graph(error) => Self::Data(DecodeError::Graph(error)),
            error => Self::Data(DecodeError::SharedArrayBufferArchive(error)),
        }
    }
}

/// Decode one complete bytecode-mode BC5 image without making it executable.
pub(in crate::runtime) fn decode_bytecode_image(
    input: &[u8],
    mode: ReaderMode,
    wire_limits: WireLimits,
    limits: BytecodeImageLimits,
    allow_object_references: bool,
) -> Result<BytecodeImage, BytecodeImageError> {
    let cursor = WireCursor::new(input, mode, wire_limits)?;
    let (cursor, image) = decode_bytecode_image_body(cursor, limits, allow_object_references)?;
    // This call is unconditional: QuickJsCompatible itself decides to accept
    // trailing bytes, rather than the image layer bypassing finalization.
    cursor.finish()?;
    Ok(image)
}

pub(in crate::runtime::binary_object) fn decode_bytecode_image_body<'input, C>(
    mut cursor: C,
    limits: BytecodeImageLimits,
    allow_object_references: bool,
) -> Result<(C, BytecodeImage), BytecodeImageError>
where
    C: DataCursor<'input>,
{
    let atoms = ImageAtomTable::read(&mut cursor)?;
    let mut machine =
        DataMachine::<ImageValue, ImageKey>::new(limits.graph(), allow_object_references)?;
    let source = machine.source();
    let mut functions = FunctionTable::new(source, limits);
    let mut modules = ModuleTable::new(source, limits);
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
        let return_to = next_target(&mut cursor, &atoms, &mut frames, &mut modules)?;
        let depth = frames
            .len()
            .checked_add(1)
            .ok_or(BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::WholeDepth,
            })?;
        limits.check(BytecodeImageResourceKind::WholeDepth, depth)?;

        let tag_offset = cursor.position();
        let tag = cursor.read_tag()?;
        if tag == BcTag::FunctionBytecode {
            let frame = functions.begin_function(&mut cursor, &atoms, tag_offset)?;
            frames
                .try_reserve(1)
                .map_err(|_| BytecodeImageError::AllocationFailed)?;
            frames.push(ActiveFrame::Function { frame, return_to });
        } else if tag == BcTag::Module {
            let frame = modules.begin_module(&mut cursor, &atoms)?;
            frames
                .try_reserve(1)
                .map_err(|_| BytecodeImageError::AllocationFailed)?;
            frames.push(ActiveFrame::Module { frame, return_to });
        } else {
            match machine.read_value_after_tag(&mut cursor, tag, tag_offset, data_depth)? {
                DataReadStep::Complete(value) => {
                    deliver_completed(&machine, &mut frames, return_to, value, &mut root)?;
                }
                DataReadStep::Pending(frame) => {
                    frames
                        .try_reserve(1)
                        .map_err(|_| BytecodeImageError::AllocationFailed)?;
                    frames.push(ActiveFrame::Data { frame, return_to });
                    data_depth =
                        data_depth
                            .checked_add(1)
                            .ok_or(BytecodeImageError::CountOverflow {
                                kind: BytecodeImageResourceKind::WholeDepth,
                            })?;
                }
            }
        }

        drain_completed(
            &mut machine,
            &mut functions,
            &mut modules,
            &mut frames,
            &mut data_depth,
            &mut root,
        )?;
    }

    // Preserve the ordinary reader's error order: strict trailing input is
    // rejected after the complete recursive record is consumed, but before
    // decoder-owned arenas and function/module tables are finalized. The
    // consuming cursor finalizer runs again at the complete-input boundary;
    // the SAB transport entrypoint consumes the same cursor together with its
    // authenticated occurrence table.
    cursor.validate_wire_end()?;

    let root = root.ok_or(BytecodeImageError::InvalidCompletionTarget)?;
    let output = machine.finish_output()?;
    let root = output.unwrap_completion(root)?;
    let function_records = functions.finish(&output)?;
    let module_records = modules.finish(&output)?;
    let parts = output.into_parts();
    let image = BytecodeImage::new(
        source,
        atoms.into_dynamic_atoms(),
        parts.nodes,
        parts.ref_table,
        function_records,
        module_records,
        root,
    );
    Ok((cursor, image))
}

#[derive(Clone, Copy)]
enum CompletionTarget {
    Root,
    DataParent {
        key: Option<PropertyDisposition<ImageKey>>,
    },
    FunctionConstant,
    ModuleRequestAttributes {
        name: ImageAtom,
    },
    ModuleFunction,
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
    Module {
        frame: ModuleFrame,
        return_to: CompletionTarget,
    },
}

fn next_target<'input, C>(
    cursor: &mut C,
    atoms: &ImageAtomTable,
    frames: &mut [ActiveFrame],
    modules: &mut ModuleTable,
) -> Result<CompletionTarget, BytecodeImageError>
where
    C: CheckedReadCursor<'input>,
{
    match frames.last_mut() {
        None => Ok(CompletionTarget::Root),
        Some(ActiveFrame::Function { .. }) => Ok(CompletionTarget::FunctionConstant),
        Some(ActiveFrame::Data { frame, .. }) => {
            let key = frame
                .expects_property_key()
                .then(|| read_key(cursor, atoms))
                .transpose()?;
            Ok(CompletionTarget::DataParent { key })
        }
        Some(ActiveFrame::Module { frame, .. }) => frame.next_target(cursor, atoms, modules),
    }
}

fn read_atom<'input, C>(
    cursor: &mut C,
    atoms: &ImageAtomTable,
) -> Result<ImageAtom, BytecodeImageError>
where
    C: CheckedReadCursor<'input>,
{
    let offset = cursor.position();
    let raw = atoms.raw_space().decode_metadata_atom(cursor)?;
    Ok(atoms.remap_atom(atoms.raw_space(), raw, offset)?)
}

fn read_key<'input, C>(
    cursor: &mut C,
    atoms: &ImageAtomTable,
) -> Result<PropertyDisposition<ImageKey>, BytecodeImageError>
where
    C: CheckedReadCursor<'input>,
{
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
    modules: &mut ModuleTable,
    frames: &mut Vec<ActiveFrame>,
    data_depth: &mut usize,
    root: &mut Option<DataCompletion<ImageValue>>,
) -> Result<(), BytecodeImageError> {
    loop {
        let complete = match frames.last() {
            Some(ActiveFrame::Data { frame, .. }) => frame.is_complete(),
            Some(ActiveFrame::Function { frame, .. }) => frame.is_complete(),
            Some(ActiveFrame::Module { frame, .. }) => frame.is_complete(),
            None => false,
        };
        if !complete {
            return Ok(());
        }

        let active = frames
            .pop()
            .ok_or(BytecodeImageError::InvalidCompletionTarget)?;
        let (return_to, value) = match active {
            ActiveFrame::Data { frame, return_to } => {
                *data_depth = data_depth
                    .checked_sub(1)
                    .ok_or(BytecodeImageError::InvalidCompletionTarget)?;
                (return_to, machine.finish_frame(frame)?)
            }
            ActiveFrame::Function { frame, return_to } => {
                let function = functions.finish_frame(frame)?;
                let value = machine.wrap_opaque_value(ImageValue::from_function(function))?;
                (return_to, value)
            }
            ActiveFrame::Module { frame, return_to } => {
                let module = modules.finish_frame(frame)?;
                let value = machine.wrap_opaque_value(ImageValue::from_module(module))?;
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
) -> Result<(), BytecodeImageError> {
    match target {
        CompletionTarget::Root => {
            if root.replace(value).is_some() {
                return Err(BytecodeImageError::InvalidCompletionTarget);
            }
        }
        CompletionTarget::DataParent { key } => {
            let Some(ActiveFrame::Data { frame, .. }) = frames.last_mut() else {
                return Err(BytecodeImageError::InvalidCompletionTarget);
            };
            machine.attach_to_frame(frame, key, value)?;
        }
        CompletionTarget::FunctionConstant => {
            let Some(ActiveFrame::Function { frame, .. }) = frames.last_mut() else {
                return Err(BytecodeImageError::InvalidCompletionTarget);
            };
            frame.push_constant(value)?;
        }
        CompletionTarget::ModuleRequestAttributes { name } => {
            let Some(ActiveFrame::Module { frame, .. }) = frames.last_mut() else {
                return Err(BytecodeImageError::InvalidCompletionTarget);
            };
            frame.push_request(name, value)?;
        }
        CompletionTarget::ModuleFunction => {
            let Some(ActiveFrame::Module { frame, .. }) = frames.last_mut() else {
                return Err(BytecodeImageError::InvalidCompletionTarget);
            };
            frame.set_func_obj(value)?;
        }
    }
    Ok(())
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

/// Move-only proof that one same-source module slot was completely filled.
///
/// As for functions, construction stays private to the decoder so a raw local
/// index cannot be rebranded as an opaque whole-image value.
pub(super) struct AuthenticatedModule {
    source: MachineSource,
    index: u32,
}

impl AuthenticatedModule {
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
