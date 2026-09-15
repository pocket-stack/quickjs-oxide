//! Integer indexed writes retain their owners, then reacquire buffer access.
use super::element::ElementStep;
use crate::engine::{
    api::{runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::TypedArrayElementKind,
    heap::ContextId,
    object::{DescriptorField, ObjectRef, OrdinaryPropertyDescriptor},
    value::{Value, conversion::NativeConversion},
};

pub(crate) enum TypedWriteStep {
    Complete(NativeConversion<bool>),
    Element {
        element: TypedArrayElementKind,
        value: Value,
        resume: TypedWriteResume,
    },
}
pub(crate) struct TypedWriteResume(Box<TypedWriteResumeState>);
impl std::ops::Deref for TypedWriteResume {
    type Target = TypedWriteResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedWriteResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedWriteResume>() <= 8);
pub(crate) struct TypedWriteResumeState {
    object: ObjectRef,
    index: Option<u64>,
    _value: Value,
}
impl TypedWriteStep {
    pub(crate) fn set(
        runtime: &Runtime,
        object: ObjectRef,
        index: Option<u64>,
        value: Value,
    ) -> Result<Self, RuntimeError> {
        let element = runtime.typed_array_snapshot(&object)?.element;
        Ok(Self::Element {
            element,
            value: value.clone(),
            resume: TypedWriteResume(Box::new(TypedWriteResumeState {
                object,
                index,
                _value: value,
            })),
        })
    }
    /// Primitive Set performs the same conversion before reacquiring buffer
    /// access, but never constructs a waiting resume or clones the view root.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn set_primitive(
        runtime: &Runtime,
        realm: ContextId,
        object: &ObjectRef,
        index: Option<u64>,
        value: &Value,
    ) -> Result<Self, RuntimeError> {
        Self::set_primitive_result(runtime, realm, object, index, value).map(Self::Complete)
    }

    /// Small transport for the same primitive conversion and final write.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn set_primitive_result(
        runtime: &Runtime,
        realm: ContextId,
        object: &ObjectRef,
        index: Option<u64>,
        value: &Value,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        if matches!(value, Value::Object(_)) {
            return Err(RuntimeError::Invariant(
                "primitive typed Set received an object",
            ));
        }
        let element = runtime.typed_array_snapshot(object)?.element;
        let result = super::element::encode_primitive(runtime, realm, element, value.clone())?;
        finish_element(runtime, object, index, result)
    }
    pub(crate) fn define(
        runtime: &Runtime,
        object: ObjectRef,
        index: u64,
        descriptor: &OrdinaryPropertyDescriptor,
    ) -> Result<Self, RuntimeError> {
        if descriptor.get.is_present()
            || descriptor.set.is_present()
            || matches!(descriptor.writable, DescriptorField::Present(false))
            || matches!(descriptor.enumerable, DescriptorField::Present(false))
            || matches!(descriptor.configurable, DescriptorField::Present(false))
        {
            return Ok(Self::Complete(NativeConversion::Value(false)));
        }
        let state = runtime.typed_array_state(&object)?;
        if state.out_of_bounds || index >= u64::from(state.length) {
            return Ok(Self::Complete(NativeConversion::Value(false)));
        }
        let DescriptorField::Present(value) = &descriptor.value else {
            return Ok(Self::Complete(NativeConversion::Value(true)));
        };
        Ok(Self::Element {
            element: state.snapshot.element,
            value: value.clone(),
            resume: TypedWriteResume(Box::new(TypedWriteResumeState {
                object,
                index: Some(index),
                _value: value.clone(),
            })),
        })
    }
    /// Advance only a primitive input through the shared conversion and write
    /// kernels. Object inputs retain the original request for the owned driver.
    #[cfg(feature = "stack-vm")]
    pub(crate) fn complete_primitive(
        self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<Self, RuntimeError> {
        match self {
            Self::Element {
                element,
                value,
                resume,
            } if !matches!(value, Value::Object(_)) => {
                let ElementStep::Complete(result) =
                    ElementStep::start(runtime, realm, element, value)?
                else {
                    return Err(RuntimeError::Invariant(
                        "primitive element conversion suspended",
                    ));
                };
                resume.element(runtime, result)
            }
            step => Ok(step),
        }
    }

    pub(crate) fn finish_sync(
        self,
        runtime: &Runtime,
        realm: ContextId,
    ) -> Result<NativeConversion<bool>, RuntimeError> {
        match self {
            Self::Complete(result) => Ok(result),
            Self::Element {
                element,
                value,
                resume,
            } => {
                let bytes = ElementStep::start(runtime, realm, element, value)?
                    .finish_sync(runtime, realm)?;
                let Self::Complete(result) = resume.element(runtime, bytes)? else {
                    return Err(RuntimeError::Invariant(
                        "TypedArray write failed to complete",
                    ));
                };
                Ok(result)
            }
        }
    }
}
impl TypedWriteResume {
    pub(crate) fn element(
        self,
        runtime: &Runtime,
        result: NativeConversion<[u8; 8]>,
    ) -> Result<TypedWriteStep, RuntimeError> {
        Ok(TypedWriteStep::Complete(finish_element(
            runtime,
            &self.0.object,
            self.0.index,
            result,
        )?))
    }
}

fn finish_element(
    runtime: &Runtime,
    object: &ObjectRef,
    index: Option<u64>,
    result: NativeConversion<[u8; 8]>,
) -> Result<NativeConversion<bool>, RuntimeError> {
    let result = match result {
        NativeConversion::Throw(value) => NativeConversion::Throw(value),
        NativeConversion::Value(bytes) => {
            if let Some(index) = index {
                // Conversion may detach, resize, or replace the backing bytes.
                // Both Set and Define ignore a failed post-conversion write.
                let _ = runtime.typed_array_write_converted_index(object, index, &bytes)?;
            }
            NativeConversion::Value(true)
        }
    };
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "stack-vm")]
    #[test]
    fn primitive_set_keeps_invalid_index_conversion_and_receiver_rules() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        assert_eq!(
            context
                .eval(
                    r#"(()=>{
            let a=new Uint8Array(1), errors=0;
            a[0]=257;
            try { a['-0']=Symbol(); } catch(e) { if(e instanceof TypeError)errors++; }
            try { a[NaN]=1n; } catch(e) { if(e instanceof TypeError)errors++; }
            let marker=Symbol(); a['01']=marker;
            let receiver={};
            let valid=Reflect.set(a,'0',19,receiver);
            let invalid=Reflect.set(a,'-0',marker,receiver);
            let b=new BigInt64Array(1); b[0]='7';
            try { b[1]=1; } catch(e) { if(e instanceof TypeError)errors++; }
            return errors===3 && a[0]===1 && a['01']===marker && b[0]===7n
                && valid && receiver[0]===19 && invalid && !('-0' in receiver);
        })()"#
                )
                .unwrap(),
            Value::Bool(true)
        );
    }
    #[cfg(feature = "stack-vm")]
    #[test]
    fn small_primitive_selection_matches_owned_wrapper_for_receiver_and_key_rules() {
        for wrapper in [false, true] {
            let runtime = Runtime::new();
            let mut context = runtime.new_context();
            for (key, input, same_receiver, expected) in [
                ("0", "257", true, "stored"),
                ("-0", "Symbol()", true, "throw"),
                ("NaN", "1n", true, "throw"),
                ("01", "Symbol()", true, "decline"),
                ("0", "Symbol()", false, "decline"),
                ("-0", "Symbol()", false, "ignore"),
                ("5", "Symbol()", false, "ignore"),
            ] {
                let Value::Object(object) = context.eval("new Uint8Array(1)").unwrap() else {
                    panic!("expected typed array");
                };
                let receiver = if same_receiver {
                    Value::Object(object.clone())
                } else {
                    Value::Object(runtime.new_object(None).unwrap())
                };
                let key = runtime.intern_property_key(key).unwrap();
                let value = context.eval(input).unwrap();
                let result = if wrapper {
                    runtime
                        .prepare_typed_array_set_in_realm(
                            Some(context.realm),
                            &object,
                            &key,
                            &value,
                            &receiver,
                        )
                        .unwrap()
                        .map(|step| step.finish_sync(&runtime, context.realm).unwrap())
                } else {
                    runtime
                        .try_typed_array_set_primitive(
                            context.realm,
                            &object,
                            &key,
                            &value,
                            &receiver,
                        )
                        .unwrap()
                };
                assert!(matches!(
                    (expected, result),
                    ("decline", None)
                        | ("stored" | "ignore", Some(NativeConversion::Value(true)))
                        | ("throw", Some(NativeConversion::Throw(_)))
                ));
                assert_eq!(
                    runtime.typed_array_read_index(&object, 0).unwrap(),
                    Some(Value::Int(if expected == "stored" { 1 } else { 0 }))
                );
            }
        }
    }

    #[cfg(feature = "stack-vm")]
    #[test]
    fn small_primitive_result_keeps_detached_conversion_and_rejects_object_inputs() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let Value::Object(buffer) = context.eval("globalThis.b=new ArrayBuffer(1); b").unwrap()
        else {
            panic!("expected buffer");
        };
        let Value::Object(object) = context.eval("new Uint8Array(b)").unwrap() else {
            panic!("expected typed array");
        };
        context.detach_array_buffer(&Value::Object(buffer)).unwrap();
        let key = runtime.intern_property_key("0").unwrap();
        let receiver = Value::Object(object.clone());
        assert!(matches!(
            runtime
                .try_typed_array_set_primitive(
                    context.realm,
                    &object,
                    &key,
                    &Value::Int(257),
                    &receiver
                )
                .unwrap(),
            Some(NativeConversion::Value(true))
        ));
        let bigint = context.eval("1n").unwrap();
        assert!(matches!(
            runtime
                .try_typed_array_set_primitive(context.realm, &object, &key, &bigint, &receiver)
                .unwrap(),
            Some(NativeConversion::Throw(_))
        ));
        let other_receiver = Value::Object(runtime.new_object(None).unwrap());
        assert!(matches!(
            runtime
                .try_typed_array_set_primitive(
                    context.realm,
                    &object,
                    &key,
                    &bigint,
                    &other_receiver
                )
                .unwrap(),
            Some(NativeConversion::Value(true))
        ));
        let value = Value::Object(runtime.new_object(None).unwrap());
        assert!(matches!(
            TypedWriteStep::set_primitive_result(&runtime, context.realm, &object, Some(0), &value),
            Err(RuntimeError::Invariant(
                "primitive typed Set received an object"
            ))
        ));
        assert!(matches!(
            runtime.try_typed_array_set_primitive(context.realm, &object, &key, &value, &receiver),
            Err(RuntimeError::Invariant(
                "primitive typed Set received an object"
            ))
        ));
        assert!(
            runtime
                .typed_array_read_index(&object, 0)
                .unwrap()
                .is_none()
        );
        assert!(runtime.0.state.borrow().active_frames.is_empty());
    }

    fn take_element(step: TypedWriteStep) -> TypedWriteResume {
        let TypedWriteStep::Element { resume, .. } = step else {
            panic!("expected element conversion")
        };
        resume
    }
    #[test]
    fn typed_write_request_owns_view_buffer_and_value_until_abandonment() {
        for define in [false, true] {
            let runtime = Runtime::new();
            let weak = std::rc::Rc::downgrade(&runtime.0);
            let mut context = runtime.new_context();
            let Value::Object(view) = context.eval("new Uint8Array(1)").unwrap() else {
                panic!("expected view")
            };
            let view_id = view.object_id();
            let buffer_id = runtime.typed_array_snapshot(&view).unwrap().buffer;
            let value = runtime.new_object(None).unwrap();
            let value_id = value.object_id();
            let step = if define {
                TypedWriteStep::define(
                    &runtime,
                    view,
                    0,
                    &OrdinaryPropertyDescriptor {
                        value: DescriptorField::Present(Value::Object(value)),
                        ..OrdinaryPropertyDescriptor::new()
                    },
                )
                .unwrap()
            } else {
                TypedWriteStep::set(&runtime, view, Some(0), Value::Object(value)).unwrap()
            };
            let resume = take_element(step);
            runtime.run_gc().unwrap();
            for id in [view_id, buffer_id, value_id] {
                assert!(runtime.0.state.borrow().heap.object(id).is_ok());
            }
            drop(resume);
            runtime.run_gc().unwrap();
            for id in [view_id, buffer_id, value_id] {
                assert!(runtime.0.state.borrow().heap.object(id).is_err());
            }
            drop(context);
            drop(runtime);
            assert!(weak.upgrade().is_none());
        }
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedWriteStep>() <= 64);
