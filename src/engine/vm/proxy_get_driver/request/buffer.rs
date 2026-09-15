//! Mechanical adapters for buffer domain requests.
use super::{DirectCallTarget, ElementStep, Resume, Step, TypedWriteStep, Value};

impl From<ElementStep> for Step {
    fn from(step: ElementStep) -> Self {
        match step {
            ElementStep::Complete(result) => Self::ElementComplete(Some(result)),
            ElementStep::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::Element(resume)),
                }
            }
            ElementStep::Call { mut resume } => {
                let callable = resume.take_call_callable();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(DirectCallTarget::Callable(callable)),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::Element(resume)),
                }
            }
        }
    }
}

impl From<TypedWriteStep> for Step {
    fn from(step: TypedWriteStep) -> Self {
        match step {
            TypedWriteStep::Complete(result) => Self::TypedComplete(Some(result)),
            TypedWriteStep::Element {
                element,
                value,
                resume,
            } => Self::Element {
                element: Some(element),
                value: Some(value),
                resume: Some(Resume::TypedElement(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::DataViewAccessStep> for Step {
    fn from(step: crate::engine::builtins::DataViewAccessStep) -> Self {
        use crate::engine::builtins::DataViewAccessStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::DataView(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::BufferMutationStep> for Step {
    fn from(step: crate::engine::builtins::BufferMutationStep) -> Self {
        use crate::engine::builtins::BufferMutationStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::BufferMutation(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedTraversalStep> for Step {
    fn from(step: crate::engine::builtins::TypedTraversalStep) -> Self {
        use crate::engine::builtins::TypedTraversalStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::TypedTraversal(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::TypedSpeciesStep> for Step {
    fn from(step: crate::engine::builtins::TypedSpeciesStep) -> Self {
        use crate::engine::builtins::TypedSpeciesStep as T;
        match step {
            T::Complete(result) => Self::TypedSpeciesComplete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::TypedSpecies(resume)),
            },
            T::Construct {
                constructor,
                arguments,
                resume,
            } => Self::Construct {
                new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                    constructor.clone(),
                )),
                target: Some(constructor),
                arguments: Some(arguments),
                resume: Some(Resume::TypedSpecies(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedIterationStep> for Step {
    fn from(step: crate::engine::builtins::TypedIterationStep) -> Self {
        use crate::engine::builtins::TypedIterationStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let object = resume.take_read_object();
                let key = resume.take_read_key();
                Self::Read {
                    receiver: Some(Value::Object(object.clone())),
                    object: Some(object),
                    key: Some(key),
                    resume: Some(Resume::TypedIteration(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::TypedIteration(resume)),
                }
            }
            T::Species { mut resume } => {
                let source = resume.take_species_source();
                let element = resume.take_species_element();
                let length = resume.take_species_length();
                Self::TypedSpecies {
                    source: Some(source),
                    element: Some(element),
                    length: Some(length),
                    resume: Some(Resume::TypedIteration(resume)),
                }
            }
            T::Element { mut resume } => {
                let element = resume.take_element_element();
                let value = resume.take_element_value();
                Self::Element {
                    element: Some(element),
                    value: Some(value),
                    resume: Some(Resume::TypedIteration(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::TypedSortStep> for Step {
    fn from(step: crate::engine::builtins::TypedSortStep) -> Self {
        use crate::engine::builtins::TypedSortStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Call {
                callable,
                arguments,
                resume,
            } => Self::Call {
                target: Some(DirectCallTarget::Callable(callable)),
                receiver: Some(Value::Undefined),
                arguments: Some(arguments),
                resume: Some(Resume::TypedSort(resume)),
            },
            T::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::TypedSort(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::BufferConstructorStep> for Step {
    fn from(step: crate::engine::builtins::BufferConstructorStep) -> Self {
        use crate::engine::builtins::BufferConstructorStep as T;
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
                resume: Some(Resume::BufferConstructor(resume)),
            },
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::BufferConstructor(resume)),
            },
            T::Prototype { new_target, resume } => Self::ConstructorSource {
                new_target: Some(new_target),
                resume: Some(Resume::BufferConstructor(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::DataViewConstructorStep> for Step {
    fn from(step: crate::engine::builtins::DataViewConstructorStep) -> Self {
        use crate::engine::builtins::DataViewConstructorStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::DataViewConstructor(resume)),
            },
            T::Prototype { new_target, resume } => Self::ConstructorSource {
                new_target: Some(new_target),
                resume: Some(Resume::DataViewConstructor(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedSetStep> for Step {
    fn from(step: crate::engine::builtins::TypedSetStep) -> Self {
        use crate::engine::builtins::TypedSetStep as T;
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
                resume: Some(Resume::TypedSet(resume)),
            },
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::TypedSet(resume)),
            },
            T::Element {
                element,
                value,
                resume,
            } => Self::Element {
                element: Some(element),
                value: Some(value),
                resume: Some(Resume::TypedSet(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedSearchStep> for Step {
    fn from(step: crate::engine::builtins::TypedSearchStep) -> Self {
        use crate::engine::builtins::TypedSearchStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::TypedSearch(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedStringStep> for Step {
    fn from(step: crate::engine::builtins::TypedStringStep) -> Self {
        use crate::engine::builtins::TypedStringStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::String),
                    resume: Some(Resume::TypedString(resume)),
                }
            }
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::TypedString(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::TypedString(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::TypedSliceStep> for Step {
    fn from(step: crate::engine::builtins::TypedSliceStep) -> Self {
        use crate::engine::builtins::TypedSliceStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                    resume: Some(Resume::TypedSlice(resume)),
                }
            }
            T::Species { mut resume } => {
                let source = resume.take_species_source();
                let element = resume.take_species_element();
                let length = resume.take_species_length();
                Self::TypedSpecies {
                    source: Some(source),
                    element: Some(element),
                    length: Some(length),
                    resume: Some(Resume::TypedSlice(resume)),
                }
            }
            T::SpeciesView { mut resume } => {
                let source = resume.take_species_view_source();
                let element = resume.take_species_view_element();
                let buffer = resume.take_species_view_buffer();
                let offset = resume.take_species_view_offset();
                let length = resume.take_species_view_length();
                Self::TypedSpeciesView {
                    source: Some(source),
                    element: Some(element),
                    buffer: Some(buffer),
                    offset: Some(offset),
                    length: Some(length),
                    resume: Some(Resume::TypedSlice(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::TypedMutationStep> for Step {
    fn from(step: crate::engine::builtins::TypedMutationStep) -> Self {
        use crate::engine::builtins::TypedMutationStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::TypedMutation(resume)),
            },
            T::Element {
                element,
                value,
                resume,
            } => Self::Element {
                element: Some(element),
                value: Some(value),
                resume: Some(Resume::TypedMutation(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::BufferSliceStep> for Step {
    fn from(step: crate::engine::builtins::BufferSliceStep) -> Self {
        use crate::engine::builtins::BufferSliceStep as T;
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
                resume: Some(Resume::BufferSlice(resume)),
            },
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::BufferSlice(resume)),
            },
            T::Construct {
                constructor,
                arguments,
                resume,
            } => Self::Construct {
                new_target: Some(crate::engine::vm::call::ConstructNewTarget::Validated(
                    constructor.clone(),
                )),
                target: Some(constructor),
                arguments: Some(arguments),
                resume: Some(Resume::BufferSlice(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedWithStep> for Step {
    fn from(step: crate::engine::builtins::TypedWithStep) -> Self {
        use crate::engine::builtins::TypedWithStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::TypedWith(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::Uint8CodecStep> for Step {
    fn from(step: crate::engine::builtins::Uint8CodecStep) -> Self {
        use crate::engine::builtins::Uint8CodecStep as T;
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
                resume: Some(Resume::Uint8Codec(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedIteratorMethodStep> for Step {
    fn from(step: crate::engine::builtins::TypedIteratorMethodStep) -> Self {
        use crate::engine::builtins::TypedIteratorMethodStep as T;
        match step {
            T::Complete(result) => Self::TypedIteratorMethodComplete(Some(result)),
            T::Read {
                receiver,
                key,
                resume,
            } => Self::ReadValue {
                receiver: Some(receiver),
                key: Some(key),
                resume: Some(Resume::TypedIteratorMethod(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedCollectStep> for Step {
    fn from(step: crate::engine::builtins::TypedCollectStep) -> Self {
        use crate::engine::builtins::TypedCollectStep as T;
        match step {
            T::Complete(result) => Self::TypedCollectComplete(Some(result)),
            T::Read {
                object,
                key,
                resume,
            } => Self::Read {
                receiver: Some(Value::Object(object.clone())),
                object: Some(object),
                key: Some(key),
                resume: Some(Resume::TypedCollect(resume)),
            },
            T::Call {
                callable,
                receiver,
                resume,
            } => Self::Call {
                target: Some(DirectCallTarget::Callable(callable)),
                receiver: Some(receiver),
                arguments: Some(Vec::new()),
                resume: Some(Resume::TypedCollect(resume)),
            },
        }
    }
}

impl From<crate::engine::builtins::TypedCreateStep> for Step {
    fn from(step: crate::engine::builtins::TypedCreateStep) -> Self {
        use crate::engine::builtins::TypedCreateStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Read { mut resume } => {
                let receiver = resume.take_read_receiver();
                let key = resume.take_read_key();
                Self::ReadValue {
                    receiver: Some(receiver),
                    key: Some(key),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Primitive { mut resume } => {
                let value = resume.take_primitive_value();
                Self::Primitive {
                    value: Some(value),
                    hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Call { mut resume } => {
                let target = resume.take_call_target();
                let receiver = resume.take_call_receiver();
                let arguments = resume.take_call_arguments();
                Self::Call {
                    target: Some(target),
                    receiver: Some(receiver),
                    arguments: Some(arguments),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Prototype { mut resume } => {
                let new_target = resume.take_prototype_new_target();
                Self::ConstructorSource {
                    new_target: Some(new_target),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Method { mut resume } => {
                let source = resume.take_method_source();
                Self::TypedIteratorMethod {
                    source: Some(source),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Collect { mut resume } => {
                let source = resume.take_collect_source();
                let method = resume.take_collect_method();
                let element = resume.take_collect_element();
                Self::TypedCollect {
                    source: Some(source),
                    method: Some(method),
                    element: Some(element),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Create { mut resume } => {
                let constructor = resume.take_create_constructor();
                let length = resume.take_create_length();
                Self::TypedCreate {
                    constructor: Some(constructor),
                    length: Some(length),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
            T::Element { mut resume } => {
                let element = resume.take_element_element();
                let value = resume.take_element_value();
                Self::Element {
                    element: Some(element),
                    value: Some(value),
                    resume: Some(Resume::TypedCreate(resume)),
                }
            }
        }
    }
}

impl From<crate::engine::builtins::AtomicsStep> for Step {
    fn from(step: crate::engine::builtins::AtomicsStep) -> Self {
        use crate::engine::builtins::AtomicsStep as T;
        match step {
            T::Complete(result) => Self::Complete(Some(result)),
            T::Primitive { value, resume } => Self::Primitive {
                value: Some(value),
                hint: Some(crate::engine::vm::ToPrimitiveHint::Number),
                resume: Some(Resume::Atomics(resume)),
            },
            T::Number { value, resume } => Self::Number {
                value: Some(value),
                resume: Some(Resume::Atomics(resume)),
            },
        }
    }
}
