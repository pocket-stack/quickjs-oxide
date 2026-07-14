//! Process-isolated Test262 runner for the pinned QuickJS compatibility suite.
//!
//! The metadata/configuration model follows QuickJS 2026-06-04
//! `run-test262.c`. Each runnable script variant runs in a fresh process so a
//! future engine crash or an already-possible infinite loop is reported
//! without taking down the coordinator.

#[path = "run_test262/capabilities.rs"]
mod capabilities;
#[path = "run_test262/config.rs"]
mod config;
#[path = "run_test262/execution.rs"]
mod execution;
#[path = "run_test262/metadata.rs"]
mod metadata;
#[path = "run_test262/report.rs"]
mod report;
#[path = "run_test262/requirements.rs"]
mod requirements;
#[path = "run_test262/scheduler.rs"]
mod scheduler;

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use capabilities::OxideProfile;
use config::{parse_config, skip_reason, validate_config, validate_suite, verify_sha256};
use execution::{run_isolated_worker, run_worker};
use metadata::{Metadata, parse_metadata};
use report::{WorkerResult, report_row, write_report};
use requirements::missing_host_capability_hints;
use scheduler::run_bounded;

const TEST262_COMMIT: &str = "5c8206929d81b2d3d727ca6aac56c18358c8d790";
const TEST262_PATCH_SHA256: &str =
    "f4b23b04641d438df0826fb17d7a5db276af2bdb085b42cc09aa8d50e0da9ba3";
const TEST262_CONFIG_SHA256: &str =
    "79c64748ff1182baf5433d0a8378e3666738a785d02faf71f0d459ed42ae897b";
const TEST262_METADATA_SHA256: &str =
    "a37219960819e56a5c5c1723d31d6a33095c778bf5347385187fde96f927a06a";
const TEST262_OXIDE_PROFILE_SHA256: &str =
    "f9bf8afb9a1147cac24da1b3cb8b65d473a8470b5f7ef0418ce4e0add8497560";
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
    oxide_profile: PathBuf,
    manifest: Option<PathBuf>,
    tests: Vec<PathBuf>,
    all: bool,
    report: PathBuf,
    mode: TestMode,
    timeout: Duration,
    workers: usize,
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
    let mut oxide_profile = None;
    let mut manifest = None;
    let mut tests = Vec::new();
    let mut report = None;
    let mut mode = TestMode::Both;
    let mut mode_explicit = false;
    let mut timeout_ms = DEFAULT_TIMEOUT_MS;
    let mut timeout_explicit = false;
    let mut workers = None;
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
            "--oxide-profile" => {
                oxide_profile = Some(PathBuf::from(take_value("--oxide-profile")?));
            }
            "--manifest" => manifest = Some(PathBuf::from(take_value("--manifest")?)),
            "--test" => tests.push(PathBuf::from(take_value("--test")?)),
            "--report" => report = Some(PathBuf::from(take_value("--report")?)),
            "--mode" => {
                mode = TestMode::parse(&take_value("--mode")?)?;
                mode_explicit = true;
            }
            "--variant" => variant = Some(Variant::parse(&take_value("--variant")?)?),
            "--timeout-ms" => {
                timeout_explicit = true;
                timeout_ms = take_value("--timeout-ms")?
                    .parse::<u64>()
                    .map_err(|_| "--timeout-ms must be an unsigned integer".to_owned())?;
                if timeout_ms == 0 {
                    return Err("--timeout-ms must be greater than zero".to_owned());
                }
            }
            "--workers" => {
                let value = take_value("--workers")?
                    .parse::<usize>()
                    .map_err(|_| "--workers must be a positive integer".to_owned())?;
                if value == 0 {
                    return Err("--workers must be greater than zero".to_owned());
                }
                workers = Some(value);
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
            || oxide_profile.is_some()
            || variant.is_some()
            || allow_failures
            || mode_explicit
            || timeout_explicit
            || workers.is_some()
        {
            return Err("--validate-metadata cannot be combined with execution options".to_owned());
        }
        return Ok(Invocation::MetadataAudit(MetadataAuditOptions {
            suite,
            records,
        }));
    }
    if worker {
        if all
            || manifest.is_some()
            || tests.len() != 1
            || report.is_some()
            || config.is_some()
            || oxide_profile.is_some()
            || allow_failures
            || mode_explicit
            || timeout_explicit
            || workers.is_some()
        {
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
    let oxide_profile = oxide_profile.ok_or_else(|| "--oxide-profile is required".to_owned())?;
    let report = report.ok_or_else(|| "--report is required".to_owned())?;
    Ok(Invocation::Coordinator(CoordinatorOptions {
        suite,
        config,
        oxide_profile,
        manifest,
        tests,
        all,
        report,
        mode,
        timeout: Duration::from_millis(timeout_ms),
        workers: workers.unwrap_or_else(default_worker_count),
        allow_failures,
    }))
}

fn print_help() {
    let default_workers = default_worker_count();
    println!(
        "run-test262 (quickjs-oxide)\n\
usage: run-test262 --suite DIR --config FILE --oxide-profile FILE (--manifest FILE | --test FILE... | --all) --report FILE [options]\n\
\n\
  --mode MODE          both, strict, sloppy, default-strict, or default-sloppy\n\
  --timeout-ms N       hard per-variant worker timeout (default: {DEFAULT_TIMEOUT_MS})\n\
  --workers N          maximum concurrent subprocesses (default: {default_workers})\n\
  --allow-failures     record a baseline without returning a failing status\n\
  --validate-metadata FILE\n\
                       serialize the complete pinned metadata inventory\n\
\n\
Every variant runs in a fresh subprocess. Module and async tests are reported\n\
as unsupported until those runtime surfaces are implemented."
    );
}

fn default_worker_count() -> usize {
    let available = thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let quickjs_style = if available >= 8 {
        available - 1
    } else {
        available
    };
    quickjs_style.clamp(1, 16)
}

struct PlannedTest {
    relative: PathBuf,
    metadata: Metadata,
}

struct PlannedRow {
    test_index: usize,
    variant: Option<Variant>,
    result: Option<WorkerResult>,
}

#[derive(Clone, Copy)]
struct RunnableJob {
    row_index: usize,
    test_index: usize,
    variant: Variant,
}

fn run_coordinator(options: &CoordinatorOptions) -> Result<bool, String> {
    validate_suite(&options.suite)?;
    validate_config(&options.config)?;
    verify_sha256(
        &options.oxide_profile,
        TEST262_OXIDE_PROFILE_SHA256,
        "quickjs-oxide Test262 capability profile",
    )?;
    let config = parse_config(&options.config)?;
    let oxide_profile = OxideProfile::load(&options.oxide_profile)?;
    validate_oxide_profile(&oxide_profile, &options.suite)?;
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
    let mut planned_tests = Vec::with_capacity(tests.len());
    let mut planned_rows = Vec::new();
    let mut runnable_jobs = Vec::new();

    for relative in tests {
        let path = options.suite.join(&relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse metadata for {}: {error}", relative.display()))?;
        let variants = metadata.variants(options.mode);
        let skip = skip_reason(&relative, &metadata, &config);
        let missing_host = missing_host_capability_hints(&relative, &source, &metadata);
        let capability =
            oxide_profile.classify(&relative, &metadata.features, metadata.negative.is_some());
        let selection_result = if let Some((outcome, detail)) = &skip {
            Some(WorkerResult::failure(outcome, "selection", "", detail))
        } else if let Some(result) = missing_host_result(&missing_host) {
            Some(result)
        } else {
            capability.map(|classification| {
                WorkerResult::failure(
                    classification.outcome,
                    "selection",
                    "EngineCapability",
                    classification.detail,
                )
            })
        };
        let test_index = planned_tests.len();
        planned_tests.push(PlannedTest { relative, metadata });

        if variants.is_empty() {
            planned_rows.push(PlannedRow {
                test_index,
                variant: None,
                result: Some(WorkerResult::failure(
                    "skipped-mode",
                    "selection",
                    "",
                    "variant excluded by mode",
                )),
            });
            continue;
        }

        for variant in variants {
            let row_index = planned_rows.len();
            planned_rows.push(PlannedRow {
                test_index,
                variant: Some(variant),
                result: selection_result.clone(),
            });
            if selection_result.is_none() {
                runnable_jobs.push(RunnableJob {
                    row_index,
                    test_index,
                    variant,
                });
            }
        }
    }

    let worker_results = run_bounded(runnable_jobs.len(), options.workers, |job_index| {
        let job = runnable_jobs[job_index];
        let test = &planned_tests[job.test_index];
        run_isolated_worker(
            &executable,
            &options.suite,
            &test.relative,
            job.variant,
            options.timeout,
        )
    })?;
    for (job, result) in runnable_jobs.iter().zip(worker_results) {
        planned_rows[job.row_index].result = Some(result);
    }

    let mut rows = Vec::with_capacity(planned_rows.len());
    let mut summary = BTreeMap::<String, usize>::new();
    for row in planned_rows {
        let test = &planned_tests[row.test_index];
        let result = row
            .result
            .ok_or_else(|| format!("missing result for {}", test.relative.display()))?;
        *summary.entry(result.outcome.clone()).or_default() += 1;
        rows.push(report_row(
            &test.relative,
            row.variant.map_or("none", Variant::name),
            &test.metadata,
            &result,
        ));
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
    println!(
        "execution: runnable={} workers={}",
        runnable_jobs.len(),
        options.workers.min(runnable_jobs.len())
    );
    println!("report={}", options.report.display());
    Ok(options.allow_failures || failed == 0)
}

fn validate_oxide_profile(profile: &OxideProfile, suite: &Path) -> Result<(), String> {
    for relative in profile.audited_negative_paths() {
        let relative = Path::new(relative);
        validate_relative_test_path(relative)?;
        let path = suite.join(relative);
        let source = fs::read_to_string(&path)
            .map_err(|error| format!("read audited negative {}: {error}", path.display()))?;
        let metadata = parse_metadata(&source).map_err(|error| {
            format!(
                "parse metadata for audited negative {}: {error}",
                relative.display()
            )
        })?;
        if metadata.negative.is_none() {
            return Err(format!(
                "oxide profile path is not a negative test: {}",
                relative.display()
            ));
        }
    }
    Ok(())
}

fn missing_host_result(missing: &[String]) -> Option<WorkerResult> {
    if missing.is_empty() {
        return None;
    }
    let has_module = missing.iter().any(|capability| capability == "module");
    let has_async = missing.iter().any(|capability| capability == "async");
    let detail = format!("missing execution capabilities: {}", missing.join(", "));
    if has_module || has_async {
        let outcome = match (has_module, has_async) {
            (true, true) => "unsupported-module-async",
            (true, false) => "unsupported-module",
            (false, true) => "unsupported-async",
            (false, false) => unreachable!(),
        };
        return Some(WorkerResult::failure(
            outcome,
            "selection",
            "ExecutionMode",
            detail,
        ));
    }

    let first = missing.first().expect("missing capabilities were checked");
    let outcome = match first.as_str() {
        "abstract-module-source" => "unsupported-host-abstract-module-source",
        "agent" => "unsupported-host-agent",
        "can-block:false" => "unsupported-host-can-block-false",
        "create-realm" => "unsupported-host-create-realm",
        "detach-array-buffer" => "unsupported-host-detach-array-buffer",
        "eval-script" => "unsupported-host-eval-script",
        "gc" => "unsupported-host-gc",
        "global" => "unsupported-host-global",
        "is-html-dda" => "unsupported-host-is-html-dda",
        unknown if unknown.starts_with("unknown:") => "unsupported-host-unknown-hook",
        _ => "unsupported-host",
    };
    Some(WorkerResult::failure(
        outcome,
        "selection",
        "HostCapability",
        detail,
    ))
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

#[cfg(test)]
mod cli_tests {
    use std::ffi::OsString;

    use super::{Invocation, default_worker_count, parse_args};

    fn parse(values: &[&str]) -> Result<Invocation, String> {
        parse_args(values.iter().map(OsString::from))
    }

    fn parse_error(values: &[&str]) -> String {
        match parse(values) {
            Ok(_) => panic!("arguments unexpectedly parsed"),
            Err(error) => error,
        }
    }

    #[test]
    fn coordinator_accepts_an_explicit_positive_worker_bound() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "compat/test262-oxide.conf",
            "--manifest",
            "manifest",
            "--report",
            "report.tsv",
            "--workers",
            "3",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(options.workers, 3);
    }

    #[test]
    fn zero_workers_and_missing_profile_are_rejected() {
        let zero = parse_error(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "profile",
            "--all",
            "--report",
            "report.tsv",
            "--workers",
            "0",
        ]);
        assert_eq!(zero, "--workers must be greater than zero");

        let missing = parse_error(&["--suite", "suite", "--all", "--report", "report.tsv"]);
        assert_eq!(missing, "--oxide-profile is required");
    }

    #[test]
    fn internal_and_metadata_modes_reject_coordinator_tuning() {
        let audit = parse_error(&[
            "--suite",
            "suite",
            "--validate-metadata",
            "records",
            "--workers",
            "2",
        ]);
        assert!(audit.contains("cannot be combined"));

        let worker = parse_error(&[
            "--worker-one",
            "--suite",
            "suite",
            "--test",
            "test/a.js",
            "--variant",
            "sloppy",
            "--timeout-ms",
            "10",
        ]);
        assert_eq!(worker, "invalid coordinator option passed to --worker-one");
    }

    #[test]
    fn automatic_worker_bound_is_nonzero_and_capped() {
        assert!((1..=16).contains(&default_worker_count()));
    }
}
