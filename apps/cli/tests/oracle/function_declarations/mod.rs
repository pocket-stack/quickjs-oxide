// Keep the function-declaration oracle implementations in isolated modules
// while Cargo builds one integration target.

use crate::quickjs_argv_completion_oracle;

mod oracle_block_functions;
mod oracle_function_body_declarations;
mod oracle_function_body_lexicals;
