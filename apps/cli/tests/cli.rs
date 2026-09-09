fn run_cli(
    program: &std::ffi::OsStr,
    options: &[&str],
    source: &str,
    description: &str,
) -> std::process::Output {
    std::process::Command::new(program)
        .args(options)
        .args(["-e", source])
        .output()
        .unwrap_or_else(|error| panic!("could not run CLI for {description}: {error}"))
}

use std::ffi::OsStr;
use std::fs;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_MODULE_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct ModuleFixture {
    root: PathBuf,
}

impl ModuleFixture {
    fn new() -> Self {
        let id = NEXT_MODULE_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "quickjs-oxide-cli-module-{}-{id}",
            std::process::id()
        ));
        if root.exists() {
            fs::remove_dir_all(&root).expect("remove stale CLI module fixture");
        }
        fs::create_dir_all(&root).expect("create CLI module fixture");
        Self { root }
    }

    fn write(&self, relative: &str, source: &str) -> PathBuf {
        self.write_bytes(relative, source.as_bytes())
    }

    fn write_bytes(&self, relative: &str, source: &[u8]) -> PathBuf {
        let path = self.root.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create CLI module fixture directory");
        }
        fs::write(&path, source).expect("write CLI module fixture");
        path
    }
}

impl Drop for ModuleFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn qjs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_qjs"))
}

fn cli_path(path: &Path) -> String {
    let path = path.to_string_lossy();
    #[cfg(windows)]
    return path.replace('\\', "/");
    #[cfg(not(windows))]
    path.into_owned()
}

#[path = "cli/evaluation.rs"]
mod evaluation;

#[path = "cli/arguments.rs"]
mod arguments;

#[path = "cli/printing.rs"]
mod printing;

#[path = "cli/modules.rs"]
mod modules;

#[path = "cli/options.rs"]
mod options;
