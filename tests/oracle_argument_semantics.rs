// Keep the arguments oracle implementations in isolated modules while Cargo
// builds one integration target.

use crate::quickjs_argv_completion_oracle;

#[path = "oracle/arguments/oracle_argument_spread.rs"]
mod oracle_argument_spread;
#[path = "oracle/arguments/oracle_arguments.rs"]
mod oracle_arguments;
