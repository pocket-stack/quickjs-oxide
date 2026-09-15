//! Mechanical adapters for conversion domain requests.
use super::{DirectCallTarget, NumberStep, Resume, Step, Value};

impl From<NumberStep> for Step {
    fn from(step: NumberStep) -> Self {
        match step {
            NumberStep::Complete(result) => Self::NumberComplete(Some(result)),
            NumberStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Number(resume)),
                }
            }
            NumberStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Number(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::value::conversion::primitive::PrimitiveStep> for Step {
    fn from(step: crate::engine::value::conversion::primitive::PrimitiveStep) -> Self {
        use crate::engine::value::conversion::primitive::PrimitiveStep;
        match step {
            PrimitiveStep::Complete(result) => Self::Complete(Some(result)),
            PrimitiveStep::Get { mut resume } => {
                let (object, key) = resume.take_get();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Primitive(resume)),
                }
            }
            PrimitiveStep::Call { mut resume } => {
                let callable = resume.take_callable();
                let receiver = resume.take_receiver();
                let arguments = resume.take_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Primitive(resume)),
                }
            }
        }
    }
}
