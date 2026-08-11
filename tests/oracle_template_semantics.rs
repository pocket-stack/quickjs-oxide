// Keep the template-semantics oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::quickjs_syntax_diagnostic_oracle;

#[path = "oracle/templates/oracle_tagged_templates.rs"]
mod oracle_tagged_templates;
#[path = "oracle/templates/oracle_template_literals.rs"]
mod oracle_template_literals;
