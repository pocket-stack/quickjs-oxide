// Keep the numeric conversion and text kernels in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/number_kernels/oracle_bigint_to_number.rs"]
mod oracle_bigint_to_number;
#[path = "oracle/number_kernels/oracle_global_number_parsers.rs"]
mod oracle_global_number_parsers;
#[path = "oracle/number_kernels/oracle_number_formatting_kernel.rs"]
mod oracle_number_formatting_kernel;
#[path = "oracle/number_kernels/oracle_number_parse_kernel.rs"]
mod oracle_number_parse_kernel;
