//! Checksum-pinned Unicode 17 normalization used by String intrinsics.
//!
//! This is a direct Rust port of QuickJS's `unicode_normalize`, including its
//! compressed decomposition, composition, and canonical-combining-class table
//! readers. It deliberately does not use a Rust Unicode crate: QuickJS pins a
//! particular Unicode release, and ECMAScript strings must preserve lone
//! UTF-16 surrogates that Rust scalar-value APIs cannot represent.

#[cfg(test)]
use std::cell::Cell;
use std::cmp::Ordering;

use crate::value::{JsString, JsStringError};

mod tables {
    include!("unicode_normalize_tables.rs");
}

const INDEX_BLOCK_LEN: usize = 32;
const CODE_MASK: u32 = (1 << 21) - 1;
const DECOMPOSITION_MAX_LEN: usize = 18;

const DECOMP_TYPE_C1: u32 = 0;
const DECOMP_TYPE_L1: u32 = 1;
const DECOMP_TYPE_L7: u32 = 7;
const DECOMP_TYPE_LL1: u32 = 8;
const DECOMP_TYPE_LL2: u32 = 9;
const DECOMP_TYPE_S1: u32 = 10;
const DECOMP_TYPE_S5: u32 = 14;
const DECOMP_TYPE_I1: u32 = 15;
const DECOMP_TYPE_I2_0: u32 = 16;
const DECOMP_TYPE_I4_2: u32 = 21;
const DECOMP_TYPE_B1: u32 = 22;
const DECOMP_TYPE_B8: u32 = 29;
const DECOMP_TYPE_B18: u32 = 30;
const DECOMP_TYPE_LS2: u32 = 31;
const DECOMP_TYPE_PAT3: u32 = 32;
const DECOMP_TYPE_S2_UL: u32 = 33;
const DECOMP_TYPE_LS2_UL: u32 = 34;

/// The four normalization forms accepted by `String.prototype.normalize`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum NormalizationForm {
    Nfc,
    Nfd,
    Nfkc,
    Nfkd,
}

impl NormalizationForm {
    const fn is_compatibility(self) -> bool {
        matches!(self, Self::Nfkc | Self::Nfkd)
    }

    const fn is_decomposed(self) -> bool {
        matches!(self, Self::Nfd | Self::Nfkd)
    }
}

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_NORMALIZE_RESERVATION: Cell<bool> = const { Cell::new(false) };
}

/// Force the next normalization buffer reservation to fail. The hook is
/// thread-local so concurrently running tests cannot consume one another's
/// injected failure.
#[cfg(test)]
pub(crate) fn fail_next_normalize_reservation_for_test() {
    FAIL_NEXT_NORMALIZE_RESERVATION.with(|armed| {
        assert!(
            !armed.replace(true),
            "normalization reservation failure was already armed"
        );
    });
}

fn try_reserve_exact<T>(values: &mut Vec<T>, additional: usize) -> Result<(), JsStringError> {
    #[cfg(test)]
    if FAIL_NEXT_NORMALIZE_RESERVATION.with(|armed| armed.replace(false)) {
        return Err(JsStringError::OutOfMemory);
    }
    values
        .try_reserve_exact(additional)
        .map_err(|_| JsStringError::OutOfMemory)
}

fn push_fallible<T>(values: &mut Vec<T>, value: T) -> Result<(), JsStringError> {
    if values.len() == values.capacity() {
        // Match QuickJS DynBuf's 1.5x growth policy. Reserving exactly one
        // element here would make repeated compatibility decompositions such
        // as U+FDFA perform O(n^2) reallocations once they outgrow the input-
        // sized initial buffer, which is especially costly under WebAssembly.
        let additional = (values.capacity() / 2).max(1);
        try_reserve_exact(values, additional)?;
    }
    values.push(value);
    Ok(())
}

fn get_u16(data: &[u8], position: usize) -> u32 {
    u32::from(data[position]) | (u32::from(data[position + 1]) << 8)
}

fn short_code(code: u32) -> u32 {
    const SHORT_TABLE: [u32; 2] = [0x2044, 0x2215];
    if code < 0x80 {
        code
    } else if code < 0xd0 {
        code - 0x80 + 0x300
    } else {
        SHORT_TABLE[(code - 0xd0) as usize]
    }
}

fn lower_simple(mut code_point: u32) -> u32 {
    if code_point < 0x100 || (0x410..=0x42f).contains(&code_point) {
        code_point += 0x20;
    } else {
        code_point += 1;
    }
    code_point
}

fn decomposition_entry(
    output: &mut [u32; DECOMPOSITION_MAX_LEN],
    code_point: u32,
    table_index: usize,
    run_start: u32,
    run_len: u32,
    run_type: u32,
) -> usize {
    if run_type == DECOMP_TYPE_C1 {
        output[0] = u32::from(tables::UNICODE_DECOMP_TABLE2[table_index]);
        return 1;
    }

    let data_offset = usize::from(tables::UNICODE_DECOMP_TABLE2[table_index]);
    let data = &tables::UNICODE_DECOMP_DATA[data_offset..];
    let delta = (code_point - run_start) as usize;

    match run_type {
        DECOMP_TYPE_L1..=DECOMP_TYPE_L7 => {
            let len = (run_type - DECOMP_TYPE_L1 + 1) as usize;
            let data = &data[delta * len * 2..];
            for (index, slot) in output[..len].iter_mut().enumerate() {
                *slot = get_u16(data, index * 2);
                if *slot == 0 {
                    return 0;
                }
            }
            len
        }
        DECOMP_TYPE_LL1..=DECOMP_TYPE_LL2 => {
            let len = (run_type - DECOMP_TYPE_LL1 + 1) as usize;
            let high_bits_offset = run_len as usize * len * 2;
            for (item_index, slot) in (delta * len..).zip(output[..len].iter_mut()) {
                let high_bits =
                    (data[high_bits_offset + item_index / 4] >> ((item_index % 4) * 2)) & 3;
                *slot = get_u16(data, item_index * 2) | (u32::from(high_bits) << 16);
                if *slot == 0 {
                    return 0;
                }
            }
            len
        }
        DECOMP_TYPE_S1..=DECOMP_TYPE_S5 => {
            let len = (run_type - DECOMP_TYPE_S1 + 1) as usize;
            let data = &data[delta * len..];
            for (index, slot) in output[..len].iter_mut().enumerate() {
                *slot = short_code(u32::from(data[index]));
                if *slot == 0 {
                    return 0;
                }
            }
            len
        }
        DECOMP_TYPE_I1 => {
            output[0] = get_u16(data, 0) + code_point - run_start;
            1
        }
        DECOMP_TYPE_I2_0..=DECOMP_TYPE_I4_2 => {
            let len = (2 + ((run_type - DECOMP_TYPE_I2_0) >> 1)) as usize;
            let incremented = ((run_type - DECOMP_TYPE_I2_0) & 1) as usize + usize::from(len > 2);
            for (index, slot) in output[..len].iter_mut().enumerate() {
                *slot = get_u16(data, index * 2);
                if index == incremented {
                    *slot += code_point - run_start;
                }
            }
            len
        }
        DECOMP_TYPE_B1..=DECOMP_TYPE_B8 | DECOMP_TYPE_B18 => {
            let len = if run_type == DECOMP_TYPE_B18 {
                18
            } else {
                (run_type - DECOMP_TYPE_B1 + 1) as usize
            };
            let minimum = get_u16(data, 0);
            let data = &data[2 + delta * len..];
            for (index, slot) in output[..len].iter_mut().enumerate() {
                *slot = if data[index] == 0xff {
                    0x20
                } else {
                    minimum + u32::from(data[index])
                };
            }
            len
        }
        DECOMP_TYPE_LS2 => {
            let data = &data[delta * 3..];
            output[0] = get_u16(data, 0);
            if output[0] == 0 {
                return 0;
            }
            output[1] = short_code(u32::from(data[2]));
            2
        }
        DECOMP_TYPE_PAT3 => {
            output[0] = get_u16(data, 0);
            output[2] = get_u16(data, 2);
            output[1] = get_u16(data, 4 + delta * 2);
            3
        }
        DECOMP_TYPE_S2_UL | DECOMP_TYPE_LS2_UL => {
            let mut data = data;
            let mut first;
            if run_type == DECOMP_TYPE_S2_UL {
                data = &data[delta & !1..];
                first = short_code(u32::from(data[0]));
                data = &data[1..];
            } else {
                data = &data[(delta >> 1) * 3..];
                first = get_u16(data, 0);
                data = &data[2..];
            }
            if delta & 1 != 0 {
                first = lower_simple(first);
            }
            output[0] = first;
            output[1] = short_code(u32::from(data[0]));
            2
        }
        _ => 0,
    }
}

fn decomposition(
    output: &mut [u32; DECOMPOSITION_MAX_LEN],
    code_point: u32,
    compatibility: bool,
) -> usize {
    let mut lower = 0_usize;
    let mut upper = tables::UNICODE_DECOMP_TABLE1.len();
    while lower < upper {
        let index = (lower + upper) / 2;
        let entry = tables::UNICODE_DECOMP_TABLE1[index];
        let run_start = entry >> 14;
        let run_len = (entry >> 7) & 0x7f;
        if code_point < run_start {
            upper = index;
        } else if code_point >= run_start + run_len {
            lower = index + 1;
        } else {
            let is_compatibility = entry & 1 != 0;
            if is_compatibility && !compatibility {
                return 0;
            }
            let run_type = (entry >> 1) & 0x3f;
            return decomposition_entry(output, code_point, index, run_start, run_len, run_type);
        }
    }
    0
}

fn append_decomposed(
    output: &mut Vec<u32>,
    code_point: u32,
    compatibility: bool,
) -> Result<(), JsStringError> {
    if (0xac00..0xd7a4).contains(&code_point) {
        let syllable = code_point - 0xac00;
        push_fallible(output, 0x1100 + syllable / 588)?;
        push_fallible(output, 0x1161 + (syllable % 588) / 28)?;
        let trailing = syllable % 28;
        if trailing != 0 {
            push_fallible(output, 0x11a7 + trailing)?;
        }
        return Ok(());
    }

    let mut mapped = [0_u32; DECOMPOSITION_MAX_LEN];
    let mapped_len = decomposition(&mut mapped, code_point, compatibility);
    if mapped_len == 0 {
        push_fallible(output, code_point)
    } else {
        for mapped_code_point in &mapped[..mapped_len] {
            append_decomposed(output, *mapped_code_point, compatibility)?;
        }
        Ok(())
    }
}

fn read_le24(table: &[u8], entry: usize) -> u32 {
    let position = entry * 3;
    u32::from(table[position])
        | (u32::from(table[position + 1]) << 8)
        | (u32::from(table[position + 2]) << 16)
}

/// Return the compressed-table byte position and code point at the start of
/// the containing index block, matching QuickJS `get_index_pos`.
fn index_position(code_point: u32, index: &[u8]) -> Option<(usize, u32)> {
    let entry_count = index.len() / 3;
    let first = read_le24(index, 0) & CODE_MASK;
    if code_point < first {
        return Some((0, 0));
    }

    // QuickJS deliberately reads the final upper-bound entry without masking
    // its packed position bits. Unicode's generated upper bound has none.
    if code_point >= read_le24(index, entry_count - 1) {
        return None;
    }

    let mut lower = 0_usize;
    let mut upper = entry_count - 1;
    while upper - lower > 1 {
        let middle = (lower + upper) / 2;
        let entry = read_le24(index, middle);
        if code_point < entry & CODE_MASK {
            upper = middle;
        } else {
            lower = middle;
        }
    }
    let entry = read_le24(index, lower);
    Some((
        (lower + 1) * INDEX_BLOCK_LEN + (entry >> 21) as usize,
        entry & CODE_MASK,
    ))
}

fn combining_class(code_point: u32) -> u32 {
    let Some((mut position, mut code)) = index_position(code_point, &tables::UNICODE_CC_INDEX)
    else {
        return 0;
    };

    loop {
        let byte = tables::UNICODE_CC_TABLE[position];
        position += 1;
        let class_type = byte >> 6;
        let mut run_len = u32::from(byte & 0x3f);
        if (48..56).contains(&run_len) {
            run_len = ((run_len - 48) << 8) | u32::from(tables::UNICODE_CC_TABLE[position]);
            position += 1;
            run_len += 48;
        } else if run_len >= 56 {
            run_len = ((run_len - 56) << 16)
                | (u32::from(tables::UNICODE_CC_TABLE[position]) << 8)
                | u32::from(tables::UNICODE_CC_TABLE[position + 1]);
            position += 2;
            run_len += 48 + (1 << 11);
        }
        let explicit_class = if class_type <= 1 {
            let class = tables::UNICODE_CC_TABLE[position];
            position += 1;
            Some(class)
        } else {
            None
        };
        let run_end = code + run_len + 1;
        if code_point < run_end {
            return match class_type {
                0 => u32::from(explicit_class.expect("class-zero run lost its payload")),
                1 => {
                    u32::from(explicit_class.expect("linear class run lost its payload"))
                        + code_point
                        - code
                }
                2 => 0,
                _ => 230,
            };
        }
        code = run_end;
    }
}

fn canonical_order(code_points: &mut [u32]) {
    let mut index = 0;
    while index < code_points.len() {
        if combining_class(code_points[index]) == 0 {
            index += 1;
            continue;
        }

        let start = index;
        let mut end = start + 1;
        while end < code_points.len() && combining_class(code_points[end]) != 0 {
            let code_point = code_points[end];
            let class = combining_class(code_point);
            let mut insertion = end;
            while insertion > start && combining_class(code_points[insertion - 1]) > class {
                code_points[insertion] = code_points[insertion - 1];
                insertion -= 1;
            }
            code_points[insertion] = code_point;
            end += 1;
        }
        index = end;
    }
}

fn unicode_compose_pair(first: u32, second: u32) -> Option<u32> {
    let mut lower = 0_usize;
    let mut upper = tables::UNICODE_COMP_TABLE.len();
    while lower < upper {
        let index = (lower + upper) / 2;
        let decomposition_index = tables::UNICODE_COMP_TABLE[index];
        let table_index = usize::from(decomposition_index >> 6);
        let run_offset = u32::from(decomposition_index & 0x3f);
        let entry = tables::UNICODE_DECOMP_TABLE1[table_index];
        let run_start = entry >> 14;
        let run_len = (entry >> 7) & 0x7f;
        let run_type = (entry >> 1) & 0x3f;
        let composed = run_start + run_offset;
        let mut pair = [0_u32; DECOMPOSITION_MAX_LEN];
        let pair_len = decomposition_entry(
            &mut pair,
            composed,
            table_index,
            run_start,
            run_len,
            run_type,
        );
        debug_assert_eq!(pair_len, 2);
        match (first, second).cmp(&(pair[0], pair[1])) {
            Ordering::Less => upper = index,
            Ordering::Greater => lower = index + 1,
            Ordering::Equal => return Some(composed),
        }
    }
    None
}

fn compose_pair(first: u32, second: u32) -> Option<u32> {
    if (0x1100..0x1100 + 19).contains(&first) && (0x1161..0x1161 + 21).contains(&second) {
        Some(0xac00 + (first - 0x1100) * 588 + (second - 0x1161) * 28)
    } else if (0xac00..0xac00 + 11_172).contains(&first)
        && (first - 0xac00) % 28 == 0
        && (0x11a7..0x11a7 + 28).contains(&second)
    {
        Some(first + second - 0x11a7)
    } else {
        unicode_compose_pair(first, second)
    }
}

fn compose(code_points: &mut Vec<u32>) {
    if code_points.len() <= 1 {
        return;
    }

    let input_len = code_points.len();
    let mut input = 1_usize;
    let mut output_len = 1_usize;
    while input < input_len {
        let mut last_class = combining_class(code_points[input]);
        let mut starter = output_len as isize - 1;
        let mut blocked = false;
        while starter >= 0 {
            let class = combining_class(code_points[starter as usize]);
            if class == 0 {
                break;
            }
            if class >= last_class {
                blocked = true;
                break;
            }
            last_class = 256;
            starter -= 1;
        }

        if !blocked && starter >= 0 {
            let starter = starter as usize;
            if let Some(composed) = compose_pair(code_points[starter], code_points[input]) {
                code_points[starter] = composed;
                input += 1;
                continue;
            }
        }

        code_points[output_len] = code_points[input];
        output_len += 1;
        input += 1;
    }
    code_points.truncate(output_len);
}

fn next_code_point(input: &JsString, index: &mut usize) -> u32 {
    let first = input
        .code_unit_at(*index)
        .expect("normalization advanced past its input");
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

pub(crate) fn normalize_code_points(
    input: &JsString,
    form: NormalizationForm,
) -> Result<Vec<u32>, JsStringError> {
    let mut output = Vec::new();
    try_reserve_exact(&mut output, input.len())?;

    if form == NormalizationForm::Nfc && input.utf16_units().all(|unit| unit < 0x100) {
        output.extend(input.utf16_units().map(u32::from));
        return Ok(output);
    }

    let mut index = 0;
    while index < input.len() {
        let code_point = next_code_point(input, &mut index);
        append_decomposed(&mut output, code_point, form.is_compatibility())?;
    }
    canonical_order(&mut output);
    if !form.is_decomposed() {
        compose(&mut output);
    }
    Ok(output)
}

fn string_from_code_points_with_limit(
    code_points: &[u32],
    max_len: usize,
) -> Result<JsString, JsStringError> {
    let max_len = max_len.min(JsString::MAX_LEN);
    let mut utf16_len = 0_usize;
    let mut wide = false;
    for code_point in code_points {
        debug_assert!(*code_point <= 0x10_ffff);
        utf16_len = utf16_len
            .checked_add(usize::from(*code_point >= 0x1_0000) + 1)
            .filter(|length| *length <= max_len)
            .ok_or(JsStringError::TooLong)?;
        wide |= *code_point >= 0x100;
    }

    if !wide {
        let mut output = Vec::new();
        try_reserve_exact(&mut output, utf16_len)?;
        output.extend(code_points.iter().map(|code_point| *code_point as u8));
        return Ok(JsString::from_owned_latin1(output));
    }

    let mut output = Vec::new();
    try_reserve_exact(&mut output, utf16_len)?;
    for code_point in code_points {
        if *code_point < 0x1_0000 {
            output.push(*code_point as u16);
        } else {
            let adjusted = *code_point - 0x1_0000;
            output.push(0xd800 | (adjusted >> 10) as u16);
            output.push(0xdc00 | (adjusted & 0x3ff) as u16);
        }
    }
    Ok(JsString::from_owned_utf16(output))
}

pub(crate) fn normalize_with_limit(
    input: &JsString,
    form: NormalizationForm,
    max_len: usize,
) -> Result<JsString, JsStringError> {
    if max_len >= JsString::MAX_LEN {
        return normalize(input, form);
    }
    let output = normalize_code_points(input, form)?;
    string_from_code_points_with_limit(&output, max_len)
}

/// Normalize an ECMAScript UTF-16 string with QuickJS 2026-06-04's Unicode 17
/// data. Valid surrogate pairs are decoded and re-encoded; lone surrogates are
/// intentionally preserved as lone code units.
pub(crate) fn normalize(
    input: &JsString,
    form: NormalizationForm,
) -> Result<JsString, JsStringError> {
    let output = normalize_code_points(input, form)?;
    string_from_code_points_with_limit(&output, JsString::MAX_LEN)
}

#[cfg(test)]
mod tests {
    use super::{
        DECOMPOSITION_MAX_LEN, NormalizationForm, combining_class, compose_pair, decomposition,
        decomposition_entry, fail_next_normalize_reservation_for_test, normalize,
        normalize_code_points, normalize_with_limit, push_fallible, tables,
    };
    use crate::value::{JsString, JsStringError};

    fn units(value: &JsString) -> Vec<u16> {
        value.utf16_units().collect()
    }

    fn normalized(source: &str, form: NormalizationForm) -> String {
        normalize(&JsString::try_from_utf8(source).unwrap(), form)
            .unwrap()
            .to_utf8_lossy()
    }

    #[test]
    fn generated_tables_are_pinned_to_quickjs_unicode_17() {
        assert_eq!(
            tables::SOURCE_SHA256,
            "cf782bc7a07549e976f606bd3cb8555858482b279574554dcb8d46412986006c"
        );
        assert_eq!(tables::UNICODE_CC_TABLE.len(), 937);
        assert_eq!(tables::UNICODE_CC_INDEX.len(), 90);
        assert_eq!(tables::UNICODE_DECOMP_TABLE1.len(), 709);
        assert_eq!(tables::UNICODE_DECOMP_TABLE2.len(), 709);
        assert_eq!(tables::UNICODE_DECOMP_DATA.len(), 9_452);
        assert_eq!(tables::UNICODE_COMP_TABLE.len(), 965);
    }

    #[test]
    fn canonical_and_compatibility_forms_match_quickjs_samples() {
        assert_eq!(normalized("e\u{301}", NormalizationForm::Nfc), "é");
        assert_eq!(normalized("é", NormalizationForm::Nfd), "e\u{301}");
        assert_eq!(normalized("\u{fb01}", NormalizationForm::Nfc), "ﬁ");
        assert_eq!(normalized("\u{fb01}", NormalizationForm::Nfkc), "fi");
        assert_eq!(normalized("\u{a0}", NormalizationForm::Nfkd), " ");
    }

    #[test]
    fn combining_marks_are_stably_ordered_and_block_composition() {
        assert_eq!(
            normalized("a\u{315}\u{300}", NormalizationForm::Nfc),
            "à\u{315}"
        );
        assert_eq!(
            normalized("a\u{301}\u{300}", NormalizationForm::Nfc),
            "á\u{300}"
        );
        assert_eq!(
            normalized("\u{344}", NormalizationForm::Nfc),
            "\u{308}\u{301}"
        );
        assert_eq!(combining_class(0x300), 230);
        assert_eq!(combining_class(0x315), 232);
        assert_eq!(combining_class(0x34f), 0);
    }

    #[test]
    fn hangul_decomposition_and_composition_follow_quickjs_boundaries() {
        assert_eq!(normalized("각", NormalizationForm::Nfd), "각");
        assert_eq!(normalized("각", NormalizationForm::Nfc), "각");
        assert_eq!(compose_pair(0x1100, 0x1161), Some(0xac00));
        assert_eq!(compose_pair(0xac00, 0x11a8), Some(0xac01));
        assert_eq!(compose_pair(0xac00, 0x11a7), Some(0xac00));
    }

    #[test]
    fn valid_pairs_are_decoded_while_lone_surrogates_are_preserved() {
        let input = JsString::try_from_utf16([0xd800, 0x65, 0x301, 0xdc00, 0xd83d, 0xde00, 0xdfff])
            .unwrap();
        let output = normalize(&input, NormalizationForm::Nfc).unwrap();
        assert_eq!(
            units(&output),
            [0xd800, 0xe9, 0xdc00, 0xd83d, 0xde00, 0xdfff]
        );
        assert_eq!(
            normalize_code_points(&input, NormalizationForm::Nfc).unwrap(),
            [0xd800, 0xe9, 0xdc00, 0x1f600, 0xdfff]
        );
    }

    #[test]
    fn output_uses_quickjs_narrow_or_wide_storage_from_content() {
        let narrow = normalize(
            &JsString::try_from_utf8("plain latin-1 ÿ").unwrap(),
            NormalizationForm::Nfc,
        )
        .unwrap();
        assert!(!narrow.is_wide());

        let narrowed = normalize(
            &JsString::try_from_utf8("\u{a0}").unwrap(),
            NormalizationForm::Nfkc,
        )
        .unwrap();
        assert_eq!(narrowed.to_utf8_lossy(), " ");
        assert!(!narrowed.is_wide());

        let composed_narrow = normalize(
            &JsString::try_from_utf8("e\u{301}").unwrap(),
            NormalizationForm::Nfc,
        )
        .unwrap();
        assert!(!composed_narrow.is_wide());

        let wide = normalize(
            &JsString::try_from_utf8("a\u{304}").unwrap(),
            NormalizationForm::Nfc,
        )
        .unwrap();
        assert!(wide.is_wide());
    }

    #[test]
    fn normalization_reports_the_utf16_output_length_limit() {
        let expands = JsString::try_from_utf8("\u{fd}ﬃ").unwrap();
        assert_eq!(
            normalize_with_limit(&expands, NormalizationForm::Nfkd, 4),
            Err(JsStringError::TooLong)
        );
        assert_eq!(
            normalize_with_limit(&expands, NormalizationForm::Nfkd, 5)
                .unwrap()
                .to_utf8_lossy(),
            "y\u{301}ffi"
        );

        let astral = JsString::try_from_utf8("😀").unwrap();
        assert_eq!(
            normalize_with_limit(&astral, NormalizationForm::Nfc, 1),
            Err(JsStringError::TooLong)
        );
    }

    #[test]
    fn normalization_reservation_failure_is_recoverable() {
        let input = JsString::try_from_utf8("e\u{301}").unwrap();
        fail_next_normalize_reservation_for_test();
        assert_eq!(
            normalize(&input, NormalizationForm::Nfc),
            Err(JsStringError::OutOfMemory)
        );
        assert_eq!(
            normalize(&input, NormalizationForm::Nfc)
                .unwrap()
                .to_utf8_lossy(),
            "é"
        );
    }

    #[test]
    fn fallible_output_growth_is_geometric_for_expanding_decompositions() {
        let mut values = Vec::new();
        let mut previous_capacity = values.capacity();
        let mut growths = 0;
        for value in 0..16_384 {
            push_fallible(&mut values, value).unwrap();
            if values.capacity() != previous_capacity {
                assert!(
                    values.capacity() >= previous_capacity + (previous_capacity / 2).max(1),
                    "normalization output did not retain QuickJS-style geometric growth",
                );
                previous_capacity = values.capacity();
                growths += 1;
            }
        }
        assert!(growths < 32, "normalization output grew {growths} times");

        let source = JsString::try_from_utf8(&"\u{fdFA}".repeat(1_024)).unwrap();
        let output = normalize(&source, NormalizationForm::Nfkd).unwrap();
        assert_eq!(output.len(), 18 * 1_024);
    }

    #[test]
    fn pinned_decomposition_and_composition_samples_are_stable() {
        let mut mapping = [0_u32; DECOMPOSITION_MAX_LEN];
        let len = decomposition(&mut mapping, 0x1e69, false);
        assert_eq!(&mapping[..len], [0x1e63, 0x307]);
        let len = decomposition(&mut mapping, 0xfb03, false);
        assert_eq!(len, 0);
        let len = decomposition(&mut mapping, 0xfb03, true);
        assert_eq!(&mapping[..len], [0x66, 0x66, 0x69]);
        assert_eq!(compose_pair(0x1e63, 0x307), Some(0x1e69));
        assert_eq!(compose_pair(0x308, 0x301), Some(0x344));
    }

    #[test]
    fn exhaustive_table_readers_match_pinned_quickjs_c_fingerprints() {
        fn hash_u32(mut hash: u64, value: u32) -> u64 {
            for byte in value.to_le_bytes() {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(1_099_511_628_211);
            }
            hash
        }

        let mut decomposition_hash = 14_695_981_039_346_656_037_u64;
        let mut class_hash = 14_695_981_039_346_656_037_u64;
        let mut canonical_count = 0_u32;
        let mut compatibility_count = 0_u32;
        let mut nonzero_class_count = 0_u32;
        let mut mapping = [0_u32; DECOMPOSITION_MAX_LEN];
        for code_point in 0..=0x10_ffff {
            let class = combining_class(code_point);
            nonzero_class_count += u32::from(class != 0);
            class_hash = hash_u32(class_hash, code_point);
            class_hash = hash_u32(class_hash, class);

            for compatibility in [false, true] {
                let len = decomposition(&mut mapping, code_point, compatibility);
                if compatibility {
                    compatibility_count += u32::from(len != 0);
                } else {
                    canonical_count += u32::from(len != 0);
                }
                decomposition_hash = hash_u32(decomposition_hash, code_point);
                decomposition_hash = hash_u32(decomposition_hash, u32::from(compatibility));
                decomposition_hash = hash_u32(decomposition_hash, len as u32);
                for mapped in &mapping[..len] {
                    decomposition_hash = hash_u32(decomposition_hash, *mapped);
                }
            }
        }

        assert_eq!(canonical_count, 2_081);
        assert_eq!(compatibility_count, 5_914);
        assert_eq!(nonzero_class_count, 968);
        assert_eq!(decomposition_hash, 6_126_396_769_325_200_388);
        assert_eq!(class_hash, 2_580_281_225_329_042_492);

        let mut composition_hash = 14_695_981_039_346_656_037_u64;
        for (index, decomposition_index) in tables::UNICODE_COMP_TABLE.iter().enumerate() {
            let table_index = usize::from(decomposition_index >> 6);
            let run_offset = u32::from(decomposition_index & 0x3f);
            let entry = tables::UNICODE_DECOMP_TABLE1[table_index];
            let run_start = entry >> 14;
            let run_len = (entry >> 7) & 0x7f;
            let run_type = (entry >> 1) & 0x3f;
            let composed = run_start + run_offset;
            let pair_len = decomposition_entry(
                &mut mapping,
                composed,
                table_index,
                run_start,
                run_len,
                run_type,
            );
            let actual = compose_pair(mapping[0], mapping[1]).unwrap();
            composition_hash = hash_u32(composition_hash, index as u32);
            composition_hash = hash_u32(composition_hash, pair_len as u32);
            composition_hash = hash_u32(composition_hash, mapping[0]);
            composition_hash = hash_u32(composition_hash, mapping[1]);
            composition_hash = hash_u32(composition_hash, actual);
        }
        assert_eq!(composition_hash, 17_411_631_189_690_117_515);
    }
}
