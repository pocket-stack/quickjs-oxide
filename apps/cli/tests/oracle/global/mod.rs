// Keep the global-semantics oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::support::quickjs_plain_eval_oracle;

mod oracle_global_numeric_predicates;
mod oracle_global_this;
mod oracle_global_to_string_tag;
mod oracle_global_uri_codecs;
