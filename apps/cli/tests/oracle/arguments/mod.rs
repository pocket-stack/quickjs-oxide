// Keep the arguments oracle implementations in isolated modules while Cargo
// builds one integration target.

use crate::quickjs_argv_completion_oracle;

mod oracle_argument_spread;
mod oracle_arguments;
