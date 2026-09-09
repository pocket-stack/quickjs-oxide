// Keep the JSON oracle implementations in isolated modules while Cargo builds
// one integration target.

mod oracle_json_parse;
mod oracle_json_raw;
mod oracle_json_stringify;
