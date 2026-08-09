// Keep the async method oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/async_methods/oracle_async_class_method.rs"]
mod oracle_async_class_method;
#[path = "oracle/async_methods/oracle_async_generator_class_method.rs"]
mod oracle_async_generator_class_method;
#[path = "oracle/async_methods/oracle_async_generator_object_method.rs"]
mod oracle_async_generator_object_method;
#[path = "oracle/async_methods/oracle_async_generator_private_class_method.rs"]
mod oracle_async_generator_private_class_method;
#[path = "oracle/async_methods/oracle_async_object_method.rs"]
mod oracle_async_object_method;
#[path = "oracle/async_methods/oracle_async_private_class_method.rs"]
mod oracle_async_private_class_method;
