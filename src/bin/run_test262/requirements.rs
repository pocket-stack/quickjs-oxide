use std::collections::BTreeSet;
use std::path::Path;

use super::metadata::Metadata;

/// Return conservative, stable IDs for Test262 execution capabilities which
/// the current runner cannot provide.
///
/// Metadata is authoritative for declared execution modes. Includes and `$262`
/// source tokens are hints for host hooks: JavaScript can replace the writable
/// `$262` global, so the execution layer must still retain dynamic provenance
/// before treating one of those hook hints as the cause of a result.
pub(super) fn missing_host_capability_hints(
    path: &Path,
    source: &str,
    metadata: &Metadata,
    allow_async: bool,
) -> Vec<String> {
    let mut missing = BTreeSet::new();
    // Host-hook discovery is intentionally fail-closed: do not apply the
    // approximate RegExp lexical goal used by the scoped async audit, because
    // mistaking division for a literal could hide a real `$262` access.
    let tokens = source_tokens(source, false);

    if metadata.is_module() {
        missing.insert("module".to_owned());
    }
    if metadata.is_async() && !allow_async {
        missing.insert("async".to_owned());
    }
    if metadata.flags.contains("CanBlockIsFalse") {
        missing.insert("can-block:false".to_owned());
    }

    // These feature names are explicit Test262 host requirements at the
    // pinned suite revision. `cross-realm` is deliberately not mapped here:
    // that feature is neither necessary nor sufficient evidence that the test
    // actually calls `$262.createRealm`.
    if metadata
        .features
        .iter()
        .any(|feature| feature == "host-gc-required")
    {
        missing.insert("gc".to_owned());
    }
    if metadata
        .features
        .iter()
        .any(|feature| feature == "IsHTMLDDA")
    {
        missing.insert("is-html-dda".to_owned());
    }

    let shadows_host_262 = is_detach_helper_shadow_test(path, &tokens);

    // atomicsHelper.js immediately consumes `$262.agent`. The detach helper
    // normally consumes `$262.detachArrayBuffer` when the test calls it, except
    // for the harness self-test which intentionally installs its own `$262`.
    for include in &metadata.includes {
        match include.as_str() {
            "atomicsHelper.js" => {
                missing.insert("agent".to_owned());
            }
            "detachArrayBuffer.js" if !shadows_host_262 => {
                missing.insert("detach-array-buffer".to_owned());
            }
            // The QuickJS patch makes this an optional fast path with a
            // JavaScript fallback. Absence is not a host requirement.
            "regExpUtils.js" => {}
            _ => {}
        }
    }

    for hook in member_names(&tokens) {
        let capability = match hook {
            "agent" => Some("agent"),
            "createRealm" => Some("create-realm"),
            "evalScript" => Some("eval-script"),
            "detachArrayBuffer" => Some("detach-array-buffer"),
            "IsHTMLDDA" => Some("is-html-dda"),
            "gc" => Some("gc"),
            "AbstractModuleSource" => Some("abstract-module-source"),
            "global" => Some("global"),
            // codePointRange is a QuickJS-only optional optimization used by
            // patched harness code and must remain absent when unsupported so
            // `typeof` can select the fallback.
            "codePointRange" => None,
            unknown => {
                missing.insert(format!("unknown:$262.{unknown}"));
                None
            }
        };
        if let Some(capability) = capability {
            missing.insert(capability.to_owned());
        }
    }

    missing.into_iter().collect()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SourceToken<'source> {
    Identifier(&'source str),
    Dot,
    LeftParen,
    RightParen,
    LeftBracket,
    RightBracket,
    LeftBrace,
    RightBrace,
    Arrow,
    LineTerminator,
    Literal,
    Other(u8),
}

fn member_names<'source>(tokens: &[SourceToken<'source>]) -> Vec<&'source str> {
    significant_tokens(tokens)
        .windows(3)
        .filter_map(|window| match window {
            [
                SourceToken::Identifier("$262"),
                SourceToken::Dot,
                SourceToken::Identifier(name),
            ] => Some(*name),
            _ => None,
        })
        .collect()
}

fn is_detach_helper_shadow_test(path: &Path, tokens: &[SourceToken<'_>]) -> bool {
    let normalized = path.to_string_lossy().replace('\\', "/");
    normalized.ends_with("test/harness/detachArrayBuffer-host-detachArrayBuffer.js")
        && significant_tokens(tokens).windows(2).any(|window| {
            matches!(
                window,
                [
                    SourceToken::Identifier("var" | "let" | "const"),
                    SourceToken::Identifier("$262")
                ]
            )
        })
}

fn significant_tokens<'source>(tokens: &[SourceToken<'source>]) -> Vec<SourceToken<'source>> {
    tokens
        .iter()
        .copied()
        .filter(|token| !matches!(token, SourceToken::LineTerminator))
        .collect()
}

/// Return whether one test in the pinned generator/destructuring admission
/// cohort contains async function or async-arrow grammar which its
/// non-exhaustive feature metadata does not declare.
///
/// This is deliberately not a general JavaScript parser. The feature check
/// keeps the lexical audit inside the checksum-bound cohort whose synchronous
/// complement is independently run by the R3t gate. The coordinator uses it
/// only as the final admission guard after every authoritative classification
/// has accepted the test.
pub(super) fn generator_destructuring_source_needs_async_guard(
    source: &str,
    metadata: &Metadata,
) -> bool {
    metadata
        .features
        .iter()
        .any(|feature| matches!(feature.as_str(), "generators" | "destructuring-binding"))
        && contains_async_function_or_arrow_syntax(&source_tokens(source, true))
}

fn contains_async_function_or_arrow_syntax(tokens: &[SourceToken<'_>]) -> bool {
    for (index, token) in tokens.iter().enumerate() {
        if !matches!(token, SourceToken::Identifier("async")) {
            continue;
        }

        let Some((head_index, false)) = next_significant_token(tokens, index + 1) else {
            continue;
        };
        match tokens[head_index] {
            SourceToken::Identifier("function") => return true,
            SourceToken::Identifier(_) => {
                if let Some((next_index, crossed_line_terminator)) =
                    next_significant_token(tokens, head_index + 1)
                {
                    if matches!(tokens[next_index], SourceToken::Arrow) && !crossed_line_terminator
                    {
                        return true;
                    }
                }
            }
            SourceToken::LeftParen => {
                let mut depth = 1usize;
                let mut cursor = head_index + 1;
                while cursor < tokens.len() {
                    match tokens[cursor] {
                        SourceToken::LeftParen => depth += 1,
                        SourceToken::RightParen => {
                            depth -= 1;
                            if depth == 0 {
                                if matches!(
                                    next_significant_token(tokens, cursor + 1),
                                    Some((arrow_index, false))
                                        if matches!(tokens[arrow_index], SourceToken::Arrow)
                                ) {
                                    return true;
                                }
                                break;
                            }
                        }
                        _ => {}
                    }
                    cursor += 1;
                }
            }
            _ => {}
        }
    }
    false
}

/// Return the next code token and whether a line terminator occurred before it.
fn next_significant_token(tokens: &[SourceToken<'_>], mut index: usize) -> Option<(usize, bool)> {
    let mut crossed_line_terminator = false;
    while index < tokens.len() {
        if matches!(tokens[index], SourceToken::LineTerminator) {
            crossed_line_terminator = true;
            index += 1;
        } else {
            return Some((index, crossed_line_terminator));
        }
    }
    None
}

fn source_tokens(source: &str, skip_regexp_literals: bool) -> Vec<SourceToken<'_>> {
    let mut tokens = Vec::new();
    let mut index = 0;
    scan_code(source, &mut index, None, skip_regexp_literals, &mut tokens);
    tokens
}

/// Tokenize only the small lexical surface needed for `$262 . hook` hints and
/// async callable classification. Full parsing is intentionally avoided
/// because unsupported grammar is one of the things the Test262 runner
/// measures.
fn scan_code<'source>(
    source: &'source str,
    index: &mut usize,
    mut template_brace_depth: Option<usize>,
    skip_regexp_literals: bool,
    tokens: &mut Vec<SourceToken<'source>>,
) {
    let bytes = source.as_bytes();
    while *index < bytes.len() {
        if let Some(length) = line_terminator_length(bytes, *index) {
            push_line_terminator(tokens);
            *index += length;
            continue;
        }

        let byte = bytes[*index];
        let next = bytes.get(*index + 1).copied();
        match (byte, next) {
            (b'/', Some(b'/')) => skip_line_comment(bytes, index),
            (b'/', Some(b'*')) => {
                if skip_block_comment(bytes, index) {
                    push_line_terminator(tokens);
                }
            }
            (b'/', _)
                if skip_regexp_literals
                    && regexp_literal_allowed(tokens)
                    && skip_regexp_literal(bytes, index) =>
            {
                tokens.push(SourceToken::Literal);
            }
            (b'\'' | b'"', _) => {
                skip_quoted_string(bytes, index, byte);
                tokens.push(SourceToken::Literal);
            }
            (b'`', _) => scan_template(source, index, skip_regexp_literals, tokens),
            (b'{', _) if template_brace_depth.is_some() => {
                template_brace_depth = template_brace_depth.map(|depth| depth + 1);
                tokens.push(SourceToken::LeftBrace);
                *index += 1;
            }
            (b'}', _) if template_brace_depth.is_some() => {
                let depth = template_brace_depth.expect("template depth was checked");
                *index += 1;
                if depth == 1 {
                    return;
                }
                template_brace_depth = Some(depth - 1);
                tokens.push(SourceToken::RightBrace);
            }
            (b'.', _) => {
                tokens.push(SourceToken::Dot);
                *index += 1;
            }
            (b'(', _) => {
                tokens.push(SourceToken::LeftParen);
                *index += 1;
            }
            (b')', _) => {
                tokens.push(SourceToken::RightParen);
                *index += 1;
            }
            (b'[', _) => {
                tokens.push(SourceToken::LeftBracket);
                *index += 1;
            }
            (b']', _) => {
                tokens.push(SourceToken::RightBracket);
                *index += 1;
            }
            (b'{', _) => {
                tokens.push(SourceToken::LeftBrace);
                *index += 1;
            }
            (b'}', _) => {
                tokens.push(SourceToken::RightBrace);
                *index += 1;
            }
            (b'=', Some(b'>')) => {
                tokens.push(SourceToken::Arrow);
                *index += 2;
            }
            (byte, _) if is_ascii_identifier_start(byte) => {
                let start = *index;
                *index += 1;
                while *index < bytes.len() && is_ascii_identifier_continue(bytes[*index]) {
                    *index += 1;
                }
                tokens.push(SourceToken::Identifier(&source[start..*index]));
            }
            (byte, _) if byte.is_ascii_digit() => {
                skip_number(bytes, index);
                tokens.push(SourceToken::Literal);
            }
            (byte, _) if byte.is_ascii_whitespace() => *index += 1,
            _ => {
                tokens.push(SourceToken::Other(byte));
                *index += 1;
            }
        }
    }
}

fn scan_template<'source>(
    source: &'source str,
    index: &mut usize,
    skip_regexp_literals: bool,
    tokens: &mut Vec<SourceToken<'source>>,
) {
    let bytes = source.as_bytes();
    // Keep code tokens on either side of a template literal separate while
    // still scanning `${ ... }` substitutions using the code lexical goal.
    tokens.push(SourceToken::Literal);
    *index += 1;
    while *index < bytes.len() {
        match (bytes[*index], bytes.get(*index + 1).copied()) {
            (b'\\', _) => {
                *index += 1;
                if *index < bytes.len() {
                    *index += 1;
                }
            }
            (b'`', _) => {
                *index += 1;
                return;
            }
            (b'$', Some(b'{')) => {
                *index += 2;
                tokens.push(SourceToken::Other(b'{'));
                scan_code(source, index, Some(1), skip_regexp_literals, tokens);
                tokens.push(SourceToken::Literal);
            }
            _ => *index += 1,
        }
    }
}

fn skip_line_comment(bytes: &[u8], index: &mut usize) {
    *index += 2;
    while *index < bytes.len() && line_terminator_length(bytes, *index).is_none() {
        *index += 1;
    }
}

fn skip_block_comment(bytes: &[u8], index: &mut usize) -> bool {
    let mut contained_line_terminator = false;
    *index += 2;
    while *index < bytes.len() {
        if bytes[*index] == b'*' && bytes.get(*index + 1) == Some(&b'/') {
            *index += 2;
            return contained_line_terminator;
        }
        if let Some(length) = line_terminator_length(bytes, *index) {
            contained_line_terminator = true;
            *index += length;
        } else {
            *index += 1;
        }
    }
    contained_line_terminator
}

fn skip_quoted_string(bytes: &[u8], index: &mut usize, quote: u8) {
    *index += 1;
    while *index < bytes.len() {
        match bytes[*index] {
            b'\\' => {
                *index += 1;
                if *index < bytes.len() {
                    *index += 1;
                }
            }
            byte if byte == quote => {
                *index += 1;
                return;
            }
            _ => *index += 1,
        }
    }
}

fn skip_number(bytes: &[u8], index: &mut usize) {
    *index += 1;
    while *index < bytes.len()
        && (bytes[*index].is_ascii_alphanumeric()
            || matches!(bytes[*index], b'_' | b'.')
            || ((bytes[*index] == b'+' || bytes[*index] == b'-')
                && matches!(bytes.get(*index - 1), Some(b'e' | b'E' | b'p' | b'P'))))
    {
        *index += 1;
    }
}

fn regexp_literal_allowed(tokens: &[SourceToken<'_>]) -> bool {
    let previous = tokens
        .iter()
        .rev()
        .find(|token| !matches!(token, SourceToken::LineTerminator));
    match previous {
        None => true,
        Some(SourceToken::Identifier(keyword)) => matches!(
            *keyword,
            "await"
                | "case"
                | "delete"
                | "do"
                | "else"
                | "in"
                | "instanceof"
                | "new"
                | "of"
                | "return"
                | "throw"
                | "typeof"
                | "void"
                | "yield"
        ),
        Some(
            SourceToken::Dot
            | SourceToken::RightParen
            | SourceToken::RightBracket
            | SourceToken::RightBrace
            | SourceToken::Literal,
        ) => false,
        Some(
            SourceToken::LeftParen
            | SourceToken::LeftBracket
            | SourceToken::LeftBrace
            | SourceToken::Arrow
            | SourceToken::Other(_)
            | SourceToken::LineTerminator,
        ) => true,
    }
}

fn skip_regexp_literal(bytes: &[u8], index: &mut usize) -> bool {
    let mut cursor = *index + 1;
    let mut in_character_class = false;
    while cursor < bytes.len() {
        if line_terminator_length(bytes, cursor).is_some() {
            return false;
        }
        match bytes[cursor] {
            b'\\' => {
                cursor += 1;
                if cursor < bytes.len() {
                    cursor += 1;
                }
            }
            b'[' if !in_character_class => {
                in_character_class = true;
                cursor += 1;
            }
            b']' if in_character_class => {
                in_character_class = false;
                cursor += 1;
            }
            b'/' if !in_character_class => {
                cursor += 1;
                while cursor < bytes.len() && is_ascii_identifier_continue(bytes[cursor]) {
                    cursor += 1;
                }
                *index = cursor;
                return true;
            }
            _ => cursor += 1,
        }
    }
    false
}

fn push_line_terminator(tokens: &mut Vec<SourceToken<'_>>) {
    if !matches!(tokens.last(), Some(SourceToken::LineTerminator)) {
        tokens.push(SourceToken::LineTerminator);
    }
}

fn line_terminator_length(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes.get(index..) {
        Some([b'\r', b'\n', ..]) => Some(2),
        Some([b'\n' | b'\r', ..]) => Some(1),
        Some([0xe2, 0x80, 0xa8 | 0xa9, ..]) => Some(3),
        _ => None,
    }
}

const fn is_ascii_identifier_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || matches!(byte, b'_' | b'$')
}

const fn is_ascii_identifier_continue(byte: u8) -> bool {
    is_ascii_identifier_start(byte) || byte.is_ascii_digit()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{generator_destructuring_source_needs_async_guard, missing_host_capability_hints};
    use crate::metadata::Metadata;

    fn metadata(flags: &[&str], features: &[&str], includes: &[&str]) -> Metadata {
        Metadata {
            flags: flags.iter().map(|value| (*value).to_owned()).collect(),
            features: features.iter().map(|value| (*value).to_owned()).collect(),
            includes: includes.iter().map(|value| (*value).to_owned()).collect(),
            ..Metadata::default()
        }
    }

    fn generator_metadata() -> Metadata {
        metadata(&[], &["generators"], &[])
    }

    #[test]
    fn combines_modes_flags_features_includes_and_hooks_in_stable_order() {
        let metadata = metadata(
            &["module", "async", "CanBlockIsFalse"],
            &["host-gc-required", "IsHTMLDDA"],
            &["atomicsHelper.js", "detachArrayBuffer.js"],
        );
        let actual = missing_host_capability_hints(
            Path::new("test/example.js"),
            "$262.createRealm(); $262.evalScript('0'); $262.gc();",
            &metadata,
            false,
        );
        assert_eq!(
            actual,
            [
                "agent",
                "async",
                "can-block:false",
                "create-realm",
                "detach-array-buffer",
                "eval-script",
                "gc",
                "is-html-dda",
                "module",
            ]
        );
    }

    #[test]
    fn can_block_true_is_the_supported_default_and_is_not_missing() {
        let metadata = metadata(&["CanBlockIsTrue"], &[], &[]);
        assert!(
            missing_host_capability_hints(Path::new("test/example.js"), "0;", &metadata, false)
                .is_empty()
        );
    }

    #[test]
    fn scoped_async_host_removes_only_the_async_execution_gap() {
        let metadata = metadata(&["module", "async"], &[], &[]);
        assert_eq!(
            missing_host_capability_hints(Path::new("test/example.js"), "0;", &metadata, true,),
            ["module"]
        );
    }

    #[test]
    fn declared_module_remains_the_authoritative_execution_gap() {
        let metadata = metadata(&["module"], &["generators"], &[]);
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                "const callable = async () => 1;",
                &metadata,
                false,
            ),
            ["module"]
        );
        assert!(generator_destructuring_source_needs_async_guard(
            "const callable = async () => 1;",
            &metadata,
        ));
    }

    #[test]
    fn generator_admission_guard_detects_async_functions_and_arrows() {
        let metadata = generator_metadata();
        let sources = [
            "async function ordinary() {}",
            "const generator = async function* () {};",
            "const arrow = async value => value;",
            "const arrow = async (value, nested = (item => item)) => value;",
            "async function outer() { function* nested() { yield 1; } }",
            "const from_substitution = `${async function () {}}`;",
        ];

        for source in sources {
            assert!(
                generator_destructuring_source_needs_async_guard(source, &metadata),
                "source should require the scoped async guard: {source}",
            );
        }
    }

    #[test]
    fn generator_admission_guard_is_feature_scoped_and_skips_hidden_text() {
        let metadata = generator_metadata();
        let sources = [
            "var async = 1;",
            "async(value);",
            "({ async() {} });",
            "async['computed']();",
            "// async function commented() {}\n0;",
            "/* async value => value */ 0;",
            "'async function inString() {}';",
            "\"async value => value\";",
            "`async function inTemplateRaw() {}; async value => value`;",
            "const expression = /async function inPattern() {}/;",
            "const expressions = [/async value => value/, /async\\s+function/gi];",
        ];

        for source in sources {
            assert!(
                !generator_destructuring_source_needs_async_guard(source, &metadata),
                "source should not require the scoped async guard: {source}",
            );
        }
        assert!(!generator_destructuring_source_needs_async_guard(
            "async function outside_the_cohort() {}",
            &Metadata::default(),
        ));
    }

    #[test]
    fn scoped_async_heads_honor_no_line_terminator_restrictions() {
        let metadata = generator_metadata();
        let sources = [
            "async\nfunction split() {}",
            "async\r\nfunction split() {}",
            "async\u{2028}function split() {}",
            "async\u{2029}value => value",
            "async\nvalue => value",
            "async value\n=> value",
            "async\n(value) => value",
            "async (value)\n=> value",
            "({ async\nmethod() {} });",
            "({ async\n*generatorMethod() {} });",
            "async /* comment with\nline */ function split() {}",
        ];

        for source in sources {
            assert!(
                !generator_destructuring_source_needs_async_guard(source, &metadata),
                "line terminator should split the async callable head: {source:?}",
            );
        }

        for source in [
            "async /* comment */ function joined() {}",
            "async /* comment */ value /* comment */ => value",
            "async /* comment */ (value) /* comment */ => value",
        ] {
            assert!(
                generator_destructuring_source_needs_async_guard(source, &metadata),
                "comment trivia without a line terminator should preserve the head: {source}",
            );
        }
    }

    #[test]
    fn scanner_skips_comments_quoted_strings_and_template_raw_text() {
        let source = r#"
            // $262.gc()
            /* $262.agent.start('') */
            '$262.createRealm()';
            "$262.evalScript('0')";
            `$262.detachArrayBuffer(buffer) ${$262.IsHTMLDDA}`;
            `outer ${`inner raw $262.gc ${$262.AbstractModuleSource}`}`;
        "#;
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["abstract-module-source", "is-html-dda"]
        );
    }

    #[test]
    fn scanner_accepts_trivia_around_member_access_and_deduplicates() {
        let source = "$262 /* a */ . // b\n gc(); $262.gc();";
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["gc"]
        );
    }

    #[test]
    fn host_scanner_does_not_hide_a_hook_behind_the_regexp_heuristic() {
        let source = "let x = 4, y = 2; x++ / $262.gc() / y;";
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["gc"]
        );
    }

    #[test]
    fn base_and_unknown_properties_fail_closed_but_optional_hooks_do_not() {
        let source = "$262.global; $262.codePointRange; $262.futureHook();";
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/example.js"),
                source,
                &Metadata::default(),
                false,
            ),
            ["global", "unknown:$262.futureHook"]
        );
    }

    #[test]
    fn detach_harness_self_test_shadow_suppresses_the_include_hint() {
        let metadata = metadata(&[], &[], &["detachArrayBuffer.js"]);
        let source = "var /* intentional host shadow */ $262 = { detachArrayBuffer() {} };";
        assert!(
            missing_host_capability_hints(
                Path::new("test/harness/detachArrayBuffer-host-detachArrayBuffer.js"),
                source,
                &metadata,
                false,
            )
            .is_empty()
        );

        assert_eq!(
            missing_host_capability_hints(Path::new("test/ordinary.js"), source, &metadata, false,),
            ["detach-array-buffer"]
        );
    }

    #[test]
    fn all_seven_required_hooks_have_explicit_capability_ids() {
        let source = r#"
            $262.agent;
            $262.createRealm;
            $262.evalScript;
            $262.detachArrayBuffer;
            $262.IsHTMLDDA;
            $262.gc;
            $262.AbstractModuleSource;
        "#;
        let actual = missing_host_capability_hints(
            Path::new("test/example.js"),
            source,
            &Metadata::default(),
            false,
        );
        assert_eq!(
            actual.into_iter().collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "abstract-module-source".to_owned(),
                "agent".to_owned(),
                "create-realm".to_owned(),
                "detach-array-buffer".to_owned(),
                "eval-script".to_owned(),
                "gc".to_owned(),
                "is-html-dda".to_owned(),
            ])
        );
    }
}
