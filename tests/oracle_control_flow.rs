// Keep the control-flow oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "support/quickjs_argv_completion_oracle.rs"]
mod quickjs_argv_completion_oracle;
#[path = "support/quickjs_control_value_oracle.rs"]
mod quickjs_control_value_oracle;
#[path = "support/quickjs_syntax_diagnostic_oracle.rs"]
mod quickjs_syntax_diagnostic_oracle;

#[path = "oracle/control_flow/oracle_statement_control_flow.rs"]
mod oracle_statement_control_flow;
#[path = "oracle/control_flow/oracle_switch_control_flow.rs"]
mod oracle_switch_control_flow;
#[path = "oracle/control_flow/oracle_try_catch_finally.rs"]
mod oracle_try_catch_finally;
