use crate::quickjs_raw_source_oracle::{
    RawModuleObservation, normalized_json_module_filename, observe_raw_json_module,
    raw_json_module_source,
};
use quickjs_oxide::{
    Context, JsString, ModuleImportAttributes, ModuleLoadResult, ModuleLoader, ModuleLoaderError,
    Runtime, RuntimeError, Value,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Grammar {
    Strict,
    Extended,
}

impl Grammar {
    const fn is_extended(self) -> bool {
        matches!(self, Self::Extended)
    }

    const fn payload_filename(self) -> &'static str {
        match self {
            Self::Strict => "value.json",
            Self::Extended => "value.data",
        }
    }
}

struct Case {
    group: &'static str,
    description: &'static str,
    grammar: Grammar,
    authored: &'static [u8],
}

const CASES: &[Case] = &[
    Case {
        group: "strict baseline",
        description: "strict JSON module executes from a raw byte buffer",
        grammar: Grammar::Strict,
        authored: br#"{"value":42}"#,
    },
    Case {
        group: "strict string",
        description: "strict JSON preserves a WTF-8 lone high surrogate",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xed\xa0\x80\"}",
    },
    Case {
        group: "strict string",
        description: "strict JSON preserves a WTF-8 lone low surrogate",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xed\xbf\xbf\"}",
    },
    Case {
        group: "strict string",
        description: "strict JSON preserves both CESU-8 surrogate code units",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xed\xa0\xbd\xed\xb8\x80\"}",
    },
    Case {
        group: "strict string",
        description: "strict JSON decodes a canonical four-byte scalar to a surrogate pair",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xf0\x9f\x98\x80\"}",
    },
    Case {
        group: "strict BOM",
        description: "a UTF-8 BOM is accepted as JSON string text",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xef\xbb\xbf\"}",
    },
    Case {
        group: "strict BOM",
        description: "a leading UTF-8 BOM is not JSON whitespace",
        grammar: Grammar::Strict,
        authored: b"\xef\xbb\xbf{\"value\":42}",
    },
    Case {
        group: "strict NUL",
        description: "an unescaped NUL is rejected inside a JSON string",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\0\"}",
    },
    Case {
        group: "strict NUL",
        description: "a NUL is rejected in ordinary JSON token context",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\0}",
    },
    Case {
        group: "strict malformed string",
        description: "a continuation byte is rejected as a bad UTF-8 sequence in string text",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\x80\"}",
    },
    Case {
        group: "strict malformed string",
        description: "an invalid FF lead byte is rejected as a bad UTF-8 sequence in string text",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xff\"}",
    },
    Case {
        group: "strict malformed string",
        description: "an overlong two-byte sequence is rejected in string text",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xc0\x80\"}",
    },
    Case {
        group: "strict malformed string",
        description: "a truncated three-byte sequence is rejected in string text",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\xe2\x82\"}",
    },
    Case {
        group: "strict malformed escape",
        description: "a continuation byte after a backslash is a bad escaped character",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\"\\\x80\"}",
    },
    Case {
        group: "strict malformed property",
        description: "a malformed byte in a property name is a bad UTF-8 sequence",
        grammar: Grammar::Strict,
        authored: b"{\"va\x80lue\":42}",
    },
    Case {
        group: "strict malformed token",
        description: "a continuation byte is rejected in ordinary JSON token context",
        grammar: Grammar::Strict,
        authored: b"{\"value\":\x80}",
    },
    Case {
        group: "strict malformed number",
        description: "a continuation byte after a number sign uses the raw percent-c diagnostic",
        grammar: Grammar::Strict,
        authored: b"-\x80",
    },
    Case {
        group: "strict malformed number",
        description: "an embedded NUL after a number sign truncates the C diagnostic",
        grammar: Grammar::Strict,
        authored: b"-\0",
    },
    Case {
        group: "strict location",
        description: "a malformed string on the second line retains its raw location",
        grammar: Grammar::Strict,
        authored: b"{\n\"value\":\"\x80\"}",
    },
    Case {
        group: "strict raw column",
        description: "a canonical four-byte scalar contributes one QuickJS column",
        grammar: Grammar::Strict,
        authored: b"\"\xf0\x9f\x98\x80\"@",
    },
    Case {
        group: "strict raw column",
        description: "a CESU-8 pair contributes two QuickJS columns",
        grammar: Grammar::Strict,
        authored: b"\"\xed\xa0\xbd\xed\xb8\x80\"@",
    },
    Case {
        group: "extended baseline",
        description: "raw JSON5 accepts identifiers plus signs and trailing commas",
        grammar: Grammar::Extended,
        authored: b"{value:+42,}",
    },
    Case {
        group: "extended string",
        description: "JSON5 preserves a WTF-8 surrogate in single-quoted text",
        grammar: Grammar::Extended,
        authored: b"{value:'\xed\xa0\x80'}",
    },
    Case {
        group: "extended string",
        description: "JSON5 preserves both CESU-8 surrogate code units",
        grammar: Grammar::Extended,
        authored: b"{value:'\xed\xa0\xbd\xed\xb8\x80'}",
    },
    Case {
        group: "extended comments",
        description: "a block comment skips a malformed continuation byte",
        grammar: Grammar::Extended,
        authored: b"/*\x80*/{value:42}",
    },
    Case {
        group: "extended comments",
        description: "a line comment skips an invalid FF lead byte",
        grammar: Grammar::Extended,
        authored: b"//\xff\n{value:42}",
    },
    Case {
        group: "extended comments",
        description: "a block comment skips an embedded NUL",
        grammar: Grammar::Extended,
        authored: b"/*\0*/{value:42}",
    },
    Case {
        group: "extended malformed string",
        description: "JSON5 rejects a continuation byte in string text as bad UTF-8",
        grammar: Grammar::Extended,
        authored: b"{value:'\x80'}",
    },
    Case {
        group: "extended malformed escape",
        description: "JSON5 reports a malformed byte after a backslash as a bad escape",
        grammar: Grammar::Extended,
        authored: b"{value:'\\\x80'}",
    },
    Case {
        group: "extended malformed token",
        description: "JSON5 rejects a continuation byte in ordinary token context",
        grammar: Grammar::Extended,
        authored: b"{value:\x80}",
    },
    Case {
        group: "extended malformed number",
        description: "an invalid lead byte after a plus sign uses the raw percent-c diagnostic",
        grammar: Grammar::Extended,
        authored: b"+\xff",
    },
    Case {
        group: "extended malformed number",
        description: "a continuation byte after a radix prefix uses the raw percent-c diagnostic",
        grammar: Grammar::Extended,
        authored: b"0x\x80",
    },
    Case {
        group: "extended malformed number",
        description: "an embedded NUL after a radix prefix truncates the C diagnostic",
        grammar: Grammar::Extended,
        authored: b"0x\0",
    },
    Case {
        group: "extended BOM",
        description: "a leading UTF-8 BOM is not JSON5 whitespace",
        grammar: Grammar::Extended,
        authored: b"\xef\xbb\xbf{value:42}",
    },
    Case {
        group: "extended BOM",
        description: "a UTF-8 BOM is accepted as JSON5 string text",
        grammar: Grammar::Extended,
        authored: b"{value:'\xef\xbb\xbf'}",
    },
    Case {
        group: "extended raw column",
        description: "a continuation byte skipped by a comment contributes zero columns",
        grammar: Grammar::Extended,
        authored: b"/*\x80*/@",
    },
    Case {
        group: "extended raw column",
        description: "an invalid lead byte skipped by a comment contributes one column",
        grammar: Grammar::Extended,
        authored: b"/*\xff*/@",
    },
    Case {
        group: "extended raw column",
        description: "an overlong sequence skipped by a comment contributes one column",
        grammar: Grammar::Extended,
        authored: b"/*\xc0\x80*/@",
    },
    Case {
        group: "extended raw column",
        description: "a canonical four-byte scalar skipped by a comment contributes one column",
        grammar: Grammar::Extended,
        authored: b"/*\xf0\x9f\x98\x80*/@",
    },
    Case {
        group: "extended raw column",
        description: "a CESU-8 pair skipped by a comment contributes two columns",
        grammar: Grammar::Extended,
        authored: b"/*\xed\xa0\xbd\xed\xb8\x80*/@",
    },
];

#[derive(Debug)]
struct RawJsonModuleLoader {
    source: Vec<u8>,
    grammar: Grammar,
}

impl ModuleLoader for RawJsonModuleLoader {
    fn load_with_attributes(
        &self,
        normalized_name: &JsString,
        _attributes: &ModuleImportAttributes,
    ) -> Result<ModuleLoadResult, ModuleLoaderError> {
        let expected_name = JsString::try_from_utf8(self.grammar.payload_filename()).unwrap();
        if normalized_name != &expected_name {
            return Err(ModuleLoaderError::new(format!(
                "unexpected raw JSON module name: {}",
                normalized_name.to_utf8_lossy(),
            )));
        }
        Ok(match self.grammar {
            Grammar::Strict => ModuleLoadResult::JsonBytes(self.source.clone()),
            Grammar::Extended => ModuleLoadResult::Json5Bytes(self.source.clone()),
        })
    }
}

#[test]
fn raw_json_module_bytes_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP raw JSON module byte differential: set QJS_ORACLE to pinned upstream qjs");
        return;
    };
    assert!(
        CASES.len() >= 40,
        "raw JSON module differential lost its broad matrix"
    );
    assert!(
        CASES.iter().any(|case| case.grammar == Grammar::Strict)
            && CASES.iter().any(|case| case.grammar == Grammar::Extended),
        "raw JSON module differential must cover both parser modes"
    );

    let mut failures = Vec::new();
    for case in CASES {
        let quickjs = observe_raw_json_module(
            &oracle,
            case.authored,
            case.grammar.is_extended(),
            case.description,
        );
        let oxide = oxide_observation(case);
        if oxide != quickjs {
            failures.push(format!(
                "{} / {} / {:?}\nsource: {}\noxide: {oxide:?}\nquickjs: {quickjs:?}",
                case.group,
                case.description,
                case.grammar,
                hex_bytes(case.authored),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "raw JSON module bytes differed from pinned QuickJS in {} case(s):\n\n{}",
        failures.len(),
        failures.join("\n\n"),
    );
}

fn oxide_observation(case: &Case) -> RawModuleObservation {
    let runtime = Runtime::new();
    let registration = runtime.set_module_loader(RawJsonModuleLoader {
        source: case.authored.to_vec(),
        grammar: case.grammar,
    });
    let mut context = runtime.new_context();
    let result = context
        .compile_module_with_filename(
            raw_json_module_source(case.grammar.is_extended()),
            "entry.mjs",
        )
        .and_then(|module| context.execute_module(&module));

    let observation = match result {
        Ok(_) => match context.eval("globalThis.__qjoRawJsonEncodedObservation") {
            Ok(Value::String(value)) => RawModuleObservation::Return(value.to_utf8_lossy()),
            Ok(value) => RawModuleObservation::EngineFailure(format!(
                "raw JSON module observer returned an unexpected value: {value:?}"
            )),
            Err(error) => RawModuleObservation::EngineFailure(format!(
                "raw JSON module observer could not read its result: {error}"
            )),
        },
        Err(RuntimeError::Exception) => oxide_exception(&runtime, &mut context, case),
        Err(error) => RawModuleObservation::EngineFailure(error.to_string()),
    };
    drop(registration);
    observation
}

fn oxide_exception(runtime: &Runtime, context: &mut Context, case: &Case) -> RawModuleObservation {
    let Some(Value::Object(error)) = context.take_exception().unwrap() else {
        return RawModuleObservation::EngineFailure(
            "raw JSON module exception was not an object".to_owned(),
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
    let expected_filename = case.grammar.payload_filename();
    if actual_filename != expected_filename {
        return RawModuleObservation::EngineFailure(format!(
            "raw JSON module filename was {actual_filename:?}, expected {expected_filename:?}"
        ));
    }

    RawModuleObservation::Throw {
        name: name.to_utf8_lossy(),
        message: message.to_utf8_lossy(),
        filename: normalized_json_module_filename().to_owned(),
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
