// Keep the ordinary-function oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/function_semantics/oracle_function_bind_to_string.rs"]
mod oracle_function_bind_to_string;
#[path = "oracle/function_semantics/oracle_function_constructor.rs"]
mod oracle_function_constructor;
#[path = "oracle/function_semantics/oracle_function_debug_accessors.rs"]
mod oracle_function_debug_accessors;
#[path = "oracle/function_semantics/oracle_function_dynamic_wtf8.rs"]
mod oracle_function_dynamic_wtf8;
#[path = "oracle/function_semantics/oracle_function_prototype_prefix.rs"]
mod oracle_function_prototype_prefix;
#[path = "oracle/function_semantics/oracle_functions.rs"]
mod oracle_functions;
