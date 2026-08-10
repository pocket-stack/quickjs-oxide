// Keep the primitive-intrinsic oracle implementations in isolated modules
// while Cargo builds one integration target.

#[path = "oracle/primitive_intrinsics/oracle_bigint_intrinsic.rs"]
mod oracle_bigint_intrinsic;
#[path = "oracle/primitive_intrinsics/oracle_boolean_intrinsic.rs"]
mod oracle_boolean_intrinsic;
#[path = "oracle/primitive_intrinsics/oracle_symbol_intrinsic.rs"]
mod oracle_symbol_intrinsic;
