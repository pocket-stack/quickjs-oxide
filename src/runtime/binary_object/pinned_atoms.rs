//! Release-pinned QuickJS atom manifest used by the binary-object format.
//!
//! These entries mirror `quickjs-atom.h` from QuickJS 2026-06-04 exactly.
//! Their numeric identities are part of the bytecode/binary-object ABI: atom
//! zero is reserved, this manifest occupies `1..=242`, and runtime-created
//! atoms start at [`FIRST_DYNAMIC_ATOM`].

use super::wire::WireString;

/// Number of predefined atoms in the pinned QuickJS release.
pub(in crate::runtime) const PINNED_ATOM_COUNT: u32 = 242;

/// First atom ID available to a binary object's dynamic atom table.
pub(in crate::runtime) const FIRST_DYNAMIC_ATOM: u32 = PINNED_ATOM_COUNT + 1;

const LAST_STRING_ATOM: u32 = 228;
const PRIVATE_ATOM: u32 = 229;

/// QuickJS's three predefined-atom identity classes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime) enum PinnedAtomKind {
    String,
    Private,
    Symbol,
}

/// A validated nonzero ID into the release-pinned atom manifest.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(in crate::runtime) struct PinnedAtomId(u32);

impl PinnedAtomId {
    /// Validate a raw QuickJS atom ID against the pinned manifest.
    #[must_use]
    pub(in crate::runtime) const fn from_raw(raw: u32) -> Option<Self> {
        if raw > 0 && raw <= PINNED_ATOM_COUNT {
            Some(Self(raw))
        } else {
            None
        }
    }

    /// Return the exact QuickJS atom ID.
    #[must_use]
    pub(in crate::runtime) const fn raw(self) -> u32 {
        self.0
    }

    /// Return the identity class assigned by `quickjs-atom.h`.
    #[must_use]
    pub(in crate::runtime) const fn kind(self) -> PinnedAtomKind {
        if self.0 <= LAST_STRING_ATOM {
            PinnedAtomKind::String
        } else if self.0 == PRIVATE_ATOM {
            PinnedAtomKind::Private
        } else {
            PinnedAtomKind::Symbol
        }
    }

    /// Return the string spelling or symbol description from the manifest.
    #[must_use]
    pub(in crate::runtime) const fn spelling(self) -> &'static str {
        PINNED_ATOM_SPELLINGS[self.0 as usize - 1]
    }
}

/// Find a predefined string atom by exact UTF-16 code-unit value.
///
/// QuickJS accepts both narrow and wide encodings for these ASCII spellings.
/// Private and symbol atoms are identity-bearing, so they are intentionally
/// excluded even when their descriptions equal an ordinary string atom.
#[must_use]
pub(in crate::runtime) fn lookup_string(value: &WireString) -> Option<PinnedAtomId> {
    PINNED_ATOM_SPELLINGS[..LAST_STRING_ATOM as usize]
        .iter()
        .position(|spelling| wire_string_equals_ascii(value, spelling.as_bytes()))
        .and_then(|index| u32::try_from(index + 1).ok())
        .and_then(PinnedAtomId::from_raw)
}

fn wire_string_equals_ascii(value: &WireString, ascii: &[u8]) -> bool {
    match value {
        WireString::Narrow(bytes) => bytes.as_ref() == ascii,
        WireString::Wide(units) => {
            units.len() == ascii.len()
                && units
                    .iter()
                    .zip(ascii)
                    .all(|(&unit, &byte)| unit == u16::from(byte))
        }
    }
}

// Allocation order copied from QuickJS 2026-06-04 `quickjs-atom.h`.
const PINNED_ATOM_SPELLINGS: [&str; PINNED_ATOM_COUNT as usize] = [
    "null",
    "false",
    "true",
    "if",
    "else",
    "return",
    "var",
    "this",
    "delete",
    "void",
    "typeof",
    "new",
    "in",
    "instanceof",
    "do",
    "while",
    "for",
    "break",
    "continue",
    "switch",
    "case",
    "default",
    "throw",
    "try",
    "catch",
    "finally",
    "function",
    "debugger",
    "with",
    "class",
    "const",
    "enum",
    "export",
    "extends",
    "import",
    "super",
    "implements",
    "interface",
    "let",
    "package",
    "private",
    "protected",
    "public",
    "static",
    "yield",
    "await",
    "",
    "keys",
    "size",
    "length",
    "fileName",
    "lineNumber",
    "columnNumber",
    "message",
    "cause",
    "errors",
    "stack",
    "name",
    "toString",
    "toLocaleString",
    "valueOf",
    "eval",
    "prototype",
    "constructor",
    "configurable",
    "writable",
    "enumerable",
    "value",
    "get",
    "set",
    "of",
    "__proto__",
    "undefined",
    "number",
    "boolean",
    "string",
    "object",
    "symbol",
    "integer",
    "unknown",
    "arguments",
    "callee",
    "caller",
    "<eval>",
    "<ret>",
    "<var>",
    "<arg_var>",
    "<with>",
    "lastIndex",
    "target",
    "index",
    "input",
    "defineProperties",
    "apply",
    "join",
    "concat",
    "split",
    "construct",
    "getPrototypeOf",
    "setPrototypeOf",
    "isExtensible",
    "preventExtensions",
    "has",
    "deleteProperty",
    "defineProperty",
    "getOwnPropertyDescriptor",
    "ownKeys",
    "add",
    "done",
    "next",
    "values",
    "source",
    "flags",
    "global",
    "unicode",
    "raw",
    "rawJSON",
    "new.target",
    "this.active_func",
    "<home_object>",
    "<computed_field>",
    "<static_computed_field>",
    "<class_fields_init>",
    "<brand>",
    "#constructor",
    "as",
    "from",
    "meta",
    "*default*",
    "*",
    "Module",
    "then",
    "resolve",
    "reject",
    "promise",
    "proxy",
    "revoke",
    "async",
    "exec",
    "groups",
    "indices",
    "status",
    "reason",
    "globalThis",
    "bigint",
    "-0",
    "Infinity",
    "-Infinity",
    "NaN",
    "hasIndices",
    "ignoreCase",
    "multiline",
    "dotAll",
    "sticky",
    "unicodeSets",
    "not-equal",
    "timed-out",
    "ok",
    "toISOString",
    "alphabet",
    "lastChunkHandling",
    "omitPadding",
    "toJSON",
    "maxByteLength",
    "Object",
    "Array",
    "Error",
    "Number",
    "String",
    "Boolean",
    "Symbol",
    "Arguments",
    "Math",
    "JSON",
    "Date",
    "Function",
    "GeneratorFunction",
    "ForInIterator",
    "RegExp",
    "ArrayBuffer",
    "SharedArrayBuffer",
    "Uint8ClampedArray",
    "Int8Array",
    "Uint8Array",
    "Int16Array",
    "Uint16Array",
    "Int32Array",
    "Uint32Array",
    "BigInt64Array",
    "BigUint64Array",
    "Float16Array",
    "Float32Array",
    "Float64Array",
    "DataView",
    "BigInt",
    "WeakRef",
    "FinalizationRegistry",
    "Map",
    "Set",
    "WeakMap",
    "WeakSet",
    "Iterator",
    "Iterator Helper",
    "Iterator Concat",
    "Iterator Wrap",
    "Map Iterator",
    "Set Iterator",
    "Array Iterator",
    "String Iterator",
    "RegExp String Iterator",
    "Generator",
    "Proxy",
    "Promise",
    "PromiseResolveFunction",
    "PromiseRejectFunction",
    "AsyncFunction",
    "AsyncFunctionResolve",
    "AsyncFunctionReject",
    "AsyncGeneratorFunction",
    "AsyncGenerator",
    "EvalError",
    "RangeError",
    "ReferenceError",
    "SyntaxError",
    "TypeError",
    "URIError",
    "InternalError",
    "AggregateError",
    "<brand>",
    "Symbol.toPrimitive",
    "Symbol.iterator",
    "Symbol.match",
    "Symbol.matchAll",
    "Symbol.replace",
    "Symbol.search",
    "Symbol.split",
    "Symbol.toStringTag",
    "Symbol.isConcatSpreadable",
    "Symbol.hasInstance",
    "Symbol.species",
    "Symbol.unscopables",
    "Symbol.asyncIterator",
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::WellKnownSymbol;

    fn id(raw: u32) -> PinnedAtomId {
        PinnedAtomId::from_raw(raw).expect("test ID is inside the pinned manifest")
    }

    #[test]
    fn manifest_count_boundaries_and_sentinels_match_quickjs() {
        assert_eq!(PINNED_ATOM_SPELLINGS.len(), 242);
        assert_eq!(PINNED_ATOM_COUNT, 242);
        assert_eq!(FIRST_DYNAMIC_ATOM, 243);
        assert_eq!(PinnedAtomId::from_raw(0), None);
        assert_eq!(PinnedAtomId::from_raw(FIRST_DYNAMIC_ATOM), None);

        let sentinels = [
            (1, PinnedAtomKind::String, "null"),
            (47, PinnedAtomKind::String, ""),
            (124, PinnedAtomKind::String, "<brand>"),
            (228, PinnedAtomKind::String, "AggregateError"),
            (229, PinnedAtomKind::Private, "<brand>"),
            (230, PinnedAtomKind::Symbol, "Symbol.toPrimitive"),
            (242, PinnedAtomKind::Symbol, "Symbol.asyncIterator"),
        ];
        for (raw, kind, spelling) in sentinels {
            let atom = id(raw);
            assert_eq!(atom.raw(), raw);
            assert_eq!(atom.kind(), kind);
            assert_eq!(atom.spelling(), spelling);
        }
    }

    #[test]
    fn narrow_and_wide_strings_lookup_by_code_unit_value() {
        for raw in 1..=LAST_STRING_ATOM {
            let expected = id(raw);
            let spelling = expected.spelling();
            let narrow = WireString::Narrow(spelling.as_bytes().to_vec().into_boxed_slice());
            let wide = WireString::Wide(
                spelling
                    .as_bytes()
                    .iter()
                    .map(|&byte| u16::from(byte))
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            );
            assert_eq!(lookup_string(&narrow), Some(expected));
            assert_eq!(lookup_string(&wide), Some(expected));
        }

        assert_eq!(
            lookup_string(&WireString::Narrow(b"missing".to_vec().into_boxed_slice(),)),
            None
        );
        assert_eq!(lookup_string(&WireString::Wide(Box::from([0x100]))), None);
    }

    #[test]
    fn well_known_symbol_order_and_descriptions_match_manifest() {
        for (offset, symbol) in WellKnownSymbol::ALL.into_iter().enumerate() {
            let raw = 230 + u32::try_from(offset).expect("thirteen symbols fit in u32");
            let atom = id(raw);
            assert_eq!(atom.kind(), PinnedAtomKind::Symbol);
            assert_eq!(atom.spelling(), symbol.description());
        }
    }

    #[test]
    fn duplicate_brand_spellings_keep_distinct_identities() {
        let ordinary = id(124);
        let private = id(229);
        assert_eq!(ordinary.spelling(), private.spelling());
        assert_ne!(ordinary, private);
        assert_eq!(ordinary.kind(), PinnedAtomKind::String);
        assert_eq!(private.kind(), PinnedAtomKind::Private);

        let narrow_brand = WireString::Narrow(Box::from(*b"<brand>"));
        let wide_brand = WireString::Wide(Box::from([
            u16::from(b'<'),
            u16::from(b'b'),
            u16::from(b'r'),
            u16::from(b'a'),
            u16::from(b'n'),
            u16::from(b'd'),
            u16::from(b'>'),
        ]));
        assert_eq!(lookup_string(&narrow_brand), Some(ordinary));
        assert_eq!(lookup_string(&wide_brand), Some(ordinary));
    }
}
