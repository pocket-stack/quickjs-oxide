//! Byte-exact carrier text for dynamically supplied ECMAScript source.
//!
//! QuickJS parses an explicitly sized byte buffer. Rust `str` cannot represent
//! its accepted surrogate encodings or malformed bytes, so [`SourceText`]
//! keeps the exact input beside an equal-width scalar carrier for the existing
//! lexer. Surrogate code units use marked three-byte private-use scalars;
//! malformed bytes use marked one-byte DEL scalars. Side tables distinguish
//! those substitutions from genuine source PUA and DEL characters.

use std::collections::TryReserveError;
use std::iter::FusedIterator;
use std::ops::Range;

use crate::value::{JsString, JsStringError, decode_quickjs_utf8};

const HIGH_SURROGATE_START: u16 = 0xd800;
const LOW_SURROGATE_END: u16 = 0xdfff;
const CARRIER_START: u32 = 0xe000;
const CARRIER_UTF8_LEN: usize = 3;
const INVALID_BYTE_CARRIER: char = '\u{7f}';

/// One surrogate-code-unit substitution in a [`SourceText`] carrier.
///
/// Invariants maintained by `SourceText` construction:
///
/// - entries are sorted by unique carrier/raw byte offset;
/// - every offset is a UTF-8 character boundary and points at the carrier
///   scalar `U+E000 + (unit - 0xD800)`;
/// - `unit` is one UTF-16 surrogate decoded from a three-byte WTF-8/CESU-8
///   sequence;
/// - both the original encoding and carrier occupy exactly three bytes.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct SurrogateMarker {
    byte_offset: u32,
    unit: u16,
}

#[cfg_attr(not(test), allow(dead_code))]
impl SurrogateMarker {
    #[must_use]
    pub(crate) const fn byte_offset(self) -> u32 {
        self.byte_offset
    }

    #[must_use]
    pub(crate) const fn unit(self) -> u16 {
        self.unit
    }
}

/// Exact source bytes plus an equal-width UTF-8 carrier for the scalar lexer.
///
/// Genuine `U+E000..=U+E7FF` source characters are stored without side-table
/// entries, and genuine ASCII DEL bytes have no invalid-byte entry, so both
/// remain distinguishable from carrier scalars.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceText {
    /// Present only when carrier substitution made `carrier.as_bytes()` differ
    /// from the original input.
    raw: Option<Box<[u8]>>,
    carrier: Box<str>,
    surrogates: Box<[SurrogateMarker]>,
    invalid_bytes: Box<[u32]>,
}

/// A character-boundary-aligned byte range of [`SourceText`].
#[derive(Clone, Copy, Debug)]
pub(crate) struct SourceTextSlice<'a> {
    raw: &'a [u8],
    carrier: &'a str,
    base_byte_offset: u32,
    surrogates: &'a [SurrogateMarker],
    invalid_bytes: &'a [u32],
}

/// The original ECMAScript UTF-16 code units recovered from carrier text.
pub(crate) struct SourceTextUtf16Units<'a> {
    chars: std::str::CharIndices<'a>,
    base_byte_offset: u32,
    surrogates: &'a [SurrogateMarker],
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
            raw: None,
            carrier: source.into(),
            surrogates: Box::default(),
            invalid_bytes: Box::default(),
        }
    }

    /// Encode one explicitly sized QuickJS source buffer as equal-width
    /// scalar carrier text while retaining the exact original bytes.
    ///
    /// Structurally valid Unicode scalars remain byte-identical. Three-byte
    /// surrogate encodings become marked PUA scalars, and every byte belonging
    /// to malformed, overlong, or out-of-range input becomes one marked DEL.
    /// The latter byte-by-byte representation keeps every raw byte offset
    /// addressable without allowing the marker to acquire source semantics.
    ///
    /// # Errors
    /// Returns [`JsStringError::TooLong`] when offsets cannot be represented,
    /// or [`JsStringError::OutOfMemory`] when storage cannot be reserved.
    pub(crate) fn try_from_raw_bytes(source: &[u8]) -> Result<Self, JsStringError> {
        if source.len() > u32::MAX as usize {
            return Err(JsStringError::TooLong);
        }

        let mut carrier = String::new();
        carrier
            .try_reserve_exact(source.len())
            .map_err(|_| JsStringError::OutOfMemory)?;
        let mut surrogates = Vec::new();
        let mut invalid_bytes = Vec::new();

        let mut offset = 0;
        while offset < source.len() {
            let byte = source[offset];
            if byte < 0x80 {
                carrier.push(char::from(byte));
                offset += 1;
                continue;
            }

            match decode_quickjs_utf8(&source[offset..]) {
                Some((code_point, consumed)) if code_point <= 0x10_ffff => {
                    if let Ok(unit) = u16::try_from(code_point)
                        && is_surrogate(unit)
                    {
                        let byte_offset =
                            u32::try_from(offset).map_err(|_| JsStringError::TooLong)?;
                        surrogates
                            .try_reserve(1)
                            .map_err(|_| JsStringError::OutOfMemory)?;
                        surrogates.push(SurrogateMarker { byte_offset, unit });
                        carrier.push(surrogate_carrier(unit));
                    } else {
                        carrier.push(
                            char::from_u32(code_point)
                                .expect("validated non-surrogate Unicode scalar"),
                        );
                    }
                    offset += consumed;
                }
                Some(_) | None => {
                    invalid_bytes
                        .try_reserve(1)
                        .map_err(|_| JsStringError::OutOfMemory)?;
                    invalid_bytes.push(u32::try_from(offset).map_err(|_| JsStringError::TooLong)?);
                    carrier.push(INVALID_BYTE_CARRIER);
                    offset += 1;
                }
            }
            debug_assert_eq!(carrier.len(), offset);
        }

        let raw = if surrogates.is_empty() && invalid_bytes.is_empty() {
            None
        } else {
            let mut raw = Vec::new();
            raw.try_reserve_exact(source.len())
                .map_err(|_| JsStringError::OutOfMemory)?;
            raw.extend_from_slice(source);
            Some(raw.into_boxed_slice())
        };
        let result = Self {
            raw,
            carrier: carrier.into_boxed_str(),
            surrogates: surrogates.into_boxed_slice(),
            invalid_bytes: invalid_bytes.into_boxed_slice(),
        };
        debug_assert!(result.invariants_hold());
        Ok(result)
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
        let raw = source
            .try_to_wtf8_bytes()
            .map_err(|_| JsStringError::OutOfMemory)?;
        Self::try_from_raw_bytes(&raw)
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
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        self.raw.as_deref().unwrap_or(self.carrier.as_bytes())
    }

    #[must_use]
    pub(crate) fn has_surrogate_markers(&self) -> bool {
        !self.surrogates.is_empty()
    }

    #[must_use]
    pub(crate) fn surrogate_markers(&self) -> &[SurrogateMarker] {
        &self.surrogates
    }

    /// Return the source surrogate represented at one carrier/raw byte offset.
    #[must_use]
    pub(crate) fn surrogate_at(&self, byte_offset: usize) -> Option<u16> {
        let byte_offset = u32::try_from(byte_offset).ok()?;
        self.surrogates
            .binary_search_by_key(&byte_offset, |marker| marker.byte_offset)
            .ok()
            .map(|index| self.surrogates[index].unit)
    }

    /// Whether one carrier DEL represents a malformed raw source byte.
    #[must_use]
    pub(crate) fn invalid_byte_at(&self, byte_offset: usize) -> bool {
        u32::try_from(byte_offset)
            .ok()
            .is_some_and(|offset| self.invalid_bytes.binary_search(&offset).is_ok())
    }

    /// Select a half-open carrier byte range aligned to UTF-8 boundaries.
    #[must_use]
    pub(crate) fn slice(&self, range: Range<usize>) -> Option<SourceTextSlice<'_>> {
        let carrier = self.carrier.get(range.clone())?;
        let raw = self.raw_bytes().get(range.clone())?;
        let start = u32::try_from(range.start).ok()?;
        let end = u32::try_from(range.end).ok()?;
        let surrogate_start = self
            .surrogates
            .partition_point(|marker| marker.byte_offset < start);
        let surrogate_end = self
            .surrogates
            .partition_point(|marker| marker.byte_offset < end);
        let invalid_start = self.invalid_bytes.partition_point(|offset| *offset < start);
        let invalid_end = self.invalid_bytes.partition_point(|offset| *offset < end);
        Some(SourceTextSlice {
            raw,
            carrier,
            base_byte_offset: start,
            surrogates: &self.surrogates[surrogate_start..surrogate_end],
            invalid_bytes: &self.invalid_bytes[invalid_start..invalid_end],
        })
    }

    #[must_use]
    pub(crate) fn semantic_utf16_units(&self) -> Option<SourceTextUtf16Units<'_>> {
        if !self.invalid_bytes.is_empty() {
            return None;
        }
        Some(SourceTextUtf16Units {
            chars: self.carrier.char_indices(),
            base_byte_offset: 0,
            surrogates: &self.surrogates,
            marker_index: 0,
            pending_low_surrogate: None,
        })
    }

    /// Clone the exact explicitly sized source payload.
    pub(crate) fn try_to_raw_bytes(&self) -> Result<Vec<u8>, TryReserveError> {
        self.full_slice().try_to_raw_bytes()
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
        let Some(slice) = self.slice(range) else {
            return Ok(None);
        };
        if slice.has_invalid_bytes() {
            return Ok(None);
        }
        slice.try_to_js_string().map(Some)
    }

    /// Clone one valid carrier byte range as exact original source bytes.
    ///
    /// `Ok(None)` means the range is out of bounds or splits a UTF-8 scalar.
    pub(crate) fn try_range_to_raw_bytes(
        &self,
        range: Range<usize>,
    ) -> Result<Option<Vec<u8>>, TryReserveError> {
        self.slice(range)
            .map(|slice| slice.try_to_raw_bytes())
            .transpose()
    }

    fn full_slice(&self) -> SourceTextSlice<'_> {
        SourceTextSlice {
            raw: self.raw_bytes(),
            carrier: &self.carrier,
            base_byte_offset: 0,
            surrogates: &self.surrogates,
            invalid_bytes: &self.invalid_bytes,
        }
    }

    fn invariants_hold(&self) -> bool {
        if self.raw_bytes().len() != self.carrier.len()
            || (!self.surrogates.is_empty() || !self.invalid_bytes.is_empty()) && self.raw.is_none()
            || self
                .raw
                .as_deref()
                .is_some_and(|raw| raw == self.carrier.as_bytes())
        {
            return false;
        }
        let mut previous_offset = None;
        let surrogates_hold = self.surrogates.iter().all(|marker| {
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
                && decode_quickjs_utf8(&self.raw_bytes()[offset..])
                    == Some((u32::from(marker.unit), CARRIER_UTF8_LEN))
        });
        let invalid_hold = self.invalid_bytes.windows(2).all(|pair| pair[0] < pair[1])
            && self.invalid_bytes.iter().all(|offset| {
                let offset = *offset as usize;
                let preceding_surrogate = self
                    .surrogates
                    .partition_point(|marker| marker.byte_offset as usize <= offset)
                    .checked_sub(1)
                    .and_then(|index| self.surrogates.get(index));
                self.carrier.as_bytes().get(offset) == Some(&(INVALID_BYTE_CARRIER as u8))
                    && self.carrier.is_char_boundary(offset)
                    && self
                        .raw
                        .as_deref()
                        .and_then(|raw| raw.get(offset))
                        .is_some_and(|byte| *byte >= 0x80)
                    && preceding_surrogate.is_none_or(|marker| {
                        offset >= marker.byte_offset as usize + CARRIER_UTF8_LEN
                    })
            });
        surrogates_hold && invalid_hold
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
impl<'a> SourceTextSlice<'a> {
    #[must_use]
    pub(crate) fn raw_bytes(&self) -> &[u8] {
        self.raw
    }

    #[must_use]
    pub(crate) fn carrier(&self) -> &str {
        self.carrier
    }

    #[must_use]
    pub(crate) fn base_byte_offset(&self) -> u32 {
        self.base_byte_offset
    }

    #[must_use]
    pub(crate) fn has_surrogate_markers(&self) -> bool {
        !self.surrogates.is_empty()
    }

    #[must_use]
    pub(crate) fn has_invalid_bytes(&self) -> bool {
        !self.invalid_bytes.is_empty()
    }

    #[must_use]
    pub(crate) fn semantic_utf16_units(&self) -> Option<SourceTextUtf16Units<'a>> {
        if self.has_invalid_bytes() {
            return None;
        }
        Some(SourceTextUtf16Units {
            chars: self.carrier.char_indices(),
            base_byte_offset: self.base_byte_offset,
            surrogates: self.surrogates,
            marker_index: 0,
            pending_low_surrogate: None,
        })
    }

    pub(crate) fn try_to_js_string(&self) -> Result<JsString, JsStringError> {
        JsString::try_from_bytes(self.raw)
    }

    pub(crate) fn try_to_raw_bytes(&self) -> Result<Vec<u8>, TryReserveError> {
        let mut output = Vec::new();
        output.try_reserve_exact(self.raw.len())?;
        output.extend_from_slice(self.raw);
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
            .surrogates
            .get(self.marker_index)
            .is_some_and(|marker| marker.byte_offset == absolute_offset)
        {
            let marker = self.surrogates[self.marker_index];
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

const fn is_surrogate(unit: u16) -> bool {
    unit >= HIGH_SURROGATE_START && unit <= LOW_SURROGATE_END
}

fn surrogate_carrier(unit: u16) -> char {
    let code_point = CARRIER_START + (u32::from(unit) - u32::from(HIGH_SURROGATE_START));
    char::from_u32(code_point).expect("surrogate carrier is a private-use scalar")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordinary_utf8_is_an_identity_carrier() {
        let original = "ASCII\0\u{7f}\u{feff}é\u{e000}中😀\r\n";
        let source = SourceText::from_utf8(original);

        assert_eq!(source.carrier(), original);
        assert!(source.raw.is_none());
        assert!(!source.has_surrogate_markers());
        assert!(source.surrogate_markers().is_empty());
        assert_eq!(source.try_to_raw_bytes().unwrap(), original.as_bytes());
        assert_eq!(source.as_ref(), original);
        assert_eq!(
            source.try_to_js_string().unwrap(),
            JsString::try_from_utf8(original).unwrap()
        );
    }

    #[test]
    fn raw_scalars_share_the_identity_storage_path() {
        let raw = b"plain\0\x7f\xef\xbb\xbf\xee\x80\x80\xf0\x9f\x98\x80";
        let source = SourceText::try_from_raw_bytes(raw).unwrap();

        assert!(source.raw.is_none());
        assert_eq!(source.carrier().as_bytes(), raw);
        assert_eq!(source.raw_bytes(), raw);
        assert!(source.surrogate_markers().is_empty());
        assert!(source.invalid_bytes.is_empty());
    }

    #[test]
    fn surrogate_bytes_use_same_width_marked_carriers_without_pua_collision() {
        let units = [0xd800, 0xe000, 0xdc00, 0xe7ff, 0xdfff];
        let original = JsString::try_from_utf16(units).unwrap();
        let source = SourceText::try_from_js_string(&original).unwrap();

        assert_eq!(source.carrier().chars().count(), units.len());
        assert_eq!(source.carrier().len(), CARRIER_UTF8_LEN * units.len());
        assert_eq!(
            source
                .surrogate_markers()
                .iter()
                .map(|marker| (marker.byte_offset(), marker.unit()))
                .collect::<Vec<_>>(),
            vec![(0, 0xd800), (6, 0xdc00), (12, 0xdfff)]
        );
        assert_eq!(source.surrogate_at(0), Some(0xd800));
        assert_eq!(source.surrogate_at(3), None);
        assert_eq!(source.surrogate_at(6), Some(0xdc00));
        assert_eq!(source.surrogate_at(12), Some(0xdfff));
        assert_eq!(
            source.semantic_utf16_units().unwrap().collect::<Vec<_>>(),
            units
        );
        assert_eq!(source.try_to_js_string().unwrap(), original);
        assert_eq!(
            source.try_to_raw_bytes().unwrap(),
            original.try_to_wtf8_bytes().unwrap()
        );
    }

    #[test]
    fn cesu8_pairs_remain_two_markers_and_exact_raw_bytes() {
        let raw = [0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80];
        let source = SourceText::try_from_raw_bytes(&raw).unwrap();

        assert_eq!(source.carrier().len(), raw.len());
        assert_eq!(source.raw_bytes(), raw);
        assert_eq!(
            source.semantic_utf16_units().unwrap().collect::<Vec<_>>(),
            [0xd83d, 0xde00]
        );
        assert_eq!(
            source
                .surrogate_markers()
                .iter()
                .map(|marker| (marker.byte_offset(), marker.unit()))
                .collect::<Vec<_>>(),
            [(0, 0xd83d), (3, 0xde00)]
        );
    }

    #[test]
    fn malformed_bytes_use_distinct_equal_width_del_markers() {
        let raw = [b'a', 0x7f, 0x80, 0xc0, 0x80, 0xf4, 0x90, 0x80, 0x80, b'z'];
        let source = SourceText::try_from_raw_bytes(&raw).unwrap();

        assert_eq!(source.raw_bytes(), raw);
        assert_eq!(source.carrier().len(), raw.len());
        assert_eq!(
            source.carrier().as_bytes(),
            b"a\x7f\x7f\x7f\x7f\x7f\x7f\x7f\x7fz"
        );
        assert!(!source.invalid_byte_at(1));
        assert_eq!(source.invalid_bytes.as_ref(), &[2, 3, 4, 5, 6, 7, 8]);
        for offset in 2..=8 {
            assert!(source.invalid_byte_at(offset));
        }
        assert!(source.semantic_utf16_units().is_none());
        assert_eq!(source.try_range_to_js_string(0..raw.len()).unwrap(), None);
        assert_eq!(source.try_to_raw_bytes().unwrap(), raw);
        assert_eq!(
            source
                .try_to_js_string()
                .unwrap()
                .utf16_units()
                .collect::<Vec<_>>(),
            [0x61, 0x7f, 0xfffd, 0xfffd, 0x7a]
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
        assert_eq!(
            source.semantic_utf16_units().unwrap().collect::<Vec<_>>(),
            units
        );
        assert_eq!(source.try_to_js_string().unwrap(), original);
        assert_eq!(
            source.try_to_raw_bytes().unwrap(),
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
        assert!(slice.has_surrogate_markers());
        assert!(!slice.has_invalid_bytes());
        assert_eq!(
            slice.semantic_utf16_units().unwrap().collect::<Vec<_>>(),
            units[1..7]
        );
        let expected = JsString::try_from_utf16(units[1..7].iter().copied()).unwrap();
        assert_eq!(slice.try_to_js_string().unwrap(), expected);
        assert_eq!(
            slice.try_to_raw_bytes().unwrap(),
            expected.try_to_wtf8_bytes().unwrap()
        );
        assert_eq!(
            source.try_range_to_js_string(1..15).unwrap(),
            Some(expected.clone())
        );
        assert_eq!(
            source.try_range_to_raw_bytes(1..15).unwrap(),
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
        assert_eq!(source.surrogate_at(8196), Some(0xd800));
        assert_eq!(source.try_to_js_string().unwrap(), rope);
        assert_eq!(
            source.try_to_raw_bytes().unwrap(),
            rope.try_to_wtf8_bytes().unwrap()
        );
    }

    #[test]
    fn raw_slices_preserve_malformed_bytes_and_quickjs_string_decode() {
        let source = SourceText::try_from_raw_bytes(b"a/*\x80X*/z").unwrap();
        let slice = source.slice(1..7).unwrap();

        assert_eq!(slice.raw_bytes(), b"/*\x80X*/");
        assert!(slice.has_invalid_bytes());
        assert_eq!(slice.try_to_raw_bytes().unwrap(), b"/*\x80X*/");
        assert_eq!(
            slice
                .try_to_js_string()
                .unwrap()
                .utf16_units()
                .collect::<Vec<_>>(),
            [0x2f, 0x2a, 0xfffd, 0x2a, 0x2f]
        );
    }
}
