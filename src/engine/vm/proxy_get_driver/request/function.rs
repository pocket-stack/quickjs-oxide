//! Mechanical adapters for function domain requests.
use super::{DirectCallTarget, Resume, Step, Value};

impl From<crate::engine::builtins::ArgumentsStep> for Step {
    fn from(step: crate::engine::builtins::ArgumentsStep) -> Self {
        use crate::engine::builtins::ArgumentsStep;
        match step {
            ArgumentsStep::Complete(result) => Self::ArgumentsComplete(Some(result)),
            ArgumentsStep::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::Arguments(resume)),
            },
            ArgumentsStep::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::Arguments(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::InvokeStep> for Step {
    fn from(step: crate::engine::builtins::InvokeStep) -> Self {
        use crate::engine::builtins::InvokeStep;
        match step {
            InvokeStep::Complete(result) => Self::Complete(Some(result)),
            InvokeStep::Construct(request) => {
                let target = request.target;
                let new_target = request.new_target;
                let arguments = request.arguments;
                Self::Construct {
                    target: Some(target),
                    new_target: Some(new_target),
                    arguments: Some(arguments),
                    resume: Some(Resume::Identity),
                }
            }
            InvokeStep::Arguments { mut resume } => {
                let value = resume.take_arguments_value();
                Self::Arguments {
                    value: Some(value),
                    resume: Some(Resume::Invoke(resume)),
                }
            }
            InvokeStep::Call(request) => {
                let target = request.target;
                let receiver = request.receiver;
                let arguments = request.arguments;
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Identity),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::InstanceStep> for Step {
    fn from(step: crate::engine::builtins::InstanceStep) -> Self {
        use crate::engine::builtins::InstanceStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Instance(resume)),
                }
            }
            T::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Instance(resume)),
                }
            }
            T::Prototype { mut resume } => {
                let object = resume.take_prototype_object();
                Self::GetPrototype {
                    object: Some(object),
                    resume: Some(Resume::Instance(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::vm::call::prototype::ProtoSourceStep> for Step {
    fn from(step: crate::engine::vm::call::prototype::ProtoSourceStep) -> Self {
        use crate::engine::vm::call::prototype::ProtoSourceStep as T;
        match step {
            T::Complete(result) => Self::ConstructorSourceComplete(Some(result)),
            T::ReadValue {
                receiver,
                key,
                resume,
            } => Self::ReadValue {
                receiver: Some(receiver),
                key: Some(key),
                resume: Some(Resume::ConstructorSource(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::BindStep> for Step {
    fn from(step: crate::engine::builtins::BindStep) -> Self {
        use crate::engine::builtins::BindStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::Bind(resume)),
            },
            T::Own {
                object,
                key,
                resume,
            } => Self::OwnFlag {
                object: Some(object),
                key: Some(key),
                enumerable: Some(false),
                resume: Some(Resume::Bind(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::FunctionTextStep> for Step {
    fn from(step: crate::engine::builtins::FunctionTextStep) -> Self {
        use crate::engine::builtins::FunctionTextStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::FunctionText(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::FunctionText(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::DynamicFunctionStep> for Step {
    fn from(step: crate::engine::builtins::DynamicFunctionStep) -> Self {
        use crate::engine::builtins::DynamicFunctionStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::DynamicFunction(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::DynamicFunction(resume)),
                }
            }
            T::Eval { mut resume } => {
                let source = resume.take_eval_source();
                Self::IndirectEval {
                    source: Some(source),
                    resume: Some(Resume::DynamicFunction(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::WeakConstructorStep> for Step {
    fn from(step: crate::engine::builtins::WeakConstructorStep) -> Self {
        use crate::engine::builtins::WeakConstructorStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Prototype { new_target, resume } => Self::ConstructorSource {
                new_target: Some(new_target),
                resume: Some(Resume::WeakConstructor(resume)),
            },
        }
    }
}
