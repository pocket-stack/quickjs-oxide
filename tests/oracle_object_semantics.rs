// Keep the Object oracle implementations in separate modules so their private
// helpers remain isolated while Cargo builds one integration target.

#[path = "support/quickjs_object_pattern_oracle.rs"]
mod quickjs_object_pattern_oracle;
#[path = "support/quickjs_object_super_oracle.rs"]
mod quickjs_object_super_oracle;
#[path = "support/quickjs_oracle.rs"]
mod quickjs_oracle;

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

#[path = "oracle/object/oracle_object_accessors.rs"]
mod oracle_object_accessors;
#[path = "oracle/object/oracle_object_assign.rs"]
mod oracle_object_assign;
#[path = "oracle/object/oracle_object_assignment.rs"]
mod oracle_object_assignment;
#[path = "oracle/object/oracle_object_bindings.rs"]
mod oracle_object_bindings;
#[path = "oracle/object/oracle_object_descriptors.rs"]
mod oracle_object_descriptors;
#[path = "oracle/object/oracle_object_enumeration.rs"]
mod oracle_object_enumeration;
#[path = "oracle/object/oracle_object_extensibility.rs"]
mod oracle_object_extensibility;
#[path = "oracle/object/oracle_object_from_entries.rs"]
mod oracle_object_from_entries;
#[path = "oracle/object/oracle_object_group_by.rs"]
mod oracle_object_group_by;
#[path = "oracle/object/oracle_object_has_own.rs"]
mod oracle_object_has_own;
#[path = "oracle/object/oracle_object_integrity.rs"]
mod oracle_object_integrity;
#[path = "oracle/object/oracle_object_intrinsic.rs"]
mod oracle_object_intrinsic;
#[path = "oracle/object/oracle_object_is.rs"]
mod oracle_object_is;
#[path = "oracle/object/oracle_object_literals.rs"]
mod oracle_object_literals;
#[path = "oracle/object/oracle_object_methods.rs"]
mod oracle_object_methods;
#[path = "oracle/object/oracle_object_rest.rs"]
mod oracle_object_rest;
#[path = "oracle/object/oracle_object_super.rs"]
mod oracle_object_super;
#[path = "oracle/object/oracle_object_super_arrow.rs"]
mod oracle_object_super_arrow;
#[path = "oracle/object/oracle_object_super_eval.rs"]
mod oracle_object_super_eval;
#[path = "oracle/object/oracle_objects.rs"]
mod oracle_objects;
