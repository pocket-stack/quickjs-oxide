//! Pinned QuickJS 2026-06-04 `RegExp` intrinsic.
//!
//! The pure matcher lives in [`crate::regexp`].  This module owns the
//! observable ECMAScript shell around it: realm-local constructor/prototype
//! identities, derived allocation, accessors, `lastIndex`, and match result
//! objects. RegExp literals, the legacy `compile` method, `@@replace`,
//! `@@match`, `@@search`, and `@@split` are linked; `RegExp.escape` and the
//! remaining Symbol protocols stay separate parity slices.

mod compile;
mod constructor;
mod exec;
mod match_protocol;
mod prototype;
mod replace;
mod search;
mod split;

use crate::heap::{RegExpFlagKind, RegExpNativeKind, RegExpRealmData};

use super::*;

impl Runtime {
    /// Install the linked subset of pinned `js_regexp_funcs` and
    /// `js_regexp_proto_funcs`.
    ///
    /// This routine creates `%RegExp.prototype%` as an ordinary object, then
    /// atomically attaches the constructor, prototype, and initial instance
    /// shape to `ContextData` after their public graph is complete.
    pub(in crate::runtime) fn initialize_regexp_intrinsic(
        &self,
        realm: ContextId,
        function_prototype: &ObjectRef,
        object_prototype: &ObjectRef,
        global_object: &ObjectRef,
    ) -> Result<(), RuntimeError> {
        let regexp_prototype = self.new_object(Some(object_prototype))?;
        // Preserve pinned source-table order.  OrdinaryOwnPropertyKeys later
        // performs its own integer/String/Symbol grouping.
        for (kind, property_name, getter_name) in [
            (RegExpNativeKind::Flags, "flags", "get flags"),
            (RegExpNativeKind::Source, "source", "get source"),
        ] {
            self.define_native_builtin_getter_on(
                &regexp_prototype,
                function_prototype,
                realm,
                NativeFunctionId::RegExp(kind),
                property_name,
                getter_name,
            )?;
        }
        for kind in [
            RegExpFlagKind::Global,
            RegExpFlagKind::IgnoreCase,
            RegExpFlagKind::Multiline,
            RegExpFlagKind::DotAll,
            RegExpFlagKind::Unicode,
            RegExpFlagKind::UnicodeSets,
            RegExpFlagKind::Sticky,
            RegExpFlagKind::HasIndices,
        ] {
            self.define_native_builtin_getter_on(
                &regexp_prototype,
                function_prototype,
                realm,
                NativeFunctionId::RegExp(RegExpNativeKind::Flag(kind)),
                regexp_flag_property_name(kind),
                regexp_flag_getter_name(kind),
            )?;
        }
        for (kind, name, length, min_readable_args) in [
            (RegExpNativeKind::Exec, "exec", 1, 1),
            (RegExpNativeKind::Compile, "compile", 2, 2),
            (RegExpNativeKind::Test, "test", 1, 1),
            (RegExpNativeKind::ToString, "toString", 0, 0),
        ] {
            self.define_native_builtin_auto_init(
                &regexp_prototype,
                realm,
                NativeFunctionId::RegExp(kind),
                name,
                length,
                min_readable_args,
            )?;
        }
        let replace_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Replace));
        self.define_native_builtin_auto_init_with_key(
            &regexp_prototype,
            realm,
            &replace_key,
            NativeFunctionId::RegExp(RegExpNativeKind::Replace),
            "[Symbol.replace]",
            2,
            2,
            PropertyFlags::data(true, false, true),
        )?;
        let match_key = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Match));
        self.define_native_builtin_auto_init_with_key(
            &regexp_prototype,
            realm,
            &match_key,
            NativeFunctionId::RegExp(RegExpNativeKind::Match),
            "[Symbol.match]",
            1,
            1,
            PropertyFlags::data(true, false, true),
        )?;
        let search = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Search));
        self.define_native_builtin_auto_init_with_key(
            &regexp_prototype,
            realm,
            &search,
            NativeFunctionId::RegExp(RegExpNativeKind::Search),
            "[Symbol.search]",
            1,
            1,
            PropertyFlags::data(true, false, true),
        )?;
        let split = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Split));
        self.define_native_builtin_auto_init_with_key(
            &regexp_prototype,
            realm,
            &split,
            NativeFunctionId::RegExp(RegExpNativeKind::Split),
            "[Symbol.split]",
            2,
            2,
            PropertyFlags::data(true, false, true),
        )?;

        let constructor_kind = RegExpNativeKind::Constructor;
        let constructor = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::RegExp(constructor_kind),
            2,
            "RegExp",
            2,
        )?;

        let species_kind = RegExpNativeKind::Species;
        let species_getter = self.new_native_builtin(
            function_prototype,
            realm,
            NativeFunctionId::RegExp(species_kind),
            0,
            "get [Symbol.species]",
            0,
        )?;
        let species = PropertyKey::from(self.well_known_symbol(WellKnownSymbol::Species));
        if !self.define_own_property(
            constructor.as_object(),
            &species,
            &OrdinaryPropertyDescriptor {
                get: DescriptorField::Present(AccessorValue::Callable(species_getter)),
                set: DescriptorField::Present(AccessorValue::Undefined),
                enumerable: DescriptorField::Present(false),
                configurable: DescriptorField::Present(true),
                ..OrdinaryPropertyDescriptor::new()
            },
        )? {
            return Err(RuntimeError::Invariant(
                "RegExp species definition was rejected",
            ));
        }

        // JS_NewCConstructor publishes the global before installing the two
        // constructor relationship properties.
        self.define_function_data_property(
            global_object,
            "RegExp",
            Value::Object(constructor.as_object().clone()),
            true,
            true,
        )?;
        self.define_constructor_relationship(&constructor, &regexp_prototype)?;

        // QuickJS retains a realm-local canonical shape for literal-created
        // RegExp objects.  Constructors with a custom derived prototype use a
        // shape with the same property layout but that explicit prototype.
        let last_index = self.intern_property_key("lastIndex")?;
        let entries = [ShapeEntry {
            atom: last_index.atom(),
            flags: PropertyFlags::data(true, false, false),
        }];
        let object_shape = self
            .0
            .state
            .borrow_mut()
            .get_or_create_shape(Some(regexp_prototype.object_id()), &entries)?;
        let mut state = self.0.state.borrow_mut();
        let attached = state.heap.attach_regexp_intrinsics(
            realm,
            RegExpRealmData {
                prototype: regexp_prototype.object_id(),
                constructor: constructor.as_object().object_id(),
                object_shape,
            },
            last_index.atom(),
        );
        // `get_or_create_shape` returned one construction reference. The
        // Context owns the durable edge only when `attached` succeeded; on
        // failure this release reclaims the unpublished shape instead.
        let cleanup = state.heap.release_shape(object_shape)?;
        state.apply_cleanup(cleanup)?;
        attached?;
        Ok(())
    }

    /// Typed top-level dispatcher for every native identity installed above.
    pub(in crate::runtime) fn call_regexp_native(
        &self,
        realm: ContextId,
        kind: RegExpNativeKind,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        match kind {
            RegExpNativeKind::Constructor => {
                self.call_regexp_constructor(realm, invocation, arguments)
            }
            RegExpNativeKind::Species => self.call_regexp_species(invocation),
            RegExpNativeKind::Source | RegExpNativeKind::Flags | RegExpNativeKind::Flag(_) => {
                self.call_regexp_accessor(realm, kind, invocation)
            }
            RegExpNativeKind::Exec | RegExpNativeKind::Test => {
                self.call_regexp_exec_native(realm, kind, invocation, arguments)
            }
            RegExpNativeKind::Compile => self.call_regexp_compile(realm, invocation, arguments),
            RegExpNativeKind::ToString => self.call_regexp_to_string(realm, invocation),
            RegExpNativeKind::Replace => {
                self.call_regexp_symbol_replace(realm, invocation, arguments)
            }
            RegExpNativeKind::Match => self.call_regexp_symbol_match(realm, invocation, arguments),
            RegExpNativeKind::Search => {
                self.call_regexp_symbol_search(realm, invocation, arguments)
            }
            RegExpNativeKind::Split => self.call_regexp_symbol_split(realm, invocation, arguments),
        }
    }
}

const fn regexp_flag_property_name(kind: RegExpFlagKind) -> &'static str {
    match kind {
        RegExpFlagKind::HasIndices => "hasIndices",
        RegExpFlagKind::Global => "global",
        RegExpFlagKind::IgnoreCase => "ignoreCase",
        RegExpFlagKind::Multiline => "multiline",
        RegExpFlagKind::DotAll => "dotAll",
        RegExpFlagKind::Unicode => "unicode",
        RegExpFlagKind::UnicodeSets => "unicodeSets",
        RegExpFlagKind::Sticky => "sticky",
    }
}

const fn regexp_flag_getter_name(kind: RegExpFlagKind) -> &'static str {
    match kind {
        RegExpFlagKind::HasIndices => "get hasIndices",
        RegExpFlagKind::Global => "get global",
        RegExpFlagKind::IgnoreCase => "get ignoreCase",
        RegExpFlagKind::Multiline => "get multiline",
        RegExpFlagKind::DotAll => "get dotAll",
        RegExpFlagKind::Unicode => "get unicode",
        RegExpFlagKind::UnicodeSets => "get unicodeSets",
        RegExpFlagKind::Sticky => "get sticky",
    }
}
