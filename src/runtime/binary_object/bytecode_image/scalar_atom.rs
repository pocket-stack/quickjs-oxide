//! Sealed string projection for one authenticated function atom operand.
//!
//! A completed bytecode image owns semantic [`ImageAtom`] identities, but
//! scalar admission must not learn their release-pinned numeric spelling or
//! reuse an image-local dynamic index as a runtime atom. This module binds the
//! projection to a [`FunctionId`] authenticated by the same image and exposes
//! only the resulting ECMAScript String spelling.

use std::fmt;

use super::super::pinned_atoms::{FIRST_DYNAMIC_ATOM, PinnedAtomKind};
use super::super::wire::WireString;
use super::{BytecodeImage, FunctionId, ImageAtom};

/// A sealed String spelling projected from exactly one function relocation.
///
/// The representation is deliberately private. Callers can classify the
/// spelling through the three mutually exclusive accessors, but cannot forge
/// a projection or recover the source `ImageAtom`, pinned atom ID, header
/// slot, or image-local dynamic atom index.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) struct ImageStringAtomProjection<'image> {
    operand_offset: u32,
    spelling: ImageStringAtomSpelling<'image>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ImageStringAtomSpelling<'image> {
    Manifest(&'static str),
    CanonicalDecimal(u32),
    Dynamic(&'image WireString),
}

impl<'image> ImageStringAtomProjection<'image> {
    const fn new(operand_offset: u32, spelling: ImageStringAtomSpelling<'image>) -> Self {
        Self {
            operand_offset,
            spelling,
        }
    }

    /// Return the byte offset of the authenticated atom operand in its code.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn operand_offset(self) -> u32 {
        self.operand_offset
    }

    /// Return an ordinary release-manifest String spelling, if present.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn manifest_spelling(self) -> Option<&'static str> {
        match self.spelling {
            ImageStringAtomSpelling::Manifest(spelling) => Some(spelling),
            ImageStringAtomSpelling::CanonicalDecimal(_) | ImageStringAtomSpelling::Dynamic(_) => {
                None
            }
        }
    }

    /// Return the value of a canonical non-negative decimal String atom.
    ///
    /// This is the semantic number whose decimal spelling is the String
    /// value, not a raw or release-pinned atom identity.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn canonical_decimal(self) -> Option<u32> {
        match self.spelling {
            ImageStringAtomSpelling::CanonicalDecimal(value) => Some(value),
            ImageStringAtomSpelling::Manifest(_) | ImageStringAtomSpelling::Dynamic(_) => None,
        }
    }

    /// Borrow an image-owned dynamic String spelling, if present.
    #[must_use]
    pub(in crate::runtime::binary_object) const fn dynamic_string(
        self,
    ) -> Option<&'image WireString> {
        match self.spelling {
            ImageStringAtomSpelling::Dynamic(spelling) => Some(spelling),
            ImageStringAtomSpelling::Manifest(_) | ImageStringAtomSpelling::CanonicalDecimal(_) => {
                None
            }
        }
    }
}

/// Failure to project one authenticated function's sole atom as a String.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(in crate::runtime::binary_object) enum ImageStringAtomProjectionError {
    /// The supplied identity belongs to another image or is not a function in
    /// this image.
    FunctionNotInImage,
    /// The function does not contain exactly one relocated atom operand.
    AtomRelocationCount { actual: usize },
    /// The canonical scalar cohort permits at most one input atom slot.
    InputAtomSlotCount { actual: u32 },
    /// A sole input atom slot was not the source of the sole relocation.
    UnpairedInputAtomSlot,
    /// The completed code sidecar did not retain its authenticated operand.
    MissingAtomOperand,
    /// QuickJS's reserved atom-zero sentinel is not a String value.
    NullAtom,
    /// A private-name atom would evaluate as a Symbol, not a String.
    PrivateAtom,
    /// A symbol atom would evaluate as a Symbol, not a String.
    SymbolAtom,
    /// A completed image referred outside its own dynamic String arena.
    MissingDynamicString,
}

impl fmt::Display for ImageStringAtomProjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FunctionNotInImage => {
                formatter.write_str("function identity does not belong to this bytecode image")
            }
            Self::AtomRelocationCount { actual } => write!(
                formatter,
                "function contains {actual} atom relocations instead of exactly one"
            ),
            Self::InputAtomSlotCount { actual } => write!(
                formatter,
                "bytecode image contains {actual} input atom slots instead of at most one"
            ),
            Self::UnpairedInputAtomSlot => formatter.write_str(
                "bytecode image's sole input atom slot is not the function's sole atom operand",
            ),
            Self::MissingAtomOperand => {
                formatter.write_str("function atom sidecar points outside its code payload")
            }
            Self::NullAtom => formatter.write_str("null atom is not a String value"),
            Self::PrivateAtom => formatter.write_str("private atom is not a String value"),
            Self::SymbolAtom => formatter.write_str("symbol atom is not a String value"),
            Self::MissingDynamicString => {
                formatter.write_str("dynamic atom is absent from its bytecode image")
            }
        }
    }
}

impl std::error::Error for ImageStringAtomProjectionError {}

impl BytecodeImage {
    /// Project the sole atom relocation of one same-image function as a
    /// String spelling without exposing either atom namespace.
    ///
    /// `FunctionId` authenticates both the source image and function slot.
    /// The relocation is then selected from that function internally, so a
    /// caller cannot pair an atom sidecar from another decoded image with this
    /// image's dynamic String arena.
    pub(in crate::runtime::binary_object) fn project_single_string_atom(
        &self,
        function: FunctionId,
    ) -> Result<ImageStringAtomProjection<'_>, ImageStringAtomProjectionError> {
        let function = self
            .function(function)
            .ok_or(ImageStringAtomProjectionError::FunctionNotInImage)?;
        let code = function.envelope().code();
        let relocations = code.atom_relocations();
        let [relocation] = relocations else {
            return Err(ImageStringAtomProjectionError::AtomRelocationCount {
                actual: relocations.len(),
            });
        };
        let operand_offset = relocation.operand_offset();
        authenticate_input_atom_pairing(
            self.input_atom_slot_count(),
            code.as_bytes(),
            operand_offset,
        )?;
        let spelling = match relocation.atom() {
            ImageAtom::Null => return Err(ImageStringAtomProjectionError::NullAtom),
            ImageAtom::Index(value) => ImageStringAtomSpelling::CanonicalDecimal(value),
            ImageAtom::Predefined(atom) => match atom.kind() {
                PinnedAtomKind::String => ImageStringAtomSpelling::Manifest(atom.spelling()),
                PinnedAtomKind::Private => {
                    return Err(ImageStringAtomProjectionError::PrivateAtom);
                }
                PinnedAtomKind::Symbol => {
                    return Err(ImageStringAtomProjectionError::SymbolAtom);
                }
            },
            ImageAtom::Dynamic(atom) => {
                let spelling = self
                    .atoms()
                    .get(atom.as_usize())
                    .ok_or(ImageStringAtomProjectionError::MissingDynamicString)?;
                ImageStringAtomSpelling::Dynamic(spelling)
            }
        };
        Ok(ImageStringAtomProjection::new(operand_offset, spelling))
    }
}

/// Authenticate the canonical scalar cohort's header/relocation provenance.
///
/// The native-code scanner already authenticated both this operand boundary
/// and its semantic `ImageAtom`. This check consults the retained raw operand
/// only to prove whether the image's sole header slot was its origin; semantic
/// String classification continues to use the relocated atom exclusively.
fn authenticate_input_atom_pairing(
    input_atom_slots: u32,
    code: &[u8],
    operand_offset: u32,
) -> Result<(), ImageStringAtomProjectionError> {
    match input_atom_slots {
        0 => Ok(()),
        1 => {
            let start = usize::try_from(operand_offset)
                .map_err(|_| ImageStringAtomProjectionError::MissingAtomOperand)?;
            let end = start
                .checked_add(size_of::<u32>())
                .ok_or(ImageStringAtomProjectionError::MissingAtomOperand)?;
            let operand = code
                .get(start..end)
                .and_then(|bytes| <[u8; 4]>::try_from(bytes).ok())
                .map(u32::from_le_bytes)
                .ok_or(ImageStringAtomProjectionError::MissingAtomOperand)?;
            if operand == FIRST_DYNAMIC_ATOM {
                Ok(())
            } else {
                Err(ImageStringAtomProjectionError::UnpairedInputAtomSlot)
            }
        }
        actual => Err(ImageStringAtomProjectionError::InputAtomSlotCount { actual }),
    }
}
