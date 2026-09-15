//! Object construction owns custom new.target prototype selection before allocation.
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::ObjectRef,
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{
            ConstructorPrototypeSource, NativeArguments, NativeInvocation,
            prototype::{ProtoSourceStep, finish as finish_source},
        },
    },
};
pub(crate) enum ObjectConstructorStep {
    Complete(Completion),
    Prototype {
        new_target: Value,
        resume: ObjectConstructorResume,
    },
}
pub(crate) struct ObjectConstructorResume;
impl ObjectConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Construct { new_target } = invocation else {
            return Err(RuntimeError::Invariant(
                "Object constructor did not receive constructor-or-function invocation",
            ));
        };
        let active = runtime.active_function()?;
        let is_active = matches!(new_target, Value::Object(object) if object == &active);
        if !matches!(new_target, Value::Undefined) && !is_active {
            return Ok(Self::Prototype {
                new_target: new_target.clone(),
                resume: ObjectConstructorResume,
            });
        }
        let argument = arguments.readable.first().ok_or(RuntimeError::Invariant(
            "Object constructor argv was not padded",
        ))?;
        if matches!(argument, Value::Null | Value::Undefined) {
            return ObjectConstructorResume.prototype(
                runtime,
                NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)),
            );
        }
        Ok(Self::Complete(
            match runtime.native_to_object(realm, argument.clone())? {
                NativeConversion::Value(object) => Completion::Return(Value::Object(object)),
                NativeConversion::Throw(value) => Completion::Throw(value),
            },
        ))
    }
}
impl ObjectConstructorResume {
    pub(crate) fn prototype(
        self,
        runtime: &Runtime,
        result: NativeConversion<ConstructorPrototypeSource>,
    ) -> Result<ObjectConstructorStep, RuntimeError> {
        let prototype = match result {
            NativeConversion::Throw(value) => {
                return Ok(ObjectConstructorStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(prototype)) => prototype,
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                let prototype = runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .context(realm)?
                    .object_prototype;
                ObjectRef::from_borrowed_handle(runtime.clone(), prototype)?
            }
        };
        Ok(ObjectConstructorStep::Complete(Completion::Return(
            Value::Object(runtime.new_object(Some(&prototype))?),
        )))
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    step: ObjectConstructorStep,
) -> Result<Completion, RuntimeError> {
    match step {
        ObjectConstructorStep::Complete(result) => Ok(result),
        ObjectConstructorStep::Prototype { new_target, resume } => finish(
            runtime,
            realm,
            resume.prototype(
                runtime,
                finish_source(
                    runtime,
                    realm,
                    ProtoSourceStep::start(runtime, realm, new_target)?,
                )?,
            )?,
        ),
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ObjectConstructorStep>() <= 64);
