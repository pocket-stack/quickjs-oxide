// Keep the function-declaration oracle implementations in isolated modules
// while Cargo builds one integration target.

#[path = "support/quickjs_argv_completion_oracle.rs"]
mod quickjs_argv_completion_oracle;

#[path = "oracle/function_declarations/oracle_block_functions.rs"]
mod oracle_block_functions;
#[path = "oracle/function_declarations/oracle_function_body_declarations.rs"]
mod oracle_function_body_declarations;
#[path = "oracle/function_declarations/oracle_function_body_lexicals.rs"]
mod oracle_function_body_lexicals;
