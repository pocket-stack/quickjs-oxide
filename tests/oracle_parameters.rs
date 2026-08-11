// Keep the parameter oracle implementations in isolated modules while Cargo
// builds one integration target.

#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

#[path = "support/quickjs_argv_completion_oracle.rs"]
mod quickjs_argv_completion_oracle;

#[path = "oracle/parameters/oracle_identifier_default_parameters.rs"]
mod oracle_identifier_default_parameters;
#[path = "oracle/parameters/oracle_parameter_binding_patterns.rs"]
mod oracle_parameter_binding_patterns;
#[path = "oracle/parameters/oracle_parameter_direct_eval.rs"]
mod oracle_parameter_direct_eval;
#[path = "oracle/parameters/oracle_parameter_expression_binding_patterns.rs"]
mod oracle_parameter_expression_binding_patterns;
#[path = "oracle/parameters/oracle_rest_parameters.rs"]
mod oracle_rest_parameters;
