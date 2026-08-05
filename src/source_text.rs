//! Reversible UTF-8 carrier text for dynamically supplied ECMAScript source.
//!
//! Rust `str` cannot contain the lone UTF-16 surrogates which QuickJS accepts
//! through dynamic `eval`. [`SourceText`] keeps the lexer on its existing
//! `&str` boundary without losing those code units: every lone surrogate is
//! represented by a private-use scalar with the same three-byte UTF-8 width,
//! and a side table records which private-use scalars are carriers rather than
//! real source text.

use std::collections::TryReserveError;
use std::iter::FusedIterator;
use std::ops::Range;

use crate::value::{JsString, JsStringError};

const HIGH_SURROGATE_START: u16 = 0xd800;
const HIGH_SURROGATE_END: u16 = 0xdbff;
const LOW_SURROGATE_START: u16 = 0xdc00;
const LOW_SURROGATE_END: u16 = 0xdfff;
const CARRIER_START: u32 = 0xe000;
const CARRIER_UTF8_LEN: usize = 3;

/// Check whether bytes are the unique WTF-8 encoding of their ECMAScript
/// UTF-16 code-unit sequence.
///
/// Canonical WTF-8 permits a three-byte encoding for an unpaired surrogate,
/// but rejects malformed/overlong UTF-8 and CESU-8 spellings of valid
/// surrogate pairs. Decoding with QuickJS's byte-string rules and requiring an
/// exact re-encoding keeps this validator aligned with the engine's pinned
/// String semantics.
pub(crate) fn try_is_canonical_wtf8(bytes: &[u8]) -> Result<bool, JsStringError> {
    let decoded = JsString::try_from_bytes(bytes)?;
    let canonical = decoded
        .try_to_wtf8_bytes()
        .map_err(|_| JsStringError::OutOfMemory)?;
    Ok(canonical == bytes)
}

/// One reversible substitution in a [`SourceText`] carrier.
///
/// Invariants maintained by `SourceText` construction:
///
/// - entries are sorted by unique carrier byte offset;
/// - every offset is a UTF-8 character boundary and points at the carrier
///   scalar `U+E000 + (unit - 0xD800)`;
/// - `unit` is an unpaired UTF-16 surrogate;
/// - both the original WTF-8 encoding and the carrier occupy exactly three
///   bytes, so source spans retain QuickJS's byte offsets.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct LoneSurrogate {
    byte_offset: u32,
    unit: u16,
}

#[cfg_attr(not(test), allow(dead_code))]
impl LoneSurrogate {
    #[must_use]
    pub(crate) const fn byte_offset(self) -> u32 {
        self.byte_offset
    }

    #[must_use]
    pub(crate) const fn unit(self) -> u16 {
        self.unit
    }
}

/// UTF-8 source accepted by the scalar-oriented lexer plus reversible lone
/// surrogate metadata.
///
/// Genuine `U+E000..=U+E7FF` source characters are stored without side-table
/// entries and therefore remain distinguishable from carrier scalars.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceText {
    carrier: Box<str>,
    lone_surrogates: Box<[LoneSurrogate]>,
}

/// A character-boundary-aligned byte range of [`SourceText`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceTextSlice<'a> {
    carrier: &'a str,
    base_byte_offset: u32,
    lone_surrogates: &'a [LoneSurrogate],
}

/// The original ECMAScript UTF-16 code units recovered from carrier text.
pub(crate) struct SourceTextUtf16Units<'a> {
    chars: std::str::CharIndices<'a>,
    base_byte_offset: u32,
    lone_surrogates: &'a [LoneSurrogate],
    marker_index: usize,
    pending_low_surrogate: Option<u16>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl SourceText {
    /// Wrap ordinary, well-formed UTF-8 source.
    ///
    /// This mirrors the engine's existing infallible `&str` source boundary;
    /// dynamic ECMAScript Strings use [`Self::try_from_js_string`] instead.
    #[must_use]
    pub(crate) fn from_utf8(source: &str) -> Self {
        Self {
            carrier: source.into(),
            lone_surrogates: Box::default(),
        }
    }

    /// Encode an ECMAScript String as reversible scalar-oriented source.
    ///
    /// Valid surrogate pairs become their Unicode scalar. Each unpaired
    /// surrogate becomes `U+E000..=U+E7FF` and receives a side-table entry.
    ///
    /// # Errors
    /// Returns [`JsStringError::OutOfMemory`] if carrier or marker storage
    /// cannot be reserved.
    pub(crate) fn try_from_js_string(source: &JsString) -> Result<Self, JsStringError> {
        let (carrier_len, marker_count) = measure_utf16(source.utf16_units())?;
        let mut carrier = String::new();
        carrier
            .try_reserve_exact(carrier_len)
            .map_err(|_| JsStringError::OutOfMemory)?;
        let mut lone_surrogates = Vec::new();
        lone_surrogates
            .try_reserve_exact(marker_count)
            .map_err(|_| JsStringError::OutOfMemory)?;

        let mut units = source.utf16_units().peekable();
        while let Some(unit) = units.next() {
            if is_high_surrogate(unit) && units.peek().copied().is_some_and(is_low_surrogate) {
                let low = units.next().expect("peeked low surrogate disappeared");
                carrier.push(surrogate_pair_to_char(unit, low));
            } else if is_surrogate(unit) {
                let byte_offset =
                    u32::try_from(carrier.len()).map_err(|_| JsStringError::TooLong)?;
                lone_surrogates.push(LoneSurrogate { byte_offset, unit });
                carrier.push(surrogate_carrier(unit));
            } else {
                carrier.push(char::from_u32(u32::from(unit)).expect("non-surrogate BMP scalar"));
            }
        }

        debug_assert_eq!(carrier.len(), carrier_len);
        debug_assert_eq!(lone_surrogates.len(), marker_count);
        let result = Self {
            carrier: carrier.into_boxed_str(),
            lone_surrogates: lone_surrogates.into_boxed_slice(),
        };
        debug_assert!(result.invariants_hold());
        Ok(result)
    }

    /// Build source directly from ECMAScript UTF-16 code units.
    ///
    /// # Errors
    /// Propagates the QuickJS String length limit and allocation errors.
    pub(crate) fn try_from_utf16(
        units: impl IntoIterator<Item = u16>,
    ) -> Result<Self, JsStringError> {
        let source = JsString::try_from_utf16(units)?;
        Self::try_from_js_string(&source)
    }

    #[must_use]
    pub(crate) fn carrier(&self) -> &str {
        &self.carrier
    }

    #[must_use]
    pub(crate) fn has_lone_surrogates(&self) -> bool {
        !self.lone_surrogates.is_empty()
    }

    #[must_use]
    pub(crate) fn lone_surrogates(&self) -> &[LoneSurrogate] {
        &self.lone_surrogates
    }

    /// Return the original surrogate represented at one carrier byte offset.
    #[must_use]
    pub(crate) fn lone_surrogate_at(&self, byte_offset: usize) -> Option<u16> {
        let byte_offset = u32::try_from(byte_offset).ok()?;
        self.lone_surrogates
            .binary_search_by_key(&byte_offset, |marker| marker.byte_offset)
            .ok()
            .map(|index| self.lone_surrogates[index].unit)
    }

    /// Select a half-open carrier byte range aligned to UTF-8 boundaries.
    #[must_use]
    pub(crate) fn slice(&self, range: Range<usize>) -> Option<SourceTextSlice<'_>> {
        let carrier = self.carrier.get(range.clone())?;
        let start = u32::try_from(range.start).ok()?;
        let end = u32::try_from(range.end).ok()?;
        let marker_start = self
            .lone_surrogates
            .partition_point(|marker| marker.byte_offset < start);
        let marker_end = self
            .lone_surrogates
            .partition_point(|marker| marker.byte_offset < end);
        Some(SourceTextSlice {
            carrier,
            base_byte_offset: start,
            lone_surrogates: &self.lone_surrogates[marker_start..marker_end],
        })
    }

    #[must_use]
    pub(crate) fn semantic_utf16_units(&self) -> SourceTextUtf16Units<'_> {
        SourceTextUtf16Units {
            chars: self.carrier.char_indices(),
            base_byte_offset: 0,
            lone_surrogates: &self.lone_surrogates,
            marker_index: 0,
            pending_low_surrogate: None,
        }
    }

    /// Reconstruct the exact QuickJS-compatible WTF-8 bytes of this source.
    pub(crate) fn try_to_wtf8_bytes(&self) -> Result<Vec<u8>, TryReserveError> {
        self.full_slice().try_to_wtf8_bytes()
    }

    /// Reconstruct the original ECMAScript String.
    pub(crate) fn try_to_js_string(&self) -> Result<JsString, JsStringError> {
        self.full_slice().try_to_js_string()
    }

    /// Reconstruct one valid carrier byte range as an ECMAScript String.
    ///
    /// `Ok(None)` means the range is out of bounds or splits a UTF-8 scalar.
    pub(crate) fn try_range_to_js_string(
        &self,
        range: Range<usize>,
    ) -> Result<Option<JsString>, JsStringError> {
        self.slice(range)
            .map(|slice| slice.try_to_js_string())
            .transpose()
    }

    /// Reconstruct one valid carrier byte range as exact WTF-8 bytes.
    ///
    /// `Ok(None)` means the range is out of bounds or splits a UTF-8 scalar.
    pub(crate) fn try_range_to_wtf8_bytes(
        &self,
        range: Range<usize>,
    ) -> Result<Option<Vec<u8>>, TryReserveError> {
        self.slice(range)
            .map(|slice| slice.try_to_wtf8_bytes())
            .transpose()
    }

    fn full_slice(&self) -> SourceTextSlice<'_> {
        SourceTextSlice {
            carrier: &self.carrier,
            base_byte_offset: 0,
            lone_surrogates: &self.lone_surrogates,
        }
    }

    fn invariants_hold(&self) -> bool {
        let mut previous_offset = None;
        self.lone_surrogates.iter().all(|marker| {
            let offset = marker.byte_offset as usize;
            let ordered = previous_offset.is_none_or(|previous| previous < marker.byte_offset);
            previous_offset = Some(marker.byte_offset);
            ordered
                && is_surrogate(marker.unit)
                && self.carrier.is_char_boundary(offset)
                && self
                    .carrier
                    .get(offset..)
                    .and_then(|tail| tail.chars().next())
                    == Some(surrogate_carrier(marker.unit))
        })
    }
}

impl AsRef<str> for SourceText {
    fn as_ref(&self) -> &str {
        self.carrier()
    }
}

impl From<&str> for SourceText {
    fn from(source: &str) -> Self {
        Self::from_utf8(source)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl SourceTextSlice<'_> {
    #[must_use]
    pub(crate) fn carrier(&self) -> &str {
        self.carrier
    }

    #[must_use]
    pub(crate) fn base_byte_offset(&self) -> u32 {
        self.base_byte_offset
    }

    #[must_use]
    pub(crate) fn has_lone_surrogates(&self) -> bool {
        !self.lone_surrogates.is_empty()
    }

    #[must_use]
    pub(crate) fn semantic_utf16_units(&self) -> SourceTextUtf16Units<'_> {
        SourceTextUtf16Units {
            chars: self.carrier.char_indices(),
            base_byte_offset: self.base_byte_offset,
            lone_surrogates: self.lone_surrogates,
            marker_index: 0,
            pending_low_surrogate: None,
        }
    }

    pub(crate) fn try_to_js_string(&self) -> Result<JsString, JsStringError> {
        JsString::try_from_utf16(self.semantic_utf16_units())
    }

    pub(crate) fn try_to_wtf8_bytes(&self) -> Result<Vec<u8>, TryReserveError> {
        let mut output = Vec::new();
        output.try_reserve_exact(self.carrier.len())?;
        let mut copied_until = 0;
        for marker in self.lone_surrogates {
            let marker_offset = (marker.byte_offset - self.base_byte_offset) as usize;
            output.extend_from_slice(&self.carrier.as_bytes()[copied_until..marker_offset]);
            output.extend_from_slice(&encode_lone_surrogate_wtf8(marker.unit));
            copied_until = marker_offset + CARRIER_UTF8_LEN;
        }
        output.extend_from_slice(&self.carrier.as_bytes()[copied_until..]);
        debug_assert_eq!(output.len(), self.carrier.len());
        Ok(output)
    }
}

impl Iterator for SourceTextUtf16Units<'_> {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        if let Some(unit) = self.pending_low_surrogate.take() {
            return Some(unit);
        }

        let (relative_offset, ch) = self.chars.next()?;
        let absolute_offset = self.base_byte_offset + relative_offset as u32;
        if self
            .lone_surrogates
            .get(self.marker_index)
            .is_some_and(|marker| marker.byte_offset == absolute_offset)
        {
            let marker = self.lone_surrogates[self.marker_index];
            self.marker_index += 1;
            debug_assert_eq!(ch, surrogate_carrier(marker.unit));
            return Some(marker.unit);
        }

        let mut encoded = [0_u16; 2];
        let encoded = ch.encode_utf16(&mut encoded);
        if encoded.len() == 2 {
            self.pending_low_surrogate = Some(encoded[1]);
        }
        Some(encoded[0])
    }
}

impl FusedIterator for SourceTextUtf16Units<'_> {}

fn measure_utf16(units: impl IntoIterator<Item = u16>) -> Result<(usize, usize), JsStringError> {
    let mut carrier_len = 0_usize;
    let mut marker_count = 0_usize;
    let mut units = units.into_iter().peekable();
    while let Some(unit) = units.next() {
        let encoded_len =
            if is_high_surrogate(unit) && units.peek().copied().is_some_and(is_low_surrogate) {
                units.next();
                4
            } else if is_surrogate(unit) {
                marker_count = marker_count.checked_add(1).ok_or(JsStringError::TooLong)?;
                CARRIER_UTF8_LEN
            } else {
                char::from_u32(u32::from(unit))
                    .expect("non-surrogate BMP scalar")
                    .len_utf8()
            };
        carrier_len = carrier_len
            .checked_add(encoded_len)
            .ok_or(JsStringError::TooLong)?;
    }
    Ok((carrier_len, marker_count))
}

const fn is_high_surrogate(unit: u16) -> bool {
    unit >= HIGH_SURROGATE_START && unit <= HIGH_SURROGATE_END
}

const fn is_low_surrogate(unit: u16) -> bool {
    unit >= LOW_SURROGATE_START && unit <= LOW_SURROGATE_END
}

const fn is_surrogate(unit: u16) -> bool {
    unit >= HIGH_SURROGATE_START && unit <= LOW_SURROGATE_END
}

fn surrogate_pair_to_char(high: u16, low: u16) -> char {
    let code_point = 0x1_0000
        + ((u32::from(high) - u32::from(HIGH_SURROGATE_START)) << 10)
        + (u32::from(low) - u32::from(LOW_SURROGATE_START));
    char::from_u32(code_point).expect("validated UTF-16 surrogate pair")
}

fn surrogate_carrier(unit: u16) -> char {
    let code_point = CARRIER_START + (u32::from(unit) - u32::from(HIGH_SURROGATE_START));
    char::from_u32(code_point).expect("surrogate carrier is a private-use scalar")
}

fn encode_lone_surrogate_wtf8(unit: u16) -> [u8; 3] {
    let value = u32::from(unit);
    [
        0xe0 | ((value >> 12) as u8),
        0x80 | (((value >> 6) & 0x3f) as u8),
        0x80 | ((value & 0x3f) as u8),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_utf8_is_an_identity_carrier() {
        let original = "ASCII\0é\u{e000}中😀\r\n";
        let source = SourceText::from_utf8(original);

        assert_eq!(source.carrier(), original);
        assert!(!source.has_lone_surrogates());
        assert!(source.lone_surrogates().is_empty());
        assert_eq!(source.try_to_wtf8_bytes().unwrap(), original.as_bytes());
        assert_eq!(source.as_ref(), original);
        assert_eq!(
            source.try_to_js_string().unwrap(),
            JsString::try_from_utf8(original).unwrap()
        );
    }

    #[test]
    fn canonical_wtf8_validator_accepts_scalars_and_lone_surrogates() {
        for bytes in [
            b"plain\0text".as_slice(),
            "é中😀\u{e000}".as_bytes(),
            &[0xed, 0xa0, 0x80],
            &[0xed, 0xaf, 0xbf],
            &[0xed, 0xb0, 0x80],
            &[0xed, 0xbf, 0xbf],
            &[b'a', 0xed, 0xa0, 0x80, b'b', 0xed, 0xbf, 0xbf],
        ] {
            assert!(try_is_canonical_wtf8(bytes).unwrap(), "{bytes:02x?}");
        }
    }

    #[test]
    fn canonical_wtf8_validator_rejects_noncanonical_byte_sequences() {
        for bytes in [
            &[0x80][..],
            &[0xc0, 0x80],
            &[0xe0, 0x80, 0x80],
            &[0xe2, 0x82],
            &[0xf4, 0x90, 0x80, 0x80],
            // CESU-8 for U+1F600 must instead be the four-byte scalar UTF-8.
            &[0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80],
        ] {
            assert!(!try_is_canonical_wtf8(bytes).unwrap(), "{bytes:02x?}");
        }
    }

    #[test]
    fn lone_surrogates_use_same_width_marked_carriers_without_pua_collision() {
        let units = [0xd800, 0xe000, 0xdc00, 0xe7ff, 0xdfff];
        let original = JsString::try_from_utf16(units).unwrap();
        let source = SourceText::try_from_js_string(&original).unwrap();

        assert_eq!(source.carrier().chars().count(), units.len());
        assert_eq!(source.carrier().len(), CARRIER_UTF8_LEN * units.len());
        assert_eq!(
            source
                .lone_surrogates()
                .iter()
                .map(|marker| (marker.byte_offset(), marker.unit()))
                .collect::<Vec<_>>(),
            vec![(0, 0xd800), (6, 0xdc00), (12, 0xdfff)]
        );
        assert_eq!(source.lone_surrogate_at(0), Some(0xd800));
        assert_eq!(source.lone_surrogate_at(3), None);
        assert_eq!(source.lone_surrogate_at(6), Some(0xdc00));
        assert_eq!(source.lone_surrogate_at(12), Some(0xdfff));
        assert_eq!(source.semantic_utf16_units().collect::<Vec<_>>(), units);
        assert_eq!(source.try_to_js_string().unwrap(), original);
        assert_eq!(
            source.try_to_wtf8_bytes().unwrap(),
            original.try_to_wtf8_bytes().unwrap()
        );
    }

    #[test]
    fn valid_surrogate_pairs_become_scalars_and_lone_neighbors_remain_marked() {
        let units = [0xd800, 0xd83d, 0xde80, 0xdc00, 0xdbff, 0xdfff];
        let original = JsString::try_from_utf16(units).unwrap();
        let source = SourceText::try_from_utf16(units).unwrap();

        assert_eq!(source.carrier().chars().count(), 4);
        assert_eq!(
            source.carrier().chars().collect::<Vec<_>>(),
            vec![
                surrogate_carrier(0xd800),
                '🚀',
                surrogate_carrier(0xdc00),
                '􏿿'
            ]
        );
        assert_eq!(source.semantic_utf16_units().collect::<Vec<_>>(), units);
        assert_eq!(source.try_to_js_string().unwrap(), original);
        assert_eq!(
            source.try_to_wtf8_bytes().unwrap(),
            original.try_to_wtf8_bytes().unwrap()
        );
    }

    #[test]
    fn semantic_slices_recover_original_units_and_wtf8() {
        let units = [0x61, 0xd800, 0x62, 0xe000, 0xd83d, 0xde80, 0xdc00, 0x63];
        let source = SourceText::try_from_utf16(units).unwrap();
        let slice = source.slice(1..15).unwrap();

        assert_eq!(slice.base_byte_offset(), 1);
        assert_eq!(slice.carrier(), &source.carrier()[1..15]);
        assert!(slice.has_lone_surrogates());
        assert_eq!(
            slice.semantic_utf16_units().collect::<Vec<_>>(),
            units[1..7]
        );
        let expected = JsString::try_from_utf16(units[1..7].iter().copied()).unwrap();
        assert_eq!(slice.try_to_js_string().unwrap(), expected);
        assert_eq!(
            slice.try_to_wtf8_bytes().unwrap(),
            expected.try_to_wtf8_bytes().unwrap()
        );
        assert_eq!(
            source.try_range_to_js_string(1..15).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            source.try_range_to_wtf8_bytes(1..15).unwrap(),
            Some(expected.try_to_wtf8_bytes().unwrap())
        );
        assert!(source.slice(2..15).is_none());
        assert!(
            source
                .try_range_to_js_string(0..usize::MAX)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn surrogate_pair_detection_crosses_js_string_rope_leaf_boundaries() {
        let mut left_units = vec![u16::from(b'a'); 8192];
        left_units.push(0xd83d);
        let left = JsString::try_from_utf16(left_units).unwrap();
        let right = JsString::try_from_utf16(
            [0xde80, 0xd800, u16::from(b'b')]
                .into_iter()
                .chain(std::iter::repeat_n(u16::from(b'c'), 512)),
        )
        .unwrap();
        let rope = left.try_concat(&right).unwrap();
        let source = SourceText::try_from_js_string(&rope).unwrap();

        assert_eq!(source.carrier().get(8192..8196), Some("🚀"));
        assert_eq!(source.lone_surrogate_at(8196), Some(0xd800));
        assert_eq!(source.try_to_js_string().unwrap(), rope);
        assert_eq!(
            source.try_to_wtf8_bytes().unwrap(),
            rope.try_to_wtf8_bytes().unwrap()
        );
    }
}
