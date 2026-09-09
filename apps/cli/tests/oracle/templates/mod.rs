// Keep the template-semantics oracle implementations in isolated modules while
// Cargo builds one integration target.

use crate::quickjs_syntax_diagnostic_oracle;

mod oracle_tagged_templates;
mod oracle_template_literals;
