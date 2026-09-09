use super::*;

const STRING_CREATE_HTML_ENTRIES: [(&str, StringCreateHtmlKind, u8); 13] = [
    ("anchor", StringCreateHtmlKind::Anchor, 1),
    ("big", StringCreateHtmlKind::Big, 0),
    ("blink", StringCreateHtmlKind::Blink, 0),
    ("bold", StringCreateHtmlKind::Bold, 0),
    ("fixed", StringCreateHtmlKind::Fixed, 0),
    ("fontcolor", StringCreateHtmlKind::FontColor, 1),
    ("fontsize", StringCreateHtmlKind::FontSize, 1),
    ("italics", StringCreateHtmlKind::Italics, 0),
    ("link", StringCreateHtmlKind::Link, 1),
    ("small", StringCreateHtmlKind::Small, 0),
    ("strike", StringCreateHtmlKind::Strike, 0),
    ("sub", StringCreateHtmlKind::Sub, 0),
    ("sup", StringCreateHtmlKind::Sup, 0),
];

const STRING_CASE_ENTRIES: [(&str, StringCaseKind); 4] = [
    ("toLowerCase", StringCaseKind::Lower),
    ("toUpperCase", StringCaseKind::Upper),
    ("toLocaleLowerCase", StringCaseKind::Lower),
    ("toLocaleUpperCase", StringCaseKind::Upper),
];

mod registration;

mod code_points;

mod unicode;

mod search;

mod regexp_protocol;

mod split;

mod case_conversion;

mod html;

mod repeat;

mod padding;

mod trimming;

mod subranges;

mod recursion;

mod construction;
