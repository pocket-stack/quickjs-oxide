use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

const HEADER: &str = "kind\tgroup\tpath\tsource_sha256\tincludes\tflags\tfeatures\tnegative_phase\tnegative_type\tclosure_file_count\tpriority\trequest_index\tspecifier\tnormalized_path\tpolicy\tcohort";
const EMPTY_FIELD: &str = "-";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct NegativeMetadataContract {
    pub(super) phase: String,
    pub(super) error_type: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct ModuleMetadataContract {
    pub(super) includes: Vec<String>,
    pub(super) flags: Vec<String>,
    pub(super) features: Vec<String>,
    pub(super) negative: Option<NegativeMetadataContract>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModuleAdmission {
    pub(super) group: String,
    pub(super) path: String,
    pub(super) source_sha256: String,
    pub(super) metadata: ModuleMetadataContract,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModuleRequestAdmission {
    pub(super) specifier: String,
    pub(super) normalized_path: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModuleGraphFileAdmission {
    pub(super) path: String,
    pub(super) source_sha256: String,
    pub(super) metadata: ModuleMetadataContract,
    pub(super) requests: Vec<ModuleRequestAdmission>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct ModuleGraphRootAdmission {
    pub(super) group: String,
    pub(super) path: String,
    pub(super) closure_file_count: usize,
    pub(super) priority: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct AgentHostAdmission {
    pub(super) path: String,
    pub(super) source_sha256: String,
    pub(super) features: Vec<String>,
    pub(super) cohort: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SupplementalPolicy {
    AtomicsCrossRealm,
    ExactFeatures,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SupplementalAdmission {
    pub(super) path: String,
    pub(super) source_sha256: String,
    pub(super) features: Vec<String>,
    pub(super) policy: SupplementalPolicy,
}

#[derive(Clone, Debug, Default)]
pub(super) struct AdmissionCatalog {
    modules: BTreeMap<String, ModuleAdmission>,
    graph_roots: Vec<ModuleGraphRootAdmission>,
    graph_files: BTreeMap<String, Vec<ModuleGraphFileAdmission>>,
    agent_hosts: BTreeMap<String, AgentHostAdmission>,
    supplemental: BTreeMap<String, SupplementalAdmission>,
}

struct PendingRequest {
    group: String,
    path: String,
    index: usize,
    specifier: String,
    normalized_path: String,
}

impl AdmissionCatalog {
    pub(super) fn load(path: &Path, expected_sha256: &str) -> Result<Self, String> {
        let file_type = fs::symlink_metadata(path)
            .map_err(|error| format!("stat Test262 admissions {}: {error}", path.display()))?
            .file_type();
        if !file_type.is_file() || file_type.is_symlink() {
            return Err(format!(
                "Test262 admissions must be a regular non-symlink file: {}",
                path.display()
            ));
        }
        let bytes = fs::read(path)
            .map_err(|error| format!("read Test262 admissions {}: {error}", path.display()))?;
        let actual_sha256 = sha256(&bytes);
        if actual_sha256 != expected_sha256 {
            return Err(format!(
                "Test262 admissions checksum mismatch: expected {expected_sha256}, found {actual_sha256}"
            ));
        }
        let source = String::from_utf8(bytes).map_err(|error| {
            format!(
                "Test262 admissions are not valid UTF-8 at byte {}: {}",
                error.utf8_error().valid_up_to(),
                path.display()
            )
        })?;
        Self::parse(&source)
            .map_err(|error| format!("parse Test262 admissions {}: {error}", path.display()))
    }

    pub(super) fn parse(source: &str) -> Result<Self, String> {
        if source.contains('\r') {
            return Err("admissions must use LF line endings".to_owned());
        }
        if !source.ends_with('\n') {
            return Err("admissions must end with a newline".to_owned());
        }
        let mut lines = source.split_terminator('\n');
        if lines.next() != Some(HEADER) {
            return Err("admissions header does not match schema".to_owned());
        }

        let mut catalog = Self::default();
        let mut pending_requests = Vec::new();
        let mut root_keys = BTreeSet::new();
        let mut previous_line: Option<&str> = None;
        for (index, line) in lines.enumerate() {
            let line_number = index + 2;
            if line.is_empty() {
                return Err(format!("admissions line {line_number} is empty"));
            }
            if previous_line.is_some_and(|previous| previous >= line) {
                return Err(format!(
                    "admissions line {line_number} is not in strict bytewise order"
                ));
            }
            previous_line = Some(line);
            if line
                .chars()
                .any(|character| character.is_control() && character != '\t')
            {
                return Err(format!(
                    "admissions line {line_number} contains a control character"
                ));
            }
            let raw_fields = line.split('\t').collect::<Vec<_>>();
            if raw_fields.len() != 16 {
                return Err(format!(
                    "admissions line {line_number} has {} fields instead of 16",
                    raw_fields.len()
                ));
            }
            if raw_fields.iter().any(|field| field.is_empty()) {
                return Err(format!(
                    "admissions line {line_number} has an empty field instead of {EMPTY_FIELD:?}"
                ));
            }
            let fields = raw_fields
                .iter()
                .map(|field| if *field == EMPTY_FIELD { "" } else { *field })
                .collect::<Vec<_>>();
            validate_group(fields[1], line_number)?;
            match fields[0] {
                "module" => {
                    require_empty(&fields, &[9, 10, 11, 12, 13, 14, 15], line_number)?;
                    validate_test_path(fields[2], false, line_number)?;
                    validate_sha256(fields[3], "source_sha256", line_number)?;
                    let admission = ModuleAdmission {
                        group: fields[1].to_owned(),
                        path: fields[2].to_owned(),
                        source_sha256: fields[3].to_owned(),
                        metadata: parse_metadata_contract(&fields, line_number)?,
                    };
                    if catalog
                        .modules
                        .insert(admission.path.clone(), admission)
                        .is_some()
                    {
                        return Err(format!(
                            "admissions line {line_number} duplicates module path {}",
                            fields[2]
                        ));
                    }
                }
                "graph-root" => {
                    require_empty(
                        &fields,
                        &[3, 4, 5, 6, 7, 8, 11, 12, 13, 14, 15],
                        line_number,
                    )?;
                    validate_test_path(fields[2], false, line_number)?;
                    let closure_file_count =
                        parse_usize(fields[9], "closure_file_count", false, line_number)?;
                    let priority = parse_usize(fields[10], "priority", true, line_number)?;
                    let key = (fields[1].to_owned(), fields[2].to_owned());
                    if !root_keys.insert(key) {
                        return Err(format!(
                            "admissions line {line_number} duplicates graph root {}/{}",
                            fields[1], fields[2]
                        ));
                    }
                    catalog.graph_roots.push(ModuleGraphRootAdmission {
                        group: fields[1].to_owned(),
                        path: fields[2].to_owned(),
                        closure_file_count,
                        priority,
                    });
                }
                "graph-file" => {
                    require_empty(&fields, &[9, 10, 11, 12, 13, 14, 15], line_number)?;
                    validate_test_path(fields[2], true, line_number)?;
                    validate_sha256(fields[3], "source_sha256", line_number)?;
                    let admission = ModuleGraphFileAdmission {
                        path: fields[2].to_owned(),
                        source_sha256: fields[3].to_owned(),
                        metadata: parse_metadata_contract(&fields, line_number)?,
                        requests: Vec::new(),
                    };
                    let files = catalog.graph_files.entry(fields[1].to_owned()).or_default();
                    if files.iter().any(|file| file.path == admission.path) {
                        return Err(format!(
                            "admissions line {line_number} duplicates graph file {}/{}",
                            fields[1], fields[2]
                        ));
                    }
                    files.push(admission);
                }
                "graph-request" => {
                    require_empty(&fields, &[3, 4, 5, 6, 7, 8, 9, 10, 14, 15], line_number)?;
                    validate_test_path(fields[2], true, line_number)?;
                    let request_index =
                        parse_usize(fields[11], "request_index", true, line_number)?;
                    validate_specifier(fields[12], line_number)?;
                    validate_test_path(fields[13], true, line_number)?;
                    pending_requests.push(PendingRequest {
                        group: fields[1].to_owned(),
                        path: fields[2].to_owned(),
                        index: request_index,
                        specifier: fields[12].to_owned(),
                        normalized_path: fields[13].to_owned(),
                    });
                }
                "agent" => {
                    require_empty(&fields, &[4, 5, 7, 8, 9, 10, 11, 12, 13, 14], line_number)?;
                    validate_test_path(fields[2], false, line_number)?;
                    validate_sha256(fields[3], "source_sha256", line_number)?;
                    let features = parse_list(fields[6], "features", false, line_number)?;
                    validate_nonempty(fields[15], "cohort", line_number)?;
                    let admission = AgentHostAdmission {
                        path: fields[2].to_owned(),
                        source_sha256: fields[3].to_owned(),
                        features,
                        cohort: fields[15].to_owned(),
                    };
                    if catalog
                        .agent_hosts
                        .insert(admission.path.clone(), admission)
                        .is_some()
                    {
                        return Err(format!(
                            "admissions line {line_number} duplicates agent path {}",
                            fields[2]
                        ));
                    }
                }
                "supplemental" => {
                    require_empty(&fields, &[4, 5, 7, 8, 9, 10, 11, 12, 13, 15], line_number)?;
                    validate_test_path(fields[2], false, line_number)?;
                    validate_sha256(fields[3], "source_sha256", line_number)?;
                    let features = parse_list(fields[6], "features", false, line_number)?;
                    let policy = match fields[14] {
                        "atomics-cross-realm" => SupplementalPolicy::AtomicsCrossRealm,
                        "exact-features" => SupplementalPolicy::ExactFeatures,
                        unknown => {
                            return Err(format!(
                                "admissions line {line_number} has unknown supplemental policy {unknown:?}"
                            ));
                        }
                    };
                    let admission = SupplementalAdmission {
                        path: fields[2].to_owned(),
                        source_sha256: fields[3].to_owned(),
                        features,
                        policy,
                    };
                    if catalog
                        .supplemental
                        .insert(admission.path.clone(), admission)
                        .is_some()
                    {
                        return Err(format!(
                            "admissions line {line_number} duplicates supplemental path {}",
                            fields[2]
                        ));
                    }
                }
                unknown => {
                    return Err(format!(
                        "admissions line {line_number} has unknown kind {unknown:?}"
                    ));
                }
            }
        }

        pending_requests.sort_by(|left, right| {
            (&left.group, &left.path, left.index).cmp(&(&right.group, &right.path, right.index))
        });
        let mut previous_request: Option<(String, String, usize)> = None;
        for request in pending_requests {
            if previous_request.as_ref().is_some_and(|previous| {
                previous.0 == request.group
                    && previous.1 == request.path
                    && previous.2 == request.index
            }) {
                return Err(format!(
                    "duplicate graph request index {}/{}/{}",
                    request.group, request.path, request.index
                ));
            }
            previous_request = Some((request.group.clone(), request.path.clone(), request.index));
            let group_files = catalog.graph_files.get_mut(&request.group).ok_or_else(|| {
                format!(
                    "graph request references unknown group {}/{}",
                    request.group, request.path
                )
            })?;
            if !group_files
                .iter()
                .any(|file| file.path == request.normalized_path)
            {
                return Err(format!(
                    "graph request escapes admission group {}: {} -> {}",
                    request.group, request.specifier, request.normalized_path
                ));
            }
            let file = group_files
                .iter_mut()
                .find(|file| file.path == request.path)
                .ok_or_else(|| {
                    format!(
                        "graph request references unknown source {}/{}",
                        request.group, request.path
                    )
                })?;
            if request.index != file.requests.len() {
                return Err(format!(
                    "graph request indexes are not contiguous for {}/{}: expected {}, found {}",
                    request.group,
                    request.path,
                    file.requests.len(),
                    request.index
                ));
            }
            file.requests.push(ModuleRequestAdmission {
                specifier: request.specifier,
                normalized_path: request.normalized_path,
            });
        }

        catalog.validate_graphs()?;
        Ok(catalog)
    }

    fn validate_graphs(&self) -> Result<(), String> {
        let mut priorities = BTreeSet::new();
        let mut covered = BTreeMap::<&str, BTreeSet<&str>>::new();
        for root in &self.graph_roots {
            if !priorities.insert((&root.path, root.priority)) {
                return Err(format!(
                    "ambiguous graph root priority for {}: {}",
                    root.path, root.priority
                ));
            }
            let files = self.graph_files.get(&root.group).ok_or_else(|| {
                format!(
                    "graph root references unknown group: {}/{}",
                    root.group, root.path
                )
            })?;
            if !files.iter().any(|file| file.path == root.path) {
                return Err(format!(
                    "graph root is absent from its file ledger: {}/{}",
                    root.group, root.path
                ));
            }
            let visited = reachable_paths(files, &root.path)?;
            if visited.len() != root.closure_file_count {
                return Err(format!(
                    "graph closure size drifted for {}: expected {}, found {}",
                    root.path,
                    root.closure_file_count,
                    visited.len()
                ));
            }
            covered.entry(&root.group).or_default().extend(visited);
        }
        for (group, files) in &self.graph_files {
            let Some(group_covered) = covered.get(group.as_str()) else {
                return Err(format!("graph file group has no roots: {group}"));
            };
            let file_paths = files
                .iter()
                .map(|file| file.path.as_str())
                .collect::<BTreeSet<_>>();
            if *group_covered != file_paths {
                let uncovered = file_paths
                    .difference(group_covered)
                    .copied()
                    .collect::<Vec<_>>();
                return Err(format!(
                    "graph file group has unreachable files: {group}: {}",
                    uncovered.join(", ")
                ));
            }
        }
        Ok(())
    }

    pub(super) fn module(&self, path: &Path) -> Option<&ModuleAdmission> {
        self.modules.get(path.to_str()?)
    }

    #[cfg(test)]
    pub(super) fn modules(&self) -> impl Iterator<Item = &ModuleAdmission> {
        self.modules.values()
    }

    #[cfg(test)]
    pub(super) fn modules_in_group<'a>(
        &'a self,
        group: &'a str,
    ) -> impl Iterator<Item = &'a ModuleAdmission> {
        self.modules
            .values()
            .filter(move |admission| admission.group == group)
    }

    pub(super) fn graph_root(&self, path: &Path) -> Option<&ModuleGraphRootAdmission> {
        let path = path.to_str()?;
        self.graph_roots
            .iter()
            .filter(|root| root.path == path)
            .min_by_key(|root| root.priority)
    }

    #[cfg(test)]
    pub(super) fn graph_roots(&self) -> impl Iterator<Item = &ModuleGraphRootAdmission> {
        self.graph_roots.iter()
    }

    #[cfg(test)]
    pub(super) fn graph_roots_in_group<'a>(
        &'a self,
        group: &'a str,
    ) -> impl Iterator<Item = &'a ModuleGraphRootAdmission> {
        self.graph_roots
            .iter()
            .filter(move |admission| admission.group == group)
    }

    pub(super) fn graph_files(&self, group: &str) -> &[ModuleGraphFileAdmission] {
        self.graph_files.get(group).map_or(&[], Vec::as_slice)
    }

    pub(super) fn agent_host(&self, path: &Path) -> Option<&AgentHostAdmission> {
        self.agent_hosts.get(path.to_str()?)
    }

    #[cfg(test)]
    pub(super) fn agent_hosts(&self) -> impl Iterator<Item = &AgentHostAdmission> {
        self.agent_hosts.values()
    }

    pub(super) fn supplemental(&self, path: &Path) -> Option<&SupplementalAdmission> {
        self.supplemental.get(path.to_str()?)
    }

    #[cfg(test)]
    pub(super) fn supplemental_admissions(&self) -> impl Iterator<Item = &SupplementalAdmission> {
        self.supplemental.values()
    }
}

fn sha256(input: &[u8]) -> String {
    const ROUND_CONSTANTS: [u32; 64] = [
        0x428a_2f98,
        0x7137_4491,
        0xb5c0_fbcf,
        0xe9b5_dba5,
        0x3956_c25b,
        0x59f1_11f1,
        0x923f_82a4,
        0xab1c_5ed5,
        0xd807_aa98,
        0x1283_5b01,
        0x2431_85be,
        0x550c_7dc3,
        0x72be_5d74,
        0x80de_b1fe,
        0x9bdc_06a7,
        0xc19b_f174,
        0xe49b_69c1,
        0xefbe_4786,
        0x0fc1_9dc6,
        0x240c_a1cc,
        0x2de9_2c6f,
        0x4a74_84aa,
        0x5cb0_a9dc,
        0x76f9_88da,
        0x983e_5152,
        0xa831_c66d,
        0xb003_27c8,
        0xbf59_7fc7,
        0xc6e0_0bf3,
        0xd5a7_9147,
        0x06ca_6351,
        0x1429_2967,
        0x27b7_0a85,
        0x2e1b_2138,
        0x4d2c_6dfc,
        0x5338_0d13,
        0x650a_7354,
        0x766a_0abb,
        0x81c2_c92e,
        0x9272_2c85,
        0xa2bf_e8a1,
        0xa81a_664b,
        0xc24b_8b70,
        0xc76c_51a3,
        0xd192_e819,
        0xd699_0624,
        0xf40e_3585,
        0x106a_a070,
        0x19a4_c116,
        0x1e37_6c08,
        0x2748_774c,
        0x34b0_bcb5,
        0x391c_0cb3,
        0x4ed8_aa4a,
        0x5b9c_ca4f,
        0x682e_6ff3,
        0x748f_82ee,
        0x78a5_636f,
        0x84c8_7814,
        0x8cc7_0208,
        0x90be_fffa,
        0xa450_6ceb,
        0xbef9_a3f7,
        0xc671_78f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let bit_length = u64::try_from(input.len())
        .expect("admission file length fits u64")
        .checked_mul(8)
        .expect("admission file bit length fits u64");
    let mut padded = Vec::with_capacity(input.len().saturating_add(72));
    padded.extend_from_slice(input);
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());

    for chunk in padded.chunks_exact(64) {
        let mut schedule = [0u32; 64];
        for (word, bytes) in schedule[..16].iter_mut().zip(chunk.chunks_exact(4)) {
            *word = u32::from_be_bytes(bytes.try_into().expect("SHA-256 word has four bytes"));
        }
        for index in 16..64 {
            let s0 = schedule[index - 15].rotate_right(7)
                ^ schedule[index - 15].rotate_right(18)
                ^ (schedule[index - 15] >> 3);
            let s1 = schedule[index - 2].rotate_right(17)
                ^ schedule[index - 2].rotate_right(19)
                ^ (schedule[index - 2] >> 10);
            schedule[index] = schedule[index - 16]
                .wrapping_add(s0)
                .wrapping_add(schedule[index - 7])
                .wrapping_add(s1);
        }

        let mut working = state;
        for index in 0..64 {
            let choice = (working[4] & working[5]) ^ (!working[4] & working[6]);
            let majority =
                (working[0] & working[1]) ^ (working[0] & working[2]) ^ (working[1] & working[2]);
            let upper_a = working[0].rotate_right(2)
                ^ working[0].rotate_right(13)
                ^ working[0].rotate_right(22);
            let upper_e = working[4].rotate_right(6)
                ^ working[4].rotate_right(11)
                ^ working[4].rotate_right(25);
            let temporary_one = working[7]
                .wrapping_add(upper_e)
                .wrapping_add(choice)
                .wrapping_add(ROUND_CONSTANTS[index])
                .wrapping_add(schedule[index]);
            let temporary_two = upper_a.wrapping_add(majority);
            working = [
                temporary_one.wrapping_add(temporary_two),
                working[0],
                working[1],
                working[2],
                working[3].wrapping_add(temporary_one),
                working[4],
                working[5],
                working[6],
            ];
        }
        for (state_word, working_word) in state.iter_mut().zip(working) {
            *state_word = state_word.wrapping_add(working_word);
        }
    }

    state.iter().map(|word| format!("{word:08x}")).collect()
}

fn reachable_paths<'a>(
    files: &'a [ModuleGraphFileAdmission],
    root: &str,
) -> Result<BTreeSet<&'a str>, String> {
    let mut visited = BTreeSet::new();
    let mut pending = vec![root];
    while let Some(path) = pending.pop() {
        let Some(file) = files.iter().find(|file| file.path == path) else {
            return Err(format!("graph edge escaped authenticated files: {path}"));
        };
        if !visited.insert(file.path.as_str()) {
            continue;
        }
        for request in file.requests.iter().rev() {
            if !files
                .iter()
                .any(|file| file.path == request.normalized_path)
            {
                return Err(format!(
                    "graph request escaped authenticated files: {} -> {}",
                    request.specifier, request.normalized_path
                ));
            }
            pending.push(&request.normalized_path);
        }
    }
    Ok(visited)
}

fn parse_metadata_contract(
    fields: &[&str],
    line_number: usize,
) -> Result<ModuleMetadataContract, String> {
    let includes = parse_list(fields[4], "includes", true, line_number)?;
    let flags = parse_list(fields[5], "flags", true, line_number)?;
    let features = parse_list(fields[6], "features", true, line_number)?;
    let negative = match (fields[7], fields[8]) {
        ("", "") => None,
        (phase, error_type) if !phase.is_empty() && !error_type.is_empty() => {
            validate_scalar(phase, "negative_phase", line_number)?;
            validate_scalar(error_type, "negative_type", line_number)?;
            Some(NegativeMetadataContract {
                phase: phase.to_owned(),
                error_type: error_type.to_owned(),
            })
        }
        _ => {
            return Err(format!(
                "admissions line {line_number} must provide both negative phase and type"
            ));
        }
    };
    Ok(ModuleMetadataContract {
        includes,
        flags,
        features,
        negative,
    })
}

fn parse_list(
    value: &str,
    name: &str,
    allow_empty: bool,
    line_number: usize,
) -> Result<Vec<String>, String> {
    if value.is_empty() {
        return if allow_empty {
            Ok(Vec::new())
        } else {
            Err(format!(
                "admissions line {line_number} requires non-empty {name}"
            ))
        };
    }
    let mut seen = BTreeSet::new();
    let mut output = Vec::new();
    for item in value.split(',') {
        validate_scalar(item, name, line_number)?;
        if !seen.insert(item) {
            return Err(format!(
                "admissions line {line_number} has duplicate {name} item {item:?}"
            ));
        }
        output.push(item.to_owned());
    }
    Ok(output)
}

fn validate_group(value: &str, line_number: usize) -> Result<(), String> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(format!(
            "admissions line {line_number} has invalid group {value:?}"
        ));
    }
    Ok(())
}

fn validate_test_path(value: &str, allow_fixture: bool, line_number: usize) -> Result<(), String> {
    if !value.starts_with("test/")
        || !value.ends_with(".js")
        || value.contains('\\')
        || value.contains("//")
        || value
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
        || (!allow_fixture && value.ends_with("_FIXTURE.js"))
    {
        return Err(format!(
            "admissions line {line_number} has invalid Test262 path {value:?}"
        ));
    }
    Ok(())
}

fn validate_specifier(value: &str, line_number: usize) -> Result<(), String> {
    let Some(relative) = value.strip_prefix("./") else {
        return Err(format!(
            "admissions line {line_number} has non-relative module specifier {value:?}"
        ));
    };
    if relative.is_empty()
        || relative.contains('\\')
        || relative
            .split('/')
            .any(|component| matches!(component, "" | "." | ".."))
    {
        return Err(format!(
            "admissions line {line_number} has unsafe module specifier {value:?}"
        ));
    }
    Ok(())
}

fn validate_sha256(value: &str, name: &str, line_number: usize) -> Result<(), String> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(format!("admissions line {line_number} has invalid {name}"));
    }
    Ok(())
}

fn validate_nonempty(value: &str, name: &str, line_number: usize) -> Result<(), String> {
    if value.is_empty() || value.trim() != value {
        return Err(format!("admissions line {line_number} has invalid {name}"));
    }
    Ok(())
}

fn validate_scalar(value: &str, name: &str, line_number: usize) -> Result<(), String> {
    if value.is_empty() || value.trim() != value || value.contains(',') {
        return Err(format!(
            "admissions line {line_number} has invalid {name} value {value:?}"
        ));
    }
    Ok(())
}

fn parse_usize(
    value: &str,
    name: &str,
    allow_zero: bool,
    line_number: usize,
) -> Result<usize, String> {
    if value.is_empty()
        || (value.len() > 1 && value.starts_with('0'))
        || !value.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(format!(
            "admissions line {line_number} has non-canonical {name}"
        ));
    }
    let value = value
        .parse::<usize>()
        .map_err(|_| format!("admissions line {line_number} has overflowing {name}"))?;
    if !allow_zero && value == 0 {
        return Err(format!(
            "admissions line {line_number} requires positive {name}"
        ));
    }
    Ok(value)
}

fn require_empty(fields: &[&str], indexes: &[usize], line_number: usize) -> Result<(), String> {
    if let Some(index) = indexes.iter().find(|index| !fields[**index].is_empty()) {
        return Err(format!(
            "admissions line {line_number} has unexpected data in field {}",
            HEADER.split('\t').nth(*index).expect("field index")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{AdmissionCatalog, HEADER, SupplementalPolicy, sha256};

    const SHA: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn row(fields: [&str; 16]) -> String {
        fields
            .map(|field| if field.is_empty() { "-" } else { field })
            .join("\t")
    }

    fn minimal_catalog() -> String {
        let mut rows = [
            row([
                "graph-file",
                "graph",
                "test/root.js",
                SHA,
                "",
                "module",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
            row([
                "graph-root",
                "graph",
                "test/root.js",
                "",
                "",
                "",
                "",
                "",
                "",
                "1",
                "0",
                "",
                "",
                "",
                "",
                "",
            ]),
            row([
                "module",
                "modules",
                "test/module.js",
                SHA,
                "",
                "module",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
            ]),
            row([
                "supplemental",
                "supplemental",
                "test/supplemental.js",
                SHA,
                "",
                "",
                "feature",
                "",
                "",
                "",
                "",
                "",
                "",
                "",
                "exact-features",
                "",
            ]),
        ];
        rows.sort();
        format!("{HEADER}\n{}\n", rows.join("\n"))
    }

    #[test]
    fn parses_a_canonical_catalog() {
        let catalog = AdmissionCatalog::parse(&minimal_catalog()).unwrap();
        assert!(catalog.module(Path::new("test/module.js")).is_some());
        assert_eq!(
            catalog
                .graph_root(Path::new("test/root.js"))
                .unwrap()
                .closure_file_count,
            1
        );
        assert_eq!(
            catalog
                .supplemental(Path::new("test/supplemental.js"))
                .unwrap()
                .policy,
            SupplementalPolicy::ExactFeatures
        );
    }

    #[test]
    fn internal_sha256_matches_standard_vectors() {
        assert_eq!(
            sha256(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn rejects_noncanonical_and_open_graph_data() {
        let catalog = minimal_catalog();
        let mut lines = catalog.lines().collect::<Vec<_>>();
        lines.swap(1, 2);
        let unsorted = format!("{}\n", lines.join("\n"));
        let error = AdmissionCatalog::parse(&unsorted).unwrap_err();
        assert!(error.contains("strict bytewise order"), "{error}");

        let request = row([
            "graph-request",
            "graph",
            "test/root.js",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "",
            "0",
            "./missing.js",
            "test/missing.js",
            "",
            "",
        ]);
        let escaped = minimal_catalog().replace(
            "graph-root\tgraph\ttest/root.js",
            &format!("{request}\ngraph-root\tgraph\ttest/root.js"),
        );
        let error = AdmissionCatalog::parse(&escaped).unwrap_err();
        assert!(
            error.contains("unknown") || error.contains("escapes"),
            "{error}"
        );

        let empty_field = minimal_catalog().replacen("\t-", "\t", 1);
        let error = AdmissionCatalog::parse(&empty_field).unwrap_err();
        assert!(error.contains("empty field"), "{error}");
    }

    #[test]
    fn checked_in_catalog_is_hash_authenticated() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("dev-support/test262/admissions.tsv");
        let expected = "083a22e3cf4a6b817d09238eb787bd6aed8a8cb4cb91d13796a3e1644513a866";
        AdmissionCatalog::load(&path, expected).unwrap();
        let error = AdmissionCatalog::load(
            &path,
            "0000000000000000000000000000000000000000000000000000000000000000",
        )
        .unwrap_err();
        assert!(error.contains("checksum mismatch"), "{error}");
    }

    #[test]
    fn checked_in_catalog_has_the_exact_migrated_shape() {
        let source = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/dev-support/test262/admissions.tsv"
        ));
        let catalog = AdmissionCatalog::parse(source).unwrap();
        assert_eq!(catalog.modules().count(), 166);
        assert_eq!(catalog.graph_roots().count(), 109);
        assert_eq!(
            catalog.graph_files.values().map(Vec::len).sum::<usize>(),
            150
        );
        assert_eq!(catalog.agent_hosts().count(), 59);
        assert_eq!(catalog.supplemental_admissions().count(), 2);
        assert_eq!(
            catalog
                .graph_roots()
                .map(|root| root.path.as_str())
                .collect::<BTreeSet<_>>()
                .len(),
            108
        );
        assert_eq!(
            catalog
                .modules()
                .map(|module| module.path.as_str())
                .chain(catalog.graph_roots().map(|root| root.path.as_str()))
                .chain(catalog.agent_hosts().map(|agent| agent.path.as_str()))
                .collect::<BTreeSet<_>>()
                .len(),
            333
        );
        assert_eq!(
            catalog
                .modules()
                .map(|module| module.path.as_str())
                .chain(
                    catalog
                        .graph_files
                        .values()
                        .flatten()
                        .map(|file| file.path.as_str()),
                )
                .chain(catalog.agent_hosts().map(|agent| agent.path.as_str()))
                .collect::<BTreeSet<_>>()
                .len(),
            373
        );
    }
}
