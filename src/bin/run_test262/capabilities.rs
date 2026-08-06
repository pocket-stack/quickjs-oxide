use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const FEATURES_SECTION: &str = "features";
const AUDITED_NEGATIVE_TESTS_SECTION: &str = "audited-negative-tests";
const EXECUTION_SECTION: &str = "execution";
const HOST_AGENT_TESTS_SECTION: &str = "host-agent-tests";
const SECTION_ORDER: [&str; 4] = [
    FEATURES_SECTION,
    AUDITED_NEGATIVE_TESTS_SECTION,
    EXECUTION_SECTION,
    HOST_AGENT_TESTS_SECTION,
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FailClosedClassification {
    pub(super) outcome: &'static str,
    pub(super) detail: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct OxideProfile {
    features: BTreeSet<String>,
    audited_negative_tests: BTreeSet<String>,
    host_agent_tests: BTreeSet<String>,
    async_execution: bool,
}

impl OxideProfile {
    pub(super) fn load(path: &Path) -> Result<Self, String> {
        let source = fs::read_to_string(path)
            .map_err(|error| format!("read {}: {error}", path.display()))?;
        Self::parse(&source).map_err(|error| format!("parse {}: {error}", path.display()))
    }

    pub(super) fn parse(source: &str) -> Result<Self, String> {
        let mut profile = Self::default();
        let mut seen_sections = BTreeSet::new();
        let mut section_index = None;
        let mut last_section_position = None;
        let mut previous_entry: Option<String> = None;
        let mut saw_async_execution_entry = false;

        for (line_index, raw_line) in source.lines().enumerate() {
            let line_number = line_index + 1;
            let line = raw_line.trim();
            if line.is_empty() {
                if !raw_line.is_empty() {
                    return Err(format!(
                        "line {line_number}: whitespace-only lines are not allowed"
                    ));
                }
                continue;
            }
            if line != raw_line {
                return Err(format!(
                    "line {line_number}: leading or trailing whitespace is not allowed"
                ));
            }
            if line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') || line.ends_with(']') {
                let Some(name) = line
                    .strip_prefix('[')
                    .and_then(|line| line.strip_suffix(']'))
                else {
                    return Err(format!("line {line_number}: malformed section header"));
                };
                if name.is_empty() || name.contains(['[', ']']) {
                    return Err(format!("line {line_number}: malformed section header"));
                }
                if !SECTION_ORDER.contains(&name) {
                    return Err(format!("line {line_number}: unknown section [{name}]"));
                }
                if !seen_sections.insert(name.to_owned()) {
                    return Err(format!("line {line_number}: duplicate section [{name}]"));
                }
                let section_position = SECTION_ORDER
                    .iter()
                    .position(|section| *section == name)
                    .expect("known section was checked");
                if last_section_position.is_some_and(|previous| section_position <= previous) {
                    return Err(format!(
                        "line {line_number}: section [{name}] is out of order"
                    ));
                }
                section_index = Some(section_position);
                last_section_position = Some(section_position);
                previous_entry = None;
                continue;
            }

            let Some(current_section) = section_index else {
                return Err(format!(
                    "line {line_number}: entry appears before the [features] section"
                ));
            };
            validate_entry(line, current_section, line_number)?;

            if current_section == 2 {
                if saw_async_execution_entry {
                    return Err(format!(
                        "line {line_number}: duplicate entry {line:?} in [{EXECUTION_SECTION}]"
                    ));
                }
                profile.async_execution = true;
                saw_async_execution_entry = true;
                continue;
            }

            let entries = match current_section {
                0 => &mut profile.features,
                1 => &mut profile.audited_negative_tests,
                3 => &mut profile.host_agent_tests,
                _ => unreachable!("execution entries were handled above"),
            };
            if entries.contains(line) {
                return Err(format!(
                    "line {line_number}: duplicate entry {line:?} in [{}]",
                    SECTION_ORDER[current_section]
                ));
            }
            if let Some(previous) = &previous_entry
                && previous.as_str() > line
            {
                return Err(format!(
                    "line {line_number}: entry {line:?} is out of order after {previous:?} in [{}]",
                    SECTION_ORDER[current_section]
                ));
            }
            entries.insert(line.to_owned());
            previous_entry = Some(line.to_owned());
        }

        for section in [FEATURES_SECTION, AUDITED_NEGATIVE_TESTS_SECTION] {
            if !seen_sections.contains(section) {
                return Err(format!("missing required section [{section}]"));
            }
        }
        if seen_sections.contains(EXECUTION_SECTION) && !saw_async_execution_entry {
            return Err(format!(
                "section [{EXECUTION_SECTION}] must contain async=true"
            ));
        }
        Ok(profile)
    }

    pub(super) fn allows_async_execution(&self) -> bool {
        self.async_execution
    }

    pub(super) fn allows_agent_host(&self, path: &Path) -> bool {
        self.host_agent_tests.contains(&test_path(path))
    }

    pub(super) fn audited_negative_paths(&self) -> impl Iterator<Item = &str> {
        self.audited_negative_tests.iter().map(String::as_str)
    }

    /// Return the first fail-closed classification for one Test262 test.
    ///
    /// Declared feature gaps take precedence and are reported together in
    /// stable lexical order. Feature metadata can safely reject a test, but an
    /// otherwise featureless negative still needs an exact audited path before
    /// its expected exception may contribute to the conformance pass count.
    pub(super) fn classify(
        &self,
        path: &Path,
        declared_features: &[String],
        is_negative: bool,
    ) -> Option<FailClosedClassification> {
        let unsupported = declared_features
            .iter()
            .filter(|feature| !self.features.contains(feature.as_str()))
            .cloned()
            .collect::<BTreeSet<_>>();
        if !unsupported.is_empty() {
            return Some(FailClosedClassification {
                outcome: "unsupported-feature",
                detail: format!(
                    "quickjs-oxide does not declare Test262 feature support: {}",
                    unsupported.into_iter().collect::<Vec<_>>().join(", ")
                ),
            });
        }

        let path = test_path(path);
        if is_negative && !self.audited_negative_tests.contains(&path) {
            return Some(FailClosedClassification {
                outcome: "unsupported-negative-provenance",
                detail: format!("negative Test262 path has not been audited: {path}"),
            });
        }
        None
    }
}

fn validate_entry(entry: &str, section_index: usize, line_number: usize) -> Result<(), String> {
    if section_index == 2 {
        if entry != "async=true" {
            return Err(format!(
                "line {line_number}: [{EXECUTION_SECTION}] only accepts async=true"
            ));
        }
        return Ok(());
    }
    if entry
        .chars()
        .any(|character| character.is_whitespace() || character.is_control())
        || entry.contains(['=', '[', ']', '#', ';'])
    {
        return Err(format!(
            "line {line_number}: malformed entry {entry:?} in [{}]",
            SECTION_ORDER[section_index]
        ));
    }
    if matches!(section_index, 1 | 3)
        && (!entry.starts_with("test/")
            || !entry.ends_with(".js")
            || entry.contains('\\')
            || entry
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | "..")))
    {
        return Err(format!(
            "line {line_number}: [{}] entry must be an exact test/*.js path: {entry:?}",
            SECTION_ORDER[section_index]
        ));
    }
    Ok(())
}

fn test_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::path::Path;

    use super::{
        AUDITED_NEGATIVE_TESTS_SECTION, EXECUTION_SECTION, FEATURES_SECTION,
        HOST_AGENT_TESTS_SECTION, OxideProfile, SECTION_ORDER,
    };

    const CHECKED_IN_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/compat/test262-oxide.conf"
    ));
    const ATOMICS_NON_SHARED_CORE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-atomics-non-shared-core.conf"
    ));
    const SHARED_ARRAY_BUFFER_CORE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-shared-array-buffer-core.conf"
    ));
    const SHARED_ATOMICS_NONBLOCKING_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-shared-atomics-nonblocking.conf"
    ));
    const ATOMICS_WAIT_NONAGENT_BOUNDED_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-atomics-wait-nonagent-bounded.conf"
    ));
    const ATOMICS_PAUSE_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-atomics-pause-global-parent.conf"
    ));
    const ATOMICS_PAUSE_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-atomics-pause-global-candidate.conf"
    ));
    const ERROR_REGEXP_TYPEDARRAY_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-error-regexp-typedarray-global-candidate.conf"
    ));
    const SHARED_ATOMICS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-shared-atomics-global-parent.conf"
    ));
    const SHARED_ATOMICS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-shared-atomics-global-candidate.conf"
    ));
    const AGENT_STAGE_A_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-stage-a-global-candidate.conf"
    ));
    const AGENT_BROADCAST_A_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-broadcast-a-parent.conf"
    ));
    const AGENT_BROADCAST_A_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-broadcast-a-candidate.conf"
    ));
    const AGENT_BROADCAST_A_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-broadcast-a-global-parent.conf"
    ));
    const AGENT_BROADCAST_A_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-broadcast-a-global-candidate.conf"
    ));
    const AGENT_BROADCAST_A_ACTIVATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-broadcast-a.txt"
    ));
    const AGENT_WAIT_BOUNDED_A_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wait-bounded-a-parent.conf"
    ));
    const AGENT_WAIT_BOUNDED_A_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wait-bounded-a-candidate.conf"
    ));
    const AGENT_WAIT_BOUNDED_A_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wait-bounded-a-global-parent.conf"
    ));
    const AGENT_WAIT_BOUNDED_A_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wait-bounded-a-global-candidate.conf"
    ));
    const AGENT_WAIT_BOUNDED_A_ACTIVATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wait-bounded-a.txt"
    ));
    const AGENT_WAKE_COUNT_LOCATION_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wake-count-location-parent.conf"
    ));
    const AGENT_WAKE_COUNT_LOCATION_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wake-count-location-candidate.conf"
    ));
    const AGENT_WAKE_COUNT_LOCATION_ACTIVATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wake-count-location.txt"
    ));
    const AGENT_FIFO_WAKE_ORDER_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-fifo-wake-order-parent.conf"
    ));
    const AGENT_FIFO_WAKE_ORDER_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-fifo-wake-order-candidate.conf"
    ));
    const AGENT_FIFO_WAKE_ORDER_ACTIVATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-fifo-wake-order.txt"
    ));
    const AGENT_WAKE_FIFO_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wake-fifo-global-parent.conf"
    ));
    const AGENT_WAKE_FIFO_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-agent-wake-fifo-global-candidate.conf"
    ));
    const PROPERTY_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-regexp-unicode-properties.txt"
    ));
    const GENERATOR_DESTRUCTURING_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-generator-destructuring.conf"
    ));
    const OPTIONAL_CHAINING_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-optional-chaining.conf"
    ));
    const ITERATOR_HELPERS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-iterator-helpers-global-parent.conf"
    ));
    const ITERATOR_HELPERS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-iterator-helpers-global-candidate.conf"
    ));
    const GLOBAL_THIS_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-global-this-parent.conf"
    ));
    const GLOBAL_THIS_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-global-this-candidate.conf"
    ));
    const GLOBAL_THIS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-global-this-global-parent.conf"
    ));
    const GLOBAL_THIS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-global-this-global-candidate.conf"
    ));
    const PROMISE_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-promise-global-parent.conf"
    ));
    const PROMISE_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-promise-global-candidate.conf"
    ));
    const UINT8ARRAY_CODECS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-uint8array-codecs-global-parent.conf"
    ));
    const UINT8ARRAY_CODECS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-uint8array-codecs-global-candidate.conf"
    ));
    const RESIZABLE_ARRAYBUFFER_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-resizable-arraybuffer-global-parent.conf"
    ));
    const RESIZABLE_ARRAYBUFFER_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-resizable-arraybuffer-global-candidate.conf"
    ));
    const COMPUTED_PROPERTY_NAMES_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-computed-property-names-global-parent.conf"
    ));
    const COMPUTED_PROPERTY_NAMES_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-computed-property-names-global-candidate.conf"
    ));
    const REST_PARAMETERS_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-rest-parameters-parent.conf"
    ));
    const REST_PARAMETERS_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-rest-parameters-candidate.conf"
    ));
    const REST_PARAMETERS_ACTIVATION: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-rest-parameters-activation.txt"
    ));
    const DEFAULT_PARAMETERS_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-default-parameters-parent.conf"
    ));
    const DEFAULT_PARAMETERS_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-default-parameters-candidate.conf"
    ));
    const DEFAULT_PARAMETERS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-default-parameters-global-candidate.conf"
    ));
    const DATA_VIEW_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-data-view-global-parent.conf"
    ));
    const DATA_VIEW_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-data-view-global-candidate.conf"
    ));
    const OBJECT_REST_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-object-rest-global-parent.conf"
    ));
    const OBJECT_REST_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-object-rest-global-candidate.conf"
    ));
    const WEAK_COLLECTIONS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-weak-collections-global-parent.conf"
    ));
    const WEAK_COLLECTIONS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-weak-collections-global-candidate.conf"
    ));
    const WEAK_REF_FINALIZATION_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-weak-ref-finalization-global-parent.conf"
    ));
    const WEAK_REF_FINALIZATION_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-weak-ref-finalization-global-candidate.conf"
    ));
    const HOST_GC_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-host-gc-global-parent.conf"
    ));
    const HOST_GC_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-host-gc-global-candidate.conf"
    ));
    const REALM_HOSTS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-realm-hosts-global-parent.conf"
    ));
    const REALM_HOSTS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-realm-hosts-global-candidate.conf"
    ));
    const BINARY_DATA_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-binary-data-global-parent.conf"
    ));
    const BINARY_DATA_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-binary-data-global-candidate.conf"
    ));
    const PROMISE_TRY_WITH_RESOLVERS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-promise-try-with-resolvers-global-parent.conf"
    ));
    const PROMISE_TRY_WITH_RESOLVERS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-promise-try-with-resolvers-global-candidate.conf"
    ));
    const HTML_COMMENTS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-html-comments-global-parent.conf"
    ));
    const HTML_COMMENTS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-html-comments-global-candidate.conf"
    ));
    const HTML_COMMENTS_GLOBAL_ADDED_NEGATIVES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-html-comments-global-added-negatives.txt"
    ));
    const DEBUGGER_STATEMENT_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-debugger-statement-global-parent.conf"
    ));
    const DEBUGGER_STATEMENT_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-debugger-statement-global-candidate.conf"
    ));
    const DEBUGGER_STATEMENT_GLOBAL_ADDED_NEGATIVES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-debugger-statement-global-added-negatives.txt"
    ));
    const FUTURE_RESERVED_WORDS_SCOPED_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-future-reserved-words-scoped.conf"
    ));
    const FUTURE_RESERVED_WORDS_GLOBAL_PARENT_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-future-reserved-words-global-parent.conf"
    ));
    const FUTURE_RESERVED_WORDS_GLOBAL_CANDIDATE_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-future-reserved-words-global-candidate.conf"
    ));
    const FUTURE_RESERVED_WORDS_GLOBAL_ADDED_NEGATIVES: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-future-reserved-words-global-added-negatives.txt"
    ));
    const FUTURE_RESERVED_WORDS_NEGATIVE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-future-reserved-words-negative.txt"
    ));
    const DEFAULT_PARAMETERS_STRICT_BODY: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-default-parameters-strict-body.txt"
    ));
    const PROPERTY_POSITIVE_PATHS: [&str; 2] = [
        "test/built-ins/RegExp/property-escapes/character-class.js",
        "test/built-ins/RegExp/property-escapes/special-property-value-Script_Extensions-Unknown.js",
    ];
    const EXPECTED_FEATURES: [&str; 132] = [
        "AggregateError",
        "Array.prototype.at",
        "Array.prototype.flat",
        "Array.prototype.flatMap",
        "Array.prototype.includes",
        "ArrayBuffer",
        "Atomics",
        "Atomics.pause",
        "BigInt",
        "DataView",
        "DataView.prototype.getFloat32",
        "DataView.prototype.getFloat64",
        "DataView.prototype.getInt16",
        "DataView.prototype.getInt32",
        "DataView.prototype.getInt8",
        "DataView.prototype.getUint16",
        "DataView.prototype.getUint32",
        "DataView.prototype.setUint8",
        "Error.isError",
        "FinalizationRegistry",
        "Float16Array",
        "Float32Array",
        "Float64Array",
        "Int16Array",
        "Int32Array",
        "Int8Array",
        "Map",
        "Math.sumPrecise",
        "Object.fromEntries",
        "Object.hasOwn",
        "Promise",
        "Promise.allSettled",
        "Promise.any",
        "Promise.prototype.finally",
        "Proxy",
        "Reflect",
        "Reflect.construct",
        "Reflect.set",
        "Reflect.setPrototypeOf",
        "RegExp.escape",
        "Set",
        "SharedArrayBuffer",
        "String.fromCodePoint",
        "String.prototype.at",
        "String.prototype.endsWith",
        "String.prototype.includes",
        "String.prototype.isWellFormed",
        "String.prototype.matchAll",
        "String.prototype.replaceAll",
        "String.prototype.toWellFormed",
        "String.prototype.trimEnd",
        "String.prototype.trimStart",
        "Symbol",
        "Symbol.asyncIterator",
        "Symbol.hasInstance",
        "Symbol.isConcatSpreadable",
        "Symbol.iterator",
        "Symbol.match",
        "Symbol.matchAll",
        "Symbol.prototype.description",
        "Symbol.replace",
        "Symbol.search",
        "Symbol.species",
        "Symbol.split",
        "Symbol.toPrimitive",
        "Symbol.toStringTag",
        "Symbol.unscopables",
        "TypedArray",
        "TypedArray.prototype.at",
        "Uint16Array",
        "Uint32Array",
        "Uint8Array",
        "Uint8ClampedArray",
        "WeakMap",
        "WeakRef",
        "WeakSet",
        "__getter__",
        "__proto__",
        "__setter__",
        "align-detached-buffer-semantics-with-web-reality",
        "array-find-from-last",
        "array-grouping",
        "arraybuffer-transfer",
        "arrow-function",
        "async-functions",
        "async-iteration",
        "change-array-by-copy",
        "coalesce-expression",
        "computed-property-names",
        "const",
        "default-parameters",
        "destructuring-binding",
        "error-cause",
        "exponentiation",
        "for-in-order",
        "generators",
        "globalThis",
        "hashbang",
        "host-create-realm-required",
        "host-eval-script-required",
        "host-gc-required",
        "iterator-helpers",
        "iterator-sequencing",
        "json-parse-with-source",
        "let",
        "logical-assignment-operators",
        "new.target",
        "numeric-separator-literal",
        "object-rest",
        "object-spread",
        "optional-catch-binding",
        "optional-chaining",
        "promise-try",
        "promise-with-resolvers",
        "regexp-dotall",
        "regexp-duplicate-named-groups",
        "regexp-lookbehind",
        "regexp-match-indices",
        "regexp-modifiers",
        "regexp-named-groups",
        "regexp-unicode-property-escapes",
        "resizable-arraybuffer",
        "rest-parameters",
        "set-methods",
        "string-trimming",
        "super",
        "symbols-as-weakmap-keys",
        "template",
        "u180e",
        "uint8array-base64",
        "upsert",
        "well-formed-json-stringify",
    ];
    const EXPECTED_AUDITED_NEGATIVES: [&str; 281] = [
        "test/language/comments/hashbang/escaped-bang-041.js",
        "test/language/eval-code/direct/var-env-global-lex-non-strict.js",
        "test/language/expressions/assignment/target-cover-newtarget.js",
        "test/language/expressions/assignment/target-newtarget.js",
        "test/language/expressions/coalesce/cannot-chain-head-with-logical-and.js",
        "test/language/expressions/coalesce/cannot-chain-head-with-logical-or.js",
        "test/language/expressions/coalesce/cannot-chain-tail-with-logical-and.js",
        "test/language/expressions/coalesce/cannot-chain-tail-with-logical-or.js",
        "test/language/expressions/logical-assignment/lgcl-and-arguments-strict.js",
        "test/language/expressions/logical-assignment/lgcl-and-assignment-operator-non-simple-lhs.js",
        "test/language/expressions/logical-assignment/lgcl-and-eval-strict.js",
        "test/language/expressions/logical-assignment/lgcl-and-non-simple.js",
        "test/language/expressions/logical-assignment/lgcl-nullish-arguments-strict.js",
        "test/language/expressions/logical-assignment/lgcl-nullish-assignment-operator-non-simple-lhs.js",
        "test/language/expressions/logical-assignment/lgcl-nullish-eval-strict.js",
        "test/language/expressions/logical-assignment/lgcl-nullish-non-simple.js",
        "test/language/expressions/logical-assignment/lgcl-or-arguments-strict.js",
        "test/language/expressions/logical-assignment/lgcl-or-assignment-operator-non-simple-lhs.js",
        "test/language/expressions/logical-assignment/lgcl-or-eval-strict.js",
        "test/language/expressions/logical-assignment/lgcl-or-non-simple.js",
        "test/language/expressions/object/11.1.5-1gs.js",
        "test/language/expressions/object/__proto__-duplicate.js",
        "test/language/expressions/object/getter-body-strict-inside.js",
        "test/language/expressions/object/getter-body-strict-outside.js",
        "test/language/expressions/object/method-definition/early-errors-object-method-duplicate-parameters.js",
        "test/language/expressions/object/method-definition/escaped-get-e.js",
        "test/language/expressions/object/method-definition/escaped-get-g.js",
        "test/language/expressions/object/method-definition/escaped-get-t.js",
        "test/language/expressions/object/method-definition/escaped-get.js",
        "test/language/expressions/object/method-definition/escaped-set-e.js",
        "test/language/expressions/object/method-definition/escaped-set-s.js",
        "test/language/expressions/object/method-definition/escaped-set-t.js",
        "test/language/expressions/object/method-definition/escaped-set.js",
        "test/language/expressions/object/method-definition/name-param-redecl.js",
        "test/language/expressions/object/method-definition/name-super-call-body.js",
        "test/language/expressions/object/setter-body-strict-inside.js",
        "test/language/expressions/object/setter-body-strict-outside.js",
        "test/language/expressions/object/setter-param-arguments-strict-inside.js",
        "test/language/expressions/object/setter-param-arguments-strict-outside.js",
        "test/language/expressions/object/setter-param-eval-strict-inside.js",
        "test/language/expressions/object/setter-param-eval-strict-outside.js",
        "test/language/expressions/postfix-decrement/target-cover-newtarget.js",
        "test/language/expressions/postfix-decrement/target-newtarget.js",
        "test/language/expressions/postfix-increment/target-cover-newtarget.js",
        "test/language/expressions/postfix-increment/target-newtarget.js",
        "test/language/expressions/prefix-decrement/target-cover-newtarget.js",
        "test/language/expressions/prefix-decrement/target-newtarget.js",
        "test/language/expressions/prefix-increment/target-cover-newtarget.js",
        "test/language/expressions/prefix-increment/target-newtarget.js",
        "test/language/expressions/template-literal/unicode-escape-nls-err.js",
        "test/language/global-code/decl-lex-restricted-global.js",
        "test/language/global-code/new.target-arrow.js",
        "test/language/global-code/new.target.js",
        "test/language/identifiers/unicode-escape-nls-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-bil-bd-nsl-bd-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-bil-nsl-bd-dunder-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-bil-nsl-bd-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-dd-nsl-dds-dunder-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-dd-nsl-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-dds-nsl-dds-dunder-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-dds-nsl-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-hil-hd-nsl-hd-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-hil-nsl-hd-dunder-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-hil-nsl-hd-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-lol-00-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-lol-01-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-lol-07-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-lol-0_0-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-lol-0_1-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-lol-0_7-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-nonoctal-08-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-nonoctal-09-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-nonoctal-0_8-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-nonoctal-0_9-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-nzd-nsl-dds-dunder-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-nzd-nsl-dds-leading-zero-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-oil-nsl-od-dunder-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-oil-nsl-od-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-oil-od-nsl-od-err.js",
        "test/language/literals/bigint/numeric-separators/numeric-separator-literal-unicode-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-bil-bd-nsl-bd-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-bil-nsl-bd-dunder-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-bil-nsl-bd-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dd-nsl-dds-dunder-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dd-nsl-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dds-nsl-dds-dunder-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dds-nsl-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dil-dot-dds-nsl-ep-dd-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dil-dot-nsl-dd-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dil-dot-nsl-ep-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dil-dot-nsl-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dot-dds-nsl-ep-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dot-nsl-ep-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-dot-nsl-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-hil-hd-nsl-hd-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-hil-nsl-hd-dunder-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-hil-nsl-hd-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-lol-00-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-lol-01-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-lol-07-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-lol-0_0-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-lol-0_1-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-lol-0_7-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-nonoctal-08-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-nonoctal-09-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-nonoctal-0_8-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-nonoctal-0_9-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-nzd-nsl-dds-dunder-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-nzd-nsl-dds-leading-zero-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-oil-nsl-od-dunder-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-oil-nsl-od-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-oil-od-nsl-od-err.js",
        "test/language/literals/numeric/numeric-separators/numeric-separator-literal-unicode-err.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-add-remove-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-add-remove-m.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-add-remove-multi-duplicate.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-add-remove-s-escape.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-add-remove-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-both-empty.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-code-point-repeat-i-1.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-code-point-repeat-i-2.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-no-colon-1.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-no-colon-2.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-no-colon-3.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-arbitrary.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-combining-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-combining-m.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-combining-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-d.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-g.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-non-display-1.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-non-display-2.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-non-flag.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-u.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-uppercase-I.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-y.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-zwj.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-zwnbsp.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-other-code-point-zwnj.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-add-remove-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-add-remove-m.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-add-remove-multi-duplicate.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-add-remove-s-escape.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-add-remove-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-code-point-repeat-i-1.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-code-point-repeat-i-2.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-arbitrary.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-combining-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-combining-m.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-combining-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-d.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-g.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-non-display-1.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-non-display-2.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-non-flag.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-u.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-uppercase-I.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-y.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-zwj.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-zwnbsp.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-other-code-point-zwnj.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-should-not-case-fold-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-should-not-case-fold-m.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-should-not-case-fold-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-should-not-unicode-case-fold-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-reverse-should-not-unicode-case-fold-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-should-not-case-fold-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-should-not-case-fold-m.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-should-not-case-fold-s.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-should-not-unicode-case-fold-i.js",
        "test/language/literals/regexp/early-err-arithmetic-modifiers-should-not-unicode-case-fold-s.js",
        "test/language/literals/regexp/early-err-modifiers-code-point-repeat-i-1.js",
        "test/language/literals/regexp/early-err-modifiers-code-point-repeat-i-2.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-arbitrary.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-combining-i.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-combining-m.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-combining-s.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-d.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-g.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-non-display-1.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-non-display-2.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-non-flag.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-u.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-uppercase-I.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-y.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-zwj.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-zwnbsp.js",
        "test/language/literals/regexp/early-err-modifiers-other-code-point-zwnj.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-case-fold-i.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-case-fold-m.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-case-fold-s.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-unicode-case-fold-i.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-unicode-case-fold-s.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-unicode-escape-i.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-unicode-escape-m.js",
        "test/language/literals/regexp/early-err-modifiers-should-not-unicode-escape-s.js",
        "test/language/literals/regexp/invalid-optional-lookbehind.js",
        "test/language/literals/regexp/invalid-optional-negative-lookbehind.js",
        "test/language/literals/regexp/invalid-range-lookbehind.js",
        "test/language/literals/regexp/invalid-range-negative-lookbehind.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-2-u.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-2.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-3-u.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-3.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-4-u.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-4.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-5.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-u.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname-without-group-u.js",
        "test/language/literals/regexp/named-groups/invalid-dangling-groupname.js",
        "test/language/literals/regexp/named-groups/invalid-duplicate-groupspecifier-2-u.js",
        "test/language/literals/regexp/named-groups/invalid-duplicate-groupspecifier-2.js",
        "test/language/literals/regexp/named-groups/invalid-duplicate-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-duplicate-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-empty-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-empty-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-identity-escape-in-capture-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-2-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-2.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-3-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-3.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-4.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-5.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-6.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-without-group-2-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-without-group-3-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname-without-group-u.js",
        "test/language/literals/regexp/named-groups/invalid-incomplete-groupname.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-continue-groupspecifier-4-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-continue-groupspecifier-4.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-continue-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-2-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-2.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-3.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-4-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-4.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-5-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-5.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-6.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-7.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-8-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-8.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-9-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-non-id-start-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-numeric-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-numeric-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-punctuator-starting-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-punctuator-starting-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-punctuator-within-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-punctuator-within-groupspecifier.js",
        "test/language/literals/regexp/named-groups/invalid-unterminated-groupspecifier-u.js",
        "test/language/literals/regexp/named-groups/invalid-unterminated-groupspecifier.js",
        "test/language/literals/regexp/u-invalid-legacy-octal-escape.js",
        "test/language/literals/regexp/u-invalid-oob-decimal-escape.js",
        "test/language/literals/regexp/u-invalid-optional-lookbehind.js",
        "test/language/literals/regexp/u-invalid-optional-negative-lookbehind.js",
        "test/language/literals/regexp/u-invalid-range-lookbehind.js",
        "test/language/literals/regexp/u-invalid-range-negative-lookbehind.js",
        "test/language/literals/regexp/unicode-escape-nls-err.js",
        "test/language/literals/string/unicode-escape-nls-err-double.js",
        "test/language/literals/string/unicode-escape-nls-err-single.js",
        "test/language/statements/const/global-use-before-initialization-in-declaration-statement.js",
        "test/language/statements/const/syntax/with-initializer-while-expression-statement.js",
        "test/language/statements/for/S12.6.3_A7_T2.js",
        "test/language/statements/function/early-body-super-prop.js",
        "test/language/statements/if/S12.5_A8.js",
        "test/language/statements/if/if-cls-else-cls.js",
        "test/language/statements/labeled/continue.js",
        "test/language/statements/let/global-use-before-initialization-in-prior-statement.js",
        "test/language/statements/switch/scope-lex-const.js",
        "test/language/statements/try/early-catch-lex.js",
        "test/language/statements/try/optional-catch-binding-parens.js",
        "test/language/statements/variable/S12.2_A8_T2.js",
        "test/language/statements/variable/S12.2_A8_T7.js",
        "test/language/statements/variable/arguments-strict-list-first-init.js",
        "test/language/statements/variable/arguments-strict-list-middle-init.js",
        "test/language/statements/variable/eval-strict-list-final-init.js",
        "test/language/statements/while/decl-fun.js",
        "test/language/white-space/mongolian-vowel-separator.js",
    ];

    #[test]
    fn checked_in_profile_covers_the_fixed_smoke_contract() {
        let profile = OxideProfile::parse(CHECKED_IN_PROFILE).unwrap();
        let loaded = OxideProfile::load(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("compat/test262-oxide.conf"),
        )
        .unwrap();

        assert_eq!(profile, loaded);
        assert!(profile.allows_async_execution());
        assert!(
            profile
                .features
                .iter()
                .map(String::as_str)
                .eq(EXPECTED_FEATURES)
        );
        let previously_audited_negatives = EXPECTED_AUDITED_NEGATIVES
            .into_iter()
            .chain(PROPERTY_MANIFEST.lines().filter(|path| {
                path.starts_with("test/built-ins/RegExp/property-escapes/")
                    && !path.starts_with("test/built-ins/RegExp/property-escapes/generated/")
                    && !PROPERTY_POSITIVE_PATHS.contains(path)
            }))
            .collect::<BTreeSet<_>>();
        assert_eq!(previously_audited_negatives.len(), 423);

        let generator_destructuring_profile =
            OxideProfile::parse(GENERATOR_DESTRUCTURING_PROFILE).unwrap();
        assert_eq!(
            generator_destructuring_profile.audited_negative_tests.len(),
            379
        );
        assert!(
            generator_destructuring_profile
                .audited_negative_tests
                .iter()
                .all(|path| !previously_audited_negatives.contains(path.as_str()))
        );

        let previously_audited_negatives = previously_audited_negatives
            .into_iter()
            .chain(
                generator_destructuring_profile
                    .audited_negative_tests
                    .iter()
                    .map(String::as_str),
            )
            .collect::<BTreeSet<_>>();
        assert_eq!(previously_audited_negatives.len(), 802);

        let optional_chaining_profile = OxideProfile::parse(OPTIONAL_CHAINING_PROFILE).unwrap();
        let iterator_helpers_global_parent =
            OxideProfile::parse(ITERATOR_HELPERS_GLOBAL_PARENT_PROFILE).unwrap();
        let iterator_helpers_global_candidate =
            OxideProfile::parse(ITERATOR_HELPERS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let global_this_parent = OxideProfile::parse(GLOBAL_THIS_PARENT_PROFILE).unwrap();
        let global_this_candidate = OxideProfile::parse(GLOBAL_THIS_CANDIDATE_PROFILE).unwrap();
        let global_this_global_parent =
            OxideProfile::parse(GLOBAL_THIS_GLOBAL_PARENT_PROFILE).unwrap();
        let global_this_global_candidate =
            OxideProfile::parse(GLOBAL_THIS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let promise_global_parent = OxideProfile::parse(PROMISE_GLOBAL_PARENT_PROFILE).unwrap();
        let promise_global_candidate =
            OxideProfile::parse(PROMISE_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let uint8array_codecs_global_parent =
            OxideProfile::parse(UINT8ARRAY_CODECS_GLOBAL_PARENT_PROFILE).unwrap();
        let uint8array_codecs_global_candidate =
            OxideProfile::parse(UINT8ARRAY_CODECS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let resizable_arraybuffer_global_parent =
            OxideProfile::parse(RESIZABLE_ARRAYBUFFER_GLOBAL_PARENT_PROFILE).unwrap();
        let resizable_arraybuffer_global_candidate =
            OxideProfile::parse(RESIZABLE_ARRAYBUFFER_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let computed_property_names_global_parent =
            OxideProfile::parse(COMPUTED_PROPERTY_NAMES_GLOBAL_PARENT_PROFILE).unwrap();
        let computed_property_names_global_candidate =
            OxideProfile::parse(COMPUTED_PROPERTY_NAMES_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let rest_parameters_parent = OxideProfile::parse(REST_PARAMETERS_PARENT_PROFILE).unwrap();
        let rest_parameters_candidate =
            OxideProfile::parse(REST_PARAMETERS_CANDIDATE_PROFILE).unwrap();
        let default_parameters_parent =
            OxideProfile::parse(DEFAULT_PARAMETERS_PARENT_PROFILE).unwrap();
        let default_parameters_candidate =
            OxideProfile::parse(DEFAULT_PARAMETERS_CANDIDATE_PROFILE).unwrap();
        let default_parameters_global_candidate =
            OxideProfile::parse(DEFAULT_PARAMETERS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let data_view_global_parent = OxideProfile::parse(DATA_VIEW_GLOBAL_PARENT_PROFILE).unwrap();
        let data_view_global_candidate =
            OxideProfile::parse(DATA_VIEW_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let object_rest_global_parent =
            OxideProfile::parse(OBJECT_REST_GLOBAL_PARENT_PROFILE).unwrap();
        let object_rest_global_candidate =
            OxideProfile::parse(OBJECT_REST_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let weak_collections_global_parent =
            OxideProfile::parse(WEAK_COLLECTIONS_GLOBAL_PARENT_PROFILE).unwrap();
        let weak_collections_global_candidate =
            OxideProfile::parse(WEAK_COLLECTIONS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let weak_ref_finalization_global_parent =
            OxideProfile::parse(WEAK_REF_FINALIZATION_GLOBAL_PARENT_PROFILE).unwrap();
        let weak_ref_finalization_global_candidate =
            OxideProfile::parse(WEAK_REF_FINALIZATION_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let host_gc_global_parent = OxideProfile::parse(HOST_GC_GLOBAL_PARENT_PROFILE).unwrap();
        let host_gc_global_candidate =
            OxideProfile::parse(HOST_GC_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let realm_hosts_global_parent =
            OxideProfile::parse(REALM_HOSTS_GLOBAL_PARENT_PROFILE).unwrap();
        let realm_hosts_global_candidate =
            OxideProfile::parse(REALM_HOSTS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let binary_data_global_parent =
            OxideProfile::parse(BINARY_DATA_GLOBAL_PARENT_PROFILE).unwrap();
        let binary_data_global_candidate =
            OxideProfile::parse(BINARY_DATA_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let promise_try_with_resolvers_global_parent =
            OxideProfile::parse(PROMISE_TRY_WITH_RESOLVERS_GLOBAL_PARENT_PROFILE).unwrap();
        let promise_try_with_resolvers_global_candidate =
            OxideProfile::parse(PROMISE_TRY_WITH_RESOLVERS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let html_comments_global_parent =
            OxideProfile::parse(HTML_COMMENTS_GLOBAL_PARENT_PROFILE).unwrap();
        let html_comments_global_candidate =
            OxideProfile::parse(HTML_COMMENTS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let debugger_statement_global_parent =
            OxideProfile::parse(DEBUGGER_STATEMENT_GLOBAL_PARENT_PROFILE).unwrap();
        let debugger_statement_global_candidate =
            OxideProfile::parse(DEBUGGER_STATEMENT_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let future_reserved_words_global_parent =
            OxideProfile::parse(FUTURE_RESERVED_WORDS_GLOBAL_PARENT_PROFILE).unwrap();
        let future_reserved_words_global_candidate =
            OxideProfile::parse(FUTURE_RESERVED_WORDS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let atomics_pause_global_parent =
            OxideProfile::parse(ATOMICS_PAUSE_GLOBAL_PARENT_PROFILE).unwrap();
        let atomics_pause_global_candidate =
            OxideProfile::parse(ATOMICS_PAUSE_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let error_regexp_typedarray_global_candidate =
            OxideProfile::parse(ERROR_REGEXP_TYPEDARRAY_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let shared_atomics_global_parent =
            OxideProfile::parse(SHARED_ATOMICS_GLOBAL_PARENT_PROFILE).unwrap();
        let shared_atomics_global_candidate =
            OxideProfile::parse(SHARED_ATOMICS_GLOBAL_CANDIDATE_PROFILE).unwrap();
        assert_eq!(optional_chaining_profile, iterator_helpers_global_parent);
        assert_eq!(optional_chaining_profile.audited_negative_tests.len(), 828);
        assert!(previously_audited_negatives.iter().all(|path| {
            optional_chaining_profile
                .audited_negative_tests
                .contains(*path)
        }));
        assert_eq!(
            optional_chaining_profile
                .audited_negative_tests
                .iter()
                .filter(|path| !previously_audited_negatives.contains(path.as_str()))
                .count(),
            26
        );
        assert_eq!(
            iterator_helpers_global_candidate
                .features
                .difference(&iterator_helpers_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["iterator-helpers"]
        );
        assert!(
            iterator_helpers_global_parent
                .features
                .difference(&iterator_helpers_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            iterator_helpers_global_candidate.audited_negative_tests,
            iterator_helpers_global_parent.audited_negative_tests
        );
        assert_eq!(
            iterator_helpers_global_candidate.allows_async_execution(),
            iterator_helpers_global_parent.allows_async_execution()
        );
        assert!(
            iterator_helpers_global_candidate
                .features
                .difference(&profile.features)
                .next()
                .is_none()
        );
        assert!(
            iterator_helpers_global_candidate
                .audited_negative_tests
                .difference(&profile.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            profile.allows_async_execution(),
            iterator_helpers_global_candidate.allows_async_execution()
        );
        assert_eq!(global_this_parent, iterator_helpers_global_candidate);
        assert_eq!(
            global_this_candidate
                .features
                .difference(&global_this_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["globalThis"]
        );
        assert!(
            global_this_parent
                .features
                .difference(&global_this_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            global_this_candidate.audited_negative_tests,
            global_this_parent.audited_negative_tests
        );
        assert_eq!(
            global_this_candidate.allows_async_execution(),
            global_this_parent.allows_async_execution()
        );
        assert_eq!(global_this_global_parent, global_this_parent);
        assert_eq!(global_this_global_candidate, global_this_candidate);
        assert!(
            global_this_global_candidate
                .features
                .difference(&profile.features)
                .next()
                .is_none()
        );
        assert!(
            global_this_global_candidate
                .audited_negative_tests
                .difference(&profile.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            profile.allows_async_execution(),
            global_this_global_candidate.allows_async_execution()
        );
        assert_eq!(promise_global_parent, global_this_global_candidate);
        assert_eq!(
            promise_global_candidate
                .features
                .difference(&promise_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "Promise",
                "Promise.allSettled",
                "Promise.any",
                "Promise.prototype.finally",
            ]
        );
        assert!(
            promise_global_parent
                .features
                .difference(&promise_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            promise_global_candidate.audited_negative_tests,
            promise_global_parent.audited_negative_tests
        );
        assert_eq!(
            promise_global_candidate.allows_async_execution(),
            promise_global_parent.allows_async_execution()
        );
        assert_eq!(uint8array_codecs_global_parent, promise_global_candidate);
        assert_eq!(
            uint8array_codecs_global_candidate
                .features
                .difference(&uint8array_codecs_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["uint8array-base64"]
        );
        assert!(
            uint8array_codecs_global_parent
                .features
                .difference(&uint8array_codecs_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            uint8array_codecs_global_candidate.audited_negative_tests,
            uint8array_codecs_global_parent.audited_negative_tests
        );
        assert_eq!(
            uint8array_codecs_global_candidate.allows_async_execution(),
            uint8array_codecs_global_parent.allows_async_execution()
        );
        assert_eq!(
            resizable_arraybuffer_global_parent,
            uint8array_codecs_global_candidate
        );
        assert_eq!(
            resizable_arraybuffer_global_candidate
                .features
                .difference(&resizable_arraybuffer_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["resizable-arraybuffer"]
        );
        assert!(
            resizable_arraybuffer_global_parent
                .features
                .difference(&resizable_arraybuffer_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            resizable_arraybuffer_global_candidate.audited_negative_tests,
            resizable_arraybuffer_global_parent.audited_negative_tests
        );
        assert_eq!(
            resizable_arraybuffer_global_candidate.allows_async_execution(),
            resizable_arraybuffer_global_parent.allows_async_execution()
        );
        assert_eq!(
            computed_property_names_global_parent,
            resizable_arraybuffer_global_candidate
        );
        assert_eq!(
            computed_property_names_global_candidate
                .features
                .difference(&computed_property_names_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["computed-property-names"]
        );
        assert!(
            computed_property_names_global_parent
                .features
                .difference(&computed_property_names_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            computed_property_names_global_candidate.audited_negative_tests,
            computed_property_names_global_parent.audited_negative_tests
        );
        assert_eq!(
            computed_property_names_global_candidate.allows_async_execution(),
            computed_property_names_global_parent.allows_async_execution()
        );
        assert_eq!(
            rest_parameters_parent,
            computed_property_names_global_candidate
        );
        assert_eq!(
            rest_parameters_candidate
                .features
                .difference(&rest_parameters_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["rest-parameters"]
        );
        assert!(
            rest_parameters_parent
                .features
                .difference(&rest_parameters_candidate.features)
                .next()
                .is_none()
        );
        let expected_rest_negatives = REST_PARAMETERS_ACTIVATION
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_rest_negatives.len(), 96);
        assert_eq!(
            rest_parameters_candidate
                .audited_negative_tests
                .difference(&rest_parameters_parent.audited_negative_tests)
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_rest_negatives
        );
        assert!(
            rest_parameters_parent
                .audited_negative_tests
                .difference(&rest_parameters_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            rest_parameters_candidate.allows_async_execution(),
            rest_parameters_parent.allows_async_execution()
        );
        assert_eq!(default_parameters_parent, rest_parameters_candidate);
        assert_eq!(data_view_global_parent, default_parameters_global_candidate);
        assert_eq!(object_rest_global_parent, data_view_global_candidate);
        assert_eq!(weak_collections_global_parent, object_rest_global_candidate);
        assert_eq!(
            weak_ref_finalization_global_parent,
            weak_collections_global_candidate
        );
        assert_eq!(
            host_gc_global_parent,
            weak_ref_finalization_global_candidate
        );
        assert_eq!(realm_hosts_global_parent, host_gc_global_candidate);
        assert_eq!(binary_data_global_parent, realm_hosts_global_candidate);
        assert_eq!(
            promise_try_with_resolvers_global_parent,
            binary_data_global_candidate
        );
        assert_eq!(
            html_comments_global_parent,
            promise_try_with_resolvers_global_candidate
        );
        assert_eq!(
            data_view_global_candidate
                .features
                .difference(&data_view_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["DataView"]
        );
        assert_eq!(
            data_view_global_candidate.audited_negative_tests,
            data_view_global_parent.audited_negative_tests
        );
        assert_eq!(
            data_view_global_candidate.allows_async_execution(),
            data_view_global_parent.allows_async_execution()
        );
        assert_eq!(
            object_rest_global_candidate
                .features
                .difference(&object_rest_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["object-rest"]
        );
        assert!(
            object_rest_global_parent
                .features
                .difference(&object_rest_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            object_rest_global_candidate
                .audited_negative_tests
                .difference(&object_rest_global_parent.audited_negative_tests)
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "test/language/expressions/assignment/dstr/obj-rest-not-last-element-invalid.js",
                "test/language/statements/for-in/dstr/obj-rest-not-last-element-invalid.js",
                "test/language/statements/for-of/dstr/obj-rest-not-last-element-invalid.js",
            ])
        );
        assert!(
            object_rest_global_parent
                .audited_negative_tests
                .difference(&object_rest_global_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            object_rest_global_candidate.allows_async_execution(),
            object_rest_global_parent.allows_async_execution()
        );
        assert_eq!(
            weak_collections_global_candidate
                .features
                .difference(&weak_collections_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["WeakMap", "WeakSet", "symbols-as-weakmap-keys", "upsert"]
        );
        assert!(
            weak_collections_global_parent
                .features
                .difference(&weak_collections_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            weak_collections_global_candidate.audited_negative_tests,
            weak_collections_global_parent.audited_negative_tests
        );
        assert_eq!(
            weak_collections_global_candidate.allows_async_execution(),
            weak_collections_global_parent.allows_async_execution()
        );
        assert_eq!(
            weak_ref_finalization_global_candidate
                .features
                .difference(&weak_ref_finalization_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["FinalizationRegistry", "WeakRef"]
        );
        assert!(
            weak_ref_finalization_global_parent
                .features
                .difference(&weak_ref_finalization_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            weak_ref_finalization_global_candidate.audited_negative_tests,
            weak_ref_finalization_global_parent.audited_negative_tests
        );
        assert_eq!(
            weak_ref_finalization_global_candidate.allows_async_execution(),
            weak_ref_finalization_global_parent.allows_async_execution()
        );
        assert_eq!(
            host_gc_global_candidate
                .features
                .difference(&host_gc_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["host-gc-required"]
        );
        assert!(
            host_gc_global_parent
                .features
                .difference(&host_gc_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            host_gc_global_candidate.audited_negative_tests,
            host_gc_global_parent.audited_negative_tests
        );
        assert_eq!(
            host_gc_global_candidate.allows_async_execution(),
            host_gc_global_parent.allows_async_execution()
        );
        assert_eq!(
            realm_hosts_global_candidate
                .features
                .difference(&realm_hosts_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["host-create-realm-required", "host-eval-script-required"]
        );
        assert!(
            realm_hosts_global_parent
                .features
                .difference(&realm_hosts_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            realm_hosts_global_candidate.audited_negative_tests,
            realm_hosts_global_parent.audited_negative_tests
        );
        assert_eq!(
            realm_hosts_global_candidate.allows_async_execution(),
            realm_hosts_global_parent.allows_async_execution()
        );
        assert_eq!(
            binary_data_global_candidate
                .features
                .difference(&binary_data_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "DataView.prototype.getFloat32",
                "DataView.prototype.getFloat64",
                "DataView.prototype.getInt16",
                "DataView.prototype.getInt32",
                "DataView.prototype.getInt8",
                "DataView.prototype.getUint16",
                "DataView.prototype.getUint32",
                "DataView.prototype.setUint8",
                "Float16Array",
                "Float32Array",
                "Float64Array",
                "Int16Array",
                "Int32Array",
                "Int8Array",
                "Uint16Array",
                "Uint32Array",
                "Uint8Array",
                "Uint8ClampedArray",
            ]
        );
        assert!(
            binary_data_global_parent
                .features
                .difference(&binary_data_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            binary_data_global_candidate.audited_negative_tests,
            binary_data_global_parent.audited_negative_tests
        );
        assert_eq!(
            binary_data_global_candidate.allows_async_execution(),
            binary_data_global_parent.allows_async_execution()
        );
        assert_eq!(
            promise_try_with_resolvers_global_candidate
                .features
                .difference(&promise_try_with_resolvers_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["promise-try", "promise-with-resolvers"]
        );
        assert!(
            promise_try_with_resolvers_global_parent
                .features
                .difference(&promise_try_with_resolvers_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            promise_try_with_resolvers_global_candidate.audited_negative_tests,
            promise_try_with_resolvers_global_parent.audited_negative_tests
        );
        assert_eq!(
            promise_try_with_resolvers_global_candidate.allows_async_execution(),
            promise_try_with_resolvers_global_parent.allows_async_execution()
        );
        assert_eq!(
            html_comments_global_candidate.features,
            html_comments_global_parent.features
        );
        let expected_html_comment_negatives = HTML_COMMENTS_GLOBAL_ADDED_NEGATIVES
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_html_comment_negatives.len(), 10);
        assert_eq!(
            html_comments_global_candidate
                .audited_negative_tests
                .difference(&html_comments_global_parent.audited_negative_tests)
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_html_comment_negatives
        );
        assert!(
            html_comments_global_parent
                .audited_negative_tests
                .difference(&html_comments_global_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            html_comments_global_candidate.allows_async_execution(),
            html_comments_global_parent.allows_async_execution()
        );
        assert_eq!(
            debugger_statement_global_parent,
            html_comments_global_candidate
        );
        assert_eq!(
            debugger_statement_global_candidate.features,
            debugger_statement_global_parent.features
        );
        let expected_debugger_statement_negatives = DEBUGGER_STATEMENT_GLOBAL_ADDED_NEGATIVES
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_debugger_statement_negatives.len(), 5);
        assert_eq!(
            debugger_statement_global_candidate
                .audited_negative_tests
                .difference(&debugger_statement_global_parent.audited_negative_tests)
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_debugger_statement_negatives
        );
        assert!(
            debugger_statement_global_parent
                .audited_negative_tests
                .difference(&debugger_statement_global_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            debugger_statement_global_candidate.allows_async_execution(),
            debugger_statement_global_parent.allows_async_execution()
        );
        assert_eq!(
            future_reserved_words_global_parent,
            debugger_statement_global_candidate
        );
        assert_eq!(
            FUTURE_RESERVED_WORDS_GLOBAL_PARENT_PROFILE.lines().count(),
            1309
        );
        assert_eq!(
            FUTURE_RESERVED_WORDS_GLOBAL_CANDIDATE_PROFILE
                .lines()
                .count(),
            1334
        );
        assert_eq!(
            future_reserved_words_global_parent
                .audited_negative_tests
                .len(),
            1172
        );
        assert_eq!(
            future_reserved_words_global_candidate
                .audited_negative_tests
                .len(),
            1197
        );
        assert_eq!(
            future_reserved_words_global_candidate.features,
            future_reserved_words_global_parent.features
        );
        let expected_future_reserved_word_negatives = FUTURE_RESERVED_WORDS_GLOBAL_ADDED_NEGATIVES
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_future_reserved_word_negatives.len(), 25);
        assert_eq!(
            future_reserved_words_global_candidate
                .audited_negative_tests
                .difference(&future_reserved_words_global_parent.audited_negative_tests)
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_future_reserved_word_negatives
        );
        assert!(
            future_reserved_words_global_parent
                .audited_negative_tests
                .difference(&future_reserved_words_global_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            future_reserved_words_global_candidate.allows_async_execution(),
            future_reserved_words_global_parent.allows_async_execution()
        );
        assert_eq!(
            atomics_pause_global_parent
                .features
                .difference(&future_reserved_words_global_candidate.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["Array.prototype.flat", "Array.prototype.flatMap"]
        );
        assert!(
            future_reserved_words_global_candidate
                .features
                .difference(&atomics_pause_global_parent.features)
                .next()
                .is_none()
        );
        assert_eq!(
            atomics_pause_global_parent.audited_negative_tests,
            future_reserved_words_global_candidate.audited_negative_tests
        );
        assert_eq!(
            atomics_pause_global_parent.allows_async_execution(),
            future_reserved_words_global_candidate.allows_async_execution()
        );
        assert_eq!(
            atomics_pause_global_candidate
                .features
                .difference(&atomics_pause_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["Atomics.pause"]
        );
        assert!(
            atomics_pause_global_parent
                .features
                .difference(&atomics_pause_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            atomics_pause_global_candidate.audited_negative_tests,
            atomics_pause_global_parent.audited_negative_tests
        );
        assert_eq!(
            atomics_pause_global_candidate.allows_async_execution(),
            atomics_pause_global_parent.allows_async_execution()
        );
        assert_eq!(
            error_regexp_typedarray_global_candidate
                .features
                .difference(&atomics_pause_global_candidate.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["Error.isError", "RegExp.escape", "TypedArray.prototype.at"]
        );
        assert!(
            atomics_pause_global_candidate
                .features
                .difference(&error_regexp_typedarray_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            error_regexp_typedarray_global_candidate.audited_negative_tests,
            atomics_pause_global_candidate.audited_negative_tests
        );
        assert_eq!(
            error_regexp_typedarray_global_candidate.allows_async_execution(),
            atomics_pause_global_candidate.allows_async_execution()
        );
        assert_eq!(
            shared_atomics_global_parent,
            error_regexp_typedarray_global_candidate
        );
        assert_eq!(
            shared_atomics_global_candidate
                .features
                .difference(&shared_atomics_global_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["Atomics", "SharedArrayBuffer"]
        );
        assert!(
            shared_atomics_global_parent
                .features
                .difference(&shared_atomics_global_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            shared_atomics_global_candidate.audited_negative_tests,
            shared_atomics_global_parent.audited_negative_tests
        );
        assert_eq!(
            shared_atomics_global_candidate.allows_async_execution(),
            shared_atomics_global_parent.allows_async_execution()
        );
        let mut agent_stage_a_global_candidate = shared_atomics_global_candidate.clone();
        assert!(agent_stage_a_global_candidate.host_agent_tests.is_empty());
        assert!(
            agent_stage_a_global_candidate
                .host_agent_tests
                .insert("test/built-ins/Atomics/wait/good-views.js".to_owned())
        );
        assert_eq!(
            agent_stage_a_global_candidate,
            OxideProfile::parse(AGENT_STAGE_A_GLOBAL_CANDIDATE_PROFILE).unwrap()
        );
        let mut agent_broadcast_a_global_candidate = agent_stage_a_global_candidate;
        agent_broadcast_a_global_candidate
            .host_agent_tests
            .extend(AGENT_BROADCAST_A_ACTIVATION.lines().map(str::to_owned));
        assert_eq!(
            agent_broadcast_a_global_candidate,
            OxideProfile::parse(AGENT_BROADCAST_A_GLOBAL_CANDIDATE_PROFILE).unwrap()
        );
        let mut agent_wait_bounded_a_global_candidate = agent_broadcast_a_global_candidate;
        agent_wait_bounded_a_global_candidate
            .host_agent_tests
            .extend(AGENT_WAIT_BOUNDED_A_ACTIVATION.lines().map(str::to_owned));
        assert_eq!(
            agent_wait_bounded_a_global_candidate,
            OxideProfile::parse(AGENT_WAIT_BOUNDED_A_GLOBAL_CANDIDATE_PROFILE).unwrap()
        );
        let mut agent_wake_fifo_global_candidate = agent_wait_bounded_a_global_candidate;
        agent_wake_fifo_global_candidate.host_agent_tests.extend(
            AGENT_WAKE_COUNT_LOCATION_ACTIVATION
                .lines()
                .chain(AGENT_FIFO_WAKE_ORDER_ACTIVATION.lines())
                .map(str::to_owned),
        );
        assert_eq!(profile, agent_wake_fifo_global_candidate);
        assert_eq!(CHECKED_IN_PROFILE, AGENT_WAKE_FIFO_GLOBAL_CANDIDATE_PROFILE);
        assert_eq!(
            default_parameters_candidate
                .features
                .difference(&default_parameters_parent.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["default-parameters"]
        );
        assert!(
            default_parameters_parent
                .features
                .difference(&default_parameters_candidate.features)
                .next()
                .is_none()
        );
        assert_eq!(
            default_parameters_candidate
                .audited_negative_tests
                .difference(&default_parameters_parent.audited_negative_tests)
                .count(),
            219
        );
        assert!(
            default_parameters_parent
                .audited_negative_tests
                .difference(&default_parameters_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            default_parameters_candidate.allows_async_execution(),
            default_parameters_parent.allows_async_execution()
        );
        assert_eq!(
            default_parameters_global_candidate.features,
            default_parameters_candidate.features
        );
        let expected_strict_body_negatives = DEFAULT_PARAMETERS_STRICT_BODY
            .lines()
            .filter(|line| !line.is_empty() && !line.starts_with('#'))
            .filter(|path| {
                !default_parameters_candidate
                    .audited_negative_tests
                    .contains(*path)
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_strict_body_negatives.len(), 11);
        assert_eq!(
            default_parameters_global_candidate
                .audited_negative_tests
                .difference(&default_parameters_candidate.audited_negative_tests)
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected_strict_body_negatives
        );
        assert!(
            default_parameters_candidate
                .audited_negative_tests
                .difference(&default_parameters_global_candidate.audited_negative_tests)
                .next()
                .is_none()
        );
        assert_eq!(
            default_parameters_global_candidate.allows_async_execution(),
            default_parameters_candidate.allows_async_execution()
        );
    }

    #[test]
    fn atomics_non_shared_core_profile_is_exact_and_selection_only() {
        let historical_global = OxideProfile::parse(ATOMICS_PAUSE_GLOBAL_PARENT_PROFILE).unwrap();
        let scoped = OxideProfile::parse(ATOMICS_NON_SHARED_CORE_PROFILE).unwrap();

        assert_eq!(
            scoped
                .features
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "Array.prototype.includes",
                "ArrayBuffer",
                "Atomics",
                "Atomics.pause",
                "BigInt",
                "Reflect.construct",
                "SharedArrayBuffer",
                "Symbol",
                "Symbol.toStringTag",
                "TypedArray",
                "resizable-arraybuffer",
            ]
        );
        assert_eq!(
            scoped
                .features
                .difference(&historical_global.features)
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec!["Atomics", "Atomics.pause", "SharedArrayBuffer"]
        );
        assert!(scoped.audited_negative_tests.is_empty());
        assert!(!scoped.allows_async_execution());
    }

    #[test]
    fn shared_array_buffer_core_profile_is_exact_and_selection_only() {
        let scoped = OxideProfile::parse(SHARED_ARRAY_BUFFER_CORE_PROFILE).unwrap();

        assert_eq!(
            scoped
                .features
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "ArrayBuffer",
                "BigInt",
                "DataView",
                "Int8Array",
                "Reflect",
                "Reflect.construct",
                "SharedArrayBuffer",
                "Symbol",
                "Symbol.species",
                "Symbol.toStringTag",
                "TypedArray",
                "align-detached-buffer-semantics-with-web-reality",
                "arraybuffer-transfer",
                "arrow-function",
                "cross-realm",
                "host-create-realm-required",
                "resizable-arraybuffer",
            ]
        );
        assert!(scoped.audited_negative_tests.is_empty());
        assert!(!scoped.allows_async_execution());
    }

    #[test]
    fn shared_atomics_nonblocking_profile_is_exact_and_selection_only() {
        let scoped = OxideProfile::parse(SHARED_ATOMICS_NONBLOCKING_PROFILE).unwrap();

        assert_eq!(
            scoped
                .features
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "ArrayBuffer",
                "Atomics",
                "BigInt",
                "DataView",
                "Float32Array",
                "Float64Array",
                "Int8Array",
                "Reflect.construct",
                "SharedArrayBuffer",
                "Symbol",
                "Symbol.toPrimitive",
                "TypedArray",
                "Uint16Array",
                "Uint8Array",
                "Uint8ClampedArray",
                "arrow-function",
                "resizable-arraybuffer",
            ]
        );
        assert!(scoped.audited_negative_tests.is_empty());
        assert!(!scoped.allows_async_execution());
    }

    #[test]
    fn atomics_wait_nonagent_bounded_profile_is_exact() {
        let scoped = OxideProfile::parse(ATOMICS_WAIT_NONAGENT_BOUNDED_PROFILE).unwrap();

        assert_eq!(
            scoped
                .features
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            vec![
                "ArrayBuffer",
                "Atomics",
                "BigInt",
                "DataView",
                "Float32Array",
                "Float64Array",
                "Int8Array",
                "SharedArrayBuffer",
                "Symbol",
                "Symbol.toPrimitive",
                "TypedArray",
                "Uint16Array",
                "Uint8Array",
                "Uint8ClampedArray",
                "resizable-arraybuffer",
            ]
        );
        assert!(scoped.audited_negative_tests.is_empty());
        assert!(!scoped.allows_async_execution());
    }

    #[test]
    fn future_reserved_words_scoped_profile_is_exact_and_fail_closed() {
        let profile = OxideProfile::parse(FUTURE_RESERVED_WORDS_SCOPED_PROFILE).unwrap();
        let expected = FUTURE_RESERVED_WORDS_NEGATIVE
            .lines()
            .collect::<BTreeSet<_>>();

        assert!(profile.features.is_empty());
        assert!(!profile.allows_async_execution());
        assert_eq!(profile.audited_negative_tests.len(), 26);
        assert_eq!(
            profile
                .audited_negative_tests
                .iter()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            expected
        );
        assert_eq!(
            profile.classify(
                Path::new("test/language/future-reserved-words/enum.js"),
                &[],
                true,
            ),
            None
        );
        assert!(
            profile
                .classify(
                    Path::new("test/language/future-reserved-words/not-pinned.js"),
                    &[],
                    true,
                )
                .is_some()
        );
        assert_eq!(
            profile.classify(
                Path::new("test/staging/sm/misc/future-reserved-words.js"),
                &[],
                false,
            ),
            None
        );
    }

    #[test]
    fn feature_gaps_are_deduplicated_and_sorted_before_negative_provenance() {
        let profile = OxideProfile::parse(CHECKED_IN_PROFILE).unwrap();
        let classification = profile
            .classify(
                Path::new("test/not-audited.js"),
                &[
                    "class".to_owned(),
                    "class-fields-private".to_owned(),
                    "class".to_owned(),
                ],
                true,
            )
            .unwrap();

        assert_eq!(classification.outcome, "unsupported-feature");
        assert_eq!(
            classification.detail,
            "quickjs-oxide does not declare Test262 feature support: class, class-fields-private"
        );
    }

    #[test]
    fn unaudited_negatives_fail_closed_but_positive_tests_do_not() {
        let profile = OxideProfile::parse(CHECKED_IN_PROFILE).unwrap();
        let path = Path::new("test/language/expressions/arrow-function/params-duplicate.js");
        let classification = profile.classify(path, &[], true).unwrap();

        assert_eq!(classification.outcome, "unsupported-negative-provenance");
        assert_eq!(
            classification.detail,
            "negative Test262 path has not been audited: test/language/expressions/arrow-function/params-duplicate.js"
        );
        assert_eq!(profile.classify(path, &[], false), None);
    }

    #[test]
    fn audited_negative_paths_are_exact() {
        let profile = OxideProfile::parse(CHECKED_IN_PROFILE).unwrap();
        let path = Path::new("test/language/statements/variable/S12.2_A8_T2.js");

        assert_eq!(profile.classify(path, &[], true), None);
        assert!(
            profile
                .classify(
                    Path::new("./test/language/statements/variable/S12.2_A8_T2.js"),
                    &[],
                    true,
                )
                .is_some()
        );
    }

    #[test]
    fn parser_rejects_unknown_duplicate_missing_and_out_of_order_sections() {
        let unknown = "[features]\nBigInt\n[unknown]\ntest/a.js\n";
        assert!(
            OxideProfile::parse(unknown)
                .unwrap_err()
                .contains("unknown section [unknown]")
        );

        let duplicate = "[features]\nBigInt\n[features]\nSymbol\n[audited-negative-tests]\n";
        assert!(
            OxideProfile::parse(duplicate)
                .unwrap_err()
                .contains("duplicate section [features]")
        );

        let missing = "[features]\nBigInt\n";
        assert_eq!(
            OxideProfile::parse(missing).unwrap_err(),
            "missing required section [audited-negative-tests]"
        );

        let reversed = "[audited-negative-tests]\ntest/a.js\n[features]\nBigInt\n";
        assert!(
            OxideProfile::parse(reversed)
                .unwrap_err()
                .contains("section [features] is out of order")
        );
    }

    #[test]
    fn parser_rejects_duplicate_unsorted_and_malformed_entries() {
        let duplicate = "[features]\nBigInt\nBigInt\n[audited-negative-tests]\ntest/a.js\n";
        assert!(
            OxideProfile::parse(duplicate)
                .unwrap_err()
                .contains("duplicate entry \"BigInt\"")
        );

        let unsorted = "[features]\nSymbol\nBigInt\n[audited-negative-tests]\ntest/a.js\n";
        assert!(
            OxideProfile::parse(unsorted)
                .unwrap_err()
                .contains("entry \"BigInt\" is out of order")
        );

        let malformed = "[features]\nBigInt = yes\n[audited-negative-tests]\ntest/a.js\n";
        assert!(
            OxideProfile::parse(malformed)
                .unwrap_err()
                .contains("malformed entry")
        );

        let indented = "[features]\n BigInt\n[audited-negative-tests]\ntest/a.js\n";
        assert!(
            OxideProfile::parse(indented)
                .unwrap_err()
                .contains("leading or trailing whitespace")
        );

        let invalid_path = "[features]\nBigInt\n[audited-negative-tests]\ntest/../escape.js\n";
        assert!(
            OxideProfile::parse(invalid_path)
                .unwrap_err()
                .contains("must be an exact test/*.js path")
        );
    }

    #[test]
    fn optional_execution_section_enables_only_the_async_host() {
        let source = "[features]\nPromise\n[audited-negative-tests]\n[execution]\nasync=true\n";
        let profile = OxideProfile::parse(source).unwrap();
        assert!(profile.allows_async_execution());

        for invalid in [
            "[features]\nPromise\n[audited-negative-tests]\n[execution]\n",
            "[features]\nPromise\n[audited-negative-tests]\n[execution]\nasync=false\n",
            "[features]\nPromise\n[audited-negative-tests]\n[execution]\nasync=true\nasync=true\n",
        ] {
            assert!(
                OxideProfile::parse(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn optional_agent_section_is_exact_sorted_and_can_skip_execution() {
        let source = "[features]\nAtomics\n[audited-negative-tests]\n[host-agent-tests]\ntest/built-ins/Atomics/wait/good-views.js\n";
        let profile = OxideProfile::parse(source).unwrap();
        assert!(!profile.allows_async_execution());
        assert!(profile.allows_agent_host(Path::new("test/built-ins/Atomics/wait/good-views.js")));
        assert!(!profile.allows_agent_host(Path::new("test/built-ins/Atomics/wait/wake.js")));

        for invalid in [
            "[features]\nAtomics\n[audited-negative-tests]\n[host-agent-tests]\ntest/b.js\ntest/a.js\n",
            "[features]\nAtomics\n[audited-negative-tests]\n[host-agent-tests]\ntest/a.js\ntest/a.js\n",
            "[features]\nAtomics\n[audited-negative-tests]\n[host-agent-tests]\n../test/a.js\n",
            "[features]\nAtomics\n[audited-negative-tests]\n[host-agent-tests]\ntest/a.js\n[execution]\nasync=true\n",
        ] {
            assert!(
                OxideProfile::parse(invalid).is_err(),
                "accepted {invalid:?}"
            );
        }
    }

    #[test]
    fn agent_broadcast_a_profiles_add_only_the_exact_activation_allowlist() {
        let parent = OxideProfile::parse(AGENT_BROADCAST_A_PARENT_PROFILE).unwrap();
        let candidate = OxideProfile::parse(AGENT_BROADCAST_A_CANDIDATE_PROFILE).unwrap();
        let global_parent = OxideProfile::parse(AGENT_BROADCAST_A_GLOBAL_PARENT_PROFILE).unwrap();
        let global_candidate =
            OxideProfile::parse(AGENT_BROADCAST_A_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let activation = AGENT_BROADCAST_A_ACTIVATION
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(activation.len(), 15);
        assert_eq!(
            global_parent,
            OxideProfile::parse(AGENT_STAGE_A_GLOBAL_CANDIDATE_PROFILE).unwrap()
        );
        for (baseline, admitted) in [(&parent, &candidate), (&global_parent, &global_candidate)] {
            assert_eq!(admitted.features, baseline.features);
            assert_eq!(
                admitted.audited_negative_tests,
                baseline.audited_negative_tests
            );
            assert_eq!(
                admitted.allows_async_execution(),
                baseline.allows_async_execution()
            );
            assert_eq!(
                admitted
                    .host_agent_tests
                    .difference(&baseline.host_agent_tests)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                activation
            );
            assert!(
                baseline
                    .host_agent_tests
                    .difference(&admitted.host_agent_tests)
                    .next()
                    .is_none()
            );
            assert!(
                baseline.allows_agent_host(Path::new("test/built-ins/Atomics/wait/good-views.js"))
            );
        }
    }

    #[test]
    fn agent_wait_bounded_a_profiles_add_only_the_exact_activation_allowlist() {
        let predecessor = OxideProfile::parse(AGENT_BROADCAST_A_CANDIDATE_PROFILE).unwrap();
        let parent = OxideProfile::parse(AGENT_WAIT_BOUNDED_A_PARENT_PROFILE).unwrap();
        let candidate = OxideProfile::parse(AGENT_WAIT_BOUNDED_A_CANDIDATE_PROFILE).unwrap();
        let global_predecessor =
            OxideProfile::parse(AGENT_BROADCAST_A_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let global_parent =
            OxideProfile::parse(AGENT_WAIT_BOUNDED_A_GLOBAL_PARENT_PROFILE).unwrap();
        let global_candidate =
            OxideProfile::parse(AGENT_WAIT_BOUNDED_A_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let activation = AGENT_WAIT_BOUNDED_A_ACTIVATION
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(activation.len(), 22);
        assert_eq!(parent, predecessor);
        assert_eq!(candidate.features, parent.features);
        assert_eq!(
            candidate.audited_negative_tests,
            parent.audited_negative_tests
        );
        assert_eq!(
            candidate.allows_async_execution(),
            parent.allows_async_execution()
        );
        assert_eq!(
            candidate
                .host_agent_tests
                .difference(&parent.host_agent_tests)
                .cloned()
                .collect::<BTreeSet<_>>(),
            activation
        );
        assert!(
            parent
                .host_agent_tests
                .difference(&candidate.host_agent_tests)
                .next()
                .is_none()
        );
        assert_eq!(global_parent, global_predecessor);
        assert_eq!(global_candidate.features, global_parent.features);
        assert_eq!(
            global_candidate.audited_negative_tests,
            global_parent.audited_negative_tests
        );
        assert_eq!(
            global_candidate.allows_async_execution(),
            global_parent.allows_async_execution()
        );
        assert_eq!(
            global_candidate
                .host_agent_tests
                .difference(&global_parent.host_agent_tests)
                .cloned()
                .collect::<BTreeSet<_>>(),
            activation
        );
        assert!(
            global_parent
                .host_agent_tests
                .difference(&global_candidate.host_agent_tests)
                .next()
                .is_none()
        );
    }

    #[test]
    fn agent_wake_count_location_profiles_add_only_the_exact_activation_allowlist() {
        let predecessor = OxideProfile::parse(AGENT_WAIT_BOUNDED_A_CANDIDATE_PROFILE).unwrap();
        let parent = OxideProfile::parse(AGENT_WAKE_COUNT_LOCATION_PARENT_PROFILE).unwrap();
        let candidate = OxideProfile::parse(AGENT_WAKE_COUNT_LOCATION_CANDIDATE_PROFILE).unwrap();
        let activation = AGENT_WAKE_COUNT_LOCATION_ACTIVATION
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(activation.len(), 17);
        assert_eq!(parent, predecessor);
        assert_eq!(candidate.features, parent.features);
        assert_eq!(
            candidate.audited_negative_tests,
            parent.audited_negative_tests
        );
        assert_eq!(
            candidate.allows_async_execution(),
            parent.allows_async_execution()
        );
        assert_eq!(
            candidate
                .host_agent_tests
                .difference(&parent.host_agent_tests)
                .cloned()
                .collect::<BTreeSet<_>>(),
            activation
        );
        assert!(
            parent
                .host_agent_tests
                .difference(&candidate.host_agent_tests)
                .next()
                .is_none()
        );
        assert_eq!(parent.host_agent_tests.len(), 38);
        assert_eq!(candidate.host_agent_tests.len(), 55);
    }

    #[test]
    fn agent_fifo_wake_order_profiles_add_only_the_exact_activation_allowlist() {
        let predecessor = OxideProfile::parse(AGENT_WAKE_COUNT_LOCATION_CANDIDATE_PROFILE).unwrap();
        let parent = OxideProfile::parse(AGENT_FIFO_WAKE_ORDER_PARENT_PROFILE).unwrap();
        let candidate = OxideProfile::parse(AGENT_FIFO_WAKE_ORDER_CANDIDATE_PROFILE).unwrap();
        let activation = AGENT_FIFO_WAKE_ORDER_ACTIVATION
            .lines()
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(activation.len(), 4);
        assert_eq!(parent, predecessor);
        assert_eq!(candidate.features, parent.features);
        assert_eq!(
            candidate.audited_negative_tests,
            parent.audited_negative_tests
        );
        assert_eq!(
            candidate.allows_async_execution(),
            parent.allows_async_execution()
        );
        assert_eq!(
            candidate
                .host_agent_tests
                .difference(&parent.host_agent_tests)
                .cloned()
                .collect::<BTreeSet<_>>(),
            activation
        );
        assert!(
            parent
                .host_agent_tests
                .difference(&candidate.host_agent_tests)
                .next()
                .is_none()
        );
        assert_eq!(parent.host_agent_tests.len(), 55);
        assert_eq!(candidate.host_agent_tests.len(), 59);
    }

    #[test]
    fn agent_wake_fifo_global_profiles_add_only_the_exact_21_path_allowlist() {
        let predecessor =
            OxideProfile::parse(AGENT_WAIT_BOUNDED_A_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let parent = OxideProfile::parse(AGENT_WAKE_FIFO_GLOBAL_PARENT_PROFILE).unwrap();
        let candidate = OxideProfile::parse(AGENT_WAKE_FIFO_GLOBAL_CANDIDATE_PROFILE).unwrap();
        let scoped_candidate =
            OxideProfile::parse(AGENT_FIFO_WAKE_ORDER_CANDIDATE_PROFILE).unwrap();
        let activation = AGENT_WAKE_COUNT_LOCATION_ACTIVATION
            .lines()
            .chain(AGENT_FIFO_WAKE_ORDER_ACTIVATION.lines())
            .map(str::to_owned)
            .collect::<BTreeSet<_>>();

        assert_eq!(activation.len(), 21);
        assert_eq!(parent, predecessor);
        assert_eq!(candidate.features, parent.features);
        assert_eq!(
            candidate.audited_negative_tests,
            parent.audited_negative_tests
        );
        assert_eq!(
            candidate.allows_async_execution(),
            parent.allows_async_execution()
        );
        assert_eq!(
            candidate
                .host_agent_tests
                .difference(&parent.host_agent_tests)
                .cloned()
                .collect::<BTreeSet<_>>(),
            activation
        );
        assert!(
            parent
                .host_agent_tests
                .difference(&candidate.host_agent_tests)
                .next()
                .is_none()
        );
        assert_eq!(parent.host_agent_tests.len(), 38);
        assert_eq!(candidate.host_agent_tests.len(), 59);
        assert_eq!(
            candidate.host_agent_tests,
            scoped_candidate.host_agent_tests
        );
    }

    #[test]
    fn parser_requires_entries_to_follow_the_fixed_section_order() {
        let source = "BigInt\n[features]\n[audited-negative-tests]\n";
        assert!(
            OxideProfile::parse(source)
                .unwrap_err()
                .contains("entry appears before the [features] section")
        );
        assert_eq!(
            SECTION_ORDER,
            [
                FEATURES_SECTION,
                AUDITED_NEGATIVE_TESTS_SECTION,
                EXECUTION_SECTION,
                HOST_AGENT_TESTS_SECTION,
            ]
        );
    }
}
