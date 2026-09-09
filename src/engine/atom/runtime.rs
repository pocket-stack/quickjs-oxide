use crate::engine::api::error::{Error, ErrorKind, NativeErrorMessage};
use crate::engine::api::runtime::Runtime;
use crate::engine::api::runtime_error::RuntimeError;

use crate::engine::atom::{AtomError, AtomKind, AtomSpelling};
use crate::engine::object::{PropertyKey, SymbolRef, WellKnownSymbol};
use crate::engine::value::JsString;

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
