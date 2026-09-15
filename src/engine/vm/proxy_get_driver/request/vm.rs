//! Mechanical adapters for vm domain requests.
use super::{Completion, Resume, Step, Value};

impl From<crate::engine::vm::environment_bindings::operation::EnvironmentStep> for Step {
    fn from(step: crate::engine::vm::environment_bindings::operation::EnvironmentStep) -> Self {
        use crate::engine::vm::environment_bindings::operation::EnvironmentStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    receiver: Some(receiver),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Environment(resume)),
                }
            }
            T::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Environment(resume)),
                }
            }
            T::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Environment(resume)),
                }
            }
            T::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                Self::Set {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    value: Some(value),
                    resume: Some(Resume::Environment(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::vm::numeric::operation::NumericStep> for Step {
    fn from(step: crate::engine::vm::numeric::operation::NumericStep) -> Self {
        use crate::engine::vm::numeric::operation::NumericStep as T;
        match step {
            T::Complete { value, previous } => Self::NumericComplete {
                value: Some(value),
                previous: Some(previous),
            },
            T::Throw(value) => Self::Complete(Some(Completion::Throw(value))),
            T::Primitive {
                value,
                hint,
                resume,
            } => Self::Primitive {
                value: Some(value),
                hint: Some(hint),
                resume: Some(Resume::VmNumeric(resume)),
            },
            T::HtmlDda { value, resume } => Self::NumericHtmlDda {
                value: Some(value),
                resume: Some(resume),
            },
        }
    }
}

impl From<crate::engine::vm::for_in::operation::ForInStep> for Step {
    fn from(step: crate::engine::vm::for_in::operation::ForInStep) -> Self {
        use crate::engine::vm::for_in::operation::ForInStep as T;
        match step {
            T::Complete { value, done } => Self::ForInComplete {
                value: Some(value),
                done: Some(done),
            },
            T::Throw(value) => Self::Complete(Some(Completion::Throw(value))),
            T::Keys { object, resume } => Self::Keys {
                object: Some(object),
                resume: Some(Resume::ForIn(resume)),
            },
            T::Enumerable {
                object,
                key,
                resume,
            } => Self::SnapshotEnumerable {
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::ForIn(resume)),
            },
            T::Own {
                object,
                key,
                resume,
            } => Self::OwnFlag {
                object: Some(object),
                key: Some(key),
                enumerable: Some(false),
                resume: Some(Resume::ForIn(resume)),
            },
            T::Prototype { object, resume } => Self::GetPrototype {
                object: Some(object),
                resume: Some(Resume::ForIn(resume)),
            },
        }
    }
}

impl From<crate::engine::vm::suspend::creation::CreationStep> for super::Step {
    fn from(step: crate::engine::vm::suspend::creation::CreationStep) -> Self {
        match step {
            crate::engine::vm::suspend::creation::CreationStep::Complete(completion) => {
                Self::Complete(Some(completion))
            }
            crate::engine::vm::suspend::creation::CreationStep::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(crate::engine::value::Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(super::Resume::GeneratorPrototype(resume)),
            },
        }
    }
}

impl From<crate::engine::vm::async_function::AsyncStep> for super::Step {
    fn from(step: crate::engine::vm::async_function::AsyncStep) -> Self {
        use crate::engine::vm::async_function::AsyncStep as S;
        match step {
            S::Complete(completion) => Self::Complete(Some(completion)),
            S::Run { mut resume } => {
                let activation = resume.take_run_activation();
                let input = resume.take_run_input();
                Self::ResumeFrame {
                    activation: Some(activation),
                    input: Some(input),
                    resume: Some(super::Resume::Async(resume)),
                }
            }
            S::Resolve { mut resume } => {
                let value = resume.take_resolve_value();
                let realm = resume.take_resolve_realm();
                Self::IntrinsicPromiseResolve {
                    value: Some(value),
                    realm: Some(realm),
                    resume: Some(super::Resume::Async(resume)),
                }
            }
            S::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let value = resume.take_call_value();
                Self::Call {
                    target: Some(super::DirectCallTarget::Callable(callable)),
                    receiver: Some(crate::engine::value::Value::Undefined),
                    arguments: Some(vec![value]),
                    resume: Some(super::Resume::Async(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::vm::async_generator::AsyncGeneratorStep> for super::Step {
    fn from(step: crate::engine::vm::async_generator::AsyncGeneratorStep) -> Self {
        use crate::engine::vm::async_generator::AsyncGeneratorStep as S;
        match step {
            S::Complete(completion) => Self::Complete(Some(completion)),
            S::Run { mut resume } => {
                let activation = resume.take_run_activation();
                let input = resume.take_run_input();
                Self::ResumeFrame {
                    activation: Some(activation),
                    input: Some(input),
                    resume: Some(super::Resume::AsyncGenerator(resume)),
                }
            }
            S::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let value = resume.take_call_value();
                Self::Call {
                    target: Some(super::DirectCallTarget::Callable(callable)),
                    receiver: Some(Value::Undefined),
                    arguments: Some(vec![value]),
                    resume: Some(super::Resume::AsyncGenerator(resume)),
                }
            }
            S::Resolve { mut resume } => {
                let value = resume.take_resolve_value();
                let realm = resume.take_resolve_realm();
                Self::IntrinsicPromiseResolve {
                    value: Some(value),
                    realm: Some(realm),
                    resume: Some(super::Resume::AsyncGenerator(resume)),
                }
            }
        }
    }
}
