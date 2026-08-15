use std::ffi::OsStr;
use std::fs::{OpenOptions, create_dir, remove_dir, remove_file};
use std::io::{ErrorKind, Write};
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

const NORMALIZED_FILENAME: &str = "<raw-script>";
static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);

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

pub(crate) fn normalized_filename() -> &'static str {
    NORMALIZED_FILENAME
}

pub(crate) fn observe_raw_script(
    oracle: &OsStr,
    source: &[u8],
    description: &str,
) -> RawScriptObservation {
    let script = TempScript::new(source, description);
    let output = Command::new(oracle)
        .arg("--script")
        .arg(&script.path)
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS raw script for {description}: {error}")
        });

    if output.status.success() {
        assert!(
            output.stderr.is_empty(),
            "QuickJS raw script emitted stderr for {description}: {}",
            String::from_utf8_lossy(&output.stderr),
        );
        let stdout = String::from_utf8(output.stdout).unwrap_or_else(|error| {
            panic!("QuickJS raw script stdout was not UTF-8 for {description}: {error}")
        });
        let lines = stdout.lines().collect::<Vec<_>>();
        assert_eq!(
            lines.len(),
            1,
            "QuickJS raw script must emit one observation for {description}; stdout was {stdout:?}",
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
        "QuickJS rejected raw script but emitted stdout for {description}: {}",
        String::from_utf8_lossy(&output.stdout),
    );
    let stderr = String::from_utf8(output.stderr).unwrap_or_else(|error| {
        panic!("QuickJS raw script stderr was not UTF-8 for {description}: {error}")
    });
    let lines = stderr
        .lines()
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        2,
        "unexpected QuickJS raw diagnostic shape for {description}: {stderr:?}",
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
        script.path.to_string_lossy(),
        "QuickJS changed the raw script filename for {description}",
    );

    RawScriptObservation::Throw {
        name: name.to_owned(),
        message: message.to_owned(),
        filename: NORMALIZED_FILENAME.to_owned(),
        line,
        column,
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

struct TempScript {
    directory: PathBuf,
    path: PathBuf,
}

impl TempScript {
    fn new(source: &[u8], description: &str) -> Self {
        let directory = loop {
            let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
            let candidate = std::env::temp_dir().join(format!(
                "quickjs-oxide-raw-script-{}-{id}",
                std::process::id(),
            ));
            match create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == ErrorKind::AlreadyExists => continue,
                Err(error) => {
                    panic!(
                        "could not create QuickJS raw-script directory for {description}: {error}"
                    )
                }
            }
        };
        let path = directory.join("source.js");
        let script = Self { directory, path };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&script.path)
            .unwrap_or_else(|error| {
                panic!("could not create QuickJS raw script for {description}: {error}")
            });
        file.write_all(source).unwrap_or_else(|error| {
            panic!("could not write QuickJS raw script for {description}: {error}")
        });
        file.flush().unwrap_or_else(|error| {
            panic!("could not flush QuickJS raw script for {description}: {error}")
        });
        drop(file);
        script
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = remove_file(&self.path);
        let _ = remove_dir(&self.directory);
    }
}
