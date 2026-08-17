//! Module-record planning inside the shared whole-image writer state machine.

use super::super::super::super::wire::BcTag;
use super::super::super::budget::{
    BytecodeImageBudgetError, BytecodeImageResourceKind, ModuleResourceKind, ModuleUsage,
};
use super::super::super::model::{ModuleId, ModuleRecord};
use super::super::{BytecodeImageEncodeError, ModuleIntegerField};
use super::{MAX_QUICKJS_POSITIVE_INT, PlanBuilder, PlanTask, ValueRef};

impl<'a> PlanBuilder<'a> {
    pub(super) fn plan_module(
        &mut self,
        module: ModuleId,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let record = self.module_record(module)?;
        if self.active_modules.contains(&module) {
            return Err(BytecodeImageEncodeError::CircularModule {
                module_index: module.zero_based(),
            });
        }
        self.charge_module_occurrence()?;
        if !self.seen_modules.contains(&module) {
            let expected = u32::try_from(self.seen_modules.len()).map_err(|_| {
                BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::Modules,
                }
            })?;
            if module.zero_based() != expected {
                return Err(BytecodeImageEncodeError::ModulePreorder {
                    expected,
                    found: module.zero_based(),
                });
            }
            self.seen_modules
                .try_reserve(1)
                .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
            self.seen_modules.insert(module);
        }
        self.active_modules
            .try_reserve(1)
            .map_err(|_| BytecodeImageEncodeError::AllocationFailed)?;
        self.active_modules.insert(module);

        self.push_u8(BcTag::Module.to_byte())?;
        self.plan_atom(record.name())?;
        let request_count = module_positive_count(
            module,
            record.requests().len(),
            ModuleIntegerField::RequestCount,
        )?;
        self.charge_module_entries(ModuleResourceKind::Requests, record.requests().len())?;
        self.push_uleb(request_count)?;
        self.push_task(PlanTask::ContinueModuleRequests {
            module,
            next_request: 0,
            whole_depth,
            graph_parent_depth,
        })
    }

    pub(super) fn continue_module_requests(
        &mut self,
        module: ModuleId,
        next_request: usize,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let record = self.module_record(module)?;
        if let Some(request) = record.requests().get(next_request) {
            self.plan_atom(request.name())?;
            let following_request = next_request
                .checked_add(1)
                .ok_or(BytecodeImageEncodeError::EncodedLengthOverflow)?;
            self.reserve_tasks(Some(2))?;
            self.tasks.push(PlanTask::ContinueModuleRequests {
                module,
                next_request: following_request,
                whole_depth,
                graph_parent_depth,
            });
            self.tasks.push(PlanTask::Value {
                value: ValueRef::Image(request.attributes()),
                whole_parent_depth: whole_depth,
                graph_parent_depth,
            });
            return Ok(());
        }
        self.plan_module_tail(module, record, whole_depth, graph_parent_depth)
    }

    fn plan_module_tail(
        &mut self,
        module: ModuleId,
        record: &'a ModuleRecord,
        whole_depth: usize,
        graph_parent_depth: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let export_count = module_positive_count(
            module,
            record.exports().len(),
            ModuleIntegerField::ExportCount,
        )?;
        self.charge_module_entries(ModuleResourceKind::Exports, record.exports().len())?;
        self.push_uleb(export_count)?;
        for (export_index, export) in record.exports().iter().enumerate() {
            let export_type = export.export_type();
            self.push_u8(export_type)?;
            if export_type == 0 {
                let variable_index = export.local_variable_index().ok_or(
                    BytecodeImageEncodeError::InvalidModuleExport {
                        module_index: module.zero_based(),
                        export_index,
                    },
                )?;
                self.push_uleb(module_positive_index(
                    module,
                    variable_index,
                    ModuleIntegerField::ExportVariableIndex,
                )?)?;
            } else {
                let request_index = export.request_index().ok_or(
                    BytecodeImageEncodeError::InvalidModuleExport {
                        module_index: module.zero_based(),
                        export_index,
                    },
                )?;
                self.push_uleb(module_positive_index(
                    module,
                    request_index,
                    ModuleIntegerField::ExportRequestIndex,
                )?)?;
                let local_name =
                    export
                        .local_name()
                        .ok_or(BytecodeImageEncodeError::InvalidModuleExport {
                            module_index: module.zero_based(),
                            export_index,
                        })?;
                self.plan_atom(local_name)?;
            }
            self.plan_atom(export.export_name())?;
        }

        let star_export_count = module_positive_count(
            module,
            record.star_export_request_indices().len(),
            ModuleIntegerField::StarExportCount,
        )?;
        self.charge_module_entries(
            ModuleResourceKind::StarExports,
            record.star_export_request_indices().len(),
        )?;
        self.push_uleb(star_export_count)?;
        for request_index in record.star_export_request_indices() {
            self.push_uleb(module_positive_index(
                module,
                *request_index,
                ModuleIntegerField::StarExportRequestIndex,
            )?)?;
        }

        let import_count = module_positive_count(
            module,
            record.imports().len(),
            ModuleIntegerField::ImportCount,
        )?;
        self.charge_module_entries(ModuleResourceKind::Imports, record.imports().len())?;
        self.push_uleb(import_count)?;
        for import in record.imports() {
            self.push_uleb(module_positive_index(
                module,
                import.variable_index(),
                ModuleIntegerField::ImportVariableIndex,
            )?)?;
            self.push_u8(u8::from(import.is_star()))?;
            self.plan_atom(import.import_name())?;
            self.push_uleb(module_positive_index(
                module,
                import.request_index(),
                ModuleIntegerField::ImportRequestIndex,
            )?)?;
        }
        self.push_u8(u8::from(record.has_tla()))?;

        // QuickJS writes the function object recursively only after every
        // metadata table. Keep the module active until that value completes so
        // Function/Module mixed cycles are rejected without involving the
        // ordinary-object reference state.
        self.reserve_tasks(Some(2))?;
        self.tasks.push(PlanTask::LeaveModule(module));
        self.tasks.push(PlanTask::Value {
            value: ValueRef::Image(record.func_obj()),
            whole_parent_depth: whole_depth,
            graph_parent_depth,
        });
        Ok(())
    }

    fn module_record(
        &self,
        module: ModuleId,
    ) -> Result<&'a ModuleRecord, BytecodeImageEncodeError> {
        self.image
            .module(module)
            .ok_or(BytecodeImageEncodeError::ForeignModule {
                module_index: module.zero_based(),
            })
    }

    fn charge_module_occurrence(&mut self) -> Result<(), BytecodeImageEncodeError> {
        let requested =
            self.emitted_modules
                .checked_add(1)
                .ok_or(BytecodeImageBudgetError::CountOverflow {
                    kind: BytecodeImageResourceKind::Modules,
                })?;
        self.options
            .limits
            .check(BytecodeImageResourceKind::Modules, requested)?;
        self.emitted_modules = requested;
        Ok(())
    }

    fn charge_module_entries(
        &mut self,
        kind: ModuleResourceKind,
        requested: usize,
    ) -> Result<(), BytecodeImageEncodeError> {
        let remaining = self.module_totals.remaining(self.options.limits)?;
        let module_limits = remaining.intersect(self.options.limits.module());
        if let Err(error) = module_limits.check(kind, requested) {
            if let Some(error) = self.module_totals.aggregate_error_for_module(
                &error,
                remaining,
                self.options.limits,
            ) {
                return Err(error.into());
            }
            return Err(error.into());
        }
        let usage = match kind {
            ModuleResourceKind::Requests => ModuleUsage::new(requested, 0, 0, 0),
            ModuleResourceKind::Exports => ModuleUsage::new(0, requested, 0, 0),
            ModuleResourceKind::StarExports => ModuleUsage::new(0, 0, requested, 0),
            ModuleResourceKind::Imports => ModuleUsage::new(0, 0, 0, requested),
        };
        self.module_totals = self.module_totals.checked_add(usage, self.options.limits)?;
        Ok(())
    }
}

fn module_positive_count(
    module: ModuleId,
    value: usize,
    field: ModuleIntegerField,
) -> Result<u32, BytecodeImageEncodeError> {
    if value > MAX_QUICKJS_POSITIVE_INT {
        return Err(BytecodeImageEncodeError::ModuleIntegerOutOfRange {
            module_index: module.zero_based(),
            field,
            value: u64::try_from(value).unwrap_or(u64::MAX),
        });
    }
    Ok(value as u32)
}

fn module_positive_index(
    module: ModuleId,
    value: u32,
    field: ModuleIntegerField,
) -> Result<u32, BytecodeImageEncodeError> {
    if value > i32::MAX as u32 {
        return Err(BytecodeImageEncodeError::ModuleIntegerOutOfRange {
            module_index: module.zero_based(),
            field,
            value: u64::from(value),
        });
    }
    Ok(value)
}
