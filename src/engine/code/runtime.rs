pub(crate) use super::executable::{
    OrdinaryAuthentication, PublishedEvalEnvironment, PublishedFunctionData,
    PublishedFunctionSnapshot,
};
#[cfg(feature = "test262-host")]
use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::code::bytecode_publish;

use crate::engine::code::function::metadata::{
    ClosureVariable, ClosureVariableName, EvalEnvironment, FunctionMetadata,
    ParameterEnvironmentLayout, VariableDefinition,
};
use crate::engine::code::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::heap::{
    BytecodeConstant, ContextId, FunctionBytecodeData, FunctionDebugInfo, PublishedPrivateBindings,
    RawValue,
};
use crate::engine::value::{JsString, Value};
#[cfg(test)]
use crate::source::LineColumn;
use std::rc::Rc;

impl Runtime {
    /// Consume a verified compiler draft and publish an immutable bytecode GC
    /// node in `realm`.
    pub(crate) fn publish_unlinked_function(
        &self,
        realm: ContextId,
        function: UnlinkedFunction,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let function = bytecode_publish::VerifiedFunction::script(function)?;
        self.publish_verified_unlinked_function(realm, function)
    }

    pub(crate) fn publish_verified_unlinked_function(
        &self,
        realm: ContextId,
        function: bytecode_publish::VerifiedFunction,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        #[cfg(feature = "profiling")]
        let _phase_timer = crate::engine::api::profiling::PhaseTimer::start(
            crate::engine::api::profiling::CompilePhase::Publish,
        );
        let flat_functions = bytecode_publish::flatten_unlinked_tree(function.into_function())?;
        #[cfg(feature = "test262-host")]
        if !self.0.dynamic_import_bytecode_allowed.get()
            && flat_functions.iter().any(|function| {
                function.code.iter().any(|instruction| {
                    matches!(
                        instruction,
                        crate::engine::code::bytecode::Instruction::Import
                    )
                })
            })
        {
            return Err(Error::internal(
                "host dynamic-import bytecode policy rejected publication",
            )
            .into());
        }
        let _operation = self.operation();
        let mut roots: Vec<Option<FunctionBytecodeRef>> = Vec::with_capacity(flat_functions.len());

        for function in flat_functions {
            let mut linked_constants = Vec::with_capacity(function.constants.len());
            let mut atom_string_constants = Vec::new();
            let mut children = Vec::new();
            let mut materialized_constant_roots = Vec::new();
            let constants_linked = (|| -> Result<(), RuntimeError> {
                for constant in function.constants {
                    match constant {
                        FlatConstant::Value(value) => {
                            let raw = self.raw_property_value(&value)?;
                            linked_constants.push(BytecodeConstant::Value(raw));
                        }
                        FlatConstant::AtomString(value) => {
                            atom_string_constants.push(linked_constants.len());
                            let raw = self.raw_property_value(&Value::String(value))?;
                            linked_constants.push(BytecodeConstant::Value(raw));
                        }
                        FlatConstant::RegExp { pattern, program } => {
                            linked_constants.push(BytecodeConstant::RegExp { pattern, program });
                        }
                        FlatConstant::TemplateObject { cooked, raw } => {
                            let template = self.instantiate_template_object(realm, cooked, raw)?;
                            linked_constants.push(BytecodeConstant::Value(RawValue::Object(
                                template.object_id(),
                            )));
                            materialized_constant_roots.push(template);
                        }
                        FlatConstant::Child(index) => {
                            let child = roots.get(index).and_then(Option::as_ref).ok_or(
                                RuntimeError::Invariant(
                                    "flattened child function root was unavailable",
                                ),
                            )?;
                            linked_constants.push(BytecodeConstant::Function(child.bytecode_id()));
                            children.push(index);
                        }
                    }
                }
                Ok(())
            })();
            if let Err(error) = constants_linked {
                // No bytecode node will retain these converted constants, so
                // drop their caller-owned string/BigInt producer edges now.
                release_constant_edges(self, &linked_constants);
                return Err(error);
            }

            let mut closure_variables = function.closure_variables;
            let eval_environments = function.eval_environments;
            let argument_definitions = function.argument_definitions;
            let local_definitions = function.local_definitions;
            let mut linked_argument_definitions = Vec::with_capacity(argument_definitions.len());
            let mut linked_local_definitions = Vec::with_capacity(local_definitions.len());
            let mut linked_eval_environments = Vec::with_capacity(eval_environments.len());
            let mut linked_private_bindings = PublishedPrivateBindings::none();
            let mut unlinked_debug = function.debug;
            let mut linked_debug = None;
            let mut auxiliary_atoms = Vec::new();
            let mut property_key_atoms = Vec::new();
            let id = {
                let mut state = self.0.state.borrow_mut();
                let linking = (|| -> Result<(), RuntimeError> {
                    for index in atom_string_constants {
                        let value = match linked_constants.get(index) {
                            Some(BytecodeConstant::Value(RawValue::String(value))) => {
                                state.heap.string(*value)?.clone()
                            }
                            Some(BytecodeConstant::Value(_))
                            | Some(BytecodeConstant::RegExp { .. })
                            | Some(BytecodeConstant::Function(_))
                            | None => {
                                return Err(RuntimeError::Invariant(
                                    "atom-string constant lost its String payload",
                                ));
                            }
                        };
                        let atom = state.atoms.intern_property_key_js_string(&value)?;
                        // QuickJS falls back to an ordinary independent cpool
                        // String when JS_NewAtomStr produces a tagged integer.
                        if atom.is_immediate_integer() {
                            continue;
                        }
                        auxiliary_atoms.push(atom);
                        let canonical = state.atoms.to_js_string(atom)?;
                        let canonical = state.heap.allocate_string(canonical)?;
                        // The replaced draft string is never stored, so its
                        // producer edge dies here; the bytecode node retains
                        // its own edge for the canonical constant.
                        let previous = std::mem::replace(
                            &mut linked_constants[index],
                            BytecodeConstant::Value(RawValue::String(canonical)),
                        );
                        if let BytecodeConstant::Value(previous) = previous {
                            self.release_converted_value_edge(&previous);
                        }
                    }
                    property_key_atoms = bytecode_publish::link_constant_property_keys(
                        &mut state,
                        &function.code,
                        &linked_constants,
                        &mut auxiliary_atoms,
                    )?;
                    let private_binding_publication =
                        bytecode_publish::prepare_private_binding_publication(
                            &local_definitions,
                            &closure_variables,
                            &linked_constants,
                            &state.heap,
                        )?;
                    if let Some(debug) = unlinked_debug.take() {
                        let filename =
                            state.atoms.intern_property_key_js_string(&debug.filename)?;
                        auxiliary_atoms.push(filename);
                        linked_debug = Some(FunctionDebugInfo {
                            filename,
                            pc2line: debug.pc2line,
                            source: debug.source,
                        });
                    }
                    for descriptor in &mut closure_variables {
                        let ClosureVariableName::Constant(index) = descriptor.name else {
                            continue;
                        };
                        let name = usize::try_from(index)
                            .ok()
                            .and_then(|index| linked_constants.get(index))
                            .and_then(|constant| match constant {
                                BytecodeConstant::Value(RawValue::String(name)) => Some(name),
                                BytecodeConstant::Value(_)
                                | BytecodeConstant::RegExp { .. }
                                | BytecodeConstant::Function(_) => None,
                            })
                            .ok_or(RuntimeError::Invariant(
                                "verified closure name was not a string constant",
                            ))?;
                        let text = state.heap.string(*name)?.clone();
                        let atom = state.atoms.intern_property_key_js_string(&text)?;
                        auxiliary_atoms.push(atom);
                        descriptor.name = ClosureVariableName::Atom(atom);
                    }
                    for definition in argument_definitions {
                        let name = definition
                            .name
                            .as_ref()
                            .map(|name| state.atoms.intern_property_key_js_string(name))
                            .transpose()?;
                        auxiliary_atoms.extend(name);
                        linked_argument_definitions.push(VariableDefinition {
                            name,
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            is_parameter_initializer: definition.is_parameter_initializer,
                            kind: definition.kind,
                        });
                    }
                    for definition in local_definitions {
                        let name = definition
                            .name
                            .as_ref()
                            .map(|name| state.atoms.intern_property_key_js_string(name))
                            .transpose()?;
                        auxiliary_atoms.extend(name);
                        linked_local_definitions.push(VariableDefinition {
                            name,
                            is_lexical: definition.is_lexical,
                            is_const: definition.is_const,
                            is_parameter_initializer: definition.is_parameter_initializer,
                            kind: definition.kind,
                        });
                    }
                    linked_private_bindings = private_binding_publication
                        .authenticate(&linked_local_definitions, &closure_variables)?;
                    linked_eval_environments = bytecode_publish::link_eval_environments(
                        &mut state,
                        eval_environments,
                        &mut auxiliary_atoms,
                    )?;
                    Ok(())
                })();
                if let Err(error) = linking {
                    release_constant_edges(self, &linked_constants);
                    state.release_atoms(auxiliary_atoms.drain(..))?;
                    return Err(error);
                }

                let owned_atoms = auxiliary_atoms.clone();
                let bytecode = FunctionBytecodeData {
                    executable: Default::default(),

                    fusion: Default::default(),
                    code: function.code.into(),
                    constants: linked_constants.into(),
                    property_key_atoms: (!property_key_atoms.is_empty())
                        .then(|| property_key_atoms.into()),
                    realm,
                    metadata: function.metadata,
                    parameter_environment: function.parameter_environment,
                    func_name: function.func_name,
                    argument_definitions: linked_argument_definitions.into(),
                    local_definitions: linked_local_definitions.into(),
                    closure_variables: closure_variables.into(),
                    private_bindings: linked_private_bindings,
                    eval_environments: linked_eval_environments.into(),
                    debug: linked_debug,
                    auxiliary_atoms: auxiliary_atoms.into_boxed_slice(),
                };
                let producer_values = bytecode
                    .constants
                    .iter()
                    .filter_map(|constant| match constant {
                        BytecodeConstant::Value(raw) => Some(raw.clone()),
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                match state.heap.allocate_function_bytecode(bytecode) {
                    Ok(id) => {
                        // The bytecode node retained its own copy of every
                        // constant edge; release the caller-owned producer
                        // edges carried by the boundary conversion.
                        for raw in &producer_values {
                            self.release_converted_value_edge(raw);
                        }
                        id
                    }
                    Err(error) => {
                        for raw in &producer_values {
                            self.release_converted_value_edge(raw);
                        }
                        state.release_atoms(owned_atoms)?;
                        return Err(error.into());
                    }
                }
            };
            let root = FunctionBytecodeRef::from_owned_handle(self.clone(), id);
            // The bytecode node now owns every materialized template object
            // through its constant-pool RawValue edge.
            drop(materialized_constant_roots);

            // The parent node now owns each child through its cpool edge.
            for child in children {
                drop(roots[child].take());
            }
            roots.push(Some(root));
        }

        roots
            .last_mut()
            .and_then(Option::take)
            .ok_or(RuntimeError::Invariant(
                "unlinked function tree produced no published root",
            ))
    }

    #[cfg(test)]
    pub fn test_function_debug_location(
        &self,
        function: &FunctionBytecodeRef,
        pc: Option<usize>,
    ) -> Result<Option<(JsString, LineColumn)>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        let Some(debug) = &bytecode.debug else {
            return Ok(None);
        };
        let filename = state.atoms.to_js_string(debug.filename)?;
        let position = debug
            .pc2line
            .as_ref()
            .map(|table| table.lookup(pc.and_then(|pc| u32::try_from(pc).ok())));
        Ok(position.map(|position| (filename, position)))
    }

    #[cfg(test)]
    pub fn test_function_debug_source(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        Ok(state
            .heap
            .function_bytecode(function.bytecode_id())?
            .debug
            .as_ref()
            .and_then(|debug| debug.source.as_deref())
            .map(<[u8]>::to_vec))
    }

    #[cfg(test)]
    pub fn test_function_code(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Vec<crate::engine::code::bytecode::Instruction>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(function.bytecode_id())?
            .code
            .to_vec())
    }

    #[cfg(test)]
    pub fn test_function_name(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<JsString>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        Ok(self
            .0
            .state
            .borrow()
            .heap
            .function_bytecode(function.bytecode_id())?
            .func_name
            .clone())
    }

    #[cfg(test)]
    pub fn test_debug_filename_atom_ownership(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<Option<(usize, Option<u32>)>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        let Some(filename) = bytecode.debug.as_ref().map(|debug| debug.filename) else {
            return Ok(None);
        };
        let local_ownership = bytecode
            .auxiliary_atoms
            .iter()
            .filter(|atom| **atom == filename)
            .count();
        let total_ref_count = state.atoms.resolve(filename)?.ref_count;
        Ok(Some((local_ownership, total_ref_count)))
    }

    #[cfg(test)]
    pub fn test_atom_count(&self) -> usize {
        self.0.state.borrow().atoms.len()
    }

    #[cfg(test)]
    pub fn test_child_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
        constant_index: usize,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let id = {
            let state = self.0.state.borrow();
            let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
            match bytecode.constants.get(constant_index) {
                Some(BytecodeConstant::Function(id)) => *id,
                Some(BytecodeConstant::Value(_) | BytecodeConstant::RegExp { .. }) => {
                    return Err(RuntimeError::Invariant(
                        "requested child constant is a value",
                    ));
                }
                None => {
                    return Err(RuntimeError::Invariant(
                        "requested child constant is out of bounds",
                    ));
                }
            }
        };
        Ok(FunctionBytecodeRef::from_borrowed_handle(self.clone(), id)?)
    }
}

pub(crate) enum FlatConstant {
    /// Unlinked primitive payload. String and BigInt literals stay as public
    /// values here; the publish transaction below is their node creation
    /// point (§2.2), where they enter the constant pool as `RawValue`s.
    Value(Value),
    AtomString(JsString),
    RegExp {
        pattern: JsString,
        program: Rc<crate::regexp::CompiledRegExp>,
    },
    TemplateObject {
        cooked: Box<[Option<JsString>]>,
        raw: Box<[JsString]>,
    },
    Child(usize),
}

/// Release every caller-owned string/BigInt producer edge carried by
/// converted value constants. Idempotent for constants without a node edge.
fn release_constant_edges(runtime: &Runtime, constants: &[BytecodeConstant]) {
    for raw in constants.iter().filter_map(|constant| match constant {
        BytecodeConstant::Value(raw) => Some(raw),
        _ => None,
    }) {
        runtime.release_converted_value_edge(raw);
    }
}

pub(crate) struct FlatFunction {
    pub(crate) code: Vec<crate::engine::code::bytecode::Instruction>,
    pub(crate) constants: Vec<FlatConstant>,
    pub(crate) metadata: FunctionMetadata,
    pub(crate) parameter_environment: Option<ParameterEnvironmentLayout>,
    pub(crate) func_name: Option<JsString>,
    pub(crate) argument_definitions: Vec<UnlinkedVariableDefinition>,
    pub(crate) local_definitions: Vec<UnlinkedVariableDefinition>,
    pub(crate) closure_variables: Vec<ClosureVariable>,
    pub(crate) eval_environments: Vec<EvalEnvironment<JsString>>,
    pub(crate) debug: Option<UnlinkedFunctionDebug>,
}

pub(crate) struct FlattenFrame {
    pub(crate) code: Vec<crate::engine::code::bytecode::Instruction>,
    pub(crate) remaining: std::vec::IntoIter<UnlinkedConstant>,
    pub(crate) constants: Vec<FlatConstant>,
    pub(crate) metadata: FunctionMetadata,
    pub(crate) parameter_environment: Option<ParameterEnvironmentLayout>,
    pub(crate) func_name: Option<JsString>,
    pub(crate) argument_definitions: Vec<UnlinkedVariableDefinition>,
    pub(crate) local_definitions: Vec<UnlinkedVariableDefinition>,
    pub(crate) closure_variables: Vec<ClosureVariable>,
    pub(crate) eval_environments: Vec<EvalEnvironment<JsString>>,
    pub(crate) debug: Option<UnlinkedFunctionDebug>,
}

impl FlattenFrame {
    pub(crate) fn new(function: UnlinkedFunction) -> Self {
        let parts = function.into_parts();
        Self {
            code: parts.code,
            constants: Vec::with_capacity(parts.constants.len()),
            remaining: parts.constants.into_iter(),
            metadata: parts.metadata,
            parameter_environment: parts.parameter_environment,
            func_name: parts.func_name,
            argument_definitions: parts.argument_definitions,
            local_definitions: parts.local_definitions,
            closure_variables: parts.closure_variables,
            eval_environments: parts.eval_environments,
            debug: parts.debug,
        }
    }
}
