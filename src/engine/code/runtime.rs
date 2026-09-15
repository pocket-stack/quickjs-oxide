use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::atom::Atom;
use crate::engine::code::bytecode_publish;
use crate::engine::code::debug::DebugInfoMode;
use crate::source::QuickJsSourceLocator;

use crate::engine::code::function::metadata::{
    ClosureVariable, ClosureVariableName, EvalEnvironment, FunctionMetadata,
    ParameterEnvironmentLayout, VariableDefinition,
};
use crate::engine::code::function::{
    UnlinkedConstant, UnlinkedFunction, UnlinkedFunctionDebug, UnlinkedVariableDefinition,
};
use crate::engine::code::rooted::FunctionBytecodeRef;
use crate::engine::compiler::{
    compile_unlinked_script_bytes_with_filename, compile_unlinked_script_with_filename,
};
use crate::engine::heap::{
    BytecodeConstant, ContextId, FunctionBytecodeData, FunctionDebugInfo, PublishedPrivateBindings,
    RawValue,
};
use crate::engine::value::{JsString, Value};
use crate::engine::vm::frames::ExplicitBacktraceLocation;
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
        bytecode_publish::verify_unlinked_tree(&function)?;
        self.publish_verified_unlinked_function(realm, function)
    }

    pub(crate) fn publish_verified_unlinked_function(
        &self,
        realm: ContextId,
        function: UnlinkedFunction,
    ) -> Result<FunctionBytecodeRef, RuntimeError> {
        let flat_functions = bytecode_publish::flatten_unlinked_tree(function)?;
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
            for constant in function.constants {
                match constant {
                    FlatConstant::Value(value) => {
                        linked_constants.push(BytecodeConstant::Value(value));
                    }
                    FlatConstant::AtomString(value) => {
                        atom_string_constants.push(linked_constants.len());
                        linked_constants.push(BytecodeConstant::Value(RawValue::String(value)));
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

            let mut closure_variables = function.closure_variables;
            let eval_environments = function.eval_environments;
            let argument_definitions = function.argument_definitions;
            let local_definitions = function.local_definitions;
            let private_binding_publication =
                bytecode_publish::prepare_private_binding_publication(
                    &local_definitions,
                    &closure_variables,
                    &linked_constants,
                )?;
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
                            Some(BytecodeConstant::Value(RawValue::String(value))) => value.clone(),
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
                        linked_constants[index] =
                            BytecodeConstant::Value(RawValue::String(canonical));
                    }
                    property_key_atoms = bytecode_publish::link_constant_property_keys(
                        &mut state,
                        &function.code,
                        &linked_constants,
                        &mut auxiliary_atoms,
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
                        let atom = state.atoms.intern_property_key_js_string(name)?;
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
                    state.release_atoms(auxiliary_atoms.drain(..))?;
                    return Err(error);
                }

                let owned_atoms = auxiliary_atoms.clone();
                let bytecode = FunctionBytecodeData {
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
                match state.heap.allocate_function_bytecode(bytecode) {
                    Ok(id) => id,
                    Err(error) => {
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

    /// Compile and publish source without mutating the runtime pending-
    /// exception slot. Native indirect-eval paths need the thrown value as a
    /// normal completion, while the public Context boundary installs that
    /// same value into the pending slot before returning `Exception`.
    pub(crate) fn compile_in_realm(
        &self,
        realm: ContextId,
        source: &str,
        filename: &str,
    ) -> Result<Compilation, RuntimeError> {
        self.compile_script_in_realm(
            realm,
            QuickJsSourceLocator::new(source),
            filename,
            |debug_info| compile_unlinked_script_with_filename(source, filename, debug_info),
        )
    }

    /// Construct, compile, and publish one explicitly sized source buffer
    /// inside the same exception boundary as ordinary UTF-8 source.
    pub(crate) fn compile_bytes_in_realm(
        &self,
        realm: ContextId,
        source: &[u8],
        filename: &str,
    ) -> Result<Compilation, RuntimeError> {
        self.compile_script_in_realm(
            realm,
            QuickJsSourceLocator::from_bytes(source),
            filename,
            |debug_info| compile_unlinked_script_bytes_with_filename(source, filename, debug_info),
        )
    }

    pub(crate) fn compile_script_in_realm(
        &self,
        realm: ContextId,
        source_locator: QuickJsSourceLocator<'_>,
        filename: &str,
        compile: impl FnOnce(DebugInfoMode) -> Result<UnlinkedFunction, Error>,
    ) -> Result<Compilation, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let debug_info = self.debug_info_mode();
        let function = match compile(debug_info) {
            Ok(function) => function,
            Err(error) => {
                let Some(kind) = NativeErrorKind::from_javascript_error(error.kind()) else {
                    return Err(RuntimeError::Engine(error));
                };
                let explicit_location = if error.kind() == ErrorKind::Syntax {
                    if let Some(span) = error.span() {
                        let position = source_locator
                            .locate_byte_offset(span.start.byte_offset)
                            .map_err(|_| {
                                RuntimeError::Invariant(
                                    "syntax-error byte offset is invalid for its source",
                                )
                            })?;
                        Some(ExplicitBacktraceLocation {
                            filename: JsString::try_from_utf8(filename)?,
                            position,
                        })
                    } else {
                        None
                    }
                } else {
                    None
                };
                let exception = if error.kind() == ErrorKind::Syntax {
                    self.new_native_error_without_backtrace_from_error(realm, kind, &error)?
                } else {
                    self.new_native_error_from_error(realm, kind, &error)?
                };
                self.ensure_error_backtrace(&exception, false, explicit_location)?;
                return Ok(Compilation::Throw(exception));
            }
        };
        Ok(Compilation::Published(
            self.publish_unlinked_function(realm, function)?,
        ))
    }

    pub(crate) fn snapshot_function_bytecode(
        &self,
        function: &FunctionBytecodeRef,
    ) -> Result<PublishedFunctionSnapshot, RuntimeError> {
        let _operation = self.operation();
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("function bytecode"));
        }
        let root = function.clone();
        let state = self.0.state.borrow();
        let bytecode = state.heap.function_bytecode(function.bytecode_id())?;
        // The realm is a strong edge of the bytecode node. Validating it here
        // makes a corrupt realm edge fail before entering a VM frame.
        state.heap.context(bytecode.realm)?;
        Ok(PublishedFunctionSnapshot {
            root,
            code: bytecode.code.clone(),
            constants: bytecode.constants.clone(),
            property_key_atoms: bytecode.property_key_atoms.clone(),
            argument_definitions: bytecode.argument_definitions.clone(),
            local_definitions: bytecode.local_definitions.clone(),
            closure_variables: bytecode.closure_variables.clone(),
            eval_environments: bytecode.eval_environments.clone(),
            arg_eval_variable_object_local: bytecode
                .parameter_environment
                .as_ref()
                .and_then(|layout| layout.arg_eval_variable_object_local),
            metadata: bytecode.metadata,
            realm: bytecode.realm,
        })
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

pub(crate) enum Compilation {
    Published(FunctionBytecodeRef),
    Throw(Value),
}

/// Immutable VM inputs detached from the runtime `RefCell` borrow.
///
/// `constants` contains raw heap identities, so the owning bytecode root is
/// part of the snapshot. The raw constant pool therefore cannot outlive the GC
/// node whose edges keep those identities valid.
pub(crate) struct PublishedFunctionSnapshot {
    pub(crate) root: FunctionBytecodeRef,
    pub(crate) code: Rc<[crate::engine::code::bytecode::Instruction]>,
    pub(crate) constants: Rc<[BytecodeConstant]>,
    pub(crate) property_key_atoms: Option<Rc<[Atom]>>,
    pub(crate) argument_definitions: Rc<[VariableDefinition]>,
    pub(crate) local_definitions: Rc<[VariableDefinition]>,
    pub(crate) closure_variables: Rc<[ClosureVariable]>,
    pub(crate) eval_environments: Rc<[EvalEnvironment<Atom>]>,
    /// Parameter-scope variable-object slot, carried separately from the
    /// body `<var>` slot in `FunctionMetadata`.
    pub(crate) arg_eval_variable_object_local: Option<u16>,
    pub(crate) metadata: FunctionMetadata,
    pub(crate) realm: ContextId,
}

pub(crate) enum FlatConstant {
    Value(RawValue),
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
