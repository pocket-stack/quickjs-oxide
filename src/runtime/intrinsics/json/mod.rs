//! Pinned QuickJS `JSON` intrinsic algorithms.
//!
//! The global object table is installed in the exact `js_json_funcs` order so
//! later stringify and Raw JSON slices do not have to mutate observable own-
//! key order.  Strict parsing and reviver internalization live in separate
//! modules because their allocation and abrupt-completion boundaries are
//! independently observable.

use super::super::*;

mod parse;
mod raw;
mod reviver;
mod stringify;

#[cfg(test)]
mod tests;

impl Runtime {
    /// Install QuickJS's lazy global `JSON` object after `%RegExp%`.
    pub(in crate::runtime) fn initialize_json_intrinsic(
        &self,
        realm: ContextId,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let key = self.intern_property_key("JSON")?;
        self.store_property_slot(
            global_object,
            &key,
            PropertyFlags::data(true, false, true),
            PropertySlot::AutoInit(AutoInitProperty::Json { realm }),
        )
    }

    /// Materialize the complete pinned `js_json_funcs` property table.
    ///
    /// Stringify and Raw JSON keep honest typed frontiers until their bounded
    /// milestones land, but their callable identities are reserved now so the
    /// final object graph and own-key order need no migration.
    pub(in crate::runtime) fn instantiate_json_intrinsic(
        &self,
        realm: ContextId,
    ) -> Result<ObjectRef, RuntimeError> {
        self.0.state.borrow().heap.context(realm)?;
        let json = self.new_ordinary_object_in_realm(realm)?;
        for (kind, name, length) in [
            (JsonNativeKind::IsRawJson, "isRawJSON", 1),
            (JsonNativeKind::Parse, "parse", 2),
            (JsonNativeKind::RawJson, "rawJSON", 1),
            (JsonNativeKind::Stringify, "stringify", 3),
        ] {
            self.define_native_builtin_auto_init(
                &json,
                realm,
                NativeFunctionId::Json(kind),
                name,
                length,
                length,
            )?;
        }

        let to_string_tag = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::ToStringTag));
        if !self.define_own_property(
            &json,
            &to_string_tag,
            &OrdinaryPropertyDescriptor {
                value: DescriptorField::Present(Value::String(JsString::from_static("JSON"))),
                writable: DescriptorField::Present(false),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "JSON toStringTag definition was rejected",
            ));
        }
        Ok(json)
    }

    pub(in crate::runtime) fn call_json_native(
        &self,
        realm: ContextId,
        kind: JsonNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "JSON method did not receive a generic invocation",
            ));
        };
        match kind {
            JsonNativeKind::IsRawJson => self.call_json_is_raw_json(arguments),
            JsonNativeKind::Parse => self.call_json_parse(realm, arguments),
            JsonNativeKind::RawJson => self.call_json_raw_json(realm, arguments),
            JsonNativeKind::Stringify => self.call_json_stringify(realm, arguments),
        }
    }
}
