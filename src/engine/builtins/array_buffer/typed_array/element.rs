//! Element conversion owns ToPrimitive; no buffer credential crosses a callback.
use super::{typed_array_encode_bigint, typed_array_encode_number};
use crate::engine::object::CallableRef;
use crate::engine::value::conversion::primitive::{PrimitiveResume, PrimitiveStep};
use crate::engine::vm::ToPrimitiveHint;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::{ObjectRef, PropertyKey},
    value::{Value, conversion::NativeConversion},
    vm::Completion,
};

pub(crate) enum ElementStep {
    Complete(NativeConversion<[u8; 8]>),
    Read { resume: ElementResume },
    Call { resume: ElementResume },
}
pub(crate) struct ElementResume(Box<ElementResumeState>);
impl std::ops::Deref for ElementResume {
    type Target = ElementResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ElementResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ElementResume>() <= 8);
pub(crate) struct ElementResumeState {
    pending_effect: ElementStepPending,
    realm: ContextId,
    element: TypedArrayElementKind,
    primitive: PrimitiveResume,
}
impl ElementStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        element: TypedArrayElementKind,
        value: Value,
    ) -> Result<Self, RuntimeError> {
        from_primitive(
            runtime,
            realm,
            element,
            PrimitiveResume::start(runtime, realm, value, ToPrimitiveHint::Number),
        )
    }
    pub(crate) fn finish_sync(
        mut self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<NativeConversion<[u8; 8]>, RuntimeError> {
        loop {
            self = match self {
                Self::Complete(result) => return Ok(result),
                Self::Read { mut resume } => {
                    let object = resume.take_read_object();
                    let key = resume.take_read_key();
                    resume.resume(
                        runtime,
                        runtime.get_property_in_realm(realm, &object, &key)?,
                    )?
                }
                Self::Call { mut resume } => {
                    let callable = resume.take_call_callable();
                    let receiver = resume.take_call_receiver();
                    let arguments = resume.take_call_arguments();
                    resume.resume(
                        runtime,
                        runtime.call_internal(realm, &callable, receiver, &arguments)?,
                    )?
                }
            };
        }
    }
}
/// Shared primitive conversion after ToPrimitive has completed.
pub(super) fn encode_primitive(
    runtime: &Runtime,
    realm: ContextId,
    element: TypedArrayElementKind,
    value: Value,
) -> Result<NativeConversion<[u8; 8]>, RuntimeError> {
    Ok(if element.is_bigint() {
        match runtime.bigint_from_primitive(realm, value)? {
            NativeConversion::Value(bigint) => {
                NativeConversion::Value(typed_array_encode_bigint(&bigint)?)
            }
            NativeConversion::Throw(value) => NativeConversion::Throw(value),
        }
    } else {
        match runtime.number_from_primitive(realm, &value)? {
            NativeConversion::Value(number) => {
                NativeConversion::Value(typed_array_encode_number(element, number))
            }
            NativeConversion::Throw(value) => NativeConversion::Throw(value),
        }
    })
}
fn from_primitive(
    runtime: &Runtime,
    realm: ContextId,
    element: TypedArrayElementKind,
    step: PrimitiveStep,
) -> Result<ElementStep, RuntimeError> {
    Ok(match step {
        PrimitiveStep::Complete(Completion::Throw(value)) => {
            ElementStep::Complete(NativeConversion::Throw(value))
        }
        PrimitiveStep::Complete(Completion::Return(value)) => {
            let bytes = encode_primitive(runtime, realm, element, value)?;
            ElementStep::Complete(bytes)
        }
        PrimitiveStep::Get { mut resume } => {
            let (object, key) = resume.take_get();
            ElementStep::request_read(
                object,
                key,
                ElementResume(Box::new(ElementResumeState {
                    pending_effect: ElementStepPending::default(),
                    realm,
                    element,
                    primitive: resume,
                })),
            )
        }
        PrimitiveStep::Call { mut resume } => {
            let callable = resume.take_callable();
            let receiver = resume.take_receiver();
            let arguments = resume.take_arguments();
            ElementStep::request_call(
                callable,
                receiver,
                arguments,
                ElementResume(Box::new(ElementResumeState {
                    pending_effect: ElementStepPending::default(),
                    realm,
                    element,
                    primitive: resume,
                })),
            )
        }
    })
}
impl ElementResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        completion: Completion,
    ) -> Result<ElementStep, RuntimeError> {
        from_primitive(
            runtime,
            self.0.realm,
            self.0.element,
            self.0.primitive.resume(runtime, completion)?,
        )
    }
}

#[derive(Default)]
struct ElementStepPending {
    read_object: Option<ObjectRef>,
    read_key: Option<PropertyKey>,
    call_callable: Option<CallableRef>,
    call_receiver: Option<Value>,
    call_arguments: Option<Vec<Value>>,
}
impl ElementStep {
    pub(crate) fn request_read(
        object: ObjectRef,
        key: PropertyKey,
        mut resume: ElementResume,
    ) -> Self {
        resume.0.pending_effect.read_object = Some(object);
        resume.0.pending_effect.read_key = Some(key);
        Self::Read { resume }
    }
    pub(crate) fn request_call(
        callable: CallableRef,
        receiver: Value,
        arguments: Vec<Value>,
        mut resume: ElementResume,
    ) -> Self {
        resume.0.pending_effect.call_callable = Some(callable);
        resume.0.pending_effect.call_receiver = Some(receiver);
        resume.0.pending_effect.call_arguments = Some(arguments);
        Self::Call { resume }
    }
}
impl ElementResume {
    pub(crate) fn take_read_object(&mut self) -> ObjectRef {
        self.0
            .pending_effect
            .read_object
            .take()
            .expect("ElementStep Read object")
    }
    pub(crate) fn take_read_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .read_key
            .take()
            .expect("ElementStep Read key")
    }
    pub(crate) fn take_call_callable(&mut self) -> CallableRef {
        self.0
            .pending_effect
            .call_callable
            .take()
            .expect("ElementStep Call callable")
    }
    pub(crate) fn take_call_receiver(&mut self) -> Value {
        self.0
            .pending_effect
            .call_receiver
            .take()
            .expect("ElementStep Call receiver")
    }
    pub(crate) fn take_call_arguments(&mut self) -> Vec<Value> {
        self.0
            .pending_effect
            .call_arguments
            .take()
            .expect("ElementStep Call arguments")
    }
}
const _: () = assert!(std::mem::size_of::<ElementStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ElementStep>() <= 64);
