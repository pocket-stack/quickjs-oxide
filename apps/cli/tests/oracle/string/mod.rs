// Keep the String oracle implementations in separate modules so their private
// helpers remain isolated while Cargo builds one integration target.

use crate::quickjs_oracle;

mod oracle_string_byte_codec;
mod oracle_string_case;
mod oracle_string_conversion_core;
mod oracle_string_create_html;
mod oracle_string_exotic;
mod oracle_string_includes;
mod oracle_string_index_search;
mod oracle_string_intrinsic;
mod oracle_string_match;
mod oracle_string_match_all;
mod oracle_string_pad;
mod oracle_string_repeat;
mod oracle_string_replace;
mod oracle_string_rope;
mod oracle_string_search;
mod oracle_string_split;
mod oracle_string_subrange;
mod oracle_string_trim;
mod oracle_string_utf16_prefix;
