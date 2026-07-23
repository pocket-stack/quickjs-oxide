//! Pinned QuickJS `RegExp.escape`.

use super::super::super::*;

#[derive(Clone, Copy)]
enum EscapeEncoding {
    Raw,
    Quoted,
    Hex2,
    Hex4,
    Control(u16),
}

enum EscapeBuffer {
    Latin1(Vec<u8>),
    Utf16(Vec<u16>),
}

impl EscapeBuffer {
    fn try_with_exact_capacity(capacity: usize, wide: bool) -> Result<Self, JsStringError> {
        if wide {
            let mut units = Vec::new();
            units
                .try_reserve_exact(capacity)
                .map_err(|_| JsStringError::OutOfMemory)?;
            Ok(Self::Utf16(units))
        } else {
            let mut units = Vec::new();
            units
                .try_reserve_exact(capacity)
                .map_err(|_| JsStringError::OutOfMemory)?;
            Ok(Self::Latin1(units))
        }
    }

    fn push_unit(&mut self, unit: u16) {
        match self {
            Self::Latin1(units) => units.push(
                u8::try_from(unit).expect("narrow RegExp.escape output contained a wide unit"),
            ),
            Self::Utf16(units) => units.push(unit),
        }
    }

    fn push_code_point(&mut self, code_point: u32) {
        if code_point <= 0xffff {
            self.push_unit(code_point as u16);
            return;
        }
        let adjusted = code_point - 0x1_0000;
        self.push_unit(0xd800 | ((adjusted >> 10) as u16));
        self.push_unit(0xdc00 | ((adjusted & 0x3ff) as u16));
    }

    fn finish(self) -> JsString {
        match self {
            Self::Latin1(units) => JsString::from_owned_latin1(units),
            Self::Utf16(units) => JsString::from_owned_utf16(units),
        }
    }
}

impl Runtime {
    pub(super) fn call_regexp_escape(
        &self,
        realm: ContextId,
        invocation: NativeInvocation,
        arguments: &NativeArguments,
    ) -> Result<Completion, RuntimeError> {
        let NativeInvocation::Call { .. } = invocation else {
            return Err(RuntimeError::Invariant(
                "RegExp.escape did not receive a generic invocation",
            ));
        };
        let argument = arguments
            .readable
            .first()
            .ok_or(RuntimeError::Invariant("RegExp.escape argv was not padded"))?;
        let Value::String(source) = argument else {
            return Ok(Completion::Throw(self.new_native_error(
                realm,
                NativeErrorKind::Type,
                "not a string",
            )?));
        };
        Ok(Completion::Return(Value::String(regexp_escape_with_limit(
            source,
            JsString::MAX_LEN,
        )?)))
    }
}

fn regexp_escape_with_limit(source: &JsString, limit: usize) -> Result<JsString, JsStringError> {
    if source.is_empty() {
        return Ok(JsString::from_static(""));
    }

    // JS_ToString on an already-String input is still required by QuickJS to
    // linearize ropes before reading their width and code points.
    let source = source.linearize();
    let mut output_len = 0;
    let mut index = 0;
    while index < source.len() {
        let code_point = source
            .code_point_at(index)
            .expect("RegExp.escape index was inside its source");
        let consumed = code_point_utf16_len(code_point);
        let encoded = escape_encoding(code_point, index == 0);
        let additional = encoded_utf16_len(encoded, code_point);
        output_len = JsString::checked_length_with_limit(output_len, additional, limit)?;
        index += consumed;
    }

    let mut output = EscapeBuffer::try_with_exact_capacity(output_len, source.is_wide())?;
    let mut index = 0;
    while index < source.len() {
        let code_point = source
            .code_point_at(index)
            .expect("RegExp.escape index was inside its source");
        match escape_encoding(code_point, index == 0) {
            EscapeEncoding::Raw => output.push_code_point(code_point),
            EscapeEncoding::Quoted => {
                output.push_unit(u16::from(b'\\'));
                output.push_code_point(code_point);
            }
            EscapeEncoding::Hex2 => push_hex_escape(&mut output, b'x', code_point, 2),
            EscapeEncoding::Hex4 => push_hex_escape(&mut output, b'u', code_point, 4),
            EscapeEncoding::Control(letter) => {
                output.push_unit(u16::from(b'\\'));
                output.push_unit(letter);
            }
        }
        index += code_point_utf16_len(code_point);
    }
    Ok(output.finish())
}

const fn code_point_utf16_len(code_point: u32) -> usize {
    if code_point > 0xffff { 2 } else { 1 }
}

const fn encoded_utf16_len(encoding: EscapeEncoding, code_point: u32) -> usize {
    match encoding {
        EscapeEncoding::Raw => code_point_utf16_len(code_point),
        EscapeEncoding::Quoted | EscapeEncoding::Control(_) => 2,
        EscapeEncoding::Hex2 => 4,
        EscapeEncoding::Hex4 => 6,
    }
}

const fn escape_encoding(code_point: u32, first: bool) -> EscapeEncoding {
    if code_point < 33 {
        return match code_point {
            9 => EscapeEncoding::Control(b't' as u16),
            10 => EscapeEncoding::Control(b'n' as u16),
            11 => EscapeEncoding::Control(b'v' as u16),
            12 => EscapeEncoding::Control(b'f' as u16),
            13 => EscapeEncoding::Control(b'r' as u16),
            _ => EscapeEncoding::Hex2,
        };
    }
    if code_point < 128 {
        let ascii_alphanumeric = matches!(code_point, 0x30..=0x39 | 0x41..=0x5a | 0x61..=0x7a);
        if ascii_alphanumeric {
            return if first {
                EscapeEncoding::Hex2
            } else {
                EscapeEncoding::Raw
            };
        }
        if matches!(
            code_point,
            0x2c | 0x2d
                | 0x3d
                | 0x3c
                | 0x3e
                | 0x23
                | 0x26
                | 0x21
                | 0x25
                | 0x3a
                | 0x3b
                | 0x40
                | 0x7e
                | 0x27
                | 0x60
                | 0x22
        ) {
            return EscapeEncoding::Hex2;
        }
        return if code_point == b'_' as u32 {
            EscapeEncoding::Raw
        } else {
            EscapeEncoding::Quoted
        };
    }
    if code_point < 256 {
        return EscapeEncoding::Hex2;
    }
    if is_surrogate(code_point) || is_regexp_space(code_point) {
        EscapeEncoding::Hex4
    } else {
        EscapeEncoding::Raw
    }
}

fn push_hex_escape(output: &mut EscapeBuffer, prefix: u8, value: u32, digits: u32) {
    output.push_unit(u16::from(b'\\'));
    output.push_unit(u16::from(prefix));
    for shift in (0..digits).rev() {
        output.push_unit(u16::from(hex_digit(((value >> (shift * 4)) & 0xf) as u8)));
    }
}

const fn hex_digit(value: u8) -> u8 {
    match value {
        0..=9 => b'0' + value,
        _ => b'a' + (value - 10),
    }
}

const fn is_surrogate(code_point: u32) -> bool {
    matches!(code_point, 0xd800..=0xdfff)
}

const fn is_regexp_space(code_point: u32) -> bool {
    matches!(
        code_point,
        0x0009..=0x000d
            | 0x0020
            | 0x00a0
            | 0x1680
            | 0x2000..=0x200a
            | 0x2028
            | 0x2029
            | 0x202f
            | 0x205f
            | 0x3000
            | 0xfeff
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn escape(units: impl IntoIterator<Item = u16>) -> JsString {
        let source = JsString::try_from_utf16(units).unwrap();
        regexp_escape_with_limit(&source, JsString::MAX_LEN).unwrap()
    }

    #[test]
    fn escape_matches_pinned_quickjs_character_classes() {
        let source = JsString::try_from_utf16([
            0x61, 0x31, 0x5f, 0x2e, 0x2d, 0x00, 0x09, 0x7f, 0x80, 0xe9, 0x1680, 0x180e, 0x200b,
            0xd800, 0xd83d, 0xde00,
        ])
        .unwrap();
        let escaped = regexp_escape_with_limit(&source, JsString::MAX_LEN).unwrap();
        assert_eq!(
            escaped.to_utf8_lossy(),
            "\\x611_\\.\\x2d\\x00\\t\\\u{7f}\\x80\\xe9\\u1680\u{180e}\u{200b}\\ud800😀",
        );
    }

    #[test]
    fn escape_preserves_wide_storage_and_checks_the_expanded_limit() {
        let wide = JsString::try_from_utf16([0xd800]).unwrap();
        let escaped = regexp_escape_with_limit(&wide, 6).unwrap();
        assert!(escaped.is_wide());
        assert_eq!(escaped.to_utf8_lossy(), "\\ud800");
        assert_eq!(
            regexp_escape_with_limit(&wide, 5),
            Err(JsStringError::TooLong),
        );

        let narrow = escape([u16::from(b'a')]);
        assert!(!narrow.is_wide());
        assert_eq!(narrow.to_utf8_lossy(), "\\x61");
    }
}
