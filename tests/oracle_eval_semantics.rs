// Keep the eval oracle implementations in isolated modules while Cargo builds
// one integration target.

#[path = "oracle/eval/oracle_eval_intrinsic.rs"]
mod oracle_eval_intrinsic;
#[path = "oracle/eval/oracle_eval_var_destructuring.rs"]
mod oracle_eval_var_destructuring;
#[path = "oracle/eval/oracle_eval_wtf8_source.rs"]
mod oracle_eval_wtf8_source;
