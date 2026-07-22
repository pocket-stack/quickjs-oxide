//! Base class constructor/prototype publication.
//!
//! Pinned QuickJS keeps `OP_define_class` separate from ordinary closure
//! instantiation: the constructor is rooted in `%Function.prototype%`, while
//! its public `.prototype` is a fresh object rooted in `%Object.prototype%`.
//! This module owns that cycle and its exact descriptor shapes without growing
//! the main runtime implementation.

use super::*;

impl Runtime {
    pub(super) fn define_class_pair(
        &self,
        realm: ContextId,
        parent: Value,
        constructor: Value,
        name: &JsString,
        has_heritage: bool,
    ) -> Result<DefineClassOutcome, RuntimeError> {
        if has_heritage {
            return Err(RuntimeError::Engine(Error::internal(
                "class heritage definition is not implemented yet",
            )));
        }
        if !matches!(parent, Value::Undefined) {
            return Err(RuntimeError::Invariant(
                "base class definition parent was not undefined",
            ));
        }

        let constructor = self.callable_from_value(constructor)?;
        self.validate_base_class_constructor(realm, &constructor)?;

        let object_prototype = {
            let prototype = self.0.state.borrow().heap.context(realm)?.object_prototype;
            ObjectRef::from_borrowed_handle(self.clone(), prototype)?
        };
        let prototype = self.new_object(Some(&object_prototype))?;

        self.define_class_constructor_name(constructor.as_object(), name)?;
        // QuickJS installs the hidden HomeObject before making the public
        // constructor/prototype cycle. The metadata gate keeps constructors
        // which never read `super` free of an unnecessary heap edge.
        self.install_object_literal_home_object(&constructor, &prototype)?;
        self.define_function_data_property(
            &prototype,
            "constructor",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_function_data_property(
            constructor.as_object(),
            "prototype",
            Value::Object(prototype.clone()),
            false,
            false,
        )?;

        Ok(DefineClassOutcome::Defined {
            constructor: Value::Object(constructor.as_object().clone()),
            prototype: Value::Object(prototype),
        })
    }

    fn define_class_constructor_name(
        &self,
        constructor: &ObjectRef,
        name: &JsString,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("name")?;
        let defined = self.define_own_property(
            constructor,
            &key,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(name.clone())),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )?;
        if !defined {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "cannot define class name",
            )));
        }
        Ok(())
    }

    fn validate_base_class_constructor(
        &self,
        realm: ContextId,
        constructor: &CallableRef,
    ) -> Result<(), RuntimeError> {
        if !constructor.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("class constructor"));
        }

        let (metadata, is_constructor) = {
            let state = self.0.state.borrow();
            state.heap.context(realm)?;
            let object = state.heap.object(constructor.as_object().object_id())?;
            let ObjectPayload::BytecodeFunction { bytecode, .. } = &object.payload else {
                return Err(RuntimeError::Invariant(
                    "class constructor closure was not a bytecode function",
                ));
            };
            (
                state.heap.function_bytecode(*bytecode)?.metadata,
                object.is_constructor,
            )
        };
        if metadata.constructor_kind != ConstructorKind::Base || !is_constructor {
            return Err(RuntimeError::Invariant(
                "class constructor closure was not a base constructor",
            ));
        }
        if metadata.has_prototype {
            return Err(RuntimeError::Invariant(
                "class constructor closure already requested an ordinary prototype",
            ));
        }

        let function_prototype = {
            let prototype = self
                .0
                .state
                .borrow()
                .heap
                .context(realm)?
                .function_prototype;
            ObjectRef::from_borrowed_handle(self.clone(), prototype)?
        };
        if self.get_prototype_of(constructor.as_object())? != Some(function_prototype) {
            return Err(RuntimeError::Invariant(
                "base class constructor did not inherit from Function.prototype",
            ));
        }

        let prototype_key = self.intern_property_key("prototype")?;
        if self
            .get_own_property(constructor.as_object(), &prototype_key)?
            .is_some()
        {
            return Err(RuntimeError::Invariant(
                "class constructor already had an own prototype property",
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::Instruction;
    use crate::function::UnlinkedFunction;

    fn class_constructor(
        runtime: &Runtime,
        realm: ContextId,
        needs_home_object: bool,
    ) -> CallableRef {
        let bytecode = runtime
            .publish_unlinked_function(
                realm,
                UnlinkedFunction::new(
                    vec![
                        Instruction::CheckCtor,
                        Instruction::Undefined,
                        Instruction::Return,
                    ],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        strict: true,
                        needs_home_object,
                        has_prototype: false,
                        constructor_kind: ConstructorKind::Base,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        runtime.new_bytecode_closure(realm, &bytecode).unwrap()
    }

    fn own_descriptor(
        runtime: &Runtime,
        object: &ObjectRef,
        name: &str,
    ) -> CompleteOrdinaryPropertyDescriptor {
        let key = runtime.intern_property_key(name).unwrap();
        runtime.get_own_property(object, &key).unwrap().unwrap()
    }

    #[test]
    fn base_class_publication_creates_the_quickjs_constructor_cycle() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let constructor = class_constructor(&runtime, context.realm, true);

        let DefineClassOutcome::Defined {
            constructor: returned_constructor,
            prototype: returned_prototype,
        } = runtime
            .define_class_pair(
                context.realm,
                Value::Undefined,
                Value::Object(constructor.as_object().clone()),
                &JsString::from_static("C"),
                false,
            )
            .unwrap()
        else {
            panic!("base class definition unexpectedly threw")
        };
        assert_eq!(
            returned_constructor,
            Value::Object(constructor.as_object().clone())
        );
        let Value::Object(prototype) = returned_prototype else {
            panic!("class prototype was not an object")
        };

        assert_eq!(
            runtime.get_prototype_of(constructor.as_object()).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert_eq!(
            runtime.get_prototype_of(&prototype).unwrap(),
            Some(context.object_prototype().unwrap())
        );
        assert_eq!(
            own_descriptor(&runtime, constructor.as_object(), "name"),
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::String(JsString::from_static("C")),
                writable: false,
                enumerable: false,
                configurable: true,
            }
        );
        assert_eq!(
            own_descriptor(&runtime, constructor.as_object(), "prototype"),
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(prototype.clone()),
                writable: false,
                enumerable: false,
                configurable: false,
            }
        );
        assert_eq!(
            own_descriptor(&runtime, &prototype, "constructor"),
            CompleteOrdinaryPropertyDescriptor::Data {
                value: Value::Object(constructor.as_object().clone()),
                writable: true,
                enumerable: false,
                configurable: true,
            }
        );
        assert_eq!(
            runtime
                .bytecode_function_home_object(constructor.as_object())
                .unwrap(),
            Some(prototype)
        );
    }

    #[test]
    fn base_class_publication_rejects_unimplemented_heritage_without_mutation() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let constructor = class_constructor(&runtime, context.realm, false);

        let error = runtime
            .define_class_pair(
                context.realm,
                Value::Undefined,
                Value::Object(constructor.as_object().clone()),
                &JsString::from_static("Derived"),
                true,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RuntimeError::Engine(ref error)
                if error.kind() == ErrorKind::Internal
                    && error.message() == "class heritage definition is not implemented yet"
        ));
        let prototype = runtime.intern_property_key("prototype").unwrap();
        assert_eq!(
            runtime
                .get_own_property(constructor.as_object(), &prototype)
                .unwrap(),
            None
        );
    }
}
