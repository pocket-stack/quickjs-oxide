use crate::engine::api::error::{Error, ErrorKind, NativeErrorKind};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::heap::ContextId;
use crate::engine::object::operations::InternalDefineResult;

use crate::engine::object::{
    DescriptorField, OrdinaryPropertyDescriptor, PropertyKey, WellKnownSymbol,
};
use crate::engine::value::conversion::NativeConversion;
use crate::engine::value::{JsString, Value};
use crate::engine::vm::Completion;
use crate::engine::vm::call::{NativeArguments, NativeInvocation};

impl Runtime {
    pub(crate) fn call_iterator_prototype_iterator(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype iterator did not receive a generic invocation",
            ));
        };
        Ok(Completion::Return(this_value))
    }

    pub(crate) fn call_iterator_prototype_to_string_tag_getter(
        &self,
        invocation: NativeInvocation,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Getter { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag getter received the wrong native invocation",
            ));
        };
        Ok(Completion::Return(Value::String(JsString::from_static(
            "Iterator",
        ))))
    }

    pub(crate) fn call_iterator_prototype_to_string_tag_setter(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish_tag(
            self,
            realm,
            TagSetterStep::start(self, realm, &invocation, arguments)?,
        )
    }
}

pub(crate) enum TagSetterStep {
    Complete(Completion),
    Own { resume: TagSetterResume },
    Define { resume: TagSetterResume },
    Set { resume: TagSetterResume },
}
pub(crate) struct TagSetterResume(Box<TagSetterResumeState>);
impl std::ops::Deref for TagSetterResume {
    type Target = TagSetterResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TagSetterResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TagSetterResume>() <= 8);
pub(crate) struct TagSetterResumeState {
    pending_effect: TagSetterStepPending,
    realm: ContextId,
    receiver: crate::engine::object::ObjectRef,
    key: PropertyKey,
    value: Value,
}
impl TagSetterStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Setter { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator.prototype toStringTag setter received the wrong native invocation",
            ));
        };
        let Value::Object(receiver) = this_value else {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "not an object",
            )));
        };
        let value = arguments
            .readable
            .first()
            .cloned()
            .ok_or(RuntimeError::Invariant(
                "Iterator.prototype toStringTag setter argv was not padded",
            ))?;
        let iterator_prototype = runtime
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .iterator_prototype;
        if receiver.object_id() == iterator_prototype {
            return Err(RuntimeError::Engine(Error::new(
                ErrorKind::Type,
                "Cannot assign to read only property",
            )));
        }
        let key = PropertyKey::from(runtime.well_known_symbol(WellKnownSymbol::ToStringTag));
        Ok({
            let __pending_field_object = receiver.clone();
            let __pending_field_key = key.clone();
            let __pending_field_resume = TagSetterResume(Box::new(TagSetterResumeState {
                pending_effect: TagSetterStepPending::default(),
                realm,
                receiver: receiver.clone(),
                key,
                value,
            }));
            Self::request_own(
                __pending_field_object,
                __pending_field_key,
                __pending_field_resume,
            )
        })
    }
}
impl TagSetterResume {
    pub(crate) fn boolean(
        self,
        reply: NativeConversion<bool>,
    ) -> Result<TagSetterStep, RuntimeError> {
        match reply {
            NativeConversion::Throw(value) => Ok(TagSetterStep::Complete(Completion::Throw(value))),
            NativeConversion::Value(true) => Ok({
                let __pending_field_object = self.0.receiver.clone();
                let __pending_field_key = self.0.key.clone();
                let __pending_field_value = self.0.value.clone();
                let __pending_field_resume = self;
                TagSetterStep::request_set(
                    __pending_field_object,
                    __pending_field_key,
                    __pending_field_value,
                    __pending_field_resume,
                )
            }),
            NativeConversion::Value(false) => Ok({
                let __pending_field_object = self.0.receiver.clone();
                let __pending_field_key = self.0.key.clone();
                let __pending_field_descriptor = OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(self.0.value.clone()),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                };
                let __pending_field_resume = self;
                TagSetterStep::request_define(
                    __pending_field_object,
                    __pending_field_key,
                    __pending_field_descriptor,
                    __pending_field_resume,
                )
            }),
        }
    }
    pub(crate) fn defined(
        self,
        runtime: &Runtime,
        reply: NativeConversion<InternalDefineResult>,
    ) -> Result<TagSetterStep, RuntimeError> {
        Ok(TagSetterStep::Complete(match reply {
            NativeConversion::Value(InternalDefineResult::Defined) => {
                Completion::Return(Value::Undefined)
            }
            NativeConversion::Value(InternalDefineResult::RejectedProxyTrap) => {
                Completion::Throw(runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Type,
                    "proxy: defineProperty exception",
                )?)
            }
            NativeConversion::Value(InternalDefineResult::RejectedOrdinary(target)) => {
                let message = if !runtime.has_own_property(&target, &self.0.key)?
                    && !runtime.is_extensible(&target)?
                {
                    "object is not extensible"
                } else {
                    "property is not configurable"
                };
                Completion::Throw(runtime.new_native_error(
                    self.0.realm,
                    NativeErrorKind::Type,
                    message,
                )?)
            }
            NativeConversion::Throw(value) => Completion::Throw(value),
        }))
    }
    pub(crate) fn set(
        self,
        runtime: &Runtime,
        reply: NativeConversion<crate::engine::object::operations::InternalSetResult>,
    ) -> Result<TagSetterStep, RuntimeError> {
        Ok(TagSetterStep::Complete(
            match runtime.finish_set_property_or_throw(self.0.realm, &self.0.key, reply)? {
                Some(value) => Completion::Throw(value),
                None => Completion::Return(Value::Undefined),
            },
        ))
    }
}
pub(crate) fn finish_tag(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TagSetterStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TagSetterStep::Complete(result) => return Ok(result),
            TagSetterStep::Own { mut resume } => {
                let object = resume.take_own_object();
                let key = resume.take_own_key();
                resume.boolean(runtime.internal_has_own_property(realm, &object, &key)?)?
            }
            TagSetterStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                resume.defined(
                    runtime,
                    runtime.internal_define_own_property(realm, &object, &key, &descriptor)?,
                )?
            }
            TagSetterStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                resume.set(
                    runtime,
                    runtime.internal_set(
                        realm,
                        &object,
                        &key,
                        value,
                        Value::Object(object.clone()),
                    )?,
                )?
            }
        };
    }
}

#[derive(Default)]
struct TagSetterStepPending {
    own_object: Option<crate::engine::object::ObjectRef>,
    own_key: Option<PropertyKey>,
    define_object: Option<crate::engine::object::ObjectRef>,
    define_key: Option<PropertyKey>,
    define_descriptor: Option<OrdinaryPropertyDescriptor>,
    set_object: Option<crate::engine::object::ObjectRef>,
    set_key: Option<PropertyKey>,
    set_value: Option<Value>,
}
impl TagSetterStep {
    pub(crate) fn request_own(
        object: crate::engine::object::ObjectRef,
        key: PropertyKey,
        mut resume: TagSetterResume,
    ) -> Self {
        resume.0.pending_effect.own_object = Some(object);
        resume.0.pending_effect.own_key = Some(key);
        Self::Own { resume }
    }
    pub(crate) fn request_define(
        object: crate::engine::object::ObjectRef,
        key: PropertyKey,
        descriptor: OrdinaryPropertyDescriptor,
        mut resume: TagSetterResume,
    ) -> Self {
        resume.0.pending_effect.define_object = Some(object);
        resume.0.pending_effect.define_key = Some(key);
        resume.0.pending_effect.define_descriptor = Some(descriptor);
        Self::Define { resume }
    }
    pub(crate) fn request_set(
        object: crate::engine::object::ObjectRef,
        key: PropertyKey,
        value: Value,
        mut resume: TagSetterResume,
    ) -> Self {
        resume.0.pending_effect.set_object = Some(object);
        resume.0.pending_effect.set_key = Some(key);
        resume.0.pending_effect.set_value = Some(value);
        Self::Set { resume }
    }
}
impl TagSetterResume {
    pub(crate) fn take_own_object(&mut self) -> crate::engine::object::ObjectRef {
        self.0
            .pending_effect
            .own_object
            .take()
            .expect("TagSetterStep Own object")
    }
    pub(crate) fn take_own_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .own_key
            .take()
            .expect("TagSetterStep Own key")
    }
    pub(crate) fn take_define_object(&mut self) -> crate::engine::object::ObjectRef {
        self.0
            .pending_effect
            .define_object
            .take()
            .expect("TagSetterStep Define object")
    }
    pub(crate) fn take_define_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .define_key
            .take()
            .expect("TagSetterStep Define key")
    }
    pub(crate) fn take_define_descriptor(&mut self) -> OrdinaryPropertyDescriptor {
        self.0
            .pending_effect
            .define_descriptor
            .take()
            .expect("TagSetterStep Define descriptor")
    }
    pub(crate) fn take_set_object(&mut self) -> crate::engine::object::ObjectRef {
        self.0
            .pending_effect
            .set_object
            .take()
            .expect("TagSetterStep Set object")
    }
    pub(crate) fn take_set_key(&mut self) -> PropertyKey {
        self.0
            .pending_effect
            .set_key
            .take()
            .expect("TagSetterStep Set key")
    }
    pub(crate) fn take_set_value(&mut self) -> Value {
        self.0
            .pending_effect
            .set_value
            .take()
            .expect("TagSetterStep Set value")
    }
}
const _: () = assert!(std::mem::size_of::<TagSetterStep>() <= 64);

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TagSetterStep>() <= 64);
