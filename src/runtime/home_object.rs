//! Bytecode-function HomeObject installation.
//!
//! QuickJS installs this hidden edge while publishing a method, after inferred
//! naming and before defining the public data/accessor property. Bytecode which
//! never references `super` keeps no HomeObject edge.

use super::*;

impl Runtime {
    pub(super) fn bytecode_function_home_object(
        &self,
        function: &ObjectRef,
    ) -> Result<Option<ObjectRef>, RuntimeError> {
        if !function.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("HomeObject function"));
        }
        let home_object = {
            let state = self.0.state.borrow();
            state
                .heap
                .bytecode_function_home_object(function.object_id())?
        };
        home_object
            .map(|home_object| ObjectRef::from_borrowed_handle(self.clone(), home_object))
            .transpose()
            .map_err(RuntimeError::Heap)
    }

    pub(super) fn install_object_literal_home_object(
        &self,
        function: &CallableRef,
        home_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        if !function.as_object().belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("method function"));
        }
        if !home_object.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("method HomeObject"));
        }

        let mut state = self.0.state.borrow_mut();
        let function_id = function.as_object().object_id();
        let bytecode = match &state.heap.object(function_id)?.payload {
            ObjectPayload::BytecodeFunction { bytecode, .. } => Some(*bytecode),
            ObjectPayload::NativeFunction { .. } | ObjectPayload::BoundFunction { .. } => None,
            ObjectPayload::Ordinary
            | ObjectPayload::Array { .. }
            | ObjectPayload::Arguments { .. }
            | ObjectPayload::ArrayIterator { .. }
            | ObjectPayload::ForInIterator(_)
            | ObjectPayload::Primitive(_)
            | ObjectPayload::Date(_)
            | ObjectPayload::RegExp(_)
            | ObjectPayload::RegExpStringIterator { .. }
            | ObjectPayload::GlobalObject { .. }
            | ObjectPayload::Error
            | ObjectPayload::StringIterator { .. } => {
                return Err(RuntimeError::Invariant(
                    "validated method callable no longer has a callable payload",
                ));
            }
        };
        let Some(bytecode) = bytecode else {
            return Ok(());
        };
        if !state
            .heap
            .function_bytecode(bytecode)?
            .metadata
            .needs_home_object
        {
            return Ok(());
        }

        let cleanup = state
            .heap
            .replace_bytecode_function_home_object(function_id, Some(home_object.object_id()))?;
        state.apply_cleanup(cleanup)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bytecode::{DefineMethodKind, Instruction};
    use crate::function::UnlinkedFunction;

    fn bytecode_callable(
        runtime: &Runtime,
        realm: ContextId,
        needs_home_object: bool,
    ) -> CallableRef {
        let function = runtime
            .publish_unlinked_function(
                realm,
                UnlinkedFunction::new(
                    vec![Instruction::Undefined, Instruction::Return],
                    Vec::new(),
                    FunctionMetadata {
                        max_stack: 1,
                        needs_home_object,
                        ..FunctionMetadata::default()
                    },
                ),
            )
            .unwrap();
        runtime.new_bytecode_closure(realm, &function).unwrap()
    }

    fn stored_home_object(runtime: &Runtime, callable: &CallableRef) -> Option<ObjectRef> {
        runtime
            .bytecode_function_home_object(callable.as_object())
            .unwrap()
    }

    #[test]
    fn object_literal_methods_install_home_object_only_when_metadata_requests_it() {
        let runtime = Runtime::new();
        let mut context = runtime.new_context();
        let literal = context.new_object().unwrap();

        for (name, kind) in [
            ("method", DefineMethodKind::Method),
            ("getter", DefineMethodKind::Getter),
            ("setter", DefineMethodKind::Setter),
        ] {
            let callable = bytecode_callable(&runtime, context.realm, true);
            let key = runtime.intern_property_key(name).unwrap();
            assert!(matches!(
                runtime
                    .define_object_literal_method(
                        context.realm,
                        &literal,
                        &key,
                        Value::Object(callable.as_object().clone()),
                        kind,
                        true,
                    )
                    .unwrap(),
                PropertyDefineOutcome::Defined(true)
            ));
            assert_eq!(
                stored_home_object(&runtime, &callable),
                Some(literal.clone())
            );
        }

        let ordinary = bytecode_callable(&runtime, context.realm, false);
        let key = runtime.intern_property_key("ordinary").unwrap();
        assert!(matches!(
            runtime
                .define_object_literal_method(
                    context.realm,
                    &literal,
                    &key,
                    Value::Object(ordinary.as_object().clone()),
                    DefineMethodKind::Method,
                    true,
                )
                .unwrap(),
            PropertyDefineOutcome::Defined(true)
        ));
        assert_eq!(stored_home_object(&runtime, &ordinary), None);
    }
}
