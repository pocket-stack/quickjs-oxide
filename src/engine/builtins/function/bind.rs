//! Function.bind publishes length before reading name and roots the unpublished function.
use super::bound_function_length;
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    heap::ContextId,
    object::{CallableRef, ObjectRef, PropertyKey},
    value::{JsString, Value, conversion::NativeConversion},
    vm::{
        Completion,
        call::{NativeArguments, NativeInvocation},
    },
};
pub(crate) enum BindStep {
    Complete(Completion),
    Own {
        object: ObjectRef,
        key: PropertyKey,
        resume: BindResume,
    },
    Read {
        object: ObjectRef,
        key: PropertyKey,
        resume: BindResume,
    },
}
pub(crate) struct BindResume(Box<BindResumeState>);
impl std::ops::Deref for BindResume {
    type Target = BindResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for BindResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<BindResume>() <= 8);
pub(crate) struct BindResumeState {
    target: ObjectRef,
    bound: CallableRef,
    count: usize,
    name: bool,
}
impl BindStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "Function.prototype.bind did not receive a generic invocation",
            ));
        };
        let target = match this_value {
            Value::Object(object) => runtime.as_callable(object)?,
            _ => None,
        };
        let Some(target) = target else {
            return Ok(Self::Complete(Completion::Throw(
                runtime.new_native_error_jsvalue(realm, NativeErrorKind::Type, "not a function")?,
            )));
        };
        let count = arguments.actual_arg_count.saturating_sub(1);
        let forwarded = if arguments.actual_arg_count > 1 {
            &arguments.readable[1..arguments.actual_arg_count]
        } else {
            &[]
        };
        let bound =
            runtime.new_bound_function(realm, &target, &arguments.readable[0], forwarded)?;
        Ok(Self::Own {
            object: target.as_object().clone(),
            key: runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
            resume: BindResume(Box::new(BindResumeState {
                target: target.into_object(),
                bound,
                count,
                name: false,
            })),
        })
    }
}
impl BindResume {
    pub(crate) fn boolean(
        self,
        runtime: &Runtime,
        result: NativeConversion<bool>,
    ) -> Result<BindStep, RuntimeError> {
        match result {
            NativeConversion::Throw(value) => Ok(BindStep::Complete(Completion::Throw(value))),
            NativeConversion::Value(true) => Ok(BindStep::Read {
                object: self.0.target.clone(),
                key: runtime
                    .pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Length)?,
                resume: self,
            }),
            NativeConversion::Value(false) => self.length(runtime, Value::Int(0)),
        }
    }
    fn length(mut self, runtime: &Runtime, value: Value) -> Result<BindStep, RuntimeError> {
        runtime.define_function_data_property(
            self.0.bound.as_object(),
            "length",
            value,
            false,
            true,
        )?;
        self.0.name = true;
        Ok(BindStep::Read {
            object: self.0.target.clone(),
            key: runtime.pinned_property_key(crate::engine::atom::pinned::PinnedAtom::Name)?,
            resume: self,
        })
    }
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<BindStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            result @ Completion::Throw(_) => return Ok(BindStep::Complete(result)),
        };
        if !self.0.name {
            let length = bound_function_length(&value, self.0.count)?;
            return self.length(runtime, length);
        }
        let name = match value {
            Value::String(name) => name,
            _ => JsString::from_static(""),
        };
        let name = JsString::from_static("bound ").try_concat(&name)?;
        runtime.define_function_data_property(
            self.0.bound.as_object(),
            "name",
            Value::String(name),
            false,
            true,
        )?;
        Ok(BindStep::Complete(Completion::Return(Value::Object(
            self.0.bound.into_object(),
        ))))
    }
}
pub(super) fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: BindStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            BindStep::Complete(result) => return Ok(result),
            BindStep::Own {
                object,
                key,
                resume,
            } => resume.boolean(
                runtime,
                runtime.internal_has_own_property(realm, &object, &key)?,
            )?,
            BindStep::Read {
                object,
                key,
                resume,
            } => resume.resume(
                runtime,
                runtime.get_property_in_realm(realm, &object, &key)?,
            )?,
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<BindStep>() <= 64);
