// Keep the TypedArray method oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::support::quickjs_typed_array_oracle;

mod oracle_typed_array_from;
mod oracle_typed_array_iteration;
mod oracle_typed_array_of;
