//! QuickJS-style property-key atoms.
//!
//! `QuickJS` represents small, non-negative integer property names directly in
//! the atom value and keeps every other atom in a runtime-local intern table.
//! This module preserves that split while making ownership and invalid handles
//! explicit:
//!
//! - atom `0` is reserved for [`Atom::NULL`];
//! - canonical integers in `0..=2^31 - 1` use the high-bit immediate encoding;
//! - ordinary strings and global-symbol keys are interned independently;
//! - local symbols and private symbols are unique, even with equal descriptions;
//! - table-backed atoms have explicit [`AtomTable::retain`] and
//!   [`AtomTable::release`] operations;
//! - pinned atoms, null, and immediate integers are permanent and uncounted.
//!
//! `QuickJS` reuses freed table slots through `atom_free_index`. This rewrite
//! preserves that bounded-space behavior while adding a table domain and slot
//! generation to the safe Rust handle. The raw `u32` can therefore be reused
//! for future C-ABI compatibility without letting a stale or cross-runtime
//! [`Atom`] alias the new occupant.

use std::collections::HashMap;
use std::error::Error;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::error::NativeErrorMessage;
use crate::value::{JsString, JsStringError};

/// The tag used by `QuickJS` for immediate, non-negative integer atoms.
pub const ATOM_TAG_INT: u32 = 1 << 31;

/// Largest integer which can be represented directly in an [`Atom`].
pub const ATOM_MAX_INT: u32 = ATOM_TAG_INT - 1;

/// Largest table index admitted by `QuickJS`'s atom table.
///
/// Bit 31 belongs to immediate integers. `QuickJS` also caps table indices to 30
/// bits, which this implementation follows even though bit 30 is otherwise
/// unused by the public encoding.
pub const ATOM_MAX_TABLE_INDEX: u32 = (1 << 30) - 1;

static NEXT_ATOM_TABLE_ID: AtomicU64 = AtomicU64::new(1);

/// A checked runtime-local atom identifier.
///
/// Values are meaningful only with the [`AtomTable`] which created them.
/// Copying a table-backed atom does *not* retain it; callers which create a new
/// owning reference must call [`AtomTable::retain`], mirroring `QuickJS`'s
/// `JS_DupAtom` contract.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Atom {
    raw: u32,
    generation: u32,
    table_id: u64,
}

impl Atom {
    /// Sentinel used for "no atom". It is never a string or symbol atom.
    pub const NULL: Self = Self {
        raw: 0,
        generation: 0,
        table_id: 0,
    };

    /// Reconstruct an atom from its raw representation.
    ///
    /// This is an unbranded C-style adapter value. Immediate integers remain
    /// usable, but a table-backed value cannot pass safe [`AtomTable`]
    /// validation without the matching domain and generation.
    #[must_use]
    pub const fn from_raw(raw: u32) -> Self {
        Self {
            raw,
            generation: 0,
            table_id: 0,
        }
    }

    /// Return the compact raw representation.
    #[must_use]
    pub const fn raw(self) -> u32 {
        self.raw
    }

    /// Whether this is the reserved null sentinel.
    #[must_use]
    pub const fn is_null(self) -> bool {
        self.raw == 0
    }

    /// Whether this atom directly encodes a non-negative integer property.
    #[must_use]
    pub const fn is_immediate_integer(self) -> bool {
        self.raw & ATOM_TAG_INT != 0
    }

    /// Decode an immediate integer atom.
    #[must_use]
    pub const fn immediate_integer(self) -> Option<u32> {
        if self.is_immediate_integer() {
            Some(self.raw & !ATOM_TAG_INT)
        } else {
            None
        }
    }

    /// Construct an immediate integer atom when `value` is within `QuickJS`'s
    /// direct-encoding range.
    #[must_use]
    pub const fn from_immediate_integer(value: u32) -> Option<Self> {
        if value <= ATOM_MAX_INT {
            Some(Self {
                raw: ATOM_TAG_INT | value,
                generation: 0,
                table_id: 0,
            })
        } else {
            None
        }
    }
}

impl fmt::Debug for Atom {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_null() {
            f.write_str("Atom::NULL")
        } else if let Some(value) = self.immediate_integer() {
            write!(f, "Atom::Integer({value})")
        } else {
            f.debug_struct("Atom")
                .field("raw", &self.raw)
                .field("generation", &self.generation)
                .field("table_id", &self.table_id)
                .finish()
        }
    }
}

/// Internal atom classification needed to implement ECMAScript property keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AtomKind {
    /// An interned string property name (immediate integers also have this kind).
    String,
    /// A `Symbol.for`-style symbol, interned by its registry key.
    GlobalSymbol,
    /// A unique local symbol.
    Symbol,
    /// A unique private name/brand.
    Private,
}

impl AtomKind {
    /// Public ECMAScript-facing category. Global and local symbols both map to
    /// `Symbol`; private names remain separate.
    #[must_use]
    pub const fn property_key_kind(self) -> PropertyKeyKind {
        match self {
            Self::String => PropertyKeyKind::String,
            Self::GlobalSymbol | Self::Symbol => PropertyKeyKind::Symbol,
            Self::Private => PropertyKeyKind::Private,
        }
    }
}

/// The three categories exposed by `QuickJS`'s atom-kind query.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PropertyKeyKind {
    String,
    Symbol,
    Private,
}

/// A non-allocating reverse lookup of an atom's spelling or description.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomSpelling<'a> {
    /// A directly encoded integer property name.
    Integer(u32),
    /// A string atom, registry key, or present symbol description.
    Text(&'a JsString),
    /// A local/private symbol created without a description.
    NoDescription,
}

/// Metadata returned by [`AtomTable::resolve`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AtomInfo<'a> {
    pub kind: AtomKind,
    pub spelling: AtomSpelling<'a>,
    /// `None` means that the atom is permanent and is not reference counted.
    pub ref_count: Option<u32>,
    /// True for immediate integers and explicitly pinned table entries.
    pub is_permanent: bool,
}

/// Result of dropping one explicit atom reference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// Null, an immediate integer, or a pinned table entry; no state changed.
    Permanent,
    /// The atom remains live with the given number of owning references.
    Retained(u32),
    /// The final reference was released and the payload was reclaimed.
    Removed,
}

/// Errors from validated atom-table operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AtomError {
    /// The null sentinel was used where a real property key was required.
    NullAtom,
    /// The atom is forged, was released, or is from a different table.
    UnknownAtom(Atom),
    /// Retaining the atom would overflow its explicit reference count.
    RefCountOverflow(Atom),
    /// The monotonic table ID space has been exhausted.
    TableFull,
    /// A dynamic atom spelling could not be represented as a valid String.
    String(JsStringError),
}

impl fmt::Display for AtomError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NullAtom => f.write_str("the null atom is not a property key"),
            Self::UnknownAtom(atom) => write!(f, "unknown or released atom {atom:?}"),
            Self::RefCountOverflow(atom) => {
                write!(f, "reference count overflow while retaining {atom:?}")
            }
            Self::TableFull => f.write_str("atom table ID space is exhausted"),
            Self::String(error) => error.fmt(f),
        }
    }
}

impl Error for AtomError {}

impl From<JsStringError> for AtomError {
    fn from(error: JsStringError) -> Self {
        Self::String(error)
    }
}

#[derive(Debug)]
struct Entry {
    kind: AtomKind,
    text: Option<JsString>,
    ref_count: u32,
    pinned: bool,
}

impl Entry {
    fn info(&self) -> AtomInfo<'_> {
        AtomInfo {
            kind: self.kind,
            spelling: match self.text.as_ref() {
                Some(text) => AtomSpelling::Text(text),
                None => AtomSpelling::NoDescription,
            },
            ref_count: (!self.pinned).then_some(self.ref_count),
            is_permanent: self.pinned,
        }
    }
}

/// A runtime-local QuickJS-style atom table.
///
/// `AtomTable` intentionally needs `&mut self` for operations which change
/// ownership. A complete table belongs to one single-threaded runtime.
#[derive(Debug)]
pub struct AtomTable {
    table_id: u64,
    /// Index zero is permanently empty.
    entries: Vec<Option<Entry>>,
    generations: Vec<u32>,
    free: Vec<u32>,
    strings: HashMap<JsString, Atom>,
    global_symbols: HashMap<JsString, Atom>,
    live_table_atoms: usize,
}

impl Default for AtomTable {
    fn default() -> Self {
        Self::new()
    }
}

impl AtomTable {
    /// Create an empty table with atom zero reserved.
    #[must_use]
    pub fn new() -> Self {
        let table_id = NEXT_ATOM_TABLE_ID.fetch_add(1, Ordering::Relaxed);
        assert_ne!(table_id, 0, "atom table domain ID space exhausted");
        Self {
            table_id,
            entries: vec![None],
            generations: vec![0],
            free: Vec::new(),
            strings: HashMap::new(),
            global_symbols: HashMap::new(),
            live_table_atoms: 0,
        }
    }

    /// Create a table and pin a caller-selected set of runtime/static atoms.
    ///
    /// This is the safe-Rust equivalent of `QuickJS` preloading its generated atom
    /// list. Keeping the list at the runtime construction boundary avoids
    /// silently claiming that a partial built-in list is complete.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if any
    /// spelling exceeds [`JsString::MAX_LEN`] UTF-16 code units, or
    /// [`AtomError::TableFull`] if the caller supplies more atoms than fit in
    /// the 30-bit table ID space.
    pub fn with_static_atoms<I, S>(atoms: I) -> Result<Self, AtomError>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut table = Self::new();
        for atom in atoms {
            table.intern_static(atom.as_ref())?;
        }
        Ok(table)
    }

    /// Number of currently live table-backed atoms.
    ///
    /// Null and immediate integer atoms are not stored and are not counted.
    #[must_use]
    pub const fn len(&self) -> usize {
        self.live_table_atoms
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.live_table_atoms == 0
    }

    /// Intern a string property name and return one owning reference.
    ///
    /// Canonical decimal strings in `0..=2^31 - 1` are returned as immediate
    /// atoms. Leading-zero spellings such as `"01"` are ordinary strings.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if
    /// `text` exceeds [`JsString::MAX_LEN`] UTF-16 code units,
    /// [`AtomError::RefCountOverflow`] if an existing atom cannot be retained,
    /// or [`AtomError::TableFull`] if a new atom cannot be assigned.
    pub fn intern(&mut self, text: &str) -> Result<Atom, AtomError> {
        self.intern_js_string(&JsString::try_from_utf8(text)?)
    }

    /// Intern an exact ECMAScript UTF-16 string, including lone surrogates.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::RefCountOverflow`] if an existing atom cannot be
    /// retained, or [`AtomError::TableFull`] if a new atom cannot be assigned.
    pub fn intern_js_string(&mut self, text: &JsString) -> Result<Atom, AtomError> {
        let text = text.linearize();
        if let Some(atom) =
            parse_canonical_u32_js_string(&text).and_then(Atom::from_immediate_integer)
        {
            return Ok(atom);
        }

        if let Some(&atom) = self.strings.get(&text) {
            return self.retain(atom);
        }

        let atom = self.allocate(AtomKind::String, Some(text.clone()), false)?;
        self.strings.insert(text, atom);
        Ok(atom)
    }

    /// Alias which makes the property-key role explicit at call sites.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::intern`].
    pub fn intern_property_key(&mut self, text: &str) -> Result<Atom, AtomError> {
        self.intern(text)
    }

    /// Exact UTF-16 variant of [`Self::intern_property_key`].
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::intern_js_string`].
    pub fn intern_property_key_js_string(&mut self, text: &JsString) -> Result<Atom, AtomError> {
        self.intern_js_string(text)
    }

    /// Intern and permanently pin a string atom.
    ///
    /// Pinning an existing atom makes all current and future handles permanent;
    /// later `release` calls become no-ops, as for `QuickJS` predefined atoms.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if
    /// `text` exceeds [`JsString::MAX_LEN`] UTF-16 code units, or
    /// [`AtomError::TableFull`] if a new atom cannot be assigned.
    pub fn intern_static(&mut self, text: &str) -> Result<Atom, AtomError> {
        self.intern_static_js_string(&JsString::try_from_utf8(text)?)
    }

    /// Exact UTF-16 variant of [`Self::intern_static`].
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::TableFull`] if a new atom cannot be assigned.
    pub fn intern_static_js_string(&mut self, text: &JsString) -> Result<Atom, AtomError> {
        let text = text.linearize();
        if let Some(atom) =
            parse_canonical_u32_js_string(&text).and_then(Atom::from_immediate_integer)
        {
            return Ok(atom);
        }

        if let Some(&atom) = self.strings.get(&text) {
            self.pin(atom)?;
            return Ok(atom);
        }

        let atom = self.allocate(AtomKind::String, Some(text.clone()), true)?;
        self.strings.insert(text, atom);
        Ok(atom)
    }

    /// Create or retrieve a `Symbol.for`-style atom by registry key.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if
    /// `key` exceeds [`JsString::MAX_LEN`] UTF-16 code units,
    /// [`AtomError::RefCountOverflow`] if an existing symbol cannot be retained,
    /// or [`AtomError::TableFull`] if a new symbol cannot be assigned.
    pub fn intern_global_symbol(&mut self, key: &str) -> Result<Atom, AtomError> {
        self.intern_global_symbol_js_string(&JsString::try_from_utf8(key)?)
    }

    /// Exact UTF-16 variant of [`Self::intern_global_symbol`].
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::RefCountOverflow`] if an existing symbol cannot be
    /// retained, or [`AtomError::TableFull`] if a new symbol cannot be assigned.
    pub fn intern_global_symbol_js_string(&mut self, key: &JsString) -> Result<Atom, AtomError> {
        let key = key.linearize();
        if let Some(&atom) = self.global_symbols.get(&key) {
            return self.retain(atom);
        }

        let atom = self.allocate(AtomKind::GlobalSymbol, Some(key.clone()), false)?;
        self.global_symbols.insert(key, atom);
        Ok(atom)
    }

    /// Create or retrieve and permanently pin a global symbol.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if
    /// `key` exceeds [`JsString::MAX_LEN`] UTF-16 code units, or
    /// [`AtomError::TableFull`] if a new symbol cannot be assigned.
    pub fn intern_static_global_symbol(&mut self, key: &str) -> Result<Atom, AtomError> {
        self.intern_static_global_symbol_js_string(&JsString::try_from_utf8(key)?)
    }

    /// Exact UTF-16 variant of [`Self::intern_static_global_symbol`].
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::TableFull`] if a new symbol cannot be assigned.
    pub fn intern_static_global_symbol_js_string(
        &mut self,
        key: &JsString,
    ) -> Result<Atom, AtomError> {
        let key = key.linearize();
        if let Some(&atom) = self.global_symbols.get(&key) {
            self.pin(atom)?;
            return Ok(atom);
        }

        let atom = self.allocate(AtomKind::GlobalSymbol, Some(key.clone()), true)?;
        self.global_symbols.insert(key, atom);
        Ok(atom)
    }

    /// Create a unique local symbol with an optional description.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if the
    /// provided description exceeds [`JsString::MAX_LEN`] UTF-16 code units, or
    /// [`AtomError::TableFull`] if the symbol cannot be assigned an ID.
    pub fn new_symbol(&mut self, description: Option<&str>) -> Result<Atom, AtomError> {
        self.new_symbol_js_string(description.map(JsString::try_from_utf8).transpose()?)
    }

    /// Create a unique symbol with an exact UTF-16 description.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::TableFull`] if the symbol cannot be assigned an ID.
    pub fn new_symbol_js_string(
        &mut self,
        description: Option<JsString>,
    ) -> Result<Atom, AtomError> {
        self.allocate(AtomKind::Symbol, description, false)
    }

    /// Create a pinned unique symbol for a runtime intrinsic such as
    /// `Symbol.iterator`. It is deliberately not entered in the
    /// `Symbol.for` registry.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if the
    /// provided description exceeds [`JsString::MAX_LEN`] UTF-16 code units, or
    /// [`AtomError::TableFull`] if the symbol cannot be assigned an ID.
    pub fn new_static_symbol(&mut self, description: Option<&str>) -> Result<Atom, AtomError> {
        self.new_static_symbol_js_string(description.map(JsString::try_from_utf8).transpose()?)
    }

    /// Exact UTF-16 variant of [`Self::new_static_symbol`].
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::TableFull`] if the symbol cannot be assigned an ID.
    pub fn new_static_symbol_js_string(
        &mut self,
        description: Option<JsString>,
    ) -> Result<Atom, AtomError> {
        self.allocate(AtomKind::Symbol, description, true)
    }

    /// Create a unique private name/brand with an optional description.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::String`] wrapping [`JsStringError::TooLong`] if the
    /// provided description exceeds [`JsString::MAX_LEN`] UTF-16 code units, or
    /// [`AtomError::TableFull`] if the private name cannot be assigned.
    pub fn new_private_symbol(&mut self, description: Option<&str>) -> Result<Atom, AtomError> {
        self.new_private_symbol_js_string(description.map(JsString::try_from_utf8).transpose()?)
    }

    /// Create a unique private name with an exact UTF-16 description.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::TableFull`] if the private name cannot be assigned.
    pub fn new_private_symbol_js_string(
        &mut self,
        description: Option<JsString>,
    ) -> Result<Atom, AtomError> {
        self.allocate(AtomKind::Private, description, false)
    }

    /// Construct an atom from a `u32`, using `QuickJS`'s immediate boundary.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::intern`] when `value` is too large for
    /// an immediate atom.
    pub fn intern_u32(&mut self, value: u32) -> Result<Atom, AtomError> {
        match Atom::from_immediate_integer(value) {
            Some(atom) => Ok(atom),
            None => self.intern(&value.to_string()),
        }
    }

    /// Construct an atom from an `i64`. Negative and out-of-range values become
    /// interned decimal strings, matching `QuickJS`'s integer-to-atom behavior.
    ///
    /// # Errors
    ///
    /// Returns the same errors as [`Self::intern`] when `value` cannot use the
    /// immediate representation.
    pub fn intern_i64(&mut self, value: i64) -> Result<Atom, AtomError> {
        if let Some(atom) = u32::try_from(value)
            .ok()
            .and_then(Atom::from_immediate_integer)
        {
            return Ok(atom);
        }
        self.intern(&value.to_string())
    }

    /// Add one explicit owning reference.
    ///
    /// Permanent atoms are returned unchanged without a counter update.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::UnknownAtom`] for an invalid or released table ID,
    /// or [`AtomError::RefCountOverflow`] if its counter is already maximal.
    pub fn retain(&mut self, atom: Atom) -> Result<Atom, AtomError> {
        if atom.is_null() || atom.is_immediate_integer() {
            return Ok(atom);
        }

        let entry = self.entry_mut(atom)?;
        if entry.pinned {
            return Ok(atom);
        }
        entry.ref_count = entry
            .ref_count
            .checked_add(1)
            .ok_or(AtomError::RefCountOverflow(atom))?;
        Ok(atom)
    }

    /// Drop one explicit owning reference.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::UnknownAtom`] for an invalid or already released
    /// table ID.
    pub fn release(&mut self, atom: Atom) -> Result<ReleaseOutcome, AtomError> {
        if atom.is_null() || atom.is_immediate_integer() {
            return Ok(ReleaseOutcome::Permanent);
        }

        let index = self.valid_index(atom)?;
        let entry = self.entries[index]
            .as_mut()
            .ok_or(AtomError::UnknownAtom(atom))?;
        if entry.pinned {
            return Ok(ReleaseOutcome::Permanent);
        }

        if entry.ref_count == 0 {
            return Err(AtomError::UnknownAtom(atom));
        }
        entry.ref_count -= 1;
        if entry.ref_count != 0 {
            return Ok(ReleaseOutcome::Retained(entry.ref_count));
        }

        let entry = self.entries[index]
            .take()
            .ok_or(AtomError::UnknownAtom(atom))?;
        match entry.kind {
            AtomKind::String => {
                if let Some(text) = entry.text {
                    self.strings.remove(&text);
                }
            }
            AtomKind::GlobalSymbol => {
                if let Some(key) = entry.text {
                    self.global_symbols.remove(&key);
                }
            }
            AtomKind::Symbol | AtomKind::Private => {}
        }
        self.live_table_atoms -= 1;
        if let Some(generation) = self.generations[index].checked_add(1) {
            self.generations[index] = generation;
            self.free.push(atom.raw());
        }
        Ok(ReleaseOutcome::Removed)
    }

    /// Make a live table-backed atom permanent.
    ///
    /// Null and immediate integer atoms are already permanent, so pinning them
    /// succeeds as a no-op.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::UnknownAtom`] for an invalid or released table ID.
    pub fn pin(&mut self, atom: Atom) -> Result<(), AtomError> {
        if atom.is_null() || atom.is_immediate_integer() {
            return Ok(());
        }
        let entry = self.entry_mut(atom)?;
        entry.pinned = true;
        entry.ref_count = 0;
        Ok(())
    }

    /// Return validated atom metadata without allocating.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::NullAtom`] for the null sentinel or
    /// [`AtomError::UnknownAtom`] for an invalid or released table ID.
    pub fn resolve(&self, atom: Atom) -> Result<AtomInfo<'_>, AtomError> {
        if atom.is_null() {
            return Err(AtomError::NullAtom);
        }
        if let Some(value) = atom.immediate_integer() {
            return Ok(AtomInfo {
                kind: AtomKind::String,
                spelling: AtomSpelling::Integer(value),
                ref_count: None,
                is_permanent: true,
            });
        }
        Ok(self.entry(atom)?.info())
    }

    /// Return the detailed internal kind of a live property-key atom.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::resolve`].
    pub fn kind(&self, atom: Atom) -> Result<AtomKind, AtomError> {
        Ok(self.resolve(atom)?.kind)
    }

    /// Return the public string/symbol/private category used for key ordering.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::resolve`].
    pub fn property_key_kind(&self, atom: Atom) -> Result<PropertyKeyKind, AtomError> {
        Ok(self.kind(atom)?.property_key_kind())
    }

    /// Convert an atom back to its property spelling or symbol description.
    ///
    /// A symbol without a description converts to the empty string, matching
    /// `QuickJS`'s internal atom-to-string operation. Use [`Self::resolve`] when
    /// the distinction between an absent and an empty description matters.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::resolve`].
    pub fn to_string(&self, atom: Atom) -> Result<String, AtomError> {
        Ok(self.to_js_string(atom)?.to_utf8_lossy())
    }

    /// Convert an atom to its exact ECMAScript UTF-16 spelling.
    ///
    /// # Errors
    ///
    /// Returns the same validation errors as [`Self::resolve`].
    pub fn to_js_string(&self, atom: Atom) -> Result<JsString, AtomError> {
        match self.resolve(atom)?.spelling {
            AtomSpelling::Integer(value) => Ok(JsString::try_from_utf8(&value.to_string())?),
            AtomSpelling::Text(text) => Ok(text.clone()),
            AtomSpelling::NoDescription => Ok(JsString::from_static("")),
        }
    }

    /// Append the C-string argument produced by QuickJS
    /// `JS_AtomGetStr(..., char buf[64], 64)` to a native Error formatter.
    /// Integer and null atoms use their dedicated spellings, while table-backed
    /// atoms preserve the exact flat narrow/wide representation selected when
    /// they were interned.
    pub(crate) fn push_atom_get_str(
        &self,
        atom: Atom,
        output: &mut NativeErrorMessage,
    ) -> Result<(), AtomError> {
        if atom.is_null() {
            output.push_utf8("<null>");
            return Ok(());
        }
        match self.resolve(atom)?.spelling {
            AtomSpelling::Integer(value) => output.push_utf8(&value.to_string()),
            AtomSpelling::Text(text) => text.push_atom_get_str_to(output),
            AtomSpelling::NoDescription => {}
        }
        Ok(())
    }

    /// If `atom` is an ECMAScript array index, return its numeric value.
    ///
    /// The valid range is `0..=2^32 - 2`. Values through `2^31 - 1` are
    /// immediate; larger indices are canonical decimal string atoms.
    ///
    /// # Errors
    ///
    /// Returns [`AtomError::NullAtom`] for the null sentinel or
    /// [`AtomError::UnknownAtom`] for an invalid or released table ID.
    pub fn array_index(&self, atom: Atom) -> Result<Option<u32>, AtomError> {
        if atom.is_null() {
            return Err(AtomError::NullAtom);
        }
        if let Some(value) = atom.immediate_integer() {
            return Ok(Some(value));
        }

        let entry = self.entry(atom)?;
        if entry.kind != AtomKind::String {
            return Ok(None);
        }
        let value = entry.text.as_ref().and_then(parse_canonical_u32_js_string);
        Ok(value.filter(|value| *value != u32::MAX))
    }

    /// Whether a value is currently valid in this table.
    ///
    /// Null is a sentinel rather than a live atom. All well-formed immediate
    /// integers are live without table storage.
    #[must_use]
    pub fn is_live(&self, atom: Atom) -> bool {
        if atom.is_null() {
            return false;
        }
        if atom.is_immediate_integer() {
            return true;
        }
        self.valid_index(atom).is_ok()
    }

    fn allocate(
        &mut self,
        kind: AtomKind,
        text: Option<JsString>,
        pinned: bool,
    ) -> Result<Atom, AtomError> {
        let text = text.map(|text| text.linearize());
        debug_assert!(kind != AtomKind::String || text.is_some());
        debug_assert!(kind != AtomKind::GlobalSymbol || text.is_some());

        let index = if let Some(index) = self.free.pop() {
            index
        } else {
            let index = u32::try_from(self.entries.len()).map_err(|_| AtomError::TableFull)?;
            if index > ATOM_MAX_TABLE_INDEX {
                return Err(AtomError::TableFull);
            }
            self.entries.push(None);
            self.generations.push(1);
            index
        };
        let index_usize = index as usize;
        if self.entries[index_usize].is_some() {
            return Err(AtomError::TableFull);
        }
        let atom = Atom {
            raw: index,
            generation: self.generations[index_usize],
            table_id: self.table_id,
        };
        self.entries[index_usize] = Some(Entry {
            kind,
            text,
            ref_count: u32::from(!pinned),
            pinned,
        });
        self.live_table_atoms += 1;
        Ok(atom)
    }

    fn valid_index(&self, atom: Atom) -> Result<usize, AtomError> {
        if atom.is_null() || atom.is_immediate_integer() {
            return Err(AtomError::UnknownAtom(atom));
        }
        if atom.table_id != self.table_id {
            return Err(AtomError::UnknownAtom(atom));
        }
        let index = atom.raw() as usize;
        if self.generations.get(index).copied() != Some(atom.generation) {
            return Err(AtomError::UnknownAtom(atom));
        }
        match self.entries.get(index) {
            Some(Some(_)) => Ok(index),
            _ => Err(AtomError::UnknownAtom(atom)),
        }
    }

    fn entry(&self, atom: Atom) -> Result<&Entry, AtomError> {
        let index = self.valid_index(atom)?;
        Ok(self.entries[index]
            .as_ref()
            .expect("valid_index guarantees a live entry"))
    }

    fn entry_mut(&mut self, atom: Atom) -> Result<&mut Entry, AtomError> {
        let index = self.valid_index(atom)?;
        Ok(self.entries[index]
            .as_mut()
            .expect("valid_index guarantees a live entry"))
    }
}

/// Parse `QuickJS`'s canonical non-negative integer property spelling.
///
/// This deliberately recognizes `2^32 - 1` too: it is a canonical integer
/// property name, but [`AtomTable::array_index`] excludes it per ECMAScript.
#[must_use]
pub fn parse_canonical_u32(text: &str) -> Option<u32> {
    if text.len() > 10 {
        return None;
    }
    parse_canonical_u32_js_string(&JsString::try_from_utf8(text).ok()?)
}

fn parse_canonical_u32_js_string(text: &JsString) -> Option<u32> {
    if text.is_empty() || text.len() > 10 {
        return None;
    }
    let mut units = text.utf16_units();
    let first = units.next()?;
    if first == u16::from(b'0') {
        return (text.len() == 1).then_some(0);
    }
    if !(u16::from(b'1')..=u16::from(b'9')).contains(&first) {
        return None;
    }

    let mut value = u32::from(first - u16::from(b'0'));
    for unit in units {
        if !(u16::from(b'0')..=u16::from(b'9')).contains(&unit) {
            return None;
        }
        value = value
            .checked_mul(10)?
            .checked_add(u32::from(unit - u16::from(b'0')))?;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_string_interning_retains_one_atom() {
        let mut atoms = AtomTable::new();
        let first = atoms.intern("hello").unwrap();
        let second = atoms.intern("hello").unwrap();

        assert_eq!(first, second);
        assert_eq!(atoms.len(), 1);
        assert_eq!(atoms.resolve(first).unwrap().ref_count, Some(2));
        assert_eq!(atoms.release(first), Ok(ReleaseOutcome::Retained(1)));
        assert_eq!(atoms.to_string(second).unwrap(), "hello");
        assert_eq!(atoms.release(second), Ok(ReleaseOutcome::Removed));
        assert!(matches!(
            atoms.resolve(first),
            Err(AtomError::UnknownAtom(atom)) if atom == first
        ));
    }

    #[test]
    fn quickjs_integer_and_array_index_boundaries_are_preserved() {
        let mut atoms = AtomTable::new();

        for (text, value) in [("0", 0), ("1", 1), ("2147483647", ATOM_MAX_INT)] {
            let atom = atoms.intern(text).unwrap();
            assert!(atom.is_immediate_integer(), "{text}");
            assert_eq!(atom.immediate_integer(), Some(value));
            assert_eq!(atoms.array_index(atom), Ok(Some(value)));
            assert_eq!(atoms.to_string(atom).unwrap(), text);
            assert_eq!(atoms.release(atom), Ok(ReleaseOutcome::Permanent));
        }

        let first_non_immediate = atoms.intern("2147483648").unwrap();
        assert!(!first_non_immediate.is_immediate_integer());
        assert_eq!(atoms.array_index(first_non_immediate), Ok(Some(1 << 31)));

        let largest_index = atoms.intern("4294967294").unwrap();
        assert!(!largest_index.is_immediate_integer());
        assert_eq!(atoms.array_index(largest_index), Ok(Some(u32::MAX - 1)));

        let excluded_index = atoms.intern("4294967295").unwrap();
        assert_eq!(atoms.array_index(excluded_index), Ok(None));
        assert_eq!(parse_canonical_u32("4294967295"), Some(u32::MAX));

        let overflow = atoms.intern("4294967296").unwrap();
        assert_eq!(atoms.array_index(overflow), Ok(None));
        assert_eq!(parse_canonical_u32("4294967296"), None);

        for text in ["", "00", "01", "+1", "-0", "1.0", "１２"] {
            assert_eq!(parse_canonical_u32(text), None, "{text}");
            let atom = atoms.intern(text).unwrap();
            assert!(!atom.is_immediate_integer(), "{text}");
            assert_eq!(atoms.array_index(atom), Ok(None), "{text}");
        }

        let from_u32 = atoms.intern_u32(1 << 31).unwrap();
        assert_eq!(from_u32, first_non_immediate);
        let negative = atoms.intern_i64(-1).unwrap();
        assert_eq!(atoms.to_string(negative).unwrap(), "-1");
        assert_eq!(atoms.array_index(negative), Ok(None));
    }

    #[test]
    fn released_slots_are_reused_without_aliasing_stale_atoms() {
        let mut atoms = AtomTable::new();
        let old = atoms.intern("ephemeral").unwrap();
        assert_eq!(atoms.release(old), Ok(ReleaseOutcome::Removed));

        let new = atoms.intern("replacement").unwrap();
        assert_ne!(old, new);
        assert_eq!(old.raw(), new.raw());
        assert!(!atoms.is_live(old));
        assert!(atoms.is_live(new));
        assert!(matches!(
            atoms.retain(old),
            Err(AtomError::UnknownAtom(atom)) if atom == old
        ));

        let reinterned = atoms.intern("ephemeral").unwrap();
        assert_ne!(old, reinterned);
        assert_eq!(atoms.to_string(reinterned).unwrap(), "ephemeral");
    }

    #[test]
    fn local_and_private_symbols_are_unique_while_global_symbols_intern() {
        let mut atoms = AtomTable::new();
        let string = atoms.intern("token").unwrap();
        let symbol_a = atoms.new_symbol(Some("token")).unwrap();
        let symbol_b = atoms.new_symbol(Some("token")).unwrap();
        let private_a = atoms.new_private_symbol(Some("token")).unwrap();
        let private_b = atoms.new_private_symbol(Some("token")).unwrap();
        let global_a = atoms.intern_global_symbol("token").unwrap();
        let global_b = atoms.intern_global_symbol("token").unwrap();

        assert_ne!(string, symbol_a);
        assert_ne!(symbol_a, symbol_b);
        assert_ne!(private_a, private_b);
        assert_eq!(global_a, global_b);
        assert_eq!(atoms.kind(string), Ok(AtomKind::String));
        assert_eq!(atoms.kind(symbol_a), Ok(AtomKind::Symbol));
        assert_eq!(atoms.kind(private_a), Ok(AtomKind::Private));
        assert_eq!(atoms.kind(global_a), Ok(AtomKind::GlobalSymbol));
        assert_eq!(
            atoms.property_key_kind(global_a),
            Ok(PropertyKeyKind::Symbol)
        );
        assert_eq!(atoms.array_index(symbol_a), Ok(None));
        assert_eq!(atoms.resolve(global_a).unwrap().ref_count, Some(2));
    }

    #[test]
    fn reverse_lookup_preserves_missing_symbol_descriptions() {
        let mut atoms = AtomTable::new();
        let integer = atoms.intern("42").unwrap();
        let string = atoms.intern("answer").unwrap();
        let empty_description = atoms.new_symbol(Some("")).unwrap();
        let no_description = atoms.new_symbol(None).unwrap();
        let answer_text = JsString::from_static("answer");
        let empty_text = JsString::from_static("");

        assert_eq!(
            atoms.resolve(integer).unwrap().spelling,
            AtomSpelling::Integer(42)
        );
        assert_eq!(
            atoms.resolve(string).unwrap().spelling,
            AtomSpelling::Text(&answer_text)
        );
        assert_eq!(
            atoms.resolve(empty_description).unwrap().spelling,
            AtomSpelling::Text(&empty_text)
        );
        assert_eq!(
            atoms.resolve(no_description).unwrap().spelling,
            AtomSpelling::NoDescription
        );
        assert_eq!(atoms.to_string(integer).unwrap(), "42");
        assert_eq!(atoms.to_string(no_description).unwrap(), "");
        assert_eq!(atoms.resolve(Atom::NULL), Err(AtomError::NullAtom));
    }

    #[test]
    fn atom_get_str_dispatches_null_integer_and_missing_description() {
        let mut atoms = AtomTable::new();
        let integer = atoms.intern("2147483647").unwrap();
        let no_description = atoms.new_symbol(None).unwrap();

        for (atom, expected) in [
            (Atom::NULL, "<null>"),
            (integer, "2147483647"),
            (no_description, ""),
        ] {
            let mut message = NativeErrorMessage::new();
            message.push_utf8("[");
            atoms.push_atom_get_str(atom, &mut message).unwrap();
            message.push_utf8("]");
            assert_eq!(
                message.to_js_string().unwrap().to_utf8_lossy(),
                format!("[{expected}]")
            );
        }
    }

    #[test]
    fn atom_spelling_preserves_lone_utf16_surrogates() {
        let mut atoms = AtomTable::new();
        let high_surrogate = JsString::try_from_utf16([0xd800]).unwrap();
        let other_surrogate = JsString::try_from_utf16([0xd801]).unwrap();
        let replacement = JsString::from_static("\u{fffd}");

        let first = atoms.intern_js_string(&high_surrogate).unwrap();
        let duplicate = atoms.intern_js_string(&high_surrogate).unwrap();
        let second = atoms.intern_js_string(&other_surrogate).unwrap();
        let replacement_atom = atoms.intern_js_string(&replacement).unwrap();

        assert_eq!(first, duplicate);
        assert_ne!(first, second);
        assert_ne!(first, replacement_atom);
        assert_eq!(atoms.to_js_string(first).unwrap(), high_surrogate);

        let global = atoms
            .intern_global_symbol_js_string(&JsString::try_from_utf16([0xd800, 0x61]).unwrap())
            .unwrap();
        assert_eq!(
            atoms
                .to_js_string(global)
                .unwrap()
                .utf16_units()
                .collect::<Vec<_>>(),
            vec![0xd800, 0x61]
        );
    }

    #[test]
    fn rope_keys_linearize_and_intern_by_utf16_content() {
        let mut atoms = AtomTable::new();
        let left = JsString::try_from_utf8(&"a".repeat(8193)).unwrap();
        let right = JsString::try_from_utf8(&"b".repeat(513)).unwrap();
        let rope = left.try_concat(&right).unwrap();
        assert!(!rope.is_flat());
        let flat = JsString::try_from_utf8(&("a".repeat(8193) + &"b".repeat(513))).unwrap();

        let first = atoms.intern_js_string(&rope).unwrap();
        let duplicate = atoms.intern_property_key_js_string(&flat).unwrap();
        assert_eq!(first, duplicate);
        assert_eq!(atoms.resolve(first).unwrap().ref_count, Some(2));
        let AtomSpelling::Text(stored) = atoms.resolve(first).unwrap().spelling else {
            panic!("rope string atom did not retain text");
        };
        assert!(stored.is_flat());
        assert_eq!(stored, &flat);

        let global_rope = atoms.intern_global_symbol_js_string(&rope).unwrap();
        let global_flat = atoms.intern_global_symbol_js_string(&flat).unwrap();
        assert_eq!(global_rope, global_flat);
        let AtomSpelling::Text(global_stored) = atoms.resolve(global_rope).unwrap().spelling else {
            panic!("rope global-symbol key did not retain text");
        };
        assert!(global_stored.is_flat());

        assert_eq!(atoms.release(first), Ok(ReleaseOutcome::Retained(1)));
        assert_eq!(atoms.release(duplicate), Ok(ReleaseOutcome::Removed));
        assert_eq!(atoms.release(global_rope), Ok(ReleaseOutcome::Retained(1)));
        assert_eq!(atoms.release(global_flat), Ok(ReleaseOutcome::Removed));
    }

    #[test]
    fn pinned_static_atoms_are_never_reclaimed() {
        let mut atoms = AtomTable::with_static_atoms(["", "length", "prototype"]).unwrap();
        let length = atoms.intern_static("length").unwrap();

        assert!(atoms.resolve(length).unwrap().is_permanent);
        assert_eq!(atoms.resolve(length).unwrap().ref_count, None);
        assert_eq!(atoms.release(length), Ok(ReleaseOutcome::Permanent));
        assert_eq!(atoms.intern("length"), Ok(length));
        assert_eq!(atoms.to_string(length).unwrap(), "length");

        let well_known = atoms.new_static_symbol(Some("Symbol.iterator")).unwrap();
        assert_eq!(atoms.release(well_known), Ok(ReleaseOutcome::Permanent));
        assert_eq!(atoms.kind(well_known), Ok(AtomKind::Symbol));

        let registry_symbol = atoms.intern_global_symbol("Symbol.iterator").unwrap();
        assert_ne!(well_known, registry_symbol);
        assert_eq!(atoms.kind(registry_symbol), Ok(AtomKind::GlobalSymbol));
    }

    #[test]
    fn forged_and_cross_table_ids_are_validated() {
        let mut first = AtomTable::new();
        let mut second = AtomTable::new();
        let atom = first.intern("only in first").unwrap();
        let other = second.intern("only in second").unwrap();

        assert!(first.is_live(atom));
        assert!(!second.is_live(atom));
        assert_eq!(atom.raw(), other.raw());
        assert_ne!(atom, other);
        assert!(matches!(
            second.resolve(atom),
            Err(AtomError::UnknownAtom(candidate)) if candidate == atom
        ));
        assert!(matches!(
            first.resolve(Atom::from_raw(ATOM_MAX_TABLE_INDEX)),
            Err(AtomError::UnknownAtom(_))
        ));
    }

    #[test]
    fn unique_key_churn_reuses_bounded_slot_space() {
        let mut atoms = AtomTable::new();
        let mut previous = None;
        for index in 0..100_000 {
            let atom = atoms.intern(&format!("key-{index}")).unwrap();
            assert_eq!(atom.raw(), 1);
            if let Some(previous) = previous {
                assert_ne!(atom, previous);
                assert!(!atoms.is_live(previous));
            }
            assert_eq!(atoms.release(atom), Ok(ReleaseOutcome::Removed));
            previous = Some(atom);
        }
        assert!(atoms.is_empty());
        let after = atoms.intern("after").unwrap();
        assert_eq!(after.raw(), 1);
    }
}
