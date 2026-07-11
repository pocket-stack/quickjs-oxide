use quickjs_oxide::{Error, ErrorKind, JsString, JsStringError};

struct OversizedLowerBound;

impl Iterator for OversizedLowerBound {
    type Item = u16;

    fn next(&mut self) -> Option<Self::Item> {
        panic!("an impossible UTF-16 lower bound must reject before polling")
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (JsString::MAX_LEN + 1, None)
    }
}

#[test]
fn public_checked_constructors_preserve_utf16_and_reject_before_polling() {
    let utf8 = JsString::try_from_utf8("aé😀").unwrap();
    assert_eq!(utf8.len(), 4);
    assert_eq!(
        utf8.utf16_units().collect::<Vec<_>>(),
        [0x61, 0x00e9, 0xd83d, 0xde00]
    );

    let utf16 = JsString::try_from_utf16([0xd800, 0x61, 0xdc00]).unwrap();
    assert_eq!(
        utf16.utf16_units().collect::<Vec<_>>(),
        [0xd800, 0x61, 0xdc00]
    );
    assert_eq!(
        JsString::try_from_utf16(OversizedLowerBound),
        Err(JsStringError::TooLong)
    );
}

#[test]
fn string_length_failure_maps_to_quickjs_internal_error() {
    let error = Error::from(JsStringError::TooLong);
    assert_eq!(error.kind(), ErrorKind::JsInternal);
    assert_eq!(error.message(), "string too long");
}
