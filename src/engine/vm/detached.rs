use super::*;

#[cfg(test)]
pub(in crate::engine::vm) enum DetachedLocal {
    Initialized(Value),
    Uninitialized,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine::vm) enum DetachedEvalVariableOperation {
    Has(EvalVariableSource, u32),
    Get(EvalVariableSource, u32),
    Put(EvalVariableSource, u32, Value),
    Delete(EvalVariableSource, u32),
    Define(EvalVariableSource, u32, Value),
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq)]
pub(in crate::engine::vm) enum DetachedDynamicEnvironmentOperation {
    Has(DynamicEnvironmentSource, u32),
    Get(DynamicEnvironmentSource, u32, bool),
    Put(DynamicEnvironmentSource, u32, Value, bool),
    Delete(DynamicEnvironmentSource, u32),
    Object(DynamicEnvironmentSource),
    GlobalReference(u16),
    GetRef(Value, u32, bool),
    PutRef(Value, u32, Value, bool),
}

#[cfg(test)]
pub(in crate::engine::vm) struct DetachedHost<'a, T> {
    pub(in crate::engine::vm) function: &'a DetachedBytecode<T>,
    pub(in crate::engine::vm) locals: Vec<DetachedLocal>,
    #[cfg(test)]
    pub(in crate::engine::vm) backtrace_values: Vec<Value>,
    #[cfg(test)]
    pub(in crate::engine::vm) captured_local_reuse_preparations: usize,
    #[cfg(test)]
    pub(in crate::engine::vm) iterator_start_record: Option<(Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) iterator_next_results: VecDeque<Result<(Value, bool), Value>>,
    #[cfg(test)]
    pub(in crate::engine::vm) iterator_close_results: VecDeque<Option<Value>>,
    #[cfg(test)]
    pub(in crate::engine::vm) iterator_close_pending: Vec<bool>,
    #[cfg(test)]
    pub(in crate::engine::vm) array_from_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) array_from_inputs: Vec<Vec<Value>>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_field_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) defined_fields: Vec<(Value, u32, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_field_computed_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) defined_computed_fields: Vec<(Value, Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_method_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) defined_methods: Vec<(Value, u32, Value, DefineMethodKind, bool)>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_method_computed_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) defined_computed_methods:
        Vec<(Value, Value, Value, DefineMethodKind, bool)>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_class_results: VecDeque<DefineClassOutcome>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_class_inputs: Vec<(Value, Value, u32, bool)>,
    #[cfg(test)]
    pub(in crate::engine::vm) define_array_element_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) defined_array_elements: Vec<(Value, Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) object_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) variable_environment_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) eval_variable_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) eval_variable_operations: Vec<DetachedEvalVariableOperation>,
    #[cfg(test)]
    pub(in crate::engine::vm) dynamic_environment_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) dynamic_environment_operations:
        Vec<DetachedDynamicEnvironmentOperation>,
    #[cfg(test)]
    pub(in crate::engine::vm) arguments_results: VecDeque<(ArgumentsKind, Completion)>,
    #[cfg(test)]
    pub(in crate::engine::vm) rest_results: VecDeque<(u16, Completion)>,
    #[cfg(test)]
    pub(in crate::engine::vm) set_object_prototype_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) set_object_prototype_inputs: Vec<(Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) copy_data_properties_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) copy_data_properties_inputs: Vec<(Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) copy_data_properties_excluded_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) copy_data_properties_excluded_inputs: Vec<(Value, Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) eval_identity_results: VecDeque<Result<bool, Error>>,
    #[cfg(test)]
    pub(in crate::engine::vm) eval_identity_inputs: Vec<Value>,
    #[cfg(test)]
    pub(in crate::engine::vm) direct_eval_results: VecDeque<Result<Completion, Error>>,
    #[cfg(test)]
    pub(in crate::engine::vm) direct_eval_inputs: Vec<DirectEvalInvocation>,
    #[cfg(test)]
    pub(in crate::engine::vm) box_primitive_results: VecDeque<Result<Value, Error>>,
    #[cfg(test)]
    pub(in crate::engine::vm) box_primitive_inputs: Vec<Value>,
    #[cfg(test)]
    pub(in crate::engine::vm) call_results: VecDeque<Result<Completion, Error>>,
    #[cfg(test)]
    pub(in crate::engine::vm) call_inputs: Vec<(Value, Value, Vec<Value>)>,
    #[cfg(test)]
    pub(in crate::engine::vm) get_property_results: VecDeque<Completion>,
    #[cfg(test)]
    pub(in crate::engine::vm) get_property_inputs: Vec<(Value, Value)>,
    #[cfg(test)]
    pub(in crate::engine::vm) dynamic_import_results: VecDeque<Result<Completion, Error>>,
    #[cfg(test)]
    pub(in crate::engine::vm) dynamic_import_inputs: Vec<(Value, Value)>,
}

#[cfg(test)]
impl<'a, T> DetachedHost<'a, T> {
    pub(in crate::engine::vm) fn new(function: &'a DetachedBytecode<T>) -> Self {
        Self {
            function,
            locals: (0..function.local_count)
                .map(|_| DetachedLocal::Initialized(Value::Undefined))
                .collect(),
            #[cfg(test)]
            backtrace_values: Vec::new(),
            #[cfg(test)]
            captured_local_reuse_preparations: 0,
            #[cfg(test)]
            iterator_start_record: None,
            #[cfg(test)]
            iterator_next_results: VecDeque::new(),
            #[cfg(test)]
            iterator_close_results: VecDeque::new(),
            #[cfg(test)]
            iterator_close_pending: Vec::new(),
            #[cfg(test)]
            array_from_results: VecDeque::new(),
            #[cfg(test)]
            array_from_inputs: Vec::new(),
            #[cfg(test)]
            define_field_results: VecDeque::new(),
            #[cfg(test)]
            defined_fields: Vec::new(),
            #[cfg(test)]
            define_field_computed_results: VecDeque::new(),
            #[cfg(test)]
            defined_computed_fields: Vec::new(),
            #[cfg(test)]
            define_method_results: VecDeque::new(),
            #[cfg(test)]
            defined_methods: Vec::new(),
            #[cfg(test)]
            define_method_computed_results: VecDeque::new(),
            #[cfg(test)]
            defined_computed_methods: Vec::new(),
            #[cfg(test)]
            define_class_results: VecDeque::new(),
            #[cfg(test)]
            define_class_inputs: Vec::new(),
            #[cfg(test)]
            define_array_element_results: VecDeque::new(),
            #[cfg(test)]
            defined_array_elements: Vec::new(),
            #[cfg(test)]
            object_results: VecDeque::new(),
            #[cfg(test)]
            variable_environment_results: VecDeque::new(),
            #[cfg(test)]
            eval_variable_results: VecDeque::new(),
            #[cfg(test)]
            eval_variable_operations: Vec::new(),
            #[cfg(test)]
            dynamic_environment_results: VecDeque::new(),
            #[cfg(test)]
            dynamic_environment_operations: Vec::new(),
            #[cfg(test)]
            arguments_results: VecDeque::new(),
            #[cfg(test)]
            rest_results: VecDeque::new(),
            #[cfg(test)]
            set_object_prototype_results: VecDeque::new(),
            #[cfg(test)]
            set_object_prototype_inputs: Vec::new(),
            #[cfg(test)]
            copy_data_properties_results: VecDeque::new(),
            #[cfg(test)]
            copy_data_properties_inputs: Vec::new(),
            #[cfg(test)]
            copy_data_properties_excluded_results: VecDeque::new(),
            #[cfg(test)]
            copy_data_properties_excluded_inputs: Vec::new(),
            #[cfg(test)]
            eval_identity_results: VecDeque::new(),
            #[cfg(test)]
            eval_identity_inputs: Vec::new(),
            #[cfg(test)]
            direct_eval_results: VecDeque::new(),
            #[cfg(test)]
            direct_eval_inputs: Vec::new(),
            #[cfg(test)]
            box_primitive_results: VecDeque::new(),
            #[cfg(test)]
            box_primitive_inputs: Vec::new(),
            #[cfg(test)]
            call_results: VecDeque::new(),
            #[cfg(test)]
            call_inputs: Vec::new(),
            #[cfg(test)]
            get_property_results: VecDeque::new(),
            #[cfg(test)]
            get_property_inputs: Vec::new(),
            #[cfg(test)]
            dynamic_import_results: VecDeque::new(),
            #[cfg(test)]
            dynamic_import_inputs: Vec::new(),
        }
    }

    pub(in crate::engine::vm) fn local(&self, index: u16) -> Result<&DetachedLocal, Error> {
        self.locals
            .get(usize::from(index))
            .ok_or_else(|| Error::internal("local-variable index is out of bounds"))
    }

    pub(in crate::engine::vm) fn local_mut(
        &mut self,
        index: u16,
    ) -> Result<&mut DetachedLocal, Error> {
        self.locals
            .get_mut(usize::from(index))
            .ok_or_else(|| Error::internal("local-variable index is out of bounds"))
    }
}

#[cfg(test)]
impl<T: TestConstant + Clone + Into<Value>> VmHost for DetachedHost<'_, T> {
    fn update_active_bytecode_pc(&mut self, _pc: BytecodePc) -> Result<(), Error> {
        Ok(())
    }

    fn ensure_backtrace(&mut self, value: &Value) -> Result<(), Error> {
        #[cfg(test)]
        self.backtrace_values.push(value.clone());
        Ok(())
    }

    fn prepare_captured_local_reuse(&mut self) -> Result<(), Error> {
        #[cfg(test)]
        {
            self.captured_local_reuse_preparations += 1;
        }
        Ok(())
    }

    fn for_of_start(&mut self, _iterable: Value) -> Result<ForOfStartOutcome, Error> {
        #[cfg(test)]
        if let Some((iterator, next_method)) = self.iterator_start_record.take() {
            return Ok(ForOfStartOutcome::Record {
                iterator,
                next_method,
            });
        }
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn for_of_next(
        &mut self,
        _iterator: Value,
        _next_method: Value,
    ) -> Result<ForOfNextOutcome, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.iterator_next_results.pop_front() {
            return Ok(match outcome {
                Ok((value, done)) => ForOfNextOutcome::Result { value, done },
                Err(value) => ForOfNextOutcome::Throw(value),
            });
        }
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn iterator_get_value_done(&mut self, _result: Value) -> Result<ForOfNextOutcome, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.iterator_next_results.pop_front() {
            return Ok(match outcome {
                Ok((value, done)) => ForOfNextOutcome::Result { value, done },
                Err(value) => ForOfNextOutcome::Throw(value),
            });
        }
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn for_in_start(&mut self, _value: Value) -> Result<ForInStartOutcome, Error> {
        Err(Error::internal("detached VM has no for-in intrinsics"))
    }

    fn for_in_next(&mut self, _iterator: Value) -> Result<ForInNextOutcome, Error> {
        Err(Error::internal("detached VM has no for-in intrinsics"))
    }

    fn iterator_close(
        &mut self,
        _iterator: Value,
        exception_pending: bool,
    ) -> Result<IteratorCloseOutcome, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.iterator_close_results.pop_front() {
            self.iterator_close_pending.push(exception_pending);
            return Ok(match outcome {
                Some(value) => IteratorCloseOutcome::Throw(value),
                None => IteratorCloseOutcome::Closed,
            });
        }
        let _ = exception_pending;
        Err(Error::internal("detached VM has no iterator intrinsics"))
    }

    fn load_constant(&mut self, index: u32) -> Result<Value, Error> {
        self.function
            .constant(index)
            .cloned()
            .map(Into::into)
            .ok_or_else(|| Error::internal("constant index is out of bounds"))
    }

    fn read_only_error(&mut self, index: u32) -> Result<Error, Error> {
        let Value::String(name) = self.load_constant(index)? else {
            return Err(Error::internal(
                "read-only binding opcode referenced a non-string constant",
            ));
        };
        let mut message = crate::engine::api::error::NativeErrorMessage::new();
        message.push_utf8("'");
        name.push_atom_get_str_to(&mut message);
        message.push_utf8("' is read-only");
        Ok(Error::from_native_message(ErrorKind::Type, message))
    }

    fn redeclaration_error(&mut self, index: u32) -> Result<Error, Error> {
        let Value::String(name) = self.load_constant(index)? else {
            return Err(Error::internal(
                "redeclaration opcode referenced a non-string constant",
            ));
        };
        let mut message = crate::engine::api::error::NativeErrorMessage::new();
        message.push_utf8("redeclaration of '");
        name.push_atom_get_str_to(&mut message);
        message.push_utf8("'");
        Ok(Error::from_native_message(ErrorKind::Syntax, message))
    }

    fn to_boolean(&mut self, value: &Value) -> Result<bool, Error> {
        let Value::Object(object) = value else {
            return Ok(value.to_boolean_primitive());
        };
        object
            .runtime()
            .value_to_boolean(value)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn is_html_dda(&mut self, value: &Value) -> Result<bool, Error> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        object
            .runtime()
            .value_is_html_dda(value)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn is_callable(&mut self, value: &Value) -> Result<bool, Error> {
        let Value::Object(object) = value else {
            return Ok(false);
        };
        object
            .runtime()
            .value_is_callable(value)
            .map_err(|error| Error::internal(error.to_string()))
    }

    fn type_of(&mut self, value: &Value) -> Result<JsString, Error> {
        if let Value::Object(object) = value
            && object
                .runtime()
                .value_is_html_dda(value)
                .map_err(|error| Error::internal(error.to_string()))?
        {
            return Ok(JsString::from_static("undefined"));
        }
        Ok(JsString::from_static(value.type_of()))
    }

    fn box_primitive(&mut self, _value: Value) -> Result<Value, Error> {
        #[cfg(test)]
        {
            self.box_primitive_inputs.push(_value);
            if let Some(result) = self.box_primitive_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM has no primitive wrapper intrinsics",
        ))
    }

    fn to_primitive(&mut self, value: Value, _hint: ToPrimitiveHint) -> Result<Completion, Error> {
        if matches!(value, Value::Object(_)) {
            Err(Error::internal(
                "detached VM cannot execute object ToPrimitive",
            ))
        } else {
            Ok(Completion::Return(value))
        }
    }

    fn materialize_error(&mut self, error: Error) -> Result<Value, Error> {
        Err(error)
    }

    fn instantiate_closure(&mut self, _index: u32) -> Result<Value, Error> {
        Err(Error::internal(
            "detached VM cannot instantiate runtime-owned function bytecode",
        ))
    }

    fn set_function_name(&mut self, _value: &Value, _name_index: u32) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM cannot name a runtime-owned function object",
        ))
    }

    fn set_function_name_computed(&mut self, _value: &Value, _key: &Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM cannot name a runtime-owned function object",
        ))
    }

    fn create_arguments(&mut self, kind: ArgumentsKind) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some((expected, outcome)) = self.arguments_results.pop_front() {
            if expected != kind {
                return Err(Error::internal("unexpected detached arguments kind"));
            }
            return Ok(outcome);
        }
        let _ = kind;
        Err(Error::internal(
            "detached VM cannot create runtime-owned arguments objects",
        ))
    }

    fn create_rest(&mut self, start: u16) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some((expected, outcome)) = self.rest_results.pop_front() {
            if expected != start {
                return Err(Error::internal("unexpected detached rest start"));
            }
            return Ok(outcome);
        }
        let _ = start;
        Err(Error::internal(
            "detached VM cannot create runtime-owned rest arrays",
        ))
    }

    fn object(&mut self) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.object_results.pop_front() {
            return Ok(outcome);
        }
        Err(Error::internal(
            "detached VM cannot create runtime-owned Object values",
        ))
    }

    fn create_variable_environment(&mut self) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.variable_environment_results.pop_front() {
            return Ok(outcome);
        }
        Err(Error::internal(
            "detached VM cannot create runtime-owned eval variable environments",
        ))
    }

    fn has_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Has(source, name));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot inspect eval variable environments",
        ))
    }

    fn get_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Get(source, name));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot inspect eval variable environments",
        ))
    }

    fn put_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Put(
                    source,
                    name,
                    value.clone(),
                ));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, value);
        Err(Error::internal(
            "detached VM cannot mutate eval variable environments",
        ))
    }

    fn delete_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Delete(source, name));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot delete eval variable bindings",
        ))
    }

    fn define_eval_variable(
        &mut self,
        source: EvalVariableSource,
        name: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.eval_variable_operations
                .push(DetachedEvalVariableOperation::Define(
                    source,
                    name,
                    value.clone(),
                ));
            if let Some(outcome) = self.eval_variable_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, value);
        Err(Error::internal(
            "detached VM cannot define eval variable bindings",
        ))
    }

    fn has_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Has(source, name));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot inspect dynamic environments",
        ))
    }

    fn get_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Get(
                    source, name, strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, strict);
        Err(Error::internal(
            "detached VM cannot inspect dynamic environments",
        ))
    }

    fn put_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Put(
                    source,
                    name,
                    value.clone(),
                    strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name, value, strict);
        Err(Error::internal(
            "detached VM cannot mutate dynamic environments",
        ))
    }

    fn delete_dynamic_binding(
        &mut self,
        source: DynamicEnvironmentSource,
        name: u32,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Delete(source, name));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (source, name);
        Err(Error::internal(
            "detached VM cannot delete dynamic environment bindings",
        ))
    }

    fn dynamic_environment_object(
        &mut self,
        source: DynamicEnvironmentSource,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::Object(source));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = source;
        Err(Error::internal(
            "detached VM cannot expose dynamic environment objects",
        ))
    }

    fn global_reference(&mut self, index: u16) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::GlobalReference(index));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = index;
        Err(Error::internal(
            "detached VM cannot resolve global reference objects",
        ))
    }

    fn get_ref_value(
        &mut self,
        environment: Value,
        name: u32,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::GetRef(
                    environment.clone(),
                    name,
                    strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (environment, name, strict);
        Err(Error::internal(
            "detached VM cannot read dynamic reference values",
        ))
    }

    fn put_ref_value(
        &mut self,
        environment: Value,
        name: u32,
        value: Value,
        strict: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.dynamic_environment_operations
                .push(DetachedDynamicEnvironmentOperation::PutRef(
                    environment.clone(),
                    name,
                    value.clone(),
                    strict,
                ));
            if let Some(outcome) = self.dynamic_environment_results.pop_front() {
                return Ok(outcome);
            }
        }
        let _ = (environment, name, value, strict);
        Err(Error::internal(
            "detached VM cannot write dynamic reference values",
        ))
    }

    fn create_regexp(&mut self, _index: u32) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot create runtime-owned RegExp objects",
        ))
    }

    fn array_from(&mut self, elements: Vec<Value>) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.array_from_results.pop_front() {
            self.array_from_inputs.push(elements);
            return Ok(outcome);
        }
        let _ = elements;
        Err(Error::internal(
            "detached VM cannot create runtime-owned Array objects",
        ))
    }

    fn define_field(
        &mut self,
        base: Value,
        key_index: u32,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_field_results.pop_front() {
            self.defined_fields.push((base, key_index, value));
            return Ok(outcome);
        }
        let _ = (base, key_index, value);
        Err(Error::internal(
            "detached VM cannot define runtime-owned properties",
        ))
    }

    fn define_field_computed(
        &mut self,
        base: Value,
        key: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_field_computed_results.pop_front() {
            self.defined_computed_fields.push((base, key, value));
            return Ok(outcome);
        }
        let _ = (base, key, value);
        Err(Error::internal(
            "detached VM cannot define runtime-owned computed properties",
        ))
    }

    fn define_method(
        &mut self,
        base: Value,
        key_index: u32,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_method_results.pop_front() {
            self.defined_methods
                .push((base, key_index, function, kind, enumerable));
            return Ok(outcome);
        }
        let _ = (base, key_index, function, kind, enumerable);
        Err(Error::internal(
            "detached VM cannot define runtime-owned methods",
        ))
    }

    fn define_method_computed(
        &mut self,
        base: Value,
        key: Value,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_method_computed_results.pop_front() {
            self.defined_computed_methods
                .push((base, key, function, kind, enumerable));
            return Ok(outcome);
        }
        let _ = (base, key, function, kind, enumerable);
        Err(Error::internal(
            "detached VM cannot define runtime-owned computed methods",
        ))
    }

    fn define_class(
        &mut self,
        parent: Value,
        constructor: Value,
        name: u32,
        has_heritage: bool,
    ) -> Result<DefineClassOutcome, Error> {
        #[cfg(test)]
        {
            self.define_class_inputs
                .push((parent, constructor, name, has_heritage));
            if let Some(outcome) = self.define_class_results.pop_front() {
                return Ok(outcome);
            }
        }
        Err(Error::internal(
            "detached VM cannot define runtime-owned classes",
        ))
    }

    fn define_array_element(
        &mut self,
        base: Value,
        index: Value,
        value: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.define_array_element_results.pop_front() {
            self.defined_array_elements.push((base, index, value));
            return Ok(outcome);
        }
        let _ = (base, index, value);
        Err(Error::internal(
            "detached VM cannot define runtime-owned Array elements",
        ))
    }

    fn set_object_prototype(
        &mut self,
        object: Value,
        prototype: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.set_object_prototype_results.pop_front() {
            self.set_object_prototype_inputs.push((object, prototype));
            return Ok(outcome);
        }
        let _ = (object, prototype);
        Err(Error::internal(
            "detached VM cannot mutate runtime-owned Object prototypes",
        ))
    }

    fn copy_data_properties(&mut self, target: Value, source: Value) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.copy_data_properties_results.pop_front() {
            self.copy_data_properties_inputs.push((target, source));
            return Ok(outcome);
        }
        let _ = (target, source);
        Err(Error::internal(
            "detached VM cannot copy runtime-owned Object properties",
        ))
    }

    fn copy_data_properties_excluded(
        &mut self,
        target: Value,
        source: Value,
        excluded: Value,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(outcome) = self.copy_data_properties_excluded_results.pop_front() {
            self.copy_data_properties_excluded_inputs
                .push((target, source, excluded));
            return Ok(outcome);
        }
        let _ = (target, source, excluded);
        Err(Error::internal(
            "detached VM cannot copy excluded runtime-owned Object properties",
        ))
    }

    fn get_global_var(
        &mut self,
        _index: u16,
        _throw_if_missing: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM has no realm global environment",
        ))
    }

    fn delete_global_var(&mut self, _index: u16) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM has no realm global environment",
        ))
    }

    fn put_global_var(
        &mut self,
        _index: u16,
        _value: Value,
        _initialize: bool,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM has no realm global environment",
        ))
    }

    fn get_field(&mut self, _base: Value, _key_index: u32) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot access runtime-owned properties",
        ))
    }

    fn get_property(&mut self, _base: Value, _key: Value) -> Result<Completion, Error> {
        #[cfg(test)]
        if let Some(result) = self.get_property_results.pop_front() {
            self.get_property_inputs.push((_base, _key));
            return Ok(result);
        }
        Err(Error::internal(
            "detached VM cannot access runtime-owned properties",
        ))
    }

    fn has_property(&mut self, _key: Value, _object: ObjectRef) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot test runtime-owned properties",
        ))
    }

    fn is_instance_of(
        &mut self,
        _candidate: Value,
        _target: ObjectRef,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot perform runtime-owned instance checks",
        ))
    }

    fn convert_property_key(&mut self, _key: Value) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot convert runtime-owned property keys",
        ))
    }

    fn set_field(
        &mut self,
        _base: Value,
        _key_index: u32,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot mutate runtime-owned properties",
        ))
    }

    fn set_property(
        &mut self,
        _base: Value,
        _key: Value,
        _value: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot mutate runtime-owned properties",
        ))
    }

    fn delete_property(
        &mut self,
        _base: Value,
        _key: Value,
        _strict: bool,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot delete runtime-owned properties",
        ))
    }

    fn dynamic_import(&mut self, specifier: Value, options: Value) -> Result<Completion, Error> {
        self.dynamic_import_inputs.push((specifier, options));
        if let Some(result) = self.dynamic_import_results.pop_front() {
            return result;
        }
        Err(Error::internal(
            "detached VM cannot execute runtime-owned dynamic import",
        ))
    }

    fn call(
        &mut self,
        _function: Value,
        _this_value: Value,
        _arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.call_inputs.push((_function, _this_value, _arguments));
            if let Some(result) = self.call_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM cannot call runtime-owned function objects",
        ))
    }

    fn apply(
        &mut self,
        _function: Value,
        _this_or_new_target: Value,
        _argument_array: Value,
        _kind: ApplyKind,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot apply runtime-owned function objects",
        ))
    }

    fn build_argument_list(
        &mut self,
        _argument_array: Value,
    ) -> Result<ArgumentListOutcome, Error> {
        Err(Error::internal(
            "detached VM cannot build runtime-owned argument lists",
        ))
    }

    fn is_original_eval(&mut self, _function: &Value) -> Result<bool, Error> {
        #[cfg(test)]
        {
            self.eval_identity_inputs.push(_function.clone());
            if let Some(result) = self.eval_identity_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM cannot identify runtime-owned eval function objects",
        ))
    }

    fn direct_eval(&mut self, _invocation: DirectEvalInvocation) -> Result<Completion, Error> {
        #[cfg(test)]
        {
            self.direct_eval_inputs.push(_invocation);
            if let Some(result) = self.direct_eval_results.pop_front() {
                return result;
            }
        }
        Err(Error::internal(
            "detached VM cannot directly evaluate runtime-owned function objects",
        ))
    }

    fn construct(
        &mut self,
        _function: Value,
        _new_target: Value,
        _arguments: Vec<Value>,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot construct runtime-owned function objects",
        ))
    }

    fn init_derived_constructor(
        &mut self,
        _active_function: ObjectRef,
        _new_target: Value,
    ) -> Result<Completion, Error> {
        Err(Error::internal(
            "detached VM cannot initialize a runtime-owned derived constructor",
        ))
    }

    fn closure_count(&self) -> usize {
        0
    }

    fn get_local(&mut self, index: u16) -> Result<Value, Error> {
        match self.local(index)? {
            DetachedLocal::Initialized(value) => Ok(value.clone()),
            DetachedLocal::Uninitialized => Err(Error::internal(
                "unchecked local read reached an uninitialized lexical binding",
            )),
        }
    }

    fn put_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let local = self.local_mut(index)?;
        if matches!(local, DetachedLocal::Uninitialized) {
            return Err(Error::internal(
                "unchecked local write reached an uninitialized lexical binding",
            ));
        }
        *local = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn set_local_uninitialized(&mut self, index: u16) -> Result<(), Error> {
        *self.local_mut(index)? = DetachedLocal::Uninitialized;
        Ok(())
    }

    fn get_local_checked(&mut self, index: u16) -> Result<Value, Error> {
        match self.local(index)? {
            DetachedLocal::Initialized(value) => Ok(value.clone()),
            DetachedLocal::Uninitialized => Err(Error::new(
                ErrorKind::Reference,
                "lexical variable is not initialized",
            )),
        }
    }

    fn initialize_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        *self.local_mut(index)? = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn initialize_derived_local(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let local = self.local_mut(index)?;
        if !matches!(local, DetachedLocal::Uninitialized) {
            return Err(Error::new(
                ErrorKind::Reference,
                "'this' can be initialized only once",
            ));
        }
        *local = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn put_local_checked(&mut self, index: u16, value: Value) -> Result<(), Error> {
        let local = self.local_mut(index)?;
        if matches!(local, DetachedLocal::Uninitialized) {
            return Err(Error::new(
                ErrorKind::Reference,
                "lexical variable is not initialized",
            ));
        }
        *local = DetachedLocal::Initialized(value);
        Ok(())
    }

    fn close_local(&mut self, index: u16) -> Result<(), Error> {
        self.local(index)?;
        Ok(())
    }

    fn get_argument(&mut self, _index: u16) -> Result<Value, Error> {
        Err(Error::internal("detached VM has no argument frame"))
    }

    fn put_argument(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal("detached VM has no argument frame"))
    }

    fn get_var_ref(&mut self, _index: u16) -> Result<Value, Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn put_var_ref(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn get_var_ref_checked(&mut self, _index: u16) -> Result<Value, Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn put_var_ref_checked(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn initialize_derived_var_ref(&mut self, _index: u16, _value: Value) -> Result<(), Error> {
        Err(Error::internal(
            "detached VM has no closure-variable environment",
        ))
    }

    fn return_derived(&mut self, index: u16, value: Value) -> Result<Completion, Error> {
        // Validate the typed local operand even when an explicit Object return
        // does not observe the binding's value.
        self.local(index)?;
        match value {
            value @ Value::Object(_) => Ok(Completion::Return(value)),
            Value::Undefined => {
                let this_value = self.get_local_checked(index)?;
                if !matches!(this_value, Value::Object(_)) {
                    return Err(Error::internal(
                        "initialized derived this binding did not contain an Object",
                    ));
                }
                Ok(Completion::Return(this_value))
            }
            _ => Err(Error::new(
                ErrorKind::Type,
                "derived class constructor must return an object or undefined",
            )),
        }
    }
}
