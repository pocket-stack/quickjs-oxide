use std::collections::BTreeSet;
use std::path::Path;

use super::metadata::Metadata;

/// Return conservative, stable IDs for Test262 execution capabilities which
/// the current runner cannot provide.
///
/// Metadata is authoritative for execution modes. Includes and source tokens
/// are hints for host hooks: JavaScript can replace the writable `$262` global,
/// so the execution layer must still retain dynamic provenance before treating
/// one of these hints as the cause of a result.
pub(super) fn missing_host_capability_hints(
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Vec<String> {
    let mut missing = BTreeSet::new();

    if metadata.is_module() {
        missing.insert("module".to_owned());
    }
    if metadata.is_async() {
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

    let tokens = source_tokens(source);
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
    Other,
}

fn member_names<'source>(tokens: &[SourceToken<'source>]) -> Vec<&'source str> {
    tokens
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
        && tokens.windows(2).any(|window| {
            matches!(
                window,
                [
                    SourceToken::Identifier("var" | "let" | "const"),
                    SourceToken::Identifier("$262")
                ]
            )
        })
}

fn source_tokens(source: &str) -> Vec<SourceToken<'_>> {
    let mut tokens = Vec::new();
    let mut index = 0;
    scan_code(source, &mut index, None, &mut tokens);
    tokens
}

/// Tokenize only the small lexical surface needed for `$262 . hook` hints.
/// Full parsing is intentionally avoided because unsupported grammar is one of
/// the things the Test262 runner measures.
fn scan_code<'source>(
    source: &'source str,
    index: &mut usize,
    mut template_brace_depth: Option<usize>,
    tokens: &mut Vec<SourceToken<'source>>,
) {
    let bytes = source.as_bytes();
    while *index < bytes.len() {
        let byte = bytes[*index];
        let next = bytes.get(*index + 1).copied();
        match (byte, next) {
            (b'/', Some(b'/')) => skip_line_comment(bytes, index),
            (b'/', Some(b'*')) => skip_block_comment(bytes, index),
            (b'\'' | b'"', _) => skip_quoted_string(bytes, index, byte),
            (b'`', _) => scan_template(source, index, tokens),
            (b'{', _) if template_brace_depth.is_some() => {
                template_brace_depth = template_brace_depth.map(|depth| depth + 1);
                tokens.push(SourceToken::Other);
                *index += 1;
            }
            (b'}', _) if template_brace_depth.is_some() => {
                let depth = template_brace_depth.expect("template depth was checked");
                *index += 1;
                if depth == 1 {
                    return;
                }
                template_brace_depth = Some(depth - 1);
                tokens.push(SourceToken::Other);
            }
            (b'.', _) => {
                tokens.push(SourceToken::Dot);
                *index += 1;
            }
            (byte, _) if is_ascii_identifier_start(byte) => {
                let start = *index;
                *index += 1;
                while *index < bytes.len() && is_ascii_identifier_continue(bytes[*index]) {
                    *index += 1;
                }
                tokens.push(SourceToken::Identifier(&source[start..*index]));
            }
            (byte, _) if byte.is_ascii_whitespace() => *index += 1,
            _ => {
                tokens.push(SourceToken::Other);
                *index += 1;
            }
        }
    }
}

fn scan_template<'source>(
    source: &'source str,
    index: &mut usize,
    tokens: &mut Vec<SourceToken<'source>>,
) {
    let bytes = source.as_bytes();
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
                scan_code(source, index, Some(1), tokens);
            }
            _ => *index += 1,
        }
    }
}

fn skip_line_comment(bytes: &[u8], index: &mut usize) {
    *index += 2;
    while *index < bytes.len() && !matches!(bytes[*index], b'\n' | b'\r') {
        *index += 1;
    }
}

fn skip_block_comment(bytes: &[u8], index: &mut usize) {
    *index += 2;
    while *index < bytes.len() {
        if bytes[*index] == b'*' && bytes.get(*index + 1) == Some(&b'/') {
            *index += 2;
            return;
        }
        *index += 1;
    }
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

    use super::missing_host_capability_hints;
    use crate::metadata::Metadata;

    fn metadata(flags: &[&str], features: &[&str], includes: &[&str]) -> Metadata {
        Metadata {
            flags: flags.iter().map(|value| (*value).to_owned()).collect(),
            features: features.iter().map(|value| (*value).to_owned()).collect(),
            includes: includes.iter().map(|value| (*value).to_owned()).collect(),
            ..Metadata::default()
        }
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
            missing_host_capability_hints(Path::new("test/example.js"), "0;", &metadata).is_empty()
        );
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
                &Metadata::default()
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
                &Metadata::default()
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
                &Metadata::default()
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
            )
            .is_empty()
        );

        assert_eq!(
            missing_host_capability_hints(Path::new("test/ordinary.js"), source, &metadata,),
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
