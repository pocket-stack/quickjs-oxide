// Keep the Error oracle implementations in isolated modules while Cargo
// builds one integration target.

use crate::quickjs_argv_completion_oracle;

mod oracle_aggregate_error;
mod oracle_error_stacks;
mod oracle_errors;
mod oracle_native_error_atom_format;
mod oracle_native_error_format;
