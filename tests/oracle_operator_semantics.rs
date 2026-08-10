// Keep the expression-operator oracle implementations in isolated modules
// while Cargo builds one integration target.

#[path = "oracle/operators/oracle_identifier_delete.rs"]
mod oracle_identifier_delete;
#[path = "oracle/operators/oracle_optional_chaining.rs"]
mod oracle_optional_chaining;
#[path = "oracle/operators/oracle_relational_membership.rs"]
mod oracle_relational_membership;
