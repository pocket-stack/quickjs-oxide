use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;

use super::metadata::Metadata;
use super::{
    CoordinatorOptions, QUICKJS_VERSION, TEST262_COMMIT, TEST262_CONFIG_SHA256,
    TEST262_METADATA_SHA256, TEST262_PATCH_SHA256,
};

#[derive(Clone, Debug)]
pub(super) struct WorkerResult {
    pub(super) outcome: String,
    pub(super) actual_phase: String,
    pub(super) actual_type: String,
    pub(super) detail: String,
}

impl WorkerResult {
    pub(super) fn pass(actual_phase: impl Into<String>, actual_type: impl Into<String>) -> Self {
        Self::pass_with_detail(actual_phase, actual_type, "")
    }

    pub(super) fn pass_with_detail(
        actual_phase: impl Into<String>,
        actual_type: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            outcome: "pass".to_owned(),
            actual_phase: actual_phase.into(),
            actual_type: actual_type.into(),
            detail: detail.into(),
        }
    }

    pub(super) fn failure(
        outcome: impl Into<String>,
        actual_phase: impl Into<String>,
        actual_type: impl Into<String>,
        detail: impl Into<String>,
    ) -> Self {
        Self {
            outcome: outcome.into(),
            actual_phase: actual_phase.into(),
            actual_type: actual_type.into(),
            detail: detail.into(),
        }
    }

    pub(super) fn encode(&self) -> String {
        [
            self.outcome.as_str(),
            self.actual_phase.as_str(),
            self.actual_type.as_str(),
            self.detail.as_str(),
        ]
        .map(escape_field)
        .join("\t")
    }

    pub(super) fn decode(value: &str) -> Result<Self, String> {
        let fields = value
            .trim_end_matches(['\r', '\n'])
            .split('\t')
            .collect::<Vec<_>>();
        if fields.len() != 4 {
            return Err(format!(
                "worker returned {} fields instead of four: {value:?}",
                fields.len()
            ));
        }
        Ok(Self {
            outcome: unescape_field(fields[0])?,
            actual_phase: unescape_field(fields[1])?,
            actual_type: unescape_field(fields[2])?,
            detail: unescape_field(fields[3])?,
        })
    }
}

pub(super) fn report_row(
    relative: &Path,
    variant: &str,
    metadata: &Metadata,
    result: &WorkerResult,
) -> String {
    let relative = relative.to_string_lossy();
    let flags = metadata.flags.iter().cloned().collect::<Vec<_>>().join(",");
    let features = metadata.features.join(",");
    let (expected_phase, expected_type) = metadata
        .negative
        .as_ref()
        .map(|negative| {
            (
                negative.phase.as_deref().unwrap_or("any"),
                negative.error_type.as_deref().unwrap_or("any"),
            )
        })
        .unwrap_or(("normal", ""));
    [
        relative.as_ref(),
        variant,
        flags.as_str(),
        features.as_str(),
        expected_phase,
        expected_type,
        result.outcome.as_str(),
        result.actual_phase.as_str(),
        result.actual_type.as_str(),
        result.detail.as_str(),
    ]
    .map(escape_field)
    .join("\t")
}

pub(super) fn write_report(
    options: &CoordinatorOptions,
    rows: &[String],
    summary: &BTreeMap<String, usize>,
    oxide_profile_sha256: &str,
) -> Result<(), String> {
    let json_path = options.report.with_extension("jsonl");
    if json_path == options.report {
        return Err(
            "--report must not use the .jsonl extension reserved for its sidecar".to_owned(),
        );
    }
    let mut output = String::new();
    output.push_str("# quickjs-oxide Test262 outcome vector v2\n");
    output.push_str(&format!("# quickjs={QUICKJS_VERSION}\n"));
    output.push_str(&format!("# test262={TEST262_COMMIT}\n"));
    output.push_str(&format!("# test262_patch_sha256={TEST262_PATCH_SHA256}\n"));
    output.push_str(&format!(
        "# test262_config_sha256={TEST262_CONFIG_SHA256}\n"
    ));
    output.push_str(&format!(
        "# test262_metadata_sha256={TEST262_METADATA_SHA256}\n"
    ));
    output.push_str(&format!("# oxide_profile_sha256={oxide_profile_sha256}\n"));
    output.push_str("# profile=test262-canonical-classified-v2\n");
    output.push_str(&format!("# mode={}\n", options.mode.name()));
    output.push_str("path\tvariant\tflags\tfeatures\texpected_phase\texpected_type\toutcome\tactual_phase\tactual_type\tdetail\n");
    for row in rows {
        output.push_str(row);
        output.push('\n');
    }
    output.push_str("# summary");
    for (outcome, count) in summary {
        output.push(' ');
        output.push_str(outcome);
        output.push('=');
        output.push_str(&count.to_string());
    }
    output.push('\n');
    if let Some(parent) = options
        .report
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    let mut file = fs::File::create(&options.report)
        .map_err(|error| format!("create {}: {error}", options.report.display()))?;
    file.write_all(output.as_bytes())
        .map_err(|error| format!("write {}: {error}", options.report.display()))?;

    let mut json = String::new();
    json.push_str(&format!(
        "{{\"kind\":\"metadata\",\"schema\":2,\"quickjs\":{},\"test262\":{},\"test262_patch_sha256\":{},\"test262_config_sha256\":{},\"test262_metadata_sha256\":{},\"oxide_profile_sha256\":{},\"profile\":\"test262-canonical-classified-v2\",\"mode\":{}}}\n",
        json_string(QUICKJS_VERSION),
        json_string(TEST262_COMMIT),
        json_string(TEST262_PATCH_SHA256),
        json_string(TEST262_CONFIG_SHA256),
        json_string(TEST262_METADATA_SHA256),
        json_string(oxide_profile_sha256),
        json_string(options.mode.name()),
    ));
    for row in rows {
        json.push_str(&json_report_row(row)?);
        json.push('\n');
    }
    json.push_str("{\"kind\":\"summary\",\"outcomes\":{");
    for (index, (outcome, count)) in summary.iter().enumerate() {
        if index != 0 {
            json.push(',');
        }
        json.push_str(&json_string(outcome));
        json.push(':');
        json.push_str(&count.to_string());
    }
    json.push_str("}}\n");
    fs::write(&json_path, json).map_err(|error| format!("write {}: {error}", json_path.display()))
}

fn json_report_row(row: &str) -> Result<String, String> {
    const NAMES: [&str; 10] = [
        "path",
        "variant",
        "flags",
        "features",
        "expected_phase",
        "expected_type",
        "outcome",
        "actual_phase",
        "actual_type",
        "detail",
    ];
    let fields = row.split('\t').collect::<Vec<_>>();
    if fields.len() != NAMES.len() {
        return Err(format!(
            "report row has {} fields instead of {}",
            fields.len(),
            NAMES.len()
        ));
    }
    let mut output = String::from("{\"kind\":\"result\"");
    for (name, field) in NAMES.into_iter().zip(fields) {
        output.push(',');
        output.push_str(&json_string(name));
        output.push(':');
        output.push_str(&json_string(&unescape_field(field)?));
    }
    output.push('}');
    Ok(output)
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\u{08}' => output.push_str("\\b"),
            '\u{0c}' => output.push_str("\\f"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character < '\u{20}' => {
                output.push_str(&format!("\\u{:04x}", u32::from(character)));
            }
            _ => output.push(character),
        }
    }
    output.push('"');
    output
}

fn escape_field(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '\\' => output.push_str("\\\\"),
            '\t' => output.push_str("\\t"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            character if character.is_control() => {
                output.push_str(&format!("\\u{:04x}", u32::from(character)));
            }
            _ => output.push(character),
        }
    }
    output
}

fn unescape_field(value: &str) -> Result<String, String> {
    let mut output = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            output.push(character);
            continue;
        }
        match characters.next() {
            Some('\\') => output.push('\\'),
            Some('t') => output.push('\t'),
            Some('n') => output.push('\n'),
            Some('r') => output.push('\r'),
            Some('u') => {
                let digits = characters.by_ref().take(4).collect::<String>();
                if digits.len() != 4 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
                    return Err(format!("invalid worker Unicode escape: \\u{digits}"));
                }
                let value = u32::from_str_radix(&digits, 16)
                    .map_err(|_| format!("invalid worker Unicode escape: \\u{digits}"))?;
                let decoded = char::from_u32(value)
                    .ok_or_else(|| format!("invalid worker Unicode escape: \\u{digits}"))?;
                output.push(decoded);
            }
            Some(other) => return Err(format!("invalid worker escape: \\{other}")),
            None => return Err("trailing worker escape".to_owned()),
        }
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::{WorkerResult, escape_field, unescape_field};

    #[test]
    fn report_field_codec_roundtrips_control_characters() {
        let value = "a\\b\\u0000\tc\nd\re\0\u{8}\u{c}\u{1f}\u{7f}\u{9f}";
        let encoded = escape_field(value);
        assert!(encoded.chars().all(|character| !character.is_control()));
        assert_eq!(unescape_field(&encoded).unwrap(), value);
        let result = WorkerResult::failure("fail-runtime", "runtime", "TypeError", value);
        assert_eq!(
            WorkerResult::decode(&result.encode()).unwrap().detail,
            value
        );
    }
}
