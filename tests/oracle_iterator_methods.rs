// Keep the Iterator method oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "support/quickjs_string_result_oracle.rs"]
mod quickjs_string_result_oracle;

#[path = "oracle/iterator/oracle_iterator_concat.rs"]
mod oracle_iterator_concat;
#[path = "oracle/iterator/oracle_iterator_helpers.rs"]
mod oracle_iterator_helpers;
