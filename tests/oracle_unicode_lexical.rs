// Keep the Unicode lexical oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::quickjs_syntax_diagnostic_oracle;

#[path = "oracle/unicode_lexical/oracle_unicode_identifiers.rs"]
mod oracle_unicode_identifiers;
#[path = "oracle/unicode_lexical/oracle_unicode_u180e.rs"]
mod oracle_unicode_u180e;
