//! Computed literal definitions convert the retained key once before own definition.
use crate::engine::{
    api::{Error, ErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{
        ObjectRef, OrdinaryPropertyDescriptor, PropertyKey,
        operations::{InternalDefineResult, PropertyDefineOutcome},
    },
    value::{Value, conversion::NativeConversion},
    vm::{Completion, ToPrimitiveHint},
};

pub(crate) enum LiteralDefinitionStep {
    Complete(Completion),
    Primitive { resume: LiteralDefinitionResume },
    Define { resume: LiteralDefinitionResume },
}
pub(crate) struct LiteralDefinitionResume(Box<LiteralDefinitionState>);
struct LiteralDefinitionState {
    realm: Option<ContextId>,
    object: Option<ObjectRef>,
    value: Option<Value>,
    primitive: Option<Value>,
    key: Option<PropertyKey>,
    descriptor: Option<OrdinaryPropertyDescriptor>,
}
const _: () = assert!(std::mem::size_of::<LiteralDefinitionStep>() <= 64);
const _: () = assert!(std::mem::size_of::<LiteralDefinitionResume>() <= 8);
impl LiteralDefinitionStep {
    pub(crate) fn define(
        object: ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
    ) -> Self {
        Self::Define {
            resume: LiteralDefinitionResume(Box::new(LiteralDefinitionState {
                realm: None,
                object: Some(object),
                value: None,
                primitive: None,
                key: Some(key),
                descriptor: Some(descriptor),
            })),
        }
    }
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        key: Value,
        value: Value,
    ) -> Result<Self, RuntimeError> {
        if matches!(key, Value::Object(_)) {
            Ok(Self::Primitive {
                resume: LiteralDefinitionResume(Box::new(LiteralDefinitionState {
                    realm: Some(realm),
                    object: Some(object),
                    value: Some(value),
                    primitive: Some(key),
                    key: None,
                    descriptor: None,
                })),
            })
        } else {
            match runtime.property_key_from_primitive(realm, key)? {
                NativeConversion::Value(key) => Ok(Self::define(
                    object,
                    key,
                    Runtime::public_class_field_descriptor(value),
                )),
                NativeConversion::Throw(value) => Ok(Self::Complete(Completion::Throw(value))),
            }
        }
    }
}
impl LiteralDefinitionResume {
    pub(crate) fn take_primitive(&mut self) -> Value {
        self.0.primitive.take().expect("literal primitive request")
    }
    pub(crate) fn take_define(&mut self) -> (ObjectRef, PropertyKey, OrdinaryPropertyDescriptor) {
        (
            self.0.object.take().expect("literal object"),
            self.0.key.take().expect("literal key"),
            self.0.descriptor.take().expect("literal descriptor"),
        )
    }
    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<LiteralDefinitionStep, RuntimeError> {
        let key = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(LiteralDefinitionStep::Complete(Completion::Throw(value)));
            }
        };
        let realm = self.0.realm.take().ok_or(RuntimeError::Invariant(
            "literal definition lost its key conversion owner",
        ))?;
        let key = match runtime.property_key_from_primitive(realm, key)? {
            NativeConversion::Value(key) => key,
            NativeConversion::Throw(value) => {
                return Ok(LiteralDefinitionStep::Complete(Completion::Throw(value)));
            }
        };
        self.0.key = Some(key);
        self.0.descriptor = Some(Runtime::public_class_field_descriptor(
            self.0.value.take().expect("literal value"),
        ));
        Ok(LiteralDefinitionStep::Define { resume: self })
    }
    pub(crate) fn defined(
        self,
        result: NativeConversion<InternalDefineResult>,
    ) -> Result<LiteralDefinitionStep, RuntimeError> {
        if self.0.realm.is_some() {
            return Err(RuntimeError::Invariant(
                "literal definition reply has wrong owner",
            ));
        }
        Ok(LiteralDefinitionStep::Complete(match result {
            NativeConversion::Value(InternalDefineResult::Defined) => {
                Completion::Return(Value::Undefined)
            }
            NativeConversion::Throw(value) => Completion::Throw(value),
            NativeConversion::Value(InternalDefineResult::RejectedOrdinary(_)) => {
                return Err(Error::new(ErrorKind::Type, "property is not configurable").into());
            }
            NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => {
                return Err(RuntimeError::Invariant(
                    "literal own definition unexpectedly entered Proxy trap",
                ));
            }
        }))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: LiteralDefinitionStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            LiteralDefinitionStep::Complete(result) => return Ok(result),
            LiteralDefinitionStep::Primitive { mut resume } => {
                let value = resume.take_primitive();
                resume.resume(
                    runtime,
                    runtime.to_primitive(realm, value, ToPrimitiveHint::String)?,
                )?
            }
            LiteralDefinitionStep::Define { mut resume } => {
                let (object, key, descriptor) = resume.take_define();
                let result = match runtime.define_own_property_in_realm(
                    Some(realm),
                    &object,
                    &key,
                    &descriptor,
                )? {
                    PropertyDefineOutcome::Defined(true) => {
                        NativeConversion::Value(InternalDefineResult::Defined)
                    }
                    PropertyDefineOutcome::Defined(false) => {
                        NativeConversion::Value(InternalDefineResult::RejectedOrdinary(object))
                    }
                    PropertyDefineOutcome::Throw(value) => NativeConversion::Throw(value),
                };
                resume.defined(result)?
            }
        };
    }
}

#[cfg(test)]
mod resident_tests {
    use super::*;
    #[test]
    fn literal_conversion_and_definition_reuse_one_owner() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(object) = context.eval("({})").unwrap() else {
            panic!("object")
        };
        let key = context.eval("({toString(){return 'x'}})").unwrap();
        let primitive = context.eval("'x'").unwrap();
        let LiteralDefinitionStep::Primitive { mut resume } = LiteralDefinitionStep::start(
            &runtime,
            context.realm,
            object.clone(),
            key.clone(),
            Value::Int(42),
        )
        .unwrap() else {
            panic!("primitive request")
        };
        let address = (&*resume.0) as *const LiteralDefinitionState;
        assert_eq!(resume.take_primitive(), key);
        let LiteralDefinitionStep::Define { mut resume } = resume
            .resume(&runtime, Completion::Return(primitive))
            .unwrap()
        else {
            panic!("define request")
        };
        assert_eq!((&*resume.0) as *const LiteralDefinitionState, address);
        let (target, key, _descriptor) = resume.take_define();
        assert_eq!(target, object);
        assert_eq!(key, runtime.intern_property_key("x").unwrap());
        assert!(matches!(
            resume
                .defined(NativeConversion::Value(InternalDefineResult::Defined))
                .unwrap(),
            LiteralDefinitionStep::Complete(Completion::Return(Value::Undefined))
        ));
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<LiteralDefinitionStep>() <= 64);
