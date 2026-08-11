// Keep the Proxy and Reflect oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

#[path = "oracle/proxy_reflect/oracle_proxy.rs"]
mod oracle_proxy;
#[path = "oracle/proxy_reflect/oracle_reflect_intrinsic.rs"]
mod oracle_reflect_intrinsic;
