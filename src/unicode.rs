//! Checksum-pinned Unicode 17 primitives used by the JavaScript lexer.
//!
//! QuickJS stores binary Unicode properties as alternating run lengths with a
//! sparse 32-byte-block index. Keeping that representation avoids silently
//! inheriting the Rust toolchain or host ICU Unicode version while preserving
//! the pinned engine's exact table boundaries.

mod ident_tables {
    include!("unicode_ident_tables.rs");
}

const INDEX_BLOCK_LEN: usize = 32;
const CODE_MASK: u32 = (1 << 21) - 1;

#[must_use]
pub(crate) fn is_id_start(code_point: u32) -> bool {
    is_in_table(
        code_point,
        &ident_tables::ID_START_TABLE,
        &ident_tables::ID_START_INDEX,
    )
}

#[must_use]
pub(crate) fn is_id_continue(code_point: u32) -> bool {
    is_id_start(code_point)
        || is_in_table(
            code_point,
            &ident_tables::ID_CONTINUE_TABLE,
            &ident_tables::ID_CONTINUE_INDEX,
        )
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
    let code = entry & CODE_MASK;
    let position = (lower + 1) * INDEX_BLOCK_LEN + (entry >> 21) as usize;
    Some((code, position))
}

fn read_u24(index: &[u8], entry: usize) -> u32 {
    let offset = entry * 3;
    u32::from(index[offset])
        | (u32::from(index[offset + 1]) << 8)
        | (u32::from(index[offset + 2]) << 16)
}

#[cfg(test)]
mod tests {
    use super::{is_id_continue, is_id_start};

    #[test]
    fn unicode_17_identifier_tables_have_pinned_counts_and_boundaries() {
        let mut start_count = 0;
        let mut continue_count = 0;
        for code_point in 0..=0x10_ffff {
            let start = is_id_start(code_point);
            let continuation = is_id_continue(code_point);
            assert!(!start || continuation, "U+{code_point:04X}");
            start_count += usize::from(start);
            continue_count += usize::from(continuation);
        }
        assert_eq!(start_count, 145_916);
        assert_eq!(continue_count, 149_240);

        for code_point in [0x03c0, 0x10400, 0x33479] {
            assert!(is_id_start(code_point), "U+{code_point:04X}");
        }
        for code_point in [0x0024, 0x0030, 0x0300, 0x1f600, 0x3347a] {
            assert!(!is_id_start(code_point), "U+{code_point:04X}");
        }
        for code_point in [0x0030, 0x005f, 0x0300, 0x200c, 0x200d] {
            assert!(is_id_continue(code_point), "U+{code_point:04X}");
        }
    }
}
