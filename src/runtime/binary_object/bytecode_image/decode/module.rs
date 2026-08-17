//! Module-record frames and table state inside the shared whole-image decoder.

use std::num::NonZeroU8;

use super::super::super::graph::decode::{DataCompletion, DataMachineOutput, MachineSource};
use super::super::super::wire::WireCursor;
use super::super::atoms::{ImageAtom, ImageAtomTable, ImageKey};
use super::super::budget::{
    BytecodeImageLimits, BytecodeImageResourceKind, ModuleResourceKind, ModuleTotals, ModuleUsage,
};
use super::super::model::{ImageValue, ModuleExport, ModuleImport, ModuleRecord, ModuleRequest};
use super::{AuthenticatedModule, BytecodeImageError, CompletionTarget, ModuleField, read_atom};

const QUICKJS_POSITIVE_INT_MAX: u32 = i32::MAX as u32;

enum ModulePhase {
    Requests,
    Function,
    Complete,
}

struct PendingModuleRequest {
    name: ImageAtom,
    attributes: DataCompletion<ImageValue>,
}

pub(super) struct ModuleFrame {
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
    pub(super) fn next_target(
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

    pub(super) fn push_request(
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

    pub(super) fn set_func_obj(
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

    pub(super) fn is_complete(&self) -> bool {
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

pub(super) struct ModuleTable {
    source: MachineSource,
    limits: BytecodeImageLimits,
    slots: Vec<Option<PendingModuleRecord>>,
    totals: ModuleTotals,
}

impl ModuleTable {
    pub(super) fn new(source: MachineSource, limits: BytecodeImageLimits) -> Self {
        Self {
            source,
            limits,
            slots: Vec::new(),
            totals: ModuleTotals::default(),
        }
    }

    pub(super) fn begin_module(
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

    pub(super) fn finish_frame(
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

    pub(super) fn finish(
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
