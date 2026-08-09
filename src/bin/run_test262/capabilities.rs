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

    pub(super) fn agent_host_paths(&self) -> impl Iterator<Item = &str> {
        self.host_agent_tests.iter().map(String::as_str)
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
    use std::path::Path;

    use super::{
        AUDITED_NEGATIVE_TESTS_SECTION, EXECUTION_SECTION, FEATURES_SECTION,
        HOST_AGENT_TESTS_SECTION, OxideProfile, SECTION_ORDER,
    };

    const DATA_PROFILE: &str = "\
[features]
ArrayBuffer
Atomics

[audited-negative-tests]
test/language/negative-a.js
test/language/negative-b.js

[execution]
async=true

[host-agent-tests]
test/built-ins/Atomics/notify/notify-one.js
test/built-ins/Atomics/wait/good-views.js
";

    fn parse_error(source: &str) -> String {
        OxideProfile::parse(source).expect_err("profile unexpectedly parsed")
    }

    #[test]
    fn parser_accepts_required_and_optional_data_sections() {
        let profile = OxideProfile::parse(DATA_PROFILE).unwrap();

        assert!(profile.features.contains("ArrayBuffer"));
        assert!(profile.features.contains("Atomics"));
        assert!(profile.allows_async_execution());
        assert_eq!(
            profile.audited_negative_paths().collect::<Vec<_>>(),
            ["test/language/negative-a.js", "test/language/negative-b.js",]
        );
        assert_eq!(
            profile.agent_host_paths().collect::<Vec<_>>(),
            [
                "test/built-ins/Atomics/notify/notify-one.js",
                "test/built-ins/Atomics/wait/good-views.js",
            ]
        );
        assert!(profile.allows_agent_host(Path::new(r"test\built-ins\Atomics\wait\good-views.js")));
    }

    #[test]
    fn parser_requires_known_unique_sections_in_fixed_order() {
        let cases = [
            (
                "[features]\nArrayBuffer\n",
                "missing required section [audited-negative-tests]",
            ),
            (
                "[features]\n[audited-negative-tests]\n[unknown]\n",
                "unknown section [unknown]",
            ),
            (
                "[features]\n[audited-negative-tests]\n[features]\n",
                "duplicate section [features]",
            ),
            (
                "[audited-negative-tests]\n[features]\n",
                "section [features] is out of order",
            ),
            (
                "ArrayBuffer\n[features]\n[audited-negative-tests]\n",
                "entry appears before the [features] section",
            ),
        ];

        for (source, expected) in cases {
            assert!(
                parse_error(source).contains(expected),
                "{source:?} did not report {expected:?}"
            );
        }
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

    #[test]
    fn parser_rejects_noncanonical_entries_and_execution_values() {
        let cases = [
            (
                "[features]\nArrayBuffer\n \n[audited-negative-tests]\n",
                "whitespace-only lines are not allowed",
            ),
            (
                " [features]\n[audited-negative-tests]\n",
                "leading or trailing whitespace is not allowed",
            ),
            (
                "[features]\nAtomics\nAtomics\n[audited-negative-tests]\n",
                "duplicate entry",
            ),
            (
                "[features]\nAtomics\nArrayBuffer\n[audited-negative-tests]\n",
                "is out of order",
            ),
            (
                "[features]\nfeature=true\n[audited-negative-tests]\n",
                "malformed entry",
            ),
            (
                "[features]\n[audited-negative-tests]\ntest/../escape.js\n",
                "must be an exact test/*.js path",
            ),
            (
                "[features]\n[audited-negative-tests]\n[execution]\n",
                "must contain async=true",
            ),
            (
                "[features]\n[audited-negative-tests]\n[execution]\nasync=false\n",
                "only accepts async=true",
            ),
            (
                "[features]\n[audited-negative-tests]\n[execution]\nasync=true\nasync=true\n",
                "duplicate entry",
            ),
            (
                "[features]\n[audited-negative-tests]\n[host-agent-tests]\ntest\\bad.js\n",
                "must be an exact test/*.js path",
            ),
        ];

        for (source, expected) in cases {
            assert!(
                parse_error(source).contains(expected),
                "{source:?} did not report {expected:?}"
            );
        }
    }

    #[test]
    fn feature_gaps_are_fail_closed_deduplicated_and_sorted() {
        let profile = OxideProfile::parse("[features]\nknown\n[audited-negative-tests]\n").unwrap();
        let classification = profile
            .classify(
                Path::new("test/language/positive.js"),
                &[
                    "zeta".to_owned(),
                    "known".to_owned(),
                    "alpha".to_owned(),
                    "zeta".to_owned(),
                ],
                false,
            )
            .unwrap();

        assert_eq!(classification.outcome, "unsupported-feature");
        assert_eq!(
            classification.detail,
            "quickjs-oxide does not declare Test262 feature support: alpha, zeta"
        );
    }

    #[test]
    fn negative_provenance_is_exact_while_positive_tests_remain_eligible() {
        let profile = OxideProfile::parse(
            "[features]\nknown\n[audited-negative-tests]\ntest/language/a.js\n",
        )
        .unwrap();

        assert!(
            profile
                .classify(
                    Path::new(r"test\language\a.js"),
                    &["known".to_owned()],
                    true,
                )
                .is_none()
        );
        assert!(
            profile
                .classify(
                    Path::new("test/language/positive.js"),
                    &["known".to_owned()],
                    false,
                )
                .is_none()
        );
        let classification = profile
            .classify(
                Path::new("test/language/unaudited.js"),
                &["known".to_owned()],
                true,
            )
            .unwrap();
        assert_eq!(classification.outcome, "unsupported-negative-provenance");
        assert_eq!(
            classification.detail,
            "negative Test262 path has not been audited: test/language/unaudited.js"
        );
    }

    #[test]
    fn checked_in_profile_is_the_current_data_source() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("compat/test262-oxide.conf");
        let profile = OxideProfile::load(&path).unwrap();

        assert!(profile.allows_async_execution());
        assert!(profile.features.contains("ArrayBuffer"));
        assert!(profile.features.contains("Atomics"));
        assert!(profile.features.contains("Promise"));
        assert!(profile.audited_negative_paths().next().is_some());
        assert!(profile.agent_host_paths().next().is_some());
        assert!(
            profile.allows_agent_host(Path::new("test/built-ins/Atomics/notify/notify-one.js"))
        );
        assert!(!profile.allows_agent_host(Path::new("test/built-ins/Atomics/notify/unlisted.js")));
    }
}
