use super::*;

struct ScriptArgsCliCase {
    description: &'static str,
    arguments: Vec<String>,
    expected_values: Vec<String>,
}

const SCRIPT_ARGS_PROBE_SOURCE: &str = r#"function descriptorBits(descriptor) {
  return [descriptor.writable, descriptor.enumerable, descriptor.configurable]
    .map(Number).join("");
}
function encode(value) {
  var units = [];
  for (var i = 0; i < value.length; i++) {
    units.push(("0000" + value.charCodeAt(i).toString(16)).slice(-4));
  }
  return value.length + ":" + units.join(".");
}
var globalDescriptor = Object.getOwnPropertyDescriptor(globalThis, "scriptArgs");
var lengthDescriptor = Object.getOwnPropertyDescriptor(scriptArgs, "length");
print("values", scriptArgs.map(encode).join("|"));
print("global", globalDescriptor.value === scriptArgs, descriptorBits(globalDescriptor));
print("array", Array.isArray(scriptArgs), Object.getPrototypeOf(scriptArgs) === Array.prototype);
print("length", lengthDescriptor.value, descriptorBits(lengthDescriptor));
print("keys", Reflect.ownKeys(scriptArgs).join("|"));
print("elements", scriptArgs.map(function(_, index) {
  return descriptorBits(Object.getOwnPropertyDescriptor(scriptArgs, String(index)));
}).join("|"));
var originalArgs = scriptArgs;
originalArgs.push("pushed");
print("push", encode(originalArgs[originalArgs.length - 1]), originalArgs.length);
globalThis.scriptArgs = ["assigned"];
print("assign", scriptArgs[0]);
Object.defineProperty(globalThis, "scriptArgs", {
  value: originalArgs, writable: true, enumerable: true, configurable: true
});
print("redefine", scriptArgs === originalArgs);
print("delete", delete globalThis.scriptArgs, typeof globalThis.scriptArgs);"#;

fn script_args_cli_cases(entry: &Path) -> Vec<ScriptArgsCliCase> {
    let filename = cli_path(entry);
    vec![
        ScriptArgsCliCase {
            description: "file mode keeps filename and every following argument",
            arguments: vec![
                "--script".to_owned(),
                filename.clone(),
                "alpha".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
                "-tail".to_owned(),
                "--module".to_owned(),
            ],
            expected_values: vec![
                filename.clone(),
                "alpha".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
                "-tail".to_owned(),
                "--module".to_owned(),
            ],
        },
        ScriptArgsCliCase {
            description: "double dash stops scanning before the file",
            arguments: vec![
                "--".to_owned(),
                filename.clone(),
                "-m".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
            ],
            expected_values: vec![filename, "-m".to_owned(), String::new(), "雪🙂".to_owned()],
        },
        ScriptArgsCliCase {
            description: "eval mode publishes the unconsumed arguments",
            arguments: vec![
                "-e".to_owned(),
                SCRIPT_ARGS_PROBE_SOURCE.to_owned(),
                "alpha".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
                "-tail".to_owned(),
            ],
            expected_values: vec![
                "alpha".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
                "-tail".to_owned(),
            ],
        },
        ScriptArgsCliCase {
            description: "eval mode publishes an empty argument array",
            arguments: vec!["-e".to_owned(), SCRIPT_ARGS_PROBE_SOURCE.to_owned()],
            expected_values: Vec::new(),
        },
        ScriptArgsCliCase {
            description: "the last eval option wins before the argument tail",
            arguments: vec![
                "-e".to_owned(),
                "throw new Error('superseded eval')".to_owned(),
                "-e".to_owned(),
                SCRIPT_ARGS_PROBE_SOURCE.to_owned(),
                "tail".to_owned(),
            ],
            expected_values: vec!["tail".to_owned()],
        },
        ScriptArgsCliCase {
            description: "combined module and eval options preserve the argument tail",
            arguments: vec![
                "-me".to_owned(),
                SCRIPT_ARGS_PROBE_SOURCE.to_owned(),
                "tail".to_owned(),
            ],
            expected_values: vec!["tail".to_owned()],
        },
        ScriptArgsCliCase {
            description: "double dash preserves option-shaped eval arguments",
            arguments: vec![
                "-e".to_owned(),
                SCRIPT_ARGS_PROBE_SOURCE.to_owned(),
                "--".to_owned(),
                "-dash".to_owned(),
                "--module".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
            ],
            expected_values: vec![
                "-dash".to_owned(),
                "--module".to_owned(),
                String::new(),
                "雪🙂".to_owned(),
            ],
        },
    ]
}

fn encode_script_arg(value: &str) -> String {
    let units = value.encode_utf16().collect::<Vec<_>>();
    let encoded = units
        .iter()
        .map(|unit| format!("{unit:04x}"))
        .collect::<Vec<_>>()
        .join(".");
    format!("{}:{encoded}", units.len())
}

fn expected_script_args_probe(values: &[String]) -> Vec<u8> {
    let count = values.len();
    let encoded_values = values
        .iter()
        .map(|value| encode_script_arg(value))
        .collect::<Vec<_>>()
        .join("|");
    let mut keys = (0..count)
        .map(|index| index.to_string())
        .collect::<Vec<_>>();
    keys.push("length".to_owned());
    let element_descriptors = vec!["111"; count].join("|");
    format!(
        "values {encoded_values}\nglobal true 111\narray true true\nlength {count} 100\nkeys {}\nelements {element_descriptors}\npush 6:0070.0075.0073.0068.0065.0064 {}\nassign assigned\nredefine true\ndelete true undefined\n",
        keys.join("|"),
        count + 1,
    )
    .into_bytes()
}

fn run_script_args_case(program: &OsStr, case: &ScriptArgsCliCase) -> Output {
    Command::new(program)
        .args(&case.arguments)
        .output()
        .unwrap_or_else(|error| panic!("run {} with {program:?}: {error}", case.description))
}

#[test]
fn script_args_follow_pinned_quickjs_cli_and_descriptor_contract() {
    let fixture = ModuleFixture::new();
    let entry = fixture.write("script-args.js", SCRIPT_ARGS_PROBE_SOURCE);

    for case in script_args_cli_cases(&entry) {
        let output = run_script_args_case(OsStr::new(env!("CARGO_BIN_EXE_qjs")), &case);
        assert_eq!(
            output.status.code(),
            Some(0),
            "{}: {}",
            case.description,
            String::from_utf8_lossy(&output.stderr),
        );
        assert_eq!(
            output.stdout,
            expected_script_args_probe(&case.expected_values),
            "{}",
            case.description,
        );
        assert!(output.stderr.is_empty(), "{}", case.description);
    }
}

#[test]
fn script_args_cli_matches_quickjs_oracle() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP scriptArgs CLI differential: set QJS_ORACLE to upstream qjs");
        return;
    };
    let fixture = ModuleFixture::new();
    let entry = fixture.write("script-args.js", SCRIPT_ARGS_PROBE_SOURCE);

    for case in script_args_cli_cases(&entry) {
        let expected = expected_script_args_probe(&case.expected_values);
        let quickjs = run_script_args_case(&oracle, &case);
        assert_eq!(
            quickjs.status.code(),
            Some(0),
            "pinned QuickJS failed for {}: {}",
            case.description,
            String::from_utf8_lossy(&quickjs.stderr),
        );
        assert_eq!(
            quickjs.stdout, expected,
            "pinned QuickJS scriptArgs contract drifted for {}",
            case.description,
        );
        assert!(quickjs.stderr.is_empty(), "{}", case.description);

        let oxide = run_script_args_case(OsStr::new(env!("CARGO_BIN_EXE_qjs")), &case);
        assert_eq!(
            oxide.status.code(),
            quickjs.status.code(),
            "{}",
            case.description
        );
        assert_eq!(oxide.stdout, quickjs.stdout, "{}", case.description);
        assert_eq!(oxide.stderr, quickjs.stderr, "{}", case.description);
    }
}
