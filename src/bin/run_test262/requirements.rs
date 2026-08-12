use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use super::admissions::{
    AdmissionCatalog, AgentHostAdmission, ModuleAdmission, ModuleGraphFileAdmission,
    ModuleMetadataContract, SupplementalAdmission, SupplementalPolicy,
};
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExactModuleTest {
    DependencyFree,
    FixtureGraph,
}

#[derive(Clone, Copy)]
struct ExactModuleGraphAdmission<'a> {
    root_path: &'a str,
    files: &'a [ModuleGraphFileAdmission],
    closure_file_count: usize,
}

/// Admit only one of the pinned, dependency-free module roots above.
///
/// The coordinator and worker both call this function. An exact-path source or
/// metadata change is an audit failure, while an unlisted module is simply not
/// admitted and remains classified as unsupported by the coordinator.
pub(super) fn is_exact_dependency_free_module_test(
    admissions: &AdmissionCatalog,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = admissions.module(path) else {
        return Ok(false);
    };
    let actual_sha256 = source_sha256(source)?;
    authenticate_dependency_free_module_test(path, &actual_sha256, metadata, admission)
}

fn authenticate_dependency_free_module_test(
    path: &Path,
    actual_sha256: &str,
    metadata: &Metadata,
    admission: &ModuleAdmission,
) -> Result<bool, String> {
    if path != Path::new(&admission.path) {
        return Ok(false);
    }
    if actual_sha256 != admission.source_sha256 {
        return Err(format!(
            "dependency-free module source drifted for {}: expected SHA-256 {}, found {actual_sha256}",
            admission.path, admission.source_sha256
        ));
    }
    if !module_metadata_matches(metadata, &admission.metadata) {
        return Err(format!(
            "dependency-free module metadata shape drifted for {}",
            admission.path
        ));
    }
    Ok(true)
}

/// Authenticate one of the deliberately narrow static-module execution
/// frontiers. An unlisted module remains unadmitted without touching any
/// fixture file; an exact graph root authenticates its complete recursive
/// closure before either the coordinator or worker can remove `module` from
/// the missing-host set.
pub(super) fn exact_module_test(
    admissions: &AdmissionCatalog,
    suite: &Path,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<Option<ExactModuleTest>, String> {
    if is_exact_dependency_free_module_test(admissions, path, source, metadata)? {
        return Ok(Some(ExactModuleTest::DependencyFree));
    }
    if is_exact_fixture_graph_module_test(admissions, suite, path, source, metadata)? {
        return Ok(Some(ExactModuleTest::FixtureGraph));
    }
    Ok(None)
}

fn is_exact_fixture_graph_module_test(
    admissions: &AdmissionCatalog,
    suite: &Path,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = exact_module_graph_admission(admissions, path) else {
        return Ok(false);
    };
    let root = module_graph_file(admission, admission.root_path).ok_or_else(|| {
        format!(
            "fixture graph admission has no root file: {}",
            admission.root_path
        )
    })?;
    authenticate_module_graph_file(path, source, metadata, root)?;
    authenticate_exact_module_graph_closure(admission, |relative| {
        read_regular_module_graph_text(suite, relative)
    })?;
    Ok(true)
}

fn exact_module_graph_admission<'a>(
    admissions: &'a AdmissionCatalog,
    root_path: &Path,
) -> Option<ExactModuleGraphAdmission<'a>> {
    admissions
        .graph_root(root_path)
        .map(|root| ExactModuleGraphAdmission {
            root_path: &root.path,
            files: admissions.graph_files(&root.group),
            closure_file_count: root.closure_file_count,
        })
}

fn module_graph_file<'a>(
    admission: ExactModuleGraphAdmission<'a>,
    path: &str,
) -> Option<&'a ModuleGraphFileAdmission> {
    admission.files.iter().find(|file| file.path == path)
}

fn authenticate_exact_module_graph_closure(
    admission: ExactModuleGraphAdmission<'_>,
    mut read_source: impl FnMut(&str) -> Result<String, String>,
) -> Result<(), String> {
    let visited = reachable_module_graph_paths(admission)?;
    if visited.len() != admission.closure_file_count {
        return Err(format!(
            "fixture graph recursive closure size drifted for {}: expected {}, found {}",
            admission.root_path,
            admission.closure_file_count,
            visited.len()
        ));
    }
    for path in &visited {
        let file = module_graph_file(admission, path).ok_or_else(|| {
            format!(
                "fixture graph edge escaped the authenticated closure for {}: {path}",
                admission.root_path
            )
        })?;
        let source = read_source(path)?;
        let metadata = module_graph_file_metadata(&source, file)?;
        authenticate_module_graph_file(Path::new(path), &source, &metadata, file)?;
    }
    Ok(())
}

fn module_graph_file_metadata(
    source: &str,
    file: &ModuleGraphFileAdmission,
) -> Result<Metadata, String> {
    if file.is_json_text() {
        // JSON fixtures are authenticated and loaded byte-for-byte as UTF-8
        // text. Test262 frontmatter is a JavaScript-source convention, so JSON
        // never passes through that parser even if its string data contains a
        // frontmatter-looking sequence.
        return Ok(Metadata::default());
    }
    parse_metadata(source).map_err(|error| {
        format!(
            "parse authenticated module metadata for {}: {error}",
            file.path
        )
    })
}

fn reachable_module_graph_paths(
    admission: ExactModuleGraphAdmission<'_>,
) -> Result<BTreeSet<&str>, String> {
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
        for request in file.requests.iter().rev() {
            if module_graph_file(admission, &request.normalized_path).is_none() {
                return Err(format!(
                    "fixture graph request escaped the authenticated closure for {}: {} -> {}",
                    admission.root_path, request.specifier, request.normalized_path
                ));
            }
            pending.push(&request.normalized_path);
        }
    }
    Ok(visited)
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
    if path != Path::new(&file.path) {
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
    if file.is_json_text()
        && (metadata != &Metadata::default() || file.metadata != ModuleMetadataContract::default())
    {
        return Err(format!(
            "JSON fixture graph metadata must be empty for {}",
            file.path
        ));
    }
    if !module_metadata_matches(metadata, &file.metadata) {
        return Err(format!(
            "fixture graph module metadata shape drifted for {}",
            file.path
        ));
    }
    Ok(())
}

fn read_regular_module_graph_text(suite: &Path, relative: &str) -> Result<String, String> {
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
    admissions: &AdmissionCatalog,
    root_path: &Path,
    base_name: &str,
    specifier: &str,
) -> Result<String, String> {
    let admission = exact_module_graph_admission(admissions, root_path).ok_or_else(|| {
        format!(
            "module loader rejected unaudited root: {}",
            root_path.display()
        )
    })?;
    let reachable = reachable_module_graph_paths(admission)?;
    if !reachable.contains(base_name) {
        return Err(format!(
            "module loader rejected unaudited base module: {base_name}"
        ));
    }
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
    admissions: &AdmissionCatalog,
    suite: &Path,
    root_path: &Path,
    normalized_name: &str,
) -> Result<String, String> {
    let admission = exact_module_graph_admission(admissions, root_path).ok_or_else(|| {
        format!(
            "module loader rejected unaudited root: {}",
            root_path.display()
        )
    })?;
    load_exact_module_fixture_from_admission(admission, suite, normalized_name)
}

fn load_exact_module_fixture_from_admission(
    admission: ExactModuleGraphAdmission<'_>,
    suite: &Path,
    normalized_name: &str,
) -> Result<String, String> {
    let reachable = reachable_module_graph_paths(admission)?;
    if !reachable.contains(normalized_name) {
        return Err(format!(
            "module loader rejected unaudited fixture: {normalized_name}"
        ));
    }
    let file = module_graph_file(admission, normalized_name)
        .filter(|file| file.path != admission.root_path)
        .ok_or_else(|| format!("module loader rejected unaudited fixture: {normalized_name}"))?;
    let source = read_regular_module_graph_text(suite, &file.path)?;
    let metadata = module_graph_file_metadata(&source, file)?;
    authenticate_module_graph_file(Path::new(&file.path), &source, &metadata, file)?;
    Ok(source)
}

fn module_metadata_matches(metadata: &Metadata, contract: &ModuleMetadataContract) -> bool {
    metadata
        .includes
        .iter()
        .map(String::as_str)
        .eq(contract.includes.iter().map(String::as_str))
        && metadata
            .flags
            .iter()
            .map(String::as_str)
            .eq(contract.flags.iter().map(String::as_str))
        && metadata
            .features
            .iter()
            .map(String::as_str)
            .eq(contract.features.iter().map(String::as_str))
        && match (&metadata.negative, &contract.negative) {
            (None, None) => true,
            (Some(actual), Some(expected)) => {
                actual.phase.as_deref() == Some(expected.phase.as_str())
                    && actual.error_type.as_deref() == Some(expected.error_type.as_str())
            }
            _ => false,
        }
}

/// Admit only source- and metadata-audited `$262.agent` tests.
///
/// The exact path check prevents a profile entry from broadening the host
/// surface. The source hash and complete metadata shape prevent an in-place
/// Test262 update from silently inheriting an earlier admission.
pub(super) fn is_exact_agent_host_test(
    admissions: &AdmissionCatalog,
    path: &Path,
    source: &str,
    metadata: &Metadata,
) -> Result<bool, String> {
    let Some(admission) = admissions.agent_host(path) else {
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
            .eq(admission.features.iter().map(String::as_str))
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
pub(super) fn supplemental_feature_hints(
    admissions: &AdmissionCatalog,
    path: &Path,
    source: &str,
) -> Result<Vec<String>, String> {
    let tokens = source_tokens(source, false);
    let members = member_names(&tokens);
    let mut hints = BTreeSet::new();
    if members.contains(&"createRealm") {
        hints.insert("host-create-realm-required".to_owned());
    }
    if members.contains(&"evalScript") {
        hints.insert("host-eval-script-required".to_owned());
    }

    if let Some(admission) = admissions.supplemental(path) {
        authenticate_supplemental_source(path, source, admission)?;
        match admission.policy {
            SupplementalPolicy::AtomicsCrossRealm => {
                insert_atomics_cross_realm_feature_hints(&mut hints, &tokens, admission)?;
            }
            SupplementalPolicy::ExactFeatures => {
                hints.extend(admission.features.iter().cloned());
            }
        }
    }

    Ok(hints.into_iter().collect())
}

fn insert_atomics_cross_realm_feature_hints(
    hints: &mut BTreeSet<String>,
    tokens: &[SourceToken<'_>],
    admission: &SupplementalAdmission,
) -> Result<(), String> {
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
            "supplemental feature source shape drifted for {}",
            admission.path
        ));
    }
    hints.extend(admission.features.iter().cloned());
    Ok(())
}

fn authenticate_supplemental_source(
    path: &Path,
    source: &str,
    admission: &SupplementalAdmission,
) -> Result<(), String> {
    debug_assert_eq!(path, Path::new(&admission.path));
    let actual_sha256 = source_sha256(source)?;
    if actual_sha256 != admission.source_sha256 {
        return Err(format!(
            "supplemental feature audit drifted for {}: expected source SHA-256 {}, found {actual_sha256}",
            admission.path, admission.source_sha256
        ));
    }
    Ok(())
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
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::ops::Deref;
    use std::path::{Path, PathBuf};
    use std::sync::LazyLock;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        ExactModuleGraphAdmission, ExactModuleTest, HostCapabilities, agent_host_metadata_matches,
        authenticate_dependency_free_module_test, authenticate_exact_module_graph_closure,
        authenticate_module_graph_file, authenticate_module_graph_file_digest,
        authenticate_supplemental_source,
        exact_module_graph_admission as exact_module_graph_admission_impl,
        exact_module_test as exact_module_test_impl,
        generator_destructuring_source_needs_async_guard, insert_atomics_cross_realm_feature_hints,
        is_exact_agent_host_test as is_exact_agent_host_test_impl,
        is_exact_dependency_free_module_test as is_exact_dependency_free_module_test_impl,
        load_exact_module_fixture_from_admission, missing_host_capability_hints,
        module_metadata_matches,
        normalize_exact_module_request as normalize_exact_module_request_impl,
        reachable_module_graph_paths, source_sha256, source_tokens,
        supplemental_feature_hints as supplemental_feature_hints_impl,
    };
    use crate::admissions::{
        AdmissionCatalog, AgentHostAdmission, ModuleAdmission, ModuleGraphFileAdmission,
        ModuleGraphRootAdmission, ModuleMetadataContract, ModuleRequestAdmission,
        SupplementalAdmission, SupplementalPolicy,
    };
    use crate::metadata::{Metadata, NegativeExpectation, parse_metadata};

    const ADMISSIONS_DATA: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/dev-support/test262/admissions.tsv"
    ));

    static ADMISSIONS: LazyLock<AdmissionCatalog> =
        LazyLock::new(|| AdmissionCatalog::parse(ADMISSIONS_DATA).expect("parse admissions"));

    struct AdmissionView<T>(LazyLock<Vec<T>>);

    impl<T> Deref for AdmissionView<T> {
        type Target = [T];

        fn deref(&self) -> &Self::Target {
            &self.0
        }
    }

    impl<'a, T> IntoIterator for &'a AdmissionView<T> {
        type Item = &'a T;
        type IntoIter = std::slice::Iter<'a, T>;

        fn into_iter(self) -> Self::IntoIter {
            self.iter()
        }
    }

    fn modules(group: &str) -> Vec<ModuleAdmission> {
        ADMISSIONS.modules_in_group(group).cloned().collect()
    }

    fn graph_roots(group: &str) -> Vec<ModuleGraphRootAdmission> {
        ADMISSIONS.graph_roots_in_group(group).cloned().collect()
    }

    fn graph_files(group: &str) -> Vec<ModuleGraphFileAdmission> {
        ADMISSIONS.graph_files(group).to_vec()
    }

    static DEPENDENCY_FREE_MODULE_ADMISSIONS: AdmissionView<ModuleAdmission> =
        AdmissionView(LazyLock::new(|| modules("dependency-free")));
    static DECL_POSITION_MODULE_ADMISSIONS: AdmissionView<ModuleAdmission> =
        AdmissionView(LazyLock::new(|| modules("module-decl-position-a")));
    static STATIC_NEGATIVE_MODULE_ADMISSIONS: AdmissionView<ModuleAdmission> =
        AdmissionView(LazyLock::new(|| modules("module-static-negative-a")));
    static DEFAULT_MODULE_ROOT_ADMISSIONS: AdmissionView<ModuleGraphRootAdmission> =
        AdmissionView(LazyLock::new(|| graph_roots("module-default-a")));
    static DEFAULT_MODULE_FILE_ADMISSIONS: AdmissionView<ModuleGraphFileAdmission> =
        AdmissionView(LazyLock::new(|| graph_files("module-default-a")));
    static IMPORT_META_MODULE_ROOT_ADMISSIONS: AdmissionView<ModuleGraphRootAdmission> =
        AdmissionView(LazyLock::new(|| graph_roots("import-meta-a")));
    static IMPORT_META_MODULE_FILE_ADMISSIONS: AdmissionView<ModuleGraphFileAdmission> =
        AdmissionView(LazyLock::new(|| graph_files("import-meta-a")));
    static NAMESPACE_MODULE_ROOT_ADMISSIONS: AdmissionView<ModuleGraphRootAdmission> =
        AdmissionView(LazyLock::new(|| graph_roots("module-namespace-a")));
    static NAMESPACE_MODULE_FILE_ADMISSIONS: AdmissionView<ModuleGraphFileAdmission> =
        AdmissionView(LazyLock::new(|| graph_files("module-namespace-a")));
    static AGENT_HOST_ADMISSIONS: AdmissionView<AgentHostAdmission> =
        AdmissionView(LazyLock::new(|| {
            ADMISSIONS.agent_hosts().cloned().collect()
        }));

    #[derive(Clone)]
    struct FixtureGraphModuleAdmission {
        root_path: String,
        files: Vec<ModuleGraphFileAdmission>,
    }

    static FIXTURE_GRAPH_MODULE_ADMISSIONS: AdmissionView<FixtureGraphModuleAdmission> =
        AdmissionView(LazyLock::new(|| {
            let mut fixtures = ADMISSIONS
                .graph_roots()
                .filter(|root| root.group.starts_with("fixture-"))
                .map(|root| {
                    let mut files = ADMISSIONS.graph_files(&root.group).to_vec();
                    let root_index = files
                        .iter()
                        .position(|file| file.path == root.path)
                        .expect("fixture graph root is present");
                    files.swap(0, root_index);
                    FixtureGraphModuleAdmission {
                        root_path: root.path.clone(),
                        files,
                    }
                })
                .collect::<Vec<_>>();
            fixtures.sort_by(|left, right| left.root_path.cmp(&right.root_path));
            fixtures
        }));

    fn exact_module_graph_admission(path: &Path) -> Option<ExactModuleGraphAdmission<'static>> {
        exact_module_graph_admission_impl(&ADMISSIONS, path)
    }

    fn exact_module_test(
        suite: &Path,
        path: &Path,
        source: &str,
        metadata: &Metadata,
    ) -> Result<Option<ExactModuleTest>, String> {
        exact_module_test_impl(&ADMISSIONS, suite, path, source, metadata)
    }

    fn is_exact_dependency_free_module_test(
        path: &Path,
        source: &str,
        metadata: &Metadata,
    ) -> Result<bool, String> {
        is_exact_dependency_free_module_test_impl(&ADMISSIONS, path, source, metadata)
    }

    fn is_exact_agent_host_test(
        path: &Path,
        source: &str,
        metadata: &Metadata,
    ) -> Result<bool, String> {
        is_exact_agent_host_test_impl(&ADMISSIONS, path, source, metadata)
    }

    fn normalize_exact_module_request(
        root_path: &Path,
        base_name: &str,
        specifier: &str,
    ) -> Result<String, String> {
        normalize_exact_module_request_impl(&ADMISSIONS, root_path, base_name, specifier)
    }

    fn supplemental_feature_hints(path: &Path, source: &str) -> Result<Vec<String>, String> {
        supplemental_feature_hints_impl(&ADMISSIONS, path, source)
    }

    const DEFAULT_MODULE_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a.txt"
    ));
    const DEFAULT_MODULE_SOURCES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-sources.txt"
    ));
    const DEFAULT_MODULE_EDGES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-edges.tsv"
    ));
    const DEFAULT_MODULE_CLOSURES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-closures.tsv"
    ));
    const DEFAULT_MODULE_LEDGER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-ledger.tsv"
    ));
    const DEFAULT_MODULE_NEGATIVES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-default-a-negatives.txt"
    ));
    const DECL_POSITION_MODULE_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-decl-position-a.txt"
    ));
    const DECL_POSITION_MODULE_LEDGER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-decl-position-a-ledger.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a.txt"
    ));
    const STATIC_NEGATIVE_MODULE_LEDGER: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-ledger.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_REQUESTS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-requests.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_EXCLUSIONS: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-exclusions.tsv"
    ));
    const STATIC_NEGATIVE_MODULE_PROVENANCE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-module-static-negative-a-provenance.tsv"
    ));
    const IMPORT_META_SCRIPT_ROOTS: [&str; 5] = [
        "test/language/expressions/import.meta/syntax/goal-async-function-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-async-generator-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-function-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-generator-params-or-body.js",
        "test/language/expressions/import.meta/syntax/goal-script.js",
    ];
    const IMPORT_META_MODULE_NEGATIVES: [&str; 11] = [
        "test/language/expressions/import.meta/syntax/escape-sequence-import.js",
        "test/language/expressions/import.meta/syntax/escape-sequence-meta.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-array-rest-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-assignment-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-await-of-loop.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-in-loop.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-for-of-loop.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-object-rest-destructuring-expr.js",
        "test/language/expressions/import.meta/syntax/invalid-assignment-target-update-expr.js",
    ];
    const IMPORT_META_ADJACENT_EXCLUSIONS: [&str; 4] = [
        "test/language/expressions/assignmenttargettype/direct-import.meta.js",
        "test/language/expressions/assignmenttargettype/parenthesized-import.meta.js",
        "test/language/expressions/dynamic-import/assignment-expression/import-meta.js",
        "test/language/expressions/import.meta/distinct-for-each-module_FIXTURE.js",
    ];

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

    fn agent_metadata(admission: &AgentHostAdmission) -> Metadata {
        Metadata {
            includes: vec!["atomicsHelper.js".to_owned()],
            features: admission.features.clone(),
            ..Metadata::default()
        }
    }

    fn module_metadata(contract: &ModuleMetadataContract) -> Metadata {
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
            negative: contract
                .negative
                .as_ref()
                .map(|negative| NegativeExpectation {
                    phase: Some(negative.phase.to_owned()),
                    error_type: Some(negative.error_type.to_owned()),
                }),
        }
    }

    fn audited_module_specifiers(source: &str) -> BTreeSet<String> {
        let source = source
            .find("---*/")
            .map_or(source, |end| &source[end + "---*/".len()..]);
        source
            .lines()
            .filter_map(|line| {
                let line = line.trim_start();
                if !line.starts_with("import") && !line.starts_with("export") {
                    return None;
                }
                let request = if let Some(index) = line.find(" from ") {
                    line[index + " from ".len()..].trim_start()
                } else {
                    let request = line.strip_prefix("import")?;
                    let request = request.trim_start();
                    if !matches!(request.as_bytes().first(), Some(b'\'' | b'"')) {
                        return None;
                    }
                    request
                };
                let quote = request.as_bytes().first().copied()?;
                if !matches!(quote, b'\'' | b'"') {
                    return None;
                }
                let tail = &request[1..];
                let end = tail.as_bytes().iter().position(|byte| *byte == quote)?;
                Some(tail[..end].to_owned())
            })
            .collect()
    }

    fn complete_frontmatter(source: &str) -> &str {
        let Some(start) = source.find("/*---") else {
            return "";
        };
        let marker_end = start
            + source[start..]
                .find("---*/")
                .expect("Test262 frontmatter terminator")
            + "---*/".len();
        if source[marker_end..].starts_with("\r\n") {
            &source[start..marker_end + 2]
        } else if source[marker_end..].starts_with('\n') {
            &source[start..marker_end + 1]
        } else {
            &source[start..marker_end]
        }
    }

    fn normalized_audited_request(base: &str, specifier: &str) -> String {
        let relative = specifier
            .strip_prefix("./")
            .expect("the audited module cohorts use relative child requests");
        Path::new(base)
            .parent()
            .expect("module path has a parent")
            .join(relative)
            .to_string_lossy()
            .into_owned()
    }

    fn collect_non_fixture_js(dir: &Path, suite: &Path, paths: &mut BTreeSet<String>) {
        let mut entries = fs::read_dir(dir)
            .unwrap_or_else(|error| panic!("read {}: {error}", dir.display()))
            .map(|entry| entry.expect("read Test262 namespace entry").path())
            .collect::<Vec<_>>();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                collect_non_fixture_js(&path, suite, paths);
            } else if path.extension().is_some_and(|extension| extension == "js")
                && !path
                    .file_name()
                    .is_some_and(|name| name.to_string_lossy().ends_with("_FIXTURE.js"))
            {
                paths.insert(
                    path.strip_prefix(suite)
                        .expect("namespace file belongs to suite")
                        .to_string_lossy()
                        .into_owned(),
                );
            }
        }
    }

    fn is_default_module_graph_root_name(name: &str) -> bool {
        name.ends_with(".js")
            && !name.ends_with("_FIXTURE.js")
            && ((name.starts_with("eval-export-dflt-")
                && !name.starts_with("eval-export-dflt-expr-err-"))
                || name.starts_with("eval-gtbndng-indirect-")
                || matches!(name, "eval-rqstd-once.js" | "eval-rqstd-order.js")
                || matches!(name, "eval-self-once.js" | "export-star-as-dflt.js")
                || (name.starts_with("instn-")
                    && name.contains("dflt")
                    && !name.starts_with("instn-star-props-dflt")
                    && !name.starts_with("instn-star-as-props-dflt")))
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
            let metadata = module_metadata(&admission.metadata);
            assert!(metadata.is_module());
            assert!(module_metadata_matches(&metadata, &admission.metadata));
            assert_eq!(
                authenticate_dependency_free_module_test(
                    Path::new(&admission.path),
                    &admission.source_sha256,
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
        let exact = module_metadata(&admission.metadata);
        let source_drift = authenticate_dependency_free_module_test(
            Path::new(&admission.path),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &exact,
            admission,
        )
        .unwrap_err();
        assert!(source_drift.contains("source drifted"));
        assert!(source_drift.contains(&admission.source_sha256));

        let mut metadata_drift = exact;
        metadata_drift.flags.insert("async".to_owned());
        let metadata_drift = authenticate_dependency_free_module_test(
            Path::new(&admission.path),
            &admission.source_sha256,
            &metadata_drift,
            admission,
        )
        .unwrap_err();
        assert!(metadata_drift.contains("metadata shape drifted"));
    }

    #[test]
    fn declaration_position_module_admission_is_the_exact_natural_cohort() {
        assert_eq!(DECL_POSITION_MODULE_ADMISSIONS.len(), 86);
        assert!(
            DECL_POSITION_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            DECL_POSITION_MODULE_MANIFEST.lines().collect::<Vec<_>>(),
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .map(|admission| admission.path.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .filter(|admission| admission.path.contains("-export-"))
                .count(),
            43
        );
        assert_eq!(
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .filter(|admission| admission.path.contains("-import-"))
                .count(),
            43
        );
        assert_eq!(
            DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .filter(|admission| admission.metadata.features == ["generators"])
                .count(),
            12
        );

        let ledger_rows = DECL_POSITION_MODULE_LEDGER.lines().skip(1);
        assert_eq!(ledger_rows.clone().count(), 86);
        for (admission, row) in DECL_POSITION_MODULE_ADMISSIONS.iter().zip(ledger_rows) {
            let fields = row.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 9, "{} ledger width", admission.path);
            assert_eq!(fields[0], admission.path);
            assert_eq!(
                fields[1],
                if admission.path.contains("-export-") {
                    "export"
                } else {
                    "import"
                }
            );
            assert_eq!(fields[2], "");
            assert_eq!(fields[3], "module");
            assert_eq!(fields[4], admission.metadata.features.join(","));
            assert_eq!(fields[5], "parse");
            assert_eq!(fields[6], "SyntaxError");
            assert_eq!(fields[7], admission.source_sha256);
            let negative = admission
                .metadata
                .negative
                .as_ref()
                .expect("negative contract");
            assert_eq!(negative.phase, "parse");
            assert_eq!(negative.error_type, "SyntaxError");
        }

        let adjacent = "test/language/module-code/parse-err-export-dflt-const.js";
        assert!(
            !DECL_POSITION_MODULE_ADMISSIONS
                .iter()
                .any(|admission| admission.path == adjacent)
        );
        assert!(
            STATIC_NEGATIVE_MODULE_ADMISSIONS
                .iter()
                .any(|admission| admission.path == adjacent)
        );

        for excluded in [
            "test/language/module-code/import-attributes/import-attribute-empty.js",
            "test/language/module-code/top-level-await/await-expr-resolution.js",
        ] {
            assert_eq!(
                is_exact_dependency_free_module_test(Path::new(excluded), "", &Metadata::default()),
                Ok(false),
                "adjacent module surface was admitted: {excluded}"
            );
        }
    }

    #[test]
    fn declaration_position_module_admission_matches_the_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        for admission in &DECL_POSITION_MODULE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&admission.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", admission.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", admission.path));
            assert_eq!(
                is_exact_dependency_free_module_test(
                    Path::new(&admission.path),
                    &source,
                    &metadata
                ),
                Ok(true),
                "{}",
                admission.path
            );
            assert_eq!(
                exact_module_test(&suite, Path::new(&admission.path), &source, &metadata),
                Ok(Some(ExactModuleTest::DependencyFree)),
                "{}",
                admission.path
            );
        }

        let admission = &DECL_POSITION_MODULE_ADMISSIONS[0];
        let source = fs::read_to_string(suite.join(&admission.path)).expect("read drift canary");
        let metadata = parse_metadata(&source).expect("parse drift canary metadata");
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(&admission.path),
                &format!("{source}\n// source drift"),
                &metadata
            )
            .unwrap_err()
            .contains("source drifted")
        );
        let mut metadata_drift = metadata;
        metadata_drift.features.push("import.meta".to_owned());
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(&admission.path),
                &source,
                &metadata_drift
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );
    }

    #[test]
    fn static_negative_module_admission_is_exact_sorted_and_source_authenticated() {
        assert_eq!(STATIC_NEGATIVE_MODULE_ADMISSIONS.len(), 67);
        assert!(
            STATIC_NEGATIVE_MODULE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            STATIC_NEGATIVE_MODULE_MANIFEST.lines().collect::<Vec<_>>(),
            STATIC_NEGATIVE_MODULE_ADMISSIONS
                .iter()
                .map(|admission| admission.path.as_str())
                .collect::<Vec<_>>()
        );

        let mut feature_counts = BTreeMap::new();
        for admission in &STATIC_NEGATIVE_MODULE_ADMISSIONS {
            *feature_counts
                .entry(admission.metadata.features.join(","))
                .or_insert(0usize) += 1;
            assert!(admission.metadata.includes.is_empty());
            assert_eq!(admission.metadata.flags, ["module"]);
            let negative = admission
                .metadata
                .negative
                .as_ref()
                .expect("negative contract");
            assert_eq!(negative.phase, "parse");
            assert_eq!(negative.error_type, "SyntaxError");
            assert_eq!(
                authenticate_dependency_free_module_test(
                    Path::new(&admission.path),
                    &admission.source_sha256,
                    &module_metadata(&admission.metadata),
                    admission,
                ),
                Ok(true),
                "{}",
                admission.path
            );
            assert!(exact_module_graph_admission(Path::new(&admission.path)).is_none());
        }
        assert_eq!(
            feature_counts,
            BTreeMap::from([
                (String::new(), 57),
                ("export-star-as-namespace-from-module".to_owned(), 4),
                ("generators".to_owned(), 3),
                ("let".to_owned(), 1),
                ("let,const".to_owned(), 1),
                ("new.target".to_owned(), 1),
            ])
        );

        let ledger_rows = STATIC_NEGATIVE_MODULE_LEDGER
            .lines()
            .skip(1)
            .map(|row| {
                let fields = row.split('\t').collect::<Vec<_>>();
                assert_eq!(fields.len(), 9, "{} ledger width", fields[0]);
                (fields[0], fields)
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(ledger_rows.len(), 67);
        for admission in &STATIC_NEGATIVE_MODULE_ADMISSIONS {
            let fields = &ledger_rows[admission.path.as_str()];
            assert_eq!(fields[1], "");
            assert_eq!(fields[2], "module");
            assert_eq!(fields[3], admission.metadata.features.join(","));
            assert_eq!(fields[4], "parse");
            assert_eq!(fields[5], "SyntaxError");
            assert!(matches!(fields[6], "0" | "1"));
            assert_eq!(fields[7], admission.source_sha256);
            assert_eq!(fields[8].len(), 64);
        }

        let mut request_rows = BTreeMap::<&str, Vec<(usize, &str)>>::new();
        for row in STATIC_NEGATIVE_MODULE_REQUESTS.lines().skip(1) {
            let fields = row.split('\t').collect::<Vec<_>>();
            assert_eq!(fields.len(), 3);
            assert!(ledger_rows.contains_key(fields[0]));
            request_rows
                .entry(fields[0])
                .or_default()
                .push((fields[1].parse().expect("request index"), fields[2]));
        }
        assert_eq!(request_rows.values().map(Vec::len).sum::<usize>(), 13);
        for (path, rows) in &request_rows {
            assert_eq!(rows.len(), 1, "{path}");
            assert_eq!(rows[0].0, 0, "{path}");
            assert_eq!(ledger_rows[path][6], "1", "{path}");
        }

        assert_eq!(
            STATIC_NEGATIVE_MODULE_PROVENANCE,
            concat!(
                "metric\tvalue\n",
                "selector\tincludes=[];flags=[module];negative=parse/SyntaxError;features in {[],[export-star-as-namespace-from-module],[generators],[let],[let,const],[new.target]};subtract prior audited negatives\n",
                "parent_profile_sha256\t364f45501f0b3655e801200b4e1ecb24040384a73489da1994528c911574e362\n",
                "parent_audited_negatives\t1450\n",
                "selected_roots\t67\n",
                "manifest_sha256\tdd8e65fab5447123ad48aa383a835893b72a5e899d34d2dce3a81660bdacc145\n",
            )
        );

        let mut surfaces = BTreeMap::new();
        let exclusions = STATIC_NEGATIVE_MODULE_EXCLUSIONS
            .lines()
            .skip(1)
            .map(|row| row.split('\t').collect::<Vec<_>>())
            .collect::<Vec<_>>();
        assert_eq!(exclusions.len(), 25);
        for fields in &exclusions {
            assert_eq!(fields.len(), 10, "{} exclusion width", fields[1]);
            *surfaces.entry(fields[0]).or_insert(0usize) += 1;
            assert!(!ledger_rows.contains_key(fields[1]));
            assert_ne!(fields[2], "selected");
            assert_eq!(fields[8].len(), 64);
            assert_eq!(fields[9].len(), 64);
        }
        assert_eq!(
            surfaces,
            BTreeMap::from([
                ("adjacent-syntax", 4),
                ("class-private", 3),
                ("dynamic-import", 1),
                ("hidden-dynamic-import", 2),
                ("import-attributes", 3),
                ("import-defer", 2),
                ("source-phase-import", 2),
                ("top-level-await", 8),
            ])
        );
    }

    #[test]
    fn static_negative_module_admission_matches_the_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let ledger_rows = STATIC_NEGATIVE_MODULE_LEDGER
            .lines()
            .skip(1)
            .map(|row| {
                let fields = row.split('\t').collect::<Vec<_>>();
                (fields[0], fields)
            })
            .collect::<BTreeMap<_, _>>();
        let mut request_rows = BTreeMap::<&str, BTreeSet<String>>::new();
        for row in STATIC_NEGATIVE_MODULE_REQUESTS.lines().skip(1) {
            let fields = row.split('\t').collect::<Vec<_>>();
            request_rows
                .entry(fields[0])
                .or_default()
                .insert(fields[2].to_owned());
        }

        for admission in &STATIC_NEGATIVE_MODULE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&admission.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", admission.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", admission.path));
            assert_eq!(
                is_exact_dependency_free_module_test(
                    Path::new(&admission.path),
                    &source,
                    &metadata
                ),
                Ok(true),
                "{}",
                admission.path
            );
            assert_eq!(
                exact_module_test(&suite, Path::new(&admission.path), &source, &metadata),
                Ok(Some(ExactModuleTest::DependencyFree)),
                "{}",
                admission.path
            );
            assert_eq!(
                source_sha256(complete_frontmatter(&source)).unwrap(),
                ledger_rows[admission.path.as_str()][8],
                "{} frontmatter",
                admission.path
            );
            assert_eq!(
                audited_module_specifiers(&source),
                request_rows
                    .get(admission.path.as_str())
                    .cloned()
                    .unwrap_or_default(),
                "{} static requests",
                admission.path
            );
        }

        let admission = STATIC_NEGATIVE_MODULE_ADMISSIONS
            .iter()
            .find(|admission| {
                admission.path == "test/language/export/escaped-as-export-specifier.js"
            })
            .expect("request-shaped drift canary");
        let source = fs::read_to_string(suite.join(&admission.path)).expect("read drift canary");
        let metadata = parse_metadata(&source).expect("parse drift canary metadata");
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(&admission.path),
                &format!("{source}\n// source drift"),
                &metadata
            )
            .unwrap_err()
            .contains("source drifted")
        );
        let mut metadata_drift = metadata;
        metadata_drift.flags.insert("generated".to_owned());
        assert!(
            is_exact_dependency_free_module_test(
                Path::new(&admission.path),
                &source,
                &metadata_drift
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        for fields in STATIC_NEGATIVE_MODULE_EXCLUSIONS
            .lines()
            .skip(1)
            .map(|row| row.split('\t').collect::<Vec<_>>())
        {
            let source = fs::read_to_string(suite.join(fields[1]))
                .unwrap_or_else(|error| panic!("read {}: {error}", fields[1]));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", fields[1]));
            assert_eq!(
                source_sha256(&source).unwrap(),
                fields[8],
                "{} source",
                fields[1]
            );
            assert_eq!(
                source_sha256(complete_frontmatter(&source)).unwrap(),
                fields[9],
                "{} frontmatter",
                fields[1]
            );
            assert_eq!(
                is_exact_dependency_free_module_test(Path::new(fields[1]), &source, &metadata),
                Ok(false),
                "excluded surface entered dependency-free admission: {}",
                fields[1]
            );
        }
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
            assert!(module_metadata(&admission.files[0].metadata).is_module());
            let mut reachable = BTreeSet::new();
            let mut pending = vec![admission.root_path.as_str()];
            while let Some(path) = pending.pop() {
                assert!(reachable.insert(path), "duplicate or cyclic edge at {path}");
                let file = admission
                    .files
                    .iter()
                    .find(|file| file.path == path)
                    .expect("every request target stays in its admission");
                assert!(
                    all_paths.insert(file.path.as_str()),
                    "duplicate file {}",
                    file.path
                );
                for request in file.requests.iter().rev() {
                    assert!(request.specifier.starts_with("./"));
                    pending.push(&request.normalized_path);
                }
            }
            assert_eq!(reachable.len(), admission.files.len());
            assert!(
                admission.files[1..]
                    .iter()
                    .all(|file| module_metadata_matches(&Metadata::default(), &file.metadata))
            );
        }
    }

    #[test]
    fn default_module_admission_is_exact_sorted_and_closed() {
        assert_eq!(DEFAULT_MODULE_ROOT_ADMISSIONS.len(), 38);
        assert_eq!(DEFAULT_MODULE_FILE_ADMISSIONS.len(), 58);
        assert!(
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            DEFAULT_MODULE_FILE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            DEFAULT_MODULE_MANIFEST.lines().collect::<Vec<_>>(),
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            DEFAULT_MODULE_SOURCES.lines().collect::<Vec<_>>(),
            DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>()
        );
        assert_eq!(
            DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.requests.len())
                .sum::<usize>(),
            43
        );

        let mut union = BTreeSet::new();
        let mut rooted_request_count = 0;
        let mut self_edge_count = 0;
        let mut expected_edges =
            String::from("root_path\tbase_path\trequest_index\tspecifier\tnormalized_path\n");
        let mut expected_closures = String::from("root_path\tclosure_files\trequest_edges\n");
        for root in &DEFAULT_MODULE_ROOT_ADMISSIONS {
            let admission = ExactModuleGraphAdmission {
                root_path: &root.path,
                files: &DEFAULT_MODULE_FILE_ADMISSIONS,
                closure_file_count: root.closure_file_count,
            };
            let reachable = reachable_module_graph_paths(admission)
                .unwrap_or_else(|error| panic!("{}: {error}", root.path));
            assert_eq!(reachable.len(), root.closure_file_count, "{}", root.path);
            union.extend(reachable.iter().copied());

            let root_file = DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == root.path)
                .expect("every default cohort root is in the source ledger");
            assert!(module_metadata(&root_file.metadata).is_module());

            let mut closure_requests = 0;
            for path in reachable {
                let file = DEFAULT_MODULE_FILE_ADMISSIONS
                    .iter()
                    .find(|file| file.path == path)
                    .expect("every reachable file is in the source ledger");
                for (request_index, request) in file.requests.iter().enumerate() {
                    closure_requests += 1;
                    rooted_request_count += 1;
                    if file.path == request.normalized_path {
                        self_edge_count += 1;
                    }
                    expected_edges.push_str(&format!(
                        "{}\t{}\t{}\t{}\t{}\n",
                        root.path,
                        file.path,
                        request_index,
                        request.specifier,
                        request.normalized_path
                    ));
                }
            }
            expected_closures.push_str(&format!(
                "{}\t{}\t{}\n",
                root.path, root.closure_file_count, closure_requests
            ));
        }
        assert_eq!(union.len(), 58);
        assert_eq!(
            union,
            DEFAULT_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.path.as_str())
                .collect()
        );
        assert_eq!(rooted_request_count, 45);
        assert_eq!(self_edge_count, 21);
        assert_eq!(expected_edges, DEFAULT_MODULE_EDGES);
        assert_eq!(expected_closures, DEFAULT_MODULE_CLOSURES);

        for file in &DEFAULT_MODULE_FILE_ADMISSIONS {
            assert_eq!(file.source_sha256.len(), 64, "{}", file.path);
            assert!(
                file.source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}",
                file.path
            );
            if file.path.ends_with("_FIXTURE.js") {
                assert!(module_metadata_matches(
                    &Metadata::default(),
                    &file.metadata
                ));
            }
            let mut specifiers = BTreeSet::new();
            for request in &file.requests {
                assert!(
                    specifiers.insert(request.specifier.as_str()),
                    "duplicate request {} in {}",
                    request.specifier,
                    file.path
                );
                assert_eq!(
                    request.normalized_path,
                    normalized_audited_request(&file.path, &request.specifier),
                    "{} -> {}",
                    file.path,
                    request.specifier
                );
                assert!(
                    DEFAULT_MODULE_FILE_ADMISSIONS
                        .iter()
                        .any(|candidate| candidate.path == request.normalized_path),
                    "{} -> {}",
                    file.path,
                    request.normalized_path
                );
            }
        }

        let audited_negatives = DEFAULT_MODULE_ROOT_ADMISSIONS
            .iter()
            .filter_map(|root| {
                DEFAULT_MODULE_FILE_ADMISSIONS
                    .iter()
                    .find(|file| file.path == root.path)
                    .filter(|file| file.metadata.negative.is_some())
                    .map(|file| file.path.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            audited_negatives,
            DEFAULT_MODULE_NEGATIVES.lines().collect::<Vec<_>>()
        );
    }

    #[test]
    fn default_module_admission_matches_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let module_dir = suite.join("test/language/module-code");
        let natural_roots = fs::read_dir(&module_dir)
            .expect("read pinned module-code directory")
            .map(|entry| entry.expect("read pinned module-code entry"))
            .filter(|entry| entry.path().is_file())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| is_default_module_graph_root_name(name))
            .map(|name| format!("test/language/module-code/{name}"))
            .collect::<BTreeSet<_>>();
        assert_eq!(natural_roots.len(), 38);
        assert_eq!(
            natural_roots,
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.to_owned())
                .collect()
        );

        let ledger = DEFAULT_MODULE_LEDGER
            .lines()
            .skip(1)
            .map(|line| {
                let fields = line.split('\t').collect::<Vec<_>>();
                assert_eq!(fields.len(), 9, "{line}");
                (fields[0], fields)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(ledger.len(), 58);
        for file in &DEFAULT_MODULE_FILE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&file.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", file.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", file.path));
            authenticate_module_graph_file(Path::new(&file.path), &source, &metadata, file)
                .unwrap_or_else(|error| panic!("authenticate {}: {error}", file.path));
            assert_eq!(
                audited_module_specifiers(&source),
                file.requests
                    .iter()
                    .map(|request| request.specifier.to_owned())
                    .collect(),
                "{} static requests drifted",
                file.path
            );

            let fields = ledger.get(file.path.as_str()).expect("source ledger row");
            assert_eq!(
                fields[1],
                if file.path.ends_with("_FIXTURE.js") {
                    "fixture"
                } else {
                    "root"
                }
            );
            assert_eq!(fields[2], file.metadata.includes.join(","));
            assert_eq!(fields[3], file.metadata.flags.join(","));
            assert_eq!(fields[4], file.metadata.features.join(","));
            assert_eq!(
                fields[5],
                file.metadata
                    .negative
                    .as_ref()
                    .map_or("", |negative| negative.phase.as_str())
            );
            assert_eq!(
                fields[6],
                file.metadata
                    .negative
                    .as_ref()
                    .map_or("", |negative| negative.error_type.as_str())
            );
            assert_eq!(fields[7], file.source_sha256);
            assert_eq!(
                fields[8],
                source_sha256(complete_frontmatter(&source)).expect("hash frontmatter")
            );
        }

        for root in &DEFAULT_MODULE_ROOT_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&root.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", root.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", root.path));
            assert_eq!(
                exact_module_test(&suite, Path::new(&root.path), &source, &metadata),
                Ok(Some(ExactModuleTest::FixtureGraph)),
                "{}",
                root.path
            );
        }
    }

    #[test]
    fn default_module_admission_rejects_drift_and_preserves_cohort_boundaries() {
        let file = DEFAULT_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path == "test/language/module-code/export-star-as-dflt.js")
            .expect("audited default-export root");
        let exact = module_metadata(&file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                &file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &exact,
                file,
            )
            .unwrap_err()
            .contains("source drifted")
        );
        let mut metadata_drift = exact;
        metadata_drift.features.clear();
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                &file.source_sha256,
                &metadata_drift,
                file,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        for excluded in [
            "test/language/expressions/dynamic-import/always-create-new-promise.js",
            "test/language/module-code/top-level-await/await-expr-resolution.js",
            "test/language/import/import-attributes/json-idempotency.js",
            "test/language/module-code/source-phase-import/import-source.js",
        ] {
            assert!(
                exact_module_graph_admission(Path::new(excluded)).is_none(),
                "excluded module surface was admitted: {excluded}"
            );
        }

        let json_path = Path::new("test/language/import/import-attributes/json-value-object.js");
        assert!(exact_module_graph_admission(json_path).is_some());
        assert_eq!(
            ADMISSIONS.graph_root(json_path).unwrap().group,
            "module-json-a"
        );
        assert!(
            DEFAULT_MODULE_ROOT_ADMISSIONS
                .iter()
                .all(|root| root.path != json_path.to_str().unwrap())
        );
    }

    #[test]
    fn import_meta_module_admission_is_the_exact_closed_module_goal_cohort() {
        assert_eq!(IMPORT_META_MODULE_ROOT_ADMISSIONS.len(), 17);
        assert_eq!(IMPORT_META_MODULE_FILE_ADMISSIONS.len(), 18);
        assert!(
            IMPORT_META_MODULE_ROOT_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            IMPORT_META_MODULE_FILE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            IMPORT_META_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.requests.len())
                .sum::<usize>(),
            1
        );

        let root_paths = IMPORT_META_MODULE_ROOT_ADMISSIONS
            .iter()
            .map(|root| root.path.as_str())
            .collect::<BTreeSet<_>>();
        let file_paths = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .map(|file| file.path.as_str())
            .collect::<BTreeSet<_>>();
        assert!(root_paths.is_subset(&file_paths));
        assert_eq!(
            file_paths
                .difference(&root_paths)
                .copied()
                .collect::<Vec<_>>(),
            ["test/language/expressions/import.meta/distinct-for-each-module_FIXTURE.js",]
        );

        let mut union = BTreeSet::new();
        let mut rooted_request_count = 0;
        for root in &IMPORT_META_MODULE_ROOT_ADMISSIONS {
            assert!(!root.path.ends_with("_FIXTURE.js"));
            let admission = exact_module_graph_admission(Path::new(&root.path))
                .expect("every import.meta module root has an exact graph admission");
            assert_eq!(admission.root_path, root.path);
            assert_eq!(admission.closure_file_count, root.closure_file_count);
            assert_eq!(admission.files.len(), 18);

            let reachable = reachable_module_graph_paths(admission)
                .unwrap_or_else(|error| panic!("{}: {error}", root.path));
            assert_eq!(reachable.len(), root.closure_file_count, "{}", root.path);
            assert_eq!(
                root.closure_file_count,
                if root.path.ends_with("/distinct-for-each-module.js") {
                    2
                } else {
                    1
                },
                "{}",
                root.path
            );
            union.extend(reachable.iter().copied());
            rooted_request_count += reachable
                .iter()
                .map(|path| {
                    IMPORT_META_MODULE_FILE_ADMISSIONS
                        .iter()
                        .find(|file| file.path == *path)
                        .expect("reachable import.meta source is authenticated")
                        .requests
                        .len()
                })
                .sum::<usize>();

            let root_file = IMPORT_META_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == root.path)
                .expect("every import.meta root is in the source table");
            let metadata = module_metadata(&root_file.metadata);
            assert!(metadata.is_module());
            assert_eq!(
                metadata.features.first().map(String::as_str),
                Some("import.meta")
            );
        }
        assert_eq!(union, file_paths);
        assert_eq!(rooted_request_count, 1);

        for file in &IMPORT_META_MODULE_FILE_ADMISSIONS {
            assert_eq!(file.source_sha256.len(), 64, "{}", file.path);
            assert!(
                file.source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}",
                file.path
            );
            if file.path.ends_with("_FIXTURE.js") {
                assert!(module_metadata_matches(
                    &Metadata::default(),
                    &file.metadata
                ));
            }
            let mut specifiers = BTreeSet::new();
            for request in &file.requests {
                assert!(specifiers.insert(request.specifier.as_str()));
                assert_eq!(
                    request.normalized_path,
                    normalized_audited_request(&file.path, &request.specifier)
                );
                assert!(file_paths.contains(request.normalized_path.as_str()));
            }
        }

        let negatives = IMPORT_META_MODULE_ROOT_ADMISSIONS
            .iter()
            .filter_map(|root| {
                IMPORT_META_MODULE_FILE_ADMISSIONS
                    .iter()
                    .find(|file| file.path == root.path)
                    .filter(|file| file.metadata.negative.is_some())
                    .map(|file| file.path.as_str())
            })
            .collect::<Vec<_>>();
        assert_eq!(negatives, IMPORT_META_MODULE_NEGATIVES);
        for negative in negatives {
            let contract = &IMPORT_META_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == negative)
                .expect("audited import.meta negative")
                .metadata;
            let expected = contract
                .negative
                .as_ref()
                .expect("negative metadata contract");
            assert_eq!(expected.phase, "parse", "{negative}");
            assert_eq!(expected.error_type, "SyntaxError", "{negative}");
        }

        for script in IMPORT_META_SCRIPT_ROOTS {
            assert!(
                exact_module_graph_admission(Path::new(script)).is_none(),
                "{script}"
            );
        }
        for excluded in IMPORT_META_ADJACENT_EXCLUSIONS {
            assert!(
                exact_module_graph_admission(Path::new(excluded)).is_none(),
                "adjacent import.meta surface was admitted: {excluded}"
            );
        }
    }

    #[test]
    fn import_meta_module_admission_matches_the_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let mut natural_roots = BTreeSet::new();
        collect_non_fixture_js(
            &suite.join("test/language/expressions/import.meta"),
            &suite,
            &mut natural_roots,
        );
        assert_eq!(natural_roots.len(), 22);

        let mut module_roots = BTreeSet::new();
        let mut script_roots = BTreeSet::new();
        for path in &natural_roots {
            let source = fs::read_to_string(suite.join(path))
                .unwrap_or_else(|error| panic!("read {path}: {error}"));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {path} metadata: {error}"));
            assert!(
                metadata
                    .features
                    .iter()
                    .any(|feature| feature == "import.meta")
            );
            if metadata.is_module() {
                module_roots.insert(path.to_owned());
            } else {
                script_roots.insert(path.to_owned());
            }
        }
        assert_eq!(module_roots.len(), 17);
        assert_eq!(script_roots.len(), 5);
        assert_eq!(
            module_roots,
            IMPORT_META_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.to_owned())
                .collect()
        );
        assert_eq!(
            script_roots,
            IMPORT_META_SCRIPT_ROOTS
                .into_iter()
                .map(str::to_owned)
                .collect()
        );

        for file in &IMPORT_META_MODULE_FILE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&file.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", file.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", file.path));
            authenticate_module_graph_file(Path::new(&file.path), &source, &metadata, file)
                .unwrap_or_else(|error| panic!("authenticate {}: {error}", file.path));
            assert_eq!(
                audited_module_specifiers(&source),
                file.requests
                    .iter()
                    .map(|request| request.specifier.to_owned())
                    .collect(),
                "{} static requests drifted",
                file.path
            );
        }

        for root in &IMPORT_META_MODULE_ROOT_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&root.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", root.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", root.path));
            assert_eq!(
                exact_module_test(&suite, Path::new(&root.path), &source, &metadata),
                Ok(Some(ExactModuleTest::FixtureGraph)),
                "{}",
                root.path
            );
        }

        for script in IMPORT_META_SCRIPT_ROOTS {
            let source = fs::read_to_string(suite.join(script))
                .unwrap_or_else(|error| panic!("read {script}: {error}"));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {script} metadata: {error}"));
            assert_eq!(
                exact_module_test(&suite, Path::new(script), &source, &metadata),
                Ok(None),
                "script-goal import.meta root was admitted as a module: {script}"
            );
        }

        let dynamic_import = IMPORT_META_ADJACENT_EXCLUSIONS[2];
        let source = fs::read_to_string(suite.join(dynamic_import))
            .unwrap_or_else(|error| panic!("read {dynamic_import}: {error}"));
        let metadata = parse_metadata(&source)
            .unwrap_or_else(|error| panic!("parse {dynamic_import} metadata: {error}"));
        assert!(metadata.is_module());
        assert!(metadata.is_async());
        assert_eq!(metadata.features, ["dynamic-import", "import.meta"]);
        assert_eq!(
            exact_module_test(&suite, Path::new(dynamic_import), &source, &metadata),
            Ok(None)
        );

        for assignment_target in &IMPORT_META_ADJACENT_EXCLUSIONS[..2] {
            let source = fs::read_to_string(suite.join(assignment_target))
                .unwrap_or_else(|error| panic!("read {assignment_target}: {error}"));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {assignment_target} metadata: {error}"));
            assert!(!metadata.is_module());
            assert!(metadata.features.is_empty());
            assert_eq!(
                metadata
                    .negative
                    .as_ref()
                    .and_then(|negative| negative.phase.as_deref()),
                Some("parse")
            );
            assert_eq!(
                metadata
                    .negative
                    .as_ref()
                    .and_then(|negative| negative.error_type.as_deref()),
                Some("SyntaxError")
            );
            assert_eq!(
                exact_module_test(&suite, Path::new(assignment_target), &source, &metadata,),
                Ok(None)
            );
        }
    }

    #[test]
    fn import_meta_module_admission_rejects_every_authenticated_dimension_drift() {
        let root = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path.ends_with("/import-meta-is-an-ordinary-object.js"))
            .expect("positive import.meta root");
        let exact = module_metadata(&root.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(&root.path),
                &root.source_sha256,
                &exact,
                root,
            ),
            Ok(())
        );
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&root.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &exact,
                root,
            )
            .unwrap_err()
            .contains("source drifted")
        );

        let mut feature_drift = exact;
        feature_drift.features.clear();
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&root.path),
                &root.source_sha256,
                &feature_drift,
                root,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );
        assert!(
            authenticate_module_graph_file_digest(
                Path::new("test/language/expressions/import.meta/unlisted.js"),
                &root.source_sha256,
                &module_metadata(&root.metadata),
                root,
            )
            .unwrap_err()
            .contains("path drifted")
        );

        let negative = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path.ends_with("/escape-sequence-import.js"))
            .expect("negative import.meta root");
        let mut negative_metadata = module_metadata(&negative.metadata);
        negative_metadata
            .negative
            .as_mut()
            .expect("negative metadata")
            .phase = Some("resolution".to_owned());
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&negative.path),
                &negative.source_sha256,
                &negative_metadata,
                negative,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        let fixture = IMPORT_META_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path.ends_with("_FIXTURE.js"))
            .expect("import.meta fixture");
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&fixture.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &Metadata::default(),
                fixture,
            )
            .unwrap_err()
            .contains("source drifted")
        );

        let distinct = &IMPORT_META_MODULE_ROOT_ADMISSIONS[0];
        let closure_drift = authenticate_exact_module_graph_closure(
            ExactModuleGraphAdmission {
                root_path: &distinct.path,
                files: &IMPORT_META_MODULE_FILE_ADMISSIONS,
                closure_file_count: 1,
            },
            |_| panic!("closure size drift must fail before reading source files"),
        )
        .unwrap_err();
        assert!(closure_drift.contains("closure size drifted"));

        let request = &IMPORT_META_MODULE_FILE_ADMISSIONS[0].requests[0];
        assert_eq!(
            normalize_exact_module_request(
                Path::new(&distinct.path),
                &distinct.path,
                &request.specifier,
            ),
            Ok(request.normalized_path.to_owned())
        );
        assert!(
            normalize_exact_module_request(
                Path::new(&distinct.path),
                &distinct.path,
                "./unlisted_FIXTURE.js",
            )
            .unwrap_err()
            .contains("unaudited request")
        );
        assert!(
            normalize_exact_module_request(
                Path::new(&distinct.path),
                "test/language/expressions/import.meta/unlisted.js",
                &request.specifier,
            )
            .unwrap_err()
            .contains("unaudited base")
        );

        for excluded in IMPORT_META_ADJACENT_EXCLUSIONS {
            assert!(exact_module_graph_admission(Path::new(excluded)).is_none());
        }
    }

    #[test]
    fn namespace_module_admission_is_the_exact_natural_closed_cohort() {
        assert_eq!(NAMESPACE_MODULE_ROOT_ADMISSIONS.len(), 37);
        assert_eq!(NAMESPACE_MODULE_FILE_ADMISSIONS.len(), 48);
        assert!(
            NAMESPACE_MODULE_ROOT_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert!(
            NAMESPACE_MODULE_FILE_ADMISSIONS
                .windows(2)
                .all(|pair| pair[0].path < pair[1].path)
        );
        assert_eq!(
            NAMESPACE_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.requests.len())
                .sum::<usize>(),
            46
        );

        let mut union = BTreeSet::new();
        for root in &NAMESPACE_MODULE_ROOT_ADMISSIONS {
            assert!(!root.path.ends_with("_FIXTURE.js"));
            let admission = ExactModuleGraphAdmission {
                root_path: &root.path,
                files: &NAMESPACE_MODULE_FILE_ADMISSIONS,
                closure_file_count: root.closure_file_count,
            };
            let reachable = reachable_module_graph_paths(admission)
                .unwrap_or_else(|error| panic!("{}: {error}", root.path));
            assert_eq!(reachable.len(), root.closure_file_count, "{}", root.path);
            union.extend(reachable);

            let root_file = NAMESPACE_MODULE_FILE_ADMISSIONS
                .iter()
                .find(|file| file.path == root.path)
                .expect("every namespace root is present in the file ledger");
            assert!(module_metadata(&root_file.metadata).is_module());
        }
        assert_eq!(union.len(), 48);
        assert_eq!(
            union,
            NAMESPACE_MODULE_FILE_ADMISSIONS
                .iter()
                .map(|file| file.path.as_str())
                .collect()
        );

        for file in &NAMESPACE_MODULE_FILE_ADMISSIONS {
            assert_eq!(file.source_sha256.len(), 64, "{}", file.path);
            assert!(
                file.source_sha256
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)),
                "{}",
                file.path
            );
            let mut specifiers = BTreeSet::new();
            for request in &file.requests {
                assert!(
                    specifiers.insert(request.specifier.as_str()),
                    "duplicate request {} in {}",
                    request.specifier,
                    file.path
                );
                assert_eq!(
                    request.normalized_path,
                    normalized_audited_request(&file.path, &request.specifier),
                    "{} -> {}",
                    file.path,
                    request.specifier
                );
                assert!(
                    NAMESPACE_MODULE_FILE_ADMISSIONS
                        .iter()
                        .any(|candidate| candidate.path == request.normalized_path),
                    "{} -> {}",
                    file.path,
                    request.normalized_path
                );
            }
        }
    }

    #[test]
    fn namespace_module_admission_matches_available_pinned_suite() {
        let suite = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/oracle/quickjs-2026-06-04/test262");
        if !suite.is_dir() {
            return;
        }

        let mut natural_roots = BTreeSet::new();
        collect_non_fixture_js(
            &suite.join("test/language/module-code/namespace"),
            &suite,
            &mut natural_roots,
        );
        natural_roots.insert(
            "test/language/module-code/ambiguous-export-bindings/omitted-from-namespace.js"
                .to_owned(),
        );
        assert_eq!(natural_roots.len(), 37);
        assert_eq!(
            natural_roots,
            NAMESPACE_MODULE_ROOT_ADMISSIONS
                .iter()
                .map(|root| root.path.to_owned())
                .collect()
        );

        for file in &NAMESPACE_MODULE_FILE_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&file.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", file.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", file.path));
            authenticate_module_graph_file(Path::new(&file.path), &source, &metadata, file)
                .unwrap_or_else(|error| panic!("authenticate {}: {error}", file.path));
            assert_eq!(
                audited_module_specifiers(&source),
                file.requests
                    .iter()
                    .map(|request| request.specifier.to_owned())
                    .collect(),
                "{} static requests drifted",
                file.path
            );
        }

        for root in &NAMESPACE_MODULE_ROOT_ADMISSIONS {
            let source = fs::read_to_string(suite.join(&root.path))
                .unwrap_or_else(|error| panic!("read {}: {error}", root.path));
            let metadata = parse_metadata(&source)
                .unwrap_or_else(|error| panic!("parse {} metadata: {error}", root.path));
            assert_eq!(
                exact_module_test(&suite, Path::new(&root.path), &source, &metadata),
                Ok(Some(ExactModuleTest::FixtureGraph)),
                "{}",
                root.path
            );
        }
    }

    #[test]
    fn namespace_module_admission_rejects_source_metadata_and_path_drift() {
        let file = NAMESPACE_MODULE_FILE_ADMISSIONS
            .iter()
            .find(|file| file.path == "test/language/module-code/namespace/Symbol.iterator.js")
            .expect("audited namespace root");
        let exact = module_metadata(&file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                &file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );

        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                "0000000000000000000000000000000000000000000000000000000000000000",
                &exact,
                file,
            )
            .unwrap_err()
            .contains("source drifted")
        );

        let mut metadata_drift = exact;
        metadata_drift.features.push("Symbol".to_owned());
        assert!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                &file.source_sha256,
                &metadata_drift,
                file,
            )
            .unwrap_err()
            .contains("metadata shape drifted")
        );

        assert!(
            authenticate_module_graph_file_digest(
                Path::new("test/language/module-code/namespace/unlisted.js"),
                &file.source_sha256,
                &module_metadata(&file.metadata),
                file,
            )
            .unwrap_err()
            .contains("path drifted")
        );
    }

    #[test]
    fn module_graph_admission_rejects_request_and_closure_drift() {
        let module_metadata = ModuleMetadataContract {
            flags: vec!["module".to_owned()],
            ..ModuleMetadataContract::default()
        };
        let missing_request_files = vec![ModuleGraphFileAdmission {
            path: "test/root.js".to_owned(),
            source_sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            metadata: module_metadata.clone(),
            requests: Vec::new(),
        }];
        let missing_request = super::authenticate_exact_module_graph_closure(
            ExactModuleGraphAdmission {
                root_path: "test/root.js",
                files: &missing_request_files,
                closure_file_count: 2,
            },
            |_| panic!("closure drift must fail before reading sources"),
        )
        .unwrap_err();
        assert!(missing_request.contains("closure size drifted"));

        let escaped_request_files = vec![ModuleGraphFileAdmission {
            path: "test/root.js".to_owned(),
            source_sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            metadata: module_metadata,
            requests: vec![ModuleRequestAdmission {
                specifier: "./escaped.js".to_owned(),
                normalized_path: "test/escaped.js".to_owned(),
            }],
        }];
        let escaped_request = reachable_module_graph_paths(ExactModuleGraphAdmission {
            root_path: "test/root.js",
            files: &escaped_request_files,
            closure_file_count: 1,
        })
        .unwrap_err();
        assert!(escaped_request.contains("request escaped"));
        assert!(escaped_request.contains("./escaped.js"));
    }

    #[test]
    fn fixture_graph_file_authentication_rejects_source_metadata_and_path_drift() {
        let file = &FIXTURE_GRAPH_MODULE_ADMISSIONS[0].files[1];
        let exact = module_metadata(&file.metadata);
        assert_eq!(
            authenticate_module_graph_file_digest(
                Path::new(&file.path),
                &file.source_sha256,
                &exact,
                file,
            ),
            Ok(())
        );

        let source_drift = authenticate_module_graph_file_digest(
            Path::new(&file.path),
            "0000000000000000000000000000000000000000000000000000000000000000",
            &exact,
            file,
        )
        .unwrap_err();
        assert!(source_drift.contains("source drifted"));
        assert!(source_drift.contains(&file.source_sha256));

        let mut drifted_metadata = exact;
        drifted_metadata.flags.insert("module".to_owned());
        let metadata_drift = authenticate_module_graph_file_digest(
            Path::new(&file.path),
            &file.source_sha256,
            &drifted_metadata,
            file,
        )
        .unwrap_err();
        assert!(metadata_drift.contains("metadata shape drifted"));

        let path_drift = authenticate_module_graph_file_digest(
            Path::new("test/language/module-code/other_FIXTURE.js"),
            &file.source_sha256,
            &module_metadata(&file.metadata),
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
        let files = vec![
            ModuleGraphFileAdmission {
                path: "test/root.js".to_owned(),
                source_sha256: "32d8e8b1d38a53f8f4873d89cd0d00a115c33b0ed8294eb016e22e3edea95afe"
                    .to_owned(),
                metadata: ModuleMetadataContract {
                    flags: vec!["module".to_owned()],
                    ..ModuleMetadataContract::default()
                },
                requests: vec![ModuleRequestAdmission {
                    specifier: "./fixture_FIXTURE.js".to_owned(),
                    normalized_path: "test/fixture_FIXTURE.js".to_owned(),
                }],
            },
            ModuleGraphFileAdmission {
                path: "test/fixture_FIXTURE.js".to_owned(),
                source_sha256: "5d8f65d2774e206bc9f7a7a4ad39ca2dc563b5c31e46ab57ef4874961237ce29"
                    .to_owned(),
                metadata: ModuleMetadataContract::default(),
                requests: Vec::new(),
            },
        ];
        let admission = ExactModuleGraphAdmission {
            root_path: "test/root.js",
            files: &files,
            closure_file_count: 2,
        };

        let exact = authenticate_exact_module_graph_closure(admission, |path| match path {
            "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
            "test/fixture_FIXTURE.js" => Ok(FIXTURE_SOURCE.to_owned()),
            _ => Err(format!("unexpected path: {path}")),
        });
        assert_eq!(exact, Ok(()));

        let drift = authenticate_exact_module_graph_closure(admission, |path| match path {
            "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
            "test/fixture_FIXTURE.js" => Ok("export const value = 2;\n".to_owned()),
            _ => Err(format!("unexpected path: {path}")),
        })
        .unwrap_err();
        assert!(drift.contains("source drifted"));
        assert!(drift.contains("fixture_FIXTURE.js"));
    }

    #[test]
    fn json_fixture_graph_authenticates_and_loads_unparsed_raw_text() {
        const ROOT_SOURCE: &str = "/*---\nflags: [module]\n---*/\nimport data from \"./data_FIXTURE.json\" with { type: \"json\" };\n";
        const JSON_SOURCE: &str = "{\"note\":\"/*--- raw JSON, not Test262 metadata\"}\n";

        let root_sha256 = source_sha256(ROOT_SOURCE).unwrap();
        let json_sha256 = source_sha256(JSON_SOURCE).unwrap();
        let files = vec![
            ModuleGraphFileAdmission {
                path: "test/root.js".to_owned(),
                source_sha256: root_sha256,
                metadata: ModuleMetadataContract {
                    flags: vec!["module".to_owned()],
                    ..ModuleMetadataContract::default()
                },
                requests: vec![ModuleRequestAdmission {
                    specifier: "./data_FIXTURE.json".to_owned(),
                    normalized_path: "test/data_FIXTURE.json".to_owned(),
                }],
            },
            ModuleGraphFileAdmission {
                path: "test/data_FIXTURE.json".to_owned(),
                source_sha256: json_sha256,
                metadata: ModuleMetadataContract::default(),
                requests: Vec::new(),
            },
        ];
        let admission = ExactModuleGraphAdmission {
            root_path: "test/root.js",
            files: &files,
            closure_file_count: 2,
        };

        assert_eq!(
            authenticate_exact_module_graph_closure(admission, |path| match path {
                "test/root.js" => Ok(ROOT_SOURCE.to_owned()),
                "test/data_FIXTURE.json" => Ok(JSON_SOURCE.to_owned()),
                _ => Err(format!("unexpected path: {path}")),
            }),
            Ok(())
        );

        let suite = std::env::temp_dir().join(format!(
            "quickjs-oxide-json-graph-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(suite.join("test")).unwrap();
        fs::write(suite.join("test/data_FIXTURE.json"), JSON_SOURCE).unwrap();

        let loaded =
            load_exact_module_fixture_from_admission(admission, &suite, "test/data_FIXTURE.json")
                .unwrap();
        assert_eq!(loaded, JSON_SOURCE);

        fs::write(
            suite.join("test/data_FIXTURE.json"),
            "{\"note\":\"drifted\"}\n",
        )
        .unwrap();
        let error =
            load_exact_module_fixture_from_admission(admission, &suite, "test/data_FIXTURE.json")
                .unwrap_err();
        assert!(error.contains("source drifted"), "{error}");
        fs::remove_dir_all(suite).unwrap();
    }

    #[test]
    fn json_fixture_graph_rejects_nonempty_metadata_contracts_defensively() {
        let file = ModuleGraphFileAdmission {
            path: "test/data_FIXTURE.json".to_owned(),
            source_sha256: "0000000000000000000000000000000000000000000000000000000000000000"
                .to_owned(),
            metadata: ModuleMetadataContract {
                flags: vec!["module".to_owned()],
                ..ModuleMetadataContract::default()
            },
            requests: Vec::new(),
        };
        let error = authenticate_module_graph_file_digest(
            Path::new(&file.path),
            &file.source_sha256,
            &module_metadata(&file.metadata),
            &file,
        )
        .unwrap_err();
        assert!(error.contains("metadata must be empty"), "{error}");
    }

    #[test]
    fn fixture_graph_loader_normalization_rejects_unlisted_edges() {
        let admission = &FIXTURE_GRAPH_MODULE_ADMISSIONS[0];
        let base = admission.root_path.as_str();
        let request = &admission.files[0].requests[0];
        assert_eq!(
            normalize_exact_module_request(
                Path::new(&admission.root_path),
                base,
                &request.specifier,
            ),
            Ok(request.normalized_path.to_owned())
        );
        assert!(
            normalize_exact_module_request(
                Path::new(&admission.root_path),
                base,
                "./unlisted_FIXTURE.js",
            )
            .unwrap_err()
            .contains("unaudited request")
        );
        assert!(
            normalize_exact_module_request(
                Path::new(&admission.root_path),
                "test/language/module-code/unlisted.js",
                &request.specifier,
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
                Path::new(&DEPENDENCY_FREE_MODULE_ADMISSIONS[0].path),
                "drifted",
                &module_metadata(&DEPENDENCY_FREE_MODULE_ADMISSIONS[0].metadata),
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
            feature_shapes.insert(admission.features.clone());
            let exact = agent_metadata(admission);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(&admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(&admission.source_sha256));

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
            feature_shapes.insert(admission.features.clone());
            let exact = agent_metadata(admission);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(&admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(&admission.source_sha256));

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
            feature_shapes.insert(admission.features.clone());
            let exact = agent_metadata(admission);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(&admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(&admission.source_sha256));

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
            feature_shapes.insert(admission.features.clone());
            let exact = agent_metadata(admission);
            assert!(agent_host_metadata_matches(&exact, admission));

            let source_drift =
                is_exact_agent_host_test(Path::new(&admission.path), "/* source drift */", &exact)
                    .unwrap_err();
            assert!(source_drift.contains(&admission.source_sha256));

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
        let admission = SupplementalAdmission {
            path: PATH.to_owned(),
            source_sha256: "3cb79dbb8554f721f371c78cad9fe21234dc9b249f27e15e372abacdd014cb47"
                .to_owned(),
            features: vec!["Atomics".to_owned(), "SharedArrayBuffer".to_owned()],
            policy: SupplementalPolicy::AtomicsCrossRealm,
        };

        let mut hints = BTreeSet::from(["host-create-realm-required".to_owned()]);
        insert_atomics_cross_realm_feature_hints(
            &mut hints,
            &source_tokens(SOURCE, false),
            &admission,
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
                &source_tokens(shape_drift, false),
                &admission,
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

        let admission = SupplementalAdmission {
            path: PATH.to_owned(),
            source_sha256: SOURCE_SHA256.to_owned(),
            features: vec!["Atomics".to_owned()],
            policy: SupplementalPolicy::ExactFeatures,
        };
        assert_eq!(
            authenticate_supplemental_source(Path::new(PATH), SOURCE, &admission),
            Ok(())
        );
        assert!(authenticate_supplemental_source(Path::new(PATH), "abd", &admission).is_err());
        assert_eq!(
            supplemental_feature_hints(Path::new("test/example.js"), SOURCE).unwrap(),
            Vec::<String>::new()
        );
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
