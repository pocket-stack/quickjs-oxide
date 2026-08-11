// Keep the control-flow oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::quickjs_argv_completion_oracle;
#[path = "support/quickjs_control_value_oracle.rs"]
mod quickjs_control_value_oracle;
use crate::quickjs_syntax_diagnostic_oracle;

#[path = "oracle/control_flow/oracle_annex_b_statements.rs"]
mod oracle_annex_b_statements;
#[path = "oracle/control_flow/oracle_catch_destructuring.rs"]
mod oracle_catch_destructuring;
#[path = "oracle/control_flow/oracle_for_in.rs"]
mod oracle_for_in;
#[path = "oracle/control_flow/oracle_for_lexicals.rs"]
mod oracle_for_lexicals;
#[path = "oracle/control_flow/oracle_for_of.rs"]
mod oracle_for_of;
#[path = "oracle/control_flow/oracle_statement_control_flow.rs"]
mod oracle_statement_control_flow;
#[path = "oracle/control_flow/oracle_switch_control_flow.rs"]
mod oracle_switch_control_flow;
#[path = "oracle/control_flow/oracle_try_catch_finally.rs"]
mod oracle_try_catch_finally;
#[path = "oracle/control_flow/oracle_with.rs"]
mod oracle_with;
