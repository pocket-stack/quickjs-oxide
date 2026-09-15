//! Capability creation precedes callback validation, including Promise.try.
use super::{Phase, PromiseResume, PromiseStep, capability};
use crate::engine::api::{runtime::Runtime, runtime_error::RuntimeError};
use crate::engine::builtins::{native::PromiseNativeKind, promise::RootedPromiseCapability};
use crate::engine::heap::ContextId;
use crate::engine::object::{DescriptorField, OrdinaryPropertyDescriptor};
use crate::engine::value::{Value, conversion::NativeConversion};
use crate::engine::vm::{
    Completion,
    call::{NativeArguments, NativeInvocation},
};

impl PromiseStep {
    pub(in crate::engine::builtins::promise) fn convenience(
        runtime: &Runtime,
        realm: ContextId,
        kind: PromiseNativeKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Promise convenience received constructor invocation",
            ));
        };
        let Value::Object(object) = this_value else {
            return capability::error(runtime, realm, "not an object");
        };
        let constructor = match runtime
            .constructor_from_value(realm, Value::Object(object.clone()))?
        {
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
            NativeConversion::Value(constructor) => constructor,
        };
        Box::new(PromiseResume {
            pending_effect: super::PromiseStepPending::default(),
            realm,
            phase: Phase::ConvenienceCapability {
                kind,
                arguments: NativeArguments {
                    readable: arguments.readable.clone(),
                    actual_arg_count: arguments.actual_arg_count,
                },
            },
        })
        .capability(runtime, Some(constructor))
    }
}

pub(super) fn ready(
    runtime: &Runtime,
    realm: ContextId,
    kind: PromiseNativeKind,
    arguments: NativeArguments,
    capability: RootedPromiseCapability,
) -> Result<PromiseStep, RuntimeError> {
    if kind == PromiseNativeKind::WithResolvers {
        let RootedPromiseCapability {
            promise,
            resolve,
            reject,
        } = capability;
        let result = runtime.new_ordinary_object_in_realm(realm)?;
        for (name, value) in [
            ("promise", Value::Object(promise)),
            ("resolve", Value::Object(resolve.as_object().clone())),
            ("reject", Value::Object(reject.as_object().clone())),
        ] {
            let key = runtime.intern_property_key(name)?;
            if !runtime.define_own_property(
                &result,
                &key,
                &OrdinaryPropertyDescriptor {
                    value: DescriptorField::Present(value),
                    writable: DescriptorField::Present(true),
                    enumerable: DescriptorField::Present(true),
                    configurable: DescriptorField::Present(true),
                    ..OrdinaryPropertyDescriptor::new()
                },
            )? {
                return Err(RuntimeError::Invariant(
                    "fresh Promise.withResolvers result rejected a data property",
                ));
            }
        }
        return Ok(PromiseStep::Complete(Completion::Return(Value::Object(
            result,
        ))));
    }
    let callback = arguments
        .readable
        .first()
        .cloned()
        .ok_or(RuntimeError::Invariant(
            "Promise.try callback argv was not padded",
        ))?;
    match runtime.promise_callable(realm, callback)? {
        NativeConversion::Throw(reason) => settle(realm, capability, Completion::Throw(reason)),
        NativeConversion::Value(callable) => Ok({
            let __pending_field_callable = callable;
            let __pending_field_receiver = Value::Undefined;
            let __pending_field_arguments =
                arguments.readable[1..arguments.actual_arg_count.max(1)].to_vec();
            let __pending_field_resume = Box::new(PromiseResume {
                pending_effect: super::PromiseStepPending::default(),
                realm,
                phase: Phase::TryCallback(capability),
            });
            PromiseStep::request_call(
                __pending_field_callable,
                __pending_field_receiver,
                __pending_field_arguments,
                __pending_field_resume,
            )
        }),
    }
}

pub(super) fn settle(
    realm: ContextId,
    capability: RootedPromiseCapability,
    completion: Completion,
) -> Result<PromiseStep, RuntimeError> {
    let (callable, value) = match completion {
        Completion::Return(value) => (capability.resolve, value),
        Completion::Throw(value) => (capability.reject, value),
    };
    Ok({
        let __pending_field_callable = callable;
        let __pending_field_receiver = Value::Undefined;
        let __pending_field_arguments = vec![value];
        let __pending_field_resume = Box::new(PromiseResume {
            pending_effect: super::PromiseStepPending::default(),
            realm,
            phase: Phase::ReturnPromise(capability.promise),
        });
        PromiseStep::request_call(
            __pending_field_callable,
            __pending_field_receiver,
            __pending_field_arguments,
            __pending_field_resume,
        )
    })
}
