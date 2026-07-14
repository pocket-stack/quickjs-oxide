//! Process-isolated Test262 runner for the pinned QuickJS compatibility suite.
//!
//! The metadata/configuration model follows QuickJS 2026-06-04
//! `run-test262.c`. Each script variant runs in a fresh process so a future
//! engine crash or an already-possible infinite loop is reported without
//! taking down the coordinator.

#[path = "run_test262/config.rs"]
mod config;
#[path = "run_test262/execution.rs"]
mod execution;
#[path = "run_test262/metadata.rs"]
mod metadata;
#[path = "run_test262/report.rs"]
mod report;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use config::{parse_config, skip_reason, validate_config, validate_suite};
use execution::{run_isolated_worker, run_worker};
use metadata::parse_metadata;
use report::{WorkerResult, report_row, write_report};

const TEST262_COMMIT: &str = "5c8206929d81b2d3d727ca6aac56c18358c8d790";
const TEST262_PATCH_SHA256: &str =
    "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3";
const TEST262_CONFIG_SHA256: &str =
    "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b";
const TEST262_METADATA_SHA256: &str =
    "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a";
const QUICKJS_VERSION: &str = "2026-06-04";
const DEFAULT_TIMEOUT_MS: u64 = 5_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Variant {
    Sloppy,
    Strict,
}

impl Variant {
    fn name(self) -> &'static str {
        match self {
            Self::Sloppy => "sloppy",
            Self::Strict => "strict",
        }
    }

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "sloppy" => Ok(Self::Sloppy),
            "strict" => Ok(Self::Strict),
            _ => Err(format!("unknown Test262 variant: {value}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum TestMode {
    DefaultSloppy,
    DefaultStrict,
    Sloppy,
    Strict,
    Both,
}

impl TestMode {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default" | "default-sloppy" | "default-nostrict" => Ok(Self::DefaultSloppy),
            "default-strict" => Ok(Self::DefaultStrict),
            "sloppy" | "nostrict" => Ok(Self::Sloppy),
            "strict" => Ok(Self::Strict),
            "both" | "all" => Ok(Self::Both),
            _ => Err(format!("unknown Test262 mode: {value}")),
        }
    }

    fn name(self) -> &'static str {
        match self {
            Self::DefaultSloppy => "default-sloppy",
            Self::DefaultStrict => "default-strict",
            Self::Sloppy => "sloppy",
            Self::Strict => "strict",
            Self::Both => "both",
        }
    }
}

#[derive(Clone, Debug)]
struct CoordinatorOptions {
    suite: PathBuf,
    config: PathBuf,
    manifest: Option<PathBuf>,
    tests: Vec<PathBuf>,
    all: bool,
    report: PathBuf,
    mode: TestMode,
    timeout: Duration,
    allow_failures: bool,
}

#[derive(Clone, Debug)]
struct WorkerOptions {
    suite: PathBuf,
    test: PathBuf,
    variant: Variant,
}

#[derive(Clone, Debug)]
struct MetadataAuditOptions {
    suite: PathBuf,
    records: PathBuf,
}

enum Invocation {
    Coordinator(CoordinatorOptions),
    Worker(WorkerOptions),
    MetadataAudit(MetadataAuditOptions),
    Help,
}

fn main() -> ExitCode {
    let invocation = match parse_args(env::args_os().skip(1)) {
        Ok(invocation) => invocation,
        Err(error) => {
            eprintln!("run-test262: {error}");
            eprintln!("run-test262: use --help for usage");
            return ExitCode::from(2);
        }
    };
    match invocation {
        Invocation::Help => {
            print_help();
            ExitCode::SUCCESS
        }
        Invocation::Worker(options) => match run_worker(&options) {
            Ok(result) => {
                println!("{}", result.encode());
                ExitCode::SUCCESS
            }
            Err(error) => {
                println!(
                    "{}",
                    WorkerResult::failure("runner-error", "host", "", error).encode()
                );
                ExitCode::SUCCESS
            }
        },
        Invocation::MetadataAudit(options) => match audit_metadata(&options) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("run-test262 metadata audit: {error}");
                ExitCode::from(2)
            }
        },
        Invocation::Coordinator(options) => match run_coordinator(&options) {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(error) => {
                eprintln!("run-test262: {error}");
                ExitCode::from(2)
            }
        },
    }
}

fn parse_args(arguments: impl Iterator<Item = OsString>) -> Result<Invocation, String> {
    let arguments = arguments
        .map(|value| {
            value
                .into_string()
                .map_err(|_| "arguments must be valid UTF-8".to_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;
    if arguments
        .iter()
        .any(|argument| argument == "--help" || argument == "-h")
    {
        return Ok(Invocation::Help);
    }

    let worker = arguments.iter().any(|argument| argument == "--worker-one");
    let mut suite = None;
    let mut config = None;
    let mut manifest = None;
    let mut tests = Vec::new();
    let mut report = None;
    let mut mode = TestMode::Both;
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    let mut all = false;
    let mut allow_failures = false;
    let mut variant = None;
    let mut metadata_records = None;
    let mut index = 0;
    while index < arguments.len() {
        let argument = &arguments[index];
        index += 1;
        let mut take_value = |name: &str| -> Result<String, String> {
            let value = arguments
                .get(index)
                .cloned()
                .ok_or_else(|| format!("{name} requires a value"))?;
            index += 1;
            Ok(value)
        };
        match argument.as_str() {
            "--worker-one" => {}
            "--suite" => suite = Some(PathBuf::from(take_value("--suite")?)),
            "--config" => config = Some(PathBuf::from(take_value("--config")?)),
            "--manifest" => manifest = Some(PathBuf::from(take_value("--manifest")?)),
            "--test" => tests.push(PathBuf::from(take_value("--test")?)),
            "--report" => report = Some(PathBuf::from(take_value("--report")?)),
            "--mode" => mode = TestMode::parse(&take_value("--mode")?)?,
            "--variant" => variant = Some(Variant::parse(&take_value("--variant")?)?),
            "--timeout-ms" => {
                timeout_ms = take_value("--timeout-ms")?
                    .parse::<u64>()
                    .map_err(|_| "--timeout-ms must be an unsigned integer".to_owned())?;
                if timeout_ms == 0 {
                    return Err("--timeout-ms must be greater than zero".to_owned());
                }
            }
            "--all" => all = true,
            "--allow-failures" => allow_failures = true,
            "--validate-metadata" => {
                metadata_records = Some(PathBuf::from(take_value("--validate-metadata")?));
            }
            unknown => return Err(format!("unknown option: {unknown}")),
        }
    }

    let suite = suite.ok_or_else(|| "--suite is required".to_owned())?;
    if let Some(records) = metadata_records {
        if worker
            || all
            || manifest.is_some()
            || !tests.is_empty()
            || report.is_some()
            || config.is_some()
            || variant.is_some()
            || allow_failures
        {
            return Err("--validate-metadata cannot be combined with execution options".to_owned());
        }
        return Ok(Invocation::MetadataAudit(MetadataAuditOptions {
            suite,
            records,
        }));
    }
    if worker {
        if all || manifest.is_some() || tests.len() != 1 || report.is_some() || config.is_some() {
            return Err("invalid coordinator option passed to --worker-one".to_owned());
        }
        return Ok(Invocation::Worker(WorkerOptions {
            suite,
            test: tests.remove(0),
            variant: variant.ok_or_else(|| "--worker-one requires --variant".to_owned())?,
        }));
    }
    if variant.is_some() {
        return Err("--variant is internal to --worker-one".to_owned());
    }
    let input_count =
        usize::from(all) + usize::from(manifest.is_some()) + usize::from(!tests.is_empty());
    if input_count != 1 {
        return Err("select exactly one of --all, --manifest, or one-or-more --test".to_owned());
    }
    let config = config.unwrap_or_else(|| {
        suite
            .parent()
            .unwrap_or(Path::new("."))
            .join("test262.conf")
    });
    let report = report.ok_or_else(|| "--report is required".to_owned())?;
    Ok(Invocation::Coordinator(CoordinatorOptions {
        suite,
        config,
        manifest,
        tests,
        all,
        report,
        mode,
        timeout: Duration::from_millis(timeout_ms),
        allow_failures,
    }))
}

fn print_help() {
    println!(
        "run-test262 (quickjs-oxide)\n\
usage: run-test262 --suite DIR --config FILE (--manifest FILE | --test FILE... | --all) --report FILE [options]\n\
\n\
  --mode MODE          both, strict, sloppy, default-strict, or default-sloppy\n\
  --timeout-ms N       hard per-variant worker timeout (default: {DEFAULT_TIMEOUT_MS})\n\
  --allow-failures     record a baseline without returning a failing status\n\
  --validate-metadata FILE\n\
                       serialize the complete pinned metadata inventory\n\
\n\
Every variant runs in a fresh subprocess. Module and async tests are reported\n\
as unsupported until those runtime surfaces are implemented."
    );
}

fn run_coordinator(options: &CoordinatorOptions) -> Result<bool, String> {
    validate_suite(&options.suite)?;
    validate_config(&options.config)?;
    let config = parse_config(&options.config)?;
    let harness_dir = config
        .harness_dir
        .clone()
        .unwrap_or_else(|| options.suite.join("harness"));
    if !harness_dir.is_dir() {
        return Err(format!(
            "harness directory is missing: {}",
            harness_dir.display()
        ));
    }
    let actual_harness = fs::canonicalize(&harness_dir)
        .map_err(|error| format!("resolve {}: {error}", harness_dir.display()))?;
    let suite_harness = fs::canonicalize(options.suite.join("harness"))
        .map_err(|error| format!("resolve suite harness: {error}"))?;
    if actual_harness != suite_harness {
        return Err(format!(
            "pinned config harness does not match the suite harness: {}",
            harness_dir.display()
        ));
    }
    let tests = collect_tests(options)?;
    let executable = env::current_exe().map_err(|error| format!("locate runner: {error}"))?;
    let mut rows = Vec::new();
    let mut summary = BTreeMap::<String, usize>::new();

    for relative in tests {
        let path = options.suite.join(&relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse metadata for {}: {error}", relative.display()))?;
        let variants = metadata.variants(options.mode);
        if variants.is_empty() {
            rows.push(report_row(
                &relative,
                "none",
                &metadata,
                &WorkerResult::failure("skipped-mode", "", "", "variant excluded by mode"),
            ));
            *summary.entry("skipped-mode".to_owned()).or_default() += 1;
            continue;
        }

        let skip = skip_reason(&relative, &metadata, &config);
        for variant in variants {
            let result = if let Some((outcome, detail)) = &skip {
                WorkerResult::failure(outcome, "", "", detail)
            } else if metadata.is_module() {
                WorkerResult::failure(
                    "unsupported-module",
                    "host",
                    "",
                    "module parse/link/evaluate is not implemented",
                )
            } else if metadata.is_async() {
                WorkerResult::failure(
                    "unsupported-async",
                    "host",
                    "",
                    "Promise jobs and $DONE are not implemented",
                )
            } else {
                run_isolated_worker(
                    &executable,
                    &options.suite,
                    &relative,
                    variant,
                    options.timeout,
                )?
            };
            *summary.entry(result.outcome.clone()).or_default() += 1;
            rows.push(report_row(&relative, variant.name(), &metadata, &result));
        }
    }

    write_report(options, &rows, &summary)?;
    let total = rows.len();
    let passed = summary.get("pass").copied().unwrap_or(0);
    let skipped = summary
        .iter()
        .filter(|(name, _)| name.starts_with("skipped-"))
        .map(|(_, count)| *count)
        .sum::<usize>();
    let unsupported = summary
        .iter()
        .filter(|(name, _)| name.starts_with("unsupported-"))
        .map(|(_, count)| *count)
        .sum::<usize>();
    let failed = total.saturating_sub(passed + skipped);
    println!(
        "Test262: total={total} pass={passed} fail={failed} unsupported={unsupported} skipped={skipped}"
    );
    println!("report={}", options.report.display());
    Ok(options.allow_failures || failed == 0)
}

fn audit_metadata(options: &MetadataAuditOptions) -> Result<(), String> {
    validate_suite(&options.suite)?;
    let mut tests = Vec::new();
    collect_js_files(&options.suite.join("test"), &options.suite, &mut tests)?;
    sort_test_paths(&mut tests);
    if tests.len() != 53_125 {
        return Err(format!(
            "pinned metadata inventory has {} tests instead of 53125",
            tests.len()
        ));
    }

    let mut records = Vec::new();
    let mut counts = BTreeMap::<String, usize>::new();
    for relative in tests {
        let path = options.suite.join(&relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        if !source.contains("/*---") {
            return Err(format!("frontmatter is missing: {}", relative.display()));
        }
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse metadata for {}: {error}", relative.display()))?;
        let (phase, error_type) = if let Some(negative) = &metadata.negative {
            let phase = negative
                .phase
                .as_deref()
                .ok_or_else(|| format!("negative phase is missing: {}", relative.display()))?;
            if !matches!(phase, "parse" | "resolution" | "runtime") {
                return Err(format!(
                    "unknown negative phase {phase:?}: {}",
                    relative.display()
                ));
            }
            let error_type = negative
                .error_type
                .as_deref()
                .ok_or_else(|| format!("negative type is missing: {}", relative.display()))?;
            *counts.entry("negative".to_owned()).or_default() += 1;
            *counts.entry(format!("phase:{phase}")).or_default() += 1;
            (phase, error_type)
        } else {
            *counts.entry("positive".to_owned()).or_default() += 1;
            ("", "")
        };
        for flag in ["raw", "module", "async", "noStrict", "onlyStrict"] {
            if metadata.flags.contains(flag) {
                *counts.entry(flag.to_owned()).or_default() += 1;
            }
        }

        write_record_field(&mut records, &relative.to_string_lossy());
        write_record_field(&mut records, &metadata.includes.join(","));
        write_record_field(
            &mut records,
            &metadata.flags.iter().cloned().collect::<Vec<_>>().join(","),
        );
        write_record_field(&mut records, &metadata.features.join(","));
        write_record_field(&mut records, phase);
        records.extend_from_slice(error_type.as_bytes());
        records.push(b'\n');
    }

    if let Some(parent) = options
        .records
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)
            .map_err(|error| format!("create {}: {error}", parent.display()))?;
    }
    fs::write(&options.records, records)
        .map_err(|error| format!("write {}: {error}", options.records.display()))?;
    println!("Test262 metadata: files=53125");
    for name in [
        "raw",
        "module",
        "async",
        "noStrict",
        "onlyStrict",
        "positive",
        "negative",
        "phase:parse",
        "phase:resolution",
        "phase:runtime",
    ] {
        println!("{name}={}", counts.get(name).copied().unwrap_or(0));
    }
    println!("records={}", options.records.display());
    Ok(())
}

fn write_record_field(output: &mut Vec<u8>, value: &str) {
    output.extend_from_slice(value.as_bytes());
    output.push(0);
}

fn collect_tests(options: &CoordinatorOptions) -> Result<Vec<PathBuf>, String> {
    let values = if let Some(manifest) = &options.manifest {
        fs::read_to_string(manifest)
            .map_err(|error| format!("read {}: {error}", manifest.display()))?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .map(PathBuf::from)
            .collect::<Vec<_>>()
    } else if options.all {
        let mut values = Vec::new();
        collect_js_files(&options.suite.join("test"), &options.suite, &mut values)?;
        values
    } else {
        options.tests.clone()
    };
    let mut unique = BTreeSet::new();
    for value in values {
        validate_relative_test_path(&value)?;
        if !options.suite.join(&value).is_file() {
            return Err(format!("test file is missing: {}", value.display()));
        }
        if !unique.insert(value.clone()) {
            return Err(format!("duplicate test path: {}", value.display()));
        }
    }
    if unique.is_empty() {
        return Err("test selection is empty".to_owned());
    }
    let mut tests = unique.into_iter().collect::<Vec<_>>();
    sort_test_paths(&mut tests);
    Ok(tests)
}

fn sort_test_paths(paths: &mut [PathBuf]) {
    paths.sort_by(|left, right| {
        left.as_os_str()
            .as_encoded_bytes()
            .cmp(right.as_os_str().as_encoded_bytes())
    });
}

fn collect_js_files(
    directory: &Path,
    suite: &Path,
    output: &mut Vec<PathBuf>,
) -> Result<(), String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| format!("read {}: {error}", directory.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| format!("read {}: {error}", directory.display()))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| format!("stat {}: {error}", path.display()))?;
        if file_type.is_dir() {
            collect_js_files(&path, suite, output)?;
        } else if file_type.is_file()
            && path.extension().is_some_and(|extension| extension == "js")
            && !path
                .file_name()
                .is_some_and(|name| name.to_string_lossy().ends_with("_FIXTURE.js"))
        {
            output.push(
                path.strip_prefix(suite)
                    .map_err(|_| format!("{} escaped suite root", path.display()))?
                    .to_owned(),
            );
        }
    }
    Ok(())
}

fn validate_relative_test_path(path: &Path) -> Result<(), String> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
        || !path.starts_with("test")
        || path.extension().is_none_or(|extension| extension != "js")
        || path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains("_FIXTURE"))
    {
        return Err(format!("invalid relative Test262 path: {}", path.display()));
    }
    Ok(())
}
