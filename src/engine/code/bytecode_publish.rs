//! Atom linking and iterative flattening after draft authentication.
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::code::bytecode::Instruction;
use crate::engine::code::function::{
    UnlinkedFunction,
    metadata::{EvalBinding, EvalEnvironment, EvalScope},
};
use crate::engine::code::runtime::{FlatConstant, FlatFunction, FlattenFrame};
use crate::engine::heap::runtime::RuntimeState;
use crate::engine::heap::{BytecodeConstant, RawValue};
use crate::engine::value::{JsString, Value};

mod private_elements;
pub(crate) use crate::engine::code::verify::VerifiedFunction;
pub(crate) use private_elements::prepare_private_binding_publication;

/// Link each distinct static-name constant index once. The caller owns the
/// atom transaction: any later failure releases `auxiliary_atoms`, while
/// successful publication transfers those references to the bytecode node.
pub(crate) fn link_constant_property_keys(
    state: &mut RuntimeState,
    code: &[Instruction],
    constants: &[BytecodeConstant],
    auxiliary_atoms: &mut Vec<Atom>,
) -> Result<Vec<Atom>, RuntimeError> {
    let mut keys = Vec::new();
    for instruction in code {
        let Some(index) = instruction.constant_property_key_index() else {
            continue;
        };
        let index = usize::try_from(index)
            .map_err(|_| RuntimeError::Invariant("static name index did not fit usize"))?;
        let Some(BytecodeConstant::Value(RawValue::String(name))) = constants.get(index) else {
            return Err(RuntimeError::Invariant(
                "static name did not reference a string constant",
            ));
        };
        if keys.len() <= index {
            keys.resize(index + 1, Atom::NULL);
        }
        if keys[index].is_null() {
            let atom = state.atoms.intern_property_key_js_string(name)?;
            auxiliary_atoms.push(atom);
            keys[index] = atom;
        }
    }
    Ok(keys)
}

/// Intern every semantically retained direct-eval binding name while keeping
/// the parent publication routine's atom transaction authoritative. The
/// caller releases `auxiliary_atoms` on any later failure and transfers the
/// complete list to the bytecode node on success.
pub(crate) fn link_eval_environments(
    state: &mut RuntimeState,
    environments: Vec<EvalEnvironment<JsString>>,
    auxiliary_atoms: &mut Vec<Atom>,
) -> Result<Vec<EvalEnvironment<Atom>>, RuntimeError> {
    let mut linked_environments = Vec::with_capacity(environments.len());
    for environment in environments {
        let mut linked_scopes = Vec::with_capacity(environment.scopes.len());
        for scope in environment.scopes {
            let mut linked_bindings = Vec::with_capacity(scope.bindings.len());
            for binding in scope.bindings {
                let name = state.atoms.intern_property_key_js_string(&binding.name)?;
                auxiliary_atoms.push(name);
                linked_bindings.push(EvalBinding {
                    name,
                    source: binding.source,
                    is_lexical: binding.is_lexical,
                    is_const: binding.is_const,
                    kind: binding.kind,
                    is_catch_parameter: binding.is_catch_parameter,
                });
            }
            linked_scopes.push(EvalScope {
                kind: scope.kind,
                bindings: linked_bindings.into_boxed_slice(),
            });
        }
        linked_environments.push(EvalEnvironment {
            scopes: linked_scopes.into_boxed_slice(),
            variable_environment: environment.variable_environment,
            caller_strict: environment.caller_strict,
            super_call_allowed: environment.super_call_allowed,
            super_allowed: environment.super_allowed,
        });
    }
    Ok(linked_environments)
}

pub(crate) fn flatten_unlinked_tree(
    function: UnlinkedFunction,
) -> Result<Vec<FlatFunction>, RuntimeError> {
    let mut frames = vec![FlattenFrame::new(function)];
    let mut functions = Vec::new();

    loop {
        let next = frames
            .last_mut()
            .ok_or(RuntimeError::Invariant(
                "unlinked function flattening lost its root frame",
            ))?
            .remaining
            .next();
        if let Some(constant) = next {
            let constant = match constant.into_template_object() {
                Ok((cooked, raw)) => {
                    frames
                        .last_mut()
                        .expect("flatten frame remains present")
                        .constants
                        .push(FlatConstant::TemplateObject { cooked, raw });
                    continue;
                }
                Err(constant) => constant,
            };
            let constant = match constant.into_regexp() {
                Ok((pattern, program)) => {
                    frames
                        .last_mut()
                        .expect("flatten frame remains present")
                        .constants
                        .push(FlatConstant::RegExp { pattern, program });
                    continue;
                }
                Err(constant) => constant,
            };
            let (primitive, atom_string, child) = constant.into_parts();
            match (primitive, atom_string, child) {
                (Some(crate::engine::value::PrimitiveValue::String(value)), true, None) => frames
                    .last_mut()
                    .expect("flatten frame remains present")
                    .constants
                    .push(FlatConstant::AtomString(value)),
                (Some(value), false, None) => frames
                    .last_mut()
                    .expect("flatten frame remains present")
                    .constants
                    .push(FlatConstant::Value(raw_unlinked_primitive(value.into())?)),
                (None, false, Some(child)) => frames.push(FlattenFrame::new(child)),
                (None, _, None)
                | (Some(_), true, None)
                | (Some(_), _, Some(_))
                | (None, true, Some(_)) => {
                    return Err(RuntimeError::Invariant(
                        "unlinked constant did not contain exactly one payload",
                    ));
                }
            }
            continue;
        }

        let frame = frames.pop().ok_or(RuntimeError::Invariant(
            "unlinked function flattening lost a completed frame",
        ))?;
        let index = functions.len();
        functions.push(FlatFunction {
            code: frame.code,
            constants: frame.constants,
            metadata: frame.metadata,
            parameter_environment: frame.parameter_environment,
            func_name: frame.func_name,
            argument_definitions: frame.argument_definitions,
            local_definitions: frame.local_definitions,
            closure_variables: frame.closure_variables,
            eval_environments: frame.eval_environments,
            debug: frame.debug,
        });
        if let Some(parent) = frames.last_mut() {
            parent.constants.push(FlatConstant::Child(index));
        } else {
            return Ok(functions);
        }
    }
}

fn raw_unlinked_primitive(value: Value) -> Result<RawValue, RuntimeError> {
    match value {
        Value::Undefined => Ok(RawValue::Undefined),
        Value::Null => Ok(RawValue::Null),
        Value::Bool(value) => Ok(RawValue::Bool(value)),
        Value::Int(value) => Ok(RawValue::Int(value)),
        Value::Float(value) => Ok(RawValue::Float(value)),
        Value::BigInt(value) => Ok(RawValue::BigInt(value)),
        Value::String(value) => Ok(RawValue::String(value)),
        Value::Object(_) | Value::Symbol(_) => Err(RuntimeError::Invariant(
            "runtime-bound value escaped the unlinked constant invariant",
        )),
    }
}
