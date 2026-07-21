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
    "a1a347d2d74c946a50f1e26fca6c1756c0e9948f087de3aed2339b3a4c7d6677";
const TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256: &str =
    "8232e2c11e908f7cbf5a9e0f34fbd5223a9551b49ae64647f2a72b2314bcaf84";
const TEST262_ARRAY_BINDING_FLAT_MANIFEST_SHA256: &str =
    "db17670a1f7715a325a07087b766f6e64cf2bb24cec727278db05db3f79ee679";
const TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256: &str =
    "c770387473b6ba2e273ab635182b5f07ae80ad902f48057ba5e2fb4f036c723e";
const TEST262_ARRAY_BINDING_NESTED_MANIFEST_SHA256: &str =
    "f7c7c181cdde65c84dfcb677cbe45f77884990666a774f952bc165df89f5e8a5";
const TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256: &str =
    "b2133d90974566c72ab788525254de68d260b44756a8c5981111873fb38727af";
const TEST262_ARRAY_ASSIGNMENT_FLAT_MANIFEST_SHA256: &str =
    "046679bd745132066b4982770f13236bfecdbd953b70bdba98afa60424c599c8";
const TEST262_CATCH_BINDING_PROFILE_SHA256: &str =
    "a654327057a974e0feab6799f3c99a3104884a403cbc41bbc85f3fc226328718";
const TEST262_CATCH_BINDING_MANIFEST_SHA256: &str =
    "e3fb469169b069c185a7d9ea6b8cdce2fdb54d49181b7e87e33cff59a27c212e";
const TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256: &str =
    "5c98d19ccb72c7e2c577ddc98ee4ac83d43a0ba7d49175a8ebe271866d0feab6";
const TEST262_IDENTIFIER_DEFAULTS_MANIFEST_SHA256: &str =
    "264bb2b25e7502eed86f8a5df1b3fe8c0ccdeecd43171af390764b5e053a6472";
const TEST262_IDENTIFIER_REST_PROFILE_SHA256: &str =
    "da6a76cb6338019f5c233e252bf6d40b7f3eb5c4235a6967cf78f9a74917dced";
const TEST262_IDENTIFIER_REST_MANIFEST_SHA256: &str =
    "cc326a73c13d2cd90726150e77ad5f5a247074f12a233fe9efa382b3ec6c420e";
const TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256: &str =
    "989f5617484d5c12a15fb26a447121fa3436b19f05cd998cf400b5d3d7179a51";
const TEST262_OBJECT_ASSIGNMENT_FLAT_MANIFEST_SHA256: &str =
    "92089af97dcc157d557061120dfdb68c868f2a8823288290a227a22bfadb285b";
const TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256: &str =
    "18411f3d674a9493806bbf6a601bda903e859395aeec572e466c4a59470ceb12";
const TEST262_OBJECT_ASSIGNMENT_NESTED_MANIFEST_SHA256: &str =
    "0e5a594cee6e1c021f310c8e9d88e8b253d789171c97511aec4adcfd346d7d27";
const TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256: &str =
    "4b9f50b982dc5c3af1466d425a1665448c4a00165d465a74fd4057ef6e414206";
const TEST262_OBJECT_ASSIGNMENT_REST_MANIFEST_SHA256: &str =
    "931d743e7e2f46d78e66baf7c7c83fcf33208fd8ced6f6c72619ec5948971226";
const TEST262_OBJECT_BINDING_PROFILE_SHA256: &str =
    "aa6cdca241b5f0be7eb202461ba80e44132f917a66480f1c04225cedc410d0d7";
const TEST262_OBJECT_BINDING_MANIFEST_SHA256: &str =
    "ab9974676a1f15442875d6b9de607a27a94a76896a949c8b9cf86b05dbac18dc";
const TEST262_OBJECT_REST_BINDING_PROFILE_SHA256: &str =
    "122a2b055aaf40672a0540441861ecd1e6c09b65e88d45b947bc27a691afc45e";
const TEST262_OBJECT_REST_BINDING_MANIFEST_SHA256: &str =
    "fc75564488d2ae45a015fa8b07989f3a178f08978221d87ffdeeca0a9359fe57";
const TEST262_MAP_PROFILE_SHA256: &str =
    "16ab6bfe18540aae398c847905f492491e81500045b45a6bfb21f447fd537ea2";
const TEST262_MAP_MANIFEST_SHA256: &str =
    "f369837ef69275815349f9202ade5b6ae1d4d91e9ae0313ac816ecfb0e3a4845";
const TEST262_SET_PROFILE_SHA256: &str =
    "6869e9d28fff1d5bd4e5b698dcdf6ee677b9134a91781ad7abe226200d669455";
const TEST262_SET_MANIFEST_SHA256: &str =
    "0f560c202e9463ff4896796be6e924db984e25bc3e95ae2604a54ce9dee61e9f";
const TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256: &str =
    "ff674aafc4b1b61b0c40042f831b44c600b1f741e06b8c8c35863b876919aa7b";
const TEST262_SYMBOL_PROTOCOLS_MANIFEST_SHA256: &str =
    "6147636f7950b899f7c0eea25078e2f4c9c4c7fda2977181dd7c9671aa0bcde2";
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
    let oxide_profile_sha256 = verify_oxide_profile(options)?;
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

    write_report(options, &rows, &summary, oxide_profile_sha256)?;
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OxideProfileKind {
    Global,
    ArrayBindingFlat,
    ArrayBindingNested,
    ArrayAssignmentFlat,
    CatchBinding,
    IdentifierDefaults,
    IdentifierRest,
    ObjectAssignmentFlat,
    ObjectAssignmentNested,
    ObjectAssignmentRest,
    ObjectBinding,
    ObjectRestBinding,
    Map,
    Set,
    SymbolProtocols,
}

fn identify_oxide_profile(path: &Path) -> Result<OxideProfileKind, String> {
    let actual = fs::canonicalize(path).map_err(|error| {
        format!(
            "resolve Test262 capability profile {}: {error}",
            path.display()
        )
    })?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let profiles = [
        (
            root.join("compat/test262-oxide.conf"),
            OxideProfileKind::Global,
        ),
        (
            root.join("tests/test262-array-binding-flat.conf"),
            OxideProfileKind::ArrayBindingFlat,
        ),
        (
            root.join("tests/test262-array-binding-nested.conf"),
            OxideProfileKind::ArrayBindingNested,
        ),
        (
            root.join("tests/test262-array-assignment-flat.conf"),
            OxideProfileKind::ArrayAssignmentFlat,
        ),
        (
            root.join("tests/test262-catch-binding.conf"),
            OxideProfileKind::CatchBinding,
        ),
        (
            root.join("tests/test262-identifier-defaults.conf"),
            OxideProfileKind::IdentifierDefaults,
        ),
        (
            root.join("tests/test262-identifier-rest.conf"),
            OxideProfileKind::IdentifierRest,
        ),
        (
            root.join("tests/test262-object-assignment-flat.conf"),
            OxideProfileKind::ObjectAssignmentFlat,
        ),
        (
            root.join("tests/test262-object-assignment-nested.conf"),
            OxideProfileKind::ObjectAssignmentNested,
        ),
        (
            root.join("tests/test262-object-assignment-rest.conf"),
            OxideProfileKind::ObjectAssignmentRest,
        ),
        (
            root.join("tests/test262-object-binding.conf"),
            OxideProfileKind::ObjectBinding,
        ),
        (
            root.join("tests/test262-object-rest-binding.conf"),
            OxideProfileKind::ObjectRestBinding,
        ),
        (root.join("tests/test262-map.conf"), OxideProfileKind::Map),
        (root.join("tests/test262-set.conf"), OxideProfileKind::Set),
        (
            root.join("tests/test262-symbol-protocols.conf"),
            OxideProfileKind::SymbolProtocols,
        ),
    ];
    for (candidate, kind) in profiles {
        let candidate = fs::canonicalize(&candidate).map_err(|error| {
            format!(
                "resolve pinned Test262 capability profile {}: {error}",
                candidate.display()
            )
        })?;
        if actual == candidate {
            return Ok(kind);
        }
    }
    Err(format!(
        "unsupported Test262 capability profile: {}; expected compat/test262-oxide.conf, tests/test262-array-binding-flat.conf, tests/test262-array-binding-nested.conf, tests/test262-array-assignment-flat.conf, tests/test262-catch-binding.conf, tests/test262-identifier-defaults.conf, tests/test262-identifier-rest.conf, tests/test262-object-assignment-flat.conf, tests/test262-object-assignment-nested.conf, tests/test262-object-assignment-rest.conf, tests/test262-object-binding.conf, tests/test262-object-rest-binding.conf, tests/test262-map.conf, tests/test262-set.conf, or tests/test262-symbol-protocols.conf",
        path.display()
    ))
}

fn verify_scoped_pinned_profile(
    options: &CoordinatorOptions,
    label: &str,
    profile_sha256: &'static str,
    manifest_relative: &str,
    manifest_sha256: &str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("scoped {label} Test262 capability profile"),
    )?;
    if options.all || !options.tests.is_empty() {
        return Err(format!(
            "the scoped {label} Test262 capability profile requires its pinned manifest"
        ));
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!("the scoped {label} Test262 capability profile requires its pinned manifest")
    })?;
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve scoped {label} manifest {}: {error}",
            manifest.display()
        )
    })?;
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest_relative))
        .map_err(|error| format!("resolve pinned scoped {label} manifest: {error}"))?;
    if actual != expected {
        return Err(format!(
            "the scoped {label} Test262 capability profile requires {manifest_relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        manifest_sha256,
        &format!("scoped {label} Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_scoped_object_assignment_profile(
    options: &CoordinatorOptions,
    cohort: &str,
    profile_sha256: &'static str,
    manifest_sha256: &str,
) -> Result<&'static str, String> {
    verify_sha256(
        &options.oxide_profile,
        profile_sha256,
        &format!("scoped {cohort} object assignment Test262 capability profile"),
    )?;
    if options.all || !options.tests.is_empty() {
        return Err(format!(
            "the scoped {cohort} object assignment Test262 capability profile requires its pinned manifest"
        ));
    }
    let manifest = options.manifest.as_ref().ok_or_else(|| {
        format!(
            "the scoped {cohort} object assignment Test262 capability profile requires its pinned manifest"
        )
    })?;
    let actual = fs::canonicalize(manifest).map_err(|error| {
        format!(
            "resolve scoped {cohort} object assignment manifest {}: {error}",
            manifest.display()
        )
    })?;
    let relative = format!("tests/test262-object-assignment-{cohort}.txt");
    let expected = fs::canonicalize(Path::new(env!("CARGO_MANIFEST_DIR")).join(&relative))
        .map_err(|error| {
            format!("resolve pinned scoped {cohort} object assignment manifest: {error}")
        })?;
    if actual != expected {
        return Err(format!(
            "the scoped {cohort} object assignment Test262 capability profile requires {relative}, found {}",
            manifest.display()
        ));
    }
    verify_sha256(
        manifest,
        manifest_sha256,
        &format!("scoped {cohort} object assignment Test262 manifest"),
    )?;
    Ok(profile_sha256)
}

fn verify_oxide_profile(options: &CoordinatorOptions) -> Result<&'static str, String> {
    match identify_oxide_profile(&options.oxide_profile)? {
        OxideProfileKind::Global => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_OXIDE_PROFILE_SHA256,
                "global quickjs-oxide Test262 capability profile",
            )?;
            Ok(TEST262_OXIDE_PROFILE_SHA256)
        }
        OxideProfileKind::ArrayBindingFlat => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256,
                "scoped flat array binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped flat array binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped flat array binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped flat array binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-array-binding-flat.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped flat array binding manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped flat array binding Test262 capability profile requires tests/test262-array-binding-flat.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_ARRAY_BINDING_FLAT_MANIFEST_SHA256,
                "scoped flat array binding Test262 manifest",
            )?;
            Ok(TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256)
        }
        OxideProfileKind::ArrayBindingNested => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256,
                "scoped nested array binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped nested array binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped nested array binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped nested array binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/test262-array-binding-nested.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped nested array binding manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped nested array binding Test262 capability profile requires tests/test262-array-binding-nested.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_ARRAY_BINDING_NESTED_MANIFEST_SHA256,
                "scoped nested array binding Test262 manifest",
            )?;
            Ok(TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256)
        }
        OxideProfileKind::ArrayAssignmentFlat => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256,
                "scoped flat array assignment Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped flat array assignment Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped flat array assignment Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped flat array assignment manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/test262-array-assignment-flat.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped flat array assignment manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped flat array assignment Test262 capability profile requires tests/test262-array-assignment-flat.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_ARRAY_ASSIGNMENT_FLAT_MANIFEST_SHA256,
                "scoped flat array assignment Test262 manifest",
            )?;
            Ok(TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256)
        }
        OxideProfileKind::CatchBinding => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_CATCH_BINDING_PROFILE_SHA256,
                "scoped catch binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped catch binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped catch binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped catch binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-catch-binding.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped catch binding manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped catch binding Test262 capability profile requires tests/test262-catch-binding.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_CATCH_BINDING_MANIFEST_SHA256,
                "scoped catch binding Test262 manifest",
            )?;
            Ok(TEST262_CATCH_BINDING_PROFILE_SHA256)
        }
        OxideProfileKind::IdentifierDefaults => verify_scoped_pinned_profile(
            options,
            "identifier defaults",
            TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256,
            "tests/test262-identifier-defaults.txt",
            TEST262_IDENTIFIER_DEFAULTS_MANIFEST_SHA256,
        ),
        OxideProfileKind::IdentifierRest => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_IDENTIFIER_REST_PROFILE_SHA256,
                "scoped identifier rest Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped identifier rest Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped identifier rest Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped identifier rest manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-identifier-rest.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped identifier rest manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped identifier rest Test262 capability profile requires tests/test262-identifier-rest.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_IDENTIFIER_REST_MANIFEST_SHA256,
                "scoped identifier rest Test262 manifest",
            )?;
            Ok(TEST262_IDENTIFIER_REST_PROFILE_SHA256)
        }
        OxideProfileKind::ObjectAssignmentFlat => verify_scoped_object_assignment_profile(
            options,
            "flat",
            TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256,
            TEST262_OBJECT_ASSIGNMENT_FLAT_MANIFEST_SHA256,
        ),
        OxideProfileKind::ObjectAssignmentNested => verify_scoped_object_assignment_profile(
            options,
            "nested",
            TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256,
            TEST262_OBJECT_ASSIGNMENT_NESTED_MANIFEST_SHA256,
        ),
        OxideProfileKind::ObjectAssignmentRest => verify_scoped_object_assignment_profile(
            options,
            "rest",
            TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256,
            TEST262_OBJECT_ASSIGNMENT_REST_MANIFEST_SHA256,
        ),
        OxideProfileKind::ObjectBinding => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_OBJECT_BINDING_PROFILE_SHA256,
                "scoped object binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped object binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped object binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped object binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-object-binding.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped object binding manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped object binding Test262 capability profile requires tests/test262-object-binding.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_OBJECT_BINDING_MANIFEST_SHA256,
                "scoped object binding Test262 manifest",
            )?;
            Ok(TEST262_OBJECT_BINDING_PROFILE_SHA256)
        }
        OxideProfileKind::ObjectRestBinding => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_OBJECT_REST_BINDING_PROFILE_SHA256,
                "scoped object-rest binding Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped object-rest binding Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped object-rest binding Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped object-rest binding manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-object-rest-binding.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped object-rest binding manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped object-rest binding Test262 capability profile requires tests/test262-object-rest-binding.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_OBJECT_REST_BINDING_MANIFEST_SHA256,
                "scoped object-rest binding Test262 manifest",
            )?;
            Ok(TEST262_OBJECT_REST_BINDING_PROFILE_SHA256)
        }
        OxideProfileKind::Map => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_MAP_PROFILE_SHA256,
                "scoped Map Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped Map Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped Map Test262 capability profile requires its pinned manifest".to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped Map manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-map.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped Map manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped Map Test262 capability profile requires tests/test262-map.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_MAP_MANIFEST_SHA256,
                "scoped Map Test262 manifest",
            )?;
            Ok(TEST262_MAP_PROFILE_SHA256)
        }
        OxideProfileKind::Set => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_SET_PROFILE_SHA256,
                "scoped Set Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped Set Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped Set Test262 capability profile requires its pinned manifest".to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped Set manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-set.txt"),
            )
            .map_err(|error| format!("resolve pinned scoped Set manifest: {error}"))?;
            if actual != expected {
                return Err(format!(
                    "the scoped Set Test262 capability profile requires tests/test262-set.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_SET_MANIFEST_SHA256,
                "scoped Set Test262 manifest",
            )?;
            Ok(TEST262_SET_PROFILE_SHA256)
        }
        OxideProfileKind::SymbolProtocols => {
            verify_sha256(
                &options.oxide_profile,
                TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256,
                "scoped well-known Symbol protocol Test262 capability profile",
            )?;
            if options.all || !options.tests.is_empty() {
                return Err(
                    "the scoped well-known Symbol protocol Test262 capability profile requires its pinned manifest"
                        .to_owned(),
                );
            }
            let manifest = options.manifest.as_ref().ok_or_else(|| {
                "the scoped well-known Symbol protocol Test262 capability profile requires its pinned manifest"
                    .to_owned()
            })?;
            let actual = fs::canonicalize(manifest).map_err(|error| {
                format!(
                    "resolve scoped well-known Symbol protocol manifest {}: {error}",
                    manifest.display()
                )
            })?;
            let expected = fs::canonicalize(
                Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/test262-symbol-protocols.txt"),
            )
            .map_err(|error| {
                format!("resolve pinned scoped well-known Symbol protocol manifest: {error}")
            })?;
            if actual != expected {
                return Err(format!(
                    "the scoped well-known Symbol protocol Test262 capability profile requires tests/test262-symbol-protocols.txt, found {}",
                    manifest.display()
                ));
            }
            verify_sha256(
                manifest,
                TEST262_SYMBOL_PROTOCOLS_MANIFEST_SHA256,
                "scoped well-known Symbol protocol Test262 manifest",
            )?;
            Ok(TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256)
        }
    }
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
    use std::path::Path;

    use super::{
        Invocation, OxideProfileKind, TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256,
        TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256, TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256,
        TEST262_CATCH_BINDING_PROFILE_SHA256, TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256,
        TEST262_IDENTIFIER_REST_PROFILE_SHA256, TEST262_MAP_PROFILE_SHA256,
        TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256,
        TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256,
        TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256, TEST262_OBJECT_BINDING_PROFILE_SHA256,
        TEST262_OBJECT_REST_BINDING_PROFILE_SHA256, TEST262_SET_PROFILE_SHA256,
        TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256, default_worker_count, identify_oxide_profile,
        parse_args, verify_oxide_profile,
    };

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

    #[test]
    fn only_pinned_global_and_scoped_profiles_are_accepted() {
        assert_eq!(
            identify_oxide_profile(Path::new("compat/test262-oxide.conf")).unwrap(),
            OxideProfileKind::Global
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-binding-flat.conf")).unwrap(),
            OxideProfileKind::ArrayBindingFlat
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-binding-nested.conf")).unwrap(),
            OxideProfileKind::ArrayBindingNested
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-array-assignment-flat.conf")).unwrap(),
            OxideProfileKind::ArrayAssignmentFlat
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-catch-binding.conf")).unwrap(),
            OxideProfileKind::CatchBinding
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-identifier-defaults.conf")).unwrap(),
            OxideProfileKind::IdentifierDefaults
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-identifier-rest.conf")).unwrap(),
            OxideProfileKind::IdentifierRest
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-assignment-flat.conf")).unwrap(),
            OxideProfileKind::ObjectAssignmentFlat
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-assignment-nested.conf"))
                .unwrap(),
            OxideProfileKind::ObjectAssignmentNested
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-assignment-rest.conf")).unwrap(),
            OxideProfileKind::ObjectAssignmentRest
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-binding.conf")).unwrap(),
            OxideProfileKind::ObjectBinding
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-object-rest-binding.conf")).unwrap(),
            OxideProfileKind::ObjectRestBinding
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-map.conf")).unwrap(),
            OxideProfileKind::Map
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-set.conf")).unwrap(),
            OxideProfileKind::Set
        );
        assert_eq!(
            identify_oxide_profile(Path::new("tests/test262-symbol-protocols.conf")).unwrap(),
            OxideProfileKind::SymbolProtocols
        );

        let error = identify_oxide_profile(Path::new("Cargo.toml")).unwrap_err();
        assert!(error.contains("unsupported Test262 capability profile"));
    }

    #[test]
    fn scoped_flat_array_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-binding-flat.conf",
            "--manifest",
            "tests/test262-array-binding-flat.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_BINDING_FLAT_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/ary-name-iter-val.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-binding-flat.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_nested_array_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-binding-nested.conf",
            "--manifest",
            "tests/test262-array-binding-nested.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_BINDING_NESTED_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/ary-ptrn-elem-ary-elem-iter.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-binding-nested.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_flat_array_assignment_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-array-assignment-flat.conf",
            "--manifest",
            "tests/test262-array-assignment-flat.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_ARRAY_ASSIGNMENT_FLAT_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/expressions/assignment/dstr/array-empty-val-array.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-array-assignment-flat.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_catch_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-catch-binding.conf",
            "--manifest",
            "tests/test262-catch-binding.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_CATCH_BINDING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/try/dstr/obj-ptrn-empty.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-catch-binding.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_identifier_rest_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-identifier-rest.conf",
            "--manifest",
            "tests/test262-identifier-rest.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_IDENTIFIER_REST_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/language/rest-parameters/rest-index.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-identifier-rest.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_identifier_defaults_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-identifier-defaults.conf",
            "--manifest",
            "tests/test262-identifier-defaults.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_IDENTIFIER_DEFAULTS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/function/dflt-params-ref-prior.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-identifier-defaults.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_object_assignment_profiles_are_bound_to_their_pinned_manifests() {
        for (cohort, expected_hash) in [
            ("flat", TEST262_OBJECT_ASSIGNMENT_FLAT_PROFILE_SHA256),
            ("nested", TEST262_OBJECT_ASSIGNMENT_NESTED_PROFILE_SHA256),
            ("rest", TEST262_OBJECT_ASSIGNMENT_REST_PROFILE_SHA256),
        ] {
            let profile = format!("tests/test262-object-assignment-{cohort}.conf");
            let manifest = format!("tests/test262-object-assignment-{cohort}.txt");
            let invocation = parse(&[
                "--suite",
                "suite",
                "--oxide-profile",
                &profile,
                "--manifest",
                &manifest,
                "--report",
                "report.tsv",
            ])
            .unwrap();
            let Invocation::Coordinator(options) = invocation else {
                panic!("coordinator arguments selected another invocation");
            };
            assert_eq!(verify_oxide_profile(&options).unwrap(), expected_hash);

            for selection in [
                ["--all", ""],
                [
                    "--test",
                    "test/language/expressions/assignment/dstr/obj-empty-obj.js",
                ],
                ["--manifest", "Cargo.toml"],
            ] {
                let mut arguments = vec!["--suite", "suite", "--oxide-profile", profile.as_str()];
                arguments.push(selection[0]);
                if !selection[1].is_empty() {
                    arguments.push(selection[1]);
                }
                arguments.extend(["--report", "report.tsv"]);
                let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                    panic!("coordinator arguments selected another invocation");
                };
                assert!(verify_oxide_profile(&options).is_err());
            }
        }
    }

    #[test]
    fn scoped_object_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-object-binding.conf",
            "--manifest",
            "tests/test262-object-binding.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_OBJECT_BINDING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/obj-ptrn-empty.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-object-binding.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_object_rest_binding_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-object-rest-binding.conf",
            "--manifest",
            "tests/test262-object-rest-binding.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_OBJECT_REST_BINDING_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            [
                "--test",
                "test/language/statements/variable/dstr/obj-ptrn-rest-getter.js",
            ],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-object-rest-binding.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_map_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-map.conf",
            "--manifest",
            "tests/test262-map.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_MAP_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Map/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-map.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_set_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-set.conf",
            "--manifest",
            "tests/test262-set.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_SET_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Set/length.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-set.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }

    #[test]
    fn scoped_symbol_protocol_profile_is_bound_to_its_pinned_manifest() {
        let invocation = parse(&[
            "--suite",
            "suite",
            "--oxide-profile",
            "tests/test262-symbol-protocols.conf",
            "--manifest",
            "tests/test262-symbol-protocols.txt",
            "--report",
            "report.tsv",
        ])
        .unwrap();
        let Invocation::Coordinator(options) = invocation else {
            panic!("coordinator arguments selected another invocation");
        };
        assert_eq!(
            verify_oxide_profile(&options).unwrap(),
            TEST262_SYMBOL_PROTOCOLS_PROFILE_SHA256
        );

        for selection in [
            ["--all", ""],
            ["--test", "test/built-ins/Symbol/iterator/prop-desc.js"],
            ["--manifest", "Cargo.toml"],
        ] {
            let mut arguments = vec![
                "--suite",
                "suite",
                "--oxide-profile",
                "tests/test262-symbol-protocols.conf",
            ];
            arguments.push(selection[0]);
            if !selection[1].is_empty() {
                arguments.push(selection[1]);
            }
            arguments.extend(["--report", "report.tsv"]);
            let Invocation::Coordinator(options) = parse(&arguments).unwrap() else {
                panic!("coordinator arguments selected another invocation");
            };
            assert!(verify_oxide_profile(&options).is_err());
        }
    }
}
