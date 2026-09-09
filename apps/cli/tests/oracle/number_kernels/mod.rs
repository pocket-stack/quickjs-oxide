// Keep the numeric conversion and text kernels in isolated modules while
// Cargo builds one integration target.

mod oracle_bigint_to_number;
mod oracle_global_number_parsers;
mod oracle_number_formatting_kernel;
mod oracle_number_parse_kernel;
