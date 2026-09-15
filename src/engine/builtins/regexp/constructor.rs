//! `%RegExp%` construction and derived allocation.
//!
//! The ordering follows pinned QuickJS `js_regexp_constructor`, not a
//! rearranged specification sketch.  In particular `IsRegExp` runs first,
//! pattern conversion precedes the derived `.prototype` lookup, and flags are
//! converted only after the branded object has been allocated.

use crate::engine::api::error::Error;
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::heap::{
    ContextId, ObjectData, ObjectPayload, PropertySlot, RawValue, RegExpObjectData, RegExpRealmData,
};
use crate::engine::object::shape::{PropertyFlags, ShapeEntry};
use crate::engine::object::{ObjectRef, PropertyKey, WellKnownSymbol};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::call::{
    ConstructorPrototypeSource, ConstructorRef, NativeArguments, NativeInvocation,
    prototype::{ProtoSourceStep, finish as finish_source},
};
use crate::engine::vm::{Completion, ToPrimitiveHint};
use crate::regexp::CompiledRegExp;
use std::rc::Rc;

#[derive(Clone)]
pub(crate) struct GenuineRegExp {
    pub(crate) pattern: JsString,
    pub(crate) program: Rc<CompiledRegExp>,
}

impl Runtime {
    /// Pinned QuickJS `JS_SpeciesConstructor` specialized with this native
    /// method's defining-realm retained `%RegExp%` constructor as the default.
    pub(crate) fn regexp_species_constructor(
        &self,
        realm: ContextId,
        regexp: &ObjectRef,
    ) -> Result<NativeConversion<ConstructorRef>, RuntimeError> {
        let mut step = super::species::RegExpSpeciesStep::start(self, realm, regexp.clone())?;
        loop {
            step = match step {
                super::species::RegExpSpeciesStep::Complete(result) => return Ok(result),
                super::species::RegExpSpeciesStep::Read {
                    object,
                    key,
                    resume,
                } => resume.resume(self, self.get_property_in_realm(realm, &object, &key)?)?,
            };
        }
    }

    pub(crate) fn call_regexp_constructor(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish_constructor(
            self,
            realm,
            RegExpConstructorStep::start(self, realm, &invocation, arguments)?,
        )
    }

    pub(crate) fn compile_regexp_program(
        pattern: &JsString,
        flags: &JsString,
    ) -> Result<Rc<CompiledRegExp>, RuntimeError> {
        match crate::regexp::compile(pattern, flags) {
            Ok(program) => Ok(Rc::new(program)),
            Err(error) => {
                let kind = crate::regexp::javascript_compile_error_kind(&error);
                let message = crate::regexp::javascript_compile_error_message(&error).to_owned();
                Err(RuntimeError::Engine(Error::new(kind, message)))
            }
        }
    }

    pub(crate) fn call_regexp_species(
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

    pub(crate) fn genuine_regexp(
        &self,
        value: &Value,
    ) -> Result<Option<GenuineRegExp>, RuntimeError> {
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
            | ObjectPayload::Proxy(_)
            | ObjectPayload::RawJson
            | ObjectPayload::Promise(_)
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::IteratorHelper(_)
            | ObjectPayload::IteratorWrap(_)
            | ObjectPayload::AsyncFromSyncIterator(_)
            | ObjectPayload::IteratorConcat(_)
            | ObjectPayload::Map { .. }
            | ObjectPayload::MapIterator { .. }
            | ObjectPayload::Set { .. }
            | ObjectPayload::WeakMap { .. }
            | ObjectPayload::WeakSet { .. }
            | ObjectPayload::WeakRef { .. }
            | ObjectPayload::FinalizationRegistry(_)
            | ObjectPayload::SetIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::ArrayBuffer(_)
            | ObjectPayload::SharedArrayBuffer(_)
            | ObjectPayload::DataView(_)
            | ObjectPayload::TypedArray(_)
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. }
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::NativeFunction { .. }
            | ObjectPayload::BoundFunction { .. }
            | ObjectPayload::BytecodeFunction { .. }
            | ObjectPayload::AsyncFunctionState(_)
            | ObjectPayload::Generator { .. }
            | ObjectPayload::AsyncGenerator(_) => None,
        })
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

    pub(crate) fn regexp_realm_data(
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

    /// QuickJS `OP_regexp`: instantiate one already-compiled literal using
    /// the bytecode realm's canonical RegExp shape. This path intentionally
    /// performs no `Get` on the global constructor or its mutable `prototype`
    /// property and therefore cannot invoke user code.
    pub(crate) fn new_compiled_regexp_literal(
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

pub(crate) enum RegExpConstructorStep {
    Complete(Completion),
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: RegExpConstructorResume,
    },
    Primitive {
        value: Value,
        resume: RegExpConstructorResume,
    },
    Prototype {
        new_target: Value,
        resume: RegExpConstructorResume,
    },
}
pub(crate) struct RegExpConstructorResume(Box<RegExpConstructorResumeState>);
impl std::ops::Deref for RegExpConstructorResume {
    type Target = RegExpConstructorResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for RegExpConstructorResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<RegExpConstructorResume>() <= 8);
pub(crate) struct RegExpConstructorResumeState {
    realm: ContextId,
    new_target: Value,
    pattern: Value,
    flags: Value,
    is_regexp: bool,
    phase: RegExpConstructorPhase,
}
enum RegExpConstructorPhase {
    Match,
    Identity(ObjectRef),
    Source,
    SourceFlags(Value),
    Pattern(Value),
    Prototype(RegExpPublication),
    Flags {
        object: ObjectRef,
        pattern: JsString,
    },
}
enum RegExpPublication {
    Copy(GenuineRegExp),
    Compile { pattern: JsString, flags: Value },
}
impl RegExpConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp constructor did not receive constructor-or-function invocation",
            ));
        };
        let pattern = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant(
                "RegExp constructor pattern argv was not padded",
            ))?
            .clone();
        let flags = arguments
            .readable
            .get(1)
            .ok_or(RuntimeError::Invariant(
                "RegExp constructor flags argv was not padded",
            ))?
            .clone();
        let resume = RegExpConstructorResume(Box::new(RegExpConstructorResumeState {
            realm,
            new_target: new_target.clone(),
            pattern,
            flags,
            is_regexp: false,
            phase: RegExpConstructorPhase::Match,
        }));
        if let Value::Object(object) = &resume.pattern {
            Ok(Self::Read {
                object: object.clone(),
                key: PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::Match)),
                resume,
            })
        } else {
            resume.checked(runtime, false)
        }
    }
}
impl RegExpConstructorResume {
    fn checked(
        mut self,
        runtime: &Runtime,
        is_regexp: bool,
    ) -> Result<RegExpConstructorStep, RuntimeError> {
        self.0.is_regexp = is_regexp;
        if matches!(self.0.new_target, Value::Undefined) {
            let active = runtime.active_function()?;
            self.0.new_target = Value::Object(active.clone());
            if is_regexp && matches!(self.0.flags, Value::Undefined) {
                let Value::Object(object) = &self.0.pattern else {
                    return Err(RuntimeError::Invariant(
                        "IsRegExp accepted a primitive pattern",
                    ));
                };
                return Ok(RegExpConstructorStep::Read {
                    object: object.clone(),
                    key: runtime.intern_property_key("constructor")?,
                    resume: {
                        let updated_0 = RegExpConstructorPhase::Identity(active);
                        self.0.phase = updated_0;
                        self
                    },
                });
            }
        }
        self.prepare(runtime)
    }
    fn prepare(mut self, runtime: &Runtime) -> Result<RegExpConstructorStep, RuntimeError> {
        let genuine = runtime.genuine_regexp(&self.0.pattern)?;
        if matches!(self.0.flags, Value::Undefined)
            && let Some(genuine) = genuine.as_ref()
        {
            return Ok(self.lookup(RegExpPublication::Copy(genuine.clone())));
        }
        if let Some(genuine) = genuine {
            let flags = self.0.flags.clone();
            self.pattern_value(Value::String(genuine.pattern), flags)
        } else if self.0.is_regexp {
            let Value::Object(object) = &self.0.pattern else {
                return Err(RuntimeError::Invariant(
                    "IsRegExp accepted a primitive pattern",
                ));
            };
            Ok(RegExpConstructorStep::Read {
                object: object.clone(),
                key: runtime.intern_property_key("source")?,
                resume: {
                    let updated_0 = RegExpConstructorPhase::Source;
                    self.0.phase = updated_0;
                    self
                },
            })
        } else {
            let pattern = self.0.pattern.clone();
            let flags = self.0.flags.clone();
            self.pattern_value(pattern, flags)
        }
    }
    fn pattern_value(
        mut self,
        pattern: Value,
        flags: Value,
    ) -> Result<RegExpConstructorStep, RuntimeError> {
        if matches!(pattern, Value::Undefined) {
            Ok(self.lookup(RegExpPublication::Compile {
                pattern: JsString::from_static(""),
                flags,
            }))
        } else {
            Ok(RegExpConstructorStep::Primitive {
                value: pattern,
                resume: {
                    let updated_0 = RegExpConstructorPhase::Pattern(flags);
                    self.0.phase = updated_0;
                    self
                },
            })
        }
    }
    fn lookup(mut self, publication: RegExpPublication) -> RegExpConstructorStep {
        RegExpConstructorStep::Prototype {
            new_target: self.0.new_target.clone(),
            resume: {
                let updated_0 = RegExpConstructorPhase::Prototype(publication);
                self.0.phase = updated_0;
                self
            },
        }
    }
    fn publish(
        runtime: &Runtime,
        object: ObjectRef,
        pattern: JsString,
        flags: JsString,
    ) -> Result<RegExpConstructorStep, RuntimeError> {
        let program = Runtime::compile_regexp_program(&pattern, &flags)?;
        runtime.publish_regexp(&object, pattern, program)?;
        Ok(RegExpConstructorStep::Complete(Completion::Return(
            Value::Object(object),
        )))
    }
    pub(crate) fn prototype(
        mut self,
        runtime: &Runtime,
        result: NativeConversion<ConstructorPrototypeSource>,
    ) -> Result<RegExpConstructorStep, RuntimeError> {
        let prototype = match result {
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(value)) => value,
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                ObjectRef::from_borrowed_handle(
                    runtime.clone(),
                    runtime.regexp_realm_data(realm)?.prototype,
                )?
            }
            NativeConversion::Throw(value) => {
                return Ok(RegExpConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        let object = runtime.new_uninitialized_regexp(&prototype)?;
        let RegExpConstructorPhase::Prototype(publication) = self.0.phase else {
            return Err(RuntimeError::Invariant(
                "RegExp constructor received an unexpected prototype reply",
            ));
        };
        match publication {
            RegExpPublication::Copy(genuine) => {
                runtime.publish_regexp(&object, genuine.pattern, genuine.program)?;
                Ok(RegExpConstructorStep::Complete(Completion::Return(
                    Value::Object(object),
                )))
            }
            RegExpPublication::Compile { pattern, flags } => {
                if matches!(flags, Value::Undefined) {
                    Self::publish(runtime, object, pattern, JsString::from_static(""))
                } else {
                    Ok(RegExpConstructorStep::Primitive {
                        value: flags,
                        resume: {
                            let updated_0 = RegExpConstructorPhase::Flags { object, pattern };
                            self.0.phase = updated_0;
                            self
                        },
                    })
                }
            }
        }
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<RegExpConstructorStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(RegExpConstructorStep::Complete(Completion::Throw(value)));
            }
        };
        match self.0.phase {
            RegExpConstructorPhase::Match => {
                let Value::Object(object) = &self.0.pattern else {
                    return Err(RuntimeError::Invariant(
                        "RegExp match check lost its object",
                    ));
                };
                let is_regexp = runtime.is_regexp_from_match(object, &value)?;
                self.checked(runtime, is_regexp)
            }
            RegExpConstructorPhase::Identity(active) => {
                if value.same_value(&Value::Object(active)) {
                    Ok(RegExpConstructorStep::Complete(Completion::Return(
                        self.0.pattern,
                    )))
                } else {
                    {
                        let updated_0 = RegExpConstructorPhase::Match;
                        self.0.phase = updated_0;
                        self
                    }
                    .prepare(runtime)
                }
            }
            RegExpConstructorPhase::Source => {
                if matches!(self.0.flags, Value::Undefined) {
                    let Value::Object(object) = &self.0.pattern else {
                        return Err(RuntimeError::Invariant(
                            "RegExp source lookup lost its object",
                        ));
                    };
                    Ok(RegExpConstructorStep::Read {
                        object: object.clone(),
                        key: runtime.intern_property_key("flags")?,
                        resume: {
                            let updated_0 = RegExpConstructorPhase::SourceFlags(value);
                            self.0.phase = updated_0;
                            self
                        },
                    })
                } else {
                    let flags = self.0.flags.clone();
                    self.pattern_value(value, flags)
                }
            }
            RegExpConstructorPhase::SourceFlags(pattern) => {
                let updated_0 = RegExpConstructorPhase::Match;
                self.0.phase = updated_0;
                self
            }
            .pattern_value(pattern, value),
            RegExpConstructorPhase::Pattern(flags) => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp pattern conversion returned an object",
                    ));
                }
                let pattern = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpConstructorStep::Complete(Completion::Throw(value)));
                    }
                };
                Ok({
                    let updated_0 = RegExpConstructorPhase::Match;
                    self.0.phase = updated_0;
                    self
                }
                .lookup(RegExpPublication::Compile { pattern, flags }))
            }
            RegExpConstructorPhase::Flags { object, pattern } => {
                if matches!(value, Value::Object(_)) {
                    return Err(RuntimeError::Invariant(
                        "RegExp flags conversion returned an object",
                    ));
                }
                let flags = match runtime.native_to_js_string(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(RegExpConstructorStep::Complete(Completion::Throw(value)));
                    }
                };
                Self::publish(runtime, object, pattern, flags)
            }
            RegExpConstructorPhase::Prototype(_) => Err(RuntimeError::Invariant(
                "RegExp prototype request received an untyped reply",
            )),
        }
    }
}
fn finish_constructor(
    runtime: &Runtime,
    realm: ContextId,
    mut step: RegExpConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            RegExpConstructorStep::Complete(result) => return Ok(result),
            RegExpConstructorStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
            RegExpConstructorStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::String)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
            RegExpConstructorStep::Prototype { new_target, resume } => resume.prototype(
                runtime,
                finish_source(
                    runtime,
                    realm,
                    ProtoSourceStep::start(runtime, realm, new_target)?,
                )?,
            )?,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unpublished_regexp_is_rooted_during_flags_conversion_and_reclaimed_on_abandonment() {
        let runtime = Runtime::new();
        let weak = Rc::downgrade(&runtime.0);
        let mut context = runtime.new_context();
        let new_target = context.eval("(function(){})").unwrap();
        let flags = runtime.new_object(None).unwrap();
        let flags_id = flags.object_id();
        let invocation = NativeInvocation::Construct { new_target };
        let arguments = NativeArguments {
            actual_arg_count: 2,
            readable: vec![
                Value::String(JsString::from_static("a")),
                Value::Object(flags),
            ],
        };
        let RegExpConstructorStep::Primitive { resume, .. } =
            RegExpConstructorStep::start(&runtime, context.realm, &invocation, &arguments).unwrap()
        else {
            panic!("expected pattern conversion")
        };
        drop(invocation);
        drop(arguments);
        let RegExpConstructorStep::Prototype { resume, .. } = resume
            .resume(
                &runtime,
                Completion::Return(Value::String(JsString::from_static("a"))),
            )
            .unwrap()
        else {
            panic!("expected prototype request")
        };
        let prototype = runtime.new_object(None).unwrap();
        let prototype_id = prototype.object_id();
        let RegExpConstructorStep::Primitive { resume, .. } = resume
            .prototype(
                &runtime,
                NativeConversion::Value(ConstructorPrototypeSource::Explicit(prototype)),
            )
            .unwrap()
        else {
            panic!("expected flags conversion")
        };
        let RegExpConstructorPhase::Flags { object, .. } = &resume.phase else {
            panic!("expected unpublished result")
        };
        let object_id = object.object_id();
        runtime.run_gc().unwrap();
        for id in [object_id, prototype_id, flags_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_ok());
        }
        assert!(matches!(
            runtime
                .0
                .state
                .borrow()
                .heap
                .object(object_id)
                .unwrap()
                .payload,
            ObjectPayload::RegExp(RegExpObjectData::Uninitialized)
        ));
        drop(resume);
        runtime.run_gc().unwrap();
        for id in [object_id, prototype_id, flags_id] {
            assert!(runtime.0.state.borrow().heap.object(id).is_err());
        }
        drop(context);
        drop(runtime);
        assert!(weak.upgrade().is_none());
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<RegExpConstructorStep>() <= 64);
