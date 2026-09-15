//! Mechanical adapters for object builtins domain requests.
use super::{DirectCallTarget, Resume, Step, Value};

impl From<crate::engine::builtins::BuiltinPrototypeStep> for Step {
    fn from(step: crate::engine::builtins::BuiltinPrototypeStep) -> Self {
        use crate::engine::builtins::BuiltinPrototypeStep;
        match step {
            BuiltinPrototypeStep::Complete(result) => Self::Complete(Some(result)),
            BuiltinPrototypeStep::Get { object, resume } => Self::GetPrototype {
                object: Some(object),
                resume: Some(Resume::BuiltinPrototype(resume)),
            },
            BuiltinPrototypeStep::Set {
                object,
                prototype,
                resume,
            } => Self::SetPrototype {
                object: Some(object),
                prototype: Some(prototype),
                resume: Some(Resume::BuiltinPrototype(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::PropertyStep> for Step {
    fn from(step: crate::engine::builtins::PropertyStep) -> Self {
        use crate::engine::builtins::PropertyStep;
        match step {
            PropertyStep::Complete(result) => Self::Complete(Some(result)),
            PropertyStep::Keys { mut resume } => {
                let object = resume.take_keys_object();
                Self::Keys {
                    object: Some(object),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Key { mut resume } => {
                let value = resume.take_key_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::PropertyKey(resume)),
                }
            }
            PropertyStep::Convert { mut resume } => {
                let value = resume.take_convert_value();
                Self::Convert {
                    value: Some(value),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                let receiver = resume.take_read_receiver();
                Self::Read {
                    object: Some(object),
                    key: Some(key),
                    receiver: Some(receiver),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Set { mut resume } => {
                let object = resume.take_set_object();
                let key = resume.take_set_key();
                let value = resume.take_set_value();
                let receiver = resume.take_set_receiver();
                Self::Set {
                    object: Some(object),
                    key: Some(key),
                    value: Some(value),
                    receiver: Some(receiver),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Has { mut resume } => {
                let object = resume.take_has_object();
                let key = resume.take_has_key();
                Self::Has {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Extensible { mut resume } => {
                let object = resume.take_extensible_object();
                Self::Extensible {
                    object: Some(object),
                    resume: Some(Resume::Property(resume)),
                }
            }
            PropertyStep::Prevent { mut resume } => {
                let object = resume.take_prevent_object();
                Self::PreventExtensions {
                    object: Some(object),
                    resume: Some(Resume::Property(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::PredicateStep> for Step {
    fn from(step: crate::engine::builtins::PredicateStep) -> Self {
        use crate::engine::builtins::PredicateStep;
        match step {
            PredicateStep::Complete(result) => Self::Complete(Some(result)),
            PredicateStep::Descriptor { mut resume } => {
                let object = resume.take_descriptor_object();
                let key = resume.take_descriptor_key();
                Self::Descriptor {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Predicate(resume)),
                }
            }
            PredicateStep::Prototype { mut resume } => {
                let object = resume.take_prototype_object();
                Self::GetPrototype {
                    object: Some(object),
                    resume: Some(Resume::Predicate(resume)),
                }
            }
            PredicateStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::Predicate(resume)),
                }
            }
            PredicateStep::Key { mut resume } => {
                let value = resume.take_key_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::PredicateKey(resume)),
                }
            }
            PredicateStep::Own { mut resume } => {
                let object = resume.take_own_object();
                let key = resume.take_own_key();
                let enumerable = resume.take_own_enumerable();
                Self::OwnFlag {
                    object: Some(object),
                    key: Some(key),
                    enumerable: Some(enumerable),
                    resume: Some(Resume::Predicate(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::DefinitionsStep> for Step {
    fn from(step: crate::engine::builtins::DefinitionsStep) -> Self {
        use crate::engine::builtins::DefinitionsStep;
        match step {
            DefinitionsStep::Complete(result) => Self::Complete(Some(result)),
            DefinitionsStep::Keys { mut resume } => {
                let object = resume.take_keys_object();
                Self::Keys {
                    object: Some(object),
                    resume: Some(Resume::Definitions(resume)),
                }
            }
            DefinitionsStep::Enumerable { mut resume } => {
                let object = resume.take_enumerable_object();
                let key = resume.take_enumerable_key();
                Self::SnapshotEnumerable {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Definitions(resume)),
                }
            }
            DefinitionsStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Definitions(resume)),
                }
            }
            DefinitionsStep::Convert { mut resume } => {
                let value = resume.take_convert_value();
                Self::Convert {
                    value: Some(value),
                    resume: Some(Resume::Definitions(resume)),
                }
            }
            DefinitionsStep::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::Definitions(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ObjectStringStep> for Step {
    fn from(step: crate::engine::builtins::ObjectStringStep) -> Self {
        use crate::engine::builtins::ObjectStringStep;
        match step {
            ObjectStringStep::Complete(result) => Self::Complete(Some(result)),
            ObjectStringStep::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::ObjectString(resume)),
                }
            }
            ObjectStringStep::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(Vec::new()),
                    resume: Some(Resume::ObjectString(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::ObjectIterationStep> for Step {
    fn from(step: crate::engine::builtins::ObjectIterationStep) -> Self {
        use crate::engine::builtins::ObjectIterationStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::ObjectIteration(resume)),
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
                    resume: Some(Resume::ObjectIteration(resume)),
                }
            }
            T::Next { mut resume } => {
                let iterator = resume.take_next_iterator();
                let method = resume.take_next_method();
                Self::IteratorNext {
                    iterator: Some(iterator),
                    method: Some(method),
                    resume: Some(Resume::ObjectIteration(resume)),
                }
            }
            T::Key { mut resume } => {
                let value = resume.take_key_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::ObjectIterationKey(resume)),
                }
            }
            T::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::ObjectIteration(resume)),
                }
            }
            T::Push { mut resume } => {
                let object = resume.take_push_object();
                let value = resume.take_push_value();
                Self::ArrayPush {
                    object: Some(object),
                    value: Some(value),
                    resume: Some(Resume::ObjectIteration(resume)),
                }
            }
            T::Close {
                iterator,
                completion,
            } => Self::IteratorClose {
                iterator: Some(iterator),
                completion: Some(completion),
            },
        }
    }
}

impl From<crate::engine::builtins::ObjectCopyStep> for Step {
    fn from(step: crate::engine::builtins::ObjectCopyStep) -> Self {
        use crate::engine::builtins::ObjectCopyStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            #[cfg(feature = "stack-vm")]
            T::PreparedRead(prepared) => Self::PreparedRead {
                read: Some(prepared.read),
                key: Some(prepared.key),
                resume: Some(Resume::ObjectCopy(prepared.resume)),
            },
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::ObjectCopy(resume)),
            },
            T::Keys { object, resume } => Self::Keys {
                object: Some(object),
                resume: Some(Resume::ObjectCopy(resume)),
            },
            T::Enumerable {
                object,
                key,
                resume,
            } => Self::SnapshotEnumerable {
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::ObjectCopy(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::ObjectConstructorStep> for Step {
    fn from(step: crate::engine::builtins::ObjectConstructorStep) -> Self {
        use crate::engine::builtins::ObjectConstructorStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Prototype { new_target, resume } => Self::ConstructorSource {
                new_target: Some(new_target),
                resume: Some(Resume::ObjectConstructor(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::JsonParseStep> for Step {
    fn from(step: crate::engine::builtins::JsonParseStep) -> Self {
        use crate::engine::builtins::JsonParseStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
            T::Keys { mut resume } => {
                let object = resume.take_keys_object();
                Self::Keys {
                    object: Some(object),
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
            T::Enumerable { mut resume } => {
                let object = resume.take_enumerable_object();
                let key = resume.take_enumerable_key();
                Self::SnapshotEnumerable {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::JsonParse(resume)),
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
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
            T::Delete { mut resume } => {
                let object = resume.take_delete_object();
                let key = resume.take_delete_key();
                Self::Delete {
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
            T::Define { mut resume } => {
                let object = resume.take_define_object();
                let key = resume.take_define_key();
                let descriptor = resume.take_define_descriptor();
                Self::Define {
                    object: Some(object),
                    key: Some(key),
                    descriptor: Some(descriptor),
                    resume: Some(Resume::JsonParse(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::JsonStringifyStep> for Step {
    fn from(step: crate::engine::builtins::JsonStringifyStep) -> Self {
        use crate::engine::builtins::JsonStringifyStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::JsonStringify(resume)),
                }
            }
            T::String { mut resume } => {
                let value = resume.take_string_value();
                Self::String {
                    value: Some(value),
                    resume: Some(Resume::JsonStringify(resume)),
                }
            }
            T::Number { mut resume } => {
                let value = resume.take_number_value();
                Self::Number {
                    value: Some(value),
                    resume: Some(Resume::JsonStringify(resume)),
                }
            }
            T::Keys { mut resume } => {
                let object = resume.take_keys_object();
                Self::Keys {
                    object: Some(object),
                    resume: Some(Resume::JsonStringify(resume)),
                }
            }
            T::Enumerable { mut resume } => {
                let object = resume.take_enumerable_object();
                let key = resume.take_enumerable_key();
                Self::OwnFlag {
                    object: Some(object),
                    key: Some(key),
                    enumerable: Some(true),
                    resume: Some(Resume::JsonStringify(resume)),
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
                    resume: Some(Resume::JsonStringify(resume)),
                }
            }
        }
    }
}
