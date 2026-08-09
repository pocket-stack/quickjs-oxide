// Keep the TypedArray method oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/typed_array/oracle_typed_array_from.rs"]
mod oracle_typed_array_from;
#[path = "oracle/typed_array/oracle_typed_array_iteration.rs"]
mod oracle_typed_array_iteration;
#[path = "oracle/typed_array/oracle_typed_array_of.rs"]
mod oracle_typed_array_of;
