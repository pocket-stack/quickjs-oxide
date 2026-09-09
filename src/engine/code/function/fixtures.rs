//! Test-only builders for forged bytecode publication drafts.
use super::*;

impl UnlinkedFunction {
    fn ordinary_definitions(
        metadata: FunctionMetadata,
    ) -> (
        Vec<UnlinkedVariableDefinition>,
        Vec<UnlinkedVariableDefinition>,
    ) {
        let arguments = (0..metadata.argument_count)
            .map(|_| UnlinkedVariableDefinition::ordinary(None))
            .collect();
        let mut locals = (0..metadata.local_count)
            .map(|_| UnlinkedVariableDefinition::ordinary(None))
            .collect::<Vec<_>>();
        if let Some(index) = metadata.function_name_local {
            if let Some(definition) = locals.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::function_name(None, metadata.strict);
            }
        }
        if let Some(index) = metadata.eval_variable_object_local {
            if let Some(definition) = locals.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::eval_variable_object();
            }
        }
        (arguments, locals)
    }

    /// Forge ordinary unnamed bindings from the declared frame width.
    pub fn fixture(
        code: Vec<Instruction>,
        constants: Vec<UnlinkedConstant>,
        metadata: FunctionMetadata,
    ) -> Self {
        Self::fixture_with_closure_variables(code, constants, metadata, Vec::new())
    }

    pub fn fixture_with_closure_variables(
        code: Vec<Instruction>,
        constants: Vec<UnlinkedConstant>,
        metadata: FunctionMetadata,
        closure_variables: Vec<ClosureVariable>,
    ) -> Self {
        let (arguments, locals) = Self::ordinary_definitions(metadata);
        Self::new(
            code,
            constants,
            metadata,
            arguments,
            locals,
            closure_variables,
        )
    }

    /// Replace forged binding definitions to exercise publication validation.
    #[must_use]
    pub fn with_fixture_definitions(
        mut self,
        argument_definitions: Vec<UnlinkedVariableDefinition>,
        mut local_definitions: Vec<UnlinkedVariableDefinition>,
    ) -> Self {
        if let Some(index) = self.metadata.function_name_local {
            if let Some(definition) = local_definitions.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::function_name(
                    self.func_name.clone(),
                    self.metadata.strict,
                );
            }
        }
        if let Some(index) = self.metadata.eval_variable_object_local {
            if let Some(definition) = local_definitions.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::eval_variable_object();
            }
        }
        if let Some(index) = self
            .parameter_environment
            .as_ref()
            .and_then(|layout| layout.arg_eval_variable_object_local)
        {
            if let Some(definition) = local_definitions.get_mut(usize::from(index)) {
                *definition = UnlinkedVariableDefinition::arg_eval_variable_object();
            }
        }
        self.argument_definitions = argument_definitions;
        self.local_definitions = local_definitions;
        self
    }
}
