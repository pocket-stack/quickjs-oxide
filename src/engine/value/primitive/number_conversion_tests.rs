//! Number conversion parity across compact ASCII and the UTF-16 fallback.
use super::{JsString, StringRepr, string_to_number, string_to_number_utf16};
use std::rc::Rc;

fn assert_number(actual: f64, expected: f64, label: &str) {
    if expected.is_nan() {
        assert!(actual.is_nan(), "{label}: {actual:?}");
    } else {
        assert_eq!(actual.to_bits(), expected.to_bits(), "{label}");
    }
}

fn assert_ascii_number(text: &str, expected: f64) {
    assert!(text.is_ascii());
    let compact = JsString::try_from_utf8(text).unwrap();
    assert!(matches!(&*compact.0, StringRepr::Latin1(_)));
    let wide = JsString(Rc::new(StringRepr::Utf16(
        text.encode_utf16().collect::<Vec<_>>().into_boxed_slice(),
    )));
    assert_number(string_to_number(&compact), expected, text);
    assert_number(string_to_number(&wide), expected, text);
    assert_number(string_to_number_utf16(&compact), expected, text);
}

#[test]
fn ascii_number_grammar_preserves_sign_rounding_and_whole_input_requirement() {
    for (text, expected) in [
        ("", 0.0),
        (" \t\n\r\u{b}\u{c}", 0.0),
        ("  -0\n", -0.0),
        ("-0.000e+99", -0.0),
        ("+.5", 0.5),
        ("1.", 1.0),
        ("01", 1.0),
        ("1.e2", 100.0),
        ("1e-324", 0.0),
        ("-1e-324", -0.0),
        ("5e-324", f64::from_bits(1)),
        ("9007199254740993", 9007199254740992.0),
        ("9007199254740995", 9007199254740996.0),
        ("Infinity", f64::INFINITY),
        ("+Infinity", f64::INFINITY),
        ("-Infinity", f64::NEG_INFINITY),
        ("1e9999", f64::INFINITY),
        ("-1e9999", f64::NEG_INFINITY),
        ("0x20000000000001", 9007199254740992.0),
        ("0X20000000000003", 9007199254740996.0),
        ("0o17", 15.0),
        ("0O17", 15.0),
        ("0b101", 5.0),
        ("0B101", 5.0),
    ] {
        assert_ascii_number(text, expected);
    }
    for text in [
        "+",
        "-",
        ".",
        "+.",
        "1e",
        "1e+",
        "1e-",
        "1e1.0",
        "0x",
        "0o",
        "0b",
        "-0x1",
        "+0x1",
        "-0o1",
        "+0b1",
        "0x1_0",
        "0b2",
        "0o8",
        "1_0",
        "1n",
        "NaN",
        "inf",
        "infinity",
        "INFINITY",
        "Infinityx",
        "1 2",
        "1\0",
        "\x001",
        "\x081",
        "\x0e1",
        "\x1c1",
        "\x1f1",
        "\x7f1",
    ] {
        assert_ascii_number(text, f64::NAN);
    }
}

#[test]
fn latin1_is_not_utf8_and_unicode_space_and_surrogates_keep_fallback_semantics() {
    for unit in [
        0x0009, 0x000a, 0x000b, 0x000c, 0x000d, 0x0020, 0x00a0, 0x1680, 0x2000, 0x2001, 0x2002,
        0x2003, 0x2004, 0x2005, 0x2006, 0x2007, 0x2008, 0x2009, 0x200a, 0x2028, 0x2029, 0x202f,
        0x205f, 0x3000, 0xfeff,
    ] {
        let value = JsString::try_from_utf16([unit, b'-' as u16, b'0' as u16, unit]).unwrap();
        assert_number(string_to_number(&value), -0.0, "ECMAScript whitespace");
        let interior = JsString::try_from_utf16([b'1' as u16, unit, b'2' as u16]).unwrap();
        assert!(string_to_number(&interior).is_nan());
    }
    for unit in [0x0085, 0x180e, 0x200b, 0xd800, 0xdc00, 0xfffd, 0xff11] {
        let value = JsString::try_from_utf16([unit, b'1' as u16]).unwrap();
        assert!(string_to_number(&value).is_nan(), "{unit:x}");
    }
    let pair = JsString::try_from_utf16([0xd83d, 0xde00]).unwrap();
    assert!(string_to_number(&pair).is_nan());
    // These Latin-1 code units happen to form valid UTF-8 bytes. Treating the
    // storage as UTF-8 before proving ASCII could incorrectly trim U+00A0.
    let disguised_utf8 = JsString::from_owned_latin1(vec![0xc2, 0xa0, b'1']);
    assert!(string_to_number(&disguised_utf8).is_nan());
}

#[test]
fn long_ascii_and_rope_numbers_preserve_fallback_results_without_linearization() {
    for text in [
        format!("{}-0{}", " ".repeat(9000), "\t".repeat(9000)),
        format!("{}1.25", "0".repeat(12000)),
        format!("0x{}1", "0".repeat(12000)),
        format!("0x{}", "f".repeat(12000)),
        format!("1{}e-12000", "0".repeat(12000)),
    ] {
        let flat = JsString::try_from_utf8(&text).unwrap();
        let expected = string_to_number_utf16(&flat);
        assert_number(string_to_number(&flat), expected, "long ASCII");
        let middle = text.len() / 2;
        let rope = JsString::try_from_utf8(&text[..middle])
            .unwrap()
            .try_concat(&JsString::try_from_utf8(&text[middle..]).unwrap())
            .unwrap();
        assert!(matches!(&*rope.0, StringRepr::Rope(_)));
        assert_number(string_to_number(&rope), expected, "rope");
        let StringRepr::Rope(repr) = &*rope.0 else {
            unreachable!()
        };
        assert!(matches!(
            &*repr.state.borrow(),
            super::RopeState::Tree { .. }
        ));
    }
}
