use crate::quickjs_raw_source_oracle::{
    RawScriptObservation, normalized_filename, observe_raw_script,
};
use quickjs_oxide::{CompileOptions, Context, EvalOptions, Runtime, RuntimeError, Value};

#[derive(Clone, Copy, Debug)]
enum Api {
    Compile,
    CompileWithFilename,
    CompileWithOptions,
    Eval,
    EvalWithFilename,
    EvalWithOptions,
}

struct Case {
    group: &'static str,
    description: &'static str,
    api: Api,
    authored: &'static [u8],
}

const EXPLICIT_FILENAME: &str = "raw-script.js";

const CASES: &[Case] = &[
    Case {
        group: "file prefix",
        description: "leading UTF-8 BOM is script whitespace",
        api: Api::Compile,
        authored: b"\xef\xbb\xbfglobalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "file prefix",
        description: "shebang comment accepts a malformed continuation byte",
        api: Api::CompileWithFilename,
        authored: b"#!/usr/bin/env qjs\x80\nglobalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "NUL",
        description: "embedded NUL is accepted inside a block comment",
        api: Api::CompileWithOptions,
        authored: b"/*\0*/globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "NUL",
        description: "embedded NUL is preserved inside a string literal",
        api: Api::Eval,
        authored: b"globalThis.__qjoRawObservation = '\0';",
    },
    Case {
        group: "NUL",
        description: "embedded NUL is rejected as an ordinary token",
        api: Api::EvalWithFilename,
        authored: b"void \0;",
    },
    Case {
        group: "string literal",
        description: "WTF-8 lone high surrogate is one UTF-16 code unit",
        api: Api::EvalWithOptions,
        authored: b"globalThis.__qjoRawObservation = '\xed\xa0\x80';",
    },
    Case {
        group: "string literal",
        description: "CESU-8 surrogate pair retains both UTF-16 code units",
        api: Api::Compile,
        authored: b"globalThis.__qjoRawObservation = '\xed\xa0\xbd\xed\xb8\x80';",
    },
    Case {
        group: "template literal",
        description: "WTF-8 lone low surrogate is preserved in template text",
        api: Api::CompileWithFilename,
        authored: b"globalThis.__qjoRawObservation = `A\xed\xbf\xbfB`;",
    },
    Case {
        group: "template literal",
        description: "CESU-8 surrogate pair is preserved in template text",
        api: Api::CompileWithOptions,
        authored: b"globalThis.__qjoRawObservation = `A\xed\xa0\xbd\xed\xb8\x80B`;",
    },
    Case {
        group: "regexp literal",
        description: "WTF-8 lone high surrogate is preserved in RegExp source",
        api: Api::Eval,
        authored: b"globalThis.__qjoRawObservation = /\xed\xa0\x80/.source;",
    },
    Case {
        group: "regexp literal",
        description: "CESU-8 surrogate pair is preserved in RegExp source",
        api: Api::EvalWithFilename,
        authored: b"globalThis.__qjoRawObservation = /\xed\xa0\xbd\xed\xb8\x80/.source;",
    },
    Case {
        group: "regexp literal",
        description: "escaped WTF-8 surrogate remains an identity escape",
        api: Api::EvalWithOptions,
        authored: b"globalThis.__qjoRawObservation = /a\\\xed\xa0\x80/.source;",
    },
    Case {
        group: "malformed token",
        description: "continuation byte is rejected in ordinary token context",
        api: Api::Compile,
        authored: b"void \x80;",
    },
    Case {
        group: "malformed token",
        description: "invalid FF lead byte is rejected in ordinary token context",
        api: Api::CompileWithFilename,
        authored: b"void \xff;",
    },
    Case {
        group: "malformed token",
        description: "overlong two-byte sequence is rejected in ordinary token context",
        api: Api::CompileWithOptions,
        authored: b"void \xc0\x80;",
    },
    Case {
        group: "malformed token",
        description: "truncated three-byte sequence is rejected in ordinary token context",
        api: Api::Eval,
        authored: b"void \xe2\x82;",
    },
    Case {
        group: "malformed string",
        description: "continuation byte is rejected inside string text",
        api: Api::EvalWithFilename,
        authored: b"void '\x80';",
    },
    Case {
        group: "malformed string",
        description: "continuation byte is rejected after a string backslash",
        api: Api::EvalWithOptions,
        authored: b"void '\\\x80';",
    },
    Case {
        group: "malformed string",
        description: "continuation byte is rejected inside a fixed hex escape",
        api: Api::Compile,
        authored: b"void '\\x\x80';",
    },
    Case {
        group: "malformed template",
        description: "continuation byte is rejected inside template text",
        api: Api::CompileWithFilename,
        authored: b"void `\x80`;",
    },
    Case {
        group: "malformed template",
        description: "continuation byte is rejected inside a template escape",
        api: Api::CompileWithOptions,
        authored: b"void `\\u\x80`;",
    },
    Case {
        group: "malformed regexp",
        description: "continuation byte is rejected inside RegExp text",
        api: Api::Eval,
        authored: b"void /\x80/;",
    },
    Case {
        group: "malformed regexp",
        description: "continuation byte is rejected after a RegExp backslash",
        api: Api::EvalWithFilename,
        authored: b"void /a\\\x80/;",
    },
    Case {
        group: "malformed regexp",
        description: "continuation byte is rejected in RegExp flags",
        api: Api::EvalWithOptions,
        authored: b"void /a/\x80;",
    },
    Case {
        group: "malformed identifier",
        description: "continuation byte after identifier unicode escape keeps parser priority",
        api: Api::Compile,
        authored: b"var a\\u\x80;",
    },
    Case {
        group: "comments",
        description: "line comment accepts a malformed continuation byte",
        api: Api::CompileWithFilename,
        authored: b"//\x80\nglobalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "comments",
        description: "block comment accepts an invalid FF lead byte",
        api: Api::CompileWithOptions,
        authored: b"/*\xff*/globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "raw column",
        description: "continuation byte in comment contributes zero QuickJS columns",
        api: Api::Eval,
        authored: b"/*\x80*/@",
    },
    Case {
        group: "raw column",
        description: "invalid FF lead byte in comment contributes one QuickJS column",
        api: Api::EvalWithFilename,
        authored: b"/*\xff*/@",
    },
    Case {
        group: "raw column",
        description: "overlong sequence in comment follows raw lead and continuation columns",
        api: Api::EvalWithOptions,
        authored: b"/*\xc0\x80*/@",
    },
    Case {
        group: "raw column",
        description: "truncated sequence in comment follows raw lead and continuation columns",
        api: Api::Compile,
        authored: b"/*\xe2\x82*/@",
    },
    Case {
        group: "raw column",
        description: "canonical four-byte scalar contributes one QuickJS column",
        api: Api::CompileWithFilename,
        authored: b"/*\xf0\x9f\x98\x80*/@",
    },
    Case {
        group: "raw column",
        description: "CESU-8 pair contributes two QuickJS columns",
        api: Api::CompileWithOptions,
        authored: b"/*\xed\xa0\xbd\xed\xb8\x80*/@",
    },
    Case {
        group: "Function toString",
        description: "Function source decoding observes malformed authored comment bytes",
        api: Api::Eval,
        authored: b"globalThis.__qjoRawObservation = (function raw(){/*\x80X*/return 42;}).toString();",
    },
    Case {
        group: "Function toString",
        description: "Function source decoding preserves WTF-8 UTF-16 units",
        api: Api::EvalWithFilename,
        authored: b"globalThis.__qjoRawObservation = (function raw(){return '\xed\xa0\x80\xed\xb0\x80';}).toString();",
    },
];

const OBSERVER_SUFFIX: &[u8] = br#"
;(function () {
    function encode(value) {
        var string = String(value);
        var output = typeof value + "|";
        for (var index = 0; index < string.length; index++) {
            if (index) output += ",";
            output += ("0000" + string.charCodeAt(index).toString(16)).slice(-4);
        }
        return output;
    }
    var observation = encode(globalThis.__qjoRawObservation);
    if (typeof print === "function") print(observation);
    return observation;
})()
"#;

#[test]
fn raw_script_bytes_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP raw Script byte differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert!(
        CASES.len() >= 18,
        "raw Script differential lost its broad matrix"
    );

    let mut failures = Vec::new();
    for case in CASES {
        let source = source_for(case);
        let quickjs = observe_raw_script(&oracle, &source, case.description);
        let oxide = oxide_observation(case, &source);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {} / {:?}\nsource: {}\noxide: {oxide:?}\nquickjs: {quickjs:?}",
                case.group,
                case.description,
                case.api,
                hex_bytes(case.authored),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "raw Script bytes differed from pinned QuickJS in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn source_for(case: &Case) -> Vec<u8> {
    let mut source = Vec::with_capacity(case.authored.len() + OBSERVER_SUFFIX.len());
    source.extend_from_slice(case.authored);
    source.extend_from_slice(OBSERVER_SUFFIX);
    source
}

fn oxide_observation(case: &Case, source: &[u8]) -> RawScriptObservation {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let result = match case.api {
        Api::Compile => context
            .compile_bytes(source)
            .and_then(|function| context.execute(&function)),
        Api::CompileWithFilename => context
            .compile_bytes_with_filename(source, EXPLICIT_FILENAME)
            .and_then(|function| context.execute(&function)),
        Api::CompileWithOptions => context
            .compile_bytes_with_options(source, &CompileOptions::new(EXPLICIT_FILENAME))
            .and_then(|function| context.execute(&function)),
        Api::Eval => context.eval_bytes(source),
        Api::EvalWithFilename => context.eval_bytes_with_filename(source, EXPLICIT_FILENAME),
        Api::EvalWithOptions => {
            context.eval_bytes_with_options(source, &EvalOptions::new(EXPLICIT_FILENAME))
        }
    };

    match result {
        Ok(Value::String(value)) => RawScriptObservation::Return(value.to_utf8_lossy()),
        Ok(value) => RawScriptObservation::EngineFailure(format!(
            "raw observer returned an unexpected value: {value:?}"
        )),
        Err(RuntimeError::Exception) => oxide_exception(&runtime, &mut context, case),
        Err(error) => RawScriptObservation::EngineFailure(error.to_string()),
    }
}

fn oxide_exception(runtime: &Runtime, context: &mut Context, case: &Case) -> RawScriptObservation {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        return RawScriptObservation::EngineFailure(
            "raw Script exception was not an object".to_owned(),
        );
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(context, "name") else {
        return RawScriptObservation::EngineFailure("Error.name was not a string".to_owned());
    };
    let Value::String(message) = read(context, "message") else {
        return RawScriptObservation::EngineFailure("Error.message was not a string".to_owned());
    };
    let Value::String(filename) = read(context, "fileName") else {
        return RawScriptObservation::EngineFailure("Error.fileName was not a string".to_owned());
    };
    let Value::Int(line) = read(context, "lineNumber") else {
        return RawScriptObservation::EngineFailure("Error.lineNumber was not an Int32".to_owned());
    };
    let Value::Int(column) = read(context, "columnNumber") else {
        return RawScriptObservation::EngineFailure(
            "Error.columnNumber was not an Int32".to_owned(),
        );
    };
    let actual_filename = filename.to_utf8_lossy();
    let expected_filename = match case.api {
        Api::Compile | Api::Eval => CompileOptions::default().filename,
        Api::CompileWithFilename
        | Api::CompileWithOptions
        | Api::EvalWithFilename
        | Api::EvalWithOptions => EXPLICIT_FILENAME.to_owned(),
    };
    if actual_filename != expected_filename {
        return RawScriptObservation::EngineFailure(format!(
            "raw Script filename was {actual_filename:?}, expected {expected_filename:?}"
        ));
    }

    RawScriptObservation::Throw {
        name: name.to_utf8_lossy(),
        message: message.to_utf8_lossy(),
        filename: normalized_filename().to_owned(),
        line: u32::try_from(line).unwrap(),
        column: u32::try_from(column).unwrap(),
    }
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
