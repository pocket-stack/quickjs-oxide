use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use super::metadata::{Metadata, parse_metadata};

/// Host hooks which the concrete worker installs for every test process.
///
/// Requirement discovery remains conservative and independent of execution;
/// the coordinator subtracts this typed capability set only after the worker
/// implementation has actually published the corresponding hook.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) struct HostCapabilities {
    pub agent: bool,
    pub can_block_false: bool,
    pub create_realm: bool,
    pub detach_array_buffer: bool,
    pub eval_script: bool,
    pub gc: bool,
    pub global: bool,
    pub is_html_dda: bool,
}

impl HostCapabilities {
    pub(super) fn retain_missing(self, capabilities: &mut Vec<String>) {
        capabilities.retain(|capability| match capability.as_str() {
            "agent" => !self.agent,
            "can-block:false" => !self.can_block_false,
            "create-realm" => !self.create_realm,
            "detach-array-buffer" => !self.detach_array_buffer,
            "eval-script" => !self.eval_script,
            "gc" => !self.gc,
            "global" => !self.global,
            "is-html-dda" => !self.is_html_dda,
            _ => true,
        });
    }
}

#[derive(Clone, Copy)]
struct NegativeMetadataContract {
    phase: &'static str,
    error_type: &'static str,
}

#[derive(Clone, Copy)]
struct ModuleMetadataContract {
    includes: &'static [&'static str],
    flags: &'static [&'static str],
    features: &'static [&'static str],
    negative: Option<NegativeMetadataContract>,
}

struct DependencyFreeModuleAdmission {
    path: &'static str,
    source_sha256: &'static str,
    metadata: ModuleMetadataContract,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactModuleTest {
    DependencyFree,
    FixtureGraph,
}

#[derive(Clone, Copy)]
struct ModuleRequestAdmission {
    specifier: &'static str,
    normalized_path: &'static str,
}

#[derive(Clone, Copy)]
struct ModuleGraphFileAdmission {
    path: &'static str,
    source_sha256: &'static str,
    metadata: ModuleMetadataContract,
    requests: &'static [ModuleRequestAdmission],
}

#[derive(Clone, Copy)]
struct FixtureGraphModuleAdmission {
    root_path: &'static str,
    files: &'static [ModuleGraphFileAdmission],
}

const MODULE_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: None,
};

const MODULE_FN_GLOBAL_OBJECT_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &["fnGlobalObject.js"],
    flags: &["module"],
    features: &[],
    negative: None,
};

const MODULE_PARSE_SYNTAX_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: Some(NegativeMetadataContract {
        phase: "parse",
        error_type: "SyntaxError",
    }),
};

const MODULE_RUNTIME_TYPE_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: Some(NegativeMetadataContract {
        phase: "runtime",
        error_type: "TypeError",
    }),
};

const MODULE_RESOLUTION_SYNTAX_ERROR_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &["module"],
    features: &[],
    negative: Some(NegativeMetadataContract {
        phase: "resolution",
        error_type: "SyntaxError",
    }),
};

const MODULE_FIXTURE_METADATA: ModuleMetadataContract = ModuleMetadataContract {
    includes: &[],
    flags: &[],
    features: &[],
    negative: None,
};

/// Source- and metadata-authenticated dependency-free module roots admitted by
/// the first static-module Test262 milestone. This is deliberately not a
/// general module capability switch: every other module retains the
/// `unsupported-module` selection result.
const DEPENDENCY_FREE_MODULE_ADMISSIONS: [DependencyFreeModuleAdmission; 13] = [
    DependencyFreeModuleAdmission {
        path: "test/language/comments/hashbang/module.js",
        source_sha256: "5fe73a40369e7cbd61f4061b027c9b508d6f1752fc83b29a4f1e4af7e8471926",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module", "raw"],
            features: &["hashbang"],
            negative: None,
        },
    },
    DependencyFreeModuleAdmission {
        path: "test/language/eval-code/direct/export.js",
        source_sha256: "648a257196bc895409842b12191cc0a8d9e10d28e66886afb89059412761caca",
        metadata: MODULE_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/eval-code/direct/import.js",
        source_sha256: "28c29caa8c8649579790526b511323df04837efad886d2f9d0ea75140dc5fa89",
        metadata: MODULE_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/comment-single-line-html-open.js",
        source_sha256: "789641728f7d8496801f145059d329c8b3c9cc1d2901ecbe893ff70e5e426d11",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-dup-export-id.js",
        source_sha256: "c113c88cba6a99ba5ef7cf1c4c503c60d374aad2f6de2a3a112d6d1be937d91a",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/early-strict-mode.js",
        source_sha256: "a72ab52b0625b5becdc0a4f7e4945848582dd797493b4590b7a2ea25b63dd4e4",
        metadata: MODULE_PARSE_SYNTAX_ERROR_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/eval-self-abrupt.js",
        source_sha256: "a593ac28375f793312830e40cdda392054352f4fa692446de8ce2896c4518aa7",
        metadata: ModuleMetadataContract {
            includes: &[],
            flags: &["module"],
            features: &[],
            negative: Some(NegativeMetadataContract {
                phase: "runtime",
                error_type: "Test262Error",
            }),
        },
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/eval-this.js",
        source_sha256: "044874d01e501861c9c1d451ddd67e1c224a768045be75c5c49e0eb182d998c2",
        metadata: MODULE_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-const.js",
        source_sha256: "a36eaed3d56e39769c951b6ca041e22a9cbd1aea1e5dc3651f416992815dca81",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-fun.js",
        source_sha256: "92b10ca365a70fb2a9b4ba5e98add3e14912e6dd14c9271ee7a88f157945f784",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-let.js",
        source_sha256: "fd0c09f7adc72c46b66fa440450bbaa5db68173c09e8b6f54af58978c27f99ac",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/instn-local-bndng-var.js",
        source_sha256: "8f9e41100266ea157c23977f9cb6646ec9b8c826362d2c8eaa44b5b1c2ba232a",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
    },
    DependencyFreeModuleAdmission {
        path: "test/language/module-code/parse-export-empty.js",
        source_sha256: "eccb82249ee01600351841616110a7e8182e7056561f6eb9e44120b7aaf73cd8",
        metadata: MODULE_METADATA,
    },
];

const EVAL_GTBNDNG_INDIRECT_UPDATE_FILES: [ModuleGraphFileAdmission; 2] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update.js",
        source_sha256: "2e382b6cef4a65f3c1b58ed7a21f9311b2627e7980b410805d1018b714d4b5b6",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./eval-gtbndng-indirect-update_FIXTURE.js",
            normalized_path: "test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-gtbndng-indirect-update_FIXTURE.js",
        source_sha256: "86f9d73e4f721d046412952d46a9fdeb2864fb6bdc2917d995170945d6f7800b",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

const EVAL_REQUESTED_ABRUPT_FILES: [ModuleGraphFileAdmission; 3] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-abrupt.js",
        source_sha256: "96266e78b158e46ce04ab22c987e62a4ff5c6b9484ebb8adacd993f44e4e8f29",
        metadata: MODULE_RUNTIME_TYPE_ERROR_METADATA,
        requests: &[
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-abrupt-err-type_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-abrupt-err-type_FIXTURE.js",
            },
            ModuleRequestAdmission {
                specifier: "./eval-rqstd-abrupt-err-uri_FIXTURE.js",
                normalized_path: "test/language/module-code/eval-rqstd-abrupt-err-uri_FIXTURE.js",
            },
        ],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-abrupt-err-type_FIXTURE.js",
        source_sha256: "ce3ebfa86081c793bf36a681e6f1e4faca99e529b338bfbfc433b550e1bf27e8",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/eval-rqstd-abrupt-err-uri_FIXTURE.js",
        source_sha256: "e6bbf1d0467c9a361289d3d6a40ae8479bff3c7d928b10140c2171b309207572",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

const INSTN_RESOLVE_EMPTY_IMPORT_FILES: [ModuleGraphFileAdmission; 2] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-resolve-empty-import.js",
        source_sha256: "88161e79a99ef0372dddb122e6dc2e545961bf0d4775f53ba48531b3fcc3fadb",
        metadata: MODULE_RESOLUTION_SYNTAX_ERROR_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-resolve-empty-import_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-resolve-empty-import_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-resolve-empty-import_FIXTURE.js",
        source_sha256: "d019396c51ec65b57af8edc64bcc7b969df709c1f0a11a6b5220bc5f09545e80",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

const INSTN_SAME_GLOBAL_FILES: [ModuleGraphFileAdmission; 2] = [
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-same-global.js",
        source_sha256: "564f38753491b84941656868c73ce342c2111fc9b29ed7b681ee9732f4e5cbce",
        metadata: MODULE_FN_GLOBAL_OBJECT_METADATA,
        requests: &[ModuleRequestAdmission {
            specifier: "./instn-same-global-set_FIXTURE.js",
            normalized_path: "test/language/module-code/instn-same-global-set_FIXTURE.js",
        }],
    },
    ModuleGraphFileAdmission {
        path: "test/language/module-code/instn-same-global-set_FIXTURE.js",
        source_sha256: "ac117f0e7632295f0e7b67bace1d65b72e2f4d9a3dd2b66643b3b27d24f48f8f",
        metadata: MODULE_FIXTURE_METADATA,
        requests: &[],
    },
];

/// Source-, metadata-, edge-, and recursive-closure-authenticated module
/// graphs admitted by the loader/linker Test262 milestone. The four roots are
/// intentionally independent, so their nine total source files form the
/// smallest useful direct-import cohort and no unrelated fixture can be
/// reached through the worker loader.
const FIXTURE_GRAPH_MODULE_ADMISSIONS: [FixtureGraphModuleAdmission; 4] = [
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/eval-gtbndng-indirect-update.js",
        files: &EVAL_GTBNDNG_INDIRECT_UPDATE_FILES,
    },
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/eval-rqstd-abrupt.js",
        files: &EVAL_REQUESTED_ABRUPT_FILES,
    },
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/instn-resolve-empty-import.js",
        files: &INSTN_RESOLVE_EMPTY_IMPORT_FILES,
    },
    FixtureGraphModuleAdmission {
        root_path: "test/language/module-code/instn-same-global.js",
        files: &INSTN_SAME_GLOBAL_FILES,
    },
];

/// Admit only one of the pinned, dependency-free module roots above.
///
/// The coordinator and worker both call this function. An exact-path source or
/// metadata change is an audit failure, while an unlisted module is simply not
/// admitted and remains classified as unsupported by the coordinator.
pub(super) fn is_exact_dependency_free_module_test(
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = DEPENDENCY_FREE_MODULE_ADMISSIONS
        .iter()
        .find(|admission| path == Path::new(admission.path))
    else {
        return Ok(false);
    };
    let actual_sha256 = source_sha256(source)?;
    authenticate_dependency_free_module_test(path, &actual_sha256, metadata, admission)
}

fn authenticate_dependency_free_module_test(
    path: &Path,
    actual_sha256: &str,
    metadata: &Metadata,
    admission: &DependencyFreeModuleAdmission,
) -> Result<bool, String> {
    if path != Path::new(admission.path) {
        return Ok(false);
    }
    if actual_sha256 != admission.source_sha256 {
        return Err(format!(
            "dependency-free module source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            admission.path, admission.source_sha256
        ));
    }
    if !module_metadata_matches(metadata, admission.metadata) {
        return Err(format!(
            "dependency-free module metadata shape drifted for {}",
            admission.path
        ));
    }
    Ok(true)
}

/// Authenticate one of the two deliberately narrow static-module execution
/// frontiers. An unlisted module remains unadmitted without touching any
/// fixture file; an exact graph root authenticates its complete recursive
/// closure before either the coordinator or worker can remove `module` from
/// the missing-host set.
pub(super) fn exact_module_test(
    suite: &Path,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<Option<ExactModuleTest>, String> {
    if is_exact_dependency_free_module_test(path, source, metadata)? {
        return Ok(Some(ExactModuleTest::DependencyFree));
    }
    if is_exact_fixture_graph_module_test(suite, path, source, metadata)? {
        return Ok(Some(ExactModuleTest::FixtureGraph));
    }
    Ok(None)
}

fn is_exact_fixture_graph_module_test(
    suite: &Path,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = fixture_graph_admission(path) else {
        return Ok(false);
    };
    let root = module_graph_file(admission, admission.root_path).ok_or_else(|| {
        format!(
            "fixture graph admission has no root file: {}",
            admission.root_path
        )
    })?;
    authenticate_module_graph_file(path, source, metadata, root)?;
    authenticate_fixture_graph_closure(admission, |relative| {
        read_regular_module_source(suite, relative)
    })?;
    Ok(true)
}

fn fixture_graph_admission(root_path: &Path) -> Option<&'static FixtureGraphModuleAdmission> {
    FIXTURE_GRAPH_MODULE_ADMISSIONS
        .iter()
        .find(|admission| root_path == Path::new(admission.root_path))
}

fn module_graph_file<'a>(
    admission: &'a FixtureGraphModuleAdmission,
    path: &str,
) -> Option<&'a ModuleGraphFileAdmission> {
    admission.files.iter().find(|file| file.path == path)
}

fn authenticate_fixture_graph_closure(
    admission: &FixtureGraphModuleAdmission,
    mut read_source: impl FnMut(&str) -> Result<String, String>,
) -> Result<(), String> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![admission.root_path];
    while let Some(path) = pending.pop() {
        if !visited.insert(path) {
            continue;
        }
        let file = module_graph_file(admission, path).ok_or_else(|| {
            format!(
                "fixture graph edge escaped the authenticated closure for {}: {path}",
                admission.root_path
            )
        })?;
        let source = read_source(path)?;
        let metadata = parse_metadata(&source)
            .map_err(|error| format!("parse authenticated module metadata for {path}: {error}"))?;
        authenticate_module_graph_file(Path::new(path), &source, &metadata, file)?;
        for request in file.requests.iter().rev() {
            if module_graph_file(admission, request.normalized_path).is_none() {
                return Err(format!(
                    "fixture graph request escaped the authenticated closure for {}: {} -> {}",
                    admission.root_path, request.specifier, request.normalized_path
                ));
            }
            pending.push(request.normalized_path);
        }
    }
    if visited.len() != admission.files.len() {
        let unreachable = admission
            .files
            .iter()
            .filter(|file| !visited.contains(file.path))
            .map(|file| file.path)
            .collect::<Vec<_>>()
            .join(", ");
        return Err(format!(
            "fixture graph admission contains files outside the recursive closure for {}: {unreachable}",
            admission.root_path
        ));
    }
    Ok(())
}

fn authenticate_module_graph_file(
    path: &Path,
    source: &str,
    metadata: &Metadata,
    file: &ModuleGraphFileAdmission,
) -> Result<(), String> {
    let actual_sha256 = source_sha256(source)?;
    authenticate_module_graph_file_digest(path, &actual_sha256, metadata, file)
}

fn authenticate_module_graph_file_digest(
    path: &Path,
    actual_sha256: &str,
    metadata: &Metadata,
    file: &ModuleGraphFileAdmission,
) -> Result<(), String> {
    if path != Path::new(file.path) {
        return Err(format!(
            "fixture graph file path drifted: expected {}, found {}",
            file.path,
            path.display()
        ));
    }
    if actual_sha256 != file.source_sha256 {
        return Err(format!(
            "fixture graph module source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            file.path, file.source_sha256
        ));
    }
    if !module_metadata_matches(metadata, file.metadata) {
        return Err(format!(
            "fixture graph module metadata shape drifted for {}",
            file.path
        ));
    }
    Ok(())
}

fn read_regular_module_source(suite: &Path, relative: &str) -> Result<String, String> {
    let path = suite.join(relative);
    let file_type = fs::symlink_metadata(&path)
        .map_err(|error| format!("stat authenticated module {}: {error}", path.display()))?
        .file_type();
    if !file_type.is_file() || file_type.is_symlink() {
        return Err(format!(
            "authenticated module is not a regular non-symlink file: {}",
            path.display()
        ));
    }
    fs::read_to_string(&path)
        .map_err(|error| format!("read authenticated module {}: {error}", path.display()))
}

/// Normalize only a source-authenticated request edge from one admitted graph.
/// This deliberately refuses generic path joining, bare names, and requests
/// from an unlisted graph member.
pub(super) fn normalize_exact_module_request(
    root_path: &Path,
    base_name: &str,
    specifier: &str,
) -> Result<String, String> {
    let admission = fixture_graph_admission(root_path).ok_or_else(|| {
        format!(
            "module loader rejected unaudited root: {}",
            root_path.display()
        )
    })?;
    let base = module_graph_file(admission, base_name)
        .ok_or_else(|| format!("module loader rejected unaudited base module: {base_name}"))?;
    let request = base
        .requests
        .iter()
        .find(|request| request.specifier == specifier)
        .ok_or_else(|| {
            format!("module loader rejected unaudited request from {base_name}: {specifier}")
        })?;
    Ok(request.normalized_path.to_owned())
}

/// Load one exact fixture from a previously authenticated graph. The source
/// and metadata are checked again at the loader boundary to close the gap
/// between coordinator admission and worker resolution.
pub(super) fn load_exact_module_fixture(
    suite: &Path,
    root_path: &Path,
    normalized_name: &str,
) -> Result<String, String> {
    let admission = fixture_graph_admission(root_path).ok_or_else(|| {
        format!(
            "module loader rejected unaudited root: {}",
            root_path.display()
        )
    })?;
    let file = module_graph_file(admission, normalized_name)
        .filter(|file| file.path != admission.root_path)
        .ok_or_else(|| format!("module loader rejected unaudited fixture: {normalized_name}"))?;
    let source = read_regular_module_source(suite, file.path)?;
    let metadata = parse_metadata(&source).map_err(|error| {
        format!(
            "parse authenticated module metadata for {}: {error}",
            file.path
        )
    })?;
    authenticate_module_graph_file(Path::new(file.path), &source, &metadata, file)?;
    Ok(source)
}

fn module_metadata_matches(metadata: &Metadata, contract: ModuleMetadataContract) -> bool {
    metadata
        .includes
        .iter()
        .map(String::as_str)
        .eq(contract.includes.iter().copied())
        && metadata
            .flags
            .iter()
            .map(String::as_str)
            .eq(contract.flags.iter().copied())
        && metadata
            .features
            .iter()
            .map(String::as_str)
            .eq(contract.features.iter().copied())
        && match (&metadata.negative, contract.negative) {
            (None, None) => true,
            (Some(actual), Some(expected)) => {
                actual.phase.as_deref() == Some(expected.phase)
                    && actual.error_type.as_deref() == Some(expected.error_type)
            }
            _ => false,
        }
}

struct AgentHostAdmission {
    path: &'static str,
    source_sha256: &'static str,
    features: &'static [&'static str],
    cohort: &'static str,
}

const AGENT_HOST_ADMISSIONS: [AgentHostAdmission; 59] = [
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/bigint/notify-all-on-loc.js",
        source_sha256: "442a9e3af420e81107defd515e5bfe539a7a5a133e61797fad9a640e93439b3d",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/count-defaults-to-infinity-missing.js",
        source_sha256: "5bc3aee123dafa5dd70ff92a8c73385880a26118e5d110f79d809707225f6a6b",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/count-defaults-to-infinity-undefined.js",
        source_sha256: "57afbd3a2f85800ee919c038809d605511102f6ee99504f5b833f97cf75c7efb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/negative-count.js",
        source_sha256: "fe734b6972c67082995e6140e781449198828b79d0ddb24a51a204b5afd6390e",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-all-on-loc.js",
        source_sha256: "f2f60a1f70c6f6c47d28ad602418889b81586b4d4c1f06e8c09e063c4e510844",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-all.js",
        source_sha256: "0a68a903a51def1d8869c2c93fb7e3640bf6389f148482e5a4cb8bc42e7926d9",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-in-order-one-time.js",
        source_sha256: "9cdc624fc8932d14b137b5daf34bf27efedce16fb53a0f4ef94fcdd0f26af989",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-in-order.js",
        source_sha256: "9cdc624fc8932d14b137b5daf34bf27efedce16fb53a0f4ef94fcdd0f26af989",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-nan.js",
        source_sha256: "9d022e8e59572cbcd5dc672b3249b9c67407e9007c7e56f4156b4aea2e4857c5",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-one.js",
        source_sha256: "3364d4844004ba73efe5036da4fd0cafa1bab5218885946e5b704bd06082dd61",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-renotify-noop.js",
        source_sha256: "e69b68f7240ff876c28b5ed4130a54830eaf6e02e665db6c87e0b7c1a1cbafdb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-two.js",
        source_sha256: "03309fe924420caf6fc40817dce1095895470233268502687a2168a27769d9ab",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-with-no-agents-waiting.js",
        source_sha256: "c4f49f9a52daab30e695cea6d8fe400a7ebd38dc41daef6843b763d1006ba718",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-with-no-matching-agents-waiting.js",
        source_sha256: "85e1c3a5897d64f38b6f271b714cb025ed237267d47ebf9ab332a19b03e1a382",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/notify-zero.js",
        source_sha256: "57018cbe3c726eeecbbce24f9b40a3ab5f845372030731b0255adbbbda27c80f",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/notify/undefined-index-defaults-to-zero.js",
        source_sha256: "9235c0501b3f81cb4b7079ee73e52de6f39f987467d90b79b9c47bb44baf6550",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/false-for-timeout-agent.js",
        source_sha256: "30818849f231757c0fce413f31fa235c63236f9268eab982ce58078d427fade1",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/nan-for-timeout.js",
        source_sha256: "7109bf013ce44e8e36d88ce1eda639b0f844fee89c04f31cf8356475b6b89021",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/negative-timeout-agent.js",
        source_sha256: "098159fb9b6c3619ee5eaf445333bf5b20088fc46e9227c8de383bfd3550b014",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-no-operation.js",
        source_sha256: "9002df4475d2b76914f49e2c431e77a3396a1cc114f0715078fbdf8eb11346ee",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-add.js",
        source_sha256: "1a661c6660fbb3a33fbc097ff1af549ec5995e69758871b4851f88d36531a676",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-and.js",
        source_sha256: "8e097032ed544fcbf3c0290d4324dfbb3fa782c8669b3299b74b580b4af9223c",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-compareExchange.js",
        source_sha256: "0900f28d7cedcd006904fea08be5953415c83af9ea579ac1c31b12efb7ae612a",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-exchange.js",
        source_sha256: "27e9693ceb73db3d177d57899cf5240251af31617cb31d7ca8c21aa3848130f3",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-or.js",
        source_sha256: "b31f24fa0de4383b7a85d504629a181cba7d8400707664fc084c90c7ca29d57c",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-store.js",
        source_sha256: "ace9fe8ca799b7c9898a263f10df37beaf2ca97cc1aaaed5382aaeedce275989",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-sub.js",
        source_sha256: "5a1a1b1eff5407f32f2195f4f5a45f610c51dfae67d7e4fcf5230d600957c546",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/no-spurious-wakeup-on-xor.js",
        source_sha256: "c4c1bf8012da172bdc5114e995869a5a82a2b01d88fd6a53a39d7fffc5445e3e",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/value-not-equal.js",
        source_sha256: "6ac2ae7a18c6081df18371c6dab12bb82430f37e92ec6a6c3ff9ff5ce59df700",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/waiterlist-block-indexedposition-wake.js",
        source_sha256: "b02f89aa4a6fc7cc8e6f63c7761b95a2cfe08bd2bdb2483e6f9c4c0462975e95",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/waiterlist-order-of-operations-is-fifo.js",
        source_sha256: "bfa8cc8764efee31ea7bda7f25755853e5fbf3b109ddc72650e65c53058b3f88",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/bigint/was-woken-before-timeout.js",
        source_sha256: "f7af53430000b4c57d0e50314cf9f1a5c68f3f9f40f9d0da26fdeb40651cd11e",
        features: &["Atomics", "BigInt", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/false-for-timeout-agent.js",
        source_sha256: "1f155c405b5b137c902e5e385a5a39a858444ad63941bdde5ca6762844e978a2",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/good-views.js",
        source_sha256: "7ab45f324e0f668a9d9f3df03c866b0ac32276eb1dfb649d1e5783a88f70bb21",
        features: &["Atomics"],
        cohort: "Test262 agent Stage A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/nan-for-timeout.js",
        source_sha256: "efaa0c6981a9a485a0dd40b145fd071f3667d4d7bef28a5c844a22f9bcd1c1d2",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/negative-timeout-agent.js",
        source_sha256: "8d2236937f9a3d792cfda706d7d7703642c21bbda26729ca29421d89cb3865eb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-no-operation.js",
        source_sha256: "7436557067aa3940e9882a53387257a60ef034d6173c10335ad8f5415d15ceb9",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-add.js",
        source_sha256: "672834f107ba1c574a19323aca5284b45dbf6db0384892e9388629127ca7015c",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-and.js",
        source_sha256: "f4b63fb173a054c591a38d36bdf2d74181c1596571b3f2857d501b4c3cda1469",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-compareExchange.js",
        source_sha256: "51129ba0e54af3cea300b23b85a631f2186dab1e264b7b05edb214e1d4048eb4",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-exchange.js",
        source_sha256: "03068ee53c5a70deb59271de3311190e9524c0dffa1a081a17593103a1e1c9c9",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-or.js",
        source_sha256: "6e15a69b550977979fe2bed9a60d667a0e194f2a7ade50bc92f1e82c2fe3a086",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-store.js",
        source_sha256: "f018200f54d42e169cda405f92f99f017abae44bb2aae18319633d857d3d7171",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-sub.js",
        source_sha256: "3f77bf071ef009ebb098d3a43e2670b847bea3954177df1751252dcd07f1c5e5",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/no-spurious-wakeup-on-xor.js",
        source_sha256: "be9af683186fd217591b733ca6cb685db3b091b28331cdfd609d0cb756fb9e04",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/null-for-timeout-agent.js",
        source_sha256: "407d2a0a8bf72382dfeb22b711cce26ea562a8b2da1c79e941c50315e78f7a30",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/object-for-timeout-agent.js",
        source_sha256: "c7ecd98803298b5fbc82f6f68d16bce6f3246800a9ef79b526ae55be06d41d0f",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/poisoned-object-for-timeout-throws-agent.js",
        source_sha256: "2780f367fba1a8090ac059185fc8dd3d7f92da10dea9261ff9eb00845ef3c266",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/symbol-for-index-throws-agent.js",
        source_sha256: "b255a1f336e1fa3de54eff1a885a5b8c52d1d307ca2d21e73e5a8c5cbb472c1f",
        features: &[
            "Atomics",
            "SharedArrayBuffer",
            "Symbol",
            "Symbol.toPrimitive",
            "TypedArray",
        ],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/symbol-for-timeout-throws-agent.js",
        source_sha256: "6d37e6f2f0db2518c31b41e08aa2479e07d425ed1510a5170a30277b8698c172",
        features: &[
            "Atomics",
            "SharedArrayBuffer",
            "Symbol",
            "Symbol.toPrimitive",
            "TypedArray",
        ],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/symbol-for-value-throws-agent.js",
        source_sha256: "7176e285cd33104da37b6cc70a2f5e83a9165da02092ad15062088ed7d83b5de",
        features: &[
            "Atomics",
            "SharedArrayBuffer",
            "Symbol",
            "Symbol.toPrimitive",
            "TypedArray",
        ],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/true-for-timeout-agent.js",
        source_sha256: "742792a79f511dd8581771d134c8355bd39d7eb90b70884e6ef5e3a810680cec",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent bounded wait cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/undefined-for-timeout.js",
        source_sha256: "0dd3f74bb8ae3b06012e1b1b047fcd1e499943e63829214da4c16fc49df5d589",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/undefined-index-defaults-to-zero.js",
        source_sha256: "c0b85d26b9e50ee0c309d55d90f4a30a06ebd7139a57197e55b7c0ecec9a95fb",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/value-not-equal.js",
        source_sha256: "24a38831488f8794736387ab7cafc0528fc9fd9f2276b49a5e04d77f5ef0e4a7",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/wait-index-value-not-equal.js",
        source_sha256: "0c2103b7079f54cfbe0c57ccbaef6644bab370409dad2f32e6b0c3e9577dfa08",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent broadcast cohort A",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/waiterlist-block-indexedposition-wake.js",
        source_sha256: "87e398dbfc8e4022331380d67325a2da98dea734dfed11158ab0e34e0f417ab3",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/waiterlist-order-of-operations-is-fifo.js",
        source_sha256: "6503e1b20e4c55d661c165020ee7b83a3cd35326fed0211358df41b67b2adda1",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent FIFO wake-order cohort",
    },
    AgentHostAdmission {
        path: "test/built-ins/Atomics/wait/was-woken-before-timeout.js",
        source_sha256: "d97f474e3fe55e36d6475ef88653ce4e4b203e638d2f28f5547f9c2b30784d2a",
        features: &["Atomics", "SharedArrayBuffer", "TypedArray"],
        cohort: "Test262 agent wake/count/location cohort",
    },
];

/// Admit only source- and metadata-audited `$262.agent` tests.
///
/// The exact path check prevents a profile entry from broadening the host
/// surface. The source hash and complete metadata shape prevent an in-place
/// Test262 update from silently inheriting an earlier admission.
pub(super) fn is_exact_agent_host_test(
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = AGENT_HOST_ADMISSIONS
        .iter()
        .find(|admission| path == Path::new(admission.path))
    else {
        return Ok(false);
    };
    let actual_sha256 = source_sha256(source)?;
    if actual_sha256 != admission.source_sha256 {
        return Err(format!(
            "{} source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            admission.cohort, admission.path, admission.source_sha256
        ));
    }
    if !agent_host_metadata_matches(metadata, admission) {
        return Err(format!(
            "{} metadata shape drifted for {}",
            admission.cohort, admission.path
        ));
    }
    Ok(true)
}

fn agent_host_metadata_matches(metadata: &Metadata, admission: &AgentHostAdmission) -> bool {
    metadata.includes == ["atomicsHelper.js"]
        && metadata.flags.is_empty()
        && metadata
            .features
            .iter()
            .map(String::as_str)
            .eq(admission.features.iter().copied())
        && metadata.negative.is_none()
}

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

/// Return pinned source-audited feature requirements omitted by Test262
/// metadata or deliberately staged behind an explicit host-admission tag.
///
/// `createRealm` and `evalScript` have no standard Test262 feature tag, so
/// synthetic tags keep newly implemented worker hooks from silently changing
/// the global conformance vector before their admission gates. The
/// SpiderMonkey Atomics staging tests additionally omit feature metadata. The
/// cross-compartment test constructs a foreign `SharedArrayBuffer`, while the
/// detached-buffer test exercises non-shared `Atomics` operations. Keep these
/// path overrides exact and fail closed if their audited source changes.
pub(super) fn supplemental_feature_hints(path: &Path, source: &str) -> Result<Vec<String>, String> {
    const ATOMICS_CROSS_REALM: &str = "test/staging/sm/Atomics/cross-compartment.js";
    const ATOMICS_CROSS_REALM_SHA256: &str =
        "8b6770fe9be68c0deed01fdc484da4b80737f7068ef1c823dae3ea30de885f56";
    const ATOMICS_DETACHED_BUFFERS: &str = "test/staging/sm/Atomics/detached-buffers.js";
    const ATOMICS_DETACHED_BUFFERS_SHA256: &str =
        "c7813d0121f03dc3c97e088afccca800220e494d27ae0b75d89464f41598ee12";

    let tokens = source_tokens(source, false);
    let members = member_names(&tokens);
    let mut hints = BTreeSet::new();
    if members.contains(&"createRealm") {
        hints.insert("host-create-realm-required".to_owned());
    }
    if members.contains(&"evalScript") {
        hints.insert("host-eval-script-required".to_owned());
    }

    insert_atomics_cross_realm_feature_hints(
        &mut hints,
        path,
        source,
        &tokens,
        ATOMICS_CROSS_REALM,
        ATOMICS_CROSS_REALM_SHA256,
    )?;

    insert_exact_source_feature_hint(
        &mut hints,
        path,
        source,
        ATOMICS_DETACHED_BUFFERS,
        ATOMICS_DETACHED_BUFFERS_SHA256,
        "Atomics",
    )?;

    Ok(hints.into_iter().collect())
}

fn insert_atomics_cross_realm_feature_hints(
    hints: &mut BTreeSet<String>,
    path: &Path,
    source: &str,
    tokens: &[SourceToken<'_>],
    expected_path: &str,
    expected_sha256: &str,
) -> Result<(), String> {
    if !verify_exact_source_sha256(path, source, expected_path, expected_sha256)? {
        return Ok(());
    }
    let has_identifier = |wanted| {
        tokens
            .iter()
            .any(|token| matches!(token, SourceToken::Identifier(name) if *name == wanted))
    };
    if !hints.contains("host-create-realm-required")
        || !has_identifier("Atomics")
        || !has_identifier("SharedArrayBuffer")
    {
        return Err(format!(
            "supplemental feature source shape drifted for {expected_path}"
        ));
    }
    hints.insert("Atomics".to_owned());
    hints.insert("SharedArrayBuffer".to_owned());
    Ok(())
}

fn insert_exact_source_feature_hint(
    hints: &mut BTreeSet<String>,
    path: &Path,
    source: &str,
    expected_path: &str,
    expected_sha256: &str,
    feature: &str,
) -> Result<(), String> {
    if !verify_exact_source_sha256(path, source, expected_path, expected_sha256)? {
        return Ok(());
    }
    hints.insert(feature.to_owned());
    Ok(())
}

fn verify_exact_source_sha256(
    path: &Path,
    source: &str,
    expected_path: &str,
    expected_sha256: &str,
) -> Result<bool, String> {
    if path != Path::new(expected_path) {
        return Ok(false);
    }
    let actual_sha256 = source_sha256(source)?;
    if actual_sha256 != expected_sha256 {
        return Err(format!(
            "supplemental feature audit drifted for {expected_path}: expected source SHA-256 \
             {expected_sha256}, found {actual_sha256}"
        ));
    }
    Ok(true)
}

fn source_sha256(source: &str) -> Result<String, String> {
    let commands: [(&str, &[&str]); 2] = [("sha256sum", &[]), ("shasum", &["-a", "256"])];
    let mut unavailable = Vec::new();
    for (program, arguments) in commands {
        let mut child = match Command::new(program)
            .args(arguments)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                unavailable.push(program);
                continue;
            }
            Err(error) => return Err(format!("hash Test262 source with {program}: {error}")),
        };
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| format!("hash Test262 source with {program}: stdin unavailable"))?;
            stdin
                .write_all(source.as_bytes())
                .map_err(|error| format!("hash Test262 source with {program}: {error}"))?;
        }
        let output = child
            .wait_with_output()
            .map_err(|error| format!("hash Test262 source with {program}: {error}"))?;
        if !output.status.success() {
            return Err(format!(
                "hash Test262 source with {program}: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        return Ok(String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .to_owned());
    }
    Err(format!(
        "cannot hash Test262 source: commands are unavailable: {}",
        unavailable.join(", ")
    ))
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

    use super::{
        AGENT_HOST_ADMISSIONS, DEPENDENCY_FREE_MODULE_ADMISSIONS, ExactModuleTest,
        FIXTURE_GRAPH_MODULE_ADMISSIONS, FixtureGraphModuleAdmission, HostCapabilities,
        MODULE_FIXTURE_METADATA, MODULE_METADATA, ModuleGraphFileAdmission, ModuleMetadataContract,
        ModuleRequestAdmission, agent_host_metadata_matches,
        authenticate_dependency_free_module_test, authenticate_fixture_graph_closure,
        authenticate_module_graph_file_digest, exact_module_test,
        generator_destructuring_source_needs_async_guard, insert_atomics_cross_realm_feature_hints,
        insert_exact_source_feature_hint, is_exact_agent_host_test,
        is_exact_dependency_free_module_test, missing_host_capability_hints,
        module_metadata_matches, normalize_exact_module_request, source_sha256, source_tokens,
        supplemental_feature_hints,
    };
    use crate::metadata::{Metadata, NegativeExpectation};

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

    fn module_metadata(contract: ModuleMetadataContract) -> Metadata {
        Metadata {
            includes: contract
                .includes
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            flags: contract
                .flags
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            features: contract
                .features
                .iter()
                .map(|value| (*value).to_owned())
                .collect(),
            negative: contract.negative.map(|negative| NegativeExpectation {
                phase: Some(negative.phase.to_owned()),
                error_type: Some(negative.error_type.to_owned()),
            }),
        }
    }

    #[test]
    fn dependency_free_module_admission_is_exact_and_complete() {
        assert_eq!(DEPENDENCY_FREE_MODULE_ADMISSIONS.len(), 13);
        assert!(
            DEPENDENCY_FREE_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );

        for admission in &DEPENDENCY_FREE_MODULE_ADMISSIONS {
            let metadata = module_metadata(admission.metadata);
            assert!(metadata.is_module());
            assert!(module_metadata_matches(&metadata, admission.metadata));
            assert_eq!(
                authenticate_dependency_free_module_test(
                    Path::new(admission.path),
                    admission.source_sha256,
                    &metadata,
                    admission,
                ),
                Ok(true),
                "{}",
                admission.path
            );
        }
    }

    #[test]
    fn dependency_free_module_admission_rejects_source_and_metadata_drift() {
        let admission = &DEPENDENCY_FREE_MODULE_ADMISSIONS[0];
        let exact = module_metadata(admission.metadata);
        let source_drift = authenticate_dependency_free_module_test(
            Path::new(admission.path),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &exact,
            admission,
        )
        .unwrap_err();
        assert!(source_drift.contains("source drifted"));
        assert!(source_drift.contains(admission.source_sha256));

        let mut metadata_drift = exact;
        metadata_drift.flags.insert("async".to_owned());
        let metadata_drift = authenticate_dependency_free_module_test(
            Path::new(admission.path),
            admission.source_sha256,
            &metadata_drift,
            admission,
        )
        .unwrap_err();
        assert!(metadata_drift.contains("metadata shape drifted"));
    }

    #[test]
    fn fixture_graph_module_admission_is_exact_sorted_and_closed() {
        assert_eq!(FIXTURE_GRAPH_MODULE_ADMISSIONS.len(), 4);
        assert!(
            FIXTURE_GRAPH_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].root_path < pair[1].root_path)
        );
        assert_eq!(
            FIXTURE_GRAPH_MODULE_ADMISSIONS
                .iter()
                .map(|admission| admission.files.len())
                .sum::<usize>(),
            9
        );

        let mut all_paths = BTreeSet::new();
        for admission in &FIXTURE_GRAPH_MODULE_ADMISSIONS {
            assert_eq!(admission.files[0].path, admission.root_path);
            assert!(module_metadata(admission.files[0].metadata).is_module());
            let mut reachable = BTreeSet::new();
            let mut pending = vec![admission.root_path];
            while let Some(path) = pending.pop() {
                assert!(reachable.insert(path), "duplicate or cyclic edge at {path}");
                let file = admission
                    .files
                    .iter()
                    .find(|file| file.path == path)
                    .expect("every request target stays in its admission");
                assert!(all_paths.insert(file.path), "duplicate file {}", file.path);
                for request in file.requests.iter().rev() {
                    assert!(request.specifier.starts_with("./"));
                    pending.push(request.normalized_path);
                }
            }
            assert_eq!(reachable.len(), admission.files.len());
            assert!(
                admission.files[1..]
                    .iter()
                    .all(|file| module_metadata_matches(&Metadata::default(), file.metadata))
            );
        }
    }

    #[test]
    fn fixture_graph_file_authentication_rejects_source_metadata_and_path_drift() {
        let file = &FIXTURE_GRAPH_MODULE_ADMISSIONS[0].files[1];
        let exact = module_metadata(file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(file.path),
                file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );

        let source_drift = authenticate_module_graph_file_digest(
            Path::new(file.path),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &exact,
            file,
        )
        .unwrap_err();
        assert!(source_drift.contains("source drifted"));
        assert!(source_drift.contains(file.source_sha256));

        let mut drifted_metadata = exact;
        drifted_metadata.flags.insert("module".to_owned());
        let metadata_drift = authenticate_module_graph_file_digest(
            Path::new(file.path),
            file.source_sha256,
            &drifted_metadata,
            file,
        )
        .unwrap_err();
        assert!(metadata_drift.contains("metadata shape drifted"));

        let path_drift = authenticate_module_graph_file_digest(
            Path::new("test/language/module-code/other_FIXTURE.js"),
            file.source_sha256,
            &module_metadata(file.metadata),
            file,
        )
        .unwrap_err();
        assert!(path_drift.contains("path drifted"));
    }

    #[test]
    fn recursive_fixture_closure_authentication_rejects_nested_drift() {
        const ROOT_SOURCE: &str =
            "/*---\nflags: [module]\n---*/\nimport \"./fixture_FIXTURE.js\";\n";
        const FIXTURE_SOURCE: &str = "export const value = 1;\n";
        const REQUESTS: [ModuleRequestAdmission; 1] = [ModuleRequestAdmission {
            specifier: "./fixture_FIXTURE.js",
            normalized_path: "test/fixture_FIXTURE.js",
        }];
        const FILES: [ModuleGraphFileAdmission; 2] = [
            ModuleGraphFileAdmission {
                path: "test/root.js",
                source_sha256: "32d8e8b1d38a53f8f4873d89cd0d00a115c33b0ed8294eb016e22e3edea95afe",
                metadata: MODULE_METADATA,
                requests: &REQUESTS,
            },
            ModuleGraphFileAdmission {
                path: "test/fixture_FIXTURE.js",
                source_sha256: "5d8f65d2774e206bc9f7a7a4ad39ca2dc563b5c31e46ab57ef4874961237ce29",
                metadata: MODULE_FIXTURE_METADATA,
                requests: &[],
            },
        ];
        const ADMISSION: FixtureGraphModuleAdmission = FixtureGraphModuleAdmission {
            root_path: "test/root.js",
            files: &FILES,
        };

        let exact = authenticate_fixture_graph_closure(&ADMISSION, |path| match path {
            "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
            "test/fixture_FIXTURE.js" => Ok(FIXTURE_SOURCE.to_owned()),
            _ => Err(format!("unexpected path: {path}")),
        });
        assert_eq!(exact, Ok(()));

        let drift = authenticate_fixture_graph_closure(&ADMISSION, |path| match path {
            "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
            "test/fixture_FIXTURE.js" => Ok("export const value = 2;\n".to_owned()),
            _ => Err(format!("unexpected path: {path}")),
        })
        .unwrap_err();
        assert!(drift.contains("source drifted"));
        assert!(drift.contains("fixture_FIXTURE.js"));
    }

    #[test]
    fn fixture_graph_loader_normalization_rejects_unlisted_edges() {
        let admission = &FIXTURE_GRAPH_MODULE_ADMISSIONS[0];
        let base = admission.root_path;
        let request = admission.files[0].requests[0];
        assert_eq!(
            normalize_exact_module_request(Path::new(admission.root_path), base, request.specifier,),
            Ok(request.normalized_path.to_owned())
        );
        assert!(
            normalize_exact_module_request(
                Path::new(admission.root_path),
                base,
                "./unlisted_FIXTURE.js",
            )
            .unwrap_err()
            .contains("unaudited request")
        );
        assert!(
            normalize_exact_module_request(
                Path::new(admission.root_path),
                "test/language/module-code/unlisted.js",
                request.specifier,
            )
            .unwrap_err()
            .contains("unaudited base")
        );
    }

    #[test]
    fn ordinary_module_is_not_admitted() {
        let metadata = metadata(&["module"], &[], &[]);
        assert_eq!(
            is_exact_dependency_free_module_test(
                Path::new("test/language/module-code/not-a-pinned-root.js"),
                "export {};",
                &metadata,
            ),
            Ok(false)
        );
        assert_eq!(
            exact_module_test(
                Path::new("."),
                Path::new("test/language/module-code/not-a-pinned-root.js"),
                "export {};",
                &metadata,
            ),
            Ok(None)
        );
        assert_ne!(
            exact_module_test(
                Path::new("."),
                Path::new(DEPENDENCY_FREE_MODULE_ADMISSIONS[0].path),
                "drifted",
                &module_metadata(DEPENDENCY_FREE_MODULE_ADMISSIONS[0].metadata),
            ),
            Ok(Some(ExactModuleTest::DependencyFree))
        );
        assert_eq!(
            missing_host_capability_hints(
                Path::new("test/language/module-code/not-a-pinned-root.js"),
                "export {};",
                &metadata,
                false,
            ),
            ["module"]
        );
    }

    #[test]
    fn agent_host_admission_ledger_is_exact_sorted_and_metadata_frozen() {
        assert_eq!(AGENT_HOST_ADMISSIONS.len(), 59);
        assert!(
            AGENT_HOST_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );

        let broadcast = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent broadcast cohort A")
            .collect::<Vec<_>>();
        assert_eq!(broadcast.len(), 15);
        let ledger = broadcast
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&ledger).unwrap(),
            "b467b2cdca29ad877981b7894e5b28bdf966385034aa5e722d9d81b86b19c0cf"
        );

        let mut feature_shapes = BTreeSet::new();
        for admission in broadcast {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut drifted = exact.clone();
            drifted.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&drifted, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 3);

        let bounded_wait = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent bounded wait cohort A")
            .collect::<Vec<_>>();
        assert_eq!(bounded_wait.len(), 22);
        let ledger = bounded_wait
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&ledger).unwrap(),
            "79105013edd054a045fe16f3de55fe1b5fb233e373ac9052c1213f1c4bcea04d"
        );
        let mut feature_shapes = BTreeSet::new();
        for admission in bounded_wait {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut include_drift = exact.clone();
            include_drift.includes.push("extra.js".to_owned());
            assert!(!agent_host_metadata_matches(&include_drift, admission));

            let mut flag_drift = exact.clone();
            flag_drift.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&flag_drift, admission));

            let mut negative_drift = exact.clone();
            negative_drift.negative = Some(Default::default());
            assert!(!agent_host_metadata_matches(&negative_drift, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 2);

        let wake_count_location = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent wake/count/location cohort")
            .collect::<Vec<_>>();
        assert_eq!(wake_count_location.len(), 17);
        let source_ledger = wake_count_location
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&source_ledger).unwrap(),
            "04625efdf79624f49c5bcc24282eae8962ba29294b4e3be6b39958083763e472"
        );
        let metadata_ledger = wake_count_location
            .iter()
            .map(|admission| {
                format!(
                    "{}\tflags=-\tfeatures={}\tincludes=atomicsHelper.js\tnegative=-\n",
                    admission.path,
                    admission.features.join(",")
                )
            })
            .collect::<String>();
        assert_eq!(
            source_sha256(&metadata_ledger).unwrap(),
            "bcf9a3992212ea0dcfb401b5205dbe3afbaa21c2c8f9d459e413c0845a36897c"
        );
        let mut feature_shapes = BTreeSet::new();
        for admission in wake_count_location {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut include_drift = exact.clone();
            include_drift.includes.push("extra.js".to_owned());
            assert!(!agent_host_metadata_matches(&include_drift, admission));

            let mut flag_drift = exact.clone();
            flag_drift.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&flag_drift, admission));

            let mut negative_drift = exact.clone();
            negative_drift.negative = Some(Default::default());
            assert!(!agent_host_metadata_matches(&negative_drift, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 2);

        let fifo_wake_order = AGENT_HOST_ADMISSIONS
            .iter()
            .filter(|admission| admission.cohort == "Test262 agent FIFO wake-order cohort")
            .collect::<Vec<_>>();
        assert_eq!(fifo_wake_order.len(), 4);
        let source_ledger = fifo_wake_order
            .iter()
            .map(|admission| format!("{}\t{}\n", admission.path, admission.source_sha256))
            .collect::<String>();
        assert_eq!(
            source_sha256(&source_ledger).unwrap(),
            "6881f53503b504225342ba6611216642a6799f099255f7b6846b762b2865d358"
        );
        let metadata_ledger = fifo_wake_order
            .iter()
            .map(|admission| {
                format!(
                    "{}\tflags=-\tfeatures={}\tincludes=atomicsHelper.js\tnegative=-\n",
                    admission.path,
                    admission.features.join(",")
                )
            })
            .collect::<String>();
        assert_eq!(
            source_sha256(&metadata_ledger).unwrap(),
            "6f22656e524ec7736801c3e6a46d469c153da77437735d5fd348e0480c9ac8f7"
        );
        let mut feature_shapes = BTreeSet::new();
        for admission in fifo_wake_order {
            feature_shapes.insert(admission.features);
            let exact = metadata(&[], admission.features, &["atomicsHelper.js"]);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(admission.source_sha256));

            let mut include_drift = exact.clone();
            include_drift.includes.push("extra.js".to_owned());
            assert!(!agent_host_metadata_matches(&include_drift, admission));

            let mut flag_drift = exact.clone();
            flag_drift.flags.insert("noStrict".to_owned());
            assert!(!agent_host_metadata_matches(&flag_drift, admission));

            let mut negative_drift = exact.clone();
            negative_drift.negative = Some(Default::default());
            assert!(!agent_host_metadata_matches(&negative_drift, admission));

            let mut feature_drift = exact;
            feature_drift.features.push("feature-drift".to_owned());
            assert!(!agent_host_metadata_matches(&feature_drift, admission));
        }
        assert_eq!(feature_shapes.len(), 2);

        let stage_a = AGENT_HOST_ADMISSIONS
            .iter()
            .find(|admission| admission.cohort == "Test262 agent Stage A")
            .unwrap();
        assert_eq!(stage_a.path, "test/built-ins/Atomics/wait/good-views.js");
        assert_eq!(
            stage_a.source_sha256,
            "7ab45f324e0f668a9d9f3df03c866b0ac32276eb1dfb649d1e5783a88f70bb21"
        );
        assert!(agent_host_metadata_matches(
            &metadata(&[], &["Atomics"], &["atomicsHelper.js"]),
            stage_a
        ));
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
    fn installed_hosts_remove_only_their_typed_discovered_gaps() {
        let metadata = metadata(&["CanBlockIsFalse"], &[], &["detachArrayBuffer.js"]);
        let mut missing = missing_host_capability_hints(
            Path::new("test/example.js"),
            "$262.createRealm(); $262.detachArrayBuffer(buffer); $262.evalScript('0'); \
             $262.gc(); $262.global; $262.agent; $262.IsHTMLDDA;",
            &metadata,
            false,
        );
        HostCapabilities {
            agent: false,
            can_block_false: true,
            create_realm: true,
            detach_array_buffer: true,
            eval_script: true,
            gc: true,
            global: true,
            is_html_dda: true,
        }
        .retain_missing(&mut missing);
        assert_eq!(missing, ["agent"]);
    }

    #[test]
    fn disabled_typed_hosts_remain_missing() {
        let mut missing = vec![
            "can-block:false".to_owned(),
            "create-realm".to_owned(),
            "detach-array-buffer".to_owned(),
            "eval-script".to_owned(),
            "gc".to_owned(),
            "global".to_owned(),
        ];
        HostCapabilities::default().retain_missing(&mut missing);
        assert_eq!(
            missing,
            [
                "can-block:false",
                "create-realm",
                "detach-array-buffer",
                "eval-script",
                "gc",
                "global",
            ]
        );
    }

    #[test]
    fn atomics_cross_realm_metadata_gap_is_source_audited_and_exact() {
        const PATH: &str = "test/staging/sm/Atomics/cross-compartment.js";
        const SOURCE: &str = "const otherGlobal = $262.createRealm().global; const buffer = new \
                              otherGlobal.SharedArrayBuffer(4); Atomics.load(new \
                              otherGlobal.Int32Array(buffer), 0);";
        const SOURCE_SHA256: &str =
            "3cb79dbb8554f721f371c78cad9fe21234dc9b249f27e15e372abacdd014cb47";

        let mut hints = BTreeSet::from(["host-create-realm-required".to_owned()]);
        insert_atomics_cross_realm_feature_hints(
            &mut hints,
            Path::new(PATH),
            SOURCE,
            &source_tokens(SOURCE, false),
            PATH,
            SOURCE_SHA256,
        )
        .unwrap();
        assert_eq!(
            hints,
            BTreeSet::from([
                "Atomics".to_owned(),
                "SharedArrayBuffer".to_owned(),
                "host-create-realm-required".to_owned(),
            ])
        );
        assert_eq!(
            supplemental_feature_hints(Path::new("test/example.js"), SOURCE).unwrap(),
            ["host-create-realm-required"]
        );

        assert!(supplemental_feature_hints(Path::new(PATH), SOURCE).is_err());

        let shape_drift = "$262.createRealm(); Atomics.load;";
        let mut shape_hints = BTreeSet::from(["host-create-realm-required".to_owned()]);
        assert!(
            insert_atomics_cross_realm_feature_hints(
                &mut shape_hints,
                Path::new(PATH),
                shape_drift,
                &source_tokens(shape_drift, false),
                PATH,
                &source_sha256(shape_drift).unwrap(),
            )
            .is_err()
        );
        assert_eq!(
            shape_hints,
            BTreeSet::from(["host-create-realm-required".to_owned()])
        );
    }

    #[test]
    fn atomics_detached_buffers_requirement_is_path_and_source_hash_bound() {
        const PATH: &str = "test/staging/sm/Atomics/detached-buffers.js";
        const SOURCE: &str = "abc";
        const SOURCE_SHA256: &str =
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

        let mut hints = BTreeSet::new();
        insert_exact_source_feature_hint(
            &mut hints,
            Path::new(PATH),
            SOURCE,
            PATH,
            SOURCE_SHA256,
            "Atomics",
        )
        .unwrap();
        assert_eq!(hints, BTreeSet::from(["Atomics".to_owned()]));

        let mut wrong_path_hints = BTreeSet::new();
        insert_exact_source_feature_hint(
            &mut wrong_path_hints,
            Path::new("test/example.js"),
            SOURCE,
            PATH,
            SOURCE_SHA256,
            "Atomics",
        )
        .unwrap();
        assert!(wrong_path_hints.is_empty());

        let mut drifted_source_hints = BTreeSet::new();
        assert!(
            insert_exact_source_feature_hint(
                &mut drifted_source_hints,
                Path::new(PATH),
                "abd",
                PATH,
                SOURCE_SHA256,
                "Atomics",
            )
            .is_err()
        );
        assert!(drifted_source_hints.is_empty());
        assert!(supplemental_feature_hints(Path::new(PATH), SOURCE).is_err());
    }

    #[test]
    fn realm_host_admission_tags_are_source_scoped_and_ignore_hidden_text() {
        assert_eq!(
            supplemental_feature_hints(
                Path::new("test/example.js"),
                "$262.evalScript('0'); $262.createRealm();"
            )
            .unwrap(),
            ["host-create-realm-required", "host-eval-script-required"]
        );
        assert!(
            supplemental_feature_hints(
                Path::new("test/example.js"),
                r#""$262.createRealm"; /* $262.evalScript */ 0;"#
            )
            .unwrap()
            .is_empty()
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
