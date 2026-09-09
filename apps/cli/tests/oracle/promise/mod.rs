// Keep the Promise oracle implementations in separate modules so their
// private helpers remain isolated while Cargo builds one integration target.

mod oracle_promise_aggregates;
mod oracle_promise_all;
mod oracle_promise_finally;
mod oracle_promise_jobs;
mod oracle_promise_static;
