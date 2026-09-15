//! Constructor prototype acquisition shares explicit Get and fallback realm replies.
use super::ConstructorPrototypeSource;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::PropertyKey,
    value::{Value, conversion::NativeConversion},
    vm::Completion,
};
pub(crate) enum ProtoSourceStep {
    Complete(NativeConversion<ConstructorPrototypeSource>),
    ReadValue {
        receiver: Value,
        key: PropertyKey,
        resume: ProtoSourceResume,
    },
}
pub(crate) struct ProtoSourceResume(Box<ProtoSourceResumeState>);
impl std::ops::Deref for ProtoSourceResume {
    type Target = ProtoSourceResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for ProtoSourceResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<ProtoSourceResume>() <= 8);
pub(crate) struct ProtoSourceResumeState {
    realm: ContextId,
    new_target: Value,
}
impl ProtoSourceStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        new_target: Value,
    ) -> Result<Self, RuntimeError> {
        if matches!(new_target, Value::Undefined) {
            return Ok(Self::Complete(NativeConversion::Value(
                ConstructorPrototypeSource::Realm(realm),
            )));
        }
        Ok(Self::ReadValue {
            receiver: new_target.clone(),
            key: runtime.intern_property_key("prototype")?,
            resume: ProtoSourceResume(Box::new(ProtoSourceResumeState { realm, new_target })),
        })
    }
}
impl ProtoSourceResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        reply: Completion,
    ) -> Result<ProtoSourceStep, RuntimeError> {
        let result = match reply {
            Completion::Throw(value) => NativeConversion::Throw(value),
            Completion::Return(Value::Object(prototype)) => {
                NativeConversion::Value(ConstructorPrototypeSource::Explicit(prototype))
            }
            Completion::Return(_) => {
                match runtime.function_realm_from_value(self.0.realm, &self.0.new_target)? {
                    NativeConversion::Value(realm) => {
                        NativeConversion::Value(ConstructorPrototypeSource::Realm(realm))
                    }
                    NativeConversion::Throw(value) => NativeConversion::Throw(value),
                }
            }
        };
        Ok(ProtoSourceStep::Complete(result))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ProtoSourceStep,
) -> Result<NativeConversion<ConstructorPrototypeSource>, RuntimeError> {
    loop {
        step = match step {
            ProtoSourceStep::Complete(result) => return Ok(result),
            ProtoSourceStep::ReadValue {
                receiver,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_value_property_in_realm(realm, receiver, &key)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ProtoSourceStep>() <= 64);
