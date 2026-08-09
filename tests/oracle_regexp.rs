// Keep the RegExp oracle implementations in separate modules so their private
// helpers remain isolated while Cargo builds one integration target.

#[path = "support/quickjs_oracle.rs"]
mod quickjs_oracle;

#[cfg(test)]
mod quickjs_oracle_contract {
    use super::quickjs_oracle::{eval_std_lines, observe_completion};

    fn oracle() -> Option<std::ffi::OsString> {
        let oracle = std::env::var_os("QJS_ORACLE");
        if oracle.is_none() {
            eprintln!("SKIP QuickJS oracle helper regressions: set QJS_ORACLE to upstream qjs");
        }
        oracle
    }

    #[test]
    fn preserves_completion_protocol_and_script_arguments() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            observe_completion(&oracle, "scriptArgs.length", "scriptArgs length"),
            "return|number|1",
        );
        assert_eq!(
            observe_completion(&oracle, "", "empty source"),
            "return|undefined|undefined",
        );
        assert_eq!(
            observe_completion(&oracle, "scriptArgs[0]", "scriptArgs[0] preservation"),
            "return|string|scriptArgs[0]",
        );
        assert_eq!(
            observe_completion(&oracle, "'tail  \\n\\t'", "trailing whitespace"),
            "return|string|tail",
        );
        assert_eq!(
            observe_completion(&oracle, "throw 17", "primitive throw"),
            "throw|number|17",
        );
        assert_eq!(
            observe_completion(
                &oracle,
                "throw new TypeError('boom')",
                "Error name and message",
            ),
            "throw|object|TypeError|boom",
        );
    }

    #[test]
    fn transports_unicode_and_embedded_nul() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            observe_completion(
                &oracle,
                "/*\0*/'雪😀|'+scriptArgs[0].charCodeAt(2)",
                "Unicode and embedded NUL",
            ),
            "return|string|雪😀|0",
        );
    }

    #[test]
    fn streams_source_larger_than_two_mebibytes() {
        let Some(oracle) = oracle() else {
            return;
        };
        let source = format!("/*{}*/\n42", "x".repeat(2 * 1024 * 1024 + 1));
        assert_eq!(
            observe_completion(&oracle, &source, "source larger than two MiB"),
            "return|number|42",
        );
    }

    #[test]
    fn drains_large_stdout_and_stderr_without_deadlock() {
        const PIPE_BYTES: usize = 256 * 1024;

        let Some(oracle) = oracle() else {
            return;
        };
        let observed = observe_completion(
            &oracle,
            "std.out.puts('o'.repeat(262144));std.err.puts('e'.repeat(262144));'done'",
            "large stdout and stderr",
        );
        assert_eq!(observed.len(), PIPE_BYTES + "return|string|done".len());
        assert!(
            observed.as_bytes()[..PIPE_BYTES]
                .iter()
                .all(|byte| *byte == b'o')
        );
        assert!(observed.ends_with("return|string|done"));
    }

    #[test]
    fn concurrent_observers_keep_sources_isolated() {
        let Some(oracle) = oracle() else {
            return;
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(|| observe_completion(&oracle, "'marker-one'", "first marker"));
            let second =
                scope.spawn(|| observe_completion(&oracle, "'marker-two'", "second marker"));
            assert_eq!(first.join().unwrap(), "return|string|marker-one");
            assert_eq!(second.join().unwrap(), "return|string|marker-two");
        });
    }

    #[test]
    fn std_lines_has_no_script_arguments_or_global_bootstrap_binding() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            eval_std_lines(
                &oracle,
                "print(scriptArgs.length+'|'+typeof source)",
                "std-lines script arguments and bindings",
            ),
            ["0|undefined"],
        );
    }

    #[test]
    fn std_lines_preserves_rust_line_iteration_semantics() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert!(eval_std_lines(&oracle, "", "empty std-lines source").is_empty());
        assert_eq!(
            eval_std_lines(
                &oracle,
                "print('first');print('second')",
                "multiline std-lines output",
            ),
            ["first", "second"],
        );
        assert_eq!(
            eval_std_lines(
                &oracle,
                "std.out.puts('first\\nsecond\\n\\n')",
                "trailing std-lines newlines",
            ),
            ["first", "second", ""],
        );
    }

    #[test]
    fn std_lines_transports_unicode_and_embedded_nul() {
        let Some(oracle) = oracle() else {
            return;
        };
        assert_eq!(
            eval_std_lines(
                &oracle,
                "/*\0*/print('雪😀')",
                "std-lines Unicode and embedded NUL",
            ),
            ["雪😀"],
        );
    }

    #[test]
    fn std_lines_streams_source_larger_than_two_mebibytes() {
        let Some(oracle) = oracle() else {
            return;
        };
        let source = format!("/*{}*/\nprint(42)", "x".repeat(2 * 1024 * 1024 + 1),);
        assert_eq!(
            eval_std_lines(&oracle, &source, "large std-lines source"),
            ["42"],
        );
    }

    #[test]
    fn std_lines_drains_large_stdout_and_stderr_without_deadlock() {
        const PIPE_BYTES: usize = 256 * 1024;

        let Some(oracle) = oracle() else {
            return;
        };
        let lines = eval_std_lines(
            &oracle,
            "std.out.puts('o'.repeat(262144));std.err.puts('e'.repeat(262144));print('done')",
            "large std-lines stdout and stderr",
        );
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].len(), PIPE_BYTES + "done".len());
        assert!(
            lines[0].as_bytes()[..PIPE_BYTES]
                .iter()
                .all(|byte| *byte == b'o')
        );
        assert!(lines[0].ends_with("done"));
    }
}

#[path = "oracle/regexp/oracle_regexp_backreferences.rs"]
mod oracle_regexp_backreferences;
#[path = "oracle/regexp/oracle_regexp_compile.rs"]
mod oracle_regexp_compile;
#[path = "oracle/regexp/oracle_regexp_dotall.rs"]
mod oracle_regexp_dotall;
#[path = "oracle/regexp/oracle_regexp_engine.rs"]
mod oracle_regexp_engine;
#[path = "oracle/regexp/oracle_regexp_intrinsic.rs"]
mod oracle_regexp_intrinsic;
#[path = "oracle/regexp/oracle_regexp_lookahead.rs"]
mod oracle_regexp_lookahead;
#[path = "oracle/regexp/oracle_regexp_lookbehind.rs"]
mod oracle_regexp_lookbehind;
#[path = "oracle/regexp/oracle_regexp_match_all.rs"]
mod oracle_regexp_match_all;
#[path = "oracle/regexp/oracle_regexp_match_indices.rs"]
mod oracle_regexp_match_indices;
#[path = "oracle/regexp/oracle_regexp_modifiers.rs"]
mod oracle_regexp_modifiers;
#[path = "oracle/regexp/oracle_regexp_named_groups.rs"]
mod oracle_regexp_named_groups;
#[path = "oracle/regexp/oracle_regexp_replace.rs"]
mod oracle_regexp_replace;
#[path = "oracle/regexp/oracle_regexp_split.rs"]
mod oracle_regexp_split;
#[path = "oracle/regexp/oracle_regexp_unicode_properties.rs"]
mod oracle_regexp_unicode_properties;
#[path = "oracle/regexp/oracle_regexp_v_character_class_escapes.rs"]
mod oracle_regexp_v_character_class_escapes;
