// Keep the async callable oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "oracle/async_functions/oracle_async_arrow.rs"]
mod oracle_async_arrow;
#[path = "oracle/async_functions/oracle_async_function.rs"]
mod oracle_async_function;
#[path = "oracle/async_functions/oracle_async_generator.rs"]
mod oracle_async_generator;
#[path = "oracle/async_functions/oracle_async_generator_yield_star.rs"]
mod oracle_async_generator_yield_star;
