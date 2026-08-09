use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::config::sha256_file;
use super::metadata::parse_metadata;
use super::report::WorkerResult;
use super::{TestMode, Variant, validate_relative_test_path};

const HEADER: &str =
    "path\tvariant\tsource_sha256\tphase\ttype\trule\tmessage\tline\tcolumn\tlocation_policy";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum LocationPolicy {
    Exact,
    Absent,
}

impl LocationPolicy {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Absent => "absent",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NegativeDiagnosticExpectation {
    pub(super) path: String,
    pub(super) variant: Variant,
    pub(super) source_sha256: String,
    pub(super) phase: String,
    pub(super) error_type: String,
    pub(super) rule: String,
    pub(super) message: String,
    pub(super) line: Option<u32>,
    pub(super) column: Option<u32>,
    pub(super) location_policy: LocationPolicy,
}

impl NegativeDiagnosticExpectation {
    pub(super) fn classify(&self, result: &mut WorkerResult) {
        if !matches!(
            result.outcome.as_str(),
            "pass" | "fail-negative-mismatch" | "fail-missing-throw"
        ) {
            return;
        }
        let location_matches = match self.location_policy {
            LocationPolicy::Exact => {
                result.actual_line == self.line && result.actual_column == self.column
            }
            LocationPolicy::Absent => {
                result.actual_line.is_none() && result.actual_column.is_none()
            }
        };
        if result.actual_phase == self.phase
            && result.actual_type == self.error_type
            && result.detail == self.message
            && location_matches
        {
            result.outcome = "pass".to_owned();
        } else {
            result.outcome = "fail-negative-diagnostic-mismatch".to_owned();
        }
    }
}

#[derive(Clone, Debug)]
pub(super) struct NegativeDiagnostics {
    entries: BTreeMap<(String, Variant), NegativeDiagnosticExpectation>,
}

impl NegativeDiagnostics {
    pub(super) fn load(path: &Path, expected_sha256: &str, suite: &Path) -> Result<Self, String> {
        let actual_sha256 = sha256_file(path)?;
        if actual_sha256 != expected_sha256 {
            return Err(format!(
                "negative diagnostics checksum mismatch: expected {expected_sha256}, found {actual_sha256}"
            ));
        }
        let source = fs::read_to_string(path)
            .map_err(|error| format!("read negative diagnostics {}: {error}", path.display()))?;
        let diagnostics = Self::parse(&source)?;
        diagnostics.validate_sources(suite)?;
        Ok(diagnostics)
    }

    fn parse(source: &str) -> Result<Self, String> {
        if source.contains('\r') {
            return Err("negative diagnostics must use LF line endings".to_owned());
        }
        if !source.ends_with('\n') {
            return Err("negative diagnostics must end with a newline".to_owned());
        }
        let mut lines = source.split_terminator('\n');
        if lines.next() != Some(HEADER) {
            return Err("negative diagnostics header does not match schema".to_owned());
        }

        let mut entries = BTreeMap::new();
        let mut previous_key: Option<(String, Variant)> = None;
        for (index, line) in lines.enumerate() {
            let line_number = index + 2;
            if line.is_empty() {
                return Err(format!("negative diagnostics line {line_number} is empty"));
            }
            let fields = line.split('\t').collect::<Vec<_>>();
            if fields.len() != 10 {
                return Err(format!(
                    "negative diagnostics line {line_number} has {} fields instead of 10",
                    fields.len()
                ));
            }
            for (field_index, field) in fields.iter().enumerate() {
                if field.trim() != *field || field.chars().any(char::is_control) {
                    return Err(format!(
                        "negative diagnostics line {line_number} field {} is not canonical",
                        field_index + 1
                    ));
                }
            }

            let path = PathBuf::from(fields[0]);
            validate_relative_test_path(&path)?;
            if path.to_string_lossy() != fields[0] || fields[0].contains('\\') {
                return Err(format!(
                    "negative diagnostics line {line_number} path is not canonical"
                ));
            }
            let variant = Variant::parse(fields[1])?;
            validate_sha256(fields[2], line_number)?;
            if !matches!(fields[3], "parse" | "resolution" | "runtime") {
                return Err(format!(
                    "negative diagnostics line {line_number} has invalid phase"
                ));
            }
            if !is_identifier(fields[4]) {
                return Err(format!(
                    "negative diagnostics line {line_number} has invalid error type"
                ));
            }
            if !is_rule(fields[5]) {
                return Err(format!(
                    "negative diagnostics line {line_number} has invalid rule"
                ));
            }
            if fields[6].is_empty() {
                return Err(format!(
                    "negative diagnostics line {line_number} has an empty message"
                ));
            }
            let location_policy = match fields[9] {
                "exact" => LocationPolicy::Exact,
                "absent" => LocationPolicy::Absent,
                _ => {
                    return Err(format!(
                        "negative diagnostics line {line_number} has invalid location policy"
                    ));
                }
            };
            let line_value = parse_coordinate(fields[7], "line", line_number)?;
            let column = parse_coordinate(fields[8], "column", line_number)?;
            match location_policy {
                LocationPolicy::Exact if line_value.is_none() || column.is_none() => {
                    return Err(format!(
                        "negative diagnostics line {line_number} exact location is incomplete"
                    ));
                }
                LocationPolicy::Absent if line_value.is_some() || column.is_some() => {
                    return Err(format!(
                        "negative diagnostics line {line_number} absent location has coordinates"
                    ));
                }
                LocationPolicy::Exact | LocationPolicy::Absent => {}
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
                    "negative diagnostics line {line_number} has a {kind} path/variant key"
                ));
            }
            previous_key = Some(key.clone());
            let expectation = NegativeDiagnosticExpectation {
                path: fields[0].to_owned(),
                variant,
                source_sha256: fields[2].to_owned(),
                phase: fields[3].to_owned(),
                error_type: fields[4].to_owned(),
                rule: fields[5].to_owned(),
                message: fields[6].to_owned(),
                line: line_value,
                column,
                location_policy,
            };
            if entries.insert(key, expectation).is_some() {
                return Err(format!(
                    "negative diagnostics line {line_number} duplicates a path/variant key"
                ));
            }
        }
        if entries.is_empty() {
            return Err("negative diagnostics contain no contracts".to_owned());
        }
        Ok(Self { entries })
    }

    fn validate_sources(&self, suite: &Path) -> Result<(), String> {
        for expectation in self.entries.values() {
            let source_path = suite.join(&expectation.path);
            let actual_sha256 = sha256_file(&source_path)?;
            if actual_sha256 != expectation.source_sha256 {
                return Err(format!(
                    "negative diagnostic source hash drifted for {}: expected {}, found {}",
                    expectation.path, expectation.source_sha256, actual_sha256
                ));
            }
            let source = fs::read_to_string(&source_path)
                .map_err(|error| format!("read {}: {error}", source_path.display()))?;
            let metadata = parse_metadata(&source)
                .map_err(|error| format!("parse metadata for {}: {error}", expectation.path))?;
            let negative = metadata.negative.as_ref().ok_or_else(|| {
                format!(
                    "negative diagnostic contract is not a negative test: {}",
                    expectation.path
                )
            })?;
            if negative.phase.as_deref() != Some(expectation.phase.as_str())
                || negative.error_type.as_deref() != Some(expectation.error_type.as_str())
            {
                return Err(format!(
                    "negative diagnostic metadata drifted for {}",
                    expectation.path
                ));
            }
            if !metadata
                .variants(TestMode::Both)
                .contains(&expectation.variant)
            {
                return Err(format!(
                    "negative diagnostic variant is not selected by metadata for {}",
                    expectation.path
                ));
            }
        }
        Ok(())
    }

    pub(super) fn get(
        &self,
        path: &Path,
        variant: Variant,
    ) -> Option<&NegativeDiagnosticExpectation> {
        self.entries
            .get(&(path.to_string_lossy().replace('\\', "/"), variant))
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = &NegativeDiagnosticExpectation> {
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
            "negative diagnostics line {line_number} has invalid source SHA-256"
        ))
    }
}

fn parse_coordinate(value: &str, name: &str, line_number: usize) -> Result<Option<u32>, String> {
    if value.is_empty() {
        return Ok(None);
    }
    if value.starts_with('0') {
        return Err(format!(
            "negative diagnostics line {line_number} has non-canonical {name}"
        ));
    }
    let value = value
        .parse::<u32>()
        .map_err(|_| format!("negative diagnostics line {line_number} has invalid {name}"))?;
    if value == 0 {
        return Err(format!(
            "negative diagnostics line {line_number} has zero {name}"
        ));
    }
    Ok(Some(value))
}

fn is_identifier(value: &str) -> bool {
    let mut bytes = value.bytes();
    bytes
        .next()
        .is_some_and(|byte| byte.is_ascii_alphabetic() || byte == b'_')
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
}

fn is_rule(value: &str) -> bool {
    !value.is_empty()
        && !value.starts_with('.')
        && !value.starts_with('-')
        && !value.ends_with('.')
        && !value.ends_with('-')
        && !value.contains("..")
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{HEADER, LocationPolicy, NegativeDiagnostics};
    use crate::Variant;
    use crate::report::WorkerResult;

    const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn row(path: &str, location: &str) -> String {
        format!(
            "{path}\tsloppy\t{SHA}\tparse\tSyntaxError\tmodule.invalid-import-binding\tinvalid import binding\t{location}\n"
        )
    }

    #[test]
    fn parser_is_strict_and_accepts_nullable_locations() {
        let exact = format!("{HEADER}\n{}", row("test/a.js", "3\t7\texact"));
        let parsed = NegativeDiagnostics::parse(&exact).unwrap();
        let expectation = parsed
            .get(PathBuf::from("test/a.js").as_path(), Variant::Sloppy)
            .unwrap();
        assert_eq!((expectation.line, expectation.column), (Some(3), Some(7)));
        assert_eq!(expectation.location_policy, LocationPolicy::Exact);

        let absent = format!("{HEADER}\n{}", row("test/a.js", "\t\tabsent"));
        let parsed = NegativeDiagnostics::parse(&absent).unwrap();
        let expectation = parsed
            .get(PathBuf::from("test/a.js").as_path(), Variant::Sloppy)
            .unwrap();
        assert_eq!((expectation.line, expectation.column), (None, None));
        assert_eq!(expectation.location_policy, LocationPolicy::Absent);

        assert!(NegativeDiagnostics::parse(&exact.replace("\n", "\r\n")).is_err());
        assert!(NegativeDiagnostics::parse(exact.trim_end()).is_err());
        assert!(NegativeDiagnostics::parse(&exact.replace("\texact", "\t\texact")).is_err());
    }

    #[test]
    fn duplicate_path_variant_keys_are_rejected() {
        let duplicate = format!(
            "{HEADER}\n{}{}",
            row("test/a.js", "3\t7\texact"),
            row("test/a.js", "3\t7\texact")
        );
        assert!(
            NegativeDiagnostics::parse(&duplicate)
                .unwrap_err()
                .contains("duplicate")
        );
    }

    #[test]
    fn source_hash_drift_is_rejected() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-negative-diagnostics-{}-{unique}",
            std::process::id()
        ));
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(
            suite.join("test/a.js"),
            "/*---\nnegative:\n  phase: parse\n  type: SyntaxError\n---*/\n",
        )
        .unwrap();
        let source = format!("{HEADER}\n{}", row("test/a.js", "3\t7\texact"));
        let parsed = NegativeDiagnostics::parse(&source).unwrap();
        let error = parsed.validate_sources(&suite).unwrap_err();
        fs::remove_dir_all(suite).unwrap();
        assert!(error.contains("source hash drifted"), "{error}");
    }

    #[test]
    fn diagnostic_contract_classifies_message_and_location_mismatches() {
        let source = format!("{HEADER}\n{}", row("test/a.js", "3\t7\texact"));
        let parsed = NegativeDiagnostics::parse(&source).unwrap();
        let expectation = parsed
            .get(PathBuf::from("test/a.js").as_path(), Variant::Sloppy)
            .unwrap();

        let mut matching = WorkerResult::pass_with_diagnostic(
            "parse",
            "SyntaxError",
            "invalid import binding",
            Some(3),
            Some(7),
        );
        expectation.classify(&mut matching);
        assert_eq!(matching.outcome, "pass");

        for mut mismatch in [
            WorkerResult::pass_with_diagnostic(
                "parse",
                "SyntaxError",
                "invalid variable name in strict mode",
                Some(3),
                Some(7),
            ),
            WorkerResult::pass_with_diagnostic(
                "parse",
                "SyntaxError",
                "invalid import binding",
                Some(3),
                Some(8),
            ),
        ] {
            expectation.classify(&mut mismatch);
            assert_eq!(mismatch.outcome, "fail-negative-diagnostic-mismatch");
        }
    }
}
