use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

const STDIN_SENTINEL: &str = "qjs-oxide-stdin";

const COMPLETION_OBSERVER: &str = r#"
if(os.platform==='win32')os.ttySetRaw(0);
scriptArgs[0]=std.in.readAsString();
std.in.clearerr();
try {
  var value=std.evalScript(scriptArgs[0]);
  print('return|'+typeof value+'|'+String(value));
} catch(error) {
  if(error!==null&&typeof error==='object')print('throw|object|'+error.name+'|'+error.message);
  else print('throw|'+typeof error+'|'+String(error));
}
"#;

pub(super) fn observe(oracle: &OsStr, source: &str, description: &str) -> String {
    let mut child = Command::new(oracle)
        .args(["--std", "-e", COMPLETION_OBSERVER, "--", STDIN_SENTINEL])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("could not run QuickJS for {description}: {error}"));
    let mut stdin = child
        .stdin
        .take()
        .unwrap_or_else(|| panic!("QuickJS stdin was not available for {description}"));

    // Write source while wait_with_output drains both output pipes. This keeps
    // large inputs out of argv without introducing a pipe-order deadlock.
    let (output, write_result) = std::thread::scope(|scope| {
        let writer = scope.spawn(move || stdin.write_all(source.as_bytes()));
        let output = child.wait_with_output();
        let write_result = writer.join();
        (output, write_result)
    });
    let output = output
        .unwrap_or_else(|error| panic!("could not wait for QuickJS for {description}: {error}"));
    assert!(
        output.status.success(),
        "QuickJS observer failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    write_result
        .unwrap_or_else(|_| panic!("QuickJS stdin writer panicked for {description}"))
        .unwrap_or_else(|error| {
            panic!("could not stream QuickJS source for {description}: {error}")
        });
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
        .trim_end()
        .to_owned()
}
