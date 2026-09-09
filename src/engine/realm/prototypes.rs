use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;
use crate::engine::builtins::native::PrimitiveKind;
use crate::engine::heap::ContextId;
use crate::engine::object::ObjectRef;

impl Runtime {
    pub(crate) fn global_object_for_realm(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        let global_object = self.0.state.borrow().heap.context(realm)?.global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.clone(),
            global_object,
        )?)
    }

    pub(crate) fn primitive_prototype_for_realm(
        &self,
        realm: ContextId,
        kind: PrimitiveKind,
    ) -> Result<ObjectRef, RuntimeError> {
        let prototype = self
            .0
            .state
            .borrow()
            .heap
            .context(realm)?
            .primitive_prototypes[kind.index()]
        .ok_or(RuntimeError::Invariant(
            "primitive prototype is not implemented in this realm",
        ))?;
        Ok(ObjectRef::from_borrowed_handle(self.clone(), prototype)?)
    }
}
