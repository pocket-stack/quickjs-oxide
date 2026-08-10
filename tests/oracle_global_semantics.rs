// Keep the global-semantics oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/global/oracle_global_numeric_predicates.rs"]
mod oracle_global_numeric_predicates;
#[path = "oracle/global/oracle_global_this.rs"]
mod oracle_global_this;
#[path = "oracle/global/oracle_global_to_string_tag.rs"]
mod oracle_global_to_string_tag;
#[path = "oracle/global/oracle_global_uri_codecs.rs"]
mod oracle_global_uri_codecs;
