//! Closure descriptor permissions and global declaration order.
use super::{RootPublication, eval_variable_object_sentinel, unlinked_closure_name};
use crate::engine::api::error::Error;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::function::UnlinkedFunction;
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariableKind, EvalKind, EvalRootBinding,
};
use crate::engine::value::JsString;
use std::collections::HashMap;

pub(super) struct GlobalDeclarations {
    pub global_declaration_names: HashMap<Vec<u16>, (bool, bool)>,
    pub first_global_declaration_indices: HashMap<Vec<u16>, usize>,
    pub global_function_declarations: Vec<Vec<u16>>,
}

pub(super) fn verify(
    function: &UnlinkedFunction,
    is_root: bool,
    root_publication: RootPublication<'_>,
    expected_eval_bindings: Option<&[EvalRootBinding<JsString>]>,
) -> Result<GlobalDeclarations, RuntimeError> {
    let mut global_declaration_names = HashMap::new();
    let mut first_global_declaration_indices = HashMap::new();
    let mut global_function_declarations = Vec::new();
    let mut verified_eval_binding_count = 0_usize;
    let eval_allows_global_declarations = is_root
        && match root_publication {
            RootPublication::Script
            | RootPublication::TrustedOrdinaryLeaf
            | RootPublication::Module(_) => false,
            RootPublication::Eval {
                kind,
                caller_strict,
                expected_bindings,
                ..
            } => {
                !function.metadata().strict
                    && !caller_strict
                    && (kind == EvalKind::Indirect
                        || (kind == EvalKind::Direct
                            && !expected_bindings
                                .iter()
                                .any(|binding| binding.kind.is_eval_variable_object())))
            }
        };
    for (descriptor_index, descriptor) in function.closure_variables().iter().enumerate() {
        if descriptor.kind == ClosureVariableKind::GlobalFunction
            && (descriptor.is_lexical || descriptor.is_const)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "global function declaration descriptor has lexical metadata",
            )));
        }
        if (descriptor.source == ClosureSource::GlobalDeclaration
            && !matches!(
                descriptor.kind,
                ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
            ))
            || (descriptor.source == ClosureSource::Global
                && descriptor.kind != ClosureVariableKind::Normal)
            || (matches!(descriptor.source, ClosureSource::ParentGlobal(_))
                && !matches!(
                    descriptor.kind,
                    ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                ))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "global declaration descriptor has non-global binding metadata",
            )));
        }
        if descriptor.kind == ClosureVariableKind::GlobalFunction
            && !matches!(
                descriptor.source,
                ClosureSource::GlobalDeclaration | ClosureSource::ParentGlobal(_)
            )
        {
            return Err(RuntimeError::Engine(Error::internal(
                "global function binding kind escaped a declaration relay",
            )));
        }
        if descriptor.kind.is_eval_variable_object()
            && (descriptor.is_lexical
                || descriptor.is_const
                || !matches!(
                    descriptor.source,
                    ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentClosure(_)
                        | ClosureSource::EvalEnvironment(_)
                ))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval variable-object descriptor has invalid binding metadata",
            )));
        }
        if descriptor.kind == ClosureVariableKind::WithObject
            && (descriptor.is_lexical
                || descriptor.is_const
                || !matches!(
                    descriptor.source,
                    ClosureSource::ParentLocal(_)
                        | ClosureSource::ParentClosure(_)
                        | ClosureSource::EvalEnvironment(_)
                ))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "with-object descriptor has invalid binding metadata",
            )));
        }
        let requires_name = matches!(
            descriptor.kind,
            ClosureVariableKind::FunctionName
                | ClosureVariableKind::EvalVariableObject
                | ClosureVariableKind::ArgEvalVariableObject
                | ClosureVariableKind::WithObject
                | ClosureVariableKind::PrivateField
                | ClosureVariableKind::PrivateMethod
                | ClosureVariableKind::PrivateGetter
                | ClosureVariableKind::PrivateSetter
                | ClosureVariableKind::PrivateGetterSetter
        ) || matches!(
            descriptor.source,
            ClosureSource::GlobalDeclaration
                | ClosureSource::Global
                | ClosureSource::ParentGlobal(_)
                | ClosureSource::EvalEnvironment(_)
                | ClosureSource::ModuleDeclaration
                | ClosureSource::ModuleImport
                | ClosureSource::ModuleImportCollision
                | ClosureSource::ModuleImportMeta
        );
        let name = unlinked_closure_name(function, descriptor)?;
        if descriptor.kind.is_eval_variable_object()
            && eval_variable_object_sentinel(descriptor.kind).is_none_or(|sentinel| {
                name.is_none_or(|name| name.utf16_units().ne(sentinel.encode_utf16()))
            })
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval variable-object descriptor lost its role sentinel",
            )));
        }
        if descriptor.kind == ClosureVariableKind::WithObject
            && name.is_none_or(|name| name.utf16_units().ne("<with>".encode_utf16()))
        {
            return Err(RuntimeError::Engine(Error::internal(
                "with-object descriptor lost its sentinel name",
            )));
        }
        if descriptor.source == ClosureSource::GlobalDeclaration {
            let name = name.ok_or_else(|| {
                RuntimeError::Engine(Error::internal("global declaration descriptor has no name"))
            })?;
            let key = name.utf16_units().collect::<Vec<_>>();
            first_global_declaration_indices
                .entry(key.clone())
                .or_insert(descriptor_index);
            if descriptor.kind == ClosureVariableKind::GlobalFunction {
                global_function_declarations.push(key.clone());
            }
            match global_declaration_names.entry(key) {
                std::collections::hash_map::Entry::Vacant(entry) => {
                    entry.insert((descriptor.is_lexical, descriptor.is_lexical));
                }
                std::collections::hash_map::Entry::Occupied(mut entry) => {
                    let (first_is_lexical, seen_lexical) = *entry.get();
                    if first_is_lexical
                        && seen_lexical
                        && (descriptor.is_lexical
                            || descriptor.kind != ClosureVariableKind::GlobalFunction)
                    {
                        return Err(RuntimeError::Engine(Error::internal(
                            "duplicate lexical global declaration descriptor name",
                        )));
                    }
                    // A first sloppy Annex B normal record masks every
                    // later same-name declaration in QuickJS's global
                    // conflict lookup, including repeated lexical and var
                    // records. A first lexical remains restricted to the
                    // pinned direct Program-function exception.
                    if descriptor.is_lexical {
                        entry.get_mut().1 = true;
                    }
                }
            }
        }
        // Direct eval retains ordinary local/argument/closure names as
        // semantic metadata. Global descriptors already require names;
        // local relay names remain optional outside an eval-visible path.
        let allows_name = requires_name
            || descriptor.is_lexical
            || matches!(
                descriptor.source,
                ClosureSource::ParentLocal(_)
                    | ClosureSource::ParentArgument(_)
                    | ClosureSource::ParentClosure(_)
            );
        if (requires_name && name.is_none()) || (!allows_name && name.is_some()) {
            return Err(RuntimeError::Engine(Error::internal(
                "closure descriptor name does not match its binding kind",
            )));
        }
        if let Some(expected_bindings) = expected_eval_bindings
            && descriptor_index < expected_bindings.len()
            && descriptor.source
                != ClosureSource::EvalEnvironment(u16::try_from(descriptor_index).map_err(
                    |_| {
                        RuntimeError::Engine(Error::internal(
                            "eval environment closure prefix exceeds bytecode range",
                        ))
                    },
                )?)
        {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment closure descriptors are not an exact prefix",
            )));
        }
        if is_root {
            match root_publication {
                RootPublication::Script => {
                    if !matches!(
                        descriptor.source,
                        ClosureSource::GlobalDeclaration | ClosureSource::Global
                    ) {
                        return Err(RuntimeError::Engine(Error::internal(
                            "root bytecode closure descriptor did not use Global",
                        )));
                    }
                }
                RootPublication::TrustedOrdinaryLeaf => {
                    return Err(RuntimeError::Engine(Error::internal(
                        "trusted ordinary leaf retained a closure descriptor",
                    )));
                }
                RootPublication::Module(_) => match descriptor.source {
                    ClosureSource::ModuleDeclaration => {
                        if descriptor.kind != ClosureVariableKind::Normal
                            || (descriptor.is_const && !descriptor.is_lexical)
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "module declaration descriptor has invalid binding metadata",
                            )));
                        }
                    }
                    ClosureSource::ModuleImport => {
                        if descriptor.kind != ClosureVariableKind::ModuleImportView
                            || !descriptor.is_lexical
                            || !descriptor.is_const
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "module import descriptor has invalid binding metadata",
                            )));
                        }
                    }
                    ClosureSource::ModuleImportCollision => {
                        if !descriptor.is_lexical
                            || !descriptor.is_const
                            || !matches!(
                                descriptor.kind,
                                ClosureVariableKind::Normal | ClosureVariableKind::ModuleImportView
                            )
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "module import collision descriptor has invalid binding metadata",
                            )));
                        }
                    }
                    ClosureSource::ModuleImportMeta => {
                        if descriptor.kind != ClosureVariableKind::Normal
                            || !descriptor.is_lexical
                            || !descriptor.is_const
                            || name
                                != Some(&JsString::from_static(
                                    crate::engine::code::module::MODULE_IMPORT_META_BINDING_NAME,
                                ))
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "import.meta descriptor has invalid binding metadata",
                            )));
                        }
                    }
                    ClosureSource::Global => {
                        if descriptor.kind != ClosureVariableKind::Normal
                            || descriptor.is_lexical
                            || descriptor.is_const
                        {
                            return Err(RuntimeError::Engine(Error::internal(
                                "module global descriptor has invalid binding metadata",
                            )));
                        }
                    }
                    _ => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "module root closure descriptor used a non-module source",
                        )));
                    }
                },
                RootPublication::Eval { .. } => match descriptor.source {
                    ClosureSource::Global | ClosureSource::EvalEnvironment(_) => {}
                    ClosureSource::GlobalDeclaration
                        if eval_allows_global_declarations
                            && !descriptor.is_lexical
                            && !descriptor.is_const
                            && matches!(
                                descriptor.kind,
                                ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
                            ) => {}
                    ClosureSource::GlobalDeclaration => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval root contained an illegal global declaration descriptor",
                        )));
                    }
                    _ => {
                        return Err(RuntimeError::Engine(Error::internal(
                            "eval root closure descriptor used a non-root source",
                        )));
                    }
                },
            }
        } else {
            match descriptor.source {
                ClosureSource::GlobalDeclaration | ClosureSource::Global => {
                    return Err(RuntimeError::Engine(Error::internal(
                        "only root bytecode may resolve a global closure binding",
                    )));
                }
                ClosureSource::EvalEnvironment(_) => {
                    return Err(RuntimeError::Engine(Error::internal(
                        "only a verified eval root may use an eval environment binding",
                    )));
                }
                ClosureSource::ModuleDeclaration
                | ClosureSource::ModuleImport
                | ClosureSource::ModuleImportCollision
                | ClosureSource::ModuleImportMeta => {
                    return Err(RuntimeError::Engine(Error::internal(
                        "only a verified module root may own a module binding",
                    )));
                }
                _ => {}
            }
        }
        if let ClosureSource::EvalEnvironment(index) = descriptor.source {
            let expected_bindings = expected_eval_bindings.ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "eval environment binding escaped specialized eval publication",
                ))
            })?;
            let index = usize::from(index);
            if index != descriptor_index || index != verified_eval_binding_count {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval environment closure descriptors are not in caller binding order",
                )));
            }
            let expected = expected_bindings.get(index).ok_or_else(|| {
                RuntimeError::Engine(Error::internal(
                    "eval environment closure descriptor exceeds caller binding count",
                ))
            })?;
            if name != Some(&expected.name) {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval environment closure name disagrees with the caller binding",
                )));
            }
            if (descriptor.is_lexical, descriptor.is_const, descriptor.kind)
                != (expected.is_lexical, expected.is_const, expected.kind)
            {
                return Err(RuntimeError::Engine(Error::internal(
                    "eval environment closure flags disagree with the caller binding",
                )));
            }
            verified_eval_binding_count += 1;
        }
        if matches!(
            descriptor.source,
            ClosureSource::GlobalDeclaration
                | ClosureSource::Global
                | ClosureSource::ParentGlobal(_)
        ) && !matches!(
            descriptor.kind,
            ClosureVariableKind::Normal | ClosureVariableKind::GlobalFunction
        ) {
            return Err(RuntimeError::Engine(Error::internal(
                "global closure descriptor has a non-global binding kind",
            )));
        }
    }
    if let Some(expected_bindings) = expected_eval_bindings {
        if verified_eval_binding_count != expected_bindings.len() {
            return Err(RuntimeError::Engine(Error::internal(
                "eval environment closure descriptor count disagrees with caller bindings",
            )));
        }
    }
    Ok(GlobalDeclarations {
        global_declaration_names,
        first_global_declaration_indices,
        global_function_declarations,
    })
}
