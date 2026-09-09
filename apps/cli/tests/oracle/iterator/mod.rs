// Keep the Iterator method oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::support::quickjs_string_result_oracle;

mod oracle_iterator_concat;
mod oracle_iterator_helpers;
