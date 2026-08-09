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

const STD_LINES_EVALUATOR: &str = r#"
(function(){
  if(os.platform==='win32')os.ttySetRaw(0);
  var source=std.in.readAsString();
  std.in.clearerr();
  std.evalScript(source);
})();
"#;

// Read through the temporary --std bindings, then remove exactly those two
// globals before evaluating the streamed source. For the audited generated
// batches, this preserves the observable globals of plain `qjs -e`; callers
// must not inspect stdin or module-loader state, nor rely on the eval filename
// or signal handling.
const PLAIN_SOURCE_EVALUATOR: &str = r#"
(function(input,system){
  if(system.platform==='win32')system.ttySetRaw(0);
  var source=input.in.readAsString();
  input.in.clearerr();
  var hidStd=delete globalThis.std;
  var hidOs=delete globalThis.os;
  if(!hidStd||!hidOs)throw new Error('could not hide temporary std/os globals');
  input.evalScript(source,{backtrace_barrier:true});
})(std,os);
"#;

pub(super) fn observe_completion(oracle: &OsStr, source: &str, description: &str) -> String {
    run_stdin_utf8(
        oracle,
        COMPLETION_OBSERVER,
        Some(STDIN_SENTINEL),
        source,
        description,
        "observer",
    )
    .trim_end()
    .to_owned()
}

pub(super) fn eval_std_lines(oracle: &OsStr, source: &str, description: &str) -> Vec<String> {
    run_stdin_utf8(
        oracle,
        STD_LINES_EVALUATOR,
        None,
        source,
        description,
        "std-lines evaluation",
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

pub(super) fn eval_indexed_plain_lines(
    oracle: &OsStr,
    source: &str,
    expected_lines: usize,
    description: &str,
) -> Vec<String> {
    let stdout = run_stdin_utf8(
        oracle,
        PLAIN_SOURCE_EVALUATOR,
        None,
        source,
        description,
        "plain indexed evaluation",
    );
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(
        lines.len(),
        expected_lines,
        "QuickJS {description} emitted the wrong line count",
    );
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            let prefix = format!("{index}|");
            line.strip_prefix(&prefix)
                .unwrap_or_else(|| panic!("QuickJS {description} index mismatch: {line:?}"))
                .to_owned()
        })
        .collect()
}

fn run_stdin_utf8(
    oracle: &OsStr,
    bootstrap: &str,
    script_argument: Option<&str>,
    source: &str,
    description: &str,
    failure_label: &str,
) -> String {
    let mut command = Command::new(oracle);
    command.args(["--std", "-e", bootstrap]);
    if let Some(script_argument) = script_argument {
        command.args(["--", script_argument]);
    }
    let mut child = command
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
        "QuickJS {failure_label} failed for {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    write_result
        .unwrap_or_else(|_| panic!("QuickJS stdin writer panicked for {description}"))
        .unwrap_or_else(|error| {
            panic!("could not stream QuickJS source for {description}: {error}")
        });
    String::from_utf8(output.stdout)
        .unwrap_or_else(|error| panic!("QuickJS output was not UTF-8 for {description}: {error}"))
}
