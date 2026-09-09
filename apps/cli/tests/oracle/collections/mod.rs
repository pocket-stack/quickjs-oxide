// Keep the collection oracle implementations in separate modules so their
// private helpers remain isolated while Cargo builds one integration target.

// This target uses the completion protocol; other aggregate targets also use
// the std-lines protocol from the same shared support module.
use crate::quickjs_oracle;

mod oracle_map;
mod oracle_set;
mod oracle_weak_collections;
