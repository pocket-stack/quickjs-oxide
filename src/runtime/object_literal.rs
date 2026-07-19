//! Object-literal method publication.
//!
//! QuickJS keeps this operation separate from ordinary field definition
//! because it must infer the closure name and choose a data or accessor
//! descriptor without exposing either decision to JavaScript code.

use super::*;
use crate::bytecode::DefineMethodKind;

impl Runtime {
    pub(super) fn define_object_literal_method(
        &self,
        realm: ContextId,
        object: &ObjectRef,
        key: &PropertyKey,
        function: Value,
        kind: DefineMethodKind,
        enumerable: bool,
    ) -> Result<PropertyDefineOutcome, RuntimeError> {
        let callable = self.callable_from_value(function)?;
        let name = self.object_literal_method_name(key, kind)?;
        let function = Value::Object(callable.as_object().clone());
        self.define_object_name(&function, &name)?;

        let descriptor = match kind {
            DefineMethodKind::Method => OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(function),
                writable: DescriptorField::Present(true),
                enumerable: DescriptorField::Present(enumerable),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
            DefineMethodKind::Getter => OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(callable)),
                // `Absent` is required here: a following setter with the same
                // key must merge with this getter instead of erasing it.
                set: DescriptorField::Absent,
                enumerable: DescriptorField::Present(enumerable),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
            DefineMethodKind::Setter => OrdinaryPropertyDescriptor {
                // Symmetric accessor merging for `{ set x(v) {}, get x() {} }`.
                get: DescriptorField::Absent,
                set: DescriptorField::Present(AccessorValue::Callable(callable)),
                enumerable: DescriptorField::Present(enumerable),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        };

        self.define_own_property_in_realm(Some(realm), object, key, &descriptor)
    }

    fn object_literal_method_name(
        &self,
        key: &PropertyKey,
        kind: DefineMethodKind,
    ) -> Result<JsString, RuntimeError> {
        self.validate_object_literal_key(key)?;
        let key_name = {
            let state = self.0.state.borrow();
            match state.atoms.property_key_kind(key.atom())? {
                PropertyKeyKind::String => state.atoms.to_js_string(key.atom())?,
                PropertyKeyKind::Symbol => match state.atoms.resolve(key.atom())?.spelling {
                    // A Symbol without a description gives methods the empty
                    // name; an explicitly empty description remains `[]`.
                    AtomSpelling::NoDescription => JsString::from_static(""),
                    AtomSpelling::Text(description) => JsString::from_static("[")
                        .try_concat(description)?
                        .try_concat(&JsString::from_static("]"))?,
                    AtomSpelling::Integer(_) => {
                        return Err(RuntimeError::Invariant(
                            "symbol property key had an integer spelling",
                        ));
                    }
                },
                PropertyKeyKind::Private => {
                    return Err(RuntimeError::Invariant(
                        "object literal method used a private property key",
                    ));
                }
            }
        };

        let prefix = match kind {
            DefineMethodKind::Method => return Ok(key_name),
            DefineMethodKind::Getter => "get ",
            DefineMethodKind::Setter => "set ",
        };
        Ok(JsString::from_static(prefix).try_concat(&key_name)?)
    }

    fn validate_object_literal_key(&self, key: &PropertyKey) -> Result<(), RuntimeError> {
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("object literal method key"));
        }
        Ok(())
    }
}
