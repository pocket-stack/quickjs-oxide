// Keep the Unicode lexical oracle implementations in isolated modules while
// Cargo builds one integration target.

#[path = "support/runtime_oracle.rs"]
mod runtime_oracle;

#[path = "support/quickjs_syntax_diagnostic_oracle.rs"]
mod quickjs_syntax_diagnostic_oracle;

#[path = "oracle/unicode_lexical/oracle_unicode_identifiers.rs"]
mod oracle_unicode_identifiers;
#[path = "oracle/unicode_lexical/oracle_unicode_u180e.rs"]
mod oracle_unicode_u180e;
