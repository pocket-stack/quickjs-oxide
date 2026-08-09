// Keep the update-expression oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/update/oracle_update_expressions.rs"]
mod oracle_update_expressions;
#[path = "oracle/update/oracle_update_function_constructor.rs"]
mod oracle_update_function_constructor;
#[path = "oracle/update/oracle_update_numeric_matrix.rs"]
mod oracle_update_numeric_matrix;
