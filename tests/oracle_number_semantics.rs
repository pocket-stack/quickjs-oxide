// Keep the Number oracle implementations in isolated modules while Cargo
// builds one integration target.

#[path = "oracle/number/oracle_number_constructor_conversion.rs"]
mod oracle_number_constructor_conversion;
#[path = "oracle/number/oracle_number_intrinsic.rs"]
mod oracle_number_intrinsic;
