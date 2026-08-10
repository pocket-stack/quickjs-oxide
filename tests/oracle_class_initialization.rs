// Keep the class-initialization oracle implementations in isolated modules
// while Cargo builds one integration target.

#[path = "oracle/class_initialization/oracle_class_derived.rs"]
mod oracle_class_derived;
#[path = "oracle/class_initialization/oracle_class_field_await.rs"]
mod oracle_class_field_await;
#[path = "oracle/class_initialization/oracle_class_public_init.rs"]
mod oracle_class_public_init;
