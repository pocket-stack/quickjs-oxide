use std::ffi::OsStr;
use std::process::Command;

const NAME_ONLY_COMPLETION_OBSERVER: &str = r#"
if(os.platform==='win32')os.ttySetRaw(1);
try {
  var value=std.evalScript(scriptArgs[0]);
  std.out.puts('return|'+typeof value+'|'+String(value));
} catch(error) {
  if(error!==null&&typeof error==='object')
    std.out.puts('throw|object|'+error.name);
  else
    std.out.puts('throw|'+typeof error+'|'+String(error));
}
"#;

pub(super) fn observe_completion_name_only(
    oracle: &OsStr,
    source: &str,
    group: &str,
    description: &str,
) -> String {
    // Source stays in scriptArgs[0] because these cases observe that host
    // contract. `--` keeps leading-hyphen source out of qjs option parsing.
    let output = Command::new(oracle)
        .args(["--std", "-e", NAME_ONLY_COMPLETION_OBSERVER, "--", source])
        .output()
        .unwrap_or_else(|error| {
            panic!("could not run QuickJS for {group} / {description}: {error}")
        });
    assert!(
        output.status.success(),
        "QuickJS observer failed for {group} / {description}: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    String::from_utf8(output.stdout).unwrap_or_else(|error| {
        panic!("QuickJS output was not UTF-8 for {group} / {description}: {error}")
    })
}
