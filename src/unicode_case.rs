//! Checksum-pinned Unicode 17 case conversion used by String intrinsics.
//!
//! This is a direct Rust port of QuickJS's `lre_case_conv`, `lre_is_cased`,
//! `lre_is_case_ignorable`, UTF-16 traversal, and Final_Sigma context check.
//! It intentionally does not use Rust `char` casing, whose Unicode version is
//! coupled to the toolchain and whose scalar-only API cannot preserve lone
//! UTF-16 surrogates.

#[cfg(test)]
use std::cell::Cell;

use crate::value::{JsString, JsStringError};

mod tables {
    include!("unicode_case_tables.rs");
}

const RUN_TYPE_U: u32 = 0;
const RUN_TYPE_L: u32 = 1;
const RUN_TYPE_UF: u32 = 2;
const RUN_TYPE_LF: u32 = 3;
const RUN_TYPE_UL: u32 = 4;
const RUN_TYPE_LSU: u32 = 5;
const RUN_TYPE_U2L_399_EXT2: u32 = 6;
const RUN_TYPE_UF_D20: u32 = 7;
const RUN_TYPE_UF_D1_EXT: u32 = 8;
const RUN_TYPE_U_EXT: u32 = 9;
const RUN_TYPE_LF_EXT: u32 = 10;
const RUN_TYPE_UF_EXT2: u32 = 11;
const RUN_TYPE_LF_EXT2: u32 = 12;
const RUN_TYPE_UF_EXT3: u32 = 13;

const INDEX_BLOCK_LEN: usize = 32;
const CODE_MASK: u32 = (1 << 21) - 1;

#[cfg(test)]
thread_local! {
    static FAIL_CASE_RESERVATION_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
}

/// Force the next QuickJS-equivalent case buffer allocation to fail. The hook
/// is thread-local so parallel test binaries cannot consume one another's
/// injected failure.
#[cfg(test)]
pub(crate) fn fail_next_case_reservation_for_test() {
    fail_case_reservation_after_for_test(0);
}

#[cfg(test)]
fn fail_case_reservation_after_for_test(successful_reservations: usize) {
    FAIL_CASE_RESERVATION_AFTER.with(|armed| {
        assert!(
            armed.replace(Some(successful_reservations)).is_none(),
            "case reservation failure was already armed"
        );
    });
}

fn try_reserve_exact<T>(values: &mut Vec<T>, additional: usize) -> Result<(), JsStringError> {
    #[cfg(test)]
    if FAIL_CASE_RESERVATION_AFTER.with(|armed| match armed.get() {
        None => false,
        Some(0) => {
            armed.set(None);
            true
        }
        Some(remaining) => {
            armed.set(Some(remaining - 1));
            false
        }
    }) {
        return Err(JsStringError::OutOfMemory);
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| JsStringError::OutOfMemory)
}

enum CaseStorage {
    Latin1(Vec<u8>),
    Utf16(Vec<u16>),
}

/// Narrow-first, fallible equivalent of QuickJS's `StringBuffer`. `size` is
/// kept separately from the host Vec capacity because QuickJS grows its
/// logical buffer by 3/2 and widens the complete logical allocation.
struct CaseStringBuffer {
    storage: CaseStorage,
    size: usize,
    limit: usize,
}

impl CaseStringBuffer {
    fn new(initial_size: usize, limit: usize) -> Result<Self, JsStringError> {
        let limit = limit.min(JsString::MAX_LEN);
        let size = initial_size.min(limit);
        let mut values = Vec::new();
        try_reserve_exact(&mut values, size)?;
        Ok(Self {
            storage: CaseStorage::Latin1(values),
            size,
            limit,
        })
    }

    fn len(&self) -> usize {
        match &self.storage {
            CaseStorage::Latin1(values) => values.len(),
            CaseStorage::Utf16(values) => values.len(),
        }
    }

    fn grow_size(&self, new_len: usize) -> Result<usize, JsStringError> {
        if new_len > self.limit {
            return Err(JsStringError::TooLong);
        }
        Ok(new_len.max(self.size.saturating_mul(3) / 2).min(self.limit))
    }

    fn reserve_to<T>(values: &mut Vec<T>, size: usize) -> Result<(), JsStringError> {
        // Enter the reservation path even if the host allocator previously
        // returned spare capacity: QuickJS tracks its own logical `size`.
        try_reserve_exact(values, size.saturating_sub(values.len()))
    }

    fn widen(&mut self, size: usize) -> Result<(), JsStringError> {
        let CaseStorage::Latin1(narrow) =
            std::mem::replace(&mut self.storage, CaseStorage::Latin1(Vec::new()))
        else {
            unreachable!("case buffer was widened twice")
        };
        let mut wide = Vec::new();
        if let Err(error) = try_reserve_exact(&mut wide, size) {
            self.storage = CaseStorage::Latin1(narrow);
            return Err(error);
        }
        wide.extend(narrow.iter().copied().map(u16::from));
        self.storage = CaseStorage::Utf16(wide);
        self.size = size;
        Ok(())
    }

    fn push_code_unit(&mut self, unit: u16) -> Result<(), JsStringError> {
        let len = self.len();
        let new_len = len.checked_add(1).ok_or(JsStringError::TooLong)?;
        if new_len > self.limit {
            return Err(JsStringError::TooLong);
        }

        if len >= self.size {
            let new_size = self.grow_size(new_len)?;
            match &mut self.storage {
                CaseStorage::Latin1(_) if unit > u16::from(u8::MAX) => {
                    self.widen(new_size)?;
                }
                CaseStorage::Latin1(values) => {
                    Self::reserve_to(values, new_size)?;
                    self.size = new_size;
                }
                CaseStorage::Utf16(values) => {
                    Self::reserve_to(values, new_size)?;
                    self.size = new_size;
                }
            }
        } else if matches!(self.storage, CaseStorage::Latin1(_)) && unit > u16::from(u8::MAX) {
            self.widen(self.size)?;
        }

        match &mut self.storage {
            CaseStorage::Latin1(values) => values.push(unit as u8),
            CaseStorage::Utf16(values) => values.push(unit),
        }
        Ok(())
    }

    fn push_code_point(&mut self, code_point: u32) -> Result<(), JsStringError> {
        debug_assert!(code_point <= 0x10_ffff);
        if code_point < 0x1_0000 {
            self.push_code_unit(code_point as u16)
        } else {
            let adjusted = code_point - 0x1_0000;
            self.push_code_unit(0xd800 | (adjusted >> 10) as u16)?;
            self.push_code_unit(0xdc00 | (adjusted & 0x3ff) as u16)
        }
    }

    fn finish(self) -> JsString {
        match self.storage {
            CaseStorage::Latin1(values) => JsString::from_owned_latin1(values),
            CaseStorage::Utf16(values) => JsString::from_owned_utf16(values),
        }
    }
}

#[derive(Clone, Copy)]
struct CaseMapping {
    code_points: [u32; 3],
    len: usize,
}

impl CaseMapping {
    const fn one(code_point: u32) -> Self {
        Self {
            code_points: [code_point, 0, 0],
            len: 1,
        }
    }

    const fn two(first: u32, second: u32) -> Self {
        Self {
            code_points: [first, second, 0],
            len: 2,
        }
    }

    const fn three(first: u32, second: u32, third: u32) -> Self {
        Self {
            code_points: [first, second, third],
            len: 3,
        }
    }
}

fn case_conversion_entry(code_point: u32, to_lower: bool, index: usize, entry: u32) -> CaseMapping {
    let run_type = (entry >> 4) & 0xf;
    let data = ((entry & 0xf) << 8) | u32::from(tables::CASE_CONV_TABLE2[index]);
    let run_start = entry >> 15;
    let is_lower = u32::from(to_lower);
    let mut converted = code_point;

    match run_type {
        RUN_TYPE_U | RUN_TYPE_L | RUN_TYPE_UF | RUN_TYPE_LF => {
            if is_lower == (run_type & 1) {
                let mapped_start = tables::CASE_CONV_TABLE1[data as usize] >> 15;
                converted = code_point - run_start + mapped_start;
            }
        }
        RUN_TYPE_UL => {
            let offset = code_point - run_start;
            if (offset & 1) == 1 - is_lower {
                converted = (offset ^ 1) + run_start;
            }
        }
        RUN_TYPE_LSU => {
            let offset = code_point - run_start;
            if offset == 1 {
                converted = (i64::from(code_point) + (2 * i64::from(is_lower) - 1)) as u32;
            } else if offset == (1 - is_lower) * 2 {
                converted = (i64::from(code_point) + (2 * i64::from(is_lower) - 1) * 2) as u32;
            }
        }
        RUN_TYPE_U2L_399_EXT2 => {
            if to_lower {
                converted = code_point - run_start
                    + u32::from(tables::CASE_CONV_EXT[(data & 0x3f) as usize]);
            } else {
                return CaseMapping::two(
                    code_point - run_start + u32::from(tables::CASE_CONV_EXT[(data >> 6) as usize]),
                    0x399,
                );
            }
        }
        RUN_TYPE_UF_D20 => {
            if !to_lower {
                converted = data;
            }
        }
        RUN_TYPE_UF_D1_EXT => {
            if !to_lower {
                converted = u32::from(tables::CASE_CONV_EXT[data as usize]);
            }
        }
        RUN_TYPE_U_EXT | RUN_TYPE_LF_EXT => {
            if is_lower == run_type - RUN_TYPE_U_EXT {
                converted = u32::from(tables::CASE_CONV_EXT[data as usize]);
            }
        }
        RUN_TYPE_UF_EXT2 => {
            if !to_lower {
                return CaseMapping::two(
                    code_point - run_start + u32::from(tables::CASE_CONV_EXT[(data >> 6) as usize]),
                    u32::from(tables::CASE_CONV_EXT[(data & 0x3f) as usize]),
                );
            }
        }
        RUN_TYPE_LF_EXT2 => {
            if to_lower {
                return CaseMapping::two(
                    code_point - run_start + u32::from(tables::CASE_CONV_EXT[(data >> 6) as usize]),
                    u32::from(tables::CASE_CONV_EXT[(data & 0x3f) as usize]),
                );
            }
        }
        RUN_TYPE_UF_EXT3 => {
            if !to_lower {
                return CaseMapping::three(
                    u32::from(tables::CASE_CONV_EXT[(data >> 8) as usize]),
                    u32::from(tables::CASE_CONV_EXT[((data >> 4) & 0xf) as usize]),
                    u32::from(tables::CASE_CONV_EXT[(data & 0xf) as usize]),
                );
            }
        }
        _ => unreachable!("invalid QuickJS case-conversion run type"),
    }
    CaseMapping::one(converted)
}

fn case_conversion(code_point: u32, to_lower: bool) -> CaseMapping {
    if code_point < 128 {
        let converted = if to_lower && (u32::from(b'A')..=u32::from(b'Z')).contains(&code_point) {
            code_point + u32::from(b'a' - b'A')
        } else if !to_lower && (u32::from(b'a')..=u32::from(b'z')).contains(&code_point) {
            code_point - u32::from(b'a' - b'A')
        } else {
            code_point
        };
        return CaseMapping::one(converted);
    }

    let mut lower = 0;
    let mut upper = tables::CASE_CONV_TABLE1.len();
    while lower < upper {
        let middle = (lower + upper) / 2;
        let entry = tables::CASE_CONV_TABLE1[middle];
        let run_start = entry >> 15;
        let run_len = (entry >> 8) & 0x7f;
        if code_point < run_start {
            upper = middle;
        } else if code_point >= run_start + run_len {
            lower = middle + 1;
        } else {
            return case_conversion_entry(code_point, to_lower, middle, entry);
        }
    }
    CaseMapping::one(code_point)
}

fn index_position(code_point: u32, index: &[u8]) -> Option<(u32, usize)> {
    debug_assert!(!index.is_empty() && index.len() % 3 == 0);
    let entry_count = index.len() / 3;
    let first = read_u24(index, 0);
    let first_code = first & CODE_MASK;
    if code_point < first_code {
        return Some((0, 0));
    }

    let last = read_u24(index, entry_count - 1);
    if code_point >= last & CODE_MASK {
        return None;
    }

    let mut lower = 0;
    let mut upper = entry_count - 1;
    while upper - lower > 1 {
        let middle = (upper + lower) / 2;
        if code_point < read_u24(index, middle) & CODE_MASK {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let entry = read_u24(index, lower);
    Some((
        entry & CODE_MASK,
        (lower + 1) * INDEX_BLOCK_LEN + (entry >> 21) as usize,
    ))
}

fn read_u24(index: &[u8], entry: usize) -> u32 {
    let offset = entry * 3;
    u32::from(index[offset])
        | (u32::from(index[offset + 1]) << 8)
        | (u32::from(index[offset + 2]) << 16)
}

fn is_in_table(code_point: u32, table: &[u8], index: &[u8]) -> bool {
    let Some((mut code, mut position)) = index_position(code_point, index) else {
        return false;
    };
    let mut included = false;
    loop {
        let byte = table[position];
        position += 1;
        if byte < 0x40 {
            code += u32::from(byte >> 3) + 1;
            if code_point < code {
                return included;
            }
            included = !included;
            code += u32::from(byte & 7) + 1;
        } else if byte >= 0x80 {
            code += u32::from(byte - 0x80) + 1;
        } else if byte < 0x60 {
            code += (u32::from(byte - 0x40) << 8) | u32::from(table[position]);
            code += 1;
            position += 1;
        } else {
            code += (u32::from(byte - 0x60) << 16)
                | (u32::from(table[position]) << 8)
                | u32::from(table[position + 1]);
            code += 1;
            position += 2;
        }
        if code_point < code {
            return included;
        }
        included = !included;
    }
}

fn is_cased(code_point: u32) -> bool {
    let mut lower = 0;
    let mut upper = tables::CASE_CONV_TABLE1.len();
    while lower < upper {
        let middle = (lower + upper) / 2;
        let entry = tables::CASE_CONV_TABLE1[middle];
        let run_start = entry >> 15;
        let run_len = (entry >> 8) & 0x7f;
        if code_point < run_start {
            upper = middle;
        } else if code_point >= run_start + run_len {
            lower = middle + 1;
        } else {
            return true;
        }
    }
    is_in_table(code_point, &tables::CASED_TABLE, &tables::CASED_INDEX)
}

fn is_case_ignorable(code_point: u32) -> bool {
    is_in_table(
        code_point,
        &tables::CASE_IGNORABLE_TABLE,
        &tables::CASE_IGNORABLE_INDEX,
    )
}

fn next_code_point(input: &JsString, index: &mut usize) -> u32 {
    let first = input
        .code_unit_at(*index)
        .expect("case conversion advanced past the input");
    *index += 1;
    if (0xd800..=0xdbff).contains(&first)
        && input
            .code_unit_at(*index)
            .is_some_and(|second| (0xdc00..=0xdfff).contains(&second))
    {
        let second = input
            .code_unit_at(*index)
            .expect("checked trail surrogate disappeared");
        *index += 1;
        0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
    } else {
        u32::from(first)
    }
}

/// Return zero before the first character, matching QuickJS `string_prevc`.
fn previous_code_point(input: &JsString, index: &mut usize) -> u32 {
    if *index == 0 {
        return 0;
    }
    *index -= 1;
    let second = input
        .code_unit_at(*index)
        .expect("case conversion retreated past the input");
    if (0xdc00..=0xdfff).contains(&second) && *index > 0 {
        let first = input
            .code_unit_at(*index - 1)
            .expect("checked lead surrogate disappeared");
        if (0xd800..=0xdbff).contains(&first) {
            *index -= 1;
            return 0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00);
        }
    }
    u32::from(second)
}

fn is_final_sigma(input: &JsString, sigma_position: usize) -> bool {
    let mut index = sigma_position;
    let previous = loop {
        let code_point = previous_code_point(input, &mut index);
        if !is_case_ignorable(code_point) {
            break code_point;
        }
    };
    if !is_cased(previous) {
        return false;
    }

    let mut index = sigma_position + 1;
    loop {
        if index >= input.len() {
            return true;
        }
        let code_point = next_code_point(input, &mut index);
        if !is_case_ignorable(code_point) {
            return !is_cased(code_point);
        }
    }
}

/// Convert one raw UTF-16 string using the exact Unicode tables and contextual
/// lowercase rule bundled by QuickJS 2026-06-04. `upper = true` selects upper
/// case. Locale arguments are intentionally outside this kernel because
/// QuickJS's four String methods ignore them completely.
pub(crate) fn convert_case_with_limit(
    input: &JsString,
    upper: bool,
    max_len: usize,
) -> Result<JsString, JsStringError> {
    if input.is_empty() {
        return Ok(input.clone());
    }

    let mut output = CaseStringBuffer::new(input.len(), max_len)?;
    let mut index = 0;
    while index < input.len() {
        let position = index;
        let code_point = next_code_point(input, &mut index);
        let mapping = if !upper && code_point == 0x3a3 && is_final_sigma(input, position) {
            CaseMapping::one(0x3c2)
        } else {
            case_conversion(code_point, !upper)
        };
        for mapped in &mapping.code_points[..mapping.len] {
            output.push_code_point(*mapped)?;
        }
    }
    Ok(output.finish())
}

#[cfg(test)]
mod tests {
    use super::{
        case_conversion, convert_case_with_limit, fail_case_reservation_after_for_test,
        fail_next_case_reservation_for_test, is_case_ignorable, is_cased, tables,
    };
    use crate::value::{JsString, JsStringError};

    fn convert(input: &JsString, upper: bool) -> JsString {
        convert_case_with_limit(input, upper, JsString::MAX_LEN).unwrap()
    }

    fn units(input: &JsString) -> Vec<u16> {
        input.utf16_units().collect()
    }

    #[test]
    fn generated_tables_are_pinned_to_quickjs_unicode_17() {
        assert_eq!(
            tables::SOURCE_SHA256,
            "cf782bc7a07549e976f606bd3cb8555858482b279574554dcb8d46412986006c"
        );
        assert_eq!(tables::CASE_CONV_TABLE1.len(), 378);
        assert_eq!(tables::CASE_CONV_TABLE2.len(), 378);
        assert_eq!(tables::CASE_CONV_EXT.len(), 58);
        assert_eq!(tables::CASED_TABLE.len(), 190);
        assert_eq!(tables::CASED_INDEX.len(), 18);
        assert_eq!(tables::CASE_IGNORABLE_TABLE.len(), 785);
        assert_eq!(tables::CASE_IGNORABLE_INDEX.len(), 75);
    }

    #[test]
    fn conversion_ports_ascii_simple_extended_and_multicodepoint_runs() {
        let input = JsString::try_from_utf8("aZ ß ﬃ ŉ İ 𐐀𐐨").unwrap();
        assert_eq!(convert(&input, true).to_utf8_lossy(), "AZ SS FFI ʼN İ 𐐀𐐀");
        assert_eq!(
            convert(&input, false).to_utf8_lossy(),
            "az ß ﬃ ŉ i\u{307} 𐐨𐐨"
        );

        let greek = JsString::try_from_utf8("ΐ").unwrap();
        assert_eq!(convert(&greek, true).to_utf8_lossy(), "Ι\u{308}\u{301}");
    }

    #[test]
    fn lowercase_uses_quickjs_final_sigma_context() {
        for (source, expected) in [
            ("ΟΣ", "ος"),
            ("ΟΣΑ", "οσα"),
            ("Σ", "σ"),
            ("A\u{301}Σ", "a\u{301}ς"),
            ("AΣ\u{301}", "aς\u{301}"),
            ("AΣ\u{301}B", "aσ\u{301}b"),
            ("AΣ'", "aς'"),
            ("AΣ'B", "aσ'b"),
        ] {
            let source = JsString::try_from_utf8(source).unwrap();
            assert_eq!(convert(&source, false).to_utf8_lossy(), expected);
        }
    }

    #[test]
    fn raw_utf16_surrogates_are_preserved_while_pairs_are_decoded() {
        let input =
            JsString::try_from_utf16([0xd800, 0x61, 0xdc00, 0xd801, 0xdc00, 0x5a, 0xdfff]).unwrap();
        assert_eq!(
            units(&convert(&input, true)),
            [0xd800, 0x41, 0xdc00, 0xd801, 0xdc00, 0x5a, 0xdfff]
        );
        assert_eq!(
            units(&convert(&input, false)),
            [0xd800, 0x61, 0xdc00, 0xd801, 0xdc28, 0x7a, 0xdfff]
        );
    }

    #[test]
    fn buffer_starts_narrow_then_widens_or_stays_narrow_from_content() {
        let narrows = JsString::try_from_utf8("Ÿ").unwrap();
        let narrowed = convert(&narrows, false);
        assert_eq!(narrowed.to_utf8_lossy(), "ÿ");
        assert!(!narrowed.is_wide());

        let widens = JsString::try_from_utf8("ÿ").unwrap();
        let widened = convert(&widens, true);
        assert_eq!(widened.to_utf8_lossy(), "Ÿ");
        assert!(widened.is_wide());
    }

    #[test]
    fn empty_identity_length_limit_and_reservation_failure_match_string_buffer() {
        let empty = JsString::from_static("");
        fail_next_case_reservation_for_test();
        let converted_empty = convert_case_with_limit(&empty, true, 0).unwrap();
        assert!(converted_empty.same_representation(&empty));

        // The empty fast path did not consume the injected allocation failure.
        let ascii = JsString::from_static("a");
        assert_eq!(
            convert_case_with_limit(&ascii, true, 1),
            Err(JsStringError::OutOfMemory)
        );

        let sharp_s = JsString::try_from_utf8("ß").unwrap();
        assert_eq!(
            convert_case_with_limit(&sharp_s, true, 1),
            Err(JsStringError::TooLong)
        );
        assert_eq!(
            convert_case_with_limit(&sharp_s, false, 1)
                .unwrap()
                .to_utf8_lossy(),
            "ß"
        );
    }

    #[test]
    fn widening_and_growth_reservation_failures_are_recoverable() {
        let widens = JsString::try_from_utf8("ÿ").unwrap();
        fail_case_reservation_after_for_test(1);
        assert_eq!(
            convert_case_with_limit(&widens, true, JsString::MAX_LEN),
            Err(JsStringError::OutOfMemory)
        );

        let expands = JsString::try_from_utf8("ß").unwrap();
        fail_case_reservation_after_for_test(1);
        assert_eq!(
            convert_case_with_limit(&expands, true, JsString::MAX_LEN),
            Err(JsStringError::OutOfMemory)
        );
        assert_eq!(
            convert_case_with_limit(&expands, true, JsString::MAX_LEN)
                .unwrap()
                .to_utf8_lossy(),
            "SS"
        );
    }

    #[test]
    fn cased_and_case_ignorable_property_boundaries_are_stable() {
        for code_point in [0x41, 0x2b0, 0x345, 0x1f88, 0x10400, 0x1d7c4] {
            assert!(is_cased(code_point), "U+{code_point:04X}");
        }
        for code_point in [0, 0x20, 0x300, 0x1f600, 0x10ffff] {
            assert!(!is_cased(code_point), "U+{code_point:04X}");
        }
        for code_point in [0x27, 0x2e, 0x300, 0x5bd, 0x200d, 0xe0100] {
            assert!(is_case_ignorable(code_point), "U+{code_point:04X}");
        }
        for code_point in [0, 0x41, 0x5be, 0x1f600, 0x10ffff] {
            assert!(!is_case_ignorable(code_point), "U+{code_point:04X}");
        }
    }

    #[test]
    fn table_mapping_has_expected_unicode_17_samples() {
        let lower = case_conversion(0x0130, true);
        assert_eq!(&lower.code_points[..lower.len], [0x69, 0x307]);
        let upper = case_conversion(0xfb03, false);
        assert_eq!(&upper.code_points[..upper.len], [0x46, 0x46, 0x49]);
        assert_eq!(case_conversion(0x10d70, false).code_points[0], 0x10d50);
        assert_eq!(case_conversion(0x10d50, true).code_points[0], 0x10d70);
    }

    #[test]
    fn exhaustive_kernel_fingerprint_matches_pinned_quickjs_c() {
        fn hash_u32(mut hash: u64, value: u32) -> u64 {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(1_099_511_628_211);
            }
            hash
        }

        let mut case_hash = 14_695_981_039_346_656_037_u64;
        let mut property_hash = 14_695_981_039_346_656_037_u64;
        let mut cased_count = 0_u32;
        let mut ignorable_count = 0_u32;
        for code_point in 0..=0x10_ffff {
            let cased = is_cased(code_point);
            let ignorable = is_case_ignorable(code_point);
            cased_count += u32::from(cased);
            ignorable_count += u32::from(ignorable);
            property_hash = hash_u32(property_hash, code_point);
            property_hash = hash_u32(property_hash, u32::from(cased));
            property_hash = hash_u32(property_hash, u32::from(ignorable));

            for (conversion, to_lower) in [(0, false), (1, true)] {
                let mapping = case_conversion(code_point, to_lower);
                case_hash = hash_u32(case_hash, code_point);
                case_hash = hash_u32(case_hash, conversion);
                case_hash = hash_u32(case_hash, mapping.len as u32);
                for mapped in &mapping.code_points[..mapping.len] {
                    case_hash = hash_u32(case_hash, *mapped);
                }
            }
        }

        assert_eq!(cased_count, 4_632);
        assert_eq!(ignorable_count, 2_794);
        assert_eq!(property_hash, 0x0f20_329d_dcab_a159);
        assert_eq!(case_hash, 0x24ab_cb80_d55c_587e);
    }
}
