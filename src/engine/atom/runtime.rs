use crate::engine::api::error::{Error, ErrorKind, NativeErrorMessage};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::atom::{Atom, AtomError, AtomKind, AtomSpelling};
use crate::engine::object::{PropertyKey, SymbolRef, WellKnownSymbol};
use crate::engine::value::{JsString, Value};

impl Runtime {
    /// Intern an exact ECMAScript string as a runtime-owned property key.
    pub fn intern_property_key_js_string(&self, text: &JsString) -> Result<PropertyKey, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .intern_property_key_js_string(text)?;
        Ok(PropertyKey::from_owned_atom(self.clone(), atom))
    }

    /// Intern a UTF-8 property spelling without losing the exact UTF-16 path
    /// used by language-level keys.
    pub fn intern_property_key(&self, text: &str) -> Result<PropertyKey, AtomError> {
        self.intern_property_key_js_string(&JsString::try_from_utf8(text)?)
    }

    /// Construct an already-normalized nonnegative integer index. This is not
    /// general Number-to-key conversion: large JS numbers use scientific spelling.
    pub(crate) fn property_key_for_index(&self, index: u64) -> Result<PropertyKey, AtomError> {
        if let Some(atom) = u32::try_from(index)
            .ok()
            .and_then(Atom::from_immediate_integer)
        {
            return Ok(PropertyKey::from_owned_atom(self.clone(), atom));
        }
        self.intern_property_key(&index.to_string())
    }

    /// Allocation-free numeric subset of ToPropertyKey, including numeric -0.
    /// Larger, negative and fractional numbers retain the full conversion path.
    pub(crate) fn immediate_numeric_property_key(&self, value: &Value) -> Option<PropertyKey> {
        let index = match value {
            Value::Int(value) => u32::try_from(*value).ok()?,
            Value::Float(value)
                if *value >= 0.0 && *value <= u32::MAX as f64 && value.fract() == 0.0 =>
            {
                *value as u32
            }
            _ => return None,
        };
        Atom::from_immediate_integer(index)
            .map(|atom| PropertyKey::from_owned_atom(self.clone(), atom))
    }

    /// Create a unique ECMAScript Symbol primitive.
    pub fn new_symbol(&self, description: Option<JsString>) -> Result<SymbolRef, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .new_symbol_js_string(description)?;
        Ok(SymbolRef::from_owned_atom(self.clone(), atom))
    }

    /// Return the runtime-global symbol for an exact registry key.
    pub fn symbol_for(&self, key: &JsString) -> Result<SymbolRef, AtomError> {
        let _operation = self.operation();
        let atom = self
            .0
            .state
            .borrow_mut()
            .atoms
            .intern_global_symbol_js_string(key)?;
        Ok(SymbolRef::from_owned_atom(self.clone(), atom))
    }

    /// Return one pinned, runtime-unique well-known symbol. It is deliberately
    /// absent from the `Symbol.for` registry.
    pub fn well_known_symbol(&self, symbol: WellKnownSymbol) -> SymbolRef {
        let _operation = self.operation();
        let atom = self.0.state.borrow().well_known_symbols[&symbol];
        SymbolRef::from_owned_atom(self.clone(), atom)
    }

    /// Implement the identity test used by `Symbol.keyFor`.
    pub fn symbol_key_for(&self, symbol: &SymbolRef) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        if !symbol.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("symbol"));
        }
        let state = self.0.state.borrow();
        if state.atoms.kind(symbol.atom())? == AtomKind::GlobalSymbol {
            Ok(Some(state.atoms.to_js_string(symbol.atom())?))
        } else {
            Ok(None)
        }
    }

    /// Return a Symbol's exact optional UTF-16 description.
    ///
    /// `None` is observably distinct from an explicitly empty description via
    /// `%Symbol.prototype%.description`, even though both stringify as
    /// `Symbol()`.
    pub fn symbol_description(&self, symbol: &SymbolRef) -> Result<Option<JsString>, RuntimeError> {
        let _operation = self.operation();
        if !symbol.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("symbol"));
        }
        let state = self.0.state.borrow();
        let info = state.atoms.resolve(symbol.atom())?;
        if !matches!(info.kind, AtomKind::Symbol | AtomKind::GlobalSymbol) {
            return Err(RuntimeError::Invariant(
                "SymbolRef did not refer to a public symbol atom",
            ));
        }
        match info.spelling {
            AtomSpelling::Text(text) => Ok(Some(text.clone())),
            AtomSpelling::NoDescription => Ok(None),
            AtomSpelling::Integer(_) => Err(RuntimeError::Invariant(
                "symbol atom had an immediate-integer spelling",
            )),
        }
    }

    /// Return the exact UTF-16 spelling or symbol description of a key.
    pub fn property_key_to_js_string(&self, key: &PropertyKey) -> Result<JsString, RuntimeError> {
        let _operation = self.operation();
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        Ok(self.0.state.borrow().atoms.to_js_string(key.atom())?)
    }

    pub(crate) fn native_atom_error(
        &self,
        kind: ErrorKind,
        prefix: &str,
        key: &PropertyKey,
        suffix: &str,
    ) -> Result<Error, RuntimeError> {
        let _operation = self.operation();
        if !key.belongs_to(self) {
            return Err(RuntimeError::WrongRuntime("property key"));
        }
        let mut message = NativeErrorMessage::new();
        message.push_utf8(prefix);
        self.0
            .state
            .borrow()
            .atoms
            .push_atom_get_str(key.atom(), &mut message)?;
        message.push_utf8(suffix);
        Ok(Error::from_native_message(kind, message))
    }
}

#[cfg(test)]
mod integer_key_tests {
    use super::*;

    #[test]
    fn integer_keys_preserve_immediate_boundary_and_exact_large_spelling() {
        let runtime = Runtime::new();
        for index in [
            0,
            1,
            2_147_483_647,
            2_147_483_648,
            u32::MAX as u64,
            9_007_199_254_740_991,
            u64::MAX,
        ] {
            let key = runtime.property_key_for_index(index).unwrap();
            assert_eq!(key.atom().is_immediate_integer(), index <= 2_147_483_647);
            assert_eq!(
                runtime.property_key_to_js_string(&key).unwrap(),
                JsString::try_from_utf8(&index.to_string()).unwrap()
            );
        }
        let zero = runtime
            .immediate_numeric_property_key(&Value::Float(-0.0))
            .unwrap();
        assert_eq!(zero, runtime.intern_property_key("0").unwrap());
        assert_ne!(zero, runtime.intern_property_key("-0").unwrap());
        for number in [f64::NAN, f64::INFINITY, -1.0, 0.5, 2_147_483_648.0, 1e21] {
            assert!(
                runtime
                    .immediate_numeric_property_key(&Value::Float(number))
                    .is_none()
            );
        }
    }
}
