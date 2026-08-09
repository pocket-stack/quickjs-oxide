// Keep the Array-method oracle implementations in separate modules so their
// private helpers remain isolated while Cargo builds one integration target.

#[path = "oracle/array/oracle_array_concat.rs"]
mod oracle_array_concat;
#[path = "oracle/array/oracle_array_copy_within.rs"]
mod oracle_array_copy_within;
#[path = "oracle/array/oracle_array_fill.rs"]
mod oracle_array_fill;
#[path = "oracle/array/oracle_array_find.rs"]
mod oracle_array_find;
#[path = "oracle/array/oracle_array_flatten.rs"]
mod oracle_array_flatten;
#[path = "oracle/array/oracle_array_iteration.rs"]
mod oracle_array_iteration;
#[path = "oracle/array/oracle_array_map_filter.rs"]
mod oracle_array_map_filter;
#[path = "oracle/array/oracle_array_mutators.rs"]
mod oracle_array_mutators;
#[path = "oracle/array/oracle_array_reduce.rs"]
mod oracle_array_reduce;
#[path = "oracle/array/oracle_array_reverse.rs"]
mod oracle_array_reverse;
#[path = "oracle/array/oracle_array_slice_splice.rs"]
mod oracle_array_slice_splice;
#[path = "oracle/array/oracle_array_sort.rs"]
mod oracle_array_sort;
#[path = "oracle/array/oracle_array_stringification.rs"]
mod oracle_array_stringification;
#[path = "oracle/array/oracle_array_with.rs"]
mod oracle_array_with;
