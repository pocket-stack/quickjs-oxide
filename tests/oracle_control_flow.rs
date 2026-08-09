// Keep the control-flow oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/control_flow/oracle_statement_control_flow.rs"]
mod oracle_statement_control_flow;
#[path = "oracle/control_flow/oracle_switch_control_flow.rs"]
mod oracle_switch_control_flow;
#[path = "oracle/control_flow/oracle_try_catch_finally.rs"]
mod oracle_try_catch_finally;
