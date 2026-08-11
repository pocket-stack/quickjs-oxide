// Keep the collection oracle implementations in separate modules so their
// private helpers remain isolated while Cargo builds one integration target.

// This target uses the completion protocol; other aggregate targets also use
// the std-lines protocol from the same shared support module.
#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

#[allow(dead_code)]
#[path = "support/quickjs_oracle.rs"]
mod quickjs_oracle;

#[path = "oracle/collections/oracle_map.rs"]
mod oracle_map;
#[path = "oracle/collections/oracle_set.rs"]
mod oracle_set;
#[path = "oracle/collections/oracle_weak_collections.rs"]
mod oracle_weak_collections;
