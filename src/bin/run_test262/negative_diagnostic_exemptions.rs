use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::sha256_file;
use super::metadata::parse_metadata;
use super::{TestMode, Variant, validate_relative_test_path};

const HEADER: &str = "path\tvariant\tsource_sha256\tphase\ttype\treason";
const LEGACY_REASON: &str = "legacy-r3eg-b-phase-type-only";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NegativeDiagnosticExemption {
    pub(super) path: String,
    pub(super) variant: Variant,
    source_sha256: String,
    phase: String,
    error_type: String,
}

#[derive(Clone, Debug)]
pub(super) struct NegativeDiagnosticExemptions {
    entries: BTreeMap<(String, Variant), NegativeDiagnosticExemption>,
}

impl NegativeDiagnosticExemptions {
    pub(super) fn load(path: &Path, expected_sha256: &str, suite: &Path) -> Result<Self, String> {
        let actual_sha256 = sha256_file(path)?;
        if actual_sha256 != expected_sha256 {
            return Err(format!(
                "negative diagnostic exemptions checksum mismatch: expected {expected_sha256}, found {actual_sha256}"
            ));
        }
        let source = fs::read_to_string(path).map_err(|error| {
            format!(
                "read negative diagnostic exemptions {}: {error}",
                path.display()
            )
        })?;
        let exemptions = Self::parse(&source)?;
        exemptions.validate_sources(suite)?;
        Ok(exemptions)
    }

    fn parse(source: &str) -> Result<Self, String> {
        if source.contains('\r') {
            return Err("negative diagnostic exemptions must use LF line endings".to_owned());
        }
        if !source.ends_with('\n') {
            return Err("negative diagnostic exemptions must end with a newline".to_owned());
        }
        let mut lines = source.split_terminator('\n');
        if lines.next() != Some(HEADER) {
            return Err("negative diagnostic exemptions header does not match schema".to_owned());
        }

        let mut entries = BTreeMap::new();
        let mut previous_key: Option<(String, Variant)> = None;
        for (index, line) in lines.enumerate() {
            let line_number = index + 2;
            if line.is_empty() {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} is empty"
                ));
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 6 {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} has {} fields instead of 6",
                    fields.len()
                ));
            }
            if fields
                .iter()
                .any(|field| field.trim() != *field || field.chars().any(char::is_control))
            {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} is not canonical"
                ));
            }

            let path = PathBuf::from(fields[0]);
            validate_relative_test_path(&path)?;
            if path.to_string_lossy() != fields[0] || fields[0].contains('\\') {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} path is not canonical"
                ));
            }
            let variant = Variant::parse(fields[1])?;
            validate_sha256(fields[2], line_number)?;
            if !matches!(fields[3], "parse" | "resolution" | "runtime") {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} has invalid phase"
                ));
            }
            if !is_identifier(fields[4]) {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} has invalid error type"
                ));
            }
            if fields[5] != LEGACY_REASON {
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} has an unrecognized reason"
                ));
            }

            let key = (fields[0].to_owned(), variant);
            if previous_key
                .as_ref()
                .is_some_and(|previous| previous >= &key)
            {
                let kind = if previous_key.as_ref() == Some(&key) {
                    "duplicate"
                } else {
                    "unsorted"
                };
                return Err(format!(
                    "negative diagnostic exemptions line {line_number} has a {kind} path/variant key"
                ));
            }
            previous_key = Some(key.clone());
            entries.insert(
                key,
                NegativeDiagnosticExemption {
                    path: fields[0].to_owned(),
                    variant,
                    source_sha256: fields[2].to_owned(),
                    phase: fields[3].to_owned(),
                    error_type: fields[4].to_owned(),
                },
            );
        }
        if entries.is_empty() {
            return Err("negative diagnostic exemptions contain no entries".to_owned());
        }
        Ok(Self { entries })
    }

    fn validate_sources(&self, suite: &Path) -> Result<(), String> {
        for exemption in self.entries.values() {
            let source_path = suite.join(&exemption.path);
            let actual_sha256 = sha256_file(&source_path)?;
            if actual_sha256 != exemption.source_sha256 {
                return Err(format!(
                    "negative diagnostic exemption source hash drifted for {}: expected {}, found {actual_sha256}",
                    exemption.path, exemption.source_sha256
                ));
            }
            let source = fs::read_to_string(&source_path)
                .map_err(|error| format!("read {}: {error}", source_path.display()))?;
            let metadata = parse_metadata(&source)
                .map_err(|error| format!("parse metadata for {}: {error}", exemption.path))?;
            let negative = metadata.negative.as_ref().ok_or_else(|| {
                format!(
                    "negative diagnostic exemption is not a negative test: {}",
                    exemption.path
                )
            })?;
            if negative.phase.as_deref() != Some(exemption.phase.as_str())
                || negative.error_type.as_deref() != Some(exemption.error_type.as_str())
            {
                return Err(format!(
                    "negative diagnostic exemption metadata drifted for {}",
                    exemption.path
                ));
            }
            if !metadata
                .variants(TestMode::Both)
                .contains(&exemption.variant)
            {
                return Err(format!(
                    "negative diagnostic exemption variant is not selected by metadata for {}",
                    exemption.path
                ));
            }
        }
        Ok(())
    }

    pub(super) fn contains(&self, path: &Path, variant: Variant) -> bool {
        self.entries
            .contains_key(&(path.to_string_lossy().replace('\\', "/"), variant))
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &NegativeDiagnosticExemption> {
        self.entries.values()
    }
}

fn validate_sha256(value: &str, line_number: usize) -> Result<(), String> {
    if value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        Ok(())
    } else {
        Err(format!(
            "negative diagnostic exemptions line {line_number} has invalid source SHA-256"
        ))
    }
}

fn is_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

#[cfg(test)]
mod tests {
    use super::{HEADER, NegativeDiagnosticExemptions};

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn row(path: &str, variant: &str) -> String {
        format!("{path}\t{variant}\t{SHA}\tparse\tSyntaxError\tlegacy-r3eg-b-phase-type-only\n")
    }

    #[test]
    fn parser_accepts_only_canonical_frozen_legacy_rows() {
        let source = format!("{HEADER}\n{}", row("test/a.js", "sloppy"));
        let parsed = NegativeDiagnosticExemptions::parse(&source).unwrap();
        assert!(parsed.contains(std::path::Path::new("test/a.js"), crate::Variant::Sloppy));

        assert!(NegativeDiagnosticExemptions::parse(source.trim_end()).is_err());
        assert!(NegativeDiagnosticExemptions::parse(&source.replace('\n', "\r\n")).is_err());
        assert!(
            NegativeDiagnosticExemptions::parse(
                &source.replace("legacy-r3eg-b-phase-type-only", "new-exemption")
            )
            .is_err()
        );
    }

    #[test]
    fn duplicate_and_unsorted_keys_are_rejected() {
        let duplicate = format!(
            "{HEADER}\n{}{}",
            row("test/a.js", "sloppy"),
            row("test/a.js", "sloppy")
        );
        assert!(
            NegativeDiagnosticExemptions::parse(&duplicate)
                .unwrap_err()
                .contains("duplicate")
        );

        let unsorted = format!(
            "{HEADER}\n{}{}",
            row("test/b.js", "sloppy"),
            row("test/a.js", "sloppy")
        );
        assert!(
            NegativeDiagnosticExemptions::parse(&unsorted)
                .unwrap_err()
                .contains("unsorted")
        );
    }
}
