// Keep the lexical oracle implementations in isolated modules while Cargo
// builds one integration target.

use crate::quickjs_syntax_diagnostic_oracle;

mod oracle_string_escape_diagnostics;
