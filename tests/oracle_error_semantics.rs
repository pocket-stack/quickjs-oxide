// Keep the Error oracle implementations in isolated modules while Cargo
// builds one integration target.

use crate::quickjs_argv_completion_oracle;

#[path = "oracle/errors/oracle_aggregate_error.rs"]
mod oracle_aggregate_error;
#[path = "oracle/errors/oracle_error_stacks.rs"]
mod oracle_error_stacks;
#[path = "oracle/errors/oracle_errors.rs"]
mod oracle_errors;
#[path = "oracle/errors/oracle_native_error_atom_format.rs"]
mod oracle_native_error_atom_format;
#[path = "oracle/errors/oracle_native_error_format.rs"]
mod oracle_native_error_format;
