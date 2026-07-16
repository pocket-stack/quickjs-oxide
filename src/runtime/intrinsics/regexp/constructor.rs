//! `%RegExp%` construction and derived allocation.
//!
//! The ordering follows pinned QuickJS `js_regexp_constructor`, not a
//! rearranged specification sketch.  In particular `IsRegExp` runs first,
//! pattern conversion precedes the derived `.prototype` lookup, and flags are
//! converted only after the branded object has been allocated.

use std::rc::Rc;

use crate::heap::{RegExpObjectData, RegExpRealmData};
use crate::regexp::CompiledRegExp;

use super::super::super::*;

#[derive(Clone)]
struct GenuineRegExp {
    pattern: JsString,
    program: Rc<CompiledRegExp>,
}

impl Runtime {
    /// Pinned QuickJS `JS_SpeciesConstructor` specialized with this native
    /// method's defining-realm retained `%RegExp%` constructor as the default.
    pub(super) fn regexp_species_constructor(
        &self,
        realm: ContextId,
        regexp: &ObjectRef,
    ) -> Result<NativeConversion<CallableRef>, RuntimeError> {
        let default_id = self.regexp_realm_data(realm)?.constructor;
        let default = ObjectRef::from_borrowed_handle(self.clone(), default_id)?;
        let default = self.as_callable(&default)?.ok_or(RuntimeError::Invariant(
            "realm RegExp constructor root was not callable",
        ))?;

        let constructor_key = self.intern_property_key("constructor")?;
        let constructor = match self.get_property_in_realm(realm, regexp, &constructor_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(constructor, Value::Undefined) {
            return Ok(NativeConversion::Value(default));
        }
        let Value::Object(constructor) = constructor else {
            return Ok(NativeConversion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not an object",
            )?));
        };

        let species_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        let species = match self.get_property_in_realm(realm, &constructor, &species_key)? {
            Completion::Return(value) => value,
            Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
        };
        if matches!(species, Value::Undefined | Value::Null) {
            return Ok(NativeConversion::Value(default));
        }
        if !matches!(species, Value::Object(_)) {
            return Ok(NativeConversion::Throw(
                self.new_not_constructor_error(realm, &species)?,
            ));
        }
        self.constructor_from_value(realm, species)
    }

    pub(super) fn call_regexp_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Construct { mut new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp constructor did not receive constructor-or-function invocation",
            ));
        };
        let pattern_argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "RegExp constructor pattern argv was not padded",
        ))?;
        let flags_argument = arguments.readable.get(1).ok_or(RuntimeError::Invariant(
            "RegExp constructor flags argv was not padded",
        ))?;

        // This observable @@match lookup is deliberately the first semantic
        // operation, including for genuine RegExp objects.
        let pattern_is_regexp = match self.native_is_regexp(realm, pattern_argument)? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        if matches!(new_target, Value::Undefined) {
            let active_constructor = self.active_function()?;
            new_target = Value::Object(active_constructor.clone());
            if pattern_is_regexp && matches!(flags_argument, Value::Undefined) {
                let constructor_key = self.intern_property_key("constructor")?;
                let constructor = match self.get_value_property_in_realm(
                    realm,
                    pattern_argument.clone(),
                    &constructor_key,
                )? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                if constructor.same_value(&Value::Object(active_constructor)) {
                    return Ok(Completion::Return(pattern_argument.clone()));
                }
            }
        }

        // Exact brand checking remains independent of IsRegExp.  A genuine
        // object whose @@match is false still copies its internal source and
        // program; only the function-call identity shortcut was suppressed.
        let genuine = self.genuine_regexp(pattern_argument)?;
        if matches!(flags_argument, Value::Undefined) {
            if let Some(genuine) = genuine.as_ref() {
                let object = match self.allocate_regexp_from_new_target(realm, new_target)? {
                    NativeConversion::Value(object) => object,
                    NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
                };
                self.publish_regexp(&object, genuine.pattern.clone(), genuine.program.clone())?;
                return Ok(Completion::Return(Value::Object(object)));
            }
        }

        let (pattern_value, flags_value) = if let Some(genuine) = genuine {
            (Value::String(genuine.pattern), flags_argument.clone())
        } else if pattern_is_regexp {
            let Value::Object(pattern_object) = pattern_argument else {
                return Err(RuntimeError::Invariant(
                    "IsRegExp accepted a primitive pattern",
                ));
            };
            let source_key = self.intern_property_key("source")?;
            let pattern = match self.get_property_in_realm(realm, pattern_object, &source_key)? {
                Completion::Return(value) => value,
                Completion::Throw(value) => return Ok(Completion::Throw(value)),
            };
            let flags = if matches!(flags_argument, Value::Undefined) {
                let flags_key = self.intern_property_key("flags")?;
                match self.get_property_in_realm(realm, pattern_object, &flags_key)? {
                    Completion::Return(value) => value,
                    Completion::Throw(value) => return Ok(Completion::Throw(value)),
                }
            } else {
                flags_argument.clone()
            };
            (pattern, flags)
        } else {
            (pattern_argument.clone(), flags_argument.clone())
        };

        let pattern = if matches!(pattern_value, Value::Undefined) {
            JsString::from_static("")
        } else {
            match self.native_to_js_string(realm, &pattern_value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };

        // Pinned QuickJS performs this Get(newTarget, "prototype") and the
        // branded allocation before ToString(flags) and compilation.
        let object = match self.allocate_regexp_from_new_target(realm, new_target)? {
            NativeConversion::Value(object) => object,
            NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
        };

        let flags = if matches!(flags_value, Value::Undefined) {
            JsString::from_static("")
        } else {
            match self.native_to_js_string(realm, &flags_value)? {
                NativeConversion::Value(value) => value,
                NativeConversion::Throw(value) => return Ok(Completion::Throw(value)),
            }
        };
        let program = match crate::regexp::compile(&pattern, &flags) {
            Ok(program) => Rc::new(program),
            Err(error) => {
                let kind = crate::regexp::javascript_compile_error_kind(&error);
                let message = if kind == ErrorKind::Unsupported {
                    error.to_string()
                } else {
                    crate::regexp::javascript_compile_error_message(&error).to_owned()
                };
                return Err(RuntimeError::Engine(Error::new(kind, message)));
            }
        };
        self.publish_regexp(&object, pattern, program)?;
        Ok(Completion::Return(Value::Object(object)))
    }

    pub(super) fn call_regexp_species(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp species did not receive a getter invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    fn genuine_regexp(&self, value: &Value) -> Result<Option<GenuineRegExp>, RuntimeError> {
        let Value::Object(object) = value else {
            return Ok(None);
        };
        if !object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("RegExp argument"));
        }
        let state = self.0.state.borrow();
        Ok(match &state.heap.object(object.object_id())?.payload {
            ObjectPayload::RegExp(RegExpObjectData::Compiled { pattern, program }) => {
                Some(GenuineRegExp {
                    pattern: pattern.clone(),
                    program: program.clone(),
                })
            }
            ObjectPayload::RegExp(RegExpObjectData::Uninitialized) => {
                return Err(RuntimeError::Invariant(
                    "observable RegExp object was not initialized",
                ));
            }
            ObjectPayload::Ordinary
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. } => None,
        })
    }

    fn allocate_regexp_from_new_target(
        &self,
        caller_realm: ContextId,
        new_target: Value,
    ) -> Result<NativeConversion<ObjectRef>, RuntimeError> {
        let Value::Object(new_target_object) = new_target else {
            return Err(RuntimeError::Invariant(
                "RegExp constructor new.target was not an object",
            ));
        };
        let prototype_key = self.intern_property_key("prototype")?;
        let prototype =
            match self.get_property_in_realm(caller_realm, &new_target_object, &prototype_key)? {
                Completion::Return(Value::Object(prototype)) => prototype,
                Completion::Return(_) => {
                    // GetFunctionRealm is intentionally delayed until after the
                    // observable prototype Get returned a non-object.
                    let callable = self.callable_from_value(Value::Object(new_target_object))?;
                    let fallback_realm = self.callable_realm(&callable)?;
                    let prototype = self.regexp_realm_data(fallback_realm)?.prototype;
                    ObjectRef::from_borrowed_handle(self.clone(), prototype)?
                }
                Completion::Throw(value) => return Ok(NativeConversion::Throw(value)),
            };
        Ok(NativeConversion::Value(
            self.new_uninitialized_regexp(&prototype)?,
        ))
    }

    fn new_uninitialized_regexp(&self, prototype: &ObjectRef) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        if !prototype.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("RegExp prototype"));
        }
        let last_index = self.intern_property_key("lastIndex")?;
        let entries = [ShapeEntry {
            atom: last_index.atom(),
            flags: PropertyFlags::data(true, false, false),
        }];
        let mut state = self.0.state.borrow_mut();
        let shape = state.get_or_create_shape(Some(prototype.object_id()), &entries)?;
        let object = match state.heap.allocate_object(ObjectData::regexp(
            shape,
            vec![PropertySlot::Data(RawValue::Int(0))],
        )) {
            Ok(object) => object,
            Err(error) => {
                let cleanup = state.heap.release_shape(shape)?;
                state.apply_cleanup(cleanup)?;
                return Err(error.into());
            }
        };
        let cleanup = state.heap.release_shape(shape)?;
        state.apply_cleanup(cleanup)?;
        drop(state);
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }

    fn publish_regexp(
        &self,
        object: &ObjectRef,
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    ) -> Result<(), RuntimeError> {
        let previous = self.0.state.borrow_mut().heap.replace_regexp_data(
            object.object_id(),
            RegExpObjectData::Compiled { pattern, program },
        )?;
        if !matches!(previous, RegExpObjectData::Uninitialized) {
            return Err(RuntimeError::Invariant(
                "fresh RegExp object already had compiled data",
            ));
        }
        Ok(())
    }

    pub(super) fn regexp_realm_data(
        &self,
        realm: ContextId,
    ) -> Result<RegExpRealmData, RuntimeError> {
        self.0
            .state
            .borrow()
            .heap
            .context(realm)?
            .regexp
            .as_ref()
            .copied()
            .ok_or(RuntimeError::Invariant("realm has no RegExp intrinsic"))
    }

    /// Call the realm's retained intrinsic RegExp constructor as a constructor.
    /// String protocol fallbacks use this root directly, so replacing the
    /// global `RegExp` binding is unobservable while mutations on the retained
    /// constructor (notably its `prototype` value) remain visible to ordinary
    /// construction.
    pub(in crate::runtime) fn construct_intrinsic_regexp(
        &self,
        realm: ContextId,
        pattern: Value,
    ) -> Result<Completion, RuntimeError> {
        let constructor_id = self.regexp_realm_data(realm)?.constructor;
        let constructor = ObjectRef::from_borrowed_handle(self.clone(), constructor_id)?;
        let callable = self
            .as_callable(&constructor)?
            .ok_or(RuntimeError::Invariant(
                "realm RegExp constructor root was not callable",
            ))?;
        self.construct_internal(realm, &callable, &callable, &[pattern])
    }

    /// QuickJS `OP_regexp`: instantiate one already-compiled literal using
    /// the bytecode realm's canonical RegExp shape. This path intentionally
    /// performs no `Get` on the global constructor or its mutable `prototype`
    /// property and therefore cannot invoke user code.
    pub(in crate::runtime) fn new_compiled_regexp_literal(
        &self,
        realm: ContextId,
        pattern: JsString,
        program: Rc<CompiledRegExp>,
    ) -> Result<ObjectRef, RuntimeError> {
        let _operation = self.operation();
        let shape = self.regexp_realm_data(realm)?.object_shape;
        let object =
            self.0
                .state
                .borrow_mut()
                .heap
                .allocate_object(ObjectData::compiled_regexp(
                    shape,
                    vec![PropertySlot::Data(RawValue::Int(0))],
                    pattern,
                    program,
                ))?;
        Ok(ObjectRef::from_owned_handle(self.clone(), object))
    }
}
