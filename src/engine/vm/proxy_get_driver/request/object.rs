//! Mechanical adapters for object domain requests.
use super::{
    ArrayLengthStep, DescriptorStep, ProxyBooleanStep, ProxyDefineStep, ProxyGetStep, ProxyOwnStep,
    ProxyPrototypeStep, ProxySetStep, Resume, SetStep, Step, Value, set_completion,
};

impl From<ProxyGetStep> for Step {
    fn from(step: ProxyGetStep) -> Self {
        match step {
            ProxyGetStep::Complete(result) => Self::Complete(Some(result)),
            ProxyGetStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Get(resume)),
                }
            }
            ProxyGetStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Get(resume)),
                }
            }
            ProxyGetStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Get(resume)),
                }
            }
        }
    }
}

impl From<ProxyOwnStep> for Step {
    fn from(step: ProxyOwnStep) -> Self {
        match step {
            ProxyOwnStep::Complete(result) => Self::OwnComplete(Some(result)),
            ProxyOwnStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Own(resume)),
                }
            }
            ProxyOwnStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Own(resume)),
                }
            }
            ProxyOwnStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Own(resume)),
                }
            }
            ProxyOwnStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                Self::Extensible {
                    object: Some(object),
                    resume: Some(Resume::Own(resume)),
                }
            }
            ProxyOwnStep::Convert { mut resume } => {
                let value = resume.take_convert_value();
                Self::Convert {
                    value: Some(value),
                    resume: Some(Resume::Own(resume)),
                }
            }
        }
    }
}

impl From<DescriptorStep> for Step {
    fn from(step: DescriptorStep) -> Self {
        match step {
            DescriptorStep::Complete(resume) => Self::Converted(Some(
                crate::engine::value::conversion::NativeConversion::Value(resume.take_descriptor()),
            )),
            DescriptorStep::Throw(value) => Self::Converted(Some(
                crate::engine::value::conversion::NativeConversion::Throw(value),
            )),
            DescriptorStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Conversion(resume)),
                }
            }
            DescriptorStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Conversion(resume)),
                }
            }
        }
    }
}

impl From<ProxyBooleanStep> for Step {
    fn from(step: ProxyBooleanStep) -> Self {
        match step {
            ProxyBooleanStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
            ProxyBooleanStep::PreventExtensions { mut resume } => {
                let object = resume.take_prevent_extensions_object();
                Self::PreventExtensions {
                    object: Some(object),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
            ProxyBooleanStep::Complete(result) => Self::BooleanComplete(Some(result)),
            ProxyBooleanStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
            ProxyBooleanStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
            ProxyBooleanStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
            ProxyBooleanStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                Self::Extensible {
                    object: Some(object),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
            ProxyBooleanStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Boolean(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::object::ProxyCallStep> for Step {
    fn from(step: crate::engine::object::ProxyCallStep) -> Self {
        use crate::engine::object::ProxyCallStep;
        match step {
            ProxyCallStep::Complete(result) => Self::Complete(Some(result)),
            ProxyCallStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Call(resume)),
                }
            }
            ProxyCallStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Call(resume)),
                }
            }
        }
    }
}

impl From<SetStep> for Step {
    fn from(step: SetStep) -> Self {
        match step {
            SetStep::Complete(action) => Self::SetComplete(Some(action)),
            SetStep::Continue { resume } => Self::SetContinue(Some(resume)),
            SetStep::Proxy { mut resume } => {
                let object = resume.take_object();
                let key = resume.take_key();
                let value = resume.take_value();
                let receiver = resume.take_receiver();
                Self::SetProxy {
                    object: Some(object),
                    key: Some(key),
                    value: Some(value),
                    receiver: Some(receiver),
                    resume: Some(Resume::OrdinarySet(resume)),
                }
            }
            SetStep::Special { mut resume } => {
                let object = resume.take_object();
                let key = resume.take_key();
                let value = resume.take_value();
                let receiver = resume.take_receiver();
                Self::SetSpecial {
                    object: Some(object),
                    key: Some(key),
                    value: Some(value),
                    receiver: Some(receiver),
                    resume: Some(resume),
                }
            }
            SetStep::ArrayLength { mut resume } => {
                drop(resume.take_object());
                drop(resume.take_key());
                let value = resume.take_value();
                Self::SetLength {
                    value: Some(value),
                    resume: Some(resume),
                }
            }
            SetStep::Descriptor { mut resume } => {
                let object = resume.take_object();
                let key = resume.take_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::OrdinarySet(resume)),
                }
            }
            SetStep::Define { mut resume } => {
                let object = resume.take_object();
                let key = resume.take_key();
                let descriptor = resume.take_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::OrdinarySet(resume)),
                }
            }
        }
    }
}

impl From<ProxySetStep> for Step {
    fn from(step: ProxySetStep) -> Self {
        match step {
            ProxySetStep::Complete(result) => Self::SetComplete(Some(set_completion(result))),
            ProxySetStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::ProxySet(resume)),
                }
            }
            ProxySetStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::ProxySet(resume)),
                }
            }
            ProxySetStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                let receiver = resume.take_set_receiver();
                Self::Set {
                    object: Some(object),
                    key: Some(key),
                    value: Some(value),
                    receiver: Some(receiver),
                    resume: Some(Resume::ProxySet(resume)),
                }
            }
            ProxySetStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ProxySet(resume)),
                }
            }
        }
    }
}

impl From<ProxyDefineStep> for Step {
    fn from(step: ProxyDefineStep) -> Self {
        match step {
            ProxyDefineStep::Complete(result) => Self::Defined(Some(result)),
            ProxyDefineStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Define(resume)),
                }
            }
            ProxyDefineStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Define(resume)),
                }
            }
            ProxyDefineStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::Define(resume)),
                }
            }
            ProxyDefineStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Define(resume)),
                }
            }
        }
    }
}

impl From<ArrayLengthStep> for Step {
    fn from(step: ArrayLengthStep) -> Self {
        match step {
            ArrayLengthStep::Complete(result) => Self::LengthComplete(Some(result)),
            ArrayLengthStep::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::LengthNumber(resume)),
            },
        }
    }
}

impl From<ProxyPrototypeStep> for Step {
    fn from(step: ProxyPrototypeStep) -> Self {
        match step {
            ProxyPrototypeStep::Complete(result) => Self::Complete(Some(result)),
            ProxyPrototypeStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Prototype(resume)),
                }
            }
            ProxyPrototypeStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Prototype(resume)),
                }
            }
            ProxyPrototypeStep::Get { mut resume } => {
                let object = resume.take_get_object();
                Self::GetPrototype {
                    object: Some(object),
                    resume: Some(Resume::Prototype(resume)),
                }
            }
            ProxyPrototypeStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let prototype = resume.take_set_prototype();
                Self::SetPrototype {
                    object: Some(object),
                    prototype: Some(prototype),
                    resume: Some(Resume::Prototype(resume)),
                }
            }
            ProxyPrototypeStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                Self::Extensible {
                    object: Some(object),
                    resume: Some(Resume::Prototype(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::object::KeysStep> for Step {
    fn from(step: crate::engine::object::KeysStep) -> Self {
        use crate::engine::object::KeysStep;
        match step {
            KeysStep::Complete(result) => Self::KeysComplete(Some(result)),
            KeysStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::Keys(resume)),
                }
            }
            KeysStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Keys(resume)),
                }
            }
            KeysStep::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::Keys(resume)),
                }
            }
            KeysStep::Keys { mut resume } => {
                let object = resume.take_keys_object();
                Self::Keys {
                    object: Some(object),
                    resume: Some(Resume::Keys(resume)),
                }
            }
            KeysStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                Self::Extensible {
                    object: Some(object),
                    resume: Some(Resume::Keys(resume)),
                }
            }
            KeysStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Keys(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::object::ProxyConstructStep> for Step {
    fn from(step: crate::engine::object::ProxyConstructStep) -> Self {
        use crate::engine::object::ProxyConstructStep;
        match step {
            ProxyConstructStep::Complete(result) => Self::Complete(Some(result)),
            ProxyConstructStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::ProxyConstruct(resume)),
                }
            }
            ProxyConstructStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::ProxyConstruct(resume)),
                }
            }
            ProxyConstructStep::Construct { mut resume } => {
                let target = resume.take_construct_target();
                let new_target = resume.take_construct_new_target();
                let arguments = resume.take_construct_arguments();
                Self::Construct {
                    target: Some(target),
                    new_target: Some(new_target),
                    arguments: Some(arguments),
                    resume: Some(Resume::ProxyConstruct(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::object::object_literal::element::LiteralDefinitionStep> for Step {
    fn from(step: crate::engine::object::object_literal::element::LiteralDefinitionStep) -> Self {
        use crate::engine::object::object_literal::element::LiteralDefinitionStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { mut resume } => {
                let value = resume.take_primitive();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::LiteralDefinition(resume)),
                }
            }
            T::Define { mut resume } => {
                let (object, key, descriptor) = resume.take_define();
                Self::DefineOrdinary {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::LiteralDefinition(resume)),
                }
            }
        }
    }
}
