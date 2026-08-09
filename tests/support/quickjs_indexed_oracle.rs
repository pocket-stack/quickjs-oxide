use std::ffi::OsStr;

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

pub(super) fn eval_indexed_plain_lines(
    oracle: &OsStr,
    source: &str,
    expected_lines: usize,
    description: &str,
) -> Vec<String> {
    let stdout = super::quickjs_oracle::run_stdin_utf8(
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
