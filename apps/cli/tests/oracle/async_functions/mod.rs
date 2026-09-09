// Keep the async callable oracle implementations in isolated modules while
// Cargo builds one integration target.

mod oracle_async_arrow;
mod oracle_async_function;
mod oracle_async_generator;
mod oracle_async_generator_yield_star;
