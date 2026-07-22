//! Class constructor/prototype publication.
//!
//! Pinned QuickJS keeps `OP_define_class` separate from ordinary closure
//! instantiation.  With no heritage the constructor is rooted in
//! `%Function.prototype%` and its public `.prototype` in `%Object.prototype%`.
//! With heritage, QuickJS first validates the parent constructor and reads its
//! public `.prototype`, then roots the constructor in the parent and the fresh
//! instance prototype in that validated object (or null).  This module owns
//! that cycle and its exact descriptor shapes without growing the main runtime
//! implementation.

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
        // Match QuickJS js_op_define_class ordering.  In particular, an
        // accessor at Parent.prototype runs only after Parent has passed
        // IsConstructor, and every JavaScript-visible abrupt completion occurs
        // before the candidate constructor is mutated.
        let (constructor_parent, prototype_parent, expected_constructor_kind) = if has_heritage {
            match parent {
                Value::Null => {
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
                    (function_prototype, None, ConstructorKind::Derived)
                }
                Value::Object(parent_constructor) => {
                    if !self.is_constructor(&parent_constructor)? {
                        return Err(RuntimeError::Engine(Error::new(
                            ErrorKind::Type,
                            "parent class must be constructor",
                        )));
                    }
                    let prototype_key = self.intern_property_key("prototype")?;
                    let parent_prototype = match self.get_property_in_realm(
                        realm,
                        &parent_constructor,
                        &prototype_key,
                    )? {
                        Completion::Return(Value::Object(prototype)) => Some(prototype),
                        Completion::Return(Value::Null) => None,
                        Completion::Return(_) => {
                            return Err(RuntimeError::Engine(Error::new(
                                ErrorKind::Type,
                                "parent prototype must be an object or null",
                            )));
                        }
                        Completion::Throw(value) => {
                            return Ok(DefineClassOutcome::Throw(value));
                        }
                    };
                    (
                        parent_constructor,
                        parent_prototype,
                        ConstructorKind::Derived,
                    )
                }
                _ => {
                    return Err(RuntimeError::Engine(Error::new(
                        ErrorKind::Type,
                        "parent class must be constructor",
                    )));
                }
            }
        } else {
            if !matches!(parent, Value::Undefined) {
                return Err(RuntimeError::Invariant(
                    "base class definition parent was not undefined",
                ));
            }
            let (function_prototype, object_prototype) = {
                let state = self.0.state.borrow();
                let context = state.heap.context(realm)?;
                (context.function_prototype, context.object_prototype)
            };
            (
                ObjectRef::from_borrowed_handle(self.clone(), function_prototype)?,
                Some(ObjectRef::from_borrowed_handle(
                    self.clone(),
                    object_prototype,
                )?),
                ConstructorKind::Base,
            )
        };

        let constructor = self.callable_from_value(constructor)?;
        self.validate_class_constructor(realm, &constructor, expected_constructor_kind)?;

        // The new prototype is not reachable until the operation succeeds.
        // After the heritage getter above, the remaining definitions target
        // fresh extensible objects and cannot invoke user code.
        let prototype = self.new_object(prototype_parent.as_ref())?;
        if !self.set_prototype_of(constructor.as_object(), Some(&constructor_parent))? {
            return Err(RuntimeError::Invariant(
                "fresh class constructor rejected its authenticated parent",
            ));
        }

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

    fn validate_class_constructor(
        &self,
        realm: ContextId,
        constructor: &CallableRef,
        expected_kind: ConstructorKind,
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
        if metadata.constructor_kind != expected_kind || !is_constructor {
            return Err(RuntimeError::Invariant(
                "class constructor closure did not match its heritage",
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
                "unpublished class constructor did not inherit from Function.prototype",
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

    fn eval_object(context: &mut Context, source: &str) -> ObjectRef {
        let Value::Object(object) = context.eval(source).unwrap() else {
            panic!("{source} did not evaluate to an Object");
        };
        object
    }

    fn eval_string(context: &mut Context, source: &str) -> String {
        let Value::String(value) = context.eval(source).unwrap() else {
            panic!("{source} did not evaluate to a String");
        };
        value.to_utf8_lossy()
    }

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
                        Instruction::PushThis,
                        Instruction::PushActiveFunction,
                        Instruction::CallClassInstanceInitializer,
                        Instruction::Drop,
                        Instruction::Undefined,
                        Instruction::Return,
                    ],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 2,
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
    fn derived_class_publication_roots_both_sides_in_the_validated_parent() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let derived = eval_object(
            &mut context,
            r#"
                var HeritageParent = class {
                    constructor(a, b) { this.sum = a + b; }
                };
                var HeritageDerived = class extends HeritageParent {};
                HeritageDerived
            "#,
        );
        let parent = eval_object(&mut context, "HeritageParent");
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let Value::Object(parent_prototype) =
            context.get_property(&parent, &prototype_key).unwrap()
        else {
            panic!("parent class had no Object prototype")
        };
        let Value::Object(derived_prototype) =
            context.get_property(&derived, &prototype_key).unwrap()
        else {
            panic!("derived class had no Object prototype")
        };

        assert_eq!(runtime.get_prototype_of(&derived).unwrap(), Some(parent));
        assert_eq!(
            runtime.get_prototype_of(&derived_prototype).unwrap(),
            Some(parent_prototype)
        );
        assert_eq!(
            context.eval("(new HeritageDerived(20, 22)).sum").unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn explicit_super_initializes_one_tdz_cell_across_lexical_execution_paths() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"
                        (() => {
                            class Base {
                                constructor(value) { this.value = value; }
                            }
                            class Direct extends Base {
                                constructor() { super(42); }
                            }
                            class Arrow extends Base {
                                constructor() { (() => super(42))(); }
                            }
                            class Eval extends Base {
                                constructor() { eval("super(42)"); }
                            }
                            class Parameter extends Base {
                                constructor(value = super(42)) {}
                            }
                            if (new Direct().value !== 42
                                || new Arrow().value !== 42
                                || new Eval().value !== 42
                                || new Parameter().value !== 42) {
                                throw new Error("derived this initialization diverged");
                            }
                            return 42;
                        })()
                    "#,
                )
                .unwrap(),
            Value::Int(42)
        );
    }

    #[test]
    fn extends_null_uses_null_instance_parent_and_function_prototype_constructor_parent() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let derived = eval_object(
            &mut context,
            r#"
                var NullDerived = class extends null {
                    constructor() { return {}; }
                };
                NullDerived
            "#,
        );
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let Value::Object(derived_prototype) =
            context.get_property(&derived, &prototype_key).unwrap()
        else {
            panic!("derived class had no Object prototype")
        };

        assert_eq!(
            runtime.get_prototype_of(&derived).unwrap(),
            Some(context.function_prototype().unwrap())
        );
        assert_eq!(runtime.get_prototype_of(&derived_prototype).unwrap(), None);
        assert!(matches!(
            context.eval("new NullDerived()").unwrap(),
            Value::Object(_)
        ));
    }

    #[test]
    fn heritage_validation_precedes_prototype_access_and_does_not_mutate_the_candidate() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();

        assert_eq!(
            eval_string(
                &mut context,
                r#"
                    (() => {
                        let reads = 0;
                        const parent = {
                            get prototype() { reads++; return null; }
                        };
                        try { class Derived extends parent {} }
                        catch (error) { return error.message + "|" + reads; }
                    })()
                "#,
            ),
            "parent class must be constructor|0"
        );
        assert_eq!(
            eval_string(
                &mut context,
                r#"
                    (() => {
                        let reads = 0;
                        let computedKeys = 0;
                        const parent = (function () {}).bind(null);
                        Object.defineProperty(parent, "prototype", {
                            configurable: true,
                            get() { reads++; return 1; }
                        });
                        try {
                            class Derived extends parent {
                                [computedKeys++]() {}
                            }
                        } catch (error) {
                            return error.message + "|" + reads + "|" + computedKeys;
                        }
                    })()
                "#,
            ),
            "parent prototype must be an object or null|1|0"
        );

        // Exercise the same abrupt boundary directly with a retained candidate:
        // neither its [[Prototype]] nor an own `.prototype` may change.
        let candidate = class_constructor(&runtime, context.realm, false);
        let function_prototype = context.function_prototype().unwrap();
        let prototype_key = runtime.intern_property_key("prototype").unwrap();
        let error = runtime
            .define_class_pair(
                context.realm,
                Value::Int(1),
                Value::Object(candidate.as_object().clone()),
                &JsString::from_static("Derived"),
                true,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RuntimeError::Engine(ref error)
                if error.kind() == ErrorKind::Type
                    && error.message() == "parent class must be constructor"
        ));
        assert_eq!(
            runtime.get_prototype_of(candidate.as_object()).unwrap(),
            Some(function_prototype)
        );
        assert_eq!(
            runtime
                .get_own_property(candidate.as_object(), &prototype_key)
                .unwrap(),
            None
        );
    }

    #[test]
    fn class_constructor_metadata_must_match_the_heritage_form() {
        let runtime = Runtime::new();
        let context = runtime.new_context();
        let base_constructor = class_constructor(&runtime, context.realm, false);

        let error = runtime
            .define_class_pair(
                context.realm,
                Value::Null,
                Value::Object(base_constructor.as_object().clone()),
                &JsString::from_static("Derived"),
                true,
            )
            .unwrap_err();
        assert!(matches!(
            error,
            RuntimeError::Invariant("class constructor closure did not match its heritage")
        ));
        assert_eq!(
            runtime
                .get_own_property(
                    base_constructor.as_object(),
                    &runtime.intern_property_key("prototype").unwrap(),
                )
                .unwrap(),
            None
        );
    }
}
