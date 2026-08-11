// Keep the program declaration oracle implementations in isolated modules
// while Cargo builds one integration target.

use crate::quickjs_argv_completion_oracle;
#[path = "support/quickjs_program_property_oracle.rs"]
mod quickjs_program_property_oracle;

#[path = "oracle/program_declarations/oracle_program_functions.rs"]
mod oracle_program_functions;
#[path = "oracle/program_declarations/oracle_program_lexicals.rs"]
mod oracle_program_lexicals;
#[path = "oracle/program_declarations/oracle_program_vars.rs"]
mod oracle_program_vars;
