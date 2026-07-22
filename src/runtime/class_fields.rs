//! Public class-field property definition primitives.
//!
//! Fixed and computed public fields both use CreateDataProperty-style own
//! definition. Keeping that descriptor construction here prevents the VM host
//! from accidentally routing a future computed field through ordinary `Set`
//! or through a second observable `ToPropertyKey` conversion.

use super::*;

impl Runtime {
    pub(super) fn define_public_class_field(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        value: Value,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        let outcome = self.define_own_property_in_realm(
            Some(realm),
            object,
            key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(value),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(true),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        match outcome {
            PropertyDefineOutcome::Defined(true) | PropertyDefineOutcome::Throw(_) => Ok(outcome),
            PropertyDefineOutcome::Defined(false) => Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "property is not configurable",
            ))),
        }
    }

    fn class_initializer_callable(
        &self,
        value: Value,
        expected: ClassInitializerKind,
    ) -> Result<(CallableRef, ContextId), RuntimeError> {
        let callable = self.callable_from_value(value)?;
        let state = self.0.state.borrow();
        let object = state.heap.object(callable.as_object().object_id())?;
        let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
            return Err(RuntimeError::Invariant(
                "class initializer is not a bytecode function",
            ));
        };
        let bytecode = state.heap.function_bytecode(*bytecode)?;
        if object.is_constructor || bytecode.metadata.class_initializer_kind != Some(expected) {
            return Err(RuntimeError::Invariant(
                "class initializer bytecode has the wrong role",
            ));
        }
        let realm = bytecode.realm;
        drop(state);
        Ok((callable, realm))
    }

    fn class_constructor_object(
        &self,
        value: Value,
    ) -> Result<(ObjectRef, ContextId), RuntimeError> {
        let Value::Object(constructor) = value else {
            return Err(RuntimeError::Invariant(
                "class initializer hook received a primitive constructor",
            ));
        };
        if !constructor.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("class constructor"));
        }
        let state = self.0.state.borrow();
        let object = state.heap.object(constructor.object_id())?;
        let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
            return Err(RuntimeError::Invariant(
                "class initializer hook received a non-bytecode constructor",
            ));
        };
        let metadata = state.heap.function_bytecode(*bytecode)?.metadata;
        if !object.is_constructor
            || metadata.constructor_kind == ConstructorKind::None
            || metadata.has_prototype
            || !metadata.strict
            || metadata.class_initializer_kind.is_some()
        {
            return Err(RuntimeError::Invariant(
                "class initializer hook received malformed constructor bytecode",
            ));
        }
        let realm = state.heap.function_bytecode(*bytecode)?.realm;
        drop(state);
        Ok((constructor, realm))
    }

    fn published_class_prototype(
        &self,
        constructor: &ObjectRef,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype_key = self.intern_property_key("prototype")?;
        let Some(CompleteOrdinaryPropertyDescriptor::Data {
            value: Value::Object(prototype),
            writable: false,
            enumerable: false,
            configurable: false,
        }) = self.get_own_property(constructor, &prototype_key)?
        else {
            return Err(RuntimeError::Invariant(
                "class initializer owner has no authenticated prototype",
            ));
        };
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("class prototype"));
        }
        Ok(prototype)
    }

    fn validate_fresh_class_pair(
        &self,
        constructor: &ObjectRef,
        prototype: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let published = self.published_class_prototype(constructor)?;
        if published != *prototype {
            return Err(RuntimeError::Invariant(
                "class initializer prototype disagrees with its constructor",
            ));
        }
        let constructor_key = self.intern_property_key("constructor")?;
        if !matches!(
            self.get_own_property(prototype, &constructor_key)?,
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(ref owner),
                writable: true,
                enumerable: false,
                configurable: true,
            }) if owner == constructor
        ) {
            return Err(RuntimeError::Invariant(
                "class prototype has no authenticated constructor back-reference",
            ));
        }
        Ok(())
    }

    pub(super) fn install_class_instance_initializer(
        &self,
        caller_realm: ContextId,
        constructor: Value,
        prototype: Value,
        initializer: Value,
    ) -> Result<(), RuntimeError> {
        let (constructor, constructor_realm) = self.class_constructor_object(constructor)?;
        if caller_realm != constructor_realm {
            return Err(RuntimeError::Invariant(
                "class initializer bridge crossed constructor realms",
            ));
        }
        let Value::Object(prototype) = prototype else {
            return Err(RuntimeError::Invariant(
                "class instance initializer HomeObject is not an Object",
            ));
        };
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("class prototype"));
        }
        self.validate_fresh_class_pair(&constructor, &prototype)?;
        let (initializer, initializer_realm) =
            self.class_initializer_callable(initializer, ClassInitializerKind::InstanceFields)?;
        if initializer_realm != constructor_realm {
            return Err(RuntimeError::Invariant(
                "class instance initializer crossed its constructor realm",
            ));
        }
        let mut state = self.0.state.borrow_mut();
        state.heap.attach_bytecode_class_instance_initializer(
            constructor.object_id(),
            prototype.object_id(),
            initializer.as_object().object_id(),
        )?;
        Ok(())
    }

    pub(super) fn call_class_instance_initializer(
        &self,
        caller_realm: ContextId,
        constructor: Value,
        receiver: Value,
    ) -> Result<Completion, RuntimeError> {
        let (constructor, constructor_realm) = self.class_constructor_object(constructor)?;
        if caller_realm != constructor_realm {
            return Err(RuntimeError::Invariant(
                "class instance initializer call crossed constructor realms",
            ));
        }
        let prototype = self.published_class_prototype(&constructor)?;
        let Value::Object(receiver_object) = &receiver else {
            return Err(RuntimeError::Invariant(
                "class instance initializer receiver is not an Object",
            ));
        };
        if !receiver_object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime(
                "class instance initializer receiver",
            ));
        }
        let initializer = {
            let state = self.0.state.borrow();
            state
                .heap
                .bytecode_class_instance_initializer(constructor.object_id())?
        };
        let Some(initializer) = initializer else {
            return Ok(Completion::Return(Value::Undefined));
        };
        let initializer = ObjectRef::from_borrowed_handle(self.clone(), initializer)?;
        let (initializer, initializer_realm) = self.class_initializer_callable(
            Value::Object(initializer),
            ClassInitializerKind::InstanceFields,
        )?;
        if initializer_realm != constructor_realm
            || self.bytecode_function_home_object(initializer.as_object())? != Some(prototype)
        {
            return Err(RuntimeError::Invariant(
                "class instance initializer lost its authenticated owner",
            ));
        }
        self.call_internal(caller_realm, &initializer, receiver, &[])
    }

    pub(super) fn run_class_static_initializer(
        &self,
        caller_realm: ContextId,
        constructor: Value,
        initializer: Value,
    ) -> Result<Completion, RuntimeError> {
        let (constructor, constructor_realm) = self.class_constructor_object(constructor)?;
        if caller_realm != constructor_realm {
            return Err(RuntimeError::Invariant(
                "class static initializer bridge crossed constructor realms",
            ));
        }
        let prototype = self.published_class_prototype(&constructor)?;
        self.validate_fresh_class_pair(&constructor, &prototype)?;
        let (initializer, initializer_realm) =
            self.class_initializer_callable(initializer, ClassInitializerKind::StaticElements)?;
        if initializer_realm != constructor_realm
            || self
                .bytecode_function_home_object(initializer.as_object())?
                .is_some()
        {
            return Err(RuntimeError::Invariant(
                "class static initializer is not fresh in its constructor realm",
            ));
        }
        {
            let mut state = self.0.state.borrow_mut();
            state
                .heap
                .begin_bytecode_class_static_initializer(constructor.object_id())?;
        }
        self.install_object_literal_home_object(&initializer, &constructor)?;
        self.call_internal(caller_realm, &initializer, Value::Object(constructor), &[])
    }

    pub(super) fn call_class_static_block(
        &self,
        caller_realm: ContextId,
        static_initializer: &ObjectRef,
        this_value: Value,
        block: Value,
    ) -> Result<Completion, RuntimeError> {
        let (parent, parent_realm) = self.class_initializer_callable(
            Value::Object(static_initializer.clone()),
            ClassInitializerKind::StaticElements,
        )?;
        let (block, block_realm) =
            self.class_initializer_callable(block, ClassInitializerKind::StaticBlock)?;
        let Some(home_object) = self.bytecode_function_home_object(parent.as_object())? else {
            return Err(RuntimeError::Invariant(
                "class static block parent has no authenticated HomeObject",
            ));
        };
        if caller_realm != parent_realm
            || block_realm != parent_realm
            || this_value != Value::Object(home_object.clone())
            || self
                .bytecode_function_home_object(block.as_object())?
                .is_some()
        {
            return Err(RuntimeError::Invariant(
                "class static block is not fresh in its parent initializer",
            ));
        }
        self.install_object_literal_home_object(&block, &home_object)?;
        self.call_internal(caller_realm, &block, this_value, &[])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;

    fn bytecode_callable(
        runtime: &Runtime,
        context: &super::super::Context,
        code: Vec<Instruction>,
        metadata: FunctionMetadata,
    ) -> CallableRef {
        let function = runtime
            .publish_unlinked_function(
                context.realm,
                UnlinkedFunction::new(code, Vec::new(), metadata),
            )
            .unwrap();
        runtime
            .new_bytecode_closure(context.realm, &function)
            .unwrap()
    }

    fn computed_field_callable(runtime: &Runtime, context: &super::super::Context) -> CallableRef {
        bytecode_callable(
            runtime,
            context,
            vec![
                Instruction::GetArg(0),
                Instruction::GetArg(1),
                Instruction::GetArg(2),
                Instruction::DefineFieldComputed,
                Instruction::Return,
            ],
            FunctionMetadata {
                argument_count: 3,
                defined_argument_count: 3,
                max_stack: 3,
                strict: true,
                ..FunctionMetadata::default()
            },
        )
    }

    #[test]
    fn computed_field_bytecode_requires_three_operands_but_no_constant() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let callable = computed_field_callable(&runtime, &context);
        let published = runtime.heap_counts().function_bytecode_nodes;

        let malformed = UnlinkedFunction::new(
            vec![
                Instruction::Undefined,
                Instruction::Undefined,
                Instruction::DefineFieldComputed,
                Instruction::Return,
            ],
            Vec::new(),
            FunctionMetadata {
                max_stack: 2,
                ..FunctionMetadata::default()
            },
        );
        let RuntimeError::Engine(error) = runtime
            .publish_unlinked_function(context.realm, malformed)
            .unwrap_err()
        else {
            panic!("malformed computed field did not fail bytecode publication")
        };
        assert_eq!(error.message(), "bytecode stack underflow");
        assert_eq!(runtime.heap_counts().function_bytecode_nodes, published);
        drop(callable);
    }

    #[test]
    fn computed_field_defines_cwe_own_data_without_calling_inherited_setter() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let define = computed_field_callable(&runtime, &context);
        let throwing_setter = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::GetArg(0), Instruction::Throw],
            FunctionMetadata {
                argument_count: 1,
                defined_argument_count: 1,
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let prototype = runtime.new_object(None).unwrap();
        let object = runtime.new_object(Some(&prototype)).unwrap();
        let field = runtime.intern_property_key("field").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &prototype,
                    &field,
                    &OrdinaryPropertyDescriptor {
                        set: DescriptorField::Present(AccessorValue::Callable(throwing_setter)),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );

        assert_eq!(
            context
                .call(
                    &define,
                    Value::Undefined,
                    &[
                        Value::Object(object.clone()),
                        Value::String(JsString::from_static("field")),
                        Value::Int(42),
                    ],
                )
                .unwrap(),
            Value::Object(object.clone())
        );
        assert_eq!(
            runtime.get_own_property(&object, &field).unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(42),
                writable: true,
                enumerable: true,
                configurable: true,
            })
        );
        assert!(!context.has_exception());

        let symbol = runtime
            .new_symbol(Some(JsString::from_static("computed field")))
            .unwrap();
        assert_eq!(
            context
                .call(
                    &define,
                    Value::Undefined,
                    &[
                        Value::Object(object.clone()),
                        Value::Symbol(symbol.clone()),
                        Value::Int(7),
                    ],
                )
                .unwrap(),
            Value::Object(object.clone())
        );
        assert_eq!(
            runtime
                .get_own_property(&object, &PropertyKey::from(&symbol))
                .unwrap(),
            Some(CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Int(7),
                writable: true,
                enumerable: true,
                configurable: true,
            })
        );
    }

    #[test]
    fn computed_field_rejection_becomes_a_javascript_throw() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let define = computed_field_callable(&runtime, &context);
        let object = runtime.new_object(None).unwrap();
        runtime.prevent_extensions(&object).unwrap();

        assert_eq!(
            context.call(
                &define,
                Value::Undefined,
                &[
                    Value::Object(object),
                    Value::String(JsString::from_static("blocked")),
                    Value::Int(1),
                ],
            ),
            Err(RuntimeError::Exception)
        );
        let Value::Object(exception) = context.take_exception().unwrap().unwrap() else {
            panic!("computed field rejection did not materialize an Error object")
        };
        let name = runtime.intern_property_key("name").unwrap();
        let message = runtime.intern_property_key("message").unwrap();
        assert_eq!(
            context.get_property(&exception, &name).unwrap(),
            Value::String(JsString::from_static("TypeError"))
        );
        assert_eq!(
            context.get_property(&exception, &message).unwrap(),
            Value::String(JsString::from_static("property is not configurable"))
        );
    }

    #[test]
    fn computed_field_never_repeats_observable_property_key_conversion() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let define = computed_field_callable(&runtime, &context);
        let throwing_conversion = bytecode_callable(
            &runtime,
            &context,
            vec![Instruction::PushI32(99), Instruction::Throw],
            FunctionMetadata {
                max_stack: 1,
                strict: true,
                ..FunctionMetadata::default()
            },
        );
        let target = runtime.new_object(None).unwrap();
        let key_probe = runtime.new_object(None).unwrap();
        let to_string = runtime.intern_property_key("toString").unwrap();
        assert!(
            runtime
                .define_own_property(
                    &key_probe,
                    &to_string,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Object(
                            throwing_conversion.as_object().clone(),
                        )),
                        writable: DescriptorField::Present(true),
                        configurable: DescriptorField::Present(true),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
        );

        let RuntimeError::Engine(error) = context
            .call(
                &define,
                Value::Undefined,
                &[
                    Value::Object(target),
                    Value::Object(key_probe),
                    Value::Int(1),
                ],
            )
            .unwrap_err()
        else {
            panic!("uncanonicalized key did not fail at the VM/runtime boundary")
        };
        assert_eq!(
            error.message(),
            "computed property key was not canonicalized by ToPropKey"
        );
        assert_eq!(context.take_exception().unwrap(), None);
    }
}
