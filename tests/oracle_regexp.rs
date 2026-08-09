// Keep the RegExp oracle implementations in separate modules so their private
// helpers remain isolated while Cargo builds one integration target.

#[path = "oracle/regexp/oracle_regexp_backreferences.rs"]
mod oracle_regexp_backreferences;
#[path = "oracle/regexp/oracle_regexp_compile.rs"]
mod oracle_regexp_compile;
#[path = "oracle/regexp/oracle_regexp_dotall.rs"]
mod oracle_regexp_dotall;
#[path = "oracle/regexp/oracle_regexp_engine.rs"]
mod oracle_regexp_engine;
#[path = "oracle/regexp/oracle_regexp_intrinsic.rs"]
mod oracle_regexp_intrinsic;
#[path = "oracle/regexp/oracle_regexp_lookahead.rs"]
mod oracle_regexp_lookahead;
#[path = "oracle/regexp/oracle_regexp_lookbehind.rs"]
mod oracle_regexp_lookbehind;
#[path = "oracle/regexp/oracle_regexp_match_all.rs"]
mod oracle_regexp_match_all;
#[path = "oracle/regexp/oracle_regexp_match_indices.rs"]
mod oracle_regexp_match_indices;
#[path = "oracle/regexp/oracle_regexp_modifiers.rs"]
mod oracle_regexp_modifiers;
#[path = "oracle/regexp/oracle_regexp_named_groups.rs"]
mod oracle_regexp_named_groups;
#[path = "oracle/regexp/oracle_regexp_replace.rs"]
mod oracle_regexp_replace;
#[path = "oracle/regexp/oracle_regexp_split.rs"]
mod oracle_regexp_split;
#[path = "oracle/regexp/oracle_regexp_unicode_properties.rs"]
mod oracle_regexp_unicode_properties;
#[path = "oracle/regexp/oracle_regexp_v_character_class_escapes.rs"]
mod oracle_regexp_v_character_class_escapes;
