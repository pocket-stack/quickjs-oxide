use std::collections::BTreeSet;

use super::{TestMode, Variant};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct NegativeExpectation {
    pub(super) phase: Option<String>,
    pub(super) error_type: Option<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct Metadata {
    pub(super) includes: Vec<String>,
    pub(super) flags: BTreeSet<String>,
    pub(super) features: Vec<String>,
    pub(super) negative: Option<NegativeExpectation>,
}

impl Metadata {
    pub(super) fn is_raw(&self) -> bool {
        self.flags.contains("raw")
    }

    pub(super) fn is_async(&self) -> bool {
        self.flags.contains("async")
    }

    pub(super) fn is_module(&self) -> bool {
        self.flags.contains("module")
    }

    pub(super) fn variants(&self, mode: TestMode) -> Vec<Variant> {
        if self.is_module() {
            return vec![Variant::Sloppy];
        }
        let no_strict = self.flags.contains("noStrict") || self.is_raw();
        let only_strict = self.flags.contains("onlyStrict");
        match mode {
            TestMode::DefaultSloppy => {
                if only_strict {
                    vec![Variant::Strict]
                } else {
                    vec![Variant::Sloppy]
                }
            }
            TestMode::DefaultStrict => {
                if no_strict {
                    vec![Variant::Sloppy]
                } else {
                    vec![Variant::Strict]
                }
            }
            TestMode::Sloppy => (!only_strict)
                .then_some(Variant::Sloppy)
                .into_iter()
                .collect(),
            TestMode::Strict => (!no_strict)
                .then_some(Variant::Strict)
                .into_iter()
                .collect(),
            TestMode::Both => {
                if no_strict {
                    vec![Variant::Sloppy]
                } else if only_strict {
                    vec![Variant::Strict]
                } else {
                    vec![Variant::Sloppy, Variant::Strict]
                }
            }
        }
    }
}

pub(super) fn parse_metadata(source: &str) -> Result<Metadata, String> {
    let Some(start) = source.find("/*---") else {
        return Ok(Metadata::default());
    };
    let remainder = &source[start + 5..];
    let end = remainder
        .find("---*/")
        .ok_or_else(|| "unterminated Test262 frontmatter".to_owned())?;
    let frontmatter = &remainder[..end];
    let mut metadata = Metadata {
        includes: parse_frontmatter_list(frontmatter, "includes")?,
        flags: parse_frontmatter_list(frontmatter, "flags")?
            .into_iter()
            .collect(),
        features: parse_frontmatter_list(frontmatter, "features")?,
        negative: None,
    };
    if has_top_level_key(frontmatter, "negative") {
        metadata.negative = Some(NegativeExpectation {
            phase: parse_nested_scalar(frontmatter, "negative", "phase"),
            error_type: parse_nested_scalar(frontmatter, "negative", "type"),
        });
    }
    Ok(metadata)
}

fn has_top_level_key(frontmatter: &str, key: &str) -> bool {
    frontmatter.split(['\r', '\n']).any(|line| {
        !line.starts_with(char::is_whitespace)
            && line
                .split_once(':')
                .is_some_and(|(candidate, _)| candidate.trim() == key)
    })
}

fn parse_frontmatter_list(frontmatter: &str, key: &str) -> Result<Vec<String>, String> {
    let lines = frontmatter.split(['\r', '\n']).collect::<Vec<_>>();
    for (index, line) in lines.iter().enumerate() {
        if line.starts_with(char::is_whitespace) {
            continue;
        }
        let Some((candidate, raw_value)) = line.split_once(':') else {
            continue;
        };
        if candidate.trim() != key {
            continue;
        }
        let raw_value = raw_value.trim();
        if raw_value.starts_with('[') {
            let mut joined = raw_value.to_owned();
            let mut next = index + 1;
            while !joined.contains(']') && next < lines.len() {
                joined.push(' ');
                joined.push_str(lines[next].trim());
                next += 1;
            }
            let end = joined
                .find(']')
                .ok_or_else(|| format!("unterminated {key} list"))?;
            return Ok(split_list_items(&joined[1..end]));
        }
        if !raw_value.is_empty() {
            return Ok(vec![clean_scalar(raw_value)]);
        }
        let mut values = Vec::new();
        for nested in &lines[index + 1..] {
            if !nested.starts_with(char::is_whitespace) {
                break;
            }
            let nested = nested.trim();
            if let Some(value) = nested.strip_prefix('-') {
                values.push(clean_scalar(value.trim()));
            }
        }
        return Ok(values);
    }
    Ok(Vec::new())
}

fn parse_nested_scalar(frontmatter: &str, parent: &str, key: &str) -> Option<String> {
    let lines = frontmatter.split(['\r', '\n']).collect::<Vec<_>>();
    let parent_index = lines.iter().position(|line| {
        !line.starts_with(char::is_whitespace)
            && line
                .split_once(':')
                .is_some_and(|(candidate, _)| candidate.trim() == parent)
    })?;
    for line in &lines[parent_index + 1..] {
        if !line.starts_with(char::is_whitespace) {
            break;
        }
        let Some((candidate, value)) = line.trim().split_once(':') else {
            continue;
        };
        if candidate.trim() == key {
            return Some(clean_scalar(value.trim()));
        }
    }
    None
}

fn split_list_items(value: &str) -> Vec<String> {
    value
        .split(',')
        .flat_map(str::split_whitespace)
        .map(clean_scalar)
        .filter(|value| !value.is_empty())
        .collect()
}

fn clean_scalar(value: &str) -> String {
    value
        .trim()
        .trim_matches(|character| character == '\'' || character == '"')
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Metadata, NegativeExpectation, parse_metadata};
    use crate::{TestMode, Variant};

    #[test]
    fn metadata_parser_handles_inline_block_and_negative_fields() {
        let metadata = parse_metadata(
            r#"/*---
description: ignored
includes: [propertyHelper.js,
  compareArray.js]
flags:
  - onlyStrict
  - CanBlockIsFalse
features: [String.prototype.at, Symbol]
negative:
  phase: parse
  type: SyntaxError
---*/
$DONOTEVALUATE();"#,
        )
        .unwrap();
        assert_eq!(
            metadata,
            Metadata {
                includes: vec!["propertyHelper.js".to_owned(), "compareArray.js".to_owned()],
                flags: BTreeSet::from(["CanBlockIsFalse".to_owned(), "onlyStrict".to_owned(),]),
                features: vec!["String.prototype.at".to_owned(), "Symbol".to_owned()],
                negative: Some(NegativeExpectation {
                    phase: Some("parse".to_owned()),
                    error_type: Some("SyntaxError".to_owned()),
                }),
            }
        );
    }

    #[test]
    fn variant_matrix_matches_quickjs_all_mode() {
        let plain = Metadata::default();
        assert_eq!(
            plain.variants(TestMode::Both),
            [Variant::Sloppy, Variant::Strict]
        );
        let mut no_strict = Metadata::default();
        no_strict.flags.insert("noStrict".to_owned());
        assert_eq!(no_strict.variants(TestMode::Both), [Variant::Sloppy]);
        let mut only_strict = Metadata::default();
        only_strict.flags.insert("onlyStrict".to_owned());
        assert_eq!(only_strict.variants(TestMode::Both), [Variant::Strict]);
        let mut raw = Metadata::default();
        raw.flags.insert("raw".to_owned());
        assert_eq!(raw.variants(TestMode::Both), [Variant::Sloppy]);
    }

    #[test]
    fn metadata_parser_accepts_cr_only_frontmatter() {
        let metadata = parse_metadata(
            "/*---\rflags: [raw]\rnegative:\r  phase: runtime\r  type: TypeError\r---*/\r0;",
        )
        .unwrap();
        assert!(metadata.flags.contains("raw"));
        assert_eq!(
            metadata.negative,
            Some(NegativeExpectation {
                phase: Some("runtime".to_owned()),
                error_type: Some("TypeError".to_owned()),
            })
        );
    }
}
