// Keep the Unicode lexical oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::quickjs_syntax_diagnostic_oracle;

mod oracle_unicode_identifiers;
mod oracle_unicode_u180e;
