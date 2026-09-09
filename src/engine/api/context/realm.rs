//! Access this realm’s global objects and intrinsic roots.

use super::*;

impl Context {
    /// Return this realm's `%Object.prototype%` root.
    pub fn object_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .object_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's genuine empty `%Array.prototype%` root.
    pub fn array_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .array_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%Function.prototype%` root.
    pub fn function_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%IteratorPrototype%` root beneath the public
    /// `Iterator`, Iterator Helpers, and `Iterator.concat` intrinsic graph.
    pub fn iterator_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .iterator_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's `%StringIteratorPrototype%` root.
    pub fn string_iterator_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .string_iterator_prototype;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return this realm's boxed-+0 `%Number.prototype%` root.
    pub fn number_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Number)
    }

    /// Return this realm's boxed-false `%Boolean.prototype%` root.
    pub fn boolean_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Boolean)
    }

    /// Return this realm's branded-empty partial `%String.prototype%` root.
    pub fn string_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::String)
    }

    /// Return this realm's ordinary `%Symbol.prototype%` root.
    pub fn symbol_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::Symbol)
    }

    /// Return this realm's ordinary `%BigInt.prototype%` root.
    pub fn bigint_prototype(&self) -> Result<ObjectRef, RuntimeError> {
        self.runtime
            .primitive_prototype_for_realm(self.realm, PrimitiveKind::BigInt)
    }

    /// Return this realm's `%Function%` constructor root.
    pub fn function_constructor(&self) -> Result<CallableRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .function_constructor
            .ok_or(RuntimeError::Invariant("realm has no Function constructor"))?;
        Ok(CallableRef::from_validated_object(
            ObjectRef::from_borrowed_handle(self.runtime.clone(), object)?,
        ))
    }

    /// Return this realm's global object root.
    pub fn global_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }

    /// Return the null-prototype object used for global lexical bindings.
    pub fn global_var_object(&self) -> Result<ObjectRef, RuntimeError> {
        let object = self
            .runtime
            .0
            .state
            .borrow()
            .heap
            .context(self.realm)?
            .global_var_object;
        Ok(ObjectRef::from_borrowed_handle(
            self.runtime.clone(),
            object,
        )?)
    }
}
