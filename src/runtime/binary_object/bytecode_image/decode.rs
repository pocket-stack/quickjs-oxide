//! Bounded whole-image decoder for QuickJS 2026-06-04 bytecode images.
//!
//! This reader owns one atom table, one data-machine/object arena, one
//! function table, one module table, and one heterogeneous frame stack for the
//! entire input.
//! The resulting image is structural only: no runtime heap object is allocated
//! and no native bytecode is admitted to execution.

use std::fmt;
use std::num::NonZeroU8;

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
use super::atoms::{ImageAtom, ImageAtomError, ImageAtomTable, ImageKey};
use super::budget::{
    BytecodeImageBudgetError, BytecodeImageLimits, BytecodeImageResourceKind, FunctionTotals,
    FunctionUsage, ModuleBudgetError, ModuleResourceKind, ModuleTotals, ModuleUsage,
    RemainingFunctionBudget,
};
use super::model::{
    BytecodeImage, FunctionRecord, ImageClosureVariable, ImageCode, ImageFunctionDebug,
    ImageFunctionEnvelope, ImageInstructionSpan, ImageLocalVariable, ImageOpaque, ImageRelocation,
    ImageValue, ModuleExport, ModuleImport, ModuleRecord, ModuleRequest,
};

const QUICKJS_POSITIVE_INT_MAX: u32 = i32::MAX as u32;

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

/// Decode one complete bytecode-mode BC5 image without making it executable.
pub(in crate::runtime) fn decode_bytecode_image(
    input: &[u8],
    mode: ReaderMode,
    wire_limits: WireLimits,
    limits: BytecodeImageLimits,
    allow_object_references: bool,
) -> Result<BytecodeImage, BytecodeImageError> {
    let mut cursor = WireCursor::new(input, mode, wire_limits)?;
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

    // Strict-vs-compatible trailing-input behavior remains centralized in the
    // same cursor that read every prefix and constant-pool value.
    cursor.finish()?;

    let root = root.ok_or(BytecodeImageError::InvalidCompletionTarget)?;
    let output = machine.finish_output()?;
    let root = output.unwrap_completion(root)?;
    let function_records = functions.finish(&output)?;
    let module_records = modules.finish(&output)?;
    let parts = output.into_parts();
    Ok(BytecodeImage::new(
        source,
        atoms.into_dynamic_atoms(),
        parts.nodes,
        parts.ref_table,
        function_records,
        module_records,
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

fn next_target(
    cursor: &mut WireCursor<'_>,
    atoms: &ImageAtomTable,
    frames: &mut [ActiveFrame],
    modules: &mut ModuleTable,
) -> Result<CompletionTarget, BytecodeImageError> {
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

fn read_atom(
    cursor: &mut WireCursor<'_>,
    atoms: &ImageAtomTable,
) -> Result<ImageAtom, BytecodeImageError> {
    let offset = cursor.position();
    let raw = atoms.raw_space().decode_metadata_atom(cursor)?;
    Ok(atoms.remap_atom(atoms.raw_space(), raw, offset)?)
}

fn read_key(
    cursor: &mut WireCursor<'_>,
    atoms: &ImageAtomTable,
) -> Result<PropertyDisposition<ImageKey>, BytecodeImageError> {
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

enum ModulePhase {
    Requests,
    Function,
    Complete,
}

struct PendingModuleRequest {
    name: ImageAtom,
    attributes: DataCompletion<ImageValue>,
}

struct ModuleFrame {
    module: ModuleSlot,
    name: ImageAtom,
    expected_requests: usize,
    requests: Vec<PendingModuleRequest>,
    exports: Vec<ModuleExport>,
    star_export_request_indices: Vec<u32>,
    imports: Vec<ModuleImport>,
    has_tla: bool,
    func_obj: Option<DataCompletion<ImageValue>>,
    phase: ModulePhase,
}

impl ModuleFrame {
    fn next_target(
        &mut self,
        cursor: &mut WireCursor<'_>,
        atoms: &ImageAtomTable,
        modules: &mut ModuleTable,
    ) -> Result<CompletionTarget, BytecodeImageError> {
        match self.phase {
            ModulePhase::Requests => {
                if self.requests.len() < self.expected_requests {
                    // JS_ReadModule reads each request name immediately before
                    // recursively reading that request's arbitrary attributes.
                    let name = read_atom(cursor, atoms)?;
                    return Ok(CompletionTarget::ModuleRequestAttributes { name });
                }

                self.exports = modules.read_exports(cursor, atoms)?;
                self.star_export_request_indices = modules.read_star_exports(cursor)?;
                self.imports = modules.read_imports(cursor, atoms)?;
                self.has_tla = cursor.read_u8()? != 0;
                self.phase = ModulePhase::Function;
                Ok(CompletionTarget::ModuleFunction)
            }
            ModulePhase::Function | ModulePhase::Complete => {
                Err(BytecodeImageError::InvalidModuleState {
                    module_index: self.module.index,
                })
            }
        }
    }

    fn push_request(
        &mut self,
        name: ImageAtom,
        attributes: DataCompletion<ImageValue>,
    ) -> Result<(), BytecodeImageError> {
        if !matches!(self.phase, ModulePhase::Requests)
            || self.requests.len() >= self.expected_requests
        {
            return Err(BytecodeImageError::InvalidCompletionTarget);
        }
        self.requests
            .push(PendingModuleRequest { name, attributes });
        Ok(())
    }

    fn set_func_obj(
        &mut self,
        func_obj: DataCompletion<ImageValue>,
    ) -> Result<(), BytecodeImageError> {
        if !matches!(self.phase, ModulePhase::Function) || self.func_obj.is_some() {
            return Err(BytecodeImageError::InvalidCompletionTarget);
        }
        self.func_obj = Some(func_obj);
        self.phase = ModulePhase::Complete;
        Ok(())
    }

    fn is_complete(&self) -> bool {
        matches!(self.phase, ModulePhase::Complete)
            && self.requests.len() == self.expected_requests
            && self.func_obj.is_some()
    }
}

struct PendingModuleRecord {
    name: ImageAtom,
    requests: Vec<PendingModuleRequest>,
    exports: Vec<ModuleExport>,
    star_export_request_indices: Vec<u32>,
    imports: Vec<ModuleImport>,
    has_tla: bool,
    func_obj: DataCompletion<ImageValue>,
}

#[derive(Clone, Copy)]
struct ModuleSlot {
    source: MachineSource,
    index: u32,
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

struct ModuleTable {
    source: MachineSource,
    limits: BytecodeImageLimits,
    slots: Vec<Option<PendingModuleRecord>>,
    totals: ModuleTotals,
}

impl ModuleTable {
    fn new(source: MachineSource, limits: BytecodeImageLimits) -> Self {
        Self {
            source,
            limits,
            slots: Vec::new(),
            totals: ModuleTotals::default(),
        }
    }

    fn begin_module(
        &mut self,
        cursor: &mut WireCursor<'_>,
        atoms: &ImageAtomTable,
    ) -> Result<ModuleFrame, BytecodeImageError> {
        let requested =
            self.slots
                .len()
                .checked_add(1)
                .ok_or(BytecodeImageError::CountOverflow {
                    kind: BytecodeImageResourceKind::Modules,
                })?;
        self.limits
            .check(BytecodeImageResourceKind::Modules, requested)?;

        // No allocation is attempted until both the per-record request limit
        // and the remaining aggregate request budget have admitted the count.
        let name = read_atom(cursor, atoms)?;
        let expected_requests = self.read_count(cursor, ModuleResourceKind::Requests)?;

        let index =
            u32::try_from(self.slots.len()).map_err(|_| BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::Modules,
            })?;
        self.slots
            .try_reserve(1)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        self.slots.push(None);

        let mut requests = Vec::new();
        requests
            .try_reserve_exact(expected_requests)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        Ok(ModuleFrame {
            module: ModuleSlot {
                source: self.source,
                index,
            },
            name,
            expected_requests,
            requests,
            exports: Vec::new(),
            star_export_request_indices: Vec::new(),
            imports: Vec::new(),
            has_tla: false,
            func_obj: None,
            phase: ModulePhase::Requests,
        })
    }

    fn read_exports(
        &mut self,
        cursor: &mut WireCursor<'_>,
        atoms: &ImageAtomTable,
    ) -> Result<Vec<ModuleExport>, BytecodeImageError> {
        let count = self.read_count(cursor, ModuleResourceKind::Exports)?;
        let mut exports = Vec::new();
        exports
            .try_reserve_exact(count)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for _ in 0..count {
            let export_type = cursor.read_u8()?;
            if export_type == 0 {
                let variable_index = read_module_field(cursor, ModuleField::LocalExportVariable)?;
                let export_name = read_atom(cursor, atoms)?;
                exports.push(ModuleExport::new_local(variable_index, export_name));
            } else {
                let request_index = read_module_field(cursor, ModuleField::IndirectExportRequest)?;
                let local_name = read_atom(cursor, atoms)?;
                let export_name = read_atom(cursor, atoms)?;
                let export_type = NonZeroU8::new(export_type)
                    .ok_or(BytecodeImageError::InvalidCompletionTarget)?;
                exports.push(ModuleExport::new_non_local(
                    export_type,
                    request_index,
                    local_name,
                    export_name,
                ));
            }
        }
        Ok(exports)
    }

    fn read_star_exports(
        &mut self,
        cursor: &mut WireCursor<'_>,
    ) -> Result<Vec<u32>, BytecodeImageError> {
        let count = self.read_count(cursor, ModuleResourceKind::StarExports)?;
        let mut request_indices = Vec::new();
        request_indices
            .try_reserve_exact(count)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for _ in 0..count {
            request_indices.push(read_module_field(cursor, ModuleField::StarExportRequest)?);
        }
        Ok(request_indices)
    }

    fn read_imports(
        &mut self,
        cursor: &mut WireCursor<'_>,
        atoms: &ImageAtomTable,
    ) -> Result<Vec<ModuleImport>, BytecodeImageError> {
        let count = self.read_count(cursor, ModuleResourceKind::Imports)?;
        let mut imports = Vec::new();
        imports
            .try_reserve_exact(count)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for _ in 0..count {
            let variable_index = read_module_field(cursor, ModuleField::ImportVariable)?;
            let is_star = cursor.read_u8()? != 0;
            let import_name = read_atom(cursor, atoms)?;
            let request_index = read_module_field(cursor, ModuleField::ImportRequest)?;
            imports.push(ModuleImport::new(
                variable_index,
                is_star,
                import_name,
                request_index,
            ));
        }
        Ok(imports)
    }

    fn read_count(
        &mut self,
        cursor: &mut WireCursor<'_>,
        kind: ModuleResourceKind,
    ) -> Result<usize, BytecodeImageError> {
        let offset = cursor.position();
        let raw = cursor.read_uleb128()?;
        if raw > QUICKJS_POSITIVE_INT_MAX {
            return Err(BytecodeImageError::ModuleCountOutOfRange {
                kind,
                offset,
                count: raw,
                maximum: QUICKJS_POSITIVE_INT_MAX,
            });
        }
        let count = raw as usize;
        self.charge_count(kind, count)?;
        Ok(count)
    }

    fn charge_count(
        &mut self,
        kind: ModuleResourceKind,
        count: usize,
    ) -> Result<(), BytecodeImageError> {
        let remaining = self.totals.remaining(self.limits)?;
        let effective = remaining.intersect(self.limits.module());
        if let Err(error) = effective.check(kind, count) {
            return Err(
                match self
                    .totals
                    .aggregate_error_for_module(&error, remaining, self.limits)
                {
                    Some(error) => error.into(),
                    None => error.into(),
                },
            );
        }
        let usage = match kind {
            ModuleResourceKind::Requests => ModuleUsage::new(count, 0, 0, 0),
            ModuleResourceKind::Exports => ModuleUsage::new(0, count, 0, 0),
            ModuleResourceKind::StarExports => ModuleUsage::new(0, 0, count, 0),
            ModuleResourceKind::Imports => ModuleUsage::new(0, 0, 0, count),
        };
        self.totals = self.totals.checked_add(usage, self.limits)?;
        Ok(())
    }

    fn finish_frame(
        &mut self,
        frame: ModuleFrame,
    ) -> Result<AuthenticatedModule, BytecodeImageError> {
        let index = frame.module.index;
        if frame.module.source != self.source || !frame.is_complete() {
            return Err(BytecodeImageError::InvalidModuleState {
                module_index: index,
            });
        }
        let Some(slot) = self.slots.get_mut(index as usize) else {
            return Err(BytecodeImageError::InvalidModuleState {
                module_index: index,
            });
        };
        if slot.is_some() {
            return Err(BytecodeImageError::InvalidModuleState {
                module_index: index,
            });
        }
        let func_obj = frame
            .func_obj
            .ok_or(BytecodeImageError::InvalidModuleState {
                module_index: index,
            })?;
        *slot = Some(PendingModuleRecord {
            name: frame.name,
            requests: frame.requests,
            exports: frame.exports,
            star_export_request_indices: frame.star_export_request_indices,
            imports: frame.imports,
            has_tla: frame.has_tla,
            func_obj,
        });
        Ok(AuthenticatedModule::new(frame.module.source, index))
    }

    fn finish(
        self,
        output: &DataMachineOutput<ImageValue, ImageKey>,
    ) -> Result<Box<[ModuleRecord]>, BytecodeImageError> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(self.slots.len())
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for (index, slot) in self.slots.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::Modules,
            })?;
            let pending = slot.ok_or(BytecodeImageError::InvalidModuleState {
                module_index: index,
            })?;
            let mut requests = Vec::new();
            requests
                .try_reserve_exact(pending.requests.len())
                .map_err(|_| BytecodeImageError::AllocationFailed)?;
            for request in pending.requests {
                requests.push(ModuleRequest::new(
                    request.name,
                    output.unwrap_completion(request.attributes)?,
                ));
            }
            records.push(ModuleRecord::new(
                pending.name,
                requests.into_boxed_slice(),
                pending.exports.into_boxed_slice(),
                pending.star_export_request_indices.into_boxed_slice(),
                pending.imports.into_boxed_slice(),
                pending.has_tla,
                output.unwrap_completion(pending.func_obj)?,
            ));
        }
        Ok(records.into_boxed_slice())
    }
}

fn read_module_field(
    cursor: &mut WireCursor<'_>,
    field: ModuleField,
) -> Result<u32, BytecodeImageError> {
    let offset = cursor.position();
    let value = cursor.read_uleb128()?;
    if value > QUICKJS_POSITIVE_INT_MAX {
        return Err(BytecodeImageError::ModuleFieldOutOfRange {
            field,
            offset,
            value,
            maximum: QUICKJS_POSITIVE_INT_MAX,
        });
    }
    Ok(value)
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
    ) -> Result<(), BytecodeImageError> {
        if self.constants.len() >= self.expected_constants {
            return Err(BytecodeImageError::InvalidCompletionTarget);
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
    limits: BytecodeImageLimits,
    slots: Vec<Option<PendingFunctionRecord>>,
    totals: FunctionTotals,
}

impl FunctionTable {
    fn new(source: MachineSource, limits: BytecodeImageLimits) -> Self {
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
    ) -> Result<FunctionFrame, BytecodeImageError> {
        let requested =
            self.slots
                .len()
                .checked_add(1)
                .ok_or(BytecodeImageError::CountOverflow {
                    kind: BytecodeImageResourceKind::Functions,
                })?;
        self.limits
            .check(BytecodeImageResourceKind::Functions, requested)?;
        let index =
            u32::try_from(self.slots.len()).map_err(|_| BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::Functions,
            })?;
        self.slots
            .try_reserve(1)
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
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
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
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
    ) -> Result<AuthenticatedFunction, BytecodeImageError> {
        if frame.function.source != self.source || !frame.is_complete() {
            return Err(BytecodeImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        }
        let Some(slot) = self.slots.get_mut(frame.function.index as usize) else {
            return Err(BytecodeImageError::InvalidFunctionState {
                function_index: frame.function.index,
            });
        };
        if slot.is_some() {
            return Err(BytecodeImageError::InvalidFunctionState {
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
    ) -> Result<Box<[FunctionRecord]>, BytecodeImageError> {
        let mut records = Vec::new();
        records
            .try_reserve_exact(self.slots.len())
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
        for (index, slot) in self.slots.into_iter().enumerate() {
            let index = u32::try_from(index).map_err(|_| BytecodeImageError::CountOverflow {
                kind: BytecodeImageResourceKind::Functions,
            })?;
            let pending = slot.ok_or(BytecodeImageError::InvalidFunctionState {
                function_index: index,
            })?;
            let mut constants = Vec::new();
            constants
                .try_reserve_exact(pending.constants.len())
                .map_err(|_| BytecodeImageError::AllocationFailed)?;
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
    ) -> Result<ImageFunctionEnvelope, BytecodeImageError> {
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
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
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
            .map_err(|_| BytecodeImageError::AllocationFailed)?;
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
    ) -> Result<FunctionTotals, BytecodeImageError> {
        let additional_debug_bytes = match envelope.debug() {
            Some(debug) => debug
                .pc2line()
                .len()
                .checked_add(debug.source().len())
                .ok_or(BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::TotalDebugBytes,
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
    ) -> BytecodeImageError {
        match self
            .totals
            .aggregate_error_for_envelope(&error, remaining, self.limits)
        {
            Some(error) => error.into(),
            None => BytecodeImageError::Envelope(error),
        }
    }
}

fn relocate_code(
    atoms: &ImageAtomTable,
    parts: CodeImageParts,
) -> Result<ImageCode, BytecodeImageError> {
    let mut instructions = Vec::new();
    instructions
        .try_reserve_exact(parts.instructions.len())
        .map_err(|_| BytecodeImageError::AllocationFailed)?;
    for instruction in parts.instructions {
        instructions.push(ImageInstructionSpan::new(
            instruction.offset(),
            instruction.opcode(),
        ));
    }

    let mut relocations = Vec::new();
    relocations
        .try_reserve_exact(parts.atom_relocations.len())
        .map_err(|_| BytecodeImageError::AllocationFailed)?;
    for relocation in parts.atom_relocations {
        let offset = parts
            .payload_offset
            .checked_add(relocation.operand_offset() as usize)
            .ok_or(BytecodeImageError::OffsetOverflow {
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
