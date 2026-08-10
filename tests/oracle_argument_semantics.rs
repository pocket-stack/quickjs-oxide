// Keep the arguments oracle implementations in isolated modules while Cargo
// builds one integration target.

#[path = "support/quickjs_argv_completion_oracle.rs"]
mod quickjs_argv_completion_oracle;

#[path = "oracle/arguments/oracle_argument_spread.rs"]
mod oracle_argument_spread;
#[path = "oracle/arguments/oracle_arguments.rs"]
mod oracle_arguments;
