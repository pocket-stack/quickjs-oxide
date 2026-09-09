// Keep the parameter oracle implementations in isolated modules while Cargo
// builds one integration target.

use crate::quickjs_argv_completion_oracle;

mod oracle_identifier_default_parameters;
mod oracle_parameter_binding_patterns;
mod oracle_parameter_direct_eval;
mod oracle_parameter_expression_binding_patterns;
mod oracle_rest_parameters;
