use std::ffi::OsStr;
use std::fs::{OpenOptions, create_dir, remove_dir, remove_file};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const NORMALIZED_SCRIPT_FILENAME: &str = "<raw-script>";
const NORMALIZED_MODULE_FILENAME: &str = "<raw-module>";
const NORMALIZED_JSON_MODULE_FILENAME: &str = "<raw-json-module>";
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

const STRICT_JSON_MODULE_SOURCE: &str = r#"
import value from "./value.json" with { type: "json" };
(function () {
    function encode(value) {
        var string = String(value);
        var output = typeof value + "|";
        for (var index = 0; index < string.length; index++) {
            if (index) output += ",";
            output += ("0000" + string.charCodeAt(index).toString(16)).slice(-4);
        }
        return output;
    }
    var observation = encode(value.value);
    globalThis.__qjoRawJsonEncodedObservation = observation;
    if (typeof print === "function") print(observation);
})()
"#;

const EXTENDED_JSON_MODULE_SOURCE: &str = r#"
import value from "./value.data" with { type: "json5" };
(function () {
    function encode(value) {
        var string = String(value);
        var output = typeof value + "|";
        for (var index = 0; index < string.length; index++) {
            if (index) output += ",";
            output += ("0000" + string.charCodeAt(index).toString(16)).slice(-4);
        }
        return output;
    }
    var observation = encode(value.value);
    globalThis.__qjoRawJsonEncodedObservation = observation;
    if (typeof print === "function") print(observation);
})()
"#;

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RawScriptObservation {
    Return(String),
    Throw {
        name: String,
        message: String,
        filename: String,
        line: u32,
        column: u32,
    },
    EngineFailure(String),
}

pub(crate) type RawModuleObservation = RawScriptObservation;

pub(crate) fn normalized_filename() -> &'static str {
    NORMALIZED_SCRIPT_FILENAME
}

pub(crate) fn normalized_module_filename() -> &'static str {
    NORMALIZED_MODULE_FILENAME
}

pub(crate) fn normalized_json_module_filename() -> &'static str {
    NORMALIZED_JSON_MODULE_FILENAME
}

pub(crate) fn raw_json_module_source(extended: bool) -> &'static str {
    if extended {
        EXTENDED_JSON_MODULE_SOURCE
    } else {
        STRICT_JSON_MODULE_SOURCE
    }
}

pub(crate) fn observe_raw_script(
    oracle: &OsStr,
    source: &[u8],
    description: &str,
) -> RawScriptObservation {
    observe_raw_source(
        oracle,
        source,
        description,
        "script",
        "--script",
        "source.js",
        NORMALIZED_SCRIPT_FILENAME,
    )
}

pub(crate) fn observe_raw_module(
    oracle: &OsStr,
    source: &[u8],
    description: &str,
) -> RawModuleObservation {
    observe_raw_source(
        oracle,
        source,
        description,
        "module",
        "--module",
        "source.mjs",
        NORMALIZED_MODULE_FILENAME,
    )
}

pub(crate) fn observe_raw_json_module(
    oracle: &OsStr,
    source: &[u8],
    extended: bool,
    description: &str,
) -> RawModuleObservation {
    let graph = TempJsonModuleGraph::new(source, extended, description);
    let output = Command::new(oracle)
        .arg("--module")
        .arg(&graph.root_path)
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS raw JSON module for {description}: {error}")
        });
    observe_output(
        output,
        &graph.payload_path,
        description,
        "JSON module",
        NORMALIZED_JSON_MODULE_FILENAME,
    )
}

fn observe_raw_source(
    oracle: &OsStr,
    source: &[u8],
    description: &str,
    kind: &str,
    mode: &str,
    filename: &str,
    normalized_filename: &str,
) -> RawScriptObservation {
    let script = TempSource::new(source, description, kind, filename);
    let output = Command::new(oracle)
        .arg(mode)
        .arg(&script.path)
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS raw {kind} for {description}: {error}")
        });

    observe_output(output, &script.path, description, kind, normalized_filename)
}

fn observe_output(
    output: std::process::Output,
    diagnostic_path: &std::path::Path,
    description: &str,
    kind: &str,
    normalized_filename: &str,
) -> RawScriptObservation {
    if output.status.success() {
        assert!(
            output.stderr.is_empty(),
            "QuickJS raw {kind} emitted stderr for {description}: {}",
            String::from_utf8_lossy(&output.stderr),
        );
        let stdout = String::from_utf8(output.stdout).unwrap_or_else(|error| {
            panic!("QuickJS raw {kind} stdout was not UTF-8 for {description}: {error}")
        });
        let lines = stdout.lines().collect::<Vec<_>>();
        assert_eq!(
            lines.len(),
            1,
            "QuickJS raw {kind} must emit one observation for {description}; stdout was {stdout:?}",
        );
        assert!(
            lines[0].is_ascii(),
            "QuickJS raw observation was not ASCII for {description}: {:?}",
            lines[0],
        );
        return RawScriptObservation::Return(lines[0].to_owned());
    }

    assert!(
        output.stdout.is_empty(),
        "QuickJS rejected raw {kind} but emitted stdout for {description}: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|error| {
        panic!("QuickJS raw {kind} stderr was not UTF-8 for {description}: {error}")
    });
    let lines = stderr
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        2,
        "unexpected QuickJS raw {kind} diagnostic shape for {description}: {stderr:?}",
    );
    let (name, message) = lines[0].split_once(": ").unwrap_or_else(|| {
        panic!(
            "QuickJS raw diagnostic had no error name for {description}: {:?}",
            lines[0],
        )
    });
    let location = lines[1].trim().strip_prefix("at ").unwrap_or_else(|| {
        panic!(
            "QuickJS raw diagnostic had no source location for {description}: {:?}",
            lines[1],
        )
    });
    let mut location_parts = location.rsplitn(3, ':');
    let column = parse_location_part(location_parts.next(), "column", description, location);
    let line = parse_location_part(location_parts.next(), "line", description, location);
    let filename = location_parts.next().unwrap_or_else(|| {
        panic!("QuickJS raw diagnostic had no filename for {description}: {location:?}")
    });
    assert_eq!(
        filename,
        diagnostic_path.to_string_lossy(),
        "QuickJS changed the raw {kind} filename for {description}",
    );

    RawScriptObservation::Throw {
        name: name.to_owned(),
        message: message.to_owned(),
        filename: normalized_filename.to_owned(),
        line,
        column,
    }
}

struct TempJsonModuleGraph {
    directory: PathBuf,
    root_path: PathBuf,
    payload_path: PathBuf,
}

impl TempJsonModuleGraph {
    fn new(source: &[u8], extended: bool, description: &str) -> Self {
        let directory = create_temp_directory("json-module", description);
        let root_path = directory.join("entry.mjs");
        let payload_path = directory.join(if extended { "value.data" } else { "value.json" });
        write_source_file(
            &root_path,
            raw_json_module_source(extended).as_bytes(),
            description,
            "JSON module root",
        );
        write_source_file(
            &payload_path,
            source,
            description,
            "raw JSON module payload",
        );
        Self {
            directory,
            root_path,
            payload_path,
        }
    }
}

impl Drop for TempJsonModuleGraph {
    fn drop(&mut self) {
        let _ = remove_file(&self.root_path);
        let _ = remove_file(&self.payload_path);
        let _ = remove_dir(&self.directory);
    }
}

fn parse_location_part(part: Option<&str>, label: &str, description: &str, location: &str) -> u32 {
    part.unwrap_or_else(|| {
        panic!("QuickJS raw diagnostic had no {label} for {description}: {location:?}")
    })
    .parse::<u32>()
    .unwrap_or_else(|error| {
        panic!(
            "QuickJS raw diagnostic had an invalid {label} for {description}: {location:?}: {error}"
        )
    })
}

struct TempSource {
    directory: PathBuf,
    path: PathBuf,
}

impl TempSource {
    fn new(source: &[u8], description: &str, kind: &str, filename: &str) -> Self {
        let directory = create_temp_directory(kind, description);
        let path = directory.join(filename);
        let script = Self { directory, path };
        write_source_file(&script.path, source, description, kind);
        script
    }
}

impl Drop for TempSource {
    fn drop(&mut self) {
        let _ = remove_file(&self.path);
        let _ = remove_dir(&self.directory);
    }
}

fn create_temp_directory(kind: &str, description: &str) -> PathBuf {
    loop {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let candidate = std::env::temp_dir().join(format!(
            "quickjs-oxide-raw-{kind}-{}-{id}",
            std::process::id(),
        ));
        match create_dir(&candidate) {
            Ok(()) => return candidate,
            Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
            Err(error) => {
                panic!("could not create QuickJS raw-{kind} directory for {description}: {error}")
            }
        }
    }
}

fn write_source_file(path: &std::path::Path, source: &[u8], description: &str, kind: &str) {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap_or_else(|error| {
            panic!("could not create QuickJS raw {kind} for {description}: {error}")
        });
    file.write_all(source).unwrap_or_else(|error| {
        panic!("could not write QuickJS raw {kind} for {description}: {error}")
    });
    file.flush().unwrap_or_else(|error| {
        panic!("could not flush QuickJS raw {kind} for {description}: {error}")
    });
}
