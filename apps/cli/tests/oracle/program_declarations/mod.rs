// Keep the program declaration oracle implementations in isolated modules
// while Cargo builds one integration target.

use crate::quickjs_argv_completion_oracle;
use crate::support::quickjs_program_property_oracle;

mod oracle_program_functions;
mod oracle_program_lexicals;
mod oracle_program_vars;
