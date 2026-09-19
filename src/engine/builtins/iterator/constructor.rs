//! Iterator's abstract constructor and constructor accessor share owned replies.
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::NativeFunctionId,
    heap::{ContextId, ObjectPayload},
    object::{CallableRef, DescriptorField, ObjectRef, OrdinaryPropertyDescriptor},
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{ConstructorPrototypeSource, NativeArguments, NativeInvocation},
    },
};
pub(crate) enum ConstructorStep {
    Complete(Completion),
    Prototype {
        new_target: Value,
        resume: ConstructorResume,
    },
}
pub(crate) struct ConstructorResume;
impl ConstructorStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
    ) -> Result<Self, RuntimeError> {
        let new_target = match invocation {
            NativeInvocation::Construct {
                new_target: Value::Object(object),
            } => object,
            NativeInvocation::Construct { .. } | NativeInvocation::Call { .. } => {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error_jsvalue(
                        realm,
                        NativeErrorKind::Type,
                        "constructor requires 'new'",
                    )?,
                )));
            }
            NativeInvocation::Getter { .. } | NativeInvocation::Setter { .. } => {
                return Err(RuntimeError::Invariant(
                    "Iterator constructor received an accessor invocation",
                ));
            }
        };
        let native_iterator = {
            let state = runtime.0.state.borrow();
            matches!(&state.heap.object(new_target.object_id())?.payload, ObjectPayload::NativeFunction { data, .. } if data.target == NativeFunctionId::IteratorConstructor)
        };
        if native_iterator {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(
                    realm,
                    NativeErrorKind::Type,
                    "abstract class not constructable",
                )?,
            )));
        }
        Ok(Self::Prototype {
            new_target: Value::Object(new_target.clone()),
            resume: ConstructorResume,
        })
    }
    pub(crate) fn accessor(
        runtime: &Runtime,
        realm: ContextId,
        callable: &CallableRef,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Iterator constructor accessor did not receive a generic invocation",
            ));
        };
        let defining_realm = {
            let state = runtime.0.state.borrow();
            let ObjectPayload::NativeFunction { data, .. } =
                &state.heap.object(callable.as_object().object_id())?.payload
            else {
                return Err(RuntimeError::Invariant(
                    "Iterator constructor accessor callable lost its native payload",
                ));
            };
            if data.target != NativeFunctionId::IteratorConstructorAccessor {
                return Err(RuntimeError::Invariant(
                    "Iterator constructor accessor callable changed target",
                ));
            }
            data.realm.ok_or(RuntimeError::Invariant(
                "Iterator constructor accessor lost its defining realm",
            ))?
        };
        if arguments.actual_arg_count == 0 {
            let constructor = runtime.iterator_realm_data(defining_realm)?.constructor;
            return Ok(Self::Complete(Completion::Return(Value::Object(
                ObjectRef::from_borrowed_handle(runtime.clone(), constructor)?,
            ))));
        }
        let Some(Value::Object(value)) = arguments.readable.first() else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not an object")?,
            )));
        };
        let Value::Object(receiver) = this_value else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not an object")?,
            )));
        };
        let key =
            runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Constructor)?;
        let descriptor = OrdinaryPropertyDescriptor {
            value: DescriptorField::Present(Value::Object(value.clone())),
            writable: DescriptorField::Present(true),
            enumerable: DescriptorField::Present(false),
            configurable: DescriptorField::Present(true),
            ..OrdinaryPropertyDescriptor::new()
        };
        let completion = if runtime.define_own_property(receiver, &key, &descriptor)? {
            Completion::Return(Value::Undefined)
        } else {
            Completion::Throw(runtime.new_native_error_jsvalue(
                realm,
                NativeErrorKind::Type,
                "cannot define property",
            )?)
        };
        Ok(Self::Complete(completion))
    }
}
impl ConstructorResume {
    pub(crate) fn prototype(
        self,
        runtime: &Runtime,
        reply: NativeConversion<ConstructorPrototypeSource>,
    ) -> Result<ConstructorStep, RuntimeError> {
        let prototype = match reply {
            NativeConversion::Throw(value) => {
                return Ok(ConstructorStep::Complete(Completion::Throw(value)));
            }
            NativeConversion::Value(ConstructorPrototypeSource::Explicit(prototype)) => prototype,
            NativeConversion::Value(ConstructorPrototypeSource::Realm(realm)) => {
                let prototype = runtime
                    .0
                    .state
                    .borrow()
                    .heap
                    .context(realm)?
                    .iterator_prototype;
                ObjectRef::from_borrowed_handle(runtime.clone(), prototype)?
            }
        };
        Ok(ConstructorStep::Complete(Completion::Return(
            Value::Object(runtime.new_iterator_object(&prototype)?),
        )))
    }
}
pub(crate) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: ConstructorStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            ConstructorStep::Complete(result) => return Ok(result),
            ConstructorStep::Prototype { new_target, resume } => resume.prototype(
                runtime,
                runtime.constructor_prototype_source(realm, &new_target)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<ConstructorStep>() <= 64);
