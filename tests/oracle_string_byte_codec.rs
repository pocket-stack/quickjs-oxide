use std::ffi::OsStr;
use std::io::Write;
use std::process::{Command, Stdio};

use quickjs_oxide::JsString;

#[test]
fn quickjs_byte_constructor_and_wtf8_export_match_pinned_quickjs() {
    let Some(oracle) = std::env::var_os("QJS_ORACLE") else {
        eprintln!("SKIP String byte-codec differential: set QJS_ORACLE to upstream qjs");
        return;
    };

    let byte_inputs = vec![
        vec![],
        vec![0x00, 0x41, 0x7f],
        vec![0xc2, 0x80, 0xdf, 0xbf],
        vec![
            0xe0, 0xa0, 0x80, 0xed, 0x9f, 0xbf, 0xee, 0x80, 0x80, 0xef, 0xbf, 0xbf,
        ],
        vec![0xf0, 0x90, 0x80, 0x80, 0xf4, 0x8f, 0xbf, 0xbf],
        vec![0xed, 0xa0, 0x80],
        vec![0xed, 0xaf, 0xbf, 0xed, 0xb0, 0x80, 0xed, 0xbf, 0xbf],
        vec![0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80],
        vec![0x80, 0x41],
        vec![0x80, 0x80, 0x41, 0x80, 0x42],
        vec![0x80, 0xc2, 0xa2],
        vec![0xff, 0x41],
        vec![0xc0, 0x80, 0x41],
        vec![0xe0, 0x80, 0x80],
        vec![0xf0, 0x80, 0x80, 0x80],
        vec![0xf4, 0x90, 0x80, 0x80],
        vec![0xe2, 0x82],
        vec![0xe2, 0x28, 0xa1],
        vec![0xf8, 0x88, 0x80, 0x80, 0x80],
        vec![0xfc, 0x84, 0x80, 0x80, 0x80, 0x80],
        vec![0xef, 0xbb, 0xbf],
        vec![0x41, 0xc3, 0xa9, 0x00, 0xed, 0xa0, 0x80, 0xff, 0x42],
    ];
    let upstream_units = oracle_decode_bytes(&oracle, &byte_inputs);
    assert_eq!(upstream_units.len(), byte_inputs.len());
    for (index, (bytes, expected)) in byte_inputs.iter().zip(upstream_units).enumerate() {
        let actual = JsString::try_from_bytes(bytes).unwrap();
        assert_eq!(
            actual.utf16_units().collect::<Vec<_>>(),
            expected,
            "JS_NewStringLen byte case {index}: {bytes:02x?}"
        );
    }

    let utf16_inputs = vec![
        vec![],
        vec![0x0000, 0x0041, 0x007f],
        vec![0x0080, 0x00e9, 0x00ff],
        vec![0x0100, 0x07ff, 0x0800],
        vec![0xd800],
        vec![0xdc00],
        vec![0xd800, 0x0041],
        vec![0xdc00, 0xd800],
        vec![0xd83d, 0xde00],
        vec![0xd800, 0xdc00],
        vec![0xdbff, 0xdfff],
        vec![0xd800, 0xd800, 0xdc00],
        vec![0x0041, 0x0000],
    ];
    let upstream_bytes = oracle_encode_wtf8(&oracle, &utf16_inputs);
    let mut rust_bytes = Vec::new();
    for units in utf16_inputs {
        rust_bytes.extend(
            JsString::try_from_utf16(units)
                .unwrap()
                .try_to_wtf8_bytes()
                .unwrap(),
        );
    }
    assert_eq!(rust_bytes, upstream_bytes);
}

fn oracle_decode_bytes(oracle: &OsStr, inputs: &[Vec<u8>]) -> Vec<Vec<u16>> {
    let sizes = inputs
        .iter()
        .map(|value| value.len().to_string())
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r#"
if (os.platform === "win32") os.ttySetRaw(0);
var sizes = [{sizes}];
for (var n = 0; n < sizes.length; n++) {{
    var value = std.in.readAsString(sizes[n]);
    var units = [];
    for (var i = 0; i < value.length; i++)
        units.push(("0000" + value.charCodeAt(i).toString(16)).slice(-4));
    std.out.puts(value.length + "|" + units.join(",") + "\n");
}}
"#
    );
    let mut child = Command::new(oracle)
        .args(["--std", "-e", &source])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run QuickJS JS_NewStringLen oracle");
    {
        let mut stdin = child.stdin.take().expect("QuickJS stdin was not piped");
        for input in inputs {
            stdin
                .write_all(input)
                .expect("write raw QuickJS oracle bytes");
        }
    }
    let output = child
        .wait_with_output()
        .expect("wait for QuickJS JS_NewStringLen oracle");
    assert!(
        output.status.success(),
        "QuickJS byte constructor oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("QuickJS byte constructor oracle emitted non-ASCII observations")
        .lines()
        .map(|line| {
            let (length, units) = line.split_once('|').expect("malformed QuickJS byte row");
            let length = length
                .parse::<usize>()
                .expect("malformed QuickJS UTF-16 length");
            let units = if units.is_empty() {
                Vec::new()
            } else {
                units
                    .split(',')
                    .map(|unit| u16::from_str_radix(unit, 16).expect("malformed UTF-16 hex unit"))
                    .collect::<Vec<_>>()
            };
            assert_eq!(units.len(), length);
            units
        })
        .collect()
}

fn oracle_encode_wtf8(oracle: &OsStr, inputs: &[Vec<u16>]) -> Vec<u8> {
    let cases = inputs
        .iter()
        .map(|units| {
            format!(
                "[{}]",
                units
                    .iter()
                    .map(|unit| format!("0x{unit:04x}"))
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let source = format!(
        r#"
if (os.platform === "win32") os.ttySetRaw(1);
var cases = [{cases}];
for (var i = 0; i < cases.length; i++)
    std.out.puts(String.fromCharCode.apply(null, cases[i]));
"#
    );
    let output = Command::new(oracle)
        .args(["--std", "-e", &source])
        .output()
        .expect("run QuickJS JS_ToCStringLen2 oracle");
    assert!(
        output.status.success(),
        "QuickJS WTF-8 export oracle failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    output.stdout
}
