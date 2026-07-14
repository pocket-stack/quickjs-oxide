use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::metadata::Metadata;
use super::{TEST262_COMMIT, TEST262_CONFIG_SHA256, TEST262_PATCH_SHA256};

#[derive(Clone, Debug, Default)]
pub(super) struct SuiteConfig {
    pub(super) harness_dir: Option<PathBuf>,
    enabled_features: BTreeSet<String>,
    skipped_features: BTreeSet<String>,
    harness_exclude: BTreeSet<String>,
    excluded_tests: Vec<String>,
}

pub(super) fn validate_suite(suite: &Path) -> Result<(), String> {
    if !suite.join("test").is_dir() || !suite.join("harness").is_dir() {
        return Err(format!("not a Test262 checkout: {}", suite.display()));
    }
    let output = Command::new("git")
        .args(["-C"])
        .arg(suite)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|error| format!("run git for Test262 identity: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "could not read Test262 commit: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if commit != TEST262_COMMIT {
        return Err(format!(
            "unexpected Test262 commit: expected {TEST262_COMMIT}, found {commit}"
        ));
    }

    let source_dir = suite
        .parent()
        .ok_or_else(|| format!("Test262 suite has no parent: {}", suite.display()))?;
    let patch = source_dir.join("tests/test262.patch");
    verify_sha256(&patch, TEST262_PATCH_SHA256, "QuickJS Test262 patch")?;

    let status = Command::new("git")
        .args(["-C"])
        .arg(suite)
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .output()
        .map_err(|error| format!("inspect Test262 worktree: {error}"))?;
    if !status.status.success() {
        return Err(format!(
            "could not inspect Test262 worktree: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    let mut status_lines = String::from_utf8_lossy(&status.stdout)
        .lines()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    status_lines.sort();
    let expected_status = [
        " M harness/atomicsHelper.js".to_owned(),
        " M harness/regExpUtils.js".to_owned(),
    ];
    if status_lines != expected_status {
        return Err(format!(
            "Test262 worktree does not contain exactly the pinned QuickJS patch: {status_lines:?}"
        ));
    }

    let diff = Command::new("git")
        .args(["-C"])
        .arg(suite)
        .args([
            "diff",
            "--no-ext-diff",
            "--no-color",
            "--no-renames",
            "--abbrev=7",
            "--src-prefix=a/",
            "--dst-prefix=b/",
            "--",
            "harness/atomicsHelper.js",
            "harness/regExpUtils.js",
        ])
        .output()
        .map_err(|error| format!("inspect Test262 patch: {error}"))?;
    if !diff.status.success() {
        return Err(format!(
            "could not inspect Test262 patch: {}",
            String::from_utf8_lossy(&diff.stderr).trim()
        ));
    }
    let patch_bytes = fs::read(&patch)
        .map_err(|error| format!("read pinned patch {}: {error}", patch.display()))?;
    if diff.stdout != patch_bytes {
        return Err("Test262 harness diff does not exactly match the pinned patch".to_owned());
    }
    Ok(())
}

pub(super) fn validate_config(path: &Path) -> Result<(), String> {
    verify_sha256(path, TEST262_CONFIG_SHA256, "QuickJS Test262 config")
}

pub(super) fn verify_sha256(path: &Path, expected: &str, label: &str) -> Result<(), String> {
    if !path.is_file() {
        return Err(format!("{label} is missing: {}", path.display()));
    }
    let commands: [(&str, &[&str]); 2] = [("sha256sum", &[]), ("shasum", &["-a", "256"])];
    let mut unavailable = Vec::new();
    for (program, arguments) in commands {
        let output = match Command::new(program).args(arguments).arg(path).output() {
            Ok(output) => output,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                unavailable.push(program);
                continue;
            }
            Err(error) => return Err(format!("hash {label} with {program}: {error}")),
        };
        if !output.status.success() {
            return Err(format!(
                "hash {label} with {program}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let actual = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned();
        if actual != expected {
            return Err(format!(
                "{label} checksum mismatch: expected {expected}, found {actual}"
            ));
        }
        return Ok(());
    }
    Err(format!(
        "cannot hash {label}: commands are unavailable: {}",
        unavailable.join(", ")
    ))
}

pub(super) fn skip_reason(
    path: &Path,
    metadata: &Metadata,
    config: &SuiteConfig,
) -> Option<(String, String)> {
    let path = path.to_string_lossy().replace('\\', "/");
    let quickjs_path = format!("test262/{path}");
    if config.excluded_tests.iter().any(|excluded| {
        excluded
            .strip_suffix('/')
            .is_some_and(|prefix| quickjs_path.starts_with(&format!("{prefix}/")))
            || excluded == &quickjs_path
    }) {
        return Some((
            "skipped-config-exclude".to_owned(),
            "QuickJS config excludes this test".to_owned(),
        ));
    }
    for include in &metadata.includes {
        if config.harness_exclude.contains(include) {
            return Some((
                "skipped-harness".to_owned(),
                format!("QuickJS config excludes harness include {include}"),
            ));
        }
    }
    for feature in &metadata.features {
        if config.skipped_features.contains(feature) {
            return Some((
                "skipped-feature".to_owned(),
                format!("QuickJS config skips feature {feature}"),
            ));
        }
        if !config.enabled_features.contains(feature) {
            return Some((
                "unsupported-unknown-feature".to_owned(),
                format!("feature is absent from QuickJS config: {feature}"),
            ));
        }
    }
    None
}

pub(super) fn parse_config(path: &Path) -> Result<SuiteConfig, String> {
    let source =
        fs::read_to_string(path).map_err(|error| format!("read {}: {error}", path.display()))?;
    let base = path.parent().unwrap_or(Path::new("."));
    let mut config = SuiteConfig::default();
    let mut section = String::new();
    for raw_line in source.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            section = line[1..line.len() - 1].to_owned();
            continue;
        }
        let (name, value) = line.split_once('=').map_or((line, None), |(name, value)| {
            (name.trim(), Some(value.trim()))
        });
        match section.as_str() {
            "config" if name == "harnessdir" => {
                let value = value.ok_or_else(|| "harnessdir requires a value".to_owned())?;
                config.harness_dir = Some(base.join(value));
            }
            "config" if name == "harnessexclude" => {
                let value = value.ok_or_else(|| "harnessexclude requires a value".to_owned())?;
                config
                    .harness_exclude
                    .extend(value.split_whitespace().map(str::to_owned));
            }
            "config" if name == "features" => {
                let value = value.ok_or_else(|| "features requires a value".to_owned())?;
                config
                    .enabled_features
                    .extend(value.split_whitespace().map(str::to_owned));
            }
            "config" if name == "skip-features" => {
                let value = value.ok_or_else(|| "skip-features requires a value".to_owned())?;
                config
                    .skipped_features
                    .extend(value.split_whitespace().map(str::to_owned));
            }
            "features" => {
                if value.is_none() || value == Some("yes") {
                    config.enabled_features.insert(name.to_owned());
                } else {
                    config.skipped_features.insert(name.to_owned());
                }
            }
            "exclude" => config.excluded_tests.push(name.replace('\\', "/")),
            _ => {}
        }
    }
    Ok(config)
}
