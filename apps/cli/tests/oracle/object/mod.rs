// Keep the Object oracle implementations in separate modules so their private
// helpers remain isolated while Cargo builds one integration target.

use crate::support::{quickjs_object_pattern_oracle, quickjs_object_super_oracle};
use crate::{quickjs_array_completion_oracle, quickjs_oracle};

#[cfg(test)]
mod quickjs_object_pattern_oracle_contract {
    use super::quickjs_object_pattern_oracle::{observe_completion_strip_one_lf, run_cli_exact};

    fn oracle() -> Option<std::ffi::OsString> {
        let oracle = std::env::var_os("QJS_ORACLE");
        if oracle.is_none() {
            eprintln!(
                "SKIP object-pattern oracle helper regressions: set QJS_ORACLE to upstream qjs"
            );
        }
        oracle
    }

    #[test]
    fn completion_removes_only_the_print_line_feed() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            observe_completion_strip_one_lf(
                &oracle,
                "'tail  \\n\\t'",
                "object-pattern trailing whitespace",
            ),
            "return|string|tail  \n\t",
        );
    }

    #[test]
    fn exact_cli_keeps_stdout_and_cmdline_diagnostics() {
        let Some(oracle) = oracle() else {
            return;
        };

        let success = run_cli_exact(&oracle, "print('ok')", "successful raw CLI contract");
        assert!(success.status.success());
        assert_eq!(String::from_utf8(success.stdout).unwrap(), "ok\n");
        assert!(success.stderr.is_empty());

        let syntax = run_cli_exact(&oracle, "let {", "syntax raw CLI contract");
        assert!(!syntax.status.success());
        assert_eq!(syntax.status.code(), Some(1));
        assert!(syntax.stdout.is_empty());
        let stderr = String::from_utf8(syntax.stderr).unwrap();
        assert!(stderr.contains("SyntaxError: variable name expected"));
        assert!(stderr.contains("at <cmdline>:1:5"));
    }
}

#[cfg(test)]
mod quickjs_object_super_oracle_contract {
    use super::quickjs_object_super_oracle::observe_completion_name_only;

    fn oracle() -> Option<std::ffi::OsString> {
        let oracle = std::env::var_os("QJS_ORACLE");
        if oracle.is_none() {
            eprintln!(
                "SKIP object-super oracle helper regressions: set QJS_ORACLE to upstream qjs"
            );
        }
        oracle
    }

    #[test]
    fn preserves_argv_source_and_completion_whitespace() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            observe_completion_name_only(&oracle, "-1", "contract", "leading hyphen source"),
            "return|number|-1",
        );
        assert_eq!(
            observe_completion_name_only(
                &oracle,
                "'tail  \\n\\t\\r\\u00a0'",
                "contract",
                "trailing whitespace",
            ),
            "return|string|tail  \n\t\r\u{00a0}",
        );
    }

    #[test]
    fn preserves_name_only_object_and_primitive_throw_protocol() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            observe_completion_name_only(
                &oracle,
                "throw new TypeError('message must stay omitted')",
                "contract",
                "object throw",
            ),
            "throw|object|TypeError",
        );
        assert_eq!(
            observe_completion_name_only(
                &oracle,
                "throw 'tail  \\n\\t\\r\\u00a0'",
                "contract",
                "primitive throw",
            ),
            "throw|string|tail  \n\t\r\u{00a0}",
        );
    }
}

mod oracle_object_accessors;
mod oracle_object_assign;
mod oracle_object_assignment;
mod oracle_object_bindings;
mod oracle_object_descriptors;
mod oracle_object_enumeration;
mod oracle_object_extensibility;
mod oracle_object_from_entries;
mod oracle_object_group_by;
mod oracle_object_has_own;
mod oracle_object_integrity;
mod oracle_object_intrinsic;
mod oracle_object_is;
mod oracle_object_literals;
mod oracle_object_methods;
mod oracle_object_rest;
mod oracle_object_super;
mod oracle_object_super_arrow;
mod oracle_object_super_eval;
mod oracle_objects;
