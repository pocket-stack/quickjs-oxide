// Keep the eval oracle implementations in isolated modules while Cargo builds
// one integration target.

mod oracle_eval_intrinsic;
mod oracle_eval_var_destructuring;
mod oracle_eval_wtf8_source;
