use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

const FEATURES_SECTION: &str = "features";
const AUDITED_NEGATIVE_TESTS_SECTION: &str = "audited-negative-tests";
const SECTION_ORDER: [&str; 2] = [FEATURES_SECTION, AUDITED_NEGATIVE_TESTS_SECTION];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct FailClosedClassification {
    pub(super) outcome: &'static str,
    pub(super) detail: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(super) struct OxideProfile {
    features: BTreeSet<String>,
    audited_negative_tests: BTreeSet<String>,
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
        let mut previous_entry: Option<String> = None;

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
                let expected_index = seen_sections.len() - 1;
                if SECTION_ORDER[expected_index] != name {
                    return Err(format!(
                        "line {line_number}: section [{name}] is out of order; expected [{}]",
                        SECTION_ORDER[expected_index]
                    ));
                }
                section_index = Some(expected_index);
                previous_entry = None;
                continue;
            }

            let Some(current_section) = section_index else {
                return Err(format!(
                    "line {line_number}: entry appears before the [features] section"
                ));
            };
            validate_entry(line, current_section, line_number)?;

            let entries = if current_section == 0 {
                &mut profile.features
            } else {
                &mut profile.audited_negative_tests
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

        for section in SECTION_ORDER {
            if !seen_sections.contains(section) {
                return Err(format!("missing required section [{section}]"));
            }
        }
        Ok(profile)
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
    if section_index == 1
        && (!entry.starts_with("test/")
            || !entry.ends_with(".js")
            || entry.contains('\\')
            || entry
                .split('/')
                .any(|component| component.is_empty() || matches!(component, "." | "..")))
    {
        return Err(format!(
            "line {line_number}: audited negative test must be an exact test/*.js path: {entry:?}"
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

    use super::{AUDITED_NEGATIVE_TESTS_SECTION, FEATURES_SECTION, OxideProfile, SECTION_ORDER};

    const CHECKED_IN_PROFILE: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/compat/test262-oxide.conf"
    ));
    const PROPERTY_MANIFEST: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/test262-regexp-unicode-properties.txt"
    ));
    const PROPERTY_POSITIVE_PATHS: [&str; 2] = [
        "test/built-ins/RegExp/property-escapes/character-class.js",
        "test/built-ins/RegExp/property-escapes/special-property-value-Script_Extensions-Unknown.js",
    ];
    const EXPECTED_FEATURES: [&str; 55] = [
        "Array.prototype.at",
        "Array.prototype.includes",
        "BigInt",
        "Math.sumPrecise",
        "Object.fromEntries",
        "Object.hasOwn",
        "Reflect",
        "Reflect.construct",
        "Reflect.set",
        "Reflect.setPrototypeOf",
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
        "Symbol.isConcatSpreadable",
        "Symbol.match",
        "Symbol.matchAll",
        "Symbol.replace",
        "Symbol.search",
        "Symbol.split",
        "__getter__",
        "__proto__",
        "__setter__",
        "array-find-from-last",
        "arrow-function",
        "change-array-by-copy",
        "coalesce-expression",
        "const",
        "exponentiation",
        "for-in-order",
        "hashbang",
        "let",
        "logical-assignment-operators",
        "new.target",
        "numeric-separator-literal",
        "object-spread",
        "optional-catch-binding",
        "regexp-dotall",
        "regexp-duplicate-named-groups",
        "regexp-lookbehind",
        "regexp-match-indices",
        "regexp-modifiers",
        "regexp-named-groups",
        "regexp-unicode-property-escapes",
        "string-trimming",
        "super",
        "template",
        "u180e",
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
        assert!(
            profile
                .features
                .iter()
                .map(String::as_str)
                .eq(EXPECTED_FEATURES)
        );
        let expected_audited_negatives = EXPECTED_AUDITED_NEGATIVES
            .into_iter()
            .chain(PROPERTY_MANIFEST.lines().filter(|path| {
                path.starts_with("test/built-ins/RegExp/property-escapes/")
                    && !path.starts_with("test/built-ins/RegExp/property-escapes/generated/")
                    && !PROPERTY_POSITIVE_PATHS.contains(path)
            }))
            .collect::<BTreeSet<_>>();
        assert_eq!(expected_audited_negatives.len(), 423);
        assert!(
            profile
                .audited_negative_tests
                .iter()
                .map(String::as_str)
                .eq(expected_audited_negatives)
        );
    }

    #[test]
    fn feature_gaps_are_deduplicated_and_sorted_before_negative_provenance() {
        let profile = OxideProfile::parse(CHECKED_IN_PROFILE).unwrap();
        let classification = profile
            .classify(
                Path::new("test/not-audited.js"),
                &["class".to_owned(), "Promise".to_owned(), "class".to_owned()],
                true,
            )
            .unwrap();

        assert_eq!(classification.outcome, "unsupported-feature");
        assert_eq!(
            classification.detail,
            "quickjs-oxide does not declare Test262 feature support: Promise, class"
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
                .contains("section [audited-negative-tests] is out of order")
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
    fn parser_requires_entries_to_follow_the_fixed_section_order() {
        let source = "BigInt\n[features]\n[audited-negative-tests]\n";
        assert!(
            OxideProfile::parse(source)
                .unwrap_err()
                .contains("entry appears before the [features] section")
        );
        assert_eq!(
            SECTION_ORDER,
            [FEATURES_SECTION, AUDITED_NEGATIVE_TESTS_SECTION]
        );
    }
}
