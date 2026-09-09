use crate::runtime_completion_oracle::{
    compare_read_context_eval_completion_cases_with_prelude,
    observe_quickjs_completion_with_prelude,
};

// Differential lock for pinned QuickJS 2026-06-04 scoped RegExp modifiers.
// The parser and state restoration behavior lives in `libregexp.c`
// `re_parse_modifiers` / `re_parse_term` (1810-1950). These vectors avoid
// unrelated unsupported RegExp features so the modifier slice can close
// independently while still probing literal, constructor, Unicode, capture,
// quantifier, and stateful execution integration.

const PRELUDE: &str = r#"
function __completion(callback){
    try{return "return:"+String(callback())}
    catch(error){
        if(error!==null&&typeof error==="object")
            return "throw:"+error.name+":"+error.message;
        return "throw:"+typeof error+":"+String(error);
    }
}
"#;

const GRAMMAR_CASES: &[(&str, &str)] = &[
    (
        "literal duplicate add modifier reports the modifier error",
        r#"/(?ii:a)/"#,
    ),
    (
        "constructor duplicate remove modifier reports the modifier error",
        r#"(function(){
            return __completion(function(){return new RegExp("(?-ss:a)")});
        })()"#,
    ),
    (
        "add and remove overlap is invalid after both lists are parsed",
        r#"(function(){
            return __completion(function(){return new RegExp("(?im-ms:a)")});
        })()"#,
    ),
    (
        "empty add and remove lists are invalid",
        r#"(function(){
            return __completion(function(){return new RegExp("(?-:a)")});
        })()"#,
    ),
    (
        "missing modifier colon wins before parsing the body",
        r#"(function(){
            return __completion(function(){return new RegExp("(?i=a)")});
        })()"#,
    ),
    (
        "only i m and s enter the scoped modifier grammar",
        r#"(function(){
            return __completion(function(){return new RegExp("(?g:a)")});
        })()"#,
    ),
    (
        "duplicate add modifier wins before add remove overlap",
        r#"(function(){
            return __completion(function(){return new RegExp("(?iim-i:a)")});
        })()"#,
    ),
    (
        "unterminated modifier group reports the closing delimiter",
        r#"(function(){
            return __completion(function(){return new RegExp("(?i:a")});
        })()"#,
    ),
];

const SCOPED_MATCHING_CASES: &[(&str, &str)] = &[
    (
        "nested add remove and re-add restores each enclosing modifier state",
        r#"(function(){
            var regexp=/(?i:a(?-i:b(?i:c))d)e/;
            return [regexp.test("AbCDe"),regexp.test("ABCDe"),
                regexp.test("AbCdE")].join("|");
        })()"#,
    ),
    (
        "ignoreCase scopes over literals and classes then restores outside",
        r#"(function(){
            var regexp=/(?i:a[A-Z])(?-i:b)/;
            return [regexp.test("aZb"),regexp.test("Azb"),
                regexp.test("AZB")].join("|");
        })()"#,
    ),
    (
        "ignoreCase changes Unicode word boundary classification within scope",
        r#"(function(){
            var add=/(?i:\b\u212a\b)/u,
                remove=/(?-i:\b\u212a\b)/iu;
            return [add.test("\u212a"),remove.test("\u212a"),
                add.flags,remove.flags].join("|");
        })()"#,
    ),
    (
        "multiline add and remove select scoped anchor semantics",
        r#"(function(){
            var add=/(?m:^b$)/,
                remove=/(?-m:^b$)/m,
                input="a\nb\nc";
            return [add.test(input),remove.test(input),
                add.multiline,remove.multiline].join("|");
        })()"#,
    ),
    (
        "dotAll add and remove select scoped dot semantics",
        r#"(function(){
            var add=/(?s:a.b)/,
                remove=/(?-s:a.b)/s,
                input="a\nb";
            return [add.test(input),remove.test(input),
                add.dotAll,remove.dotAll].join("|");
        })()"#,
    ),
];

const CONSTRUCTION_AND_FRONTIER_CASES: &[(&str, &str)] = &[
    (
        "scoped modifiers do not change global flags or global exec state",
        r#"(function(){
            var regexp=/(?i:a)/g,
                first=regexp.exec("AaA"),afterFirst=regexp.lastIndex,
                second=regexp.exec("AaA"),afterSecond=regexp.lastIndex,
                third=regexp.exec("AaA"),afterThird=regexp.lastIndex,
                exhausted=regexp.exec("AaA");
            return [regexp.source,regexp.flags,regexp.global,regexp.ignoreCase,
                first[0],first.index,afterFirst,second[0],second.index,afterSecond,
                third[0],third.index,afterThird,exhausted,regexp.lastIndex].join("|");
        })()"#,
    ),
    (
        "literal and constructor paths preserve equivalent source and flags",
        r#"(function(){
            var source="(?im-s:^a.$)",
                literal=/(?im-s:^a.$)/s,
                constructed=new RegExp(source,"s"),
                input="x\nA!\ny";
            return [literal.source===constructed.source,literal.flags,
                constructed.flags,literal.test(input),constructed.test(input),
                literal.ignoreCase,literal.multiline,literal.dotAll].join("|");
        })()"#,
    ),
    (
        "scoped dot keeps legacy code-unit and Unicode code-point width",
        r#"(function(){
            var legacy=/(?s:(.))/.exec("\ud83d\ude00"),
                unicode=/(?s:(.))/u.exec("\ud83d\ude00");
            return [legacy[0].length,legacy[1].length,
                legacy[1].charCodeAt(0).toString(16),
                unicode[0].length,unicode[1].length,
                unicode[1].codePointAt(0).toString(16)].join("|");
        })()"#,
    ),
    (
        "scoped ignoreCase folds astral Unicode without changing outer flags",
        r#"(function(){
            var add=/(?i:\u{10400})/u,
                remove=/(?-i:\u{10400})/iu,
                lower="\u{10428}";
            return [add.test(lower),remove.test(lower),add.unicode,
                add.ignoreCase,remove.ignoreCase].join("|");
        })()"#,
    ),
    (
        "modifier groups remain noncapturing under alternation and quantification",
        r#"(function(){
            var regexp=/(?i:(a|b)+)(c)/,
                match=regexp.exec("ABc");
            return [match[0],match[1],match[2],match.length,
                regexp.test("ABC"),regexp.source,regexp.flags].join("|");
        })()"#,
    ),
];

#[test]
fn regexp_modifiers_oracle_vectors_self_check() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP RegExp modifiers oracle self-check: set QJS_ORACLE to upstream qjs");
        return;
    };
    for &(group, cases) in &[
        ("grammar", GRAMMAR_CASES),
        ("scoped matching", SCOPED_MATCHING_CASES),
        ("construction and frontier", CONSTRUCTION_AND_FRONTIER_CASES),
    ] {
        for &(description, source) in cases {
            let observation =
                observe_quickjs_completion_with_prelude(PRELUDE, &oracle, source, description);
            assert!(
                observation.starts_with("return|") || observation.starts_with("throw|"),
                "{group} oracle vector had no completion for {description}: {observation:?}",
            );
        }
    }
}

#[test]
fn regexp_modifier_grammar_and_error_priority_match_pinned_quickjs() {
    compare_read_context_eval_completion_cases_with_prelude(
        PRELUDE,
        "RegExp modifier grammar",
        GRAMMAR_CASES,
    );
}

#[test]
fn regexp_modifier_scoped_matching_matches_pinned_quickjs() {
    compare_read_context_eval_completion_cases_with_prelude(
        PRELUDE,
        "RegExp modifier scoped matching",
        SCOPED_MATCHING_CASES,
    );
}

#[test]
fn regexp_modifier_construction_and_frontier_match_pinned_quickjs() {
    compare_read_context_eval_completion_cases_with_prelude(
        PRELUDE,
        "RegExp modifier construction and frontier",
        CONSTRUCTION_AND_FRONTIER_CASES,
    );
}
