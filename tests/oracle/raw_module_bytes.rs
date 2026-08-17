use crate::quickjs_raw_source_oracle::{
    RawModuleObservation, normalized_module_filename, observe_raw_module,
};
use quickjs_oxide::{CompileOptions, Context, Runtime, RuntimeError, Value};

#[derive(Clone, Copy, Debug)]
enum Api {
    Compile,
    CompileWithFilename,
    CompileWithOptions,
}

struct Case {
    group: &'static str,
    description: &'static str,
    api: Api,
    authored: &'static [u8],
}

const EXPLICIT_FILENAME: &str = "raw-module.mjs";

const CASES: &[Case] = &[
    Case {
        group: "module source",
        description: "export syntax executes from a raw Module buffer",
        api: Api::Compile,
        authored: b"export const answer = 42; globalThis.__qjoRawObservation = answer;",
    },
    Case {
        group: "module function redeclaration",
        description: "normal function conflict reports the parameter-list token",
        api: Api::Compile,
        authored: b"var value; function value(){}",
    },
    Case {
        group: "module function redeclaration",
        description: "generator function conflict reports the parameter-list token",
        api: Api::CompileWithFilename,
        authored: b"var value; function* value(){}",
    },
    Case {
        group: "module function redeclaration",
        description: "async function conflict reports the parameter-list token",
        api: Api::CompileWithOptions,
        authored: b"var value; async function value(){}",
    },
    Case {
        group: "module function redeclaration",
        description: "async generator conflict reports the parameter-list token",
        api: Api::Compile,
        authored: b"var value; async function* value(){}",
    },
    Case {
        group: "file prefix",
        description: "leading UTF-8 BOM is Module whitespace",
        api: Api::CompileWithFilename,
        authored: b"\xef\xbb\xbfexport {}; globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "file prefix",
        description: "shebang comment accepts a malformed continuation byte",
        api: Api::CompileWithOptions,
        authored: b"#!/usr/bin/env qjs\x80\nexport {}; globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "NUL",
        description: "embedded NUL is accepted inside a block comment",
        api: Api::Compile,
        authored: b"/*\0*/export {}; globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "NUL",
        description: "embedded NUL is preserved inside a string literal",
        api: Api::CompileWithFilename,
        authored: b"export {}; globalThis.__qjoRawObservation = '\0';",
    },
    Case {
        group: "NUL",
        description: "embedded NUL is rejected as an ordinary Module token",
        api: Api::CompileWithOptions,
        authored: b"export {}; void \0;",
    },
    Case {
        group: "string literal",
        description: "WTF-8 lone high surrogate is one UTF-16 code unit",
        api: Api::Compile,
        authored: b"export {}; globalThis.__qjoRawObservation = '\xed\xa0\x80';",
    },
    Case {
        group: "string literal",
        description: "CESU-8 surrogate pair retains both UTF-16 code units",
        api: Api::CompileWithFilename,
        authored: b"export {}; globalThis.__qjoRawObservation = '\xed\xa0\xbd\xed\xb8\x80';",
    },
    Case {
        group: "template literal",
        description: "WTF-8 lone low surrogate is preserved in template text",
        api: Api::CompileWithOptions,
        authored: b"export {}; globalThis.__qjoRawObservation = `A\xed\xbf\xbfB`;",
    },
    Case {
        group: "template literal",
        description: "CESU-8 surrogate pair is preserved in template text",
        api: Api::Compile,
        authored: b"export {}; globalThis.__qjoRawObservation = `A\xed\xa0\xbd\xed\xb8\x80B`;",
    },
    Case {
        group: "regexp literal",
        description: "WTF-8 lone high surrogate is preserved in RegExp source",
        api: Api::CompileWithFilename,
        authored: b"export {}; globalThis.__qjoRawObservation = /\xed\xa0\x80/.source;",
    },
    Case {
        group: "regexp literal",
        description: "CESU-8 surrogate pair is preserved in RegExp source",
        api: Api::CompileWithOptions,
        authored: b"export {}; globalThis.__qjoRawObservation = /\xed\xa0\xbd\xed\xb8\x80/.source;",
    },
    Case {
        group: "malformed token",
        description: "continuation byte is rejected in ordinary Module token context",
        api: Api::Compile,
        authored: b"export {}; void \x80;",
    },
    Case {
        group: "malformed token",
        description: "invalid FF lead byte is rejected in ordinary Module token context",
        api: Api::CompileWithFilename,
        authored: b"export {}; void \xff;",
    },
    Case {
        group: "malformed token",
        description: "overlong two-byte sequence is rejected in ordinary Module token context",
        api: Api::CompileWithOptions,
        authored: b"export {}; void \xc0\x80;",
    },
    Case {
        group: "malformed token",
        description: "truncated three-byte sequence is rejected in ordinary Module token context",
        api: Api::Compile,
        authored: b"export {}; void \xe2\x82;",
    },
    Case {
        group: "malformed string",
        description: "continuation byte is rejected inside string text",
        api: Api::CompileWithFilename,
        authored: b"export {}; void '\x80';",
    },
    Case {
        group: "malformed string",
        description: "continuation byte is rejected after a string backslash",
        api: Api::CompileWithOptions,
        authored: b"export {}; void '\\\x80';",
    },
    Case {
        group: "malformed string",
        description: "continuation byte is rejected inside a fixed hex escape",
        api: Api::Compile,
        authored: b"export {}; void '\\x\x80';",
    },
    Case {
        group: "malformed template",
        description: "continuation byte is rejected inside template text",
        api: Api::CompileWithFilename,
        authored: b"export {}; void `\x80`;",
    },
    Case {
        group: "malformed template",
        description: "continuation byte is rejected inside a template escape",
        api: Api::CompileWithOptions,
        authored: b"export {}; void `\\u\x80`;",
    },
    Case {
        group: "malformed regexp",
        description: "continuation byte is rejected inside RegExp text",
        api: Api::Compile,
        authored: b"export {}; void /\x80/;",
    },
    Case {
        group: "malformed regexp",
        description: "continuation byte is rejected after a RegExp backslash",
        api: Api::CompileWithFilename,
        authored: b"export {}; void /a\\\x80/;",
    },
    Case {
        group: "malformed regexp",
        description: "continuation byte is rejected in RegExp flags",
        api: Api::CompileWithOptions,
        authored: b"export {}; void /a/\x80;",
    },
    Case {
        group: "comments",
        description: "line comment accepts a malformed continuation byte",
        api: Api::Compile,
        authored: b"//\x80\nexport {}; globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "comments",
        description: "block comment accepts an invalid FF lead byte",
        api: Api::CompileWithFilename,
        authored: b"/*\xff*/export {}; globalThis.__qjoRawObservation = 42;",
    },
    Case {
        group: "raw column",
        description: "continuation byte in comment contributes zero QuickJS columns",
        api: Api::CompileWithOptions,
        authored: b"export {}; /*\x80*/@",
    },
    Case {
        group: "raw column",
        description: "invalid FF lead byte in comment contributes one QuickJS column",
        api: Api::Compile,
        authored: b"export {}; /*\xff*/@",
    },
    Case {
        group: "raw column",
        description: "canonical four-byte scalar contributes one QuickJS column",
        api: Api::CompileWithFilename,
        authored: b"export {}; /*\xf0\x9f\x98\x80*/@",
    },
    Case {
        group: "raw column",
        description: "CESU-8 pair contributes two QuickJS columns",
        api: Api::CompileWithOptions,
        authored: b"export {}; /*\xed\xa0\xbd\xed\xb8\x80*/@",
    },
    Case {
        group: "Function toString",
        description: "Function source decoding observes malformed authored comment bytes",
        api: Api::Compile,
        authored: b"export {}; globalThis.__qjoRawObservation = (function raw(){/*\x80X*/return 42;}).toString();",
    },
    Case {
        group: "Function toString",
        description: "Function source decoding preserves WTF-8 UTF-16 units",
        api: Api::CompileWithFilename,
        authored: b"export {}; globalThis.__qjoRawObservation = (function raw(){return '\xed\xa0\x80\xed\xb0\x80';}).toString();",
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
    globalThis.__qjoRawEncodedObservation = observation;
    if (typeof print === "function") print(observation);
})()
"#;

#[test]
fn raw_module_bytes_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP raw Module byte differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert!(
        CASES.len() >= 18,
        "raw Module differential lost its broad matrix"
    );

    let mut failures = Vec::new();
    for case in CASES {
        let source = source_for(case);
        let quickjs = observe_raw_module(&oracle, &source, case.description);
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
        "raw Module bytes differed from pinned QuickJS in {} case(s):\n\n{}",
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

fn oxide_observation(case: &Case, source: &[u8]) -> RawModuleObservation {
    let runtime = Runtime::new();
    let mut context = runtime.new_context();
    let compilation = match case.api {
        Api::Compile => context.compile_module_bytes(source),
        Api::CompileWithFilename => {
            context.compile_module_bytes_with_filename(source, EXPLICIT_FILENAME)
        }
        Api::CompileWithOptions => context
            .compile_module_bytes_with_options(source, &CompileOptions::new(EXPLICIT_FILENAME)),
    };
    let result = compilation.and_then(|module| context.execute_module(&module));

    match result {
        Ok(_) => match context.eval("globalThis.__qjoRawEncodedObservation") {
            Ok(Value::String(value)) => RawModuleObservation::Return(value.to_utf8_lossy()),
            Ok(value) => RawModuleObservation::EngineFailure(format!(
                "raw Module observer returned an unexpected value: {value:?}"
            )),
            Err(error) => RawModuleObservation::EngineFailure(format!(
                "raw Module observer could not read its result: {error}"
            )),
        },
        Err(RuntimeError::Exception) => oxide_exception(&runtime, &mut context, case),
        Err(error) => RawModuleObservation::EngineFailure(error.to_string()),
    }
}

fn oxide_exception(runtime: &Runtime, context: &mut Context, case: &Case) -> RawModuleObservation {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        return RawModuleObservation::EngineFailure(
            "raw Module exception was not an object".to_owned(),
        );
    };
    let read = |context: &mut Context, name: &str| {
        let key = runtime.intern_property_key(name).unwrap();
        context.get_property(&error, &key).unwrap()
    };
    let Value::String(name) = read(context, "name") else {
        return RawModuleObservation::EngineFailure("Error.name was not a string".to_owned());
    };
    let Value::String(message) = read(context, "message") else {
        return RawModuleObservation::EngineFailure("Error.message was not a string".to_owned());
    };
    let Value::String(filename) = read(context, "fileName") else {
        return RawModuleObservation::EngineFailure("Error.fileName was not a string".to_owned());
    };
    let Value::Int(line) = read(context, "lineNumber") else {
        return RawModuleObservation::EngineFailure("Error.lineNumber was not an Int32".to_owned());
    };
    let Value::Int(column) = read(context, "columnNumber") else {
        return RawModuleObservation::EngineFailure(
            "Error.columnNumber was not an Int32".to_owned(),
        );
    };
    let actual_filename = filename.to_utf8_lossy();
    let expected_filename = match case.api {
        Api::Compile => CompileOptions::default().filename,
        Api::CompileWithFilename | Api::CompileWithOptions => EXPLICIT_FILENAME.to_owned(),
    };
    if actual_filename != expected_filename {
        return RawModuleObservation::EngineFailure(format!(
            "raw Module filename was {actual_filename:?}, expected {expected_filename:?}"
        ));
    }

    RawModuleObservation::Throw {
        name: name.to_utf8_lossy(),
        message: message.to_utf8_lossy(),
        filename: normalized_module_filename().to_owned(),
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
