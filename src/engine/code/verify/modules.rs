//! Module table and canonical link-entry authentication.
use super::flow::explicit_control_flow_target;
use super::{module_initializer_flow, unlinked_closure_name};
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::bytecode::verify_parts;
use crate::engine::code::function::UnlinkedConstant;
use crate::engine::code::function::metadata::{ClosureSource, ClosureVariableKind};
use crate::engine::code::module::{
    ModuleExportTarget, ModuleImportCollisionDeclaration, ModuleImportName,
    ModuleLinkInitializerValue, UnlinkedModule,
};
use crate::engine::value::{JsString, Value};
use std::collections::HashSet;

pub(super) fn verify_unlinked_module_tables(module: &UnlinkedModule) -> Result<(), RuntimeError> {
    let function = module.function();
    let descriptors = function.closure_variables();
    let request_count = module.requested_modules().len();
    let request_exists = |index: crate::engine::code::module::ModuleRequestIndex| {
        usize::try_from(index.0)
            .ok()
            .is_some_and(|index| index < request_count)
    };

    let mut imported_slots = HashSet::new();
    let mut namespace_slots = HashSet::new();
    let mut import_meta_slot = None;
    for (index, descriptor) in descriptors.iter().enumerate() {
        if descriptor.source != ClosureSource::ModuleImportMeta {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "import.meta descriptor index exceeds bytecode range",
            ))
        })?;
        if import_meta_slot.replace(index).is_some() {
            return Err(RuntimeError::Engine(Error::internal(
                "module contains more than one import.meta binding",
            )));
        }
        if !descriptor.is_lexical
            || !descriptor.is_const
            || descriptor.kind != ClosureVariableKind::Normal
            || unlinked_closure_name(function, descriptor)?
                != Some(&JsString::from_static(
                    crate::engine::code::module::MODULE_IMPORT_META_BINDING_NAME,
                ))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "import.meta descriptor has invalid binding metadata",
            )));
        }
    }
    for import in module.imports() {
        if !request_exists(import.request) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import referenced a missing request",
            )));
        }
        if !imported_slots.insert(import.closure_index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import table reused a closure slot",
            )));
        }
        let is_namespace = matches!(&import.import_name, ModuleImportName::Namespace);
        if is_namespace {
            namespace_slots.insert(import.closure_index);
        }
        let is_collision = module
            .import_collisions()
            .iter()
            .any(|collision| collision.closure_index == import.closure_index);
        let descriptor = descriptors
            .get(usize::from(import.closure_index))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "module import closure index is out of bounds",
                ))
            })?;
        let expected_source = if is_collision {
            ClosureSource::ModuleImportCollision
        } else if is_namespace {
            ClosureSource::ModuleDeclaration
        } else {
            ClosureSource::ModuleImport
        };
        let expected_kind = if is_namespace {
            ClosureVariableKind::Normal
        } else {
            ClosureVariableKind::ModuleImportView
        };
        if descriptor.source != expected_source
            || !descriptor.is_lexical
            || !descriptor.is_const
            || descriptor.kind != expected_kind
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module import table disagrees with its closure descriptor",
            )));
        }
    }

    for (index, descriptor) in descriptors.iter().enumerate() {
        if !matches!(
            descriptor.source,
            ClosureSource::ModuleImport | ClosureSource::ModuleImportCollision
        ) {
            continue;
        }
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "module import descriptor index exceeds bytecode range",
            ))
        })?;
        if !imported_slots.contains(&index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import descriptor has no import-table entry",
            )));
        }
        if descriptor.source == ClosureSource::ModuleImportCollision
            && !module
                .import_collisions()
                .iter()
                .any(|collision| collision.closure_index == index)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision descriptor has no collision ledger entry",
            )));
        }
    }

    let mut collision_declarations = vec![None; descriptors.len()];
    for collision in module.import_collisions() {
        if !imported_slots.contains(&collision.closure_index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision has no import-table entry",
            )));
        }
        let declaration = collision_declarations
            .get_mut(usize::from(collision.closure_index))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "module import collision closure index is out of bounds",
                ))
            })?;
        if declaration.replace(collision.declaration).is_some() {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision ledger reused a closure slot",
            )));
        }
    }

    let expected_declaration_slots = descriptors
        .iter()
        .enumerate()
        .filter_map(|(index, descriptor)| {
            let index = u16::try_from(index).ok()?;
            (descriptor.source == ClosureSource::ModuleImportCollision
                || descriptor.source == ClosureSource::ModuleDeclaration
                    && (!descriptor.is_lexical || !namespace_slots.contains(&index)))
            .then_some(index)
        })
        .collect::<HashSet<_>>();
    let mut seen_declarations = HashSet::with_capacity(module.declaration_order().len());
    for closure_index in module.declaration_order() {
        if !expected_declaration_slots.contains(closure_index)
            || !seen_declarations.insert(*closure_index)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module declaration-order ledger disagrees with descriptors",
            )));
        }
    }
    if seen_declarations != expected_declaration_slots {
        return Err(RuntimeError::Engine(Error::internal(
            "module declaration-order ledger omitted a declaration",
        )));
    }
    let ordered_collision_slots = module.declaration_order().iter().copied().filter(|index| {
        descriptors
            .get(usize::from(*index))
            .is_some_and(|descriptor| descriptor.source == ClosureSource::ModuleImportCollision)
    });
    if !module
        .import_collisions()
        .iter()
        .map(|collision| collision.closure_index)
        .eq(ordered_collision_slots)
    {
        return Err(RuntimeError::Engine(Error::internal(
            "module import collision ledger is not in declaration order",
        )));
    }

    let mut lexical_initializer_counts = vec![0_u8; descriptors.len()];
    let mut collision_initializer_counts = vec![0_u8; descriptors.len()];
    for instruction in function.code() {
        let (index, counts, message) = match instruction {
            crate::engine::code::bytecode::Instruction::InitializeVarRef(index) => (
                index,
                &mut lexical_initializer_counts,
                "module lexical initializer count overflowed",
            ),
            crate::engine::code::bytecode::Instruction::InitializeModuleImportCollision(index) => (
                index,
                &mut collision_initializer_counts,
                "module import collision initializer count overflowed",
            ),
            _ => continue,
        };
        let count = counts.get_mut(usize::from(*index)).ok_or_else(|| {
            RuntimeError::Engine(Error::internal(
                "module initializer is outside closure slots",
            ))
        })?;
        *count = count
            .checked_add(1)
            .ok_or_else(|| RuntimeError::Engine(Error::internal(message)))?;
    }
    for (index, (descriptor, count)) in descriptors
        .iter()
        .zip(lexical_initializer_counts)
        .enumerate()
    {
        let index = u16::try_from(index).map_err(|_| {
            RuntimeError::Engine(Error::internal(
                "module declaration descriptor index exceeds bytecode range",
            ))
        })?;
        let expected = u8::from(
            descriptor.source == ClosureSource::ModuleDeclaration
                && descriptor.is_lexical
                && !namespace_slots.contains(&index),
        );
        if count != expected {
            return Err(RuntimeError::Engine(Error::internal(
                "module lexical initializer count disagrees with declarations",
            )));
        }
    }
    for (declaration, count) in collision_declarations
        .iter()
        .copied()
        .zip(collision_initializer_counts)
    {
        let expected = match declaration {
            Some(
                ModuleImportCollisionDeclaration::Lexical
                | ModuleImportCollisionDeclaration::Function,
            ) => 1,
            Some(ModuleImportCollisionDeclaration::Var) | None => 0,
        };
        if count != expected {
            return Err(RuntimeError::Engine(Error::internal(
                "module import collision initializer count disagrees with its ledger",
            )));
        }
    }

    let mut exported_names = Vec::with_capacity(module.exports().len());
    for export in module.exports() {
        if exported_names
            .iter()
            .any(|name| name == &export.export_name)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module export table contains a duplicate exported name",
            )));
        }
        exported_names.push(export.export_name.clone());
        match &export.target {
            ModuleExportTarget::Local { closure_index } => {
                let descriptor = descriptors
                    .get(usize::from(*closure_index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "local module export closure index is out of bounds",
                        ))
                    })?;
                if !matches!(
                    descriptor.source,
                    ClosureSource::ModuleDeclaration
                        | ClosureSource::ModuleImport
                        | ClosureSource::ModuleImportCollision
                ) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "local module export referenced a non-module binding",
                    )));
                }
            }
            ModuleExportTarget::Indirect {
                request,
                import_name: _,
            } => {
                if !request_exists(*request) {
                    return Err(RuntimeError::Engine(Error::internal(
                        "indirect module export referenced a missing request",
                    )));
                }
            }
        }
    }
    for export in module.star_exports() {
        if !request_exists(export.request) {
            return Err(RuntimeError::Engine(Error::internal(
                "star module export referenced a missing request",
            )));
        }
    }
    Ok(())
}

pub(super) fn verify_module_link_entry(module: &UnlinkedModule) -> Result<(), RuntimeError> {
    let function = module.function();
    let code = function.code();
    if !matches!(
        code.get(code.len().saturating_sub(2)..),
        Some([
            crate::engine::code::bytecode::Instruction::Undefined,
            crate::engine::code::bytecode::Instruction::Return
        ])
    ) {
        return Err(RuntimeError::Engine(Error::internal(
            "module body has no canonical undefined return",
        )));
    }
    let Some(
        [
            crate::engine::code::bytecode::Instruction::PushThis,
            crate::engine::code::bytecode::Instruction::IfFalse(body),
        ],
    ) = code.get(0..2)
    else {
        return Err(RuntimeError::Engine(Error::internal(
            "module bytecode has no link-entry guard",
        )));
    };
    let body = usize::try_from(*body).map_err(|_| {
        RuntimeError::Engine(Error::internal(
            "module link-entry target is outside bytecode",
        ))
    })?;
    if body < 4 || body >= code.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "module link-entry target is outside bytecode",
        )));
    }
    if !matches!(
        code.get(body - 2..body),
        Some([
            crate::engine::code::bytecode::Instruction::Undefined,
            crate::engine::code::bytecode::Instruction::Return
        ])
    ) {
        return Err(RuntimeError::Engine(Error::internal(
            "module link entry has no undefined return",
        )));
    }
    for (pc, instruction) in code.iter().enumerate() {
        let Some(target) = explicit_control_flow_target(instruction) else {
            continue;
        };
        if pc >= body && target < body {
            return Err(RuntimeError::Engine(Error::internal(
                "module evaluation control flow entered the link phase",
            )));
        }
        if pc < body
            && (pc != 1
                || !matches!(
                    instruction,
                    crate::engine::code::bytecode::Instruction::IfFalse(_)
                )
                || target != body)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "module link phase contains non-canonical control flow",
            )));
        }
    }
    // Establish one stack-valid unwind/Gosub shape per PC before the
    // module-specific summary analysis projects and follows that shape.
    verify_parts(
        function.code(),
        function.constants().len(),
        function.metadata().max_stack,
    )?;
    module_initializer_flow::verify(module, body)?;

    let function_collision_slots = module
        .import_collisions()
        .iter()
        .filter_map(|collision| {
            (collision.declaration == ModuleImportCollisionDeclaration::Function)
                .then_some(collision.closure_index)
        })
        .collect::<HashSet<_>>();
    let expected_slots = function.closure_variables();
    let expected_slots = module
        .declaration_order()
        .iter()
        .copied()
        .filter(|index| {
            let descriptor = expected_slots.get(usize::from(*index));
            let Some(descriptor) = descriptor else {
                return false;
            };
            descriptor.source == ClosureSource::ModuleDeclaration && !descriptor.is_lexical
                || function_collision_slots.contains(index)
        })
        .collect::<Vec<_>>();
    let initializers = module.link_initializers();
    if initializers.len() != expected_slots.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "module link initializer ledger disagrees with declarations",
        )));
    }
    let mut seen_slots = HashSet::with_capacity(initializers.len());
    for initializer in initializers {
        if !seen_slots.insert(initializer.closure_index) {
            return Err(RuntimeError::Engine(Error::internal(
                "module link initializer ledger reused a closure slot",
            )));
        }
        let descriptor = function
            .closure_variables()
            .get(usize::from(initializer.closure_index))
            .ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "module link initializer closure index is out of bounds",
                ))
            })?;
        let collision = function_collision_slots.contains(&initializer.closure_index);
        let ordinary_declaration = descriptor.source == ClosureSource::ModuleDeclaration
            && !descriptor.is_lexical
            && !descriptor.is_const
            && descriptor.kind == ClosureVariableKind::Normal;
        if !collision && !ordinary_declaration {
            return Err(RuntimeError::Engine(Error::internal(
                "module link initializer disagrees with its closure descriptor",
            )));
        }
    }
    if !initializers
        .iter()
        .map(|initializer| initializer.closure_index)
        .eq(expected_slots.iter().copied())
    {
        return Err(RuntimeError::Engine(Error::internal(
            "module link initializer ledger is not in declaration order",
        )));
    }

    let initializer_code = &code[2..body - 2];
    let mut cursor = 0;
    for initializer in initializers {
        let value_is_exact = match initializer.value {
            ModuleLinkInitializerValue::Undefined => {
                let matches = matches!(
                    initializer_code.get(cursor),
                    Some(crate::engine::code::bytecode::Instruction::Undefined)
                ) && !function_collision_slots.contains(&initializer.closure_index);
                cursor = cursor.saturating_add(1);
                matches
            }
            ModuleLinkInitializerValue::Function {
                constant,
                inferred_name,
            } => {
                let child = usize::try_from(constant)
                    .ok()
                    .and_then(|index| function.constants().get(index))
                    .and_then(UnlinkedConstant::as_child)
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "module function initializer referenced non-function bytecode",
                        ))
                    })?;
                let descriptor = function
                    .closure_variables()
                    .get(usize::from(initializer.closure_index))
                    .ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "module function initializer descriptor is out of bounds",
                        ))
                    })?;
                let descriptor_name =
                    unlinked_closure_name(function, descriptor)?.ok_or_else(|| {
                        RuntimeError::Engine(Error::internal(
                            "module function initializer descriptor has no name",
                        ))
                    })?;
                let declaration_child_is_exact =
                    child.metadata().strict && child.metadata().function_name_local.is_none();
                let binding_is_exact = declaration_child_is_exact
                    && if inferred_name.is_some() {
                        let mut local_exports = module.exports().iter().filter(|export| {
                            matches!(
                                &export.target,
                                ModuleExportTarget::Local { closure_index }
                                    if *closure_index == initializer.closure_index
                            )
                        });
                        local_exports.next().is_some_and(|export| {
                            export.export_name == JsString::from_static("default")
                        }) && local_exports.next().is_none()
                            && descriptor_name
                                == &JsString::from_static(
                                    crate::engine::code::module::MODULE_DEFAULT_BINDING_NAME,
                                )
                            && child.func_name().is_none()
                    } else {
                        descriptor_name
                            != &JsString::from_static(
                                crate::engine::code::module::MODULE_DEFAULT_BINDING_NAME,
                            )
                            && child.func_name() == Some(descriptor_name)
                    };
                let matches = matches!(
                    initializer_code.get(cursor),
                    Some(crate::engine::code::bytecode::Instruction::FClosure(actual))
                        if *actual == constant
                );
                cursor = cursor.saturating_add(1);
                if let Some(name) = inferred_name {
                    let name_is_default = usize::try_from(name)
                        .ok()
                        .and_then(|index| function.constants().get(index))
                        .and_then(|constant| constant.as_primitive())
                        .is_some_and(|value| {
                            value == &Value::String(JsString::from_static("default"))
                        });
                    let set_name_is_exact = matches!(
                        initializer_code.get(cursor),
                        Some(crate::engine::code::bytecode::Instruction::SetName(actual)) if *actual == name
                    );
                    cursor = cursor.saturating_add(1);
                    matches && binding_is_exact && name_is_default && set_name_is_exact
                } else {
                    matches && binding_is_exact
                }
            }
        };
        let target_is_exact = if function_collision_slots.contains(&initializer.closure_index) {
            matches!(
                initializer_code.get(cursor),
                Some(crate::engine::code::bytecode::Instruction::InitializeModuleImportCollision(slot))
                    if *slot == initializer.closure_index
            )
        } else {
            matches!(
                initializer_code.get(cursor),
                Some(crate::engine::code::bytecode::Instruction::PutVarRef(slot))
                    if *slot == initializer.closure_index
            )
        };
        cursor = cursor.saturating_add(1);
        if !value_is_exact || !target_is_exact {
            return Err(RuntimeError::Engine(Error::internal(
                "module link entry initializer is not canonical",
            )));
        }
    }
    if cursor != initializer_code.len() {
        return Err(RuntimeError::Engine(Error::internal(
            "module link entry initializer count disagrees with declarations",
        )));
    }
    Ok(())
}
