//! Indexed `%TypedArray%.prototype` lookup and search algorithms.
//!
//! Pinned QuickJS snapshots the validated length before observable argument
//! coercion, then reads the live backing-view length before direct word
//! access. These methods deliberately do not reuse the generic Array kernels:
//! integer-indexed views have no holes or prototype lookup, and shrinking a
//! resizable buffer gives `includes(undefined)` a distinct observable result.

use crate::engine::builtins::native::{NativeFunctionId, TypedArrayNativeKind};
use crate::engine::{
    api::{error::NativeErrorKind, runtime::Runtime, runtime_error::RuntimeError},
    builtins::native::ArraySearchKind,
    heap::ContextId,
    object::ObjectRef,
    value::{Value, conversion::NativeConversion},
    vm::{
        Completion, ToPrimitiveHint,
        call::{NativeArguments, NativeInvocation},
    },
};

#[cfg(test)]
mod tests;

impl Runtime {
    pub(crate) fn call_typed_array_at(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedSearchStep::start(self, realm, TypedSearchKind::At, &invocation, arguments)?,
        )
    }

    pub(crate) fn call_typed_array_search(
        &self,
        realm: ContextId,
        kind: ArraySearchKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        finish(
            self,
            realm,
            TypedSearchStep::start(
                self,
                realm,
                TypedSearchKind::Search(kind),
                &invocation,
                arguments,
            )?,
        )
    }

    fn finish_typed_array_search(
        &self,
        realm: ContextId,
        kind: ArraySearchKind,
        target: ObjectRef,
        initial_length: i64,
        search: Value,
        from_index: Option<Value>,
    ) -> Result<Completion, RuntimeError> {
        let not_found = || match kind {
            ArraySearchKind::Includes => Value::Bool(false),
            ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => Value::Int(-1),
        };
        let (mut index, step) = match kind {
            ArraySearchKind::Includes | ArraySearchKind::IndexOf => {
                let index = if let Some(from_index) = &from_index {
                    match self.native_to_int64_clamp(
                        realm,
                        from_index,
                        0,
                        initial_length,
                        initial_length,
                    )? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                } else {
                    0
                };
                (index, 1)
            }
            ArraySearchKind::LastIndexOf => {
                let index = if let Some(from_index) = &from_index {
                    match self.native_to_int64_clamp(
                        realm,
                        from_index,
                        -1,
                        initial_length - 1,
                        initial_length,
                    )? {
                        NativeConversion::Value(value) => value,
                        NativeConversion::Throw(value) => {
                            return Ok(Completion::Throw(value));
                        }
                    }
                } else {
                    initial_length - 1
                };
                if index < 0 {
                    return Ok(Completion::Return(not_found()));
                }
                (index, -1)
            }
        };

        let current_length = i64::from(self.typed_array_state(&target)?.length);
        // QuickJS treats integer indices that disappeared during fromIndex
        // coercion as `undefined` only for includes. indexOf/lastIndexOf scan
        // direct machine words and therefore never observe that missing tail.
        if kind == ArraySearchKind::Includes
            && matches!(search, Value::Undefined)
            && initial_length > current_length
            && index < initial_length
        {
            return Ok(Completion::Return(Value::Bool(true)));
        }

        let length = initial_length.min(current_length);
        if length == 0 {
            return Ok(Completion::Return(not_found()));
        }
        let end = match kind {
            ArraySearchKind::Includes | ArraySearchKind::IndexOf => {
                index = index.min(length);
                length
            }
            ArraySearchKind::LastIndexOf => {
                index = index.min(length - 1);
                -1
            }
        };
        while index != end {
            let value = self
                .typed_array_read_index(
                    &target,
                    u64::try_from(index).map_err(|_| {
                        RuntimeError::Invariant("TypedArray search index was negative")
                    })?,
                )?
                .ok_or(RuntimeError::Invariant(
                    "stable TypedArray search range lost an element",
                ))?;
            let matches = match kind {
                ArraySearchKind::Includes => search.same_value_zero(&value),
                ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                    search.strict_equal(&value)
                }
            };
            if matches {
                return Ok(Completion::Return(match kind {
                    ArraySearchKind::Includes => Value::Bool(true),
                    ArraySearchKind::IndexOf | ArraySearchKind::LastIndexOf => {
                        Value::number(index as f64)
                    }
                }));
            }
            index += step;
        }
        Ok(Completion::Return(not_found()))
    }
}
#[derive(Clone, Copy)]
pub(crate) enum TypedSearchKind {
    At,
    Search(ArraySearchKind),
}
impl TypedSearchKind {
    pub(crate) fn for_target(target: NativeFunctionId) -> Option<Self> {
        Some(match target {
            NativeFunctionId::TypedArray(TypedArrayNativeKind::At) => Self::At,
            NativeFunctionId::TypedArray(TypedArrayNativeKind::Search(kind)) => Self::Search(kind),
            _ => return None,
        })
    }
}
pub(crate) enum TypedSearchStep {
    Complete(Completion),
    Primitive {
        value: Value,
        resume: TypedSearchResume,
    },
}
pub(crate) struct TypedSearchResume(Box<TypedSearchResumeState>);
impl std::ops::Deref for TypedSearchResume {
    type Target = TypedSearchResumeState;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}
impl std::ops::DerefMut for TypedSearchResume {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
const _: () = assert!(std::mem::size_of::<TypedSearchResume>() <= 8);
pub(crate) struct TypedSearchResumeState {
    realm: ContextId,
    target: ObjectRef,
    length: i64,
    kind: TypedSearchKind,
    search: Value,
}
impl TypedSearchStep {
    pub(crate) fn start(
        runtime: &Runtime,
        realm: ContextId,
        kind: TypedSearchKind,
        invocation: &NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Self, RuntimeError> {
        let NativeInvocation::Call { this_value } = invocation else {
            return Err(RuntimeError::Invariant(
                "TypedArray search received a constructor invocation",
            ));
        };
        let target = match runtime.require_typed_array(realm, this_value.clone())? {
            NativeConversion::Value(value) => value,
            NativeConversion::Throw(value) => return Ok(Self::Complete(Completion::Throw(value))),
        };
        let length = if matches!(kind, TypedSearchKind::At) {
            let initial = runtime.typed_array_state(&target)?;
            if initial.out_of_bounds {
                return Ok(Self::Complete(Completion::Throw(
                    runtime.new_native_error_jsvalue(
                        realm,
                        NativeErrorKind::Type,
                        "ArrayBuffer is detached",
                    )?,
                )));
            }
            i64::from(initial.length)
        } else {
            match runtime.typed_array_validated_length(realm, &target)? {
                NativeConversion::Value(value) => i64::from(value),
                NativeConversion::Throw(value) => {
                    return Ok(Self::Complete(Completion::Throw(value)));
                }
            }
        };
        match kind {
            TypedSearchKind::At => Ok(Self::Primitive {
                value: arguments
                    .readable
                    .first()
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray.at index argv was not padded",
                    ))?
                    .clone(),
                resume: TypedSearchResume(Box::new(TypedSearchResumeState {
                    realm,
                    target,
                    length,
                    kind,
                    search: Value::Undefined,
                })),
            }),
            TypedSearchKind::Search(search_kind) => {
                if length == 0 {
                    return Ok(Self::Complete(Completion::Return(match search_kind {
                        ArraySearchKind::Includes => Value::Bool(false),
                        _ => Value::Int(-1),
                    })));
                }
                let search = arguments
                    .readable
                    .first()
                    .ok_or(RuntimeError::Invariant(
                        "TypedArray search value argv was not padded",
                    ))?
                    .clone();
                if arguments.actual_arg_count <= 1 {
                    return Ok(Self::Complete(runtime.finish_typed_array_search(
                        realm,
                        search_kind,
                        target,
                        length,
                        search,
                        None,
                    )?));
                }
                Ok(Self::Primitive {
                    value: arguments
                        .readable
                        .get(1)
                        .ok_or(RuntimeError::Invariant(
                            "TypedArray search fromIndex argv was missing",
                        ))?
                        .clone(),
                    resume: TypedSearchResume(Box::new(TypedSearchResumeState {
                        realm,
                        target,
                        length,
                        kind,
                        search,
                    })),
                })
            }
        }
    }
}
impl TypedSearchResume {
    pub(crate) fn resume(
        self,
        runtime: &Runtime,
        result: Completion,
    ) -> Result<TypedSearchStep, RuntimeError> {
        let value = match result {
            Completion::Return(value) => value,
            Completion::Throw(value) => {
                return Ok(TypedSearchStep::Complete(Completion::Throw(value)));
            }
        };
        Ok(TypedSearchStep::Complete(match self.0.kind {
            TypedSearchKind::At => {
                let index = match runtime.native_to_int64_sat(self.0.realm, &value)? {
                    NativeConversion::Value(value) => value,
                    NativeConversion::Throw(value) => {
                        return Ok(TypedSearchStep::Complete(Completion::Throw(value)));
                    }
                };
                let index = if index < 0 {
                    self.0.length + index
                } else {
                    index
                };
                if index < 0 {
                    Completion::Return(Value::Undefined)
                } else {
                    Completion::Return(
                        runtime
                            .typed_array_read_index(
                                &self.0.target,
                                u64::try_from(index).map_err(|_| {
                                    RuntimeError::Invariant("TypedArray.at index overflowed u64")
                                })?,
                            )?
                            .unwrap_or(Value::Undefined),
                    )
                }
            }
            TypedSearchKind::Search(kind) => runtime.finish_typed_array_search(
                self.0.realm,
                kind,
                self.0.target,
                self.0.length,
                self.0.search,
                Some(value),
            )?,
        }))
    }
}
fn finish(
    runtime: &Runtime,
    realm: ContextId,
    mut step: TypedSearchStep,
) -> Result<Completion, RuntimeError> {
    loop {
        step = match step {
            TypedSearchStep::Complete(result) => return Ok(result),
            TypedSearchStep::Primitive { value, resume } => {
                let result = if matches!(value, Value::Object(_)) {
                    runtime.to_primitive(realm, value, ToPrimitiveHint::Number)?
                } else {
                    Completion::Return(value)
                };
                resume.resume(runtime, result)?
            }
        };
    }
}

// S11 all-domain protocol bound; inline completion stays allocation-free.
const _: () = assert!(std::mem::size_of::<TypedSearchStep>() <= 64);
