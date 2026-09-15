//! Eval frame validation and ordered capture shared by both execution layouts.
use crate::engine::api::{Error, runtime::Runtime};
use crate::engine::atom::Atom;
use crate::engine::code::function::metadata::{
    ClosureSource, ClosureVariable, ClosureVariableName, EvalBindingSource, EvalEnvironment,
};
use crate::engine::code::runtime::{PublishedEvalEnvironment, PublishedFunctionSnapshot};
use crate::engine::heap::roots::VarRefRoot;

/// Validated caller state retained while primitive-String eval is compiled.
///
/// No frame binding has been converted to a VarRef yet. This preserves
/// QuickJS's ordering: parse/publish errors occur before closure capture.
pub(crate) struct PreparedEvalEnvironment {
    pub(crate) index: u16,
    pub(crate) descriptor: PublishedEvalEnvironment,
}

/// Live cells paired with one immutable caller-environment descriptor.
///
/// Roots are flattened in the descriptor's scope/binding order. The
/// descriptor itself preserves the lexical boundaries and declaration target
/// authenticated by the eval compiler, while the roots keep the caller's
/// actual cells live for the instantiation/execution interval.
pub(crate) struct MaterializedEvalEnvironment {
    pub(crate) index: u16,
    pub(crate) descriptor: PublishedEvalEnvironment,
    pub(crate) roots: Box<[VarRefRoot]>,
}

pub(super) fn validate(
    runtime: &Runtime,
    executable: &PublishedFunctionSnapshot,
    environment: &EvalEnvironment<Atom>,
    caller_strict: bool,
    local_count: usize,
    argument_count: usize,
    closure_slots: &super::closure::ClosureSlots,
) -> Result<(), Error> {
    if environment.caller_strict != caller_strict {
        return Err(Error::internal(
            "eval environment caller strictness disagrees with its bytecode frame",
        ));
    }
    for scope in &environment.scopes {
        for binding in &scope.bindings {
            match binding.source {
                EvalBindingSource::Local(index) => {
                    (usize::from(index) < local_count)
                        .then_some(())
                        .ok_or_else(|| {
                            Error::internal("eval local binding index is out of bounds")
                        })?;
                }
                EvalBindingSource::Argument(index) => {
                    (usize::from(index) < argument_count)
                        .then_some(())
                        .ok_or_else(|| {
                            Error::internal("eval argument binding index is out of bounds")
                        })?;
                }
                EvalBindingSource::Closure(index) => {
                    let descriptor = *executable
                        .closure_variables
                        .get(usize::from(index))
                        .ok_or_else(|| {
                            Error::internal("eval closure binding index is out of bounds")
                        })?;
                    let root = closure_slots.get(usize::from(index)).ok_or_else(|| {
                        Error::internal("eval closure slot index is out of bounds")
                    })?;
                    runtime
                        .validate_var_ref_metadata(&root, descriptor)
                        .map_err(|error| Error::internal(error.to_string()))?;
                }
            }
        }
    }
    Ok(())
}

pub(super) fn materialize(
    prepared: PreparedEvalEnvironment,
    closure_slots: &super::closure::ClosureSlots,
    mut capture: impl FnMut(EvalBindingSource, ClosureVariable) -> Result<VarRefRoot, Error>,
) -> Result<MaterializedEvalEnvironment, Error> {
    let PreparedEvalEnvironment { index, descriptor } = prepared;
    let binding_count = descriptor
        .scopes
        .iter()
        .map(|scope| scope.bindings.len())
        .sum();
    let mut roots = Vec::with_capacity(binding_count);
    for scope in &descriptor.scopes {
        for binding in &scope.bindings {
            let root = match binding.source {
                EvalBindingSource::Closure(index) => closure_slots
                    .get(usize::from(index))
                    .ok_or_else(|| Error::internal("eval closure slot index is out of bounds"))?
                    .clone(),
                source => {
                    let parent = match source {
                        EvalBindingSource::Local(index) => ClosureSource::ParentLocal(index),
                        EvalBindingSource::Argument(index) => ClosureSource::ParentArgument(index),
                        EvalBindingSource::Closure(_) => unreachable!(),
                    };
                    capture(
                        source,
                        ClosureVariable {
                            source: parent,
                            name: ClosureVariableName::Atom(binding.name),
                            is_lexical: binding.is_lexical,
                            is_const: binding.is_const,
                            kind: binding.kind,
                        },
                    )?
                }
            };
            roots.push(root);
        }
    }
    Ok(MaterializedEvalEnvironment {
        index,
        descriptor,
        roots: roots.into_boxed_slice(),
    })
}
