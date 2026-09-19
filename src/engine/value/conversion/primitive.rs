//! Owned ToPrimitive phases. A reply consumes its continuation exactly once.
use super::*;
use crate::engine::object::{CallableRef, ObjectRef};
use crate::engine::value::JsValue;

pub(crate) enum PrimitiveStep {
    Get { resume: PrimitiveResume },
    Call { resume: PrimitiveResume },
    Complete(Completion),
}
const _: () = assert!(std::mem::size_of::<PrimitiveStep>() <= 64);

pub(crate) struct PrimitiveResume(Box<PrimitiveResumeState>);
impl std::ops::Deref for PrimitiveResume {
    type Target = PrimitiveResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for PrimitiveResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<PrimitiveResume>() <= 8);
pub(crate) struct PrimitiveResumeState {
    object: ObjectRef,
    realm: ContextId,
    hint: ToPrimitiveHint,
    phase: Phase,
    requested_object: Option<ObjectRef>,
    requested_key: Option<PropertyKey>,
    requested_callable: Option<CallableRef>,
    requested_receiver: Option<Value>,
    requested_arguments: Vec<Value>,
}

enum Phase {
    ExoticMethod,
    ExoticResult,
    OrdinaryMethod(bool),
    OrdinaryResult(bool),
}

impl PrimitiveResume {
    fn get(mut self, object: ObjectRef, key: PropertyKey) -> PrimitiveStep {
        self.requested_object = Some(object);
        self.requested_key = Some(key);
        PrimitiveStep::Get { resume: self }
    }
    fn call(
        mut self,
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
    ) -> PrimitiveStep {
        self.requested_callable = Some(callable);
        self.requested_receiver = Some(receiver);
        self.requested_arguments = arguments;
        PrimitiveStep::Call { resume: self }
    }
    pub(crate) fn take_get(&mut self) -> (ObjectRef, PropertyKey) {
        (
            self.requested_object.take().expect("primitive get object"),
            self.requested_key.take().expect("primitive get key"),
        )
    }
    pub(crate) fn take_callable(&mut self) -> CallableRef {
        self.requested_callable
            .take()
            .expect("primitive call callee")
    }
    pub(crate) fn take_receiver(&mut self) -> Value {
        self.requested_receiver
            .take()
            .expect("primitive call receiver")
    }
    pub(crate) fn take_arguments(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.requested_arguments)
    }

    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        value: JsValue,
        hint: ToPrimitiveHint,
    ) -> PrimitiveStep {
        let JsValue::Object(object) = value else {
            return PrimitiveStep::Complete(Completion::Return(value));
        };
        let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToPrimitive));
        let requested = object.clone();
        Self(Box::new(PrimitiveResumeState {
            object,
            realm,
            hint,
            phase: Phase::ExoticMethod,
            requested_object: None,
            requested_key: None,
            requested_callable: None,
            requested_receiver: None,
            requested_arguments: Vec::new(),
        }))
        .get(requested, key)
    }

    pub(crate) fn ordinary(
        runtime: &Runtime,
        realm: ContextId,
        object: ObjectRef,
        hint: ToPrimitiveHint,
    ) -> Result<PrimitiveStep, RuntimeError> {
        Self(Box::new(PrimitiveResumeState {
            object,
            realm,
            hint,
            phase: Phase::OrdinaryMethod(false),
            requested_object: None,
            requested_key: None,
            requested_callable: None,
            requested_receiver: None,
            requested_arguments: Vec::new(),
        }))
        .read_ordinary(runtime, false)
    }

    fn read_ordinary(
        mut self,
        runtime: &Runtime,
        second: bool,
    ) -> Result<PrimitiveStep, RuntimeError> {
        let string_first = matches!(self.0.hint, ToPrimitiveHint::String);
        let name = if string_first != second {
            crate::engine::atom::pinned::PinnedAtom::ToString
        } else {
            crate::engine::atom::pinned::PinnedAtom::ValueOf
        };
        let key = runtime.pinned_property_key(name)?;
        self.0.phase = Phase::OrdinaryMethod(second);
        let object = self.0.object.clone();
        Ok(self.get(object, key))
    }

    fn failed_method(self, runtime: &Runtime, second: bool) -> Result<PrimitiveStep, RuntimeError> {
        if second {
            self.type_error(runtime, "toPrimitive")
        } else {
            self.read_ordinary(runtime, true)
        }
    }

    fn type_error(self, runtime: &Runtime, message: &str) -> Result<PrimitiveStep, RuntimeError> {
        Ok(PrimitiveStep::Complete(Completion::Throw(
            runtime.new_native_error_jsvalue(self.0.realm, NativeErrorKind::Type, message)?,
        )))
    }

    pub(crate) fn resume(
        mut self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<PrimitiveStep, RuntimeError> {
        let value = match completion {
            Completion::Throw(value) => {
                return Ok(PrimitiveStep::Complete(Completion::Throw(value)));
            }
            Completion::Return(value) => value,
        };
        match self.0.phase {
            Phase::ExoticMethod => {
                if matches!(value, JsValue::Undefined | JsValue::Null) {
                    return self.read_ordinary(runtime, false);
                }
                let JsValue::Object(method) = value else {
                    return self.type_error(runtime, "not a function");
                };
                let method =
                    ObjectRef::from_borrowed_handle(runtime.clone(), method)?;
                let Some(callable) = runtime.as_callable(&method)? else {
                    return self.type_error(runtime, "not a function");
                };
                let argument = Value::String(JsString::from_static(match self.0.hint {
                    ToPrimitiveHint::String => "string",
                    ToPrimitiveHint::Number => "number",
                    ToPrimitiveHint::Default => "default",
                }));
                self.0.phase = Phase::ExoticResult;
                let receiver = Value::Object(self.0.object.clone());
                Ok(self.call(callable, receiver, vec![argument]))
            }
            Phase::ExoticResult => {
                if matches!(value, JsValue::Object(_)) {
                    self.type_error(runtime, "toPrimitive")
                } else {
                    Ok(PrimitiveStep::Complete(Completion::Return(value)))
                }
            }
            Phase::OrdinaryMethod(second) => {
                let JsValue::Object(method) = value else {
                    return self.failed_method(runtime, second);
                };
                let method =
                    ObjectRef::from_borrowed_handle(runtime.clone(), method)?;
                let Some(callable) = runtime.as_callable(&method)? else {
                    return self.failed_method(runtime, second);
                };
                self.0.phase = Phase::OrdinaryResult(second);
                let receiver = Value::Object(self.0.object.clone());
                Ok(self.call(callable, receiver, Vec::new()))
            }
            Phase::OrdinaryResult(second) => {
                if matches!(value, JsValue::Object(_)) {
                    self.failed_method(runtime, second)
                } else {
                    Ok(PrimitiveStep::Complete(Completion::Return(value)))
                }
            }
        }
    }
}

impl Runtime {
    /// Transitional synchronous consumer. The domain steps themselves never
    /// invoke JavaScript; the explicit driver can own the same continuations.
    pub(super) fn finish_primitive_steps(
        &self,
        realm: ContextId,
        mut step: PrimitiveStep,
    ) -> Result<Completion, RuntimeError> {
        loop {
            step = match step {
                PrimitiveStep::Complete(completion) => return Ok(completion),
                PrimitiveStep::Get { mut resume } => {
                    let (object, key) = resume.take_get();
                    let completion = self.get_property_in_realm(realm, &object, &key)?;
                    resume.resume(self, completion)?
                }
                PrimitiveStep::Call { mut resume } => {
                    let callable = resume.take_callable();
                    let receiver = resume.take_receiver();
                    let arguments = resume.take_arguments();
                    let completion = self.call_internal(realm, &callable, receiver, &arguments)?;
                    resume.resume(self, completion)?
                }
            };
        }
    }

    pub(crate) fn ordinary_to_primitive(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        hint: ToPrimitiveHint,
    ) -> Result<Completion, RuntimeError> {
        let step = PrimitiveResume::ordinary(self, realm, object.clone(), hint)?;
        self.finish_primitive_steps(realm, step)
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<PrimitiveStep>() <= 64);

#[cfg(test)]
mod resident_request_tests {
    use super::*;
    #[test]
    fn property_and_call_requests_reuse_the_primitive_resume_allocation() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let value = context.eval("({valueOf(){return 7}})").unwrap();
        let PrimitiveStep::Get { mut resume } =
            PrimitiveResume::start(&runtime, context.realm, value, ToPrimitiveHint::Number)
        else {
            panic!("first get")
        };
        let address = &*resume.0 as *const PrimitiveResumeState;
        let (object, key) = resume.take_get();
        let completion = runtime
            .get_property_in_realm(context.realm, &object, &key)
            .unwrap();
        let PrimitiveStep::Get { mut resume } = resume.resume(&runtime, completion).unwrap() else {
            panic!("ordinary get")
        };
        assert_eq!(&*resume.0 as *const PrimitiveResumeState, address);
        let (object, key) = resume.take_get();
        let completion = runtime
            .get_property_in_realm(context.realm, &object, &key)
            .unwrap();
        let PrimitiveStep::Call { mut resume } = resume.resume(&runtime, completion).unwrap()
        else {
            panic!("ordinary call")
        };
        assert_eq!(&*resume.0 as *const PrimitiveResumeState, address);
        let callable = resume.take_callable();
        let receiver = resume.take_receiver();
        let arguments = resume.take_arguments();
        let completion = runtime
            .call_internal(context.realm, &callable, receiver, &arguments)
            .unwrap();
        assert!(matches!(
            resume.resume(&runtime, completion).unwrap(),
            PrimitiveStep::Complete(Completion::Return(Value::Int(7)))
        ));
    }
}
