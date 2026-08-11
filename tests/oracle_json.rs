// Keep the JSON oracle implementations in isolated modules while Cargo builds
// one integration target.

#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

#[path = "oracle/json/oracle_json_parse.rs"]
mod oracle_json_parse;
#[path = "oracle/json/oracle_json_raw.rs"]
mod oracle_json_raw;
#[path = "oracle/json/oracle_json_stringify.rs"]
mod oracle_json_stringify;
